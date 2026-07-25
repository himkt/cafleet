# herdr equalizer: anchor every layout read on a pane

**Status**: Approved
**Progress**: 20/22 tasks complete
**Last Updated**: 2026-07-25

## Overview

The herdr pane-height equalizer silently stops working whenever the operator is
viewing a tab other than the Director's, because it resolves "which tab am I
operating on" two incompatible ways. This design replaces every bare
`herdr pane layout` read with an invoker-independent `herdr pane layout --pane
<id>` read anchored on a known pane, on both the spawn and delete paths.

## Success Criteria

- [x] Spawning a member equalizes the member column regardless of which tab or pane
      is focused at spawn time.
- [x] Deleting a member rebalances the killed pane's tab regardless of which tab or
      pane is focused at delete time.
- [x] The herdr backend issues no `herdr pane current` call on the spawn or delete
      layout paths; `context_discovery()` keeps its own `pane current` call.
- [x] Every `herdr pane layout` invocation in the backend carries `--pane <id>`.
- [x] `SPEC.md` and `docs/spec/multiplexer-backends.md` describe the anchored reads,
      with no residual mention of the removed tab-mismatch skip.
- [x] The test suite pins the new argv on both paths and contains no test asserting
      the removed guard.

---

## Background

Observed live on herdr 0.7.4 with a 5-member fleet: the member column drifted to
heights `16 / 16 / 10 / 14 / 14` in a 70-row area instead of a uniform `14`, with
split ratios `0.2286 / 0.2963 / 0.2632 / 0.5` against the algorithm's targets of
`0.2 / 0.25 / 0.3333 / 0.5`.

The backend resolves the operating tab twice, by two different rules:

| Call site | Command issued | Resolves to |
|---|---|---|
| `_resize_focused_tab_column` | `herdr pane current` | the **invoking** pane's tab |
| `_read_tab_layout` | `herdr pane layout` (bare) | the **globally focused** tab |

When those disagree, the `layout["tab_id"] != expected_tab_id` comparison fails,
`_read_tab_layout` returns `None`, and the caller returns without resizing. Because
the whole path is best-effort, the skip is silent — no error, no log line.

They disagree routinely: `_split_pane` passes `--no-focus`, so spawning a member
deliberately does not move focus. Any operator watching a non-Director tab during a
spawn gets every equalize pass skipped.

The decisive experiment ran three commands from the same pane `w23:pC` (in tab
`w23:t4`) while the globally focused pane was `w23:p1` (in tab `w23:t1`):

| Command | Returned `tab_id` | Invoker-independent? |
|---|---|---|
| `herdr pane current` | `w23:t4` — the invoker's own tab | n/a (that is its job) |
| `herdr pane layout` | `w23:t1` | no — follows global focus |
| `herdr pane layout --current` | `w23:t1` | no — also follows global focus |
| `herdr pane layout --pane w23:p8` | `w23:t3` | yes — deterministic |

`--current` is a trap: it means "the focused pane", not "the pane I am running in".
Only `--pane <ID>` is deterministic.

The rest of the algorithm was verified intact on 0.7.4 — the `splits` envelope shape,
the 1:1 signed-delta semantics of `herdr pane resize --amount`, and the `1/(N-k)`
target math — so the fix is confined to how the layout is addressed.

It was not possible to diff against a pre-upgrade herdr binary, so it is unknown
whether herdr changed bare `pane layout` from invoker-relative to focus-relative or
was always focus-relative. `--pane` is correct under either history.

---

## Specification

### The anchor rule

Every layout read in the herdr backend addresses its tab by a **pane id it already
holds**, never by focus and never by the invoking pane. The two paths differ only in
where that anchor comes from:

| Path | Anchor | Source |
|---|---|---|
| Spawn | the Director's pane | `reference.pane_id`, already in hand |
| Delete | any pane surviving in the killed pane's tab | a post-close `herdr pane list`, filtered by the pre-close `tab_id` |

`context_discovery()` keeps its `herdr pane current` call — there the question really
is "which pane am I running in", which is exactly what that command answers.

### `_read_tab_layout` — anchored and unconditional

The tab-id comparison is removed. With `--pane`, the returned layout is by
construction the anchor pane's tab, so there is nothing left to compare and no
mismatch case to skip. The helper no longer returns `None`.

```python
def _read_tab_layout(self, anchor_pane_id: str) -> tuple[list[dict], list[dict]]:
    """Return (panes, splits) for the tab containing ``anchor_pane_id``."""
    try:
        layout = _run_json(
            ["herdr", "pane", "layout", "--pane", anchor_pane_id]
        )["layout"]
        return layout["panes"], layout["splits"]
    except KeyError as exc:
        raise HerdrError(f"herdr pane layout missing {exc} field") from exc
```

Both callers drop their `if read is None: return` branch.

### Spawn path

`_equalize_focused_tab_column` / `_resize_focused_tab_column` are renamed to
`_equalize_tab_column` / `_resize_tab_column` — after the change neither has any
relationship to the focused tab, and the old names would misdescribe them. Each takes
the anchor pane id, and the `herdr pane current` round-trip disappears.

```python
def _equalize_tab_column(self, anchor_pane_id: str) -> None:
    """Rebalance the members' right column on the tab containing
    ``anchor_pane_id`` to equal heights — the tmux ``select-layout main-vertical``
    reflow that herdr has no single command for.

    Best-effort: any :class:`HerdrError` is swallowed so a resize failure never
    fails a spawn over cosmetics.
    """
    try:
        self._resize_tab_column(anchor_pane_id)
    except HerdrError:
        return

def _resize_tab_column(self, anchor_pane_id: str) -> None:
    panes, splits = self._read_tab_layout(anchor_pane_id)
    # The right column is every pane outside the leftmost (Director) column.
    min_x = min(p["rect"]["x"] for p in panes)
    column = sorted(
        (p for p in panes if p["rect"]["x"] != min_x),
        key=lambda p: p["rect"]["y"],
    )
    if len(column) < 2:
        return
    self._equalize_column(column, splits)
```

`split_window` passes the Director's pane straight through:

```python
new_pane_id = self._split_pane(max(column), "down", cwd, env_args)
self._equalize_tab_column(reference.pane_id)
```

The `split_window` comment block that names `_equalize_focused_tab_column` is updated
to the new name.

### Delete path

The killed pane is gone by the time the rebalance runs, so it cannot serve as its own
anchor. A post-close `herdr pane list` supplies a surviving one, filtered by the
`tab_id` that `_pane_tab_id` already reads before the close.

```python
def _surviving_pane_in_tab(self, tab_id: str) -> str | None:
    """A pane still open in ``tab_id`` to anchor the layout read on, or None
    when the tab has no panes left."""
    result = _run_json(["herdr", "pane", "list"])
    try:
        return next(
            (p["pane_id"] for p in result["panes"] if p.get("tab_id") == tab_id),
            None,
        )
    except KeyError as exc:
        raise HerdrError(f"herdr pane list missing {exc} field") from exc

def _resize_after_close(self, target_tab_id: str) -> None:
    anchor_pane_id = self._surviving_pane_in_tab(target_tab_id)
    if anchor_pane_id is None:
        return  # the tab has no panes left — nothing to rebalance
    panes, splits = self._read_tab_layout(anchor_pane_id)
    if not panes:
        return
    ...  # column computation and case table unchanged
```

The anchor is the **first** entry in `herdr pane list` order whose `tab_id` matches —
any pane in the tab yields the same layout, and first-match keeps the argv
deterministic for the tests. `kill_pane` and `_rebalance_after_close` are otherwise
unchanged: the `target_tab_id is None` skip, the `not_found`-tolerant close, and the
best-effort `HerdrError` swallow all stay exactly as they are.

### Resulting command sequences

Spawn of a second-or-later member:

| # | Before | After |
|---|---|---|
| 1 | `pane list` | `pane list` |
| 2 | `pane split <max(column)> --direction down --no-focus --cwd <cwd>` | unchanged |
| 3 | `pane current` | *(removed)* |
| 4 | `pane layout` | `pane layout --pane <reference.pane_id>` |
| 5 | `pane resize …` × 0..N | unchanged |
| 6 | `pane run <new> "<cmd>"` | unchanged |

Delete of a member:

| # | Before | After |
|---|---|---|
| 1 | `pane get <target>` | unchanged |
| 2 | `pane close <target>` | unchanged |
| 3 | — | `pane list` *(new — anchor lookup)* |
| 4 | `pane layout` | `pane layout --pane <surviving>` |
| 5 | `pane resize …` × 0..N | unchanged |

### Deliberate non-changes

Recorded so they are not churned during implementation:

| Item | Decision |
|---|---|
| Observability | No logging is added. The multiplexer layer has none today; the argv-pinning tests are what catch a regression. |
| herdr version floor | Nothing is documented and no runtime check is added. The docs stay silent on herdr versions, as today. |
| `if not panes: return` in `_resize_after_close` | Kept. It guards `min()` against a `ValueError` that would escape the `HerdrError` swallow and fail a delete. |
| Ratio math, split-chain detection, `1e-3` tolerance, `_equalize_column` | Untouched — verified correct on 0.7.4. |
| tmux backend | Untouched — it still uses native `select-layout main-vertical` and a bare `kill-pane`. |

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

### Step 1: Documentation

Documentation lands first, per `.claude/rules/documentation-maintenance.md`. No
`skills/*/SKILL.md` file references the equalizer or `herdr pane layout`, so the skill
surface needs no edit.

- [x] `SPEC.md` — replace the `_equalize_focused_tab_column()` bullet with
      `_equalize_tab_column(anchor_pane_id)`: the geometry read is `herdr pane layout
      --pane <anchor_pane_id>`, there is no `pane current` call and no tab-id
      comparison, and the `1/(N-k)` arithmetic and best-effort swallow are unchanged <!-- completed: 2026-07-25T02:42 -->
- [x] `SPEC.md` — update the `split_window` bullet so the equalization step is named
      `_equalize_tab_column` and its anchor is stated as `reference.pane_id` <!-- completed: 2026-07-25T02:42 -->
- [x] `SPEC.md` — rewrite the `kill_pane` phase-3 spec: `_rebalance_after_close`
      skips on a `None` tab, else `_surviving_pane_in_tab` runs `herdr pane list` and
      takes the first pane whose `tab_id` matches (`None` → skip), then
      `_read_tab_layout` reads `herdr pane layout --pane <anchor>`; delete the
      "a `tab_id` mismatch returns `None` and skips" clause; and rename both
      old-name references inside the bullet — the "`_equalize_focused_tab_column`
      arithmetic above" phrase and the shared-helper sentence — so all three
      `SPEC.md` occurrences (with the bullet heading above) are renamed <!-- completed: 2026-07-25T02:42 -->
- [x] `docs/spec/multiplexer-backends.md` — rewrite the herdr bullet in
      §*Delete-time pane layout* so tab scoping is attributed to the anchored read
      (a surviving pane in the killed pane's tab) instead of to a focused-tab
      comparison; remove the "whenever the focused-tab layout reports a different tab
      than the killed pane's" clause and state the no-surviving-pane skip <!-- completed: 2026-07-25T02:42 -->

### Step 2: Tests

`cafleet/tests/multiplexer/test_herdr.py` throughout. Written before the code so the
new argv assertions fail first.

The file has 14 `kill_pane` tests; the tasks below account for all of them — 9
re-pointed, 1 deleted (`__layout_on_different_tab_skips_resize`), and 4 untouched
because they return before the layout read (`__ignore_missing_swallows_pane_not_found`,
`__ignore_missing_does_not_swallow_other_errors`, `__default_raises_on_pane_not_found`,
`__pane_get_error_still_closes_without_rebalance` — the first three never reach the
rebalance, and the fourth has no tab anchor, so none of them issue the new
`pane list`).

- [x] Add a `_survivor_list(tab_id: str = "wT:t3")` helper returning
      `_envelope({"panes": [{"pane_id": "wT:p3", "tab_id": tab_id}]})` — the
      post-close `pane list` envelope supplying the rebalance anchor (`wT:p3` is the
      Director pane already used by `_column_layout` / `_DIRECTOR_PANE`) <!-- completed: 2026-07-25T02:45 -->
- [x] Delete `test_equalize__focus_moved_between_reads_skips` and
      `test_kill_pane__layout_on_different_tab_skips_resize` — both assert the removed
      guard <!-- completed: 2026-07-25T02:45 -->
- [x] Re-point the three surviving `_equalize_*` tests
      (`__three_member_column_drives_top_split_to_one_third`,
      `__already_balanced_column_emits_no_resize`, `__best_effort_swallows_herdr_error`)
      at `_equalize_tab_column("wT:p3")`: drop the `pane current` envelope and its argv
      entry, and expect `["herdr", "pane", "layout", "--pane", "wT:p3"]` <!-- completed: 2026-07-25T02:45 -->
- [x] Re-point `test_split_window__subsequent_member_splits_max_then_equalizes`: drop
      the `pane current` envelope and argv entry, expect
      `["herdr", "pane", "layout", "--pane", "wG:p1"]`, and update the docstring <!-- completed: 2026-07-25T02:45 -->
- [x] Re-point the nine layout-reaching `kill_pane` tests (`__argv_and_success`,
      `__rebalances_remaining_column_after_close`, `__already_balanced_column_emits_no_resize`,
      `__single_remaining_member_emits_no_resize`,
      `__last_member_residual_right_split_restores_director_width`,
      `__last_member_residue_guards_emit_no_resize`, `__layout_read_error_swallowed`,
      `__resize_error_swallowed`, `__malformed_split_chain_skips_resize`): insert
      `_survivor_list()` into `set_returns` immediately after the close entry, and
      `["herdr", "pane", "list"]` before the
      `["herdr", "pane", "layout", "--pane", "wT:p3"]` entry in the expected argv <!-- completed: 2026-07-25T02:45 -->
- [x] Rename the old helper names where they appear outside a re-pointed assertion —
      the `split_window` region comment (line 122), the `_balanced_layout` docstring
      (line 202), the `_run_json`-wrapper inline comment (line 252), and the
      `_equalize_*` section-header comment (line 307) — so Step 4's residue grep
      confirms rather than rediscovers <!-- completed: 2026-07-25T02:45 -->
- [x] Add the three regression tests: (i) the spawn path emits no
      `["herdr", "pane", "current"]` and does emit `pane layout --pane wG:p1`;
      (ii) the delete path, given a `pane list` spanning two tabs, anchors on the
      surviving pane in the target tab and not on one from another tab; (iii) the
      delete path, given a `pane list` with no pane in the target tab, stops after
      `pane list` with no layout read and no resize <!-- completed: 2026-07-25T02:45 -->
- [x] Run `mise //cafleet:test tests/multiplexer/test_herdr.py` and confirm the new and
      re-pointed assertions fail against the unmodified code <!-- completed: 2026-07-25T02:45 -->

### Step 3: Code

`cafleet/src/cafleet/multiplexer/herdr.py` throughout.

- [x] Rewrite `_read_tab_layout` to take `anchor_pane_id`, issue
      `herdr pane layout --pane <anchor_pane_id>`, drop the tab-id comparison, and
      return `tuple[list[dict], list[dict]]` <!-- completed: 2026-07-25T02:49 -->
- [x] Rename `_equalize_focused_tab_column` → `_equalize_tab_column` and
      `_resize_focused_tab_column` → `_resize_tab_column`, give each an
      `anchor_pane_id` parameter, delete the `pane current` call and the
      `if read is None` branch, and update both docstrings <!-- completed: 2026-07-25T02:49 -->
- [x] Update `split_window` to call `self._equalize_tab_column(reference.pane_id)` and
      rename the helper reference in its leading comment block <!-- completed: 2026-07-25T02:49 -->
- [x] Add `_surviving_pane_in_tab(tab_id) -> str | None` per the Specification <!-- completed: 2026-07-25T02:49 -->
- [x] Update `_resize_after_close` to resolve the anchor first, skip on `None`, and
      drop its `if read is None` branch; leave the column case table, the
      `if not panes: return` guard, `kill_pane`, `_pane_tab_id`, and
      `_rebalance_after_close` unchanged <!-- completed: 2026-07-25T02:49 -->
- [x] Run `mise //cafleet:format`, `mise //cafleet:lint`, and `mise //cafleet:typecheck` <!-- completed: 2026-07-25T02:49 -->

### Step 4: Verification

- [x] `mise //cafleet:test` — the full suite passes <!-- completed: 2026-07-25T02:54 -->
- [ ] Live check: from a herdr session, switch the view to a tab **other** than the
      Director's, spawn a member with `cafleet member create`, then read the layout
      read-only with `herdr pane layout --pane <director-pane-id>` and confirm the
      member column's split ratios match the `1/(N-k)` targets and the pane heights are
      uniform — the scenario that reproduces the reported bug <!-- completed: -->
- [ ] Live check: with the view still on another tab, delete a member and confirm the
      remaining column re-equalizes <!-- completed: -->
- [x] Grep the repository for `_equalize_focused_tab_column`, `_resize_focused_tab_column`,
      and the removed guard's wording ("focus moved", "focused-tab layout") and confirm
      no residue remains in code, tests, `SPEC.md`, or `docs/` <!-- completed: 2026-07-25T02:54 -->

**Live-check status.** The geometry half of both live checks was executed against a real
herdr 0.7.4 session on fleet 28 and passed exactly; the focus precondition was not met,
so the two boxes stay unchecked.

Measured, spawning a scratch member to take the member column from N=4 to N=5 and then
deleting it:

| Stage | Split ratios | `1/(N-k)` targets | Column heights |
|---|---|---|---|
| N=4 baseline | 0.25 / 0.3333 / 0.5 | 0.25 / 0.3333 / 0.5 | 18 / 17 / 18 / 17 |
| N=5 after spawn | 0.2 / 0.25 / 0.3333 / 0.5 | 0.2 / 0.25 / 0.3333 / 0.5 | 14 / 14 / 14 / 14 / 14 |
| N=4 after delete | 0.25 / 0.3333 / 0.5 | 0.25 / 0.3333 / 0.5 | 18 / 17 / 18 / 17 |

Every ratio landed on target and every column was uniform within integer rounding of the
70-row area — against the Background's reported failure at the same N=5 size (heights
16/16/10/14/14, ratios 0.2286/0.2963/0.2632/0.5).

What this does not establish: global focus was on the Director's tab `w23:t7` throughout
(confirmed by a bare `herdr pane layout` returning `tab_id: w23:t7` and `herdr pane
current` reporting the Director pane `focused: true`). That is the condition under which
the equalizer worked even before this change, so the run demonstrates no regression but
does not exercise the reported bug — which requires the view on a non-Director tab. The
focus-independence of the anchored read is covered by the unit tests
(`test_split_window__layout_read_anchored_on_director_not_focus` and the two delete-path
anchor regressions), which pin argv rather than live geometry.
