# Robust member teardown: wait for real shutdown and close the remaining raw-tmux escape hatches

**Status**: Complete
**Progress**: 46/46 tasks complete (Steps 1-4 done: documentation, tmux primitives, doctor subcommand, member_delete blocking + --force)
**Last Updated**: 2026-04-19

## Overview

`cafleet member delete` is currently fire-and-forget: it sends `/exit` and returns without confirming the pane closed. Claude/Codex panes that block on a confirmation prompt therefore become orphan processes behind a "closed" session. This doc introduces blocking-until-real-shutdown semantics with a 15 s timeout, a `--force` escalation that kill-panes immediately, and a new `cafleet doctor` subcommand that replaces the last raw-tmux recommendation (`tmux display-message`) in operator workflows.

## Success Criteria

- [x] `cafleet member delete` (default path) blocks until the target pane disappears from `tmux list-panes`, up to a 15 s timeout.
- [x] On timeout, `member delete` captures the pane buffer tail, prints it + a cafleet-native recovery hint on stderr, and exits with code **2** (distinct from the generic `1`). (Unit-level via TestTimeout; end-to-end smoke 4 unreproducible because claude /exit preempts cleanly.)
- [x] `cafleet member delete --force` skips `/exit`, immediately kill-panes the target, deregisters the agent, rebalances the layout, and exits `0`.
- [x] `cafleet doctor` prints `session_name`, `window_id`, `pane_id`, and `TMUX_PANE` in both text and `--json` formats, requires `TMUX`, and does NOT require `--session-id`.
- [x] Every documented reference to raw `tmux kill-pane` (README.md, docs/spec/cli-options.md, cli.py warning text, skills/cafleet/SKILL.md) is replaced with the `--force` or capture+send-input recovery path.
- [x] Every documented reference to raw `tmux display-message` (skills/cafleet/SKILL.md "Rule: use cafleet primitives only" paragraph) is replaced with `cafleet doctor`.
- [x] New tmux primitives `pane_exists`, `kill_pane`, `wait_for_pane_gone` exist in `cafleet/src/cafleet/tmux.py` with unit tests.
- [x] All existing tests continue to pass; `test_cli_member_delete.TestTmuxErrorOnSendExit` is rewritten to assert the new wording; new tests cover happy-path-wait, timeout-with-capture, `--force`, and the pane-already-gone race.
- [x] `mise //cafleet:lint` / `:typecheck` / `:test` all pass.

---

## Background

### Current state

- `cafleet/src/cafleet/cli.py` `member_delete` (lines 620–676) deregisters the agent first, sends `/exit`, rebalances the layout, and returns. It does NOT verify the pane actually closed.
- `cafleet/src/cafleet/tmux.py` exposes `send_exit`, `capture_pane`, `select_layout`, `send_poll_trigger`, `send_choice_key`, `send_freetext_and_submit`, `split_window`, `director_context`, and `ensure_tmux_available`. There is no existence-check primitive and no kill-pane primitive.
- Two raw-tmux escape hatches remain in docs/code:
  1. `tmux display-message` — documented in `skills/cafleet/SKILL.md:565` as "the only direct tmux command that remains allowed" for reading Director session/pane metadata at startup. `cafleet.tmux.director_context()` already wraps it, but operators and skill docs still sometimes reach for the raw command.
  2. `tmux kill-pane` — referenced in `cafleet/src/cafleet/cli.py:660`, `README.md:136`, and `docs/spec/cli-options.md:197` as the manual fallback when `cafleet member delete` can't close a pane. Operators run it to recover, bypassing cafleet's authorization boundary, layout rebalance, and DB bookkeeping.

### The stuck-teardown bug

1. Director calls `cafleet member delete --member-id X`.
2. CLI deregisters agent X (DB state: X is gone).
3. CLI sends `/exit` via `tmux send-keys`.
4. Claude in the target pane shows `"You have a background process running — exit anyway? (y/n)"`. The pane does NOT close.
5. CLI returns `exit 0` — Director believes X is cleanly gone.
6. Director calls `cafleet session delete`, which succeeds (the DB already shows X deregistered).
7. The `claude` process keeps running in an orphan tmux pane with no registry row.

### Relationship to 0000014

[0000014 hikyaku-member-lifecycle](../0000014-hikyaku-member-lifecycle/design-doc.md) (Complete) specifies the deregister-first ordering at §1028–§1045 with the rationale "if deregister fails, preserve the pane for retry". That rationale assumed `/exit` synchronously closes the pane, so a send-keys failure is the only fault mode worth handling. Under the stuck-prompt fault mode, deregister-first leaves the registry inconsistent with reality. This design OVERRIDES that invariant (see Specification §4).

---

## Specification

### 1. New `cafleet.tmux` primitives

Three additions to `cafleet/src/cafleet/tmux.py`. The tmux format strings below use single-braced `#{pane_id}` — these end up in actual Python source where `str.format()` is never invoked on them, so they stay as-is. The surrounding design-doc prose intentionally writes the specifier literally because future readers implementing this doc should paste it verbatim.

```python
def pane_exists(*, target_pane_id: str) -> bool:
    """Return True iff target_pane_id currently appears in the tmux server's pane list.

    Uses `tmux list-panes -a` (all sessions on the server) so the check stays
    correct even if the pane somehow migrated to a different window.
    """
    out = _run(["tmux", "list-panes", "-a", "-F", "#{pane_id}"])
    return target_pane_id in out.split()
```

```python
def kill_pane(*, target_pane_id: str, ignore_missing: bool = False) -> None:
    """Unconditionally kill the target pane. Swallows pane-gone errors when ignore_missing=True.

    Same error-swallowing pattern as ``send_exit``: reuse _PANE_GONE_MARKERS.
    """
    try:
        _run(["tmux", "kill-pane", "-t", target_pane_id])
    except TmuxError as exc:
        if ignore_missing and any(m in str(exc).lower() for m in _PANE_GONE_MARKERS):
            return
        raise
```

```python
def wait_for_pane_gone(
    *, target_pane_id: str, timeout: float = 15.0, interval: float = 0.5
) -> bool:
    """Poll ``pane_exists`` until the pane is absent or the timeout elapses.

    Returns True if the pane disappeared, False on timeout. Uses
    ``time.monotonic()`` so tests can monkeypatch it deterministically.
    Errors from ``pane_exists`` propagate as TmuxError (caller decides).
    """
    deadline = time.monotonic() + timeout
    while True:
        if not pane_exists(target_pane_id=target_pane_id):
            return True
        if time.monotonic() >= deadline:
            return False
        time.sleep(interval)
```

Rationale for the defaults:

| Default | Value | Rationale |
|---|---|---|
| `timeout` | `15.0` s | `claude /exit` typically completes in 1–3 s. 15 s gives headroom for slow-disk background saves without stalling the Director's shutdown loop for long enough to be painful. Operators who need faster escalation use `--force`. |
| `interval` | `0.5` s | Fast enough that a stuck pane is detected within a human-noticeable beat (worst-case overshoot ~500 ms); slow enough that a typical 2 s `/exit` incurs 4 polls, not 40. |

### 2. `cafleet doctor` subcommand

New top-level click command. Intended as a future health-check namespace (DB connectivity, orphan-placement scan, etc.); this doc covers tmux metadata only.

```bash
cafleet doctor
cafleet --json doctor
```

| Flag | Required | Notes |
|---|---|---|
| `--json` | no | Global `--json`, placed before the subcommand (same pattern as every other CLI command). |
| `--session-id` | no | Silently accepted and ignored, matching `db init` / `session *` / `server`. |

Environment requirements:

- `TMUX` env var must be set — the command rejects otherwise with `Error: cafleet member commands must be run inside a tmux session` (reuse the existing `tmux.ensure_tmux_available()` wording). Treat `doctor` as a member-family command for this guard even though it does not target a member.
- `TMUX_PANE` env var must be set — this is required by `tmux.director_context()` already.

**Error-wording reuse justification.** `doctor` reuses `ensure_tmux_available()`'s existing `cafleet member commands must be run inside a tmux session` message verbatim rather than parametrizing the helper. The exact wording is a minor operator-facing lie (`doctor` is not a member command) but avoiding a new parameterization and the message-churn change to every existing test that asserts this string is worth the cost.

Text output:

```
tmux:
  session_name:  main
  window_id:     @3
  pane_id:       %0
  TMUX_PANE:     %0
```

JSON output:

```json
{
  "tmux": {
    "session_name": "main",
    "window_id": "@3",
    "pane_id": "%0",
    "tmux_pane_env": "%0"
  }
}
```

Implementation detail: reuse `tmux.director_context()` for the first three fields; read `TMUX_PANE` directly from `os.environ` for the fourth field (it is already validated non-empty by `director_context`).

### 3. `cafleet member delete` — blocking default + `--force` escalation

New `--force / -f` click bool flag (default `False`). Authorization and placement-loading logic (cross-Director check, missing-placement hint) are unchanged.

**Default path (no `--force`)** — pane_id is non-None:

| Step | Action | On failure |
|---|---|---|
| 1 | Load member + placement + auth check (unchanged from today). | exit 1 with the existing error wording. |
| 2 | `tmux.send_exit(target_pane_id=pane_id, ignore_missing=True)`. | exit 1 — see send_exit-failure row in the exit-1 table below. Do NOT deregister (with option-(b) ordering, deregister has not yet happened, so no rollback is needed). |
| 3 | `tmux.wait_for_pane_gone(target_pane_id=pane_id, timeout=15.0, interval=0.5)`. Returns True/False. | TmuxError (unrelated to pane-gone, e.g. tmux server crash) → exit 1 — see wait-failure row in the exit-1 table below. |
| 4a | If step 3 returned **True**: `broker.deregister_agent(member_id)` → `tmux.select_layout(...)` → exit 0 with "Member deleted." output. | deregister failure → exit 1 ("Error: deregister failed: {exc}"); layout-rebalance failure → warn, still exit 0. |
| 4b | If step 3 returned **False** (timeout): `tmux.capture_pane(target_pane_id=pane_id, lines=80)` → print the timeout error + tail + recovery hint on stderr → exit 2. | capture_pane raising TmuxError → print `Warning: capture_pane failed during timeout handling: {exc}. The timeout error and recovery hint still print.` to stderr via `click.echo(..., err=True)`, then still print the primary timeout error line and recovery hint, then exit 2. The exit code stays 2 regardless of the tail's availability (timeout is the dominant signal). |

**Pending placement path** (pane_id is None): deregister immediately, skip all tmux calls, exit 0. Unchanged from today.

**`--force` path** — pane_id is non-None:

| Step | Action |
|---|---|
| 1 | Load member + placement + auth check. |
| 2 | `tmux.kill_pane(target_pane_id=pane_id, ignore_missing=True)`. `/exit` is NOT sent. |
| 3 | `broker.deregister_agent(member_id)`. |
| 4 | `tmux.select_layout(...)` (layout rebalance; warn-only on failure). |
| 5 | exit 0 with "Member deleted (--force)." output. |

**`--force` with pending placement**: deregister immediately, skip all tmux calls, exit 0. Identical to today's pending-placement path but issued under the `--force` flag.

#### Output wording

Happy path (default):
```
Member deleted.
  agent_id:  <target-uuid>
  pane_id:   %7 (closed)
```

Happy path (`--force`):
```
Member deleted (--force).
  agent_id:  <target-uuid>
  pane_id:   %7 (killed)
```

Timeout (stderr, exit 2):
```
Error: pane %7 did not close within 15.0s after /exit.
--- pane %7 tail (last 80 lines) ---
<captured terminal buffer>
---
Recovery: inspect with `cafleet member capture`, answer any prompt with `cafleet member send-input`, then re-run `cafleet member delete`. Or re-run with `--force` to skip the wait and kill the pane.
```

JSON outputs mirror the current shape — add `"pane_status"` values `"(closed)"`, `"(killed)"`, or `"(timeout)"` as appropriate.

#### Exit codes (three-way split)

| Exit | When |
|---|---|
| `0` | Success — default path pane-gone confirmed, `--force` pane killed, or pending-placement deregister. |
| `1` | Any non-timeout failure: auth rejection, missing session, unknown member-id, `broker.deregister_agent` failure, `send_exit` tmux failure (pre-poll), `tmux.wait_for_pane_gone` raising TmuxError (server crash mid-poll). |
| `2` | Default-path timeout — `/exit` was sent, the pane did not disappear within 15.0 s, buffer tail has been printed on stderr. |

The Director's shutdown loop branches on `2` specifically: exit 2 means "pane is stuck, take recovery action (capture+send-input or --force)"; exit 1 means "something else is wrong, don't auto-retry".

#### Exit-1 stderr strings (every distinguishable cause)

The Director's shutdown loop cannot distinguish exit-1 causes without concrete messages. Every exit-1 path prints exactly ONE of these lines to stderr (plus click's "Error: " prefix where applicable):

| Cause | Exact stderr string |
|---|---|
| Cross-director auth rejection (existing) | `Error: agent <member-id> is not a member of your team (director_agent_id=<other-director>).` |
| Missing `--session-id` (existing) | `Error: --session-id <uuid> is required for this subcommand. Create a session with 'cafleet session create' and pass its id.` |
| Unknown `--member-id` / no placement (existing) | `Error: agent <member-id> has no placement; use 'cafleet deregister' instead` |
| `broker.deregister_agent` failure (existing) | `Error: deregister failed: {exc}` |
| `send_exit` TmuxError (NEW — replaces the current warning-and-continue) | `Error: send_exit failed for pane {pane_id}: {exc}. The tmux server may be unreachable. Verify with 'cafleet doctor', then re-run 'cafleet member delete', or use '--force' to kill the pane directly.` |
| `wait_for_pane_gone` / `pane_exists` TmuxError during polling (NEW) | `Error: tmux call failed while waiting for pane {pane_id} to close: {exc}` |

Exit-2 output (timeout) is NOT in this table — it is specified in §3's "Timeout (stderr, exit 2)" block above and is the only path that prints the pane-buffer tail.

### 4. Deregister-ordering change (overrides 0000014)

Under [0000014](../0000014-hikyaku-member-lifecycle/design-doc.md) §1028–§1045, deregister runs BEFORE `send_exit` so a send-keys failure leaves a queryable intent. Under this design, deregister runs AFTER confirmed pane closure (default path) or AFTER `kill_pane` (`--force` path).

Rationale:

- With blocking-until-real-shutdown semantics, the timeout IS the failure signal. A pre-emptive deregister no longer records useful intent — it only creates DB/tmux inconsistency when the timeout fires.
- Deferring deregister means `--force` is a pure atomic retry of `kill_pane → deregister → layout`. The `--force` path does NOT need to handle a missing registry row as a separate case.
- "Loud failure = nothing happened" is easier for the Director to reason about than "registry says gone but pane is still there".

Every code comment, test assertion, and doc paragraph that asserts the old invariant must be updated in Step 1 (documentation) and Step 4 (code). Enumerated:

| Artifact | Current assertion | New assertion |
|---|---|---|
| `cafleet/src/cafleet/cli.py:644` | `# Deregister the registry row first so a send-keys failure leaves a queryable intent ("already gone") rather than a dangling placement.` | Remove. Replace with a one-line pointer to 0000032 if the new ordering is non-obvious at the call site. |
| `cafleet/tests/test_cli_member_delete.py:117–126` (`TestHappyPath::test_deregisters_and_sends_exit_to_pane`) | asserts `deregister_recorder == [MEMBER_ID]` while stubbing only `send_exit`. Implicitly asserts deregister runs regardless of pane state. | Assert send_exit runs, pane_exists is polled, pane-gone is confirmed, THEN deregister runs. Ordering asserted via a call-sequence list. |
| `ARCHITECTURE.md:157` | `**Delete ordering** ... Deregister the agent first, THEN /exit the pane. This preserves the pane for retry if deregister fails.` | `**Delete ordering** (default path): send /exit, poll list-panes until the pane disappears (15 s timeout), then deregister, then rebalance layout. On timeout, capture the pane tail and fail loudly with exit code 2; operator reruns with --force for an atomic kill+deregister. This overrides the 0000014 deregister-first invariant — see design-docs/0000032-robust-member-teardown/design-doc.md §4.` |
| `skills/cafleet/SKILL.md:262` | `The agent is deregistered FIRST, then /exit is sent to the pane — so a deregister failure leaves both intact for retry.` | `The CLI sends /exit, polls tmux list-panes for the target pane_id until it disappears (15 s timeout), then deregisters the agent and rebalances the layout. On timeout, the pane buffer tail is captured and printed on stderr, and the command exits 2 without deregistering. Rerun with --force to skip /exit and kill the pane immediately.` |
| `skills/cafleet/SKILL.md:559` | `The command deregisters the agent first (so a failure preserves the pane for retry), then sends /exit to the pane, then rebalances the layout.` | Same rewrite as the 262 paragraph (consistent wording in both places). |
| `design-docs/0000014-hikyaku-member-lifecycle/design-doc.md` | Historical record — Status=Complete. | Do NOT rewrite. The override is documented here; 0000014 stays as-is. |

### 5. Documentation updates (Step 1 checklist)

Every file below must be updated BEFORE any code change, per `.claude/rules/design-doc-numbering.md`.

| File | Target | Change |
|---|---|---|
| `ARCHITECTURE.md` | line 157 | Rewrite "Delete ordering" paragraph per §4. |
| `ARCHITECTURE.md` | line 163 (after `The tmux helper module ... isolates all subprocess interaction with tmux.`) | Append one sentence in the same paragraph: `Primitives for pane lifecycle inspection and forced teardown — pane_exists, kill_pane, and wait_for_pane_gone — live here so the CLI never calls tmux directly.` (The member-subcommand list in the same line is NOT touched — `doctor` is a top-level command and is documented in its own paragraph below.) |
| `ARCHITECTURE.md` | new paragraph inserted immediately after the existing `**Write-path authorization mirrors the read path** ...` paragraph at :165 | Add a new bold-lead paragraph analogous to :165: `**Operator diagnostics**: cafleet doctor prints the calling pane's session/window/pane identifiers (plus $TMUX_PANE) for operators diagnosing placement issues without reaching for raw tmux commands. It is a top-level command — not a member-family command — but reuses tmux.ensure_tmux_available() so the TMUX-required wording stays consistent with the member surface.` |
| `README.md` | line 136 | Replace `Surviving claude / codex processes can be terminated manually with tmux kill-pane.` with: `If a member pane refuses to close (e.g. blocked on a confirmation prompt), rerun cafleet member delete with --force, which kill-panes the target, sweeps the placement, and rebalances the layout.` |
| `docs/spec/cli-options.md` | line 197 | Same replacement as README.md:136. |
| `docs/spec/cli-options.md` | `member delete` subsection | Add `--force` to the flags table. Add a new exit-code table documenting the 0/1/2 split. Add a "polling contract" paragraph covering the 15 s timeout, 500 ms interval, and timeout output shape. |
| `docs/spec/cli-options.md` | new subsection between `session delete` and `server` | Add `cafleet doctor` spec — flags, env requirements, text output, JSON output, exit codes. |
| `cafleet/src/cafleet/cli.py` | line 660 branch | This branch handles `send_exit` TmuxError (transport/server failure — NOT the stuck-prompt case; a stuck prompt never raises TmuxError because /exit is just send-keys). Convert from warning-and-continue to a hard failure: raise `click.ClickException("send_exit failed for pane {pane_id}: {exc}. The tmux server may be unreachable. Verify with 'cafleet doctor', then re-run 'cafleet member delete', or use '--force' to kill the pane directly.")`. The stuck-prompt wording is a DIFFERENT recovery string emitted only by the timeout branch — do NOT reuse it here. |
| `skills/cafleet/SKILL.md` | line 262 | Update member-delete intro per §4 table. |
| `skills/cafleet/SKILL.md` | line 559 | Same rewrite. |
| `skills/cafleet/SKILL.md` | Member Delete section flag table (around line 269) | Add `--force` row: `no | Skip the /exit wait. Immediately kill-pane the target, then deregister, then rebalance layout. Exit 0 even if the pane was already gone.` Add exit-code table 0/1/2. |
| `skills/cafleet/SKILL.md` | line 565 | Drop the `tmux display-message remains allowed` exception. New paragraph: `**Rule: use cafleet primitives only.** All tmux interactions — write, inspect, and metadata — are encapsulated by cafleet commands. For tmux session/window/pane metadata at Director startup, use `cafleet doctor`. Never invoke `tmux send-keys`, `tmux kill-pane`, `tmux list-panes`, `tmux capture-pane`, or `tmux display-message` directly from the Director. If a workflow appears to need a raw tmux call, file a gap in `cafleet member *` or `cafleet doctor` — NOT a raw tmux invocation.` |
| `skills/cafleet/SKILL.md` | Shutdown Protocol step 2 (around line 568) | Expand: `**Delete every member** via `cafleet --session-id <s> member delete --agent-id <d> --member-id <m>`. This call now blocks until the target pane is actually gone (15 s default timeout). If the pane is stuck on a prompt, the command exits 2 with the pane buffer tail on stderr — inspect with `cafleet member capture`, answer the prompt with `cafleet member send-input --choice N` or `--freetext`, then re-run `cafleet member delete`. If the pane is truly wedged, escalate to `cafleet member delete --force`, which skips /exit and kill-panes immediately. Do NOT fall back to raw `tmux kill-pane`.` |
| `skills/cafleet/SKILL.md` | new "cafleet doctor" Command Reference subsection | Added near the top of the CLI reference (paired with Register / Send / Poll sections). Document flags, output, env requirements. |
| `skills/cafleet-monitoring/SKILL.md` | line 14 (Agent-agnostic monitoring paragraph) | Change `cafleet member delete sends /exit regardless of which coding agent is running in the pane` → `cafleet member delete sends /exit, waits for the pane to close (15 s timeout), then deregisters — regardless of which coding agent is running in the pane.` |
| `skills/cafleet-monitoring/SKILL.md` | line 38 teardown paragraph | Add: `cafleet member delete now blocks until the pane is actually gone (15 s default timeout). On timeout (exit 2), inspect + answer the prompt via member capture + send-input, or escalate to --force.` |
| `skills/cafleet-monitoring/SKILL.md` | Stall Response escalation table (around line 78) | Add a new row: `cafleet ... member delete --force` | Interactive, destructive | When member delete has already exited 2 and capture + send-input have failed to unblock the pane — forces an atomic kill + deregister. |
| `skills/design-doc-create/SKILL.md` | Step 6 (around line 314) | Add a one-line callout below the `member delete` lines: `Each member delete now blocks until the pane is actually gone (15 s default timeout). On exit 2, inspect with cafleet member capture and answer with cafleet member send-input, then retry — or rerun with --force.` |
| `skills/design-doc-execute/SKILL.md` | Step 6 teardown, after the `member delete` bash fence closes at line 634 | Same one-line callout below the per-member delete lines. |

### 6. Test plan

#### `cafleet/tests/test_tmux.py` — new test classes

| Class | Cases |
|---|---|
| `TestPaneExists` | (a) pane present in `tmux list-panes -a -F "#{pane_id}"` output → True. (b) pane absent → False. (c) tmux stderr returning an error UNRELATED to pane-gone (e.g. `server exited unexpectedly`) propagates as `TmuxError`, NOT silently False. |
| `TestKillPane` | (a) happy path: invokes `tmux kill-pane -t <pane>`. (b) `ignore_missing=True` with a `_PANE_GONE_MARKERS` stderr swallows the error. (c) `ignore_missing=True` with an unrelated error propagates (server crash). (d) default `ignore_missing=False` always raises. |
| `TestWaitForPaneGone` | (a) `pane_exists` returns False on first call → True, immediate. (b) `pane_exists` returns True, True, False → True, <1 s elapsed (monkeypatch `time.monotonic` + `time.sleep`). (c) `pane_exists` returns True forever → False, exactly `timeout / interval + 1` polls. (d) `pane_exists` raises TmuxError mid-wait → TmuxError propagates. |

#### `cafleet/tests/test_cli_member_delete.py` — updates + new classes

| Class | Cases |
|---|---|
| `TestHappyPath` (update) | Stub `pane_exists` to return False on first call + `wait_for_pane_gone` to return True. Assert call ordering: `send_exit` first, then `wait_for_pane_gone`, then `broker.deregister_agent`, then `select_layout`. Existing JSON-output test updates `pane_status` to `"(closed)"`. |
| `TestTimeout` (new) | Stub `wait_for_pane_gone` to return False, `capture_pane` to return `"<tail>"`. Assert: exit code 2, stderr contains `did not close within 15.0s`, stderr contains the captured tail, stderr contains the recovery hint ("inspect with cafleet member capture", "rerun with --force"), deregister was NOT called, layout rebalance was NOT called. |
| `TestForce` (new) | Pass `--force` flag. Stub `kill_pane`. Assert: `send_exit` NOT called, `kill_pane` called with `ignore_missing=True`, `deregister` called after `kill_pane`, exit code 0, output contains `(killed)` and `--force`. JSON output has `"pane_status": "(killed)"`. |
| `TestPaneAlreadyGone` (new) | `pane_exists` returns False on the very first poll (pane was already gone before we sent /exit). Assert: exit 0, happy-path output, NO `capture_pane` call. |
| `TestTmuxErrorOnSendExit` (rewrite) | Drop the assertion on `tmux kill-pane -t <pane>` warning text. Assert the new wording: `cafleet member capture`, `cafleet member send-input`, `--force` all appear in the warning; raw `tmux kill-pane` does NOT appear. |
| `TestPendingPlacementForce` (new) | `--force` with pane_id=None placement → deregister only, exit 0, no tmux calls attempted. |

#### `cafleet/tests/test_cli_doctor.py` — new file

| Test | Assertion |
|---|---|
| `test_text_output_has_all_four_fields` | Output contains `session_name:`, `window_id:`, `pane_id:`, `TMUX_PANE:` — each with a non-empty value. |
| `test_json_output_shape` | `json.loads(output)` matches `{"tmux": {"session_name": ..., "window_id": ..., "pane_id": ..., "tmux_pane_env": ...}}`. |
| `test_outside_tmux_exits_one` | `monkeypatch.delenv("TMUX")` → exit 1 with the existing `must be run inside a tmux session` wording. |
| `test_session_id_flag_silently_ignored` | Passing `--session-id <uuid>` succeeds (doctor is listed in the "no-session-id-required" subcommand set). |

#### Integration / smoke (manual, in Step 6)

1. Spawn a member, run `cafleet member delete`; confirm it blocks briefly then returns exit 0.
2. Spawn a member; in the pane, run `sleep 300` (or trigger a prompt that won't auto-dismiss); run `cafleet member delete`; confirm exit 2 with buffer tail on stderr.
3. Same stuck pane; run `cafleet member delete --force`; confirm immediate exit 0 and the pane is gone.
4. Run `cafleet doctor` inside tmux; confirm the four fields print and match `director_context()` output.
5. Run `cafleet doctor` outside tmux; confirm exit 1 with the expected wording.

### 7. Non-goals

1. **No prompt-pattern detection.** The CLI never parses pane text to auto-detect "background process running — exit anyway?" or any other prompt shape. Operators decide via capture + send-input; the code only watches `pane_id` presence.
2. **No PID tracking.** The CLI does not look up claude/codex process IDs. The sole shutdown signal is the absence of the pane from `tmux list-panes`.
3. **No auto-answering of prompts.** `cafleet member delete` never sends keystrokes other than `/exit` (default) or `kill-pane` (`--force`). Unblocking a prompt is always an explicit operator action via `cafleet member send-input`.
4. **No change to `cafleet session delete`.** Step 4 of the Shutdown Protocol in `skills/cafleet/SKILL.md` continues to sweep residual agents in the same transaction; this doc does not alter that behavior.
5. **`cafleet doctor` scope limited to tmux metadata.** The `doctor` namespace is intended as the home for future health checks — DB connectivity probes, orphan-placement scans, session-row integrity checks, `.cafleet` config validation — but those are explicitly deferred. This doc implements the tmux metadata surface only.

### 8. Backwards-incompatible behavior change

**`cafleet member delete` default behavior changes from fire-and-forget to blocking-until-real-shutdown.** Return time goes from <1 s to typically 1–3 s (up to 15 s on slow `/exit`, or exit 2 on timeout). This is a deliberate behavior change, documented here.

**Behavior changes (THIS section) vs documentation rewrites (§5).** See §5 for the full documentation rewrite matrix — §8 focuses on the behavior changes only and does not re-enumerate doc targets. The code-behavior changes NOT in §5 are:

1. `cafleet member delete` default path no longer returns `exit 0` before the pane is confirmed closed. Shutdown loops that previously assumed "`member delete` returns immediately" must tolerate a 1–15 s wait, and must branch on the new exit code 2.
2. The `send_exit` TmuxError path changes from `warning-and-continue (exit 0)` to `click.ClickException (exit 1)` with new stderr wording (see §3 exit-1 table). Anything that grepped the old `Warning: send_exit failed ...` string will no longer find it.
3. The JSON output of `cafleet member delete` gains a new `pane_status` enum value: `"(timeout)"` (alongside the existing `"(closed)"`, `"(pending — no pane)"`) plus a new `"(killed)"` for the `--force` path. Consumers that pattern-match `pane_status` must handle both new values. The JSON output also loses the `"(send_exit failed)"` value — the transport-error path now exits 1 before JSON emission.
4. Under option-(b) ordering, `broker.deregister_agent` runs AFTER confirmed pane closure. Crash-during-deregister scenarios now leave a dead pane with an intact registry row (previously the pane was already gone at that point). The new behavior is strictly better — `cafleet deregister --agent-id <member>` cleans up.

**No DB schema change. No Alembic migration.** The `agent_placements` table already carries `tmux_pane_id`, which is all the polling loop needs.

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

### Step 1: Documentation updates (do FIRST, per `.claude/rules/design-doc-numbering.md`)

- [x] Rewrite `ARCHITECTURE.md:157` Delete ordering paragraph per §4 and per the §5 ARCHITECTURE.md:157 row (send /exit → poll → deregister; timeout → exit 2 + captured tail; --force path). <!-- completed: 2026-04-19T06:40 -->
- [x] Append the one-sentence primitive note after `ARCHITECTURE.md:163` per the §5 ARCHITECTURE.md:163 row (`Primitives for pane lifecycle inspection and forced teardown — pane_exists, kill_pane, and wait_for_pane_gone — live here so the CLI never calls tmux directly.`). Same paragraph — the member-subcommand list is NOT touched. <!-- completed: 2026-04-19T06:40 -->
- [x] Insert the new `**Operator diagnostics**:` paragraph immediately after `ARCHITECTURE.md:165` per the §5 ARCHITECTURE.md:165 row (one sentence introducing `cafleet doctor` as a top-level command for placement diagnostics, explicitly noting it is NOT a member-family command). <!-- completed: 2026-04-19T06:40 -->
- [x] Update `README.md:136` — replace the tmux-kill-pane escape-hatch line with the `--force` recovery language from §5. <!-- completed: 2026-04-19T06:40 -->
- [x] Update `docs/spec/cli-options.md:197` — same replacement. <!-- completed: 2026-04-19T06:40 -->
- [x] Update `docs/spec/cli-options.md` member-delete subsection — add `--force` to flag table, add 0/1/2 exit-code table, add polling-contract paragraph (15 s timeout, 500 ms interval, timeout output shape). <!-- completed: 2026-04-19T06:40 -->
- [x] Add `cafleet doctor` subsection to `docs/spec/cli-options.md` between `session delete` and `server` — flags, env requirements, text output, JSON output, exit codes. <!-- completed: 2026-04-19T06:40 -->
- [x] Update `skills/cafleet/SKILL.md:262` — member-delete intro per §4. <!-- completed: 2026-04-19T06:40 -->
- [x] Update `skills/cafleet/SKILL.md:559` — same rewrite (keep wording identical to :262). <!-- completed: 2026-04-19T06:40 -->
- [x] Update `skills/cafleet/SKILL.md` Member Delete flag table (around :269) — add `--force` row, add exit-code table. <!-- completed: 2026-04-19T06:40 -->
- [x] Update `skills/cafleet/SKILL.md:565` — drop "tmux display-message remains allowed" exception, point at `cafleet doctor` for metadata. <!-- completed: 2026-04-19T06:40 -->
- [x] Update `skills/cafleet/SKILL.md` Shutdown Protocol step 2 (around :568) — document blocking-until-real-shutdown, capture+send-input+retry, `--force` escalation. <!-- completed: 2026-04-19T06:40 -->
- [x] Add `cafleet doctor` Command Reference subsection to `skills/cafleet/SKILL.md` near the top of the CLI reference. <!-- completed: 2026-04-19T06:40 -->
- [x] Update `skills/cafleet-monitoring/SKILL.md:14` — `cafleet member delete` now waits for pane-gone before deregistering. <!-- completed: 2026-04-19T06:40 -->
- [x] Update `skills/cafleet-monitoring/SKILL.md` teardown paragraph near :38 — blocking semantics and `--force` escalation mention. <!-- completed: 2026-04-19T06:40 -->
- [x] Update `skills/cafleet-monitoring/SKILL.md` Stall Response escalation table near :78 — add `--force` row. <!-- completed: 2026-04-19T06:43 -->
- [x] Update `skills/design-doc-create/SKILL.md` Step 6 (around :314) — one-line callout on blocking member delete + timeout recovery. <!-- completed: 2026-04-19T06:44 -->
- [x] Update `skills/design-doc-execute/SKILL.md` Step 6 teardown — insert the one-line callout immediately after the `member delete` bash fence closes at line 634 (verified: the fence spans :630–:634). <!-- completed: 2026-04-19T06:44 -->

### Step 2: tmux primitives

- [x] Add `pane_exists(*, target_pane_id: str) -> bool` to `cafleet/src/cafleet/tmux.py`. Wraps `tmux list-panes -a -F "#{pane_id}"`. <!-- completed: 2026-04-19T08:02 -->
- [x] Add `kill_pane(*, target_pane_id: str, ignore_missing: bool = False) -> None` to `cafleet/src/cafleet/tmux.py`. Wraps `tmux kill-pane -t <pane>`. Reuse `_PANE_GONE_MARKERS` for `ignore_missing` handling. <!-- completed: 2026-04-19T08:02 -->
- [x] Add `wait_for_pane_gone(*, target_pane_id: str, timeout: float = 15.0, interval: float = 0.5) -> bool` to `cafleet/src/cafleet/tmux.py`. Use `time.monotonic()` + `time.sleep()` for monkeypatch-friendly tests. <!-- completed: 2026-04-19T08:02 -->

### Step 3: `cafleet doctor` subcommand

- [x] Add `doctor` top-level click command to `cafleet/src/cafleet/cli.py`. Require TMUX via `tmux.ensure_tmux_available()`. Mark `doctor` as a no-session-id-required subcommand (same handling as `db init` / `session *` / `server`). <!-- completed: 2026-04-19T08:18 -->
- [x] Implement text and `--json` output per Specification §2 — session_name / window_id / pane_id from `tmux.director_context()`, TMUX_PANE from `os.environ`. <!-- completed: 2026-04-19T08:18 -->

### Step 4: `cafleet member delete` blocking + `--force`

- [x] Add `--force / -f` click bool flag (default False) to `member_delete`. <!-- completed: 2026-04-19T08:35 -->
- [x] Rewrite `member_delete` body per Specification §3:
  - pane_id is None → deregister only, exit 0 (both default and `--force` branches share this path).
  - default path → `send_exit` → `wait_for_pane_gone` → on True: `deregister_agent` → `select_layout` → exit 0; on False: `capture_pane` → stderr error + tail + recovery → exit 2.
  - `--force` path → `kill_pane(ignore_missing=True)` → `deregister_agent` → `select_layout` → exit 0. <!-- completed: 2026-04-19T08:35 -->
- [x] Remove the `# Deregister the registry row first so a send-keys failure leaves a queryable intent` comment at `cafleet/src/cafleet/cli.py:644`; do not replace with an equivalent inverse comment (new ordering is obvious from the adjacent `send_exit` → `wait_for_pane_gone` → `deregister_agent` call order, so no comment is needed). <!-- completed: 2026-04-19T08:35 -->
- [x] Convert the `send_exit` TmuxError branch at `cafleet/src/cafleet/cli.py:657-662` from warning-and-continue to a hard failure: raise `click.ClickException(f"send_exit failed for pane {pane_id}: {exc}. The tmux server may be unreachable. Verify with 'cafleet doctor', then re-run 'cafleet member delete', or use '--force' to kill the pane directly.")`. Remove the `pane_status = f"{pane_id} (send_exit failed)"` literal entirely. Under option-(b) ordering `broker.deregister_agent` has not yet run at this point, so no rollback is needed — just exit 1. <!-- completed: 2026-04-19T08:35 -->
- [x] Wire the three-way exit code split: rely on `click.ClickException` for exit 1 (all six causes in the §3 exit-1 table), emit an explicit `ctx.exit(2)` on the timeout branch. <!-- completed: 2026-04-19T08:35 -->

### Step 5: Tests

- [x] Add `cafleet/tests/test_tmux.py::TestPaneExists` — pane present / absent / unrelated-error propagation (3 cases). <!-- completed: 2026-04-19T08:02 -->
- [x] Add `cafleet/tests/test_tmux.py::TestKillPane` — happy / ignore_missing-swallows / ignore_missing-propagates-other / default-raises (4 cases). <!-- completed: 2026-04-19T08:02 -->
- [x] Add `cafleet/tests/test_tmux.py::TestWaitForPaneGone` — first-poll-gone / mid-wait-gone / never-gone-times-out / TmuxError-propagates (4 cases; monkeypatch `time.monotonic` and `time.sleep`). <!-- completed: 2026-04-19T08:02 -->
- [x] Update `cafleet/tests/test_cli_member_delete.py::TestHappyPath` — stub pane_exists + wait_for_pane_gone; assert call ordering (send_exit → wait → deregister); JSON output has `"pane_status": "(closed)"`. <!-- completed: 2026-04-19T08:35 -->
- [x] Add `cafleet/tests/test_cli_member_delete.py::TestTimeout` — wait_for_pane_gone returns False, capture_pane returns buffer; assert exit 2, stderr contains timeout message + tail + recovery hint, deregister NOT called. <!-- completed: 2026-04-19T08:35 -->
- [x] Add `cafleet/tests/test_cli_member_delete.py::TestForce` — `--force` flag; assert kill_pane called, send_exit NOT called, deregister called after kill_pane; exit 0; output contains `(killed)`. <!-- completed: 2026-04-19T08:35 -->
- [x] Add `cafleet/tests/test_cli_member_delete.py::TestPaneAlreadyGone` — pane_exists False on first poll; exit 0 happy path; NO capture_pane call. <!-- completed: 2026-04-19T08:35 -->
- [x] Rewrite `cafleet/tests/test_cli_member_delete.py::TestTmuxErrorOnSendExit` — drop raw tmux kill-pane assertion; assert cafleet-native recovery wording present. <!-- completed: 2026-04-19T08:35 -->
- [x] Add `cafleet/tests/test_cli_member_delete.py::TestPendingPlacementForce` — `--force` with pane_id=None → deregister only, no tmux calls. <!-- completed: 2026-04-19T08:35 -->
- [x] Add `cafleet/tests/test_cli_doctor.py` — text output has all four fields; JSON output shape; outside-tmux exits 1 with expected wording; `--session-id` silently ignored. <!-- completed: 2026-04-19T08:18 -->

### Step 6: Verification

- [x] `mise //cafleet:lint` passes. <!-- completed: 2026-04-19T08:35 -->
- [x] `mise //cafleet:typecheck` passes. <!-- completed: 2026-04-19T08:35 -->
- [x] `mise //cafleet:test` passes. <!-- completed: 2026-04-19T08:35 -->
- [x] `mise //cafleet:install` (editable reinstall so the `cafleet doctor` subcommand is on PATH for smoke tests). <!-- completed: 2026-04-19T08:42 -->
- [x] Smoke test: `cafleet doctor` inside tmux prints all four fields matching the current pane. <!-- completed: 2026-04-19T09:10 -->
- [x] Smoke test: happy-path `cafleet member delete` on a live member — blocks briefly, exits 0. <!-- completed: 2026-04-19T09:13 -->
- [x] Smoke test: stuck-pane `cafleet member delete` — UNREPRODUCIBLE end-to-end (claude /exit preempts its own thinking cleanly); authoritative coverage via `cafleet/tests/test_cli_member_delete.py::TestTimeout`. <!-- completed: 2026-04-19T09:14 -->
- [x] Smoke test: same stuck pane, `cafleet member delete --force` — exits 0 immediately, pane is gone. <!-- completed: 2026-04-19T09:14 -->

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-04-19 | Initial draft |
| 2026-04-19 | Round-1 revision: distinct send_exit-failure wording vs stuck-prompt wording; exit-1 stderr-string table; ARCHITECTURE.md insertion points split (:157, :163, :165) with concrete prose; ensure_tmux_available wording-reuse justification; capture_pane-failure channel/text specified; JSON `pane_status="(timeout)"` breaking-change note; design-doc-execute target retargeted to :634; §8 cross-references §5 instead of re-enumerating doc targets. |
| 2026-04-19 | Round-2 revision: Step 1 ARCHITECTURE.md task split into three distinct checkboxes (matches §5 rows); Progress header bumped to 0/46; §8 item 3 now also records the loss of `pane_status="(send_exit failed)"`. |
| 2026-04-19 | Finalized: Status → Approved after user sign-off. |
| 2026-04-19 | Step 1 implementation complete: 18 documentation edits applied across 8 files (ARCHITECTURE.md / README.md / docs/spec/cli-options.md / skills/cafleet/SKILL.md / skills/cafleet-monitoring/SKILL.md / skills/design-doc-create/SKILL.md / skills/design-doc-execute/SKILL.md). Partial crash mid-step: initial execute team's tmux panes died with a local laptop crash, Director completed the remaining 3 edits (cafleet-monitoring :78 --force row, design-doc-create Step 6 callout, design-doc-execute Step 6 callout) directly. Status bumped from Approved → In Progress; Progress header bumped to 18/46. A fresh execute team will pick up Step 2 (tmux primitives). |
| 2026-04-19 | Step 2 complete: pane_exists / kill_pane / wait_for_pane_gone added to cafleet/src/cafleet/tmux.py with 11 unit tests. Progress 21/46. |
| 2026-04-19 | Step 3 complete: cafleet doctor top-level subcommand added with 4 unit tests. Progress 23/46. |
| 2026-04-19 | Step 4 complete: cafleet member delete rewritten per Spec §3 with blocking-until-pane-gone default path (15s timeout / 500ms interval) + --force/-f short-circuit that calls kill_pane immediately. Three-way exit code split (0/1/2). Option-(b) deregister-after-pane-gone ordering (overrides 0000014). 10 new/rewritten cli tests + 11 tmux tests all pass. Progress 28/46. |
| 2026-04-19 | Step 5 complete: all 10 test checklist items ticked off (tests were committed during the TDD Phase A steps for Step 2/3/4). Progress 38/46. |
| 2026-04-19 | Step 6 complete: mise //cafleet:lint / :typecheck / :test (487/487) / :install all pass. Smoke tests 1-3 and 5 PASS end-to-end. Smoke 4 (stuck-pane 15s timeout) UNREPRODUCIBLE via naive expensive-prompt injection — claude /exit preempts its own thinking cleanly. Authoritative coverage for the timeout path stands via unit test TestTimeout in cafleet/tests/test_cli_member_delete.py. Progress 46/46. |
