use std::collections::HashMap;
use std::rc::Rc;

use serde_json::Value;

use super::{
    BackendCore, CommandRunner, Lead, MultiplexerContext, MultiplexerError, PLACEMENT_MISMATCH,
    PaneCleanup, PaneInput, PaneOwnership,
};

const PANE_NOT_FOUND: &str = "pane_not_found";

fn is_pane_not_found(stderr: &str) -> bool {
    error_code(stderr).as_deref() == Some(PANE_NOT_FOUND)
}

fn herdr_argv(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|part| part.to_string()).collect()
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
}

impl HerdrMultiplexer {
    pub(crate) fn new(runner: Rc<dyn CommandRunner>, env: HashMap<String, String>) -> Self {
        HerdrMultiplexer {
            core: BackendCore {
                runner,
                env,
                name: "herdr",
                session_variable: "HERDR_ENV",
                pane_gone: is_pane_not_found,
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

    fn run(&self, argv: &[String], timeout_secs: Option<u64>) -> Result<String, MultiplexerError> {
        self.core.run(argv, timeout_secs)
    }

    fn run_json(
        &self,
        argv: &[String],
        timeout_secs: Option<u64>,
    ) -> Result<Value, MultiplexerError> {
        let out = self.run(argv, timeout_secs)?;
        parse_envelope(&out)
    }

    fn list_panes(&self) -> Result<Vec<Value>, MultiplexerError> {
        let result = self.run_json(&herdr_argv(&["herdr", "pane", "list"]), None)?;
        panes_of(&result).cloned()
    }

    pub(crate) fn ensure_available(&self) -> Result<(), MultiplexerError> {
        self.core.ensure_available()
    }

    pub(crate) fn validate_key_target(
        &self,
        placement: &crate::broker::records::Placement,
        deadline: crate::input::Deadline,
    ) -> Result<(), MultiplexerError> {
        super::check_deadline(deadline)?;
        self.ensure_available()?;
        super::check_deadline(deadline)?;
        let current = parse_envelope(&self.core.press_run(
            &herdr_argv(&["herdr", "pane", "current"]),
            deadline,
            false,
        )?)?;
        let pane = &current["pane"];
        let session = str_field(pane, "pane current", "workspace_id")?;
        str_field(pane, "pane current", "tab_id")?;
        str_field(pane, "pane current", "pane_id")?;
        if session != placement.mux_session {
            return Err(MultiplexerError::new(PLACEMENT_MISMATCH));
        }
        let target = placement
            .mux_pane_id
            .as_deref()
            .ok_or_else(|| MultiplexerError::new("press target has no pane"))?;
        let result = parse_envelope(&self.core.press_run(
            &herdr_argv(&["herdr", "pane", "list"]),
            deadline,
            false,
        )?)?;
        let mut matches = 0;
        for pane in panes_of(&result)? {
            let pane_id = str_field(pane, "pane list", "pane_id")?;
            let workspace = str_field(pane, "pane list", "workspace_id")?;
            let tab = str_field(pane, "pane list", "tab_id")?;
            if pane_id == target {
                if workspace != placement.mux_session || tab != placement.mux_window_id {
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

    pub(crate) fn send_keys(
        &self,
        pane_id: &str,
        keys: &super::PressKeys,
        deadline: crate::input::Deadline,
    ) -> Result<(), MultiplexerError> {
        let mut argv = herdr_argv(&["herdr", "pane", "send-keys", pane_id]);
        argv.extend(keys.herdr_arguments());
        self.core.send_keys(&argv, deadline)
    }

    pub(crate) fn context_discovery(&self) -> Result<MultiplexerContext, MultiplexerError> {
        let result = self.run_json(&herdr_argv(&["herdr", "pane", "current"]), None)?;
        let pane = &result["pane"];
        Ok(MultiplexerContext {
            session: str_field(pane, "pane current", "workspace_id")?.to_string(),
            window_id: str_field(pane, "pane current", "tab_id")?.to_string(),
            pane_id: str_field(pane, "pane current", "pane_id")?.to_string(),
        })
    }

    /// Emulate tmux main-vertical: the first member splits the Director pane
    /// rightward; later members split the column's max pane downward, then the
    /// column is rebalanced to equal heights (best-effort, anchored on the
    /// Director's own pane).
    pub(crate) fn split_window(
        &self,
        reference: &MultiplexerContext,
        env: &[(String, String)],
        command: &[String],
    ) -> Result<String, MultiplexerError> {
        let cwd = std::env::current_dir().map_err(|e| {
            MultiplexerError::new(format!(
                "cannot resolve the working directory for pane spawn: {e}"
            ))
        })?;
        let cwd = cwd.display().to_string();
        let panes = self.list_panes()?;
        let mut column: Vec<String> = Vec::new();
        for pane in &panes {
            let pane_id = str_field(pane, "pane list", "pane_id")?;
            if pane["tab_id"].as_str() == Some(&reference.window_id) && pane_id != reference.pane_id
            {
                column.push(pane_id.to_string());
            }
        }
        let new_pane_id = match column.iter().max() {
            None => self.split_pane(&reference.pane_id, "right", &cwd, env)?,
            Some(target) => self.split_pane(target, "down", &cwd, env)?,
        };
        let mut pane = PaneOwnership::new(new_pane_id.clone(), |id| self.kill_pane(id, true));
        let result: Result<(), MultiplexerError> = (|| {
            if !column.is_empty() {
                let _ = self.resize_tab_column(&reference.pane_id);
            }
            self.run(
                &herdr_argv(&["herdr", "pane", "run", &new_pane_id, &shlex_join(command)]),
                None,
            )?;
            Ok(())
        })();
        if let Err(error) = result {
            return Err(error.with_pane_cleanup(pane.rollback()));
        }
        Ok(pane.finish())
    }

    fn split_pane(
        &self,
        pane_id: &str,
        direction: &str,
        cwd: &str,
        env: &[(String, String)],
    ) -> Result<String, MultiplexerError> {
        let mut args = herdr_argv(&[
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
        for (key, value) in env {
            args.push("--env".to_string());
            args.push(format!("{key}={value}"));
        }
        let result = self
            .run(&args, None)
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

    fn read_tab_layout(
        &self,
        anchor_pane_id: &str,
    ) -> Result<(Vec<Value>, Vec<Value>), MultiplexerError> {
        let result = self.run_json(
            &herdr_argv(&["herdr", "pane", "layout", "--pane", anchor_pane_id]),
            None,
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

    /// Rebalance the members' right column on the anchor's tab to equal
    /// heights — the tmux `select-layout main-vertical` reflow herdr has no
    /// single command for.
    fn resize_tab_column(&self, anchor_pane_id: &str) -> Result<(), MultiplexerError> {
        let (panes, splits) = self.read_tab_layout(anchor_pane_id)?;
        let column = right_column(&panes)?;
        if column.len() < 2 {
            return Ok(());
        }
        self.equalize_column(&column, &splits)
    }

    /// A right column of N stacked members is a right-leaning chain of `down`
    /// splits: split_k separates member_k from the subtree below it, so equal
    /// heights ⇔ split_k has ratio 1/(N-k). `resize --amount` is a signed
    /// delta on a split's ratio, so each split is driven to its target by one
    /// arithmetic resize.
    fn equalize_column(&self, column: &[Value], splits: &[Value]) -> Result<(), MultiplexerError> {
        let n = column.len();
        let col_x = f64_field(&column[0]["rect"], "pane layout", "x")?;
        let mut down_splits: Vec<&Value> = Vec::new();
        for split in splits {
            if split["direction"].as_str() == Some("down")
                && f64_field(&split["rect"], "pane layout", "x")? == col_x
            {
                down_splits.push(split);
            }
        }
        sort_by_y(&mut down_splits);
        if down_splits.len() != n - 1 {
            return Ok(()); // not the expected right-leaning chain — leave it untouched
        }
        for (i, split) in down_splits.iter().enumerate() {
            let ratio = f64_field(split, "pane layout", "ratio")?;
            let delta = round4(1.0 / (n - i) as f64 - ratio);
            if delta.abs() < 1e-3 {
                continue;
            }
            let (pane, direction, amount) = if delta > 0.0 {
                (&column[i], "down", delta)
            } else {
                (&column[i + 1], "up", -delta)
            };
            let pane_id = str_field(pane, "pane layout", "pane_id")?;
            self.run(
                &herdr_argv(&[
                    "herdr",
                    "pane",
                    "resize",
                    "--pane",
                    pane_id,
                    "--direction",
                    direction,
                    "--amount",
                    &amount.to_string(),
                ]),
                None,
            )?;
        }
        Ok(())
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
                escape: herdr_argv(&["herdr", "pane", "send-keys", target_pane_id, "escape"]),
                literal: herdr_argv(&["herdr", "pane", "send-text", target_pane_id, text]),
                enter: herdr_argv(&["herdr", "pane", "send-keys", target_pane_id, "enter"]),
            },
            lead,
        )
    }

    /// `herdr pane read` prints the raw terminal buffer (no envelope); the
    /// shared capture windowing ([`super::capture_window`]) enforces the last-N
    /// window client-side — the daemon may return more rows than requested.
    pub(crate) fn capture_pane(
        &self,
        target_pane_id: &str,
        lines: i64,
    ) -> Result<String, MultiplexerError> {
        super::check_capture_lines(lines)?;
        let fetch_depth = lines + super::CAPTURE_OVER_FETCH_LINES;
        let raw = self.run(
            &herdr_argv(&[
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
        if let Some(tab_id) = self.close_pane(target_pane_id, ignore_missing)? {
            let _ = self.resize_after_close(&tab_id);
        }
        Ok(())
    }

    pub(crate) fn close_pane(
        &self,
        target_pane_id: &str,
        ignore_missing: bool,
    ) -> Result<Option<String>, MultiplexerError> {
        let target_tab_id = self.pane_tab_id(target_pane_id);
        self.core.run_tolerating_missing(
            &herdr_argv(&["herdr", "pane", "close", target_pane_id]),
            ignore_missing,
            None,
        )?;
        Ok(target_tab_id)
    }

    /// Best-effort pre-close read of the target pane's tab; `None` (the
    /// rebalance skips) when the pane is already gone or the read fails — the
    /// close must proceed regardless.
    fn pane_tab_id(&self, pane_id: &str) -> Option<String> {
        let result = self
            .run_json(&herdr_argv(&["herdr", "pane", "get", pane_id]), None)
            .ok()?;
        result["pane"]["tab_id"].as_str().map(str::to_string)
    }

    pub(crate) fn resize_after_close(&self, target_tab_id: &str) -> Result<(), MultiplexerError> {
        let Some(anchor_pane_id) = self.surviving_pane_in_tab(target_tab_id)? else {
            return Ok(()); // the tab has no panes left — nothing to rebalance
        };
        let (panes, splits) = self.read_tab_layout(&anchor_pane_id)?;
        if panes.is_empty() {
            return Ok(());
        }
        let column = right_column(&panes)?;
        if column.len() >= 2 {
            self.equalize_column(&column, &splits)
        } else if column.is_empty() {
            self.restore_director_full_width(&panes, &splits)
        } else {
            // A lone member pane spans the column natively; the right split's
            // ratio is unaffected by a down-close.
            Ok(())
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

    fn restore_director_full_width(
        &self,
        panes: &[Value],
        splits: &[Value],
    ) -> Result<(), MultiplexerError> {
        if panes.len() != 1 {
            return Ok(()); // not the single-Director shape — leave it untouched
        }
        // Empty `splits` ⇒ the sole pane is already structurally full-width;
        // any residue other than exactly one right split is anomalous.
        if splits.len() != 1 || splits[0]["direction"].as_str() != Some("right") {
            return Ok(());
        }
        let delta = round4(1.0 - f64_field(&splits[0], "pane layout", "ratio")?);
        if delta < 1e-3 {
            return Ok(());
        }
        let pane_id = str_field(&panes[0], "pane layout", "pane_id")?;
        self.run(
            &herdr_argv(&[
                "herdr",
                "pane",
                "resize",
                "--pane",
                pane_id,
                "--direction",
                "right",
                "--amount",
                &delta.to_string(),
            ]),
            None,
        )?;
        Ok(())
    }
}

/// The members' right column: every pane outside the leftmost (Director)
/// column, sorted top to bottom.
fn right_column(panes: &[Value]) -> Result<Vec<Value>, MultiplexerError> {
    let mut min_x = f64::INFINITY;
    for pane in panes {
        min_x = min_x.min(f64_field(&pane["rect"], "pane layout", "x")?);
    }
    let mut column: Vec<&Value> = Vec::new();
    for pane in panes {
        if f64_field(&pane["rect"], "pane layout", "x")? != min_x {
            column.push(pane);
        }
    }
    sort_by_y(&mut column);
    Ok(column.into_iter().cloned().collect())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::multiplexer::test_support::{
        FakeRunner, argv, env, herdr_envelope, herdr_error_stderr, run_event, sleep_event,
    };
    use crate::multiplexer::{HerdrMultiplexer, Lead, MultiplexerContext, RunError};

    fn herdr_env() -> std::collections::HashMap<String, String> {
        env(&[("HERDR_ENV", "1")])
    }

    fn assert_styled_read(runner: &FakeRunner, pane: &str, depth: &str) {
        let calls = runner.run_argvs();
        assert_eq!(calls.len(), 1);
        assert_eq!(&calls[0][..4], ["herdr", "pane", "read", pane]);
        let options = &calls[0][4..];
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

    fn reference() -> MultiplexerContext {
        MultiplexerContext {
            session: "w1".to_string(),
            window_id: "w1:t1".to_string(),
            pane_id: "w1:p1".to_string(),
        }
    }

    fn cwd() -> String {
        std::env::current_dir()
            .unwrap()
            .to_str()
            .unwrap()
            .to_string()
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
            err.to_string()
                .starts_with("herdr returned non-JSON output:"),
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
        runner.succeed(1);
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
        runner.succeed(2);
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
                    "herdr",
                    "pane",
                    "split",
                    "w1:p2",
                    "--direction",
                    "down",
                    "--no-focus",
                    "--cwd",
                    &cwd(),
                ]),
                argv(&["herdr", "pane", "layout", "--pane", "w1:p1"]),
                argv(&[
                    "herdr",
                    "pane",
                    "resize",
                    "--pane",
                    "w1:p3",
                    "--direction",
                    "up",
                    "--amount",
                    "0.2",
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
        runner.succeed(1);
        let mux = HerdrMultiplexer::new(runner, herdr_env());
        let pane = mux
            .split_window(&reference(), &[], &argv(&["claude"]))
            .unwrap();
        assert_eq!(pane, "w1:p3", "a rebalance failure never fails the spawn");
    }

    #[test]
    fn escape_lead_sends_the_escape_key_and_settles_before_the_literal() {
        let runner = FakeRunner::with_binary("herdr");
        runner.succeed(3);
        let mux = HerdrMultiplexer::new(runner.clone(), herdr_env());
        let text = "[cafleet msg 5 from 2 timestamp] hello";
        mux.send_prompt("w1:p2", text, Lead::Escape).unwrap();
        assert_eq!(
            runner.run_argvs(),
            vec![
                argv(&["herdr", "pane", "send-keys", "w1:p2", "escape"]),
                argv(&["herdr", "pane", "send-text", "w1:p2", text]),
                argv(&["herdr", "pane", "send-keys", "w1:p2", "enter"])
            ]
        );
        let events = runner.events();
        assert_eq!(events.len(), 5, "{events:?}");
        assert_eq!(
            events[0],
            run_event(&["herdr", "pane", "send-keys", "w1:p2", "escape"], Some(1))
        );
        assert_eq!(events[1], sleep_event(0.5));
        assert_eq!(events[3], sleep_event(1.0));
    }

    #[test]
    fn capture_pane_over_fetch_survives_a_blank_tail_deeper_than_n() {
        let runner = FakeRunner::with_binary("herdr");
        runner.respond(Ok("drawn1\ndrawn2\n\n\n\n\n\n\n".to_string()));
        let mux = HerdrMultiplexer::new(runner.clone(), herdr_env());
        assert_eq!(
            mux.capture_pane("w1:p2", 2).unwrap(),
            "drawn1\ndrawn2",
            "a depth-N fetch window would have been all-blank here"
        );
        assert_styled_read(&runner, "w1:p2", "1002");
    }

    #[test]
    fn capture_pane_keeps_crlf_lines_and_trims_a_blank_crlf_tail() {
        let runner = FakeRunner::with_binary("herdr");
        runner.respond(Ok(
            "scrollback\r\n Read outside\r\n\x1b[1m ❯ 1. Yes\x1b[0m\r\n Esc to cancel\r\n\r\n  \r\n"
                .to_string(),
        ));
        let mux = HerdrMultiplexer::new(runner.clone(), herdr_env());
        assert_eq!(
            mux.capture_pane("w1:p2", 3).unwrap(),
            " Read outside\r\n\x1b[1m ❯ 1. Yes\x1b[0m\r\n Esc to cancel\r",
            "the window keeps the last 3 drawn lines with their raw CRLF bytes"
        );
        assert_styled_read(&runner, "w1:p2", "1003");
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
        runner.succeed(1);
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
                    "herdr",
                    "pane",
                    "resize",
                    "--pane",
                    "w1:p1",
                    "--direction",
                    "right",
                    "--amount",
                    "0.4",
                ]),
            ],
            "pre-close tab read → close → anchor on a survivor → restore width"
        );
    }

    #[test]
    fn closing_a_member_equalizes_the_surviving_right_column_in_geometry_order() {
        let runner = FakeRunner::with_binary("herdr");
        runner.respond(Ok(herdr_envelope(json!({"pane":{"tab_id":"w1:t1"}}))));
        runner.succeed(1);
        runner.respond(Ok(herdr_envelope(
            json!({"panes":[{"pane_id":"w1:p1", "tab_id":"w1:t1"}]}),
        )));
        runner.respond(Ok(herdr_envelope(json!({"layout": {
            "panes": [
                {"pane_id":"w1:p4", "rect":{"x":60,"y":15}},
                {"pane_id":"w1:p1", "rect":{"x":0,"y":0}},
                {"pane_id":"w1:p3", "rect":{"x":60,"y":0}}
            ],
            "splits": [{"direction":"down", "rect":{"x":60,"y":15}, "ratio":0.2}]
        }}))));
        runner.succeed(1);
        let mux = crate::multiplexer::AnyMultiplexer::Herdr(HerdrMultiplexer::new(
            runner.clone(),
            herdr_env(),
        ));
        let layout = mux.close_pane("w1:p2", false).unwrap();
        assert_eq!(runner.run_argvs().len(), 2);
        mux.restore_after_close(layout).unwrap();
        let calls = runner.run_argvs();
        assert_eq!(
            calls[4],
            argv(&[
                "herdr",
                "pane",
                "resize",
                "--pane",
                "w1:p3",
                "--direction",
                "down",
                "--amount",
                "0.3"
            ])
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
    fn failed_run_closes_the_owned_pane_once_and_preserves_cleanup_failure() {
        for close_fails in [false, true] {
            let runner = FakeRunner::with_binary("herdr");
            runner.respond(Ok(herdr_envelope(
                json!({"panes": [{"pane_id":"w1:p1", "tab_id":"w1:t1"}]}),
            )));
            runner.respond(Ok(herdr_envelope(json!({"pane":{"pane_id":"w1:p9"}}))));
            runner.respond(Err(RunError::Failed {
                stderr: "primary run failure".into(),
            }));
            runner.respond(Err(RunError::Failed {
                stderr: "optional pane lookup failed".into(),
            }));
            runner.respond(if close_fails {
                Err(RunError::Failed {
                    stderr: "close failed".into(),
                })
            } else {
                Ok(String::new())
            });
            let mux = HerdrMultiplexer::new(runner.clone(), herdr_env());
            let error = mux
                .split_window(&reference(), &[], &argv(&["agent", "a'b", "$(literal)"]))
                .unwrap_err();
            assert!(error.to_string().contains("primary run failure"));
            assert!(
                matches!(error.pane_cleanup(), Some(crate::multiplexer::PaneCleanup::Attempted { pane_id, error }) if pane_id == "w1:p9" && error.is_some() == close_fails)
            );
            drop(mux);
            let calls = runner.run_argvs();
            assert_eq!(calls.iter().filter(|call| call[2] == "close").count(), 1);
            assert_eq!(
                calls[2],
                argv(&[
                    "herdr",
                    "pane",
                    "run",
                    "w1:p9",
                    "agent 'a'\"'\"'b' '$(literal)'"
                ])
            );
        }
    }

    #[test]
    fn an_unknown_split_id_keeps_unowned_panes_untouched() {
        for split in [
            Ok(herdr_envelope(json!({"pane":{}}))),
            Ok(herdr_envelope(json!({"pane":{"pane_id":" "}}))),
            Err(RunError::Timeout),
        ] {
            let runner = FakeRunner::with_binary("herdr");
            runner.respond(Ok(herdr_envelope(
                json!({"panes": [{"pane_id":"w1:p1", "tab_id":"w1:t1"}]}),
            )));
            runner.respond(split);
            let mux = HerdrMultiplexer::new(runner.clone(), herdr_env());
            let error = mux
                .split_window(&reference(), &[], &argv(&["agent"]))
                .unwrap_err();
            assert!(matches!(
                error.pane_cleanup(),
                Some(crate::multiplexer::PaneCleanup::Unknown)
            ));
            drop(mux);
            assert_eq!(runner.run_argvs().len(), 2);
        }
    }
}
