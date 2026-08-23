# Unconditional Esc Safeguard for `send_prompt` and `send_exit`

**Status**: Approved
**Progress**: 12/12 tasks complete
**Last Updated**: 2026-08-23

## Overview

Make the Esc + settle safeguard unconditional on both multiplexer keystroke dispatch paths that still skip it — `send_prompt`'s `--shell` form and `send_exit` — on both backends (tmux and herdr). After the change, every pane keystroke path leads with `Esc` + the 0.1 s settle, and `send_prompt`'s `shell` flag controls only the `! ` payload prefix.

## Success Criteria

- [x] `cafleet member prompt --shell` dispatches `Esc` + 0.1 s settle before the `! <cmd>` payload on both tmux and herdr.
- [x] `send_exit` dispatches `Esc` + 0.1 s settle before `/exit` on both backends, and pane-gone tolerance under `ignore_missing` covers the Esc keystroke too (teardown of a dead pane never fails on the Esc).
- [x] The `shell` flag affects only the payload prefix; both forms share identical Esc and failure semantics.
- [x] `docs/docs/spec/multiplexer-backends.md`, `docs/docs/spec/cli-options.md`, and `SPEC.md` are updated before code, and the old "deliberate omission / would mis-fire" rationale is removed entirely (no historical residue).
- [x] Keystroke-shape unit tests in both backend files assert the new shapes; `mise //cafleet:test` and `mise //cafleet:lint` pass.

---

## Background

On some coding-agent backends (e.g. Claude Code), a `!` typed while a task is executing enters shell mode only when the composer processes it. Successive `member prompt --shell` dispatches against a busy pane therefore pile up as literal `!`-prefixed lines and are mis-processed. The Esc safeguard — press `Escape`, settle 0.1 s, then type the payload — already protects every other keystroke path; the two remaining exceptions are:

| Path | tmux today | herdr today |
|---|---|---|
| `send_prompt` plain form | Esc-first (`esc_first: !shell`, `tmux.rs` `send_prompt`) | `send_esc` then `herdr pane run` (`herdr.rs` `send_prompt`) |
| `send_prompt` shell form | no Esc — `! <stripped>` + `Enter` | no Esc — `herdr pane run <id> "! <stripped>"` |
| `send_exit` | no Esc — `/exit` via literal-then-Enter with `esc_first: false` | no Esc — `herdr pane run <id> "/exit"` |

The shell-form skip is currently documented as a deliberate design decision (`docs/docs/spec/multiplexer-backends.md` § *The `Esc` safeguard*: "an `Esc` before it would mis-fire"). That rationale is retired by this design and removed from every surface; git history is the archive.

**Accepted trade-off**: an Esc landing on a pane with a running task interrupts that task. This is accepted as-is — the mandatory `member ping` follow-up after every shell dispatch carries the resume clause ("then resume your work if something was still running"), which is the mitigation. No additional mitigation is added.

---

## Specification

### Keystroke contract after the change

Every keystroke path leads with `Esc` + `ESC_SETTLE_DELAY` (0.1 s, unchanged, shared constant in `multiplexer/mod.rs`; the sleep goes through the `CommandRunner` so it stays an observable test event):

| Path | tmux | herdr |
|---|---|---|
| `send_prompt` plain form | `Esc` + settle, then `<stripped>` + `Enter` (unchanged) | `send_esc`, then `herdr pane run <id> "<stripped>"` (unchanged) |
| `send_prompt` shell form | `Esc` + settle, then `! <stripped>` + `Enter` | `send_esc`, then `herdr pane run <id> "! <stripped>"` |
| `send_exit` | `Esc` + settle, then `/exit` + `Enter` | pane-gone-tolerant `esc`, then `herdr pane run <id> "/exit"` |

### Code changes

**tmux (`cafleet/src/multiplexer/tmux.rs`)**

- `send_prompt`: the final `send_literal_then_enter(..., esc_first)` argument changes from `!shell` to `true`. Payload construction is unchanged.
- `send_exit`: the `esc_first` argument changes from `false` to `true`. No other change — `send_literal_then_enter` already routes the Esc through `run_tolerating_pane_gone` with the caller's `ignore_missing` and timeout, so pane-gone tolerance covers the Esc automatically.

**herdr (`cafleet/src/multiplexer/herdr.rs`)**

- `send_prompt`: both branches lead with the existing `send_esc(target_pane_id)` (5 s timeout on the Esc, matching today's plain form); the `if shell` split reduces to the payload prefix.
- `send_exit`: extend `send_esc` with an `ignore_missing: bool` parameter that routes the `herdr pane send-keys <id> esc` through `run_tolerating_missing` (the settle sleep still follows, even on a tolerated missing pane); `send_exit` calls it with its own `ignore_missing` before the existing `pane run <id> "/exit"`. All existing `send_esc` callers pass `false`, preserving their current propagate-on-error semantics.

### Failure semantics

| Path | Esc failure behavior |
|---|---|
| `send_prompt`, both forms | Propagates and fails the dispatch — identical to today's plain form (no pane-gone tolerance; herdr keeps the 5 s timeout on the Esc). |
| `send_exit` | Follows `ignore_missing`: a pane-gone error on the Esc is tolerated exactly when the `/exit` itself would tolerate it; any other error propagates. |

### Documentation updates (written first)

| Target | Edit |
|---|---|
| `docs/docs/spec/multiplexer-backends.md` § *Backend matrix* | The two `send_prompt` rows both lead with `Esc`; the shell/plain difference is the payload prefix only. |
| `docs/docs/spec/multiplexer-backends.md` § *Prompt dispatch* | "The `shell` flag controls both the payload prefix and the Esc safeguard" → the flag controls only the payload prefix; both forms are Esc-safeguarded. |
| `docs/docs/spec/multiplexer-backends.md` § *The `Esc` safeguard* | Every keystroke path now leads with `Esc`, so the `Leads with Esc?` boolean column carries no information: state the universality in the lead-in prose and reduce the table to path / payload / why. Merge the two `member prompt` rows into one covering both forms, add the `/exit` teardown row, and delete the "deliberate omission / would mis-fire" content entirely. |
| `docs/docs/spec/cli-options.md` § `member prompt` | The flag controls only the payload prefix: both keystroke-sequence rows lead with `Esc` → settle, and the "an `Esc` before `! <cmd>` would mis-fire" sentence is deleted (the `--shell` row's stage-only + mandatory-ping behavior is unchanged). |
| `SPEC.md` § `member prompt` | The `--shell` form delivers `! <text>` Esc-safeguarded, same as the plain form; drop the no-Esc contrast. |
| `SPEC.md` tmux §6.5 (`send_exit`, `send_prompt`, Esc-first matrix) | `send_exit` and both `send_prompt` forms become **Esc-first=YES**; the matrix reads all-YES; `send_exit`'s entry notes the pane-gone tolerance covers the Esc. |
| `SPEC.md` herdr §6.5 (`send_exit`, `send_prompt`, `_SUBMIT_DELAY` paragraph) | Both `send_prompt` branches and `send_exit` are esc-then-run; the closing "`esc_first` maps to a discrete `herdr pane send-keys <id> esc` … on exactly the paths that use it" enumeration becomes all keystroke paths. |
| `skills/cafleet/reference/prompt-routing.md` § *The two forms* | Drop the stale "dispatches `! <cmd>` un-escaped" contrast with the plain form's "Esc-safeguarded" — both forms are Esc-safeguarded; the shell form's distinguishing behavior (stages only, mandatory ping follow-up) is unchanged. |

### Tests

Unit tests only (no live smoke test). The keystroke-shape tests assert the recorded runner events, including the settle sleeps. Per the removal rule, test names must not advertise the deleted behavior.

| File | Test | Change |
|---|---|---|
| `tmux.rs` | `send_prompt_shell_form_prefixes_bang_and_skips_the_esc` | Rename (e.g. `send_prompt_shell_form_is_esc_safeguarded_and_prefixes_bang`) and assert `Escape` + settle precede the `! <cmd>` payload. |
| `tmux.rs` | `send_exit_types_exit_without_esc_and_tolerates_a_missing_pane` | Rename (e.g. `send_exit_is_esc_safeguarded_and_tolerates_a_missing_pane`) and assert `Escape` + settle precede `/exit`, with pane-gone tolerated on every keystroke including the Esc. |
| `tmux.rs` | `send_exit_without_tolerance_propagates_the_failure` | Keep; still asserts propagation when `ignore_missing` is false (the failing keystroke may now be the Esc). |
| `herdr.rs` | `send_prompt_shell_and_plain_forms` | Assert both forms are `esc`-then-`run` with the settle sleep; the branches differ only in the `! ` prefix. |
| `herdr.rs` | `send_exit_runs_the_exit_line_and_tolerates_only_pane_not_found` | Assert the leading tolerant `esc` + settle before the `/exit` run; pane-not-found on either keystroke is tolerated under `ignore_missing`, other errors propagate. |

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

### Step 1: Documentation

- [x] Update `docs/docs/spec/multiplexer-backends.md` (backend matrix, § Prompt dispatch, § The `Esc` safeguard) per the Specification table <!-- completed: 2026-08-23T17:56 -->
- [x] Update `docs/docs/spec/cli-options.md` § `member prompt` (keystroke-sequence table + surrounding prose) per the Specification table <!-- completed: 2026-08-23T17:56 -->
- [x] Update `SPEC.md` (§ `member prompt`, tmux §6.5 entries + Esc-first matrix, herdr §6.5 entries + `_SUBMIT_DELAY` paragraph) <!-- completed: 2026-08-23T17:56 -->
- [x] Update `skills/cafleet/reference/prompt-routing.md` § *The two forms* wording <!-- completed: 2026-08-23T17:56 -->

### Step 2: tmux backend

- [x] `send_prompt`: pass `esc_first: true` unconditionally <!-- completed: 2026-08-23T18:05 -->
- [x] `send_exit`: pass `esc_first: true` <!-- completed: 2026-08-23T18:05 -->
- [x] Update the three tmux keystroke-shape tests per the Tests table <!-- completed: 2026-08-23T18:05 -->

### Step 3: herdr backend

- [x] `send_prompt`: lead both branches with `send_esc` <!-- completed: 2026-08-23T18:14 -->
- [x] `send_esc`: add the `ignore_missing` parameter; `send_exit` calls it with its own `ignore_missing` before the `/exit` run <!-- completed: 2026-08-23T18:14 -->
- [x] Update the two herdr keystroke-shape tests per the Tests table <!-- completed: 2026-08-23T18:14 -->

### Step 4: Verification

- [x] `mise //cafleet:test` passes <!-- completed: 2026-08-23T18:25 -->
- [x] `mise //cafleet:lint` passes <!-- completed: 2026-08-23T18:25 -->

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-08-21 | Initial draft |
| 2026-08-21 | Add `docs/docs/spec/cli-options.md` § `member prompt` to the documentation surfaces (reviewer round 1) |
