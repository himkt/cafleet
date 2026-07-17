# Explicit Working Directory for herdr Pane Spawn in `cafleet member create`

**Status**: Approved
**Progress**: 9/9 tasks complete
**Last Updated**: 2026-07-17

## Overview

`cafleet member create` spawns member panes whose working directory currently depends on the multiplexer's builtin cwd inheritance. That inheritance works on tmux but breaks on herdr, which spawns `/bin/sh` instead of the passwd login shell when `SHELL` is unset ([herdr discussion #1517](https://github.com/ogulcancelik/herdr/discussions/1517)). This design makes the herdr backend pass the invoking process's working directory explicitly via `herdr pane split --cwd`, removing the dependence on herdr's inheritance mechanism.

## Success Criteria

- [x] A member spawned via `cafleet member create` on the herdr backend receives the invoking process's working directory (the Director's pane cwd) as its start directory — verified by proxy via the pinned-argv `--cwd` tests in § Testing; live-herdr confirmation is a manual operator step outside this checklist.
- [x] Every `herdr pane split` issued by `split_window` — both the first-member right split and the subsequent down split — carries `--cwd <dir>`.
- [x] The tmux backend is unchanged (builtin inheritance remains the mechanism).
- [x] The cwd-fetching logic is annotated with the mandated 2-line `NOTE(himkt)` comment, verbatim.
- [x] An unresolvable cwd fails the spawn loudly (`HerdrError`); no fallback directory.
- [x] `mise //cafleet:test`, `mise //cafleet:lint`, and `mise //cafleet:typecheck` pass.
- [x] SPEC.md and `docs/spec/multiplexer-backends.md` reflect the new contract in this same cycle.

---

## Background

The spawn path: `cafleet member create` (`cafleet/src/cafleet/cli/member.py`) resolves the multiplexer, discovers the Director's pane context, and calls `Multiplexer.split_window(reference, env, command)`. Backend realizations differ:

| Backend | Spawn primitive | cwd today |
|---------|-----------------|-----------|
| tmux | `tmux split-window` | builtin inheritance — the new pane starts in the splitting pane's cwd. Works. |
| herdr | `herdr pane split` + `herdr pane run` | inherited via the spawned shell. Broken when `SHELL` is unset: herdr falls back to `/bin/sh` instead of the passwd login shell (discussion #1517), so the pane does not land in the expected directory. |

SPEC.md §6.5 currently documents the herdr split contract as "`--cwd` is never passed." — `herdr pane split` supports a `--cwd` flag; cafleet just never uses it. The user decided (clarification round, 2026-07-17): scope the fix to herdr only, wire the cwd at split time via `--cwd` (no `cd`-prefix on the `pane run` line), source it from `os.getcwd()` of the member-create process, and fail loudly when the cwd cannot be resolved.

---

## Specification

### Scope

| Surface | Change |
|---------|--------|
| `cafleet/src/cafleet/multiplexer/herdr.py` | `HerdrMultiplexer.split_window` fetches the cwd and passes it to every `herdr pane split`. |
| `Multiplexer` Protocol (`multiplexer/base.py`) | Unchanged — the signature stays `split_window(*, reference, env, command)`. The cwd is a herdr-internal concern. |
| `cafleet/src/cafleet/multiplexer/tmux.py` | Unchanged. |
| `cafleet/src/cafleet/cli/member.py` | Unchanged — no new parameter flows through `member create`. |
| CLI flags / `CAFLEET_*` env vars | None added. No SKILL.md or README impact. |

### cwd source and the mandated comment

`HerdrMultiplexer.split_window` fetches the working directory once per call, before the `herdr pane list` read, using `os.getcwd()`. Because `split_window` executes inside the `cafleet member create` process, this is the Director's pane cwd. The fetch site carries this exact 2-line comment, verbatim:

```python
# NOTE(himkt): once herdr respects the login shell when split, this won't be necessary
# https://github.com/ogulcancelik/herdr/discussions/1517
```

### Wiring

`_split_pane` gains a `cwd` positional parameter, and both split branches pass it. The `--cwd` flag is placed after `--no-focus` and before the `--env` flags, so the env tail append is untouched and the argv shape stays deterministic for the pinned-argv tests:

```python
def split_window(
    self,
    *,
    reference: MultiplexerContext,
    env: dict[str, str],
    command: list[str],
) -> str:
    # NOTE(himkt): once herdr respects the login shell when split, this won't be necessary
    # https://github.com/ogulcancelik/herdr/discussions/1517
    try:
        cwd = os.getcwd()
    except OSError as exc:
        raise HerdrError(
            f"cannot resolve the working directory for pane spawn: {exc}"
        ) from exc
    env_args = [arg for k, v in env.items() for arg in ("--env", f"{k}={v}")]
    ...
    if not column:
        new_pane_id = self._split_pane(reference.pane_id, "right", cwd, env_args)
    else:
        new_pane_id = self._split_pane(max(column), "down", cwd, env_args)
        self._equalize_focused_tab_column()
    ...

def _split_pane(
    self, pane_id: str, direction: str, cwd: str, env_args: list[str]
) -> str:
    result = _run_json(
        [
            "herdr",
            "pane",
            "split",
            pane_id,
            "--direction",
            direction,
            "--no-focus",
            "--cwd",
            cwd,
        ]
        + env_args
    )
    ...
```

`os.getcwd()` returns an absolute path and the argv is a list (no shell), so no quoting or normalization is needed; the path is passed verbatim.

### Error handling

| Failure | Behavior |
|---------|----------|
| `os.getcwd()` raises `OSError` (the invoking process's cwd was deleted) | Wrapped into `HerdrError("cannot resolve the working directory for pane spawn: …")`. No fallback to `$HOME` or any other directory. |
| herdr rejects `--cwd` (e.g. the directory vanished between fetch and split) | The non-zero exit propagates through `_run` as `HerdrError`, unchanged. |

Both surface at the CLI boundary through the existing `except MultiplexerError` handler in `member create`, which rolls back the member registration (`_rollback_register`) and reports the failure — the fail-fast path already in place for split failures.

### Documentation contract changes

| Document | Edit |
|----------|------|
| `SPEC.md` §6.5, herdr `split_window` bullet | Replace "`--cwd` is never passed." with the new contract: both split forms are `herdr pane split <pane> --direction <right\|down> --no-focus --cwd <cwd> [--env K=V …]`, where `<cwd>` is `os.getcwd()` of the invoking process, fetched once per `split_window` call; an `OSError` from the fetch maps to `HerdrError` (no fallback). Update the two inline argv templates accordingly. |
| `docs/spec/multiplexer-backends.md` | Add a short "Pane spawn working directory" subsection: tmux relies on builtin cwd inheritance; herdr receives the member-create process's cwd explicitly via `herdr pane split --cwd`, because herdr does not respect the passwd login shell when `SHELL` is unset (discussion #1517). |

`docs/api/multiplexer.md` is an mkdocstrings auto-render of `multiplexer/base.py`, which is unchanged — no edit. No CLI surface changes, so `docs/spec/cli-options.md`, README.md, and all SKILL.md files are unaffected.

### Testing

| Test | Change |
|------|--------|
| `tests/multiplexer/test_herdr.py::test_split_window__first_member_splits_director_right` | Monkeypatch `os.getcwd` to a fixed path; extend the pinned split argv with `--cwd <that path>` between `--no-focus` and `--env`. |
| `…::test_split_window__first_member_no_env_omits_env_flags` | Same monkeypatch; pinned argv ends `--no-focus --cwd <path>` with no `--env`. |
| `…::test_split_window__subsequent_member_splits_max_then_equalizes` | Same monkeypatch; the down-split argv gains `--cwd <path>`. |
| New: `…::test_split_window__unresolvable_cwd_raises` | Monkeypatch `os.getcwd` to raise `FileNotFoundError`; assert `HerdrError` and that no herdr subprocess ran. |
| `tests/cli/test_member.py` (member create) | No behavioral change at this layer (the CLI passes nothing new; fakes satisfy the unchanged Protocol). Verify the suite stays green; no edits expected. |

Live-herdr verification (a member pane actually starting in the Director's cwd) is manual, operator-performed — outside the automated success criteria.

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

### Step 1: Documentation

- [x] Update SPEC.md §6.5 herdr `split_window` bullet: new `--cwd` contract and argv templates <!-- completed: 2026-07-17T12:50 -->
- [x] Add the "Pane spawn working directory" subsection to `docs/spec/multiplexer-backends.md` <!-- completed: 2026-07-17T12:50 -->

### Step 2: herdr backend

- [x] In `HerdrMultiplexer.split_window`, fetch `os.getcwd()` once (annotated with the verbatim 2-line NOTE(himkt) comment), wrapping `OSError` in `HerdrError` <!-- completed: 2026-07-17T12:56 -->
- [x] Thread `cwd` through `_split_pane` and append `--cwd <cwd>` after `--no-focus` in the split argv <!-- completed: 2026-07-17T12:56 -->

### Step 3: Tests

- [x] Update the three pinned-argv `split_window` tests in `tests/multiplexer/test_herdr.py` for the `--cwd` flag (fixed-path `os.getcwd` monkeypatch) <!-- completed: 2026-07-17T12:52 -->
- [x] Add `test_split_window__unresolvable_cwd_raises` (`os.getcwd` raises → `HerdrError`, no subprocess call) <!-- completed: 2026-07-17T12:52 -->

### Step 4: Verification

- [x] `mise //cafleet:test` passes <!-- completed: 2026-07-17T13:00 -->
- [x] `mise //cafleet:lint` passes <!-- completed: 2026-07-17T13:00 -->
- [x] `mise //cafleet:typecheck` passes <!-- completed: 2026-07-17T13:00 -->

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-07-17 | Initial draft |
