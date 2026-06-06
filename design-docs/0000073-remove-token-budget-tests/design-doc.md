# Remove token-budget testing

**Status**: Approved
**Progress**: 5/5 tasks complete
**Last Updated**: 2026-06-06

## Overview

Remove all token-budget testing from the repository: the real pytest suite under `cafleet/tests/token_budget/`, the orphaned manual-measurement stub under repo-root `tests/token_budget/`, and the test-suite description paragraph in `docs/concepts/token-reduction.md`. This is a deliberate, eyes-open operator decision that knowingly drops the only automated regression guards against silent inflation of the message envelope, the member spawn prompt, and the core skill file.

## Success Criteria

- [x] Both `token_budget/` directories are deleted (7 tracked files: 3 repo-root + 4 package).
- [x] The repo-root `tests/` directory is removed entirely (it is empty once `tests/token_budget/` is gone).
- [x] `docs/concepts/token-reduction.md` no longer describes the regression suite or the baseline stub; the techniques table describing the actual architectural features is retained verbatim.
- [x] `mise //cafleet:test` collects and passes with the 12 token-budget tests gone.
- [x] No reference to `token_budget` or to the repo-root `tests/` path remains in any tracked file outside `design-docs/` (git and this design doc are the only historical record).

---

## Background

Two unrelated directories share the name `token_budget`:

1. **Repo-root `tests/token_budget/`** — an orphaned manual-measurement stub (`__init__.py`, `scenarios/__init__.py`, `scenarios/idle_3_member_baseline_stub.py`). It is a `__main__` script run by hand (`uv run python tests/token_budget/scenarios/idle_3_member_baseline_stub.py`), never collected by CI: `mise //cafleet:test` runs pytest from the `cafleet/` package directory, so it only discovers `cafleet/tests/` and never the repo-root `tests/`. The stub writes `tests/token_budget/measurement_results.md`, which is untracked and read by nothing. Its own docstring calls it a "deferred stub" — a Step 0 artifact for design `0000049-token-reduction` (now Complete). Repo-root `tests/` contains nothing else, so it becomes empty.

2. **Package `cafleet/tests/token_budget/`** — the real pytest suite (`__init__.py`, `test_envelope_size.py`, `test_skill_size.py`, `test_spawn_prompt_size.py`), 12 tests total. These are self-contained byte / line / ratio budget assertions with no dependency on the stub. They were classified "off-limits / load-bearing" by design `0000061-test-suite-cleanup`, and `test_envelope_size.py` was last modified by design `0000071-cli-ux-cleanup`.

### What the guards protect, and why this is eyes-open

The package suite is the only automated guardrail preventing silent inflation of three context-cost surfaces. Removing it drops each guard:

| Guarded surface | Source under guard | Tests dropped |
|---|---|---|
| Per-poll message envelope (compact byte budgets + slim-vs-full ratio) | `output.render_task` / `output.format_json` | `test_envelope_size.py` (4) |
| Core skill file size (≤ 350 lines / ≤ 25 KB, reference-split intact) | `skills/cafleet/SKILL.md` + `skills/cafleet/reference/*.md` | `test_skill_size.py` (4) |
| Member spawn prompt size (≤ 420 chars / ≤ 4 lines after substitution) | `cli._MEMBER_PROMPT_TEMPLATE` | `test_spawn_prompt_size.py` (4) |

Designs `0000049-token-reduction`, `0000061-test-suite-cleanup`, and `0000071-cli-ux-cleanup` established and maintained these guards. Design `0000073` deliberately removes them. The Director presented the tradeoff (loss of the byte / line / ratio guards) to the operator, who chose to remove everything anyway. This is recorded here as a deliberate decision, not an oversight. Future drift in envelope, spawn-prompt, or skill-file size will no longer fail at PR time; that risk is accepted.

---

## Specification

### Removal surface (complete, verified)

| Target | Action |
|---|---|
| Repo-root `tests/token_budget/__init__.py` | Delete |
| Repo-root `tests/token_budget/scenarios/__init__.py` | Delete |
| Repo-root `tests/token_budget/scenarios/idle_3_member_baseline_stub.py` | Delete |
| Repo-root `tests/` directory | Delete (empty after the above) |
| `cafleet/tests/token_budget/__init__.py` | Delete |
| `cafleet/tests/token_budget/test_envelope_size.py` | Delete |
| `cafleet/tests/token_budget/test_skill_size.py` | Delete |
| `cafleet/tests/token_budget/test_spawn_prompt_size.py` | Delete |
| `docs/concepts/token-reduction.md` lines 26–34 | Remove the separating blank line + trailing test-suite paragraph |

### Documentation edit detail

In `docs/concepts/token-reduction.md`, remove only the closing paragraph that describes "The token-budget regression suite under `tests/token_budget/`", the `idle_3_member_baseline_stub.py` stub, and "The char-anchored regression tests ... are the canonical contract" (the block at lines 26–34 — the blank line separating it from the table plus the paragraph itself; the three `token_budget` references sit at lines 27, 30, 33). Deleting through line 34 (the file's last line) leaves the techniques table's last row (line 25) as the final line with a single trailing newline.

**Keep** the techniques table above it (lines 16–25). That table documents the actual architectural features — compact rendered envelope, slim member spawn prompt, skill-file split, `member list --activity`, capture defaults, persisted-shape simplification, inline message preview, agent render slim — all of which remain in place. Only the test-suite description is removed; the introductory prose (lines 7–14) describing why bytes matter also stays.

### What stays (per `~/.claude/rules/removal.md`)

- No design docs are touched. Per `~/.claude/rules/removal.md`, every design doc is part of the canonical historical record. Of the existing docs, only `0000049-token-reduction`, `0000061-test-suite-cleanup`, and `0000071-cli-ux-cleanup` reference the token-budget tests (verified: they are the only three containing the string `token_budget`); those three established and maintained the guards this change removes.
- This design doc records the removal and its accepted tradeoff.
- Git history and the git log.

This is a clean removal: no replacement guard is added, and no restoration / migration plan is recorded in the doc body. After the change, the repository reads as if the token-budget tests never existed; the historical record lives in git and in the design docs above.

### Non-changes (verified)

- **No pytest config change.** `cafleet/pyproject.toml` has no `[tool.pytest.ini_options]`, no `testpaths`, and no markers; `cafleet/tests/conftest.py` does not reference `token_budget`. Removing the directories needs no config edit.
- **No orphaned path reference.** Nothing references the repo-root `tests/` path — no CI job, no mise task, no `testpaths` entry. `mise //cafleet:test` runs pytest from `cafleet/`, so it never collected repo-root `tests/` to begin with. The only documentation mentions are the three lines in `token-reduction.md` removed in Step 1; there are no `token_budget` references in `skills/` or `README.md`.

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.
>
> Order follows `.claude/rules/design-doc-numbering.md` — documentation FIRST, then code, then verification.

### Step 1: Update documentation

- [x] In `docs/concepts/token-reduction.md`, delete the trailing test-suite paragraph together with the blank line separating it from the table (lines 26–34: the "token-budget regression suite", the `idle_3_member_baseline_stub.py` description, and the "canonical contract" sentence). The file then ends at the techniques table's last row (line 25) with a single trailing newline. Keep the techniques table (lines 16–25) and the intro prose (lines 7–14) unchanged. <!-- completed: 2026-06-06T04:36 -->

### Step 2: Remove the package test suite

- [x] Delete `cafleet/tests/token_budget/` and its 4 files (`__init__.py`, `test_envelope_size.py`, `test_skill_size.py`, `test_spawn_prompt_size.py`). <!-- completed: 2026-06-06T04:44 -->

### Step 3: Remove the repo-root stub and the empty tests/ dir

- [x] Delete repo-root `tests/token_budget/` and its 3 files (`__init__.py`, `scenarios/__init__.py`, `scenarios/idle_3_member_baseline_stub.py`), then delete the now-empty repo-root `tests/` directory. <!-- completed: 2026-06-06T04:44 -->

### Step 4: Verify

- [x] Run `mise //cafleet:test` and confirm the remaining suite collects and passes with the 12 token-budget tests gone. <!-- completed: 2026-06-06T04:44 -->
- [x] Run `git grep token_budget -- ':!design-docs/'` (scoped to tracked files, so it auto-skips the gitignored `site/` build output and other generated artifacts); confirm no residual reference remains in source, docs, skills, README, or config. The only matches before this change are `docs/concepts/token-reduction.md` and the repo-root stub — both removed by Steps 1 and 3. <!-- completed: 2026-06-06T04:44 -->

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-06-06 | Initial draft |
