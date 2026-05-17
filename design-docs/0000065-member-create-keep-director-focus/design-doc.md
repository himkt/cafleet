# `cafleet member create` keeps tmux focus on the Director pane

**Status**: Approved
**Progress**: 4/7 tasks complete
**Last Updated**: 2026-05-17

## Overview

Make `cafleet member create` always spawn the new member pane with `tmux split-window -d`, so the active pane and active window stay on the caller (the Director) after the spawn. This is an unconditional behavior change — no CLI flag, no opt-out.

## Success Criteria

- [ ] After `cafleet member create` runs from a Director pane, the active tmux pane and active window are still the caller's, not the newly created pane. The new pane exists in the Director's window and is reachable via tmux navigation.
- [ ] The cross-window case is verified: if the operator is looking at window `@5` while the Director's pane lives in `@3`, the spawn drops the new pane in `@3` but the active window stays on `@5`.
- [ ] `mise //cafleet:test`, `mise //cafleet:lint`, `mise //cafleet:typecheck`, `mise //cafleet:format` all pass.

---

## Background

`cafleet member create` is invoked by Directors (operators or skill-orchestrated Director agents) to spawn a teammate's coding-agent pane. The CLI calls `tmux.split_window()` (`cafleet/src/cafleet/tmux.py` L52–L63), which runs:

```
tmux split-window -t <director_window_id> -P -F #{pane_id} -e KEY=VAL ... <spawn-command>
```

Without `-d`, tmux makes the freshly created pane the active pane and, if it lives in a different window than the current client focus, also switches the active window. For a Director that is monitoring `member list --activity`, polling its inbox, and dispatching `member ping` / `member exec` on a tick, the auto-switch:

1. Interrupts the Director's current screen (a `member list --activity` table, an in-progress prompt edit, etc.) by replacing it with the new member's `claude` / `codex` splash output.
2. Forces a manual switch back (`prefix + o`, `prefix + ;`, or `prefix + q <n>`) before the next dispatch can happen.
3. Compounds when spawning multiple members in sequence (e.g. a Drafter + Reviewer team) — every spawn yanks focus away again.

The disruption is the same for every Director and has no compensating benefit: the post-spawn placement block already prints the new pane's id, so the Director can reach it on demand via tmux navigation without being teleported there.

---

## Specification

### Behavior

A single mode. `cafleet member create` always invokes tmux with `-d`:

```
tmux split-window -t <wid> -P -F #{pane_id} -d -e KEY=VAL ... <spawn-command>
```

The new pane is created in the Director's window; the active pane and active window are unchanged. There is no CLI flag and no opt-out — operators who *want* the old focus-follows behavior can switch into the new pane manually via standard tmux navigation (`prefix + o`, `prefix + ;`, or by clicking the pane).

### tmux `-d` semantics

The `-d` flag on `tmux split-window` tells tmux not to make the new pane active. In practice it has two combined effects relevant here:

1. The active **pane** stays wherever it was when `split-window` ran.
2. The active **window** is *not* switched to the spawn target window. So a caller looking at window `@5` while the Director's pane lives in `@3` continues to see `@5` after the spawn lands in `@3`.

If the caller pane somehow no longer exists at the moment tmux processes the spawn (e.g. the operator closed it mid-call), `-d` causes tmux to leave the active-pane pointer wherever its own bookkeeping had last set it; we do not capture-and-restore the pre-split focus, because that opens a TOCTOU race that `-d` avoids in a single atomic argv.

### tmux helper

`cafleet.tmux.split_window` is updated to always append `-d` to the argv. The helper has no new parameter — the policy is hardcoded at this single call site, which matches the fact that `split_window` is only used by `member_create` and there is no use case for a non-`-d` spawn anywhere in the repo.

```python
def split_window(
    *,
    target_window_id: str,
    env: dict[str, str],
    command: list[str],
) -> str:
    """Split the target window with ``command`` and return the new pane id.

    Always invoked with ``-d`` so the new pane is not made active and the
    calling client's active window is not switched.
    """
    args = [
        "tmux", "split-window", "-t", target_window_id, "-P", "-F", "#{pane_id}", "-d",
    ]
    for k, v in env.items():
        args += ["-e", f"{k}={v}"]
    args += command
    return _run(args).strip()
```

The `-d` flag is appended *after* `-F #{pane_id}` and *before* the `-e KEY=VAL` env-forward flags, matching tmux's argv conventions. Position is not behaviorally significant — tmux parses the option block regardless — but the placement keeps the argv readable.

### `member_create` integration

The call site at `cafleet/src/cafleet/cli.py::member_create` (~L993) does **not** change shape. The existing invocation:

```python
pane_id = tmux.split_window(
    target_window_id=director_ctx.window_id,
    env=fwd_env,
    command=spawn_command,
)
```

continues to work as-is and now silently inherits the `-d` behavior from the helper. No flag, no signature change, no kwarg.

No changes are made to the surrounding `select_layout` call (rebalancing the window does not change focus on its own), the placement update, or the rollback paths. The placement block printed at the end of `member_create` already includes the new pane's `tmux_pane_id`, so the Director still has the information needed to navigate to the new pane on demand.

### Scope boundaries

This design touches **only** `cafleet member create`. The other `cafleet member` subcommands are left exactly as they are:

| Subcommand | Why it does not need a change |
|---|---|
| `member delete` | Calls `tmux.send_exit` / `tmux.kill_pane` + `select_layout`; neither operation moves focus. |
| `member capture` | Pure `tmux capture-pane -p`; no focus side-effect. |
| `member send-input` | `tmux send-keys -t <member-pane>`; targets the member's pane but does not switch focus there. |
| `member exec` | Same as `send-input` — `send-keys` does not change focus. |
| `member ping` | Wraps `tmux.send_poll_trigger`, which is `tmux send-keys`; no focus side-effect. |

`tmux.split_window` is only called by `member_create`, so hardcoding `-d` inside the helper cannot regress any other code path in this repo.

### Documentation surfaces

The behavior change must land in these documentation files in the same change, per `.claude/rules/design-doc-numbering.md`:

| File | Change |
|---|---|
| `docs/spec/cli-options.md` § `member create` | One-sentence note (no new flag row) that the spawn always invokes `tmux split-window` with `-d` so the Director's pane and window keep focus. |
| `ARCHITECTURE.md` § member-spawn (the `tmux split-window -t <window_id>` step) | One sentence noting that the spawn passes `-d`, and the cross-window consequence (active window stays on the caller). |
| `skills/cafleet/reference/director.md` § Member Create | Same one-sentence note (the section currently describes the spawn semantics; add the focus disclosure there). |
| `README.md` | Verify with `grep -n "focus\|active pane\|member create" README.md`; if any user-facing section describes spawn focus behavior, update it. Otherwise leave README untouched and record "no relevant README section found" in the task note. |

### Testing

Two existing test files cover the affected surfaces; both need targeted updates.

#### `cafleet/tests/test_tmux.py::test_split_window__argv_construction`

Update the existing test (currently L66–L104) so it asserts `-d` is always in the captured argv, and pin its position relative to the `-F #{pane_id}` block and the `-e KEY=VAL` env block.

Sketch (with a non-empty `env` so the `-d` / `-e` ordering is observable):

```python
tmux.split_window(
    target_window_id="@3", env={"K": "V"}, command=["claude", "hi"]
)
argv = run_recorder[-1]
assert "-d" in argv
assert argv.index("-d") > argv.index("#{pane_id}")  # after the -F #{pane_id} block
assert argv.index("-d") < argv.index("-e")          # before the -e env block
assert argv.index("-d") < argv.index("claude")      # before the spawn command
```

Pinning the position (rather than just "-d appears somewhere before the spawn command") guards the documented argv shape so a future refactor that drops `-d` after the env flags fails the test instead of silently passing.

The pre-existing assertions on `-P`, `-F`, `#{pane_id}`, and env-forwarding shape continue to hold and remain in the test.

#### `cafleet/tests/test_cli_member.py`

No changes are required. The argv-shape contract — `-d` is present in the right position — is owned end-to-end by `test_split_window__argv_construction` at the unit level. The existing CLI integration tests (`test_member_create__backend_spawn_argv_shape` at L405, `test_member_create__claude_default_injects_dontask_permission_mode` at L488, etc.) already dereference `split_window_recorder[0]["command"]`, which implicitly requires the recorder to be non-empty; adding an explicit `len == 1` assertion would only convert a future `IndexError` into a slightly nicer `AssertionError` without catching any genuinely distinct failure mode. There is no flag to parametrize, so there are no new opt-out paths to cover either.

`mise //cafleet:test` runs the full suite, including the updated `test_split_window__argv_construction` — that is the gate for this change.

#### Manual smoke (one-time, post-implementation)

Because the `-d` semantics affect a real tmux server's focus state, run a one-time manual check after the unit tests pass:

1. From a fresh tmux session with two windows (`@1`, `@2`), spawn a Director shell in `@1`.
2. Switch the client focus to `@2`.
3. From `@1`, run `cafleet --session-id <s> member create --agent-id <d> --name smoke --description "focus probe"`.
4. Confirm: client focus is still on `@2`; window `@1` now has a new pane but is not the active window.

Record the result in the implementation task notes; this is a sanity check, not an automated test.

### Permissions / settings impact

None. The user-level allow pattern that authorizes every `cafleet` invocation today is `Bash(cafleet *)` (`~/.claude/settings.json:77`); its `*` wildcard tail matches every `cafleet ...` invocation regardless of subcommand or trailing flags, and this design adds no new flags anyway. The project-level `cafleet/.claude/settings.json` has no `cafleet`-specific allow rows of its own to update.

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

### Step 1: Documentation

- [x] Update `docs/spec/cli-options.md` § `member create` with one sentence noting the spawn always invokes `tmux split-window` with `-d` so the Director's pane and window keep focus. No new flag row. <!-- completed: 2026-05-17T12:05 -->
- [x] Update `ARCHITECTURE.md` § member-spawn (the `tmux split-window -t <window_id>` step) with one sentence on the `-d` behavior and the cross-window consequence. <!-- completed: 2026-05-17T12:05 -->
- [x] Update `skills/cafleet/reference/director.md` § Member Create with the same one-sentence focus disclosure. <!-- completed: 2026-05-17T12:05 -->
- [x] `grep -n "focus\|active pane\|member create" README.md`; if any user-facing section describes spawn focus behavior, update it; otherwise leave README untouched and record "no relevant README section found" in the task note. <!-- completed: 2026-05-17T12:05 — README only contains a high-level CLI command list row for `member create`; no user-facing section describes spawn focus behavior, so README left untouched. -->

### Step 2: tmux helper

- [ ] Modify `cafleet/src/cafleet/tmux.py::split_window` to always append `-d` to the argv (after `-F #{pane_id}`, before the `-e KEY=VAL` block). Update the docstring to state the behavior is unconditional. No new parameter. <!-- completed: -->

### Step 3: Tests

- [ ] Update `cafleet/tests/test_tmux.py::test_split_window__argv_construction` to assert `-d` is always present in the captured argv and pin its position (`#{pane_id}` < `-d` < `-e`; `-d` < spawn command). Keep the existing `-P` / `-F` / env-forwarding assertions intact. No changes to `cafleet/tests/test_cli_member.py` — the argv-shape contract is owned by the unit-level test (see *Specification → Testing → cafleet/tests/test_cli_member.py*). <!-- completed: -->

### Step 4: Validation

- [ ] Run `mise //cafleet:test`, `mise //cafleet:lint`, `mise //cafleet:typecheck`, and `mise //cafleet:format`; all must pass. Run the manual smoke test described in *Specification → Testing → Manual smoke* and record the outcome. <!-- completed: -->

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-05-17 | Initial draft |
| 2026-05-17 | Per user feedback: drop the `--keep-focus/--no-keep-focus` flag and the `detach` helper kwarg. Make `tmux split-window -d` the unconditional behavior of `cafleet member create`. |
| 2026-05-17 | Per reviewer feedback: drop the no-op `len(split_window_recorder) == 1` CLI integration assertion. `test_split_window__argv_construction` is the sole owner of the `-d` argv-shape contract. |
| 2026-05-17 | Status → Approved after user sign-off. |
