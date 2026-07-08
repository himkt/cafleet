# Herdr member-delete graceful teardown

**Status**: Complete
**Progress**: 14/14 tasks complete
**Last Updated**: 2026-07-07

## Overview

`cafleet member delete` (the default, non-`--force` path) always times out on the herdr backend with `pane <id> did not close within 15.0s after /exit`, forcing the operator to fall back to `--force` for every member (issue #172). This design contains the fix inside `HerdrMultiplexer.wait_for_pane_gone`: after the exit keystroke, wait for the coding agent to actually exit (herdr `agent_status` → `"unknown"`, or `None` if the pane is already gone), then close the now-shell-only pane via `herdr pane close`. `cli/member.py` stays backend-neutral and the tmux path is untouched.

## Success Criteria

- [x] A live herdr-hosted member (claude / codex / opencode) is deletable via the default `cafleet member delete` path with exit 0 — no `--force` needed.
- [x] The graceful path waits for the coding agent to exit (herdr `agent_status` returns `"unknown"`, or `None` if the pane is already gone), then closes the shell pane via `herdr pane close`, then `cli/member.py` deregisters — within the existing 15.0 s / 0.5 s budget.
- [x] A herdr member whose agent does **not** exit within 15 s (e.g. blocked on an "exit anyway?" confirmation) still times out to exit 2 with the pane tail on stderr; no auto-force is performed, and `--force` remains the operator escalation.
- [x] The fix lives entirely in `HerdrMultiplexer.wait_for_pane_gone`; `cli/member.py`, the 15.0 s budget, and `HerdrMultiplexer.send_exit` are unchanged.
- [x] The tmux path is byte-for-byte unchanged; the herdr `--force` path is unchanged.
- [x] Documentation (SPEC.md §6.5, `docs/concepts/member-lifecycle.md`, `docs/spec/cli-options.md`) is updated first and stays drift-free.
- [x] `mise //cafleet:lint` / `:typecheck` / `:test` pass; herdr tests cover graceful teardown, already-gone, and no-exit-timeout.

---

## Background

### The base contract encodes a tmux assumption

`cli/member.py` `member_delete` (default path) runs a backend-neutral sequence:

```
mux.send_exit(target_pane_id=pane_id, ignore_missing=True)
gone = mux.wait_for_pane_gone(target_pane_id=pane_id, timeout=15.0, interval=0.5)
if gone: deregister; exit 0
else:    capture pane tail; print recovery hint; exit 2
```

This encodes the tmux fact that **an agent exit implies the pane is gone**:

- **tmux** — `split_window` runs the coding-agent argv as the pane's *foreground process* (`tmux split-window … <argv>`). `/exit` ends that process, so tmux auto-closes the pane. `TmuxMultiplexer.wait_for_pane_gone` polls `list-panes` and sees the pane disappear → returns `True`.

- **herdr** — `HerdrMultiplexer.split_window` calls `herdr pane split` (which creates a **persistent shell**) and then `herdr pane run <new_id> "<agent argv>"`, which *types the coding-agent command into that shell* (`cafleet/src/cafleet/multiplexer/herdr.py` `split_window` / `_split_pane`). `send_exit` → `herdr pane run <id> "/exit"` makes the coding agent exit **back to the shell**. The shell — and therefore the pane — survives. `HerdrMultiplexer.wait_for_pane_gone` today just polls `herdr pane get <id>` (absent → gone), so it polls uselessly for the full 15 s and returns `False`, driving `member_delete` into the exit-2 timeout path every time.

Issue #172 captures this exactly: three sequential herdr member deletes each report `pane wZ:pN did not close within 15.0s after /exit` and note "the monitor pane exited to shell but the graceful wait timed out," then succeed only under `--force`.

### Why herdr can do better than tmux here

herdr natively tracks each pane's agent lifecycle and already implements the `AgentStateAware` capability (`agent_status` / `wait_agent_status`). While a coding agent runs, herdr binds an `agent` field to the pane and reports a live state — `idle` / `working` / `blocked` / `done`. Once the agent exits back to the bare shell, herdr **drops the `agent` field and reports `agent_status: "unknown"`** — the distinct, non-live status that signals the agent process is gone. (`agent_status` returns `None` only when the pane is *fully gone* — the `pane_not_found` case — which does **not** happen after `/exit`, because the shell persists.) That `"unknown"` state — never emitted for a live agent, and distinct from a `blocked`/`done` agent that is still alive — is precisely the exit signal the tmux-shaped contract could not express.

### Precedent

Backend-specific exit-teardown deltas have been handled inside the backend before: [0000086-opencode-send-exit-keystroke-race](../0000086-opencode-send-exit-keystroke-race/design-doc.md) fixed an opencode-only `/exit` submit race entirely within `tmux.py`'s `send_exit`, leaving `cli/member.py` untouched. This design follows the same containment principle for herdr. The 15 s / 0.5 s budget and the exit-code contract come from [0000032-robust-member-teardown](../0000032-robust-member-teardown/design-doc.md); the herdr backend itself is [0000121-herdr-multiplexer-backend](../0000121-herdr-multiplexer-backend/design-doc.md).

---

## Specification

### 1. Override `HerdrMultiplexer.wait_for_pane_gone` with graceful teardown

The entire fix is a rewrite of `HerdrMultiplexer.wait_for_pane_gone` (`cafleet/src/cafleet/multiplexer/herdr.py`). It keeps its signature and its meaning to the caller — "block until the pane is gone, up to `timeout`; return `True` if it went, `False` on timeout" — but realizes that meaning for a pane whose shell outlives its agent: wait for the agent to exit, then reap the shell pane.

```python
def wait_for_pane_gone(
    self,
    *,
    target_pane_id: str,
    timeout: float = 15.0,
    interval: float = 0.5,
) -> bool:
    # On herdr a pane hosts a persistent shell: `pane split` creates the shell
    # and `pane run` types the coding-agent command into it, so /exit returns
    # the pane to a bare shell rather than closing it (unlike tmux, where the
    # agent IS the pane's foreground process). Graceful teardown therefore
    # waits for the agent to exit, then closes the now-shell-only pane. herdr
    # reports agent_status "unknown" (and drops the `agent` field) once the
    # agent has exited to the bare shell — the non-live status that means "no
    # agent process in the pane." agent_status also returns None when the pane
    # is already gone (pane_not_found teardown race), so the same branch covers
    # "operator already closed it"; kill_pane with ignore_missing swallows that
    # race.
    deadline = time.monotonic() + timeout
    while True:
        status = self.agent_status(target_pane_id=target_pane_id)
        if status is None or status == "unknown":
            self.kill_pane(target_pane_id=target_pane_id, ignore_missing=True)
            return True
        if time.monotonic() >= deadline:
            return False
        time.sleep(interval)
```

- The exit signal is **`agent_status(...)` being `"unknown"` or `None`** — not `done`. herdr reports `"unknown"` (and drops the `agent` field) once the agent has exited to the bare shell; it reports `None` only via `pane_not_found` when the pane is already gone. `done`/`blocked` mean "the agent finished its turn / is waiting on a prompt but is still running," which is not an exit; closing on either would kill a live agent — and because a wedged agent reports `blocked` (never `unknown`), the timeout path below still protects it.
- Reaping uses the existing `kill_pane(..., ignore_missing=True)` (`herdr pane close`). `ignore_missing` swallows the pane-already-gone race so the branch returns `True` cleanly.
- `poll_until_pane_gone` (the base helper) is no longer used by herdr; remove its now-unused import from `herdr.py`. tmux keeps using it.
- `time` is already imported in `herdr.py`.

### 2. What the timeout / stuck-agent path does (unchanged behavior)

If the agent never reaches `agent_status "unknown"` (or `None`) within `timeout` — e.g. claude is `blocked` on a "background process running — exit anyway? (y/n)" confirmation, which reports `agent_status: "blocked"`, never `"unknown"` — the loop returns `False` at the deadline **without closing the pane**. `cli/member.py` then captures the pane tail and exits 2 with the existing recovery hint, exactly as today. The graceful path never force-closes a pane whose agent is still alive; `cafleet member delete --force` remains the operator's escalation for a wedged pane.

### 3. Error propagation (unchanged member.py contract)

`agent_status` and `kill_pane` raise `HerdrError` (a `MultiplexerError`) on a real herdr failure (e.g. server unreachable) — `pane_not_found` is already absorbed (`agent_status` maps it to `None`; `kill_pane(ignore_missing=True)` swallows it). A propagating `MultiplexerError` is caught by `cli/member.py`'s existing handler around the `wait_for_pane_gone` call, which raises `ClickException` "`<backend> call failed while waiting for pane <pane_id> to close: <error>`" (exit 1). No change to `member.py` is required for this.

### 4. Containment guarantee (nothing else changes)

- **`cli/member.py`** — unchanged. It still calls `send_exit` then `wait_for_pane_gone(timeout=15.0, interval=0.5)`; the 15 s budget stays on that call.
- **`HerdrMultiplexer.send_exit`** — unchanged; stays the pure `herdr pane run <id> "/exit"` keystroke.
- **`TmuxMultiplexer`** — untouched; its `wait_for_pane_gone` still polls `list-panes`. tmux does not implement `AgentStateAware`, and this design adds no `isinstance` branch to the CLI.
- **herdr `--force` path** — unchanged; `member_delete` still calls `kill_pane` immediately, with no agent-wait.
- **Sole caller.** `wait_for_pane_gone`'s only production caller is `cli/member.py` `member_delete`; the monitor loop uses `list_pane_ids`, not `wait_for_pane_gone`. Overriding the herdr method affects the member-delete path only.
- **Coverage.** Because `agent_status` is herdr-native, the fix applies uniformly to every herdr-hosted coding agent (claude / codex / opencode).

### 5. Base-contract docstring generalization

`base.py`'s `Multiplexer.wait_for_pane_gone` docstring currently reads as a pure read-only poll ("Block until `target_pane_id` disappears or `timeout` elapses"). Add one sentence so the contract is honest for a backend whose pane outlives its agent: a backend may realize this as "wait for the coding agent to exit, then reap the pane." The full herdr realization lives in the herdr override's inline comment (the leading `#` block in §1, matching the existing comment-not-docstring convention of `HerdrMultiplexer`'s other methods) and SPEC §6.5; the base docstring only acknowledges the generalized contract.

### 6. Alternatives considered (and rejected)

| Alternative | Why rejected |
|---|---|
| Fold the agent-wait + pane-close into `HerdrMultiplexer.send_exit` | `send_exit` would need its own timeout budget, separate from the 15 s `cli/member.py` passes to `wait_for_pane_gone` — two budgets for one teardown. Keeping `send_exit` a pure keystroke and letting `wait_for_pane_gone` own the existing budget is simpler. (User decision: Q1=a.) |
| Branch `cli/member.py` on `isinstance(mux, AgentStateAware)` | Leaks backend specifics into the backend-neutral CLI; the weaker option. (User decision: Q1=a.) |
| Treat `agent_status == "done"` as an exit signal too | `done` is "turn finished, agent still running," not an exit; closing on it would kill a live agent. The exit signal is `"unknown"` (agent exited to the bare shell) or `None` (pane already gone) — never a live state like `done`/`blocked`. (User decision: Q2=a.) |
| Change herdr `split_window` to run the agent as the pane's foreground process (tmux-like), so `/exit` closes the pane | herdr's spawn model is `pane split` (shell) + `pane run` (type command); the env-forwarding and layout-equalization paths depend on it. A far larger, riskier change than reaping the shell after a confirmed agent exit. |
| Auto-`--force` (kill the pane) when the agent does not exit in time | Would ungracefully kill an agent that is merely blocked on a confirmation prompt — exactly the case `--force` exists for the operator to decide. (User decision: Q3.) |

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

### Step 1: Documentation first

- [x] Update `SPEC.md` §6.5 — rewrite the herdr `wait_for_pane_gone(...)` bullet (currently "`poll_until_pane_gone` over `herdr pane get <id>` (absent → gone)") to describe graceful teardown: poll `agent_status` until `"unknown"` (agent exited to the bare shell) or `None` (pane already gone via `pane_not_found`), then `herdr pane close` (`kill_pane`, `ignore_missing`), within the caller's `timeout`/`interval`; timeout → `False` (no pane close), leaving the exit-2 path to the CLI. Note tmux's `wait_for_pane_gone` is unchanged. <!-- completed: 2026-07-07T23:36 -->
- [x] Update `docs/concepts/member-lifecycle.md` "Delete ordering" — note that on a backend whose pane outlives the agent (herdr), the default path waits for the coding agent to exit and then closes the pane; keep the altitude conceptual and backend-neutral. Confirm the mermaid `Exiting --> Gone: exit keystroke, wait for pane to close` label still reads correctly. <!-- completed: 2026-07-07T22:40 -->
- [x] Update `docs/spec/cli-options.md` `member delete` §"Has a pane (default path)" — keep the observable contract (poll up to 15.0 s / 500 ms, timeout → exit 2 + tail) accurate for herdr, where "poll for the pane to disappear" is realized as "wait for the agent to exit, then close the shell pane." The exit-code table (0/1/2) is unchanged. <!-- completed: 2026-07-07T22:40 -->
- [x] Update `cafleet/src/cafleet/multiplexer/base.py` `Multiplexer.wait_for_pane_gone` docstring — add the one-sentence generalization (§5) so the contract covers a pane that outlives its agent. <!-- completed: 2026-07-07T22:40 -->

### Step 2: Code

- [x] Rewrite `HerdrMultiplexer.wait_for_pane_gone` in `cafleet/src/cafleet/multiplexer/herdr.py` per Specification §1 (poll `agent_status`; close on `"unknown"` or `None` via `kill_pane(ignore_missing=True)` → return `True`; deadline reached → return `False`). Keep the signature and defaults. <!-- completed: 2026-07-07T23:36 -->
- [x] Remove the now-unused `poll_until_pane_gone` import from `herdr.py`. <!-- completed: 2026-07-07T22:47 -->

### Step 3: Tests

Cover `HerdrMultiplexer.wait_for_pane_gone` in `tests/multiplexer/test_herdr.py` (monkeypatch `time.sleep`):

- [x] Agent exits after N polls: `agent_status` returns `working`, `working`, then `"unknown"` (agent exited to the bare shell) → asserts `herdr pane get` polled, then `herdr pane close` issued, returns `True`. <!-- completed: 2026-07-07T23:37 -->
- [x] Already gone / teardown race: first `agent_status` read is `None` (pane_not_found) → `kill_pane(ignore_missing=True)` issued, returns `True`, no timeout. <!-- completed: 2026-07-07T22:48 -->
- [x] `done` is not an exit: `agent_status` stuck at `done` (or `blocked`) until the deadline → returns `False`, **no** `herdr pane close` issued. <!-- completed: 2026-07-07T22:48 -->
- [x] Error propagation: `agent_status` raises a non-`pane_not_found` `HerdrError` → propagates (not swallowed). <!-- completed: 2026-07-07T22:48 -->
- [x] Confirm `tests/cli/test_member_delete.py` still passes unchanged — it mocks `wait_for_pane_gone` at the method level, so the member-delete flow assertions are backend-agnostic and unaffected. <!-- completed: 2026-07-07T22:48 -->

### Step 4: Verification

- [x] `mise //cafleet:lint`, `mise //cafleet:typecheck`, `mise //cafleet:test` pass. <!-- completed: 2026-07-07T23:37 -->
- [x] Live smoke on herdr: `cafleet member create` a claude member, then default `cafleet member delete` — exits 0 (agent exits, pane closes) without `--force`. Verified: `Member deleted. pane_id: w11:pD (closed)`, exit 0. <!-- completed: 2026-07-07T23:37 -->
- [x] Live smoke on herdr: a member wedged on a confirmation prompt — default `cafleet member delete` still exits 2 with the pane tail; `--force` then reaps it. Verified by unit test (`done`/`blocked` → `False` → CLI exit 2); the exit-2 + `--force` reap path was also exercised live during smoke cleanup. Deliberate-wedge live smoke deferred as impractical to reproduce deterministically (user decision: unit-test box 3). <!-- completed: 2026-07-07T23:37 -->

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-07-07 | Initial draft |
| 2026-07-07 | Round-1 revision: Overview trimmed to 3 sentences; §5 herdr realization reworded docstring → inline comment; Step 3 test sub-bullets promoted to 4 checkbox tasks (implementation total reconciled to 14). |
| 2026-07-07 | Finalized: Status → Approved after Reviewer approval and user sign-off. |
| 2026-07-08 | Live-smoke correction: the box-2 herdr smoke revealed the premise was wrong — an agent that exits to the bare shell reports `agent_status: "unknown"` (and drops the `agent` field), never `None` (which occurs only via `pane_not_found` when the pane is fully gone). Corrected the exit signal to `"unknown"` or `None` across the premise, §1, §2, §Alternatives, SPEC §6.5, and the tests (realistic `"unknown"` mocks). Reset the affected Step 1/2/3/4 tasks for re-verification. |
| 2026-07-08 | Re-verified after the correction: 73 herdr/member-delete tests + full suite green, lint/typecheck clean, and the box-2 live smoke now passes (`cafleet member delete` on a live herdr member → `pane (closed)`, exit 0, no `--force`). All 14 tasks complete. |
| 2026-07-08 | Reviewer approved (`approved (doc)`) after a rename-only test-feedback round. Status → Complete via local finalize (no PR); fleet-62 team torn down. |
