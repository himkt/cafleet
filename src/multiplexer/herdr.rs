use crate::multiplexer::BackendCore;
use crate::multiplexer::ColumnTarget;
use crate::multiplexer::CommandRunner;
use crate::multiplexer::Multiplexer;
use crate::multiplexer::MultiplexerContext;
use crate::multiplexer::MultiplexerError;
use crate::multiplexer::PLACEMENT_MISMATCH;
use crate::multiplexer::PaneCleanup;
use crate::multiplexer::PaneOwnership;
use crate::multiplexer::PaneTarget;
use crate::multiplexer::SpawnedPane;
use crate::multiplexer::layout::ColumnSide;
use serde_json::Value;
use std::cell::RefCell;
use std::rc::Rc;

const PANE_NOT_FOUND: &str = "pane_not_found";

fn is_pane_not_found(stderr: &str) -> bool {
    error_code(stderr).as_deref() == Some(PANE_NOT_FOUND)
}

/// The herdr error envelope's `error.code`, when the stderr carries one.
fn error_code(stderr: &str) -> Option<String> {
    let payload: Value = serde_json::from_str(stderr).ok()?;
    payload
        .get("error")?
        .get("code")?
        .as_str()
        .map(str::to_string)
}

fn parse_envelope(out: &str) -> Result<Value, MultiplexerError> {
    let payload: Value = serde_json::from_str(out)
        .map_err(|_| MultiplexerError::new(format!("herdr returned non-JSON output: {out:?}")))?;
    match payload.get("result") {
        Some(result) if result.is_object() => Ok(result.clone()),
        _ => Err(MultiplexerError::new(format!(
            "herdr output missing 'result' object: {out:?}"
        ))),
    }
}

fn str_field<'a>(value: &'a Value, context: &str, key: &str) -> Result<&'a str, MultiplexerError> {
    value[key]
        .as_str()
        .ok_or_else(|| MultiplexerError::new(format!("herdr {context} missing '{key}' field")))
}

fn f64_field(value: &Value, context: &str, key: &str) -> Result<f64, MultiplexerError> {
    value[key]
        .as_f64()
        .ok_or_else(|| MultiplexerError::new(format!("herdr {context} missing '{key}' field")))
}

fn round4(value: f64) -> f64 {
    (value * 10_000.0).round() / 10_000.0
}

fn shlex_quote(token: &str) -> String {
    let safe = !token.is_empty()
        && token
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "_@%+=:,./-".contains(c));
    if safe {
        token.to_string()
    } else {
        format!("'{}'", token.replace('\'', "'\"'\"'"))
    }
}

fn shlex_join(command: &[String]) -> String {
    command
        .iter()
        .map(|token| shlex_quote(token))
        .collect::<Vec<_>>()
        .join(" ")
}

fn panes_of(result: &Value) -> Result<&Vec<Value>, MultiplexerError> {
    result["panes"]
        .as_array()
        .ok_or_else(|| MultiplexerError::new("herdr pane list missing 'panes' field"))
}

fn sort_by_y(panes: &mut [&Value]) {
    panes.sort_by(|a, b| {
        a["rect"]["y"]
            .as_f64()
            .partial_cmp(&b["rect"]["y"].as_f64())
            .expect("layout rects carry numeric y")
    });
}

pub(crate) struct HerdrMultiplexer {
    core: BackendCore,
    /// The tab the last `close_pane` recorded, which `restore_after_close` rebalances.
    closed_tab: RefCell<Option<String>>,
}

impl HerdrMultiplexer {
    pub(crate) fn for_server(runner: Rc<dyn CommandRunner>, session: &str) -> Self {
        HerdrMultiplexer {
            core: BackendCore {
                runner,
                name: "herdr",
                pane_gone: is_pane_not_found,
                server_flag: "--session",
                server: session.to_owned(),
            },
            closed_tab: RefCell::new(None),
        }
    }

    fn run_json(&self, argv: &[String]) -> Result<Value, MultiplexerError> {
        parse_envelope(&self.core.run(argv)?)
    }

    /// The `result` of a pane read, or `None` when herdr reports `pane_not_found`.
    fn pane_read(&self, parts: &[&str]) -> Result<Option<Value>, MultiplexerError> {
        let argv = self.core.argv(parts);
        match self.core.runner.run(&argv) {
            Ok(out) => parse_envelope(&out).map(Some),
            Err(crate::multiplexer::RunError::Failed { ref stderr })
                if is_pane_not_found(stderr) =>
            {
                Ok(None)
            }
            Err(error) => Err(self.core.map_error(&argv, error)),
        }
    }

    fn list_panes(&self) -> Result<Vec<Value>, MultiplexerError> {
        let result = self.run_json(&self.core.argv(&["herdr", "pane", "list"]))?;
        panes_of(&result).cloned()
    }
}

impl Multiplexer for HerdrMultiplexer {
    fn core(&self) -> &BackendCore {
        &self.core
    }

    fn bounded(&self, deadline: crate::time::Deadline) -> Box<dyn Multiplexer> {
        Box::new(Self {
            core: self.core.bounded(deadline),
            closed_tab: RefCell::new(None),
        })
    }

    fn check_key_target(
        &self,
        target: &str,
        placement: &crate::broker::Placement,
        deadline: crate::time::Deadline,
    ) -> Result<(), MultiplexerError> {
        let result = parse_envelope(&self.core.press_run(
            &self.core.argv(&["herdr", "pane", "list"]),
            deadline,
            false,
        )?)?;
        let mut matches = 0;
        for pane in panes_of(&result)? {
            let pane_id = str_field(pane, "pane list", "pane_id")?;
            let workspace = str_field(pane, "pane list", "workspace_id")?;
            let tab = str_field(pane, "pane list", "tab_id")?;
            if pane_id == target {
                if workspace != placement.mux_session || tab != placement.concrete_window_id() {
                    return Err(MultiplexerError::new(PLACEMENT_MISMATCH));
                }
                matches += 1;
            }
        }
        if matches != 1 {
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
        let mut argv = self.core.argv(&["herdr", "pane", "send-keys", pane_id]);
        argv.extend(keys.herdr_arguments());
        self.core.send_keys(&argv, deadline)
    }

    fn pane_context(&self, pane: &str) -> Result<Option<MultiplexerContext>, MultiplexerError> {
        self.pane_read(&["herdr", "pane", "current", "--pane", pane])?
            .map(|result| current_context(&result))
            .transpose()
    }

    fn pane_process(&self, pane: &str) -> Result<Option<i64>, MultiplexerError> {
        self.pane_read(&["herdr", "pane", "process-info", "--pane", pane])?
            .map(|result| {
                result["process_info"]["shell_pid"].as_i64().ok_or_else(|| {
                    MultiplexerError::new("herdr pane process-info missing 'shell_pid' field")
                })
            })
            .transpose()
    }

    fn window_name(&self, window_id: &str) -> Result<String, MultiplexerError> {
        let result = self.run_json(&self.core.argv(&["herdr", "tab", "get", window_id]))?;
        Ok(str_field(&result["tab"], "tab get", "label")?.to_owned())
    }

    fn window_height(&self, _window_id: &str, anchors: &[String]) -> Result<i64, MultiplexerError> {
        assert!(!anchors.is_empty(), "a window height read names an anchor");
        let mut last_error = None;
        for anchor in anchors {
            let read = self
                .run_json(
                    &self
                        .core
                        .argv(&["herdr", "pane", "layout", "--pane", anchor]),
                )
                .and_then(|result| {
                    result["layout"]["area"]["height"].as_i64().ok_or_else(|| {
                        MultiplexerError::new("herdr pane layout missing 'area' field")
                    })
                });
            match read {
                Ok(height) => return Ok(height),
                Err(error) => last_error = Some(error),
            }
        }
        Err(last_error.expect("every anchor was tried"))
    }

    fn spawn_member_pane(
        &self,
        target: &PaneTarget,
        pane_name: &str,
        cwd: &str,
        command: &[String],
    ) -> Result<SpawnedPane, MultiplexerError> {
        let (spawned, reparenting) = match target {
            PaneTarget::Column(column) => self.split_into(column, cwd)?,
            PaneTarget::NewWindow {
                session,
                window_name,
            } => (self.create_tab(session, window_name, cwd)?, None),
        };
        let mut owned = PaneOwnership::new(spawned.pane_id.clone(), |id| {
            self.kill_pane(id, &spawned.window_id)
        });
        if let Err(error) = self.core.run(&self.core.argv(&[
            "herdr",
            "pane",
            "run",
            &spawned.pane_id,
            &shlex_join(command),
        ])) {
            return Err(error.with_pane_cleanup(owned.rollback()));
        }
        owned.finish();
        let _ = self.core.run(&self.core.argv(&[
            "herdr",
            "pane",
            "rename",
            &spawned.pane_id,
            pane_name,
        ]));
        if let Some(reparenting) = reparenting
            && let Err(cause) = self.reparent(&reparenting, &spawned)
        {
            eprintln!("layout re-parenting incomplete: {cause}");
        }
        let _ = self.equalize_tab(&spawned.pane_id);
        Ok(spawned)
    }

    /// `herdr pane read` prints the raw terminal buffer (no envelope); the
    /// shared capture windowing ([`crate::multiplexer::capture_window`]) enforces the last-N
    /// window client-side — the daemon may return more rows than requested.
    fn capture_pane(&self, target_pane_id: &str, lines: i64) -> Result<String, MultiplexerError> {
        crate::multiplexer::check_capture_lines(lines)?;
        let fetch_depth = lines + crate::multiplexer::CAPTURE_OVER_FETCH_LINES;
        let raw = self.core.run(&self.core.argv(&[
            "herdr",
            "pane",
            "read",
            target_pane_id,
            "--format",
            "ansi",
            "--source",
            "recent-unwrapped",
            "--lines",
            &fetch_depth.to_string(),
        ]))?;
        Ok(crate::multiplexer::capture_window(&raw, lines))
    }

    fn close_pane(&self, target_pane_id: &str, window_id: &str) -> Result<(), MultiplexerError> {
        self.core.run_tolerating_missing(&self.core.argv(&[
            "herdr",
            "pane",
            "close",
            target_pane_id,
        ]))?;
        *self.closed_tab.borrow_mut() = Some(window_id.to_owned());
        Ok(())
    }

    fn restore_after_close(&self) -> Result<(), MultiplexerError> {
        match self.closed_tab.take() {
            Some(tab_id) => self.resize_after_close(&tab_id),
            None => Ok(()),
        }
    }
}

/// The pane moves that rebuild a full-height column after its first pane was split rightward.
enum Reparenting {
    /// The left chain `L1..Lk` (k ≥ 2) moves back under `L1` beside the new right pane.
    Right { left: Vec<String> },
    /// The right chain `R1..Rm` (m ≥ 1) moves to the right of the new left pane.
    Left { right: Vec<String> },
}

impl HerdrMultiplexer {
    fn split_pane(
        &self,
        pane_id: &str,
        direction: &str,
        cwd: &str,
    ) -> Result<String, MultiplexerError> {
        let args = self.core.argv(&[
            "herdr",
            "pane",
            "split",
            pane_id,
            "--direction",
            direction,
            "--no-focus",
            "--cwd",
            cwd,
        ]);
        let result = self
            .core
            .run(&args)
            .and_then(|raw| parse_envelope(&raw))
            .map_err(|error| error.with_pane_cleanup(PaneCleanup::Unknown))?;
        let id = str_field(&result["pane"], "pane split", "pane_id")
            .map_err(|error| error.with_pane_cleanup(PaneCleanup::Unknown))?;
        if id.trim().is_empty() {
            return Err(
                MultiplexerError::new("herdr pane split returned an empty pane ID")
                    .with_pane_cleanup(PaneCleanup::Unknown),
            );
        }
        Ok(id.to_string())
    }

    /// Splits the live column order read from the anchor's tab: the column's bottom pane
    /// downward, or the opposite column's top pane rightward for an empty column.
    fn split_into(
        &self,
        column: &ColumnTarget,
        cwd: &str,
    ) -> Result<(SpawnedPane, Option<Reparenting>), MultiplexerError> {
        let anchor = column
            .column_panes
            .first()
            .or(column.other_panes.first())
            .expect("a column target names a pane of its window");
        let (panes, _) = self.read_tab_layout(anchor)?;
        let chain = columns_of(&panes)?
            .into_iter()
            .find(|chain| chain.iter().any(|pane| pane == anchor))
            .ok_or_else(|| {
                MultiplexerError::new(format!(
                    "herdr pane layout of {anchor} does not contain {anchor}"
                ))
            })?;
        let (split_target, direction, reparenting) = if !column.column_panes.is_empty() {
            (
                chain.last().expect("a column holds its anchor"),
                "down",
                None,
            )
        } else {
            let reparenting = match column.side {
                ColumnSide::Right if chain.len() == 1 => None,
                ColumnSide::Right => Some(Reparenting::Right {
                    left: chain.clone(),
                }),
                ColumnSide::Left => Some(Reparenting::Left {
                    right: chain.clone(),
                }),
            };
            (&chain[0], "right", reparenting)
        };
        let pane_id = self.split_pane(split_target, direction, cwd)?;
        Ok((
            SpawnedPane {
                pane_id,
                window_id: column.window_id.clone(),
            },
            reparenting,
        ))
    }

    fn create_tab(
        &self,
        session: &str,
        window_name: &str,
        cwd: &str,
    ) -> Result<SpawnedPane, MultiplexerError> {
        let unknown = |error: MultiplexerError| error.with_pane_cleanup(PaneCleanup::Unknown);
        let result = self
            .run_json(&self.core.argv(&[
                "herdr",
                "tab",
                "create",
                "--workspace",
                session,
                "--cwd",
                cwd,
                "--label",
                window_name,
                "--no-focus",
            ]))
            .map_err(unknown)?;
        let window_id = str_field(&result["tab"], "tab create", "tab_id").map_err(unknown)?;
        let pane_id = str_field(&result["root_pane"], "tab create", "pane_id").map_err(unknown)?;
        Ok(SpawnedPane {
            pane_id: pane_id.to_owned(),
            window_id: window_id.to_owned(),
        })
    }

    fn reparent(
        &self,
        reparenting: &Reparenting,
        spawned: &SpawnedPane,
    ) -> Result<(), MultiplexerError> {
        let tab = spawned.window_id.as_str();
        match reparenting {
            Reparenting::Right { left } => {
                let temporary = self.move_to_new_tab(&left[1])?;
                for i in 2..left.len() {
                    self.move_into(&left[i], &temporary, "down", &left[i - 1])?;
                }
                for i in 1..left.len() {
                    self.move_into(&left[i], tab, "down", &left[i - 1])?;
                }
            }
            Reparenting::Left { right } => {
                let temporary = self.move_to_new_tab(&right[0])?;
                for i in 1..right.len() {
                    self.move_into(&right[i], &temporary, "down", &right[i - 1])?;
                }
                self.move_into(&right[0], tab, "right", &spawned.pane_id)?;
                for i in 1..right.len() {
                    self.move_into(&right[i], tab, "down", &right[i - 1])?;
                }
            }
        }
        Ok(())
    }

    /// The `move_result` of a `pane move` that actually moved `pane`.
    fn moved(&self, pane: &str, parts: &[&str]) -> Result<Value, MultiplexerError> {
        let result = self.run_json(&self.core.argv(parts))?;
        let move_result = result["move_result"].clone();
        if move_result["changed"].as_bool() != Some(true) {
            return Err(MultiplexerError::new(format!(
                "herdr pane move of {pane} changed nothing: {result}"
            )));
        }
        Ok(move_result)
    }

    /// Moves `pane` into a new unfocused tab and returns that temporary tab's ID.
    fn move_to_new_tab(&self, pane: &str) -> Result<String, MultiplexerError> {
        let move_result = self.moved(
            pane,
            &["herdr", "pane", "move", pane, "--new-tab", "--no-focus"],
        )?;
        Ok(str_field(&move_result["created_tab"], "pane move", "tab_id")?.to_owned())
    }

    fn move_into(
        &self,
        pane: &str,
        tab: &str,
        direction: &str,
        target: &str,
    ) -> Result<(), MultiplexerError> {
        self.moved(
            pane,
            &[
                "herdr",
                "pane",
                "move",
                pane,
                "--tab",
                tab,
                "--split",
                direction,
                "--target-pane",
                target,
                "--no-focus",
            ],
        )?;
        Ok(())
    }

    fn read_tab_layout(
        &self,
        anchor_pane_id: &str,
    ) -> Result<(Vec<Value>, Vec<Value>), MultiplexerError> {
        let result =
            self.run_json(
                &self
                    .core
                    .argv(&["herdr", "pane", "layout", "--pane", anchor_pane_id]),
            )?;
        let layout = &result["layout"];
        let panes = layout["panes"]
            .as_array()
            .ok_or_else(|| MultiplexerError::new("herdr pane layout missing 'panes' field"))?;
        let splits = layout["splits"]
            .as_array()
            .ok_or_else(|| MultiplexerError::new("herdr pane layout missing 'splits' field"))?;
        Ok((panes.clone(), splits.clone()))
    }

    /// Equalizes every column of the anchor's tab.
    fn equalize_tab(&self, anchor_pane_id: &str) -> Result<(), MultiplexerError> {
        let (panes, splits) = self.read_tab_layout(anchor_pane_id)?;
        self.equalize_columns(&panes, &splits)
    }

    fn equalize_columns(&self, panes: &[Value], splits: &[Value]) -> Result<(), MultiplexerError> {
        for column in group_columns(panes)? {
            if column.len() >= 2 {
                self.equalize_column(&column, splits)?;
            }
        }
        Ok(())
    }

    /// A column of N stacked panes is a right-leaning chain of `down` splits at the column's x
    /// and width: split_k separates pane_k from the subtree below it, so equal heights ⇔ split_k
    /// has ratio 1/(N-k). `resize --amount` is a signed delta on a split's ratio, so each split is
    /// driven to its target by one arithmetic resize.
    fn equalize_column(&self, column: &[&Value], splits: &[Value]) -> Result<(), MultiplexerError> {
        let n = column.len();
        let col_x = f64_field(&column[0]["rect"], "pane layout", "x")?;
        let col_width = f64_field(&column[0]["rect"], "pane layout", "width")?;
        let mut down_splits: Vec<&Value> = Vec::new();
        for split in splits {
            if split["direction"].as_str() == Some("down")
                && f64_field(&split["rect"], "pane layout", "x")? == col_x
                && f64_field(&split["rect"], "pane layout", "width")? == col_width
            {
                down_splits.push(split);
            }
        }
        sort_by_y(&mut down_splits);
        if down_splits.len() != n - 1 {
            return Ok(());
        }
        for (i, split) in down_splits.iter().enumerate() {
            let ratio = f64_field(split, "pane layout", "ratio")?;
            let delta = round4(1.0 / (n - i) as f64 - ratio);
            if delta.abs() < 1e-3 {
                continue;
            }
            let (pane, direction, amount) = if delta > 0.0 {
                (column[i], "down", delta)
            } else {
                (column[i + 1], "up", -delta)
            };
            let pane_id = str_field(pane, "pane layout", "pane_id")?;
            self.core.run(&self.core.argv(&[
                "herdr",
                "pane",
                "resize",
                "--pane",
                pane_id,
                "--direction",
                direction,
                "--amount",
                &amount.to_string(),
            ]))?;
        }
        Ok(())
    }

    fn resize_after_close(&self, tab_id: &str) -> Result<(), MultiplexerError> {
        let Some(anchor_pane_id) = self.surviving_pane_in_tab(tab_id)? else {
            return Ok(());
        };
        let (panes, splits) = self.read_tab_layout(&anchor_pane_id)?;
        if panes.len() == 1 {
            self.restore_lone_pane_full_width(&panes[0], &splits)
        } else {
            self.equalize_columns(&panes, &splits)
        }
    }

    fn surviving_pane_in_tab(&self, tab_id: &str) -> Result<Option<String>, MultiplexerError> {
        for pane in &self.list_panes()? {
            if pane["tab_id"].as_str() == Some(tab_id) {
                return Ok(Some(str_field(pane, "pane list", "pane_id")?.to_string()));
            }
        }
        Ok(None)
    }

    /// Only exactly one residual `right` split is resized away; any other residue is left alone.
    fn restore_lone_pane_full_width(
        &self,
        pane: &Value,
        splits: &[Value],
    ) -> Result<(), MultiplexerError> {
        if splits.len() != 1 || splits[0]["direction"].as_str() != Some("right") {
            return Ok(());
        }
        let delta = round4(1.0 - f64_field(&splits[0], "pane layout", "ratio")?);
        if delta < 1e-3 {
            return Ok(());
        }
        let pane_id = str_field(pane, "pane layout", "pane_id")?;
        self.core.run(&self.core.argv(&[
            "herdr",
            "pane",
            "resize",
            "--pane",
            pane_id,
            "--direction",
            "right",
            "--amount",
            &delta.to_string(),
        ]))?;
        Ok(())
    }
}

fn current_context(result: &Value) -> Result<MultiplexerContext, MultiplexerError> {
    let pane = &result["pane"];
    Ok(MultiplexerContext {
        session: str_field(pane, "pane current", "workspace_id")?.to_string(),
        window_id: str_field(pane, "pane current", "tab_id")?.to_string(),
        pane_id: str_field(pane, "pane current", "pane_id")?.to_string(),
    })
}

/// The layout's columns (panes sharing `rect.x`) left to right, each sorted top to bottom.
fn group_columns(panes: &[Value]) -> Result<Vec<Vec<&Value>>, MultiplexerError> {
    let mut columns: Vec<(f64, Vec<&Value>)> = Vec::new();
    for pane in panes {
        let x = f64_field(&pane["rect"], "pane layout", "x")?;
        match columns.iter_mut().find(|(column_x, _)| *column_x == x) {
            Some((_, column)) => column.push(pane),
            None => columns.push((x, vec![pane])),
        }
    }
    columns.sort_by(|a, b| a.0.partial_cmp(&b.0).expect("layout rects carry numeric x"));
    Ok(columns
        .into_iter()
        .map(|(_, mut column)| {
            sort_by_y(&mut column);
            column
        })
        .collect())
}

/// [`group_columns`] as pane IDs.
fn columns_of(panes: &[Value]) -> Result<Vec<Vec<String>>, MultiplexerError> {
    group_columns(panes)?
        .into_iter()
        .map(|column| {
            column
                .into_iter()
                .map(|pane| Ok(str_field(pane, "pane layout", "pane_id")?.to_owned()))
                .collect()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::multiplexer::Multiplexer;

    use crate::multiplexer::ColumnTarget;
    use crate::multiplexer::HerdrMultiplexer;
    use crate::multiplexer::PaneCleanup;
    use crate::multiplexer::PaneTarget;
    use crate::multiplexer::RunError;
    use crate::multiplexer::SpawnedPane;
    use crate::multiplexer::layout::ColumnSide;
    use crate::multiplexer::test_support::FakeRunner;
    use crate::multiplexer::test_support::argv;
    use crate::multiplexer::test_support::herdr_envelope;
    use crate::multiplexer::test_support::herdr_error_stderr;
    use std::rc::Rc;

    const SESSION: &str = "work";

    fn on_server(runner: Rc<FakeRunner>) -> HerdrMultiplexer {
        HerdrMultiplexer::for_server(runner, SESSION)
    }

    /// `herdr --session <SESSION> <parts…>`.
    fn herdr(parts: &[&str]) -> Vec<String> {
        let mut command = argv(&["herdr", "--session", SESSION]);
        command.extend(argv(parts));
        command
    }

    fn assert_styled_read(runner: &FakeRunner, pane: &str, depth: &str) {
        let calls = runner.run_argvs();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0][..6], herdr(&["pane", "read", pane]));
        let options = &calls[0][6..];
        assert_eq!(options.len(), 6);
        let options: std::collections::BTreeMap<_, _> = options
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| (pair[0].as_str(), pair[1].as_str()))
            .collect();
        assert_eq!(
            options,
            std::collections::BTreeMap::from([
                ("--format", "ansi"),
                ("--source", "recent-unwrapped"),
                ("--lines", depth),
            ])
        );
    }

    #[test]
    fn structured_reads_reject_non_json_and_missing_result() {
        let runner = FakeRunner::with_binary("herdr");
        runner.respond(Ok("not json".to_string()));
        let err = on_server(runner).pane_context("w1:p1").unwrap_err();
        assert!(
            err.to_string()
                .starts_with("herdr returned non-JSON output:"),
            "got: {err}"
        );

        let runner = FakeRunner::with_binary("herdr");
        runner.respond(Ok(r#"{"id":1}"#.to_string()));
        let err = on_server(runner).pane_context("w1:p1").unwrap_err();
        assert!(
            err.to_string()
                .starts_with("herdr output missing 'result' object:"),
            "got: {err}"
        );
    }

    #[test]
    fn pane_process_reads_the_shell_pid_on_the_recorded_session() {
        let runner = FakeRunner::with_binary("herdr");
        runner.respond(Ok(herdr_envelope(json!({
            "process_info": {"shell_pid": 407363}
        }))));
        assert_eq!(
            on_server(runner.clone()).pane_process("w5:p1").unwrap(),
            Some(407363)
        );
        assert_eq!(
            runner.run_argvs(),
            vec![herdr(&["pane", "process-info", "--pane", "w5:p1"])]
        );
    }

    #[test]
    fn pane_process_of_a_pane_not_found_is_none() {
        let runner = FakeRunner::with_binary("herdr");
        runner.respond(Err(RunError::Failed {
            stderr: herdr_error_stderr("pane_not_found"),
        }));
        assert_eq!(
            on_server(runner.clone()).pane_process("w5:p9").unwrap(),
            None
        );
        assert_eq!(runner.run_argvs().len(), 1);
    }

    #[test]
    fn pane_process_without_an_integer_shell_pid_fails() {
        for result in [
            json!({"process_info": {}}),
            json!({"process_info": {"shell_pid": "407363"}}),
            json!({"process_info": {"shell_pid": null}}),
            json!({}),
        ] {
            let runner = FakeRunner::with_binary("herdr");
            runner.respond(Ok(herdr_envelope(result.clone())));
            assert_eq!(
                on_server(runner)
                    .pane_process("w5:p1")
                    .unwrap_err()
                    .to_string(),
                "herdr pane process-info missing 'shell_pid' field",
                "{result}"
            );
        }
    }

    #[test]
    fn pane_process_propagates_other_command_failures() {
        for failure in [
            RunError::Failed {
                stderr: herdr_error_stderr("server_unavailable"),
            },
            RunError::Timeout,
        ] {
            let runner = FakeRunner::with_binary("herdr");
            runner.respond(Err(failure));
            assert!(on_server(runner).pane_process("w5:p1").is_err());
        }
    }

    #[test]
    fn capture_pane_keeps_crlf_rows_and_drops_the_crlf_blank_tail() {
        let runner = FakeRunner::with_binary("herdr");
        runner.respond(Ok(
            "scrollback\r\nprompt> ls\r\nstatus bar\r\n\r\n \r\n".to_string()
        ));
        let mux = on_server(runner.clone());
        assert_eq!(
            mux.capture_pane("w1:p2", 2).unwrap(),
            "prompt> ls\r\nstatus bar\r"
        );
        assert_styled_read(&runner, "w1:p2", "1002");
    }

    const CWD: &str = "/work/project";
    const TAB: &str = "w1:t1";
    const TEMPORARY_TAB: &str = "w1:t9";

    fn command() -> Vec<String> {
        argv(&[
            "/abs/cafleet",
            "__member-run",
            "--descriptor",
            "/run/d j.json",
        ])
    }

    const COMMAND_LINE: &str = "/abs/cafleet __member-run --descriptor '/run/d j.json'";

    fn column(side: ColumnSide, column_panes: &[&str], other_panes: &[&str]) -> PaneTarget {
        PaneTarget::Column(ColumnTarget {
            window_id: TAB.into(),
            side,
            column_panes: argv(column_panes),
            other_panes: argv(other_panes),
        })
    }

    fn failed(code: &str) -> Result<String, RunError> {
        Err(RunError::Failed {
            stderr: herdr_error_stderr(code),
        })
    }

    fn ok() -> Result<String, RunError> {
        Ok(herdr_envelope(json!({})))
    }

    /// A layout pane at `(x, y)` with the given size.
    fn pane(id: &str, x: i64, y: i64, width: i64, height: i64) -> serde_json::Value {
        json!({"pane_id": id, "rect": {"x": x, "y": y, "width": width, "height": height}})
    }

    fn split(
        direction: &str,
        (x, y, width, height): (i64, i64, i64, i64),
        ratio: f64,
    ) -> serde_json::Value {
        json!({
            "direction": direction,
            "rect": {"x": x, "y": y, "width": width, "height": height},
            "ratio": ratio,
        })
    }

    fn layout(
        panes: Vec<serde_json::Value>,
        splits: Vec<serde_json::Value>,
    ) -> Result<String, RunError> {
        Ok(herdr_envelope(json!({
            "layout": {
                "area": {"x": 0, "y": 0, "width": 120, "height": 60},
                "panes": panes,
                "splits": splits,
            }
        })))
    }

    /// A full-width single column of `ids`, stacked top to bottom.
    fn stacked(ids: &[&str]) -> Vec<serde_json::Value> {
        let height = 60 / ids.len() as i64;
        ids.iter()
            .enumerate()
            .map(|(index, id)| pane(id, 0, index as i64 * height, 120, height))
            .collect()
    }

    fn split_reply(pane_id: &str) -> Result<String, RunError> {
        Ok(herdr_envelope(json!({"pane": {"pane_id": pane_id}})))
    }

    /// herdr 0.9.5's `pane move --new-tab` reply: the new temporary tab sits at
    /// `move_result.created_tab`.
    fn moved_to_new_tab(pane_id: &str, tab: &str) -> Result<String, RunError> {
        Ok(herdr_envelope(json!({
            "type": "pane_move",
            "move_result": {
                "changed": true,
                "pane": {"pane_id": pane_id, "tab_id": tab},
                "previous_tab_id": TAB,
                "previous_pane_id": pane_id,
                "created_tab": {"tab_id": tab, "focused": false, "label": "2"},
                "target_layout": {"tab_id": tab, "panes": [], "splits": []},
            }
        })))
    }

    /// herdr 0.9.5's `pane move --tab <tab>` reply; `closed_tab` is the source tab the move emptied.
    fn moved_into(pane_id: &str, tab: &str, closed_tab: Option<&str>) -> Result<String, RunError> {
        let mut move_result = json!({
            "changed": true,
            "pane": {"pane_id": pane_id, "tab_id": tab},
            "target_layout": {"tab_id": tab, "panes": [], "splits": []},
        });
        if let Some(closed) = closed_tab {
            move_result["closed_tab_id"] = json!(closed);
        }
        Ok(herdr_envelope(
            json!({"type": "pane_move", "move_result": move_result}),
        ))
    }

    /// One reply per `pane move` in `calls`, the last move emptying the temporary tab.
    fn respond_to_moves(runner: &FakeRunner, calls: &[Vec<String>]) {
        let moves: Vec<&Vec<String>> = calls
            .iter()
            .filter(|call| call[3] == "pane" && call[4] == "move")
            .collect();
        for (index, call) in moves.iter().enumerate() {
            let pane = call[5].as_str();
            if call[6] == "--new-tab" {
                runner.respond(moved_to_new_tab(pane, TEMPORARY_TAB));
            } else {
                let last = index + 1 == moves.len();
                runner.respond(moved_into(pane, &call[7], last.then_some(TEMPORARY_TAB)));
            }
        }
    }

    fn split_call(target: &str, direction: &str) -> Vec<String> {
        herdr(&[
            "pane",
            "split",
            target,
            "--direction",
            direction,
            "--no-focus",
            "--cwd",
            CWD,
        ])
    }

    fn move_new_tab(pane: &str) -> Vec<String> {
        herdr(&["pane", "move", pane, "--new-tab", "--no-focus"])
    }

    fn move_into(pane: &str, tab: &str, direction: &str, target: &str) -> Vec<String> {
        herdr(&[
            "pane",
            "move",
            pane,
            "--tab",
            tab,
            "--split",
            direction,
            "--target-pane",
            target,
            "--no-focus",
        ])
    }

    fn resize(pane: &str, direction: &str, amount: &str) -> Vec<String> {
        herdr(&[
            "pane",
            "resize",
            "--pane",
            pane,
            "--direction",
            direction,
            "--amount",
            amount,
        ])
    }

    fn calls_named(runner: &FakeRunner, verb: &str) -> Vec<Vec<String>> {
        runner
            .run_argvs()
            .into_iter()
            .filter(|call| call[3] == "pane" && call[4] == verb)
            .collect()
    }

    mod spawn {
        use crate::multiplexer::herdr::tests::*;

        #[test]
        fn a_column_with_members_splits_its_live_bottom_pane_then_runs_names_and_equalizes() {
            let runner = FakeRunner::with_binary("herdr");
            runner.respond(layout(
                vec![
                    pane("w1:p1", 0, 0, 60, 60),
                    pane("w1:p3", 60, 30, 60, 30),
                    pane("w1:p2", 60, 0, 60, 30),
                ],
                vec![
                    split("right", (0, 0, 120, 60), 0.5),
                    split("down", (60, 0, 60, 60), 0.5),
                ],
            ));
            runner.respond(split_reply("w1:p4"));
            runner.respond(ok());
            runner.respond(ok());
            runner.respond(layout(
                vec![
                    pane("w1:p1", 0, 0, 60, 60),
                    pane("w1:p2", 60, 0, 60, 30),
                    pane("w1:p3", 60, 30, 60, 15),
                    pane("w1:p4", 60, 45, 60, 15),
                ],
                vec![
                    split("right", (0, 0, 120, 60), 0.5),
                    split("down", (60, 0, 60, 60), 0.5),
                    split("down", (60, 30, 60, 30), 0.5),
                ],
            ));
            runner.respond(ok());
            let spawned = on_server(runner.clone())
                .spawn_member_pane(
                    &column(ColumnSide::Right, &["w1:p2", "w1:p3"], &["w1:p1"]),
                    "drafter",
                    CWD,
                    &command(),
                )
                .unwrap();
            assert_eq!(
                spawned,
                SpawnedPane {
                    pane_id: "w1:p4".into(),
                    window_id: TAB.into(),
                }
            );
            assert_eq!(
                runner.run_argvs(),
                vec![
                    herdr(&["pane", "layout", "--pane", "w1:p2"]),
                    split_call("w1:p3", "down"),
                    herdr(&["pane", "run", "w1:p4", COMMAND_LINE]),
                    herdr(&["pane", "rename", "w1:p4", "drafter"]),
                    herdr(&["pane", "layout", "--pane", "w1:p4"]),
                    resize("w1:p3", "up", "0.1667"),
                ],
                "the bottom pane comes from the live layout, not the registry order"
            );
        }

        #[test]
        fn the_director_windows_first_member_splits_the_lone_left_pane_rightward() {
            let runner = FakeRunner::with_binary("herdr");
            runner.respond(layout(vec![pane("w1:p1", 0, 0, 120, 60)], vec![]));
            runner.respond(split_reply("w1:p2"));
            runner.respond(ok());
            runner.respond(ok());
            runner.respond(layout(
                vec![pane("w1:p1", 0, 0, 60, 60), pane("w1:p2", 60, 0, 60, 60)],
                vec![split("right", (0, 0, 120, 60), 0.5)],
            ));
            let spawned = on_server(runner.clone())
                .spawn_member_pane(
                    &column(ColumnSide::Right, &[], &["w1:p1"]),
                    "drafter",
                    CWD,
                    &command(),
                )
                .unwrap();
            assert_eq!(spawned.pane_id, "w1:p2");
            assert_eq!(
                runner.run_argvs(),
                vec![
                    herdr(&["pane", "layout", "--pane", "w1:p1"]),
                    split_call("w1:p1", "right"),
                    herdr(&["pane", "run", "w1:p2", COMMAND_LINE]),
                    herdr(&["pane", "rename", "w1:p2", "drafter"]),
                    herdr(&["pane", "layout", "--pane", "w1:p2"]),
                ]
            );
        }

        fn right_reparenting_calls(left: &[&str], new_pane: &str) -> Vec<Vec<String>> {
            let mut calls = vec![
                herdr(&["pane", "layout", "--pane", left[0]]),
                split_call(left[0], "right"),
                herdr(&["pane", "run", new_pane, COMMAND_LINE]),
                herdr(&["pane", "rename", new_pane, "worker"]),
                move_new_tab(left[1]),
            ];
            for i in 2..left.len() {
                calls.push(move_into(left[i], TEMPORARY_TAB, "down", left[i - 1]));
            }
            for i in 1..left.len() {
                calls.push(move_into(left[i], TAB, "down", left[i - 1]));
            }
            calls.push(herdr(&["pane", "layout", "--pane", new_pane]));
            calls
        }

        #[test]
        fn an_empty_right_column_beside_a_stacked_left_column_rebuilds_the_left_chain() {
            for left in [
                &["w1:p1", "w1:p2"][..],
                &["w1:p1", "w1:p2", "w1:p3", "w1:p4"][..],
            ] {
                let runner = FakeRunner::with_binary("herdr");
                let mut shuffled = stacked(left);
                shuffled.reverse();
                runner.respond(layout(shuffled, vec![]));
                runner.respond(split_reply("w1:p9"));
                runner.respond(ok());
                runner.respond(ok());
                let expected = right_reparenting_calls(left, "w1:p9");
                respond_to_moves(&runner, &expected);
                runner.respond(failed("layout_unavailable"));
                let spawned = on_server(runner.clone())
                    .spawn_member_pane(
                        &column(ColumnSide::Right, &[], &[left[0]]),
                        "worker",
                        CWD,
                        &command(),
                    )
                    .unwrap();
                assert_eq!(
                    spawned,
                    SpawnedPane {
                        pane_id: "w1:p9".into(),
                        window_id: TAB.into(),
                    }
                );
                assert_eq!(runner.run_argvs(), expected, "k = {}", left.len());
            }
        }

        fn left_reparenting_calls(right: &[&str], new_pane: &str) -> Vec<Vec<String>> {
            let mut calls = vec![
                herdr(&["pane", "layout", "--pane", right[0]]),
                split_call(right[0], "right"),
                herdr(&["pane", "run", new_pane, COMMAND_LINE]),
                herdr(&["pane", "rename", new_pane, "worker"]),
                move_new_tab(right[0]),
            ];
            for i in 1..right.len() {
                calls.push(move_into(right[i], TEMPORARY_TAB, "down", right[i - 1]));
            }
            calls.push(move_into(right[0], TAB, "right", new_pane));
            for i in 1..right.len() {
                calls.push(move_into(right[i], TAB, "down", right[i - 1]));
            }
            calls.push(herdr(&["pane", "layout", "--pane", new_pane]));
            calls
        }

        #[test]
        fn an_empty_left_column_moves_the_right_chain_beside_the_new_pane() {
            for right in [&["w1:p5"][..], &["w1:p5", "w1:p6", "w1:p7"][..]] {
                let runner = FakeRunner::with_binary("herdr");
                let mut shuffled = stacked(right);
                shuffled.reverse();
                runner.respond(layout(shuffled, vec![]));
                runner.respond(split_reply("w1:p9"));
                runner.respond(ok());
                runner.respond(ok());
                let expected = left_reparenting_calls(right, "w1:p9");
                respond_to_moves(&runner, &expected);
                runner.respond(failed("layout_unavailable"));
                let spawned = on_server(runner.clone())
                    .spawn_member_pane(
                        &column(ColumnSide::Left, &[], &[right[0]]),
                        "worker",
                        CWD,
                        &command(),
                    )
                    .unwrap();
                assert_eq!(spawned.pane_id, "w1:p9");
                assert_eq!(spawned.window_id, TAB);
                assert_eq!(runner.run_argvs(), expected, "m = {}", right.len());
            }
        }

        #[test]
        fn a_new_window_is_a_detached_labelled_tab_whose_root_pane_is_the_member() {
            let runner = FakeRunner::with_binary("herdr");
            runner.respond(Ok(herdr_envelope(json!({
                "tab": {"tab_id": "w1:t5", "label": "cafleet-3-1"},
                "root_pane": {"pane_id": "w1:p20"},
            }))));
            runner.respond(ok());
            runner.respond(ok());
            runner.respond(layout(vec![pane("w1:p20", 0, 0, 120, 60)], vec![]));
            let spawned = on_server(runner.clone())
                .spawn_member_pane(
                    &PaneTarget::NewWindow {
                        session: "w1".into(),
                        window_name: "cafleet-3-1".into(),
                    },
                    "tester",
                    CWD,
                    &command(),
                )
                .unwrap();
            assert_eq!(
                spawned,
                SpawnedPane {
                    pane_id: "w1:p20".into(),
                    window_id: "w1:t5".into(),
                }
            );
            assert_eq!(
                runner.run_argvs(),
                vec![
                    herdr(&[
                        "tab",
                        "create",
                        "--workspace",
                        "w1",
                        "--cwd",
                        CWD,
                        "--label",
                        "cafleet-3-1",
                        "--no-focus",
                    ]),
                    herdr(&["pane", "run", "w1:p20", COMMAND_LINE]),
                    herdr(&["pane", "rename", "w1:p20", "tester"]),
                    herdr(&["pane", "layout", "--pane", "w1:p20"]),
                ]
            );
        }

        #[test]
        fn a_tab_create_reply_without_its_ids_leaves_pane_cleanup_unknown() {
            for reply in [
                Ok(herdr_envelope(json!({"tab": {"tab_id": "w1:t5"}}))),
                Ok(herdr_envelope(json!({"root_pane": {"pane_id": "w1:p20"}}))),
                Ok(herdr_envelope(json!({"tab": {}, "root_pane": {}}))),
            ] {
                let runner = FakeRunner::with_binary("herdr");
                runner.respond(reply);
                let error = on_server(runner.clone())
                    .spawn_member_pane(
                        &PaneTarget::NewWindow {
                            session: "w1".into(),
                            window_name: "cafleet-3-1".into(),
                        },
                        "tester",
                        CWD,
                        &command(),
                    )
                    .unwrap_err();
                assert_eq!(error.pane_cleanup(), Some(&PaneCleanup::Unknown));
                let message = error.to_string();
                assert!(
                    message.starts_with("herdr tab create missing '")
                        && message.contains("' field"),
                    "{message}"
                );
                assert_eq!(runner.run_argvs().len(), 1);
            }
        }

        #[test]
        fn a_failed_tab_create_leaves_pane_cleanup_unknown() {
            for reply in [Err(RunError::Timeout), failed("workspace_not_found")] {
                let runner = FakeRunner::with_binary("herdr");
                runner.respond(reply);
                let error = on_server(runner.clone())
                    .spawn_member_pane(
                        &PaneTarget::NewWindow {
                            session: "w1".into(),
                            window_name: "cafleet-3-1".into(),
                        },
                        "tester",
                        CWD,
                        &command(),
                    )
                    .unwrap_err();
                assert_eq!(error.pane_cleanup(), Some(&PaneCleanup::Unknown));
                assert_eq!(runner.run_argvs().len(), 1);
            }
        }

        #[test]
        fn an_unknown_split_id_keeps_unowned_panes_untouched() {
            for reply in [
                Ok(herdr_envelope(json!({"pane": {}}))),
                Ok(herdr_envelope(json!({"pane": {"pane_id": " "}}))),
                Err(RunError::Timeout),
            ] {
                let runner = FakeRunner::with_binary("herdr");
                runner.respond(layout(vec![pane("w1:p1", 0, 0, 120, 60)], vec![]));
                runner.respond(reply);
                let error = on_server(runner.clone())
                    .spawn_member_pane(
                        &column(ColumnSide::Right, &[], &["w1:p1"]),
                        "worker",
                        CWD,
                        &command(),
                    )
                    .unwrap_err();
                assert_eq!(error.pane_cleanup(), Some(&PaneCleanup::Unknown));
                assert_eq!(runner.run_argvs().len(), 2);
            }
        }

        #[test]
        fn a_failed_pane_run_closes_only_the_new_pane_and_moves_nothing() {
            for close_fails in [false, true] {
                let runner = FakeRunner::with_binary("herdr");
                runner.respond(layout(stacked(&["w1:p5", "w1:p6"]), vec![]));
                runner.respond(split_reply("w1:p9"));
                runner.respond(Err(RunError::Failed {
                    stderr: "primary run failure".into(),
                }));
                runner.respond(if close_fails {
                    Err(RunError::Failed {
                        stderr: "close failed".into(),
                    })
                } else {
                    Ok(String::new())
                });
                runner.respond(failed("server_unavailable"));
                runner.respond(failed("server_unavailable"));
                let error = on_server(runner.clone())
                    .spawn_member_pane(
                        &column(ColumnSide::Left, &[], &["w1:p5"]),
                        "worker",
                        CWD,
                        &command(),
                    )
                    .unwrap_err();
                assert!(error.to_string().contains("primary run failure"));
                assert!(
                    matches!(
                        error.pane_cleanup(),
                        Some(PaneCleanup::Attempted { pane_id, error })
                            if pane_id == "w1:p9" && error.is_some() == close_fails
                    ),
                    "{:?}",
                    error.pane_cleanup()
                );
                assert_eq!(
                    calls_named(&runner, "close"),
                    vec![herdr(&["pane", "close", "w1:p9"])]
                );
                assert!(calls_named(&runner, "move").is_empty());
                assert!(calls_named(&runner, "rename").is_empty());
            }
        }

        #[test]
        fn a_failed_move_stops_re_parenting_and_keeps_the_spawn() {
            let runner = FakeRunner::with_binary("herdr");
            runner.respond(layout(stacked(&["w1:p5", "w1:p6", "w1:p7"]), vec![]));
            runner.respond(split_reply("w1:p9"));
            runner.respond(ok());
            runner.respond(ok());
            runner.respond(moved_to_new_tab("w1:p5", TEMPORARY_TAB));
            runner.respond(Err(RunError::Failed {
                stderr: serde_json::json!({"error": {"code": "target_pane_not_found", "message": "target pane w1:p5 not found"}, "id": 1}).to_string(),
            }));
            runner.respond(failed("layout_unavailable"));
            let spawned = on_server(runner.clone())
                .spawn_member_pane(
                    &column(ColumnSide::Left, &[], &["w1:p5"]),
                    "worker",
                    CWD,
                    &command(),
                )
                .unwrap();
            assert_eq!(spawned.pane_id, "w1:p9");
            assert_eq!(
                calls_named(&runner, "move"),
                vec![
                    move_new_tab("w1:p5"),
                    move_into("w1:p6", TEMPORARY_TAB, "down", "w1:p5"),
                ],
                "the remaining panes stay where they are"
            );
            assert!(calls_named(&runner, "close").is_empty());
        }

        #[test]
        fn naming_and_equalization_failures_never_fail_the_spawn() {
            let runner = FakeRunner::with_binary("herdr");
            runner.respond(layout(vec![pane("w1:p1", 0, 0, 120, 60)], vec![]));
            runner.respond(split_reply("w1:p2"));
            runner.respond(ok());
            runner.respond(failed("pane_not_found"));
            runner.respond(failed("layout_unavailable"));
            let spawned = on_server(runner.clone())
                .spawn_member_pane(
                    &column(ColumnSide::Right, &[], &["w1:p1"]),
                    "drafter",
                    CWD,
                    &command(),
                )
                .unwrap();
            assert_eq!(spawned.pane_id, "w1:p2");
            assert!(calls_named(&runner, "close").is_empty());
        }
    }

    mod equalize {
        use crate::multiplexer::herdr::tests::*;

        fn two_columns() -> Result<String, RunError> {
            layout(
                vec![
                    pane("w1:pe", 60, 30, 60, 30),
                    pane("w1:pa", 0, 0, 60, 20),
                    pane("w1:pc", 0, 40, 60, 20),
                    pane("w1:pd", 60, 0, 60, 30),
                    pane("w1:pb", 0, 20, 60, 20),
                ],
                vec![
                    split("right", (0, 0, 120, 60), 0.5),
                    split("down", (0, 20, 60, 40), 0.5),
                    split("down", (0, 0, 60, 60), 0.5),
                    split("down", (60, 0, 60, 60), 0.7),
                ],
            )
        }

        #[test]
        fn restoration_equalizes_every_column_of_the_recorded_tab() {
            let runner = FakeRunner::with_binary("herdr");
            runner.respond(Ok(String::new()));
            let mux = on_server(runner.clone());
            mux.close_pane("w1:px", TAB).unwrap();
            assert_eq!(
                runner.run_argvs(),
                vec![herdr(&["pane", "close", "w1:px"])],
                "the close reads no pre-close tab"
            );
            runner.respond(Ok(herdr_envelope(json!({"panes": [
                {"pane_id": "w2:p1", "tab_id": "w2:t1"},
                {"pane_id": "w1:pd", "tab_id": TAB},
                {"pane_id": "w1:pa", "tab_id": TAB},
            ]}))));
            runner.respond(two_columns());
            runner.succeed(2);
            mux.restore_after_close().unwrap();
            let calls = runner.run_argvs();
            assert_eq!(calls[1], herdr(&["pane", "list"]));
            assert_eq!(calls[2], herdr(&["pane", "layout", "--pane", "w1:pd"]));
            let mut resizes = calls[3..].to_vec();
            resizes.sort();
            let mut expected = vec![
                resize("w1:pb", "up", "0.1667"),
                resize("w1:pe", "up", "0.2"),
            ];
            expected.sort();
            assert_eq!(resizes, expected);
        }

        #[test]
        fn a_column_without_its_own_down_chain_is_left_untouched() {
            let runner = FakeRunner::with_binary("herdr");
            runner.respond(Ok(String::new()));
            let mux = on_server(runner.clone());
            mux.close_pane("w1:px", TAB).unwrap();
            runner.respond(Ok(herdr_envelope(
                json!({"panes": [{"pane_id": "w1:pa", "tab_id": TAB}]}),
            )));
            runner.respond(layout(
                vec![
                    pane("w1:pa", 0, 0, 60, 60),
                    pane("w1:pd", 60, 0, 60, 30),
                    pane("w1:pe", 60, 30, 60, 30),
                ],
                vec![
                    split("right", (0, 0, 120, 60), 0.5),
                    split("down", (60, 0, 120, 60), 0.7),
                ],
            ));
            mux.restore_after_close().unwrap();
            assert_eq!(
                runner.run_argvs().len(),
                3,
                "a split of another width never matches"
            );
        }
    }

    mod close {
        use crate::multiplexer::herdr::tests::*;

        #[test]
        fn closing_the_last_pane_of_a_tab_leaves_nothing_to_restore() {
            let runner = FakeRunner::with_binary("herdr");
            runner.respond(Ok(String::new()));
            runner.respond(Ok(herdr_envelope(json!({"panes": [
                {"pane_id": "w1:p1", "tab_id": "w1:t0"},
            ]}))));
            on_server(runner.clone())
                .kill_pane("w1:p20", "w1:t5")
                .unwrap();
            assert_eq!(
                runner.run_argvs(),
                vec![
                    herdr(&["pane", "close", "w1:p20"]),
                    herdr(&["pane", "list"])
                ]
            );
        }

        #[test]
        fn a_lone_pane_with_one_residual_right_split_is_restored_to_full_width() {
            let runner = FakeRunner::with_binary("herdr");
            runner.respond(Ok(String::new()));
            runner.respond(Ok(herdr_envelope(
                json!({"panes": [{"pane_id": "w1:p1", "tab_id": TAB}]}),
            )));
            runner.respond(layout(
                vec![pane("w1:p1", 0, 0, 72, 60)],
                vec![split("right", (0, 0, 120, 60), 0.6)],
            ));
            runner.succeed(1);
            on_server(runner.clone()).kill_pane("w1:p2", TAB).unwrap();
            assert_eq!(
                runner.run_argvs(),
                vec![
                    herdr(&["pane", "close", "w1:p2"]),
                    herdr(&["pane", "list"]),
                    herdr(&["pane", "layout", "--pane", "w1:p1"]),
                    resize("w1:p1", "right", "0.4"),
                ]
            );
        }

        #[test]
        fn a_lone_pane_without_residual_splits_needs_no_resize() {
            let runner = FakeRunner::with_binary("herdr");
            runner.respond(Ok(String::new()));
            runner.respond(Ok(herdr_envelope(
                json!({"panes": [{"pane_id": "w1:p1", "tab_id": TAB}]}),
            )));
            runner.respond(layout(vec![pane("w1:p1", 0, 0, 120, 60)], vec![]));
            on_server(runner.clone()).kill_pane("w1:p2", TAB).unwrap();
            assert_eq!(runner.run_argvs().len(), 3);
        }

        #[test]
        fn kill_pane_tolerates_a_pane_gone_close_and_best_effort_restoration() {
            let runner = FakeRunner::with_binary("herdr");
            runner.respond(failed("pane_not_found"));
            runner.respond(failed("server_unavailable"));
            on_server(runner.clone()).kill_pane("w1:p2", TAB).unwrap();
            assert_eq!(runner.run_argvs()[0], herdr(&["pane", "close", "w1:p2"]));
        }

        #[test]
        fn a_close_failure_other_than_pane_gone_fails() {
            let refused = |close: &dyn Fn(&HerdrMultiplexer) -> bool| {
                let runner = FakeRunner::with_binary("herdr");
                runner.respond(failed("server_unavailable"));
                assert!(close(&on_server(runner.clone())));
                assert_eq!(runner.run_argvs().len(), 1);
            };
            refused(&|mux| mux.kill_pane("w1:p2", TAB).is_err());
            refused(&|mux| mux.close_pane("w1:p2", TAB).is_err());
        }

        #[test]
        fn restoration_without_a_prior_close_does_nothing() {
            let runner = FakeRunner::without_binaries();
            on_server(runner.clone()).restore_after_close().unwrap();
            assert!(runner.events().is_empty());
        }
    }

    mod window_reads {
        use crate::multiplexer::herdr::tests::*;

        fn height(value: i64) -> Result<String, RunError> {
            Ok(herdr_envelope(json!({
                "layout": {
                    "area": {"x": 0, "y": 0, "width": 200, "height": value},
                    "panes": [],
                    "splits": [],
                }
            })))
        }

        #[test]
        fn window_height_reads_the_layout_area_of_the_first_anchor() {
            let runner = FakeRunner::with_binary("herdr");
            runner.respond(height(57));
            assert_eq!(
                on_server(runner.clone())
                    .window_height(TAB, &argv(&["w1:p2", "w1:p3"]))
                    .unwrap(),
                57
            );
            assert_eq!(
                runner.run_argvs(),
                vec![herdr(&["pane", "layout", "--pane", "w1:p2"])]
            );
        }

        #[test]
        fn window_height_moves_past_anchors_that_fail() {
            let runner = FakeRunner::with_binary("herdr");
            runner.respond(failed("pane_not_found"));
            runner.respond(failed("server_unavailable"));
            runner.respond(height(122));
            assert_eq!(
                on_server(runner.clone())
                    .window_height(TAB, &argv(&["w1:p2", "w1:p3", "w1:p4"]))
                    .unwrap(),
                122
            );
            assert_eq!(
                runner.run_argvs(),
                vec![
                    herdr(&["pane", "layout", "--pane", "w1:p2"]),
                    herdr(&["pane", "layout", "--pane", "w1:p3"]),
                    herdr(&["pane", "layout", "--pane", "w1:p4"]),
                ]
            );
        }

        #[test]
        fn window_height_fails_with_the_last_anchor_error_only_when_every_anchor_fails() {
            let runner = FakeRunner::with_binary("herdr");
            runner.respond(failed("server_unavailable"));
            runner.respond(failed("pane_not_found"));
            let error = on_server(runner.clone())
                .window_height(TAB, &argv(&["w1:p2", "w1:p3"]))
                .unwrap_err()
                .to_string();
            assert!(error.contains("pane_not_found"), "{error}");
            assert!(error.contains("w1:p3"), "{error}");
            assert!(!error.contains("server_unavailable"), "{error}");
            assert_eq!(runner.run_argvs().len(), 2);
        }

        #[test]
        fn window_height_without_a_layout_area_fails() {
            let runner = FakeRunner::with_binary("herdr");
            runner.respond(Ok(herdr_envelope(
                json!({"layout": {"panes": [], "splits": []}}),
            )));
            assert_eq!(
                on_server(runner)
                    .window_height(TAB, &argv(&["w1:p2"]))
                    .unwrap_err()
                    .to_string(),
                "herdr pane layout missing 'area' field"
            );
        }

        #[test]
        fn window_name_reads_the_tab_label() {
            for label in ["agents", ""] {
                let runner = FakeRunner::with_binary("herdr");
                runner.respond(Ok(herdr_envelope(
                    json!({"tab": {"tab_id": TAB, "label": label}}),
                )));
                assert_eq!(on_server(runner.clone()).window_name(TAB).unwrap(), label);
                assert_eq!(runner.run_argvs(), vec![herdr(&["tab", "get", TAB])]);
            }
        }

        #[test]
        fn window_name_rejects_a_reply_without_a_label() {
            let runner = FakeRunner::with_binary("herdr");
            runner.respond(Ok(herdr_envelope(json!({"tab": {"tab_id": TAB}}))));
            assert_eq!(
                on_server(runner).window_name(TAB).unwrap_err().to_string(),
                "herdr tab get missing 'label' field"
            );
        }

        #[test]
        fn window_name_propagates_a_missing_tab() {
            let runner = FakeRunner::with_binary("herdr");
            runner.respond(failed("tab_not_found"));
            let error = on_server(runner).window_name(TAB).unwrap_err().to_string();
            assert!(error.contains("tab_not_found"), "{error}");
        }
    }
}
