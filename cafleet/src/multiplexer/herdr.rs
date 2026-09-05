//! herdr backend (SPEC §6.5 *HerdrMultiplexer*) — JSON envelope parsing, the
//! layout-ratio equalization arithmetic, `agent_status`, and the kill/rebalance
//! phases. The colocated tests pin the contract; see [`super::test_support`]
//! for the API.

use super::spawn::{
    Deadline, PaneSpawnRequest, SpawnExecution, SpawnMultiplexer, SpawnOps, SpawnRun,
};
use std::collections::{BTreeSet, HashMap};
use std::rc::Rc;
use std::time::Duration;

use serde_json::Value;

use super::{
    CommandRunner, ESC_SETTLE_DELAY, MultiplexerContext, MultiplexerError, PaneCleanup,
    PaneOwnership, RunError, SUBMIT_DELAY, WakeEntry, build_wake_payload_from_entries,
};

const PANE_NOT_FOUND: &str = "pane_not_found";

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

fn map_run_error(argv: &[String], timeout_secs: Option<u64>, error: RunError) -> MultiplexerError {
    let joined = argv.join(" ");
    match error {
        RunError::BinaryNotFound(detail) => {
            MultiplexerError::new(format!("herdr binary not found: {detail}"))
        }
        RunError::Timeout => {
            let secs = timeout_secs.expect("a timeout error implies a timeout was set");
            MultiplexerError::new(format!("herdr command timed out after {secs}s: {joined}"))
        }
        RunError::Failed { stderr } => MultiplexerError::new(format!(
            "herdr command failed: {joined}\nstderr: {}",
            stderr.trim()
        )),
    }
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

pub struct HerdrMultiplexer {
    runner: Rc<dyn CommandRunner>,
    env: HashMap<String, String>,
}

impl HerdrMultiplexer {
    pub fn new(runner: Rc<dyn CommandRunner>, env: HashMap<String, String>) -> Self {
        HerdrMultiplexer { runner, env }
    }

    pub fn name(&self) -> &'static str {
        "herdr"
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

    fn run_json(
        &self,
        argv: &[String],
        timeout_secs: Option<u64>,
    ) -> Result<Value, MultiplexerError> {
        let out = self.run(argv, timeout_secs)?;
        parse_envelope(&out)
    }

    fn run_tolerating_missing(
        &self,
        argv: &[String],
        ignore_missing: bool,
        timeout_secs: Option<u64>,
    ) -> Result<(), MultiplexerError> {
        match self.runner.run(argv, timeout_secs) {
            Ok(_) => Ok(()),
            Err(RunError::Failed { ref stderr })
                if ignore_missing && error_code(stderr).as_deref() == Some(PANE_NOT_FOUND) =>
            {
                Ok(())
            }
            Err(error) => Err(map_run_error(argv, timeout_secs, error)),
        }
    }

    fn send_esc(&self, target_pane_id: &str, ignore_missing: bool) -> Result<(), MultiplexerError> {
        self.run_tolerating_missing(
            &herdr_argv(&["herdr", "pane", "send-keys", target_pane_id, "esc"]),
            ignore_missing,
            Some(5),
        )?;
        self.runner.sleep(ESC_SETTLE_DELAY);
        Ok(())
    }

    fn best_effort(&self, steps: impl FnOnce() -> Result<(), MultiplexerError>) -> bool {
        if !self.runner.binary_exists("herdr") {
            return false;
        }
        steps().is_ok()
    }

    pub fn ensure_available(&self) -> Result<(), MultiplexerError> {
        if !self.runner.binary_exists("herdr") {
            return Err(MultiplexerError::new("herdr binary not found on PATH"));
        }
        if self.env_var("HERDR_ENV").is_none() {
            return Err(MultiplexerError::new(
                "cafleet member commands must be run inside a herdr session",
            ));
        }
        Ok(())
    }

    pub fn context_discovery(&self) -> Result<MultiplexerContext, MultiplexerError> {
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
    pub fn split_window(
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
        self.split_common(
            &PaneSpawnRequest {
                reference,
                env,
                command,
                cwd: Some(&cwd),
            },
            &SpawnOps {
                run: &|argv| self.run(argv, None),
                kill: &|id| self.kill_pane(id, true),
                transfer: &|| Ok(()),
            },
        )
    }

    fn split_common(
        &self,
        request: &PaneSpawnRequest<'_>,
        ops: &SpawnOps<'_>,
    ) -> Result<String, MultiplexerError> {
        let PaneSpawnRequest {
            reference,
            env,
            command,
            cwd,
        } = *request;
        let cwd = cwd
            .ok_or_else(|| {
                MultiplexerError::new("herdr pane spawn requires a prepared working directory")
            })?
            .display()
            .to_string();
        let result = parse_envelope(&(ops.run)(&herdr_argv(&["herdr", "pane", "list"]))?)?;
        let panes = result["panes"]
            .as_array()
            .ok_or_else(|| MultiplexerError::new("herdr pane list missing 'panes' field"))?;
        let mut column: Vec<String> = Vec::new();
        for pane in panes {
            let pane_id = str_field(pane, "pane list", "pane_id")?;
            if pane["tab_id"].as_str() == Some(&reference.window_id) && pane_id != reference.pane_id
            {
                column.push(pane_id.to_string());
            }
        }
        let new_pane_id = match column.iter().max() {
            None => self.split_pane(&reference.pane_id, "right", &cwd, env, ops.run)?,
            Some(target) => self.split_pane(target, "down", &cwd, env, ops.run)?,
        };
        let mut pane = PaneOwnership::new(new_pane_id.clone(), ops.kill);
        let result = (|| {
            if !column.is_empty()
                && let Err(error) = self.resize_tab_column(&reference.pane_id, ops.run)
                && error.is_timeout()
            {
                return Err(error);
            }
            (ops.run)(&herdr_argv(&[
                "herdr",
                "pane",
                "run",
                &new_pane_id,
                &shlex_join(command),
            ]))?;
            (ops.transfer)()
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
        run: &SpawnRun<'_>,
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
        let result = run(&args)
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
        run: &SpawnRun<'_>,
    ) -> Result<(Vec<Value>, Vec<Value>), MultiplexerError> {
        let result = parse_envelope(&run(&herdr_argv(&[
            "herdr",
            "pane",
            "layout",
            "--pane",
            anchor_pane_id,
        ]))?)?;
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
    fn resize_tab_column(
        &self,
        anchor_pane_id: &str,
        run: &SpawnRun<'_>,
    ) -> Result<(), MultiplexerError> {
        let (panes, splits) = self.read_tab_layout(anchor_pane_id, run)?;
        let column = right_column(&panes)?;
        if column.len() < 2 {
            return Ok(());
        }
        self.equalize_column(&column, &splits, run)
    }

    /// A right column of N stacked members is a right-leaning chain of `down`
    /// splits: split_k separates member_k from the subtree below it, so equal
    /// heights ⇔ split_k has ratio 1/(N-k). `resize --amount` is a signed
    /// delta on a split's ratio, so each split is driven to its target by one
    /// arithmetic resize.
    fn equalize_column(
        &self,
        column: &[Value],
        splits: &[Value],
        run: &SpawnRun<'_>,
    ) -> Result<(), MultiplexerError> {
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
        down_splits.sort_by(|a, b| {
            a["rect"]["y"]
                .as_f64()
                .partial_cmp(&b["rect"]["y"].as_f64())
                .expect("layout rects carry numeric y")
        });
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
            run(&herdr_argv(&[
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

    pub fn send_exit(
        &self,
        target_pane_id: &str,
        ignore_missing: bool,
    ) -> Result<(), MultiplexerError> {
        self.send_esc(target_pane_id, ignore_missing)?;
        self.run_tolerating_missing(
            &herdr_argv(&["herdr", "pane", "run", target_pane_id, "/exit"]),
            ignore_missing,
            None,
        )
    }

    pub fn send_poll_trigger(&self, target_pane_id: &str, member_id: i64) -> bool {
        let payload = format!(
            "cafleet message poll {member_id} \
             — then resume your work if something was still running."
        );
        self.best_effort(|| {
            self.send_esc(target_pane_id, false)?;
            self.run(
                &herdr_argv(&["herdr", "pane", "run", target_pane_id, &payload]),
                Some(5),
            )?;
            Ok(())
        })
    }

    /// Esc first: the wake targets the monitor member's own pane, which can
    /// be parked on a permission prompt.
    pub fn send_wake_entries(
        &self,
        target_pane_id: &str,
        fleet_id: i64,
        members: &[WakeEntry<'_>],
        director: &WakeEntry<'_>,
    ) -> Result<bool, MultiplexerError> {
        let payload = build_wake_payload_from_entries(fleet_id, members, director)?;
        Ok(self.best_effort(|| {
            self.send_esc(target_pane_id, false)?;
            self.run(
                &herdr_argv(&["herdr", "pane", "run", target_pane_id, &payload]),
                Some(5),
            )?;
            Ok(())
        }))
    }

    /// `send-text` delivers the raw 2-line payload without submitting; the
    /// single trailing enter submits the whole payload as one recipient turn.
    /// Result-returning (SPEC §6.5): the missing-binary precheck fails with
    /// the exact PATH string, and any esc/send-text/enter failure propagates
    /// as the raw error — no boolean wrapper, no retry.
    pub fn send_inline_preview(
        &self,
        target_pane_id: &str,
        message_id: i64,
        sender_id: i64,
        ts: &str,
        text: &str,
    ) -> Result<(), MultiplexerError> {
        if !self.runner.binary_exists("herdr") {
            return Err(MultiplexerError::new("herdr binary not found on PATH"));
        }
        let sanitized = text.replace("\r\n", "⏎").replace(['\n', '\r'], "⏎");
        let payload = format!("[cafleet msg {message_id} from {sender_id} {ts}]\n{sanitized}");
        self.send_esc(target_pane_id, false)?;
        self.run(
            &herdr_argv(&["herdr", "pane", "send-text", target_pane_id, &payload]),
            Some(5),
        )?;
        self.runner.sleep(SUBMIT_DELAY);
        self.run(
            &herdr_argv(&["herdr", "pane", "send-keys", target_pane_id, "enter"]),
            Some(5),
        )?;
        Ok(())
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
        self.send_esc(target_pane_id, false)?;
        self.run(
            &herdr_argv(&["herdr", "pane", "run", target_pane_id, &payload]),
            None,
        )?;
        Ok(())
    }

    /// `herdr pane read` prints the raw terminal buffer (no envelope); the
    /// shared A8 windowing ([`super::capture_window`]) enforces the last-N
    /// window client-side — the daemon may return more rows than requested.
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
            &herdr_argv(&[
                "herdr",
                "pane",
                "read",
                target_pane_id,
                "--source",
                "recent-unwrapped",
                "--lines",
                &fetch_depth.to_string(),
            ]),
            None,
        )?;
        Ok(super::capture_window(&raw, lines))
    }

    pub fn list_pane_ids(&self) -> Result<BTreeSet<String>, MultiplexerError> {
        let result = self.run_json(&herdr_argv(&["herdr", "pane", "list"]), Some(5))?;
        let panes = result["panes"]
            .as_array()
            .ok_or_else(|| MultiplexerError::new("herdr pane list missing 'panes' field"))?;
        panes
            .iter()
            .map(|pane| Ok(str_field(pane, "pane list", "pane_id")?.to_string()))
            .collect()
    }

    pub fn kill_pane(
        &self,
        target_pane_id: &str,
        ignore_missing: bool,
    ) -> Result<(), MultiplexerError> {
        let target_tab_id = self.pane_tab_id(target_pane_id);
        self.run_tolerating_missing(
            &herdr_argv(&["herdr", "pane", "close", target_pane_id]),
            ignore_missing,
            None,
        )?;
        if let Some(tab_id) = target_tab_id {
            let _ = self.resize_after_close(&tab_id);
        }
        Ok(())
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

    fn resize_after_close(&self, target_tab_id: &str) -> Result<(), MultiplexerError> {
        let Some(anchor_pane_id) = self.surviving_pane_in_tab(target_tab_id)? else {
            return Ok(()); // the tab has no panes left — nothing to rebalance
        };
        let (panes, splits) =
            self.read_tab_layout(&anchor_pane_id, &|argv| self.run(argv, None))?;
        if panes.is_empty() {
            return Ok(());
        }
        let column = right_column(&panes)?;
        if column.len() >= 2 {
            self.equalize_column(&column, &splits, &|argv| self.run(argv, None))
        } else if column.is_empty() {
            self.restore_director_full_width(&panes, &splits)
        } else {
            // A lone member pane spans the column natively; the right split's
            // ratio is unaffected by a down-close.
            Ok(())
        }
    }

    fn surviving_pane_in_tab(&self, tab_id: &str) -> Result<Option<String>, MultiplexerError> {
        let result = self.run_json(&herdr_argv(&["herdr", "pane", "list"]), None)?;
        let panes = result["panes"]
            .as_array()
            .ok_or_else(|| MultiplexerError::new("herdr pane list missing 'panes' field"))?;
        for pane in panes {
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

    /// A pane closing between the tick's `list_pane_ids` and this read is a
    /// teardown race, not an error: `pane_not_found` reads as "no agent".
    pub fn agent_status(&self, target_pane_id: &str) -> Result<Option<String>, MultiplexerError> {
        let argv = herdr_argv(&["herdr", "pane", "get", target_pane_id]);
        let out = match self.runner.run(&argv, None) {
            Ok(out) => out,
            Err(RunError::Failed { ref stderr })
                if error_code(stderr).as_deref() == Some(PANE_NOT_FOUND) =>
            {
                return Ok(None);
            }
            Err(error) => return Err(map_run_error(&argv, None, error)),
        };
        let result = parse_envelope(&out)?;
        let status = result["pane"]["agent_status"]
            .as_str()
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        Ok(status)
    }
}

/// The members' right column: every pane outside the leftmost (Director)
/// column, sorted top to bottom.
fn right_column(panes: &[Value]) -> Result<Vec<Value>, MultiplexerError> {
    let mut min_x = f64::INFINITY;
    for pane in panes {
        min_x = min_x.min(f64_field(&pane["rect"], "pane layout", "x")?);
    }
    let mut column: Vec<Value> = Vec::new();
    for pane in panes {
        if f64_field(&pane["rect"], "pane layout", "x")? != min_x {
            column.push(pane.clone());
        }
    }
    column.sort_by(|a, b| {
        a["rect"]["y"]
            .as_f64()
            .partial_cmp(&b["rect"]["y"].as_f64())
            .expect("layout rects carry numeric y")
    });
    Ok(column)
}

impl SpawnMultiplexer for HerdrMultiplexer {
    fn split_prepared(
        &self,
        request: &PaneSpawnRequest<'_>,
        deadline: &Deadline,
        execution: &SpawnExecution<'_>,
    ) -> Result<String, MultiplexerError> {
        self.split_common(
            request,
            &SpawnOps {
                run: &|argv| execution.run("herdr", argv, deadline),
                kill: &|id| {
                    self.kill_pane_with_deadline(
                        id,
                        true,
                        &Deadline::after(execution.clock, Duration::from_secs(5)),
                        execution,
                    )
                },
                transfer: &|| execution.check_transfer("herdr", deadline),
            },
        )
    }
    fn kill_pane_with_deadline(
        &self,
        pane_id: &str,
        ignore_missing: bool,
        deadline: &Deadline,
        execution: &SpawnExecution<'_>,
    ) -> Result<(), MultiplexerError> {
        execution
            .run_tolerating(
                "herdr",
                &herdr_argv(&["herdr", "pane", "close", pane_id]),
                deadline,
                &|stderr| ignore_missing && error_code(stderr).as_deref() == Some(PANE_NOT_FOUND),
            )
            .map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use crate::multiplexer::PaneCleanup;
    use crate::multiplexer::WakeEntry;
    use serde_json::json;

    use crate::multiplexer::test_support::{
        FakeRunner, argv, env, herdr_envelope, herdr_error_stderr, run_event, sleep_event,
    };
    use crate::multiplexer::{
        HerdrMultiplexer, MultiplexerContext, RunError, build_wake_payload_from_entries,
    };

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
        std::env::current_dir()
            .unwrap()
            .to_str()
            .unwrap()
            .to_string()
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
        let mux = HerdrMultiplexer::new(runner, herdr_env());
        let pane = mux
            .split_window(&reference(), &[], &argv(&["claude"]))
            .unwrap();
        assert_eq!(pane, "w1:p3", "a rebalance failure never fails the spawn");
    }

    #[test]
    fn send_exit_runs_the_exit_line_and_tolerates_only_pane_not_found() {
        for missing_keystroke in 0..2 {
            let runner = FakeRunner::with_binary("herdr");
            for _ in 0..missing_keystroke {
                runner.respond(Ok(String::new()));
            }
            runner.respond(Err(RunError::Failed {
                stderr: herdr_error_stderr("pane_not_found"),
            }));
            let mux = HerdrMultiplexer::new(runner.clone(), herdr_env());
            mux.send_exit("w1:p2", true).unwrap();
            assert_eq!(
                runner.events(),
                vec![
                    run_event(&["herdr", "pane", "send-keys", "w1:p2", "esc"], Some(5)),
                    sleep_event(0.1),
                    run_event(&["herdr", "pane", "run", "w1:p2", "/exit"], None),
                ],
                "a missing pane at keystroke {missing_keystroke} is tolerated"
            );
        }

        for failed_keystroke in 0..2 {
            let runner = FakeRunner::with_binary("herdr");
            for _ in 0..failed_keystroke {
                runner.respond(Ok(String::new()));
            }
            runner.respond(Err(RunError::Failed {
                stderr: herdr_error_stderr("internal"),
            }));
            let mux = HerdrMultiplexer::new(runner, herdr_env());
            assert!(
                mux.send_exit("w1:p2", true).is_err(),
                "a non-pane-not-found error at keystroke {failed_keystroke} propagates"
            );
        }

        let runner = FakeRunner::with_binary("herdr");
        runner.respond(Err(RunError::Failed {
            stderr: herdr_error_stderr("pane_not_found"),
        }));
        let mux = HerdrMultiplexer::new(runner, herdr_env());
        assert!(
            mux.send_exit("w1:p2", false).is_err(),
            "pane_not_found propagates when ignore_missing is false"
        );
    }

    #[test]
    fn send_poll_trigger_is_esc_then_run() {
        let runner = FakeRunner::with_binary("herdr");
        let mux = HerdrMultiplexer::new(runner.clone(), herdr_env());
        assert!(mux.send_poll_trigger("w1:p2", 14));
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
                        "cafleet message poll 14 — then resume \
                         your work if something was still running.",
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
        assert!(!mux.send_poll_trigger("w1:p2", 14));
        assert!(runner.events().is_empty());

        let runner = FakeRunner::with_binary("herdr");
        runner.respond(Err(RunError::Failed {
            stderr: "boom".to_string(),
        }));
        let mux = HerdrMultiplexer::new(runner, herdr_env());
        assert!(!mux.send_poll_trigger("w1:p2", 14));
    }

    #[test]
    fn send_wake_entries_is_esc_then_run() {
        let runner = FakeRunner::with_binary("herdr");
        let mux = HerdrMultiplexer::new(runner.clone(), herdr_env());
        let members = [WakeEntry {
            member_id: 4,
            name: "worker",
            coding_agent: "codex",
            pending_count: 0,
        }];
        let director = WakeEntry {
            member_id: 2,
            name: "Director",
            coding_agent: "claude",
            pending_count: 1,
        };
        assert!(
            mux.send_wake_entries("w1:p9", 3, &members, &director)
                .unwrap()
        );

        let payload = build_wake_payload_from_entries(3, &members, &director).unwrap();
        assert_eq!(
            runner.events(),
            vec![
                run_event(&["herdr", "pane", "send-keys", "w1:p9", "esc"], Some(5)),
                sleep_event(0.1),
                run_event(&["herdr", "pane", "run", "w1:p9", &payload], Some(5)),
            ],
            "Esc-first, then one atomic run; the payload is byte-identical to tmux's"
        );
    }

    #[test]
    fn send_inline_preview_is_esc_send_text_delay_enter() {
        let runner = FakeRunner::with_binary("herdr");
        let mux = HerdrMultiplexer::new(runner.clone(), herdr_env());
        let ts = "2026-07-30T09:00:00.000000+00:00";
        assert!(mux.send_inline_preview("w1:p2", 5, 2, ts, "a\nb").is_ok());
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
    fn send_inline_preview_without_the_binary_is_the_exact_path_error() {
        let runner = FakeRunner::without_binaries();
        let mux = HerdrMultiplexer::new(runner.clone(), herdr_env());
        let err = mux
            .send_inline_preview("w1:p2", 5, 2, "2026-07-30T09:00:00.000000+00:00", "hi")
            .unwrap_err();
        assert_eq!(err.to_string(), "herdr binary not found on PATH");
        assert!(
            runner.events().is_empty(),
            "the precheck precedes any keystroke"
        );
    }

    #[test]
    fn send_inline_preview_propagates_an_esc_failure_with_the_raw_detail() {
        let runner = FakeRunner::with_binary("herdr");
        runner.respond(Err(RunError::Failed {
            stderr: herdr_error_stderr("internal"),
        }));
        let mux = HerdrMultiplexer::new(runner.clone(), herdr_env());
        let err = mux
            .send_inline_preview("w1:p2", 5, 2, "2026-07-30T09:00:00.000000+00:00", "hi")
            .unwrap_err();
        assert_eq!(
            err.to_string(),
            format!(
                "herdr command failed: herdr pane send-keys w1:p2 esc\nstderr: {}",
                herdr_error_stderr("internal")
            )
        );
        assert_eq!(
            runner.events(),
            vec![run_event(
                &["herdr", "pane", "send-keys", "w1:p2", "esc"],
                Some(5),
            )],
            "a failed esc aborts before the payload"
        );
    }

    #[test]
    fn send_inline_preview_propagates_a_send_text_failure_with_argv_and_stderr() {
        let runner = FakeRunner::with_binary("herdr");
        runner.respond(Ok(String::new()));
        runner.respond(Err(RunError::Failed {
            stderr: "pane bridge broke".to_string(),
        }));
        let mux = HerdrMultiplexer::new(runner.clone(), herdr_env());
        let ts = "2026-07-30T09:00:00.000000+00:00";
        let err = mux
            .send_inline_preview("w1:p2", 5, 2, ts, "a\nb")
            .unwrap_err();
        let payload = format!("[cafleet msg 5 from 2 {ts}]\na⏎b");
        assert_eq!(
            err.to_string(),
            format!(
                "herdr command failed: herdr pane send-text w1:p2 {payload}\nstderr: pane bridge broke"
            )
        );
        assert_eq!(
            runner.events(),
            vec![
                run_event(&["herdr", "pane", "send-keys", "w1:p2", "esc"], Some(5)),
                sleep_event(0.1),
                run_event(&["herdr", "pane", "send-text", "w1:p2", &payload], Some(5),),
            ],
            "one attempt only — the send-text failure aborts before enter"
        );
    }

    #[test]
    fn send_inline_preview_propagates_a_trailing_enter_failure() {
        let runner = FakeRunner::with_binary("herdr");
        runner.respond(Ok(String::new()));
        runner.respond(Ok(String::new()));
        runner.respond(Err(RunError::Failed {
            stderr: "enter lost".to_string(),
        }));
        let mux = HerdrMultiplexer::new(runner.clone(), herdr_env());
        let ts = "2026-07-30T09:00:00.000000+00:00";
        let err = mux
            .send_inline_preview("w1:p2", 5, 2, ts, "hi")
            .unwrap_err();
        assert_eq!(
            err.to_string(),
            "herdr command failed: herdr pane send-keys w1:p2 enter\nstderr: enter lost"
        );
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
                        &format!("[cafleet msg 5 from 2 {ts}]\nhi"),
                    ],
                    Some(5),
                ),
                sleep_event(1.0),
                run_event(&["herdr", "pane", "send-keys", "w1:p2", "enter"], Some(5)),
            ],
            "the full choreography ran once; no retry follows the lost enter"
        );
    }

    #[test]
    fn send_wake_entries_remains_best_effort_on_delivery_failure() {
        let members = [WakeEntry {
            member_id: 4,
            name: "worker",
            coding_agent: "codex",
            pending_count: 0,
        }];
        let director = WakeEntry {
            member_id: 2,
            name: "Director",
            coding_agent: "claude",
            pending_count: 0,
        };

        let runner = FakeRunner::without_binaries();
        let mux = HerdrMultiplexer::new(runner.clone(), herdr_env());
        assert!(
            !mux.send_wake_entries("w1:p9", 3, &members, &director)
                .unwrap(),
            "herdr missing → Ok(false)"
        );
        assert!(runner.events().is_empty());

        let runner = FakeRunner::with_binary("herdr");
        runner.respond(Err(RunError::Failed {
            stderr: "boom".to_string(),
        }));
        let mux = HerdrMultiplexer::new(runner, herdr_env());
        assert!(
            !mux.send_wake_entries("w1:p9", 3, &members, &director)
                .unwrap(),
            "a keystroke error stays Ok(false), never an Err"
        );
    }

    #[test]
    fn send_prompt_shell_and_plain_forms() {
        for (text, shell, payload) in [(" ls ", true, "! ls"), ("hi there", false, "hi there")] {
            let runner = FakeRunner::with_binary("herdr");
            let mux = HerdrMultiplexer::new(runner.clone(), herdr_env());
            mux.send_prompt("w1:p2", text, shell).unwrap();
            assert_eq!(
                runner.events(),
                vec![
                    run_event(&["herdr", "pane", "send-keys", "w1:p2", "esc"], Some(5)),
                    sleep_event(0.1),
                    run_event(&["herdr", "pane", "run", "w1:p2", payload], None),
                ],
                "the shell flag changes only the payload prefix"
            );
        }

        let runner = FakeRunner::with_binary("herdr");
        runner.respond(Err(RunError::Failed {
            stderr: herdr_error_stderr("internal"),
        }));
        let mux = HerdrMultiplexer::new(runner.clone(), herdr_env());
        assert!(
            mux.send_prompt("w1:p2", "ls", true).is_err(),
            "the shell form propagates an Esc failure like the plain form"
        );
        assert_eq!(
            runner.events(),
            vec![run_event(
                &["herdr", "pane", "send-keys", "w1:p2", "esc"],
                Some(5),
            )],
            "a failed Esc aborts before the payload"
        );

        let mux = HerdrMultiplexer::new(FakeRunner::with_binary("herdr"), herdr_env());
        assert_eq!(
            mux.send_prompt("w1:p2", " ", false)
                .unwrap_err()
                .to_string(),
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
    fn capture_pane_drops_the_blank_tail_and_keeps_the_buffer() {
        let runner = FakeRunner::with_binary("herdr");
        runner.respond(Ok("raw\nbuffer\n\n \n".to_string()));
        let mux = HerdrMultiplexer::new(runner.clone(), herdr_env());
        assert_eq!(
            mux.capture_pane("w1:p2", 20).unwrap(),
            "raw\nbuffer",
            "trailing whitespace-only lines are dropped before the slice (A8)"
        );
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
                    "1020",
                ],
                None,
            )],
            "A8 over-fetches by the fixed 1000-line margin"
        );

        assert_eq!(
            mux.capture_pane("w1:p2", 0).unwrap_err().to_string(),
            "capture_pane: lines must be positive, got 0"
        );
    }

    // A8: TUI-painted empty rows carry ANSI sequences — a trailing line is
    // blank when it is whitespace-only AFTER per-line CSI stripping; the kept
    // lines keep their original bytes.
    #[test]
    fn capture_pane_blank_detection_is_ansi_aware() {
        let runner = FakeRunner::with_binary("herdr");
        runner.respond(Ok(
            "one\ntwo\x1b[0m\n\x1b[39m\x1b[49m\n \x1b[2K\n".to_string()
        ));
        let mux = HerdrMultiplexer::new(runner, herdr_env());
        assert_eq!(
            mux.capture_pane("w1:p2", 2).unwrap(),
            "one\ntwo\x1b[0m",
            "CSI-only trailing rows are visually blank and dropped; the kept \
             lines' ANSI survives untouched"
        );
    }

    // A8: the daemon may return more rows than requested; after the blank tail
    // is dropped, the last-N window is enforced client-side.
    #[test]
    fn capture_pane_enforces_the_last_n_window_client_side() {
        let runner = FakeRunner::with_binary("herdr");
        runner.respond(Ok("one\ntwo\nthree\n\n\n".to_string()));
        let mux = HerdrMultiplexer::new(runner.clone(), herdr_env());
        assert_eq!(
            mux.capture_pane("w1:p2", 2).unwrap(),
            "two\nthree",
            "the drawn bottom survives a small --lines window"
        );
        assert!(
            runner
                .run_argvs()
                .iter()
                .any(|argv| argv.contains(&"1002".to_string())),
            "the over-fetched window (N + 1000) reaches the fetch argv"
        );
    }

    // A8: the fetch depth is requested lines + 1000, so a blank tail deeper
    // than N still leaves the drawn bottom inside the fetched buffer.
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
        assert_eq!(
            runner.run_argvs(),
            vec![argv(&[
                "herdr",
                "pane",
                "read",
                "w1:p2",
                "--source",
                "recent-unwrapped",
                "--lines",
                "1002",
            ])]
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

    fn queue_first_member_split(runner: &FakeRunner) {
        runner.respond(Ok(herdr_envelope(json!({
            "panes": [{"pane_id": "w1:p1", "tab_id": "w1:t1", "workspace_id": "w1"}]
        }))));
        runner.respond(Ok(herdr_envelope(json!({"pane": {"pane_id": "w1:p9"}}))));
    }

    #[test]
    fn creation_run_failure_closes_the_known_pane_exactly_once() {
        let runner = FakeRunner::with_binary("herdr");
        queue_first_member_split(&runner);
        runner.respond(Err(RunError::Failed {
            stderr: "primary run failure".into(),
        }));
        runner.respond(Err(RunError::Failed {
            stderr: "optional layout lookup failed".into(),
        }));
        runner.respond(Ok(String::new()));
        let mux = HerdrMultiplexer::new(runner.clone(), herdr_env());
        let error = mux
            .split_window(&reference(), &[], &argv(&["claude"]))
            .unwrap_err();
        assert!(error.to_string().contains("primary run failure"), "{error}");
        assert!(
            matches!(error.pane_cleanup(), Some(PaneCleanup::Attempted { pane_id, error: None }) if pane_id == "w1:p9")
        );
        drop(mux);
        let calls = runner.run_argvs();
        assert_eq!(
            calls
                .iter()
                .map(|args| args[2].as_str())
                .collect::<Vec<_>>(),
            ["list", "split", "run", "get", "close"]
        );
        assert_eq!(
            calls.last().unwrap(),
            &argv(&["herdr", "pane", "close", "w1:p9"])
        );
        assert!(
            !calls
                .iter()
                .any(|args| args.iter().any(|arg| arg == "/exit"))
        );
    }

    #[test]
    fn creation_close_failure_retains_run_error_and_pane_id_without_retry() {
        let runner = FakeRunner::with_binary("herdr");
        queue_first_member_split(&runner);
        runner.respond(Err(RunError::Failed {
            stderr: "primary run failure".into(),
        }));
        runner.respond(Err(RunError::Failed {
            stderr: "optional layout lookup failed".into(),
        }));
        runner.respond(Err(RunError::Failed {
            stderr: "secondary close failure".into(),
        }));
        let mux = HerdrMultiplexer::new(runner.clone(), herdr_env());
        let error = mux
            .split_window(&reference(), &[], &argv(&["claude"]))
            .unwrap_err();
        assert!(
            matches!(error.pane_cleanup(), Some(PaneCleanup::Attempted { pane_id, error: Some(detail) }) if pane_id == "w1:p9" && detail.contains("secondary close failure"))
        );
        let detail = error.to_string();
        assert!(
            detail.contains("cleanup failed for pane w1:p9:"),
            "{detail}"
        );
        assert!(
            detail.find("primary run failure").unwrap()
                < detail.find("secondary close failure").unwrap()
        );
        drop(mux);
        assert_eq!(
            runner
                .run_argvs()
                .iter()
                .filter(|args| args[2] == "close")
                .count(),
            1
        );
    }

    #[test]
    fn creation_malformed_split_reports_unknown_compensation_and_never_guesses_a_pane() {
        for response in ["not-json".to_string(), herdr_envelope(json!({"pane": {}}))] {
            let runner = FakeRunner::with_binary("herdr");
            runner.respond(Ok(herdr_envelope(json!({"panes": []}))));
            runner.respond(Ok(response));
            let mux = HerdrMultiplexer::new(runner.clone(), herdr_env());
            let error = mux
                .split_window(&reference(), &[], &argv(&["claude"]))
                .unwrap_err();
            assert!(matches!(error.pane_cleanup(), Some(PaneCleanup::Unknown)));
            let detail = error.to_string().to_lowercase();
            assert!(
                detail.contains("unknown") && detail.contains("unconfirmed"),
                "{detail}"
            );
            drop(mux);
            assert_eq!(
                runner
                    .run_argvs()
                    .iter()
                    .map(|args| args[2].as_str())
                    .collect::<Vec<_>>(),
                ["list", "split"]
            );
        }
    }

    #[test]
    fn creation_failed_split_retains_primary_and_reports_unconfirmed_compensation() {
        let runner = FakeRunner::with_binary("herdr");
        runner.respond(Ok(herdr_envelope(json!({"panes": []}))));
        runner.respond(Err(RunError::Failed {
            stderr: "split transport failed".into(),
        }));
        let mux = HerdrMultiplexer::new(runner.clone(), herdr_env());
        let error = mux
            .split_window(&reference(), &[], &argv(&["claude"]))
            .unwrap_err();
        assert!(matches!(error.pane_cleanup(), Some(PaneCleanup::Unknown)));
        let detail = error.to_string().to_lowercase();
        assert!(detail.contains("split transport failed"), "{detail}");
        assert!(
            detail.contains("unknown") && detail.contains("unconfirmed"),
            "{detail}"
        );
        assert_eq!(runner.run_argvs().len(), 2);
    }

    #[test]
    fn creation_list_failure_does_not_split_or_close_any_pane() {
        let runner = FakeRunner::with_binary("herdr");
        runner.respond(Err(RunError::Failed {
            stderr: "list failed".into(),
        }));
        let mux = HerdrMultiplexer::new(runner.clone(), herdr_env());
        let error = mux
            .split_window(&reference(), &[], &argv(&["claude"]))
            .unwrap_err();
        assert!(error.to_string().contains("list failed"));
        assert!(error.pane_cleanup().is_none());
        assert_eq!(runner.run_argvs(), vec![argv(&["herdr", "pane", "list"])]);
    }

    #[test]
    fn creation_success_transfers_ownership_without_closing_the_pane_on_drop() {
        let runner = FakeRunner::with_binary("herdr");
        queue_first_member_split(&runner);
        runner.respond(Ok(String::new()));
        let mux = HerdrMultiplexer::new(runner.clone(), herdr_env());
        assert_eq!(
            mux.split_window(&reference(), &[], &argv(&["claude"]))
                .unwrap(),
            "w1:p9"
        );
        drop(mux);
        assert_eq!(
            runner
                .run_argvs()
                .iter()
                .map(|args| args[2].as_str())
                .collect::<Vec<_>>(),
            ["list", "split", "run"]
        );
    }
}
