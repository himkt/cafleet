# Herdr Pane Rearrange on `cafleet member delete`

**Status**: Approved
**Progress**: 4/19 tasks complete
**Last Updated**: 2026-07-18

## Overview

`cafleet member delete` closes a member's pane but performs no layout step afterwards; tmux natively auto-fits the remaining panes, while herdr has no native reflow, leaving the member column unbalanced. This design adds a best-effort rebalance to `HerdrMultiplexer.kill_pane` — re-equalizing the remaining member column after a close, and explicitly restoring the Director pane to full tab width after the last member is deleted — so the delete path maintains the same layout invariant the create path already enforces.

## Success Criteria

- [ ] After `cafleet member delete` closes a herdr member pane with ≥ 2 members remaining, the member-column panes are re-equalized to equal heights (the create-path invariant).
- [ ] After the last member pane is deleted, the Director pane's full-tab-width state is explicitly verified from the layout read, with a corrective resize issued when a residual right split remains.
- [ ] A layout failure (any `HerdrError` during the rebalance) never fails `member delete` — the pane is closed and the member deregistered regardless.
- [ ] The rebalance only ever resizes the killed pane's tab: when the focused-tab layout reports a different tab, no resize is emitted.
- [ ] The tmux backend's delete path is unchanged: a bare `tmux kill-pane` with no layout step.
- [ ] The `Multiplexer` Protocol, the `member delete` CLI path, and the CLI-level call-order contract (`kill_pane` → `deregister_member`) are unchanged.
- [ ] `docs/spec/multiplexer-backends.md`, `SPEC.md`, `docs/concepts/member-lifecycle.md`, and `docs/api/multiplexer.md` document the delete-time layout behavior; `mise //cafleet:test`, `mise //cafleet:lint`, and `mise //cafleet:typecheck` pass.

---

## Background

The two multiplexer backends are asymmetric on pane close:

| Path | tmux | herdr |
|:--|:--|:--|
| Create (`split_window`) | `tmux split-window` then explicit `select-layout main-vertical` (`tmux.py:185-223`) | manual `main-vertical` emulation: split, then `_equalize_focused_tab_column()` rebalances the member column to equal heights (`herdr.py:163-274`) |
| Delete (`kill_pane`) | bare `tmux kill-pane` — tmux **natively auto-fits** the remaining panes (`tmux.py:370-375`) | bare `herdr pane close` — herdr has **no native reflow**, so the remaining member column is left unbalanced (`herdr.py:409-413`) |

`cafleet member delete` (`cafleet/src/cafleet/cli/member.py:364-377`) runs exactly `kill_pane` → `deregister_member`, with no layout step; `tests/cli/test_member_delete.py:187-191` pins that call order. GitHub issue #204 targets the herdr-side gap: after a delete, the killed member's slot leaves the column heights uneven, and cafleet already owns the symmetric rebalance logic on the create path.

---

## Specification

### Decisions

| # | Question | Decision |
|:--|:--|:--|
| 1 | Scope | herdr-only fix. After a delete, the remaining member-column panes are re-equalized; tmux stays a bare `kill-pane` (native auto-fit). |
| 2 | Placement | Inside `HerdrMultiplexer.kill_pane` itself: close the pane, then rebalance. No `Multiplexer` Protocol change, no CLI change, tmux untouched; every `kill_pane` caller inherits the behavior. |
| 3 | Last member | **Explicit restore**: after the last member pane is deleted, verify from the layout read that the Director pane spans the full tab width, and issue a corrective `herdr pane resize` when a residual right split remains — do not rely solely on herdr's native space absorption. |
| 4 | Degraded behavior | When the layout shape is unexpected (the focused-tab layout is not the target tab, malformed split chain, non-single-Director residue) or the pre-close tab read fails, silently skip the rearrange; the delete still succeeds. |
| 5 | Error handling | Best-effort: swallow `HerdrError` from the rebalance so a layout failure never fails `member delete` (mirrors `_equalize_focused_tab_column`, `herdr.py:205-216`). |
| 6 | Bulk teardown | `fleet delete` and monitor-driven recovery inherit the rearrange through the same `kill_pane` path, including N sequential rebalances during full-team teardown. No suppression logic. |
| 7 | Tab scoping | The rebalance is scoped to the **killed pane's tab**: `kill_pane` reads the target's `tab_id` (`herdr pane get <target>`) before the close, and the post-close rebalance skips unless the layout read reports that same tab. `herdr pane layout` only reports the focused tab, so when the user's focus sits elsewhere (realistic during a bulk `fleet delete`), the rebalance is skipped rather than resizing an unrelated tab's layout — the next layout-touching operation on the cafleet tab (a member create or delete with focus there) restores the invariant. |

### `kill_pane` change

```python
def kill_pane(self, *, target_pane_id: str, ignore_missing: bool = False) -> None:
    target_tab_id = self._pane_tab_id(target_pane_id)
    _run_tolerating_missing(
        ["herdr", "pane", "close", target_pane_id],
        ignore_missing=ignore_missing,
    )
    self._rebalance_after_close(target_tab_id)

def _pane_tab_id(self, pane_id: str) -> str | None:
    """Best-effort pre-close read of the target pane's tab. None (the
    rebalance skips) when the pane is already gone, the read fails, or the
    envelope lacks the field — the close must proceed regardless."""
    try:
        return _run_json(["herdr", "pane", "get", pane_id])["pane"]["tab_id"]
    except (HerdrError, KeyError):
        return None
```

The pre-close `_pane_tab_id` read anchors the rebalance to the killed pane's tab (decision #7) and never blocks the close: any failure — including `pane_not_found` for a pane already gone under `ignore_missing=True` — yields `None`, the close proceeds with its existing semantics, and the rebalance is skipped (decision #4's degraded mode; with the pane already gone there is no tab to anchor to). When the close itself raises an error not tolerated, the exception propagates unchanged and no rebalance runs.

### Shared layout read (behavior-preserving refactor of the create path)

Extract the layout read + expected-tab guard from `_resize_focused_tab_column` (`herdr.py:225-237`) into a helper both paths call, parameterized by the tab the caller expects:

```python
def _read_tab_layout(self, expected_tab_id: str) -> tuple[list[dict], list[dict]] | None:
    """Return (panes, splits) when the focused-tab layout reported by
    `herdr pane layout` is the expected tab, else None."""
    try:
        layout = _run_json(["herdr", "pane", "layout"])["layout"]
        if layout["tab_id"] != expected_tab_id:
            return None
        return layout["panes"], layout["splits"]
    except KeyError as exc:
        raise HerdrError(f"herdr pane layout missing {exc} field") from exc
```

The create path keeps its exact semantics: `_resize_focused_tab_column` still resolves `expected_tab_id` from `herdr pane current` (the existing read with its missing-field `HerdrError`), then calls `_read_tab_layout(expected_tab_id)` — a `None` return is the existing "focus moved between the two reads" skip. The delete path passes the pre-close target tab instead.

Extract the existing down-split arithmetic (`herdr.py:247-274`: `down_splits` selection, the `len(down_splits) != n - 1` chain guard, the `1/(N-k)` targets, and the signed resize emission) into `_equalize_column(column, splits)`. `_resize_focused_tab_column` becomes: resolve the tab, read via the helper (`None` → return), compute `column` = non-`min_x` panes sorted by `y`, return when `len(column) < 2`, else `_equalize_column(column, splits)`. The create path's observable command sequence is unchanged.

### Delete-path rebalance

```python
def _rebalance_after_close(self, target_tab_id: str | None) -> None:
    """Best-effort: any HerdrError is swallowed so a layout failure never
    fails a delete — the pane is already closed."""
    if target_tab_id is None:
        return  # no pre-close tab anchor — skip (decision #7)
    try:
        self._resize_after_close(target_tab_id)
    except HerdrError:
        return

def _resize_after_close(self, target_tab_id: str) -> None:
    read = self._read_tab_layout(target_tab_id)
    if read is None:
        return  # focus is on another tab — never resize an unrelated layout
    panes, splits = read
    if not panes:
        return
    min_x = min(p["rect"]["x"] for p in panes)
    column = sorted(
        (p for p in panes if p["rect"]["x"] != min_x),
        key=lambda p: p["rect"]["y"],
    )
    if len(column) >= 2:
        self._equalize_column(column, splits)
    elif not column:
        self._restore_director_full_width(panes, splits)
    # len(column) == 1: a lone member pane spans the column natively and the
    # Director|column right split's ratio is unaffected by a down-close.
```

Case table for the post-close column size `n`:

| n | Action | Rationale |
|:--|:--|:--|
| ≥ 2 | `_equalize_column` — drive each down split to ratio `1/(N-k)` | The create-path invariant: equal member heights. Malformed chain (`len(down_splits) != n - 1`) skips inside `_equalize_column`. |
| 1 | Nothing | Heights are trivially equal; the right split ratio does not change when a down split collapses. |
| 0 | `_restore_director_full_width` | Decision #3 — the last member was deleted; explicitly verify/restore the Director's full width. |

### Full-width restore (last member deleted)

```python
def _restore_director_full_width(self, panes: list[dict], splits: list[dict]) -> None:
    if len(panes) != 1:
        return  # not the single-Director shape — leave it untouched
    # Empty `splits` ⇒ the sole pane is already structurally full-width;
    # any residue other than exactly one right split is anomalous (decision #4).
    if len(splits) != 1 or splits[0]["direction"] != "right":
        return
    delta = round(1.0 - splits[0]["ratio"], 4)
    if delta < 1e-3:
        return
    _run(
        [
            "herdr", "pane", "resize",
            "--pane", panes[0]["pane_id"],
            "--direction", "right",
            "--amount", str(delta),
        ]
    )
```

The verification is the layout read itself: a single pane with an empty `splits` list is structurally full-width (nothing to emit); a single residual `right` split is the correctable shape and gets one signed resize driving its ratio to 1.0; anything else is an unexpected residue and is skipped. This makes the reclaim explicit — decided from observed layout state, not assumed from herdr's close semantics.

### What does not change

- **tmux**: `kill_pane` stays a bare `tmux kill-pane` (`tmux.py:370-375`); tmux auto-fits natively.
- **`Multiplexer` Protocol** (`base.py`): `kill_pane`'s signature and documented contract ("close the target pane; tolerate a missing pane under `ignore_missing`") are unchanged — the rebalance is a herdr-internal layout detail.
- **CLI**: `member delete` (`cli/member.py`) is untouched. The CLI-level test `tests/cli/test_member_delete.py::test_default_path__kills_pane_then_deregisters` and its `["kill_pane", "deregister_member"]` call-order assertion remain valid, because the fake multiplexer's `kill_pane` is the recorded unit and the rebalance lives inside the real herdr backend.
- **Skills**: no CLI command, flag, output format, or workflow changes, so no `SKILL.md` updates are required.

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

### Step 1: Documentation

- [x] `docs/spec/multiplexer-backends.md`: add a "Delete-time pane layout" section documenting the asymmetry — tmux relies on native auto-fit after `kill-pane`; herdr's `kill_pane` reads the target's tab (`herdr pane get`), closes the pane, then runs a best-effort rebalance scoped to that tab (column re-equalization for ≥ 2 members, explicit Director full-width restore after the last member, silent skip on unexpected shapes or when the focused-tab layout is a different tab). <!-- completed: 2026-07-18T06:37 -->
- [x] `SPEC.md`: update the herdr `kill_pane` entry (~line 1884) to specify the pre-close `_pane_tab_id` read, `herdr pane close`, then the `_rebalance_after_close` algorithm (target-tab-scoped layout read with skip-on-mismatch, the column case table, the full-width restore rule, best-effort `HerdrError` swallowing), alongside the existing `_equalize_focused_tab_column` algorithm section. <!-- completed: 2026-07-18T06:37 -->
- [x] `docs/concepts/member-lifecycle.md`: note that member deletion restores the pane layout on backends without native reflow (herdr). <!-- completed: 2026-07-18T06:37 -->
- [x] `docs/api/multiplexer.md`: note on `kill_pane` that backends may restore the window layout after the close as a backend-internal detail (herdr does; tmux relies on native auto-fit). <!-- completed: 2026-07-18T06:37 -->

### Step 2: Tests (`tests/multiplexer/test_herdr.py`)

- [ ] `kill_pane` rebalance sequence: with a seeded 3-member post-close layout, `kill_pane` emits the pre-close `pane get <target>`, `pane close`, then the `pane layout` read, then the deterministic `1/(N-k)` resize deltas (mirroring the create-path equalize tests at `test_herdr.py:307-398`). <!-- completed: -->
- [ ] Already-balanced column after close → the read pair runs but no `pane resize` is emitted. <!-- completed: -->
- [ ] Single remaining member (`n == 1`) → no resize emitted. <!-- completed: -->
- [ ] Last member deleted, residual right split (e.g. ratio 0.62) → exactly one corrective resize `--pane <director> --direction right --amount 0.38`. <!-- completed: -->
- [ ] Last member deleted, single pane with empty `splits` → no resize emitted; anomalous residue (multiple splits, or a non-right residual split, or ≥ 2 panes all at `min_x`) → no resize emitted. <!-- completed: -->
- [ ] Tab scoping: the layout read reports a `tab_id` different from the pre-close target tab (focus on an unrelated tab) → no resize emitted; a pane already gone at the pre-close `pane get` (`pane_not_found`, `ignore_missing=True`) → the close is tolerated and no layout read or resize is emitted. <!-- completed: -->
- [ ] Best-effort semantics: a `HerdrError` from the pre-close `pane get` yields `None` and the close still runs (no rebalance); a `HerdrError` from the layout read or a resize is swallowed and `kill_pane` still succeeds; a non-tolerated close error propagates and no rebalance commands are emitted; the malformed-chain guard skips. Update the existing `kill_pane` argv tests (`test_herdr.py:434-462`) to seed the pre-close `pane get` and rebalance read responses. <!-- completed: -->
- [ ] Create-path regression: the existing `split_window` / `_equalize_focused_tab_column` tests pass unchanged (the `_read_tab_layout` / `_equalize_column` extraction is behavior-preserving). <!-- completed: -->
- [ ] CLI regression: `tests/cli/test_member_delete.py` passes unchanged, including the `["kill_pane", "deregister_member"]` call-order assertion. <!-- completed: -->

### Step 3: Implementation (`cafleet/src/cafleet/multiplexer/herdr.py`)

- [ ] Extract `_read_tab_layout(expected_tab_id)` and `_equalize_column(column, splits)` from `_resize_focused_tab_column`, preserving the create path's observable command sequence. <!-- completed: -->
- [ ] Add `_pane_tab_id(pane_id)` (best-effort pre-close tab read, `None` on any failure) and `_rebalance_after_close(target_tab_id)` / `_resize_after_close(target_tab_id)` with the column case table (≥ 2 → equalize, 1 → no-op, 0 → full-width restore). <!-- completed: -->
- [ ] Add `_restore_director_full_width(panes, splits)` with the residue guards and the single corrective resize. <!-- completed: -->
- [ ] Wire the pre-close `_pane_tab_id` read and `self._rebalance_after_close(target_tab_id)` into `kill_pane` around `_run_tolerating_missing`. <!-- completed: -->

### Step 4: Verification

- [ ] `mise //cafleet:test` passes. <!-- completed: -->
- [ ] `mise //cafleet:lint` and `mise //cafleet:typecheck` pass. <!-- completed: -->

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-07-18 | Initial draft |
| 2026-07-18 | Review round 1: rebalance scoped to the killed pane's tab via a pre-close `pane get` anchor (decision #7); collapsed the full-width-restore residue guards |
