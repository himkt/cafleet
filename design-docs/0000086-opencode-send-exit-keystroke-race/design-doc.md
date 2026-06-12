# Fix opencode `/exit` Keystroke Race in `TmuxMultiplexer.send_exit`

**Status**: Approved
**Progress**: 6/13 tasks complete
**Last Updated**: 2026-06-12

## Overview

`TmuxMultiplexer.send_exit` sends `/exit` and Enter in a single fused `tmux send-keys` call with zero delay, which races against opencode's slash-command autocomplete popup: the Enter is consumed before the input state settles, `/exit` never submits, and the default `cafleet member delete` path times out (exit 2), forcing operators to rerun with `--force`. This change routes `send_exit` through the existing `_send_literal_then_enter` helper (literal-mode send, `_SUBMIT_DELAY` gap, separate Enter) plus a trailing second Enter, with pane-gone tolerance on every keystroke.

## Success Criteria

- [ ] A live opencode member is deletable via the default `cafleet member delete` path with exit 0 (no `--force`)
- [ ] Live claude and codex members still delete cleanly via the default path (exit 0) with the new keystroke sequence
- [ ] `send_exit` issues three separate `tmux send-keys` invocations — literal `/exit`, Enter, trailing Enter — each separated by `_SUBMIT_DELAY` and each pane-gone-tolerant when `ignore_missing=True`
- [ ] `_send_literal_then_enter`'s new `ignore_missing` parameter defaults to `False`, leaving the four other callers (`send_poll_trigger`, `send_inline_preview`, `send_bash_command`, `send_freetext_and_submit`) behaviorally unchanged
- [ ] `_run_tolerating_pane_gone` keeps its pane-gone-marker swallowing behavior (its new `timeout` parameter is optional and backward compatible for existing callers); `kill_pane` and all three `send_exit` call sites keep their existing contracts
- [ ] `mise //cafleet:test`, `mise //cafleet:lint`, and `mise //cafleet:typecheck` pass

---

## Background

Reproduced live on 2026-06-12 (fleets 31 and 32): opencode's TUI opens a slash-command autocomplete popup on `/`, and the Enter fused into the same `send-keys` call (`cafleet/src/cafleet/multiplexer/tmux.py:137`) is consumed before the input line settles. The pane stays alive with `/exit` unsubmitted, `wait_for_pane_gone` times out after 15 s, and `member delete` exits 2. A single bare Enter sent afterwards exited opencode immediately.

The fix sequence was validated by hand: literal-mode text (`tmux send-keys -l "/exit"`) followed by a separate delayed Enter exits opencode cleanly on the **first** Enter — two Enters are not required when the gap is long enough. The hand test used multi-second gaps, so `_SUBMIT_DELAY` (0.12 s) is unvalidated against opencode's popup render time; the trailing second Enter below is insurance for that gap and is harmless on claude/codex.

---

## Specification

### New keystroke sequence

`send_exit` switches from one fused invocation to three separate invocations:

| # | tmux invocation | Gap after | Pane-gone tolerance |
|---|---|---|---|
| 1 | `tmux send-keys -t <pane> -l "/exit"` | `_SUBMIT_DELAY` (0.12 s) | per `ignore_missing` |
| 2 | `tmux send-keys -t <pane> Enter` | `_SUBMIT_DELAY` (0.12 s) | per `ignore_missing` |
| 3 | `tmux send-keys -t <pane> Enter` | — | per `ignore_missing` |

Decision rationale:

- **Literal `-l` send + delayed separate Enter** is the hand-validated fix: the delay lets opencode's popup/input state settle so the first Enter submits.
- **Trailing second Enter** (instead of bumping the delay): keeps `_SUBMIT_DELAY` at its codex-validated 0.12 s while covering the unvalidated case where opencode's popup render outlasts 0.12 s. On claude/codex the first Enter already submits `/exit`, so the trailing Enter lands in a closing or already-gone pane and is swallowed by pane-gone tolerance.
- **Backend-agnostic**: the sequence is identical for claude, codex, and opencode. No backend plumbing into `send_exit`.
- **Latency**: the sequence adds 0.24 s of sleeps per `send_exit`, negligible against the 15 s `wait_for_pane_gone` budget. The 15 s timeout, polling interval, and `--force` path are unchanged.

### Code changes (`cafleet/src/cafleet/multiplexer/tmux.py`)

`_run_tolerating_pane_gone` gains a pass-through `timeout` parameter (default `None`, backward compatible). The pass-through exists solely so `_send_literal_then_enter` keeps honoring the `timeout=5` passed by `send_poll_trigger` / `send_inline_preview` after rerouting through `_run_tolerating_pane_gone`:

```python
def _run_tolerating_pane_gone(
    args: list[str], *, ignore_missing: bool, timeout: float | None = None
) -> None:
    try:
        _run(args, timeout=timeout)
    except TmuxError as exc:
        if ignore_missing and any(m in str(exc).lower() for m in _PANE_GONE_MARKERS):
            return
        raise
```

`_send_literal_then_enter` gains `ignore_missing: bool = False` and routes both invocations through `_run_tolerating_pane_gone`. With `ignore_missing=False` the wrapper re-raises every `TmuxError`, so the four existing callers see identical behavior:

```python
def _send_literal_then_enter(
    *,
    target_pane_id: str,
    payload: str,
    timeout: float | None = None,
    ignore_missing: bool = False,
) -> None:
    _run_tolerating_pane_gone(
        ["tmux", "send-keys", "-t", target_pane_id, "-l", payload],
        ignore_missing=ignore_missing,
        timeout=timeout,
    )
    time.sleep(_SUBMIT_DELAY)
    _run_tolerating_pane_gone(
        ["tmux", "send-keys", "-t", target_pane_id, "Enter"],
        ignore_missing=ignore_missing,
        timeout=timeout,
    )
```

`send_exit` keeps its public signature (`target_pane_id`, `ignore_missing: bool = False`) and becomes:

```python
def send_exit(self, *, target_pane_id: str, ignore_missing: bool = False) -> None:
    """Send literal ``/exit`` + Enter + trailing Enter, swallowing pane-gone errors when requested."""
    _send_literal_then_enter(
        target_pane_id=target_pane_id,
        payload="/exit",
        ignore_missing=ignore_missing,
    )
    time.sleep(_SUBMIT_DELAY)
    _run_tolerating_pane_gone(
        ["tmux", "send-keys", "-t", target_pane_id, "Enter"],
        ignore_missing=ignore_missing,
    )
```

The `_SUBMIT_DELAY` comment is extended to name both settle cases it now covers: codex bracketed-paste finalization and opencode slash-popup settling.

### `ignore_missing` semantics

- With `ignore_missing=True`, **every** keystroke in the exit sequence is pane-gone-tolerant. A pane dying mid-sequence (e.g. right after the first Enter exits the agent) is success, not error: each subsequent invocation fails with a pane-gone marker and is swallowed independently.
- With `ignore_missing=False`, any `TmuxError` propagates from whichever invocation raised it. **Behavioral delta vs. today**: the old fused single invocation cannot fail after its keystroke is delivered, but the three-invocation sequence adds new failure windows — a pane dying between the first Enter and the trailing Enter (the normal claude/codex case once `/exit` submits) now raises `TmuxError` where the old code returned success. This is acceptable because all three in-repo call sites (`member.py:215`, `:223`, `:302`) pass `ignore_missing=True`, so no production path is affected.
- Non-pane-gone errors (e.g. tmux server crash) propagate regardless of the flag, exactly as `_run_tolerating_pane_gone` behaves today.

### Out of scope

- Other keystroke paths (`send_poll_trigger`, `send_inline_preview`, `send_bash_command`, `send_freetext_and_submit`) keep their current two-invocation sequence and non-tolerant behavior.
- `kill_pane`, the three `send_exit` call sites (`cafleet/src/cafleet/cli/member.py:215`, `:223`, `:302`), the 15 s `wait_for_pane_gone` timeout, and the `--force` path are unchanged.
- The `_SUBMIT_DELAY` value (0.12 s) is unchanged.

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

### Step 1: Documentation

- [x] Update `docs/spec/cli-options.md` § `member delete` default-path description (~line 560): replace "sends `/exit` via `tmux send-keys`" with the three-invocation sequence (literal `/exit`, 0.12 s gap, Enter, 0.12 s gap, trailing Enter) and a one-line rationale (opencode slash-popup settle; trailing Enter harmless on claude/codex) <!-- completed: 2026-06-12T10:35 -->
- [x] Update `docs/concepts/member-lifecycle.md` default-path wording (~line 57) if it implies a single fused keystroke; keep the altitude conceptual <!-- completed: 2026-06-12T10:35 -->

### Step 2: Code

- [x] Add `timeout: float | None = None` pass-through to `_run_tolerating_pane_gone` <!-- completed: 2026-06-12T10:41 -->
- [x] Add `ignore_missing: bool = False` to `_send_literal_then_enter` and route both invocations through `_run_tolerating_pane_gone` <!-- completed: 2026-06-12T10:41 -->
- [x] Rewrite `send_exit` per the Specification (helper call + delayed trailing tolerant Enter) <!-- completed: 2026-06-12T10:41 -->
- [x] Extend the `_SUBMIT_DELAY` comment to cover the opencode slash-popup settle case alongside codex bracketed-paste <!-- completed: 2026-06-12T10:41 -->

### Step 3: Tests

- [ ] Rework `test_send_exit__success_and_ignore_missing_semantics` in `tests/multiplexer/test_tmux.py`: assert the three-invocation argv sequence (`-l /exit`, `Enter`, `Enter`), per-invocation pane-gone tolerance with `ignore_missing=True` (including pane dying mid-sequence), propagation of non-pane-gone errors, and propagation of pane-gone with `ignore_missing=False`; monkeypatch `time.sleep` to avoid real delays <!-- completed: -->
- [ ] Add `_send_literal_then_enter` `ignore_missing` coverage: default `False` still raises on pane-gone (existing caller behavior pinned); `True` swallows pane-gone on either invocation <!-- completed: -->
- [ ] Confirm `tests/cli/test_member_delete.py` and `tests/cli/test_member.py` still pass — they mock `send_exit` at method level and assert kwargs only, so no changes are expected <!-- completed: -->
- [ ] Run `mise //cafleet:test`, `mise //cafleet:lint`, `mise //cafleet:typecheck` <!-- completed: -->

### Step 4: Live verification (one member per backend)

- [ ] claude: `cafleet member create` + default `cafleet member delete` exits 0 <!-- completed: -->
- [ ] codex: `cafleet member create` + default `cafleet member delete` exits 0 <!-- completed: -->
- [ ] opencode: `cafleet member create` + default `cafleet member delete` exits 0 without `--force` <!-- completed: -->

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-06-12 | Initial draft |
