# Consolidate Alembic schema-management files into the `db` package

**Status**: Approved
**Progress**: 4/15 tasks complete
**Last Updated**: 2026-06-07

## Overview

Relocate the Alembic configuration (`alembic.ini`) and migration tree (`alembic/`) from the top of the `cafleet` package into the `db` subpackage, so every schema/DB concern lives under `cafleet/src/cafleet/db/`. This is a pure relocation plus reference-update change: zero behavior change, no `.ini` content edits, no migration rename, no schema change.

## Success Criteria

- [ ] `alembic.ini` lives at `cafleet/src/cafleet/db/alembic.ini` and the migration tree at `cafleet/src/cafleet/db/alembic/` (`env.py`, `script.py.mako`, `versions/0001_initial_schema.py`).
- [ ] `cafleet db init` and the Alembic smoke tests resolve the bundled `alembic.ini` from the `cafleet.db` package and run migrations to head successfully.
- [ ] `mise //cafleet:test`, `mise //cafleet:lint`, and `mise //cafleet:typecheck` are all green.
- [ ] `mise //cafleet:build` produces a wheel that bundles `db/alembic.ini` and `db/alembic/**/*`.
- [ ] No stale `importlib.resources.files("cafleet") / "alembic.ini"` lookup and no stale `src/cafleet/alembic` / `cafleet/src/cafleet/alembic` path reference remains anywhere in the repo outside `design-docs/`.

---

## Background

All other DB concerns already live under `cafleet/src/cafleet/db/` (`models.py`, `engine.py`), but the Alembic config and migration tree still sit one level up at the package root (`cafleet/src/cafleet/alembic.ini`, `cafleet/src/cafleet/alembic/`). Moving them under `db/` co-locates schema management with the models and engine it manages.

The move is mechanically safe because of two existing properties:

- `alembic.ini` uses `script_location = %(here)s/alembic`, which resolves relative to the `.ini` file's own directory. Moving `alembic.ini` and `alembic/` **together** into `db/` (the `.ini` as a *sibling* of `alembic/`) preserves the link with **no edit** to the `script_location` line.
- `alembic/env.py` imports are absolute (`from cafleet.config import settings`, `from cafleet.db.models import Base`), and `versions/0001_initial_schema.py` imports `from alembic import op` (the third-party package). None of these are affected by the relocation.

---

## Specification

### Target layout

```
cafleet/src/cafleet/db/
├── __init__.py
├── engine.py
├── models.py
├── alembic.ini              # moved from cafleet/src/cafleet/alembic.ini
└── alembic/                 # moved from cafleet/src/cafleet/alembic/
    ├── env.py
    ├── script.py.mako
    └── versions/
        └── 0001_initial_schema.py
```

`alembic.ini` is a **sibling** of `alembic/`, not inside it. With `script_location = %(here)s/alembic`, `%(here)s` is `cafleet/src/cafleet/db/`, so the script location resolves to `cafleet/src/cafleet/db/alembic/` — exactly the moved tree.

### What does NOT change

| Item | Reason |
|---|---|
| `alembic.ini` content (`script_location`, `sqlalchemy.url`, logging config) | `script_location` is relative to the `.ini`'s own dir; the `.ini` and `alembic/` move together. |
| `alembic/env.py` | Imports are absolute and already reference `cafleet.db.models` / `cafleet.config`. |
| `versions/0001_initial_schema.py` | Imports `from alembic import op` — the third-party package, unaffected by relocation. The revision id (`0001`, `down_revision=None`) is unchanged. |
| `docs/concepts/storage.md` | Its `alembic stamp head` / `alembic revision --autogenerate` mentions are **CLI commands**, not file paths. No edit. |
| `README.md` | Contains no Alembic path reference. No edit. |

### Packaging decisions

- **Keep `alembic/` a non-package.** Do **not** add an `__init__.py`. Alembic loads `env.py` and the `versions/*` scripts by *path* (via `script_location`), not by import, so the directory must stay a plain directory. The wheel `include` glob bundles the contents regardless of package status.
- **Namespace is safe.** The relocated directory's fully-qualified name would be `cafleet.db.alembic`, which is distinct from the top-level third-party `alembic`. Python 3 uses absolute imports, so `from alembic import …` inside `env.py` / `versions/*` resolves to the third-party package; there is no shadowing.

### Resource-lookup change

The runtime locates the bundled `alembic.ini` via `importlib.resources.files(...)`. Because the `.ini` moves into the `cafleet.db` package, the anchor package for the lookup changes from `cafleet` to `cafleet.db`:

```python
# before
importlib.resources.files("cafleet") / "alembic.ini"
# after
importlib.resources.files("cafleet.db") / "alembic.ini"
```

`cafleet.db` is an importable package (it has `__init__.py`), so `files("cafleet.db")` resolves correctly under both editable installs (source tree) and zipped-wheel installs (the existing `importlib.resources.as_file` materialization at each call site is unchanged).

### Live reference sites (complete inventory)

A repo-wide sweep (excluding `design-docs/`, which is the historical record and exempt per the removal rule) confirms exactly these live sites. All resource-lookup sites take the `cafleet` → `cafleet.db` change; the packaging and doc sites take the path change.

| # | Site | Current | After |
|---|---|---|---|
| 1 | `cafleet/src/cafleet/cli.py:191` (`db init`) | `files("cafleet") / "alembic.ini"` | `files("cafleet.db") / "alembic.ini"` |
| 2 | `cafleet/tests/test_alembic_smoke.py:21` | `files("cafleet") / "alembic.ini"` | `files("cafleet.db") / "alembic.ini"` |
| 3 | `cafleet/tests/test_alembic_smoke.py:64` | `files("cafleet") / "alembic.ini"` | `files("cafleet.db") / "alembic.ini"` |
| 4 | `cafleet/tests/_helpers.py:11` (dead — see note) | `files("cafleet") / "alembic.ini"` | `files("cafleet.db") / "alembic.ini"` |
| 5 | `cafleet/pyproject.toml:26` (wheel `include`) | `"src/cafleet/alembic.ini"` | `"src/cafleet/db/alembic.ini"` |
| 6 | `cafleet/pyproject.toml:27` (wheel `include`) | `"src/cafleet/alembic/**/*"` | `"src/cafleet/db/alembic/**/*"` |
| 7 | `docs/concepts/overview.md:46` (component table) | `cafleet/src/cafleet/alembic/` | `cafleet/src/cafleet/db/alembic/` |
| 8 | `docs/spec/data-model.md:7` (path mention) | `cafleet/src/cafleet/alembic/` | `cafleet/src/cafleet/db/alembic/` |

> `cafleet/src/cafleet/cli.py:186` contains the prose comment "the bundled ``alembic.ini``" — a description, not a path. No change.
>
> **Site #4 is dead code.** `cafleet/tests/_helpers.py` (`_make_alembic_cfg`, `_now_iso`) is imported by no test — its callers were deleted in design 0000061, which deferred pruning the orphaned helpers to a follow-up sweep. The `files("cafleet") / "alembic.ini"` lookup lives inside the uncalled `_make_alembic_cfg`, so it never executes under `mise //cafleet:test`. **Decision (minimal scope, per user Q1 "out of scope beyond move + reference updates" and Q6 "retarget `_helpers.py:11`"): keep the file and retarget the lookup to `cafleet.db`** so the Step-4 removal sweep finds zero stale `files("cafleet")` lookups; the 0000061 dead-code pruning stays out of scope for this relocation. The retargeted lookup is dead-but-correct — it is not test-covered.

### Atomicity

The file move (Step 2) and the in-tree reference updates (Step 3) MUST land in a **single commit**. If the resource lookup is retargeted to `cafleet.db` before the files move (or the files move before the lookups are retargeted), the test suite observes an inconsistent state and fails. The documentation updates (Step 1) may be a separate earlier commit per the docs-first rule, because docs describe the target state and do not affect test execution. This design doc itself is committed with the Step 1 docs-first commit, per the project's `.claude/rules/git-workflow.md` override (design docs are committed on the feature branch in this project).

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

### Step 1: Documentation (docs-first, per `.claude/rules/design-doc-numbering.md`)

- [x] Update `docs/concepts/overview.md:46` — change the `alembic/` component-table row location from `cafleet/src/cafleet/alembic/` to `cafleet/src/cafleet/db/alembic/`. <!-- completed: 2026-06-07T06:11 -->
- [x] Update `docs/spec/data-model.md:7` — change the schema-management path mention from `cafleet/src/cafleet/alembic/` to `cafleet/src/cafleet/db/alembic/`. <!-- completed: 2026-06-07T06:11 -->
- [x] Confirm no other doc / `README.md` / `SKILL.md` Alembic **path** reference remains (`docs/concepts/storage.md` keeps its `alembic stamp head` / `alembic revision` command mentions; `README.md` has none). <!-- completed: 2026-06-07T06:11 -->
- [x] Commit the Step 1 doc edits **and this design doc** (`design-docs/0000076-consolidate-alembic-into-db/design-doc.md`) as the docs-first commit, per `.claude/rules/git-workflow.md` (this project commits design docs on the feature branch). <!-- completed: 2026-06-07T06:12 -->

### Step 2: Relocate files with `git mv` (same commit as Step 3)

- [ ] `git mv cafleet/src/cafleet/alembic.ini cafleet/src/cafleet/db/alembic.ini` <!-- completed: -->
- [ ] `git mv cafleet/src/cafleet/alembic cafleet/src/cafleet/db/alembic` (moves `env.py`, `script.py.mako`, `versions/0001_initial_schema.py`; do not add `__init__.py`). <!-- completed: -->

### Step 3: Update in-tree reference sites (same commit as Step 2)

- [ ] `cafleet/src/cafleet/cli.py:191` — `files("cafleet") / "alembic.ini"` → `files("cafleet.db") / "alembic.ini"`. <!-- completed: -->
- [ ] `cafleet/tests/test_alembic_smoke.py:21` and `:64` — both → `files("cafleet.db") / "alembic.ini"`. <!-- completed: -->
- [ ] `cafleet/tests/_helpers.py:11` — → `files("cafleet.db") / "alembic.ini"`. <!-- completed: -->
- [ ] `cafleet/pyproject.toml` `[tool.hatch.build.targets.wheel].include` — `"src/cafleet/alembic.ini"` → `"src/cafleet/db/alembic.ini"` and `"src/cafleet/alembic/**/*"` → `"src/cafleet/db/alembic/**/*"`. <!-- completed: -->

### Step 4: Verify

- [ ] `mise //cafleet:test` green — `test_alembic_smoke.py` exercises the relocated bundle directly (migrate to head, expected tables, single `0001` revision) and `test_db_init.py` exercises it via `cafleet db init` → `cli.py:191`. (`_helpers.py:11` is dead code, not run by the suite; it is verified only by the Step-4 grep sweep below.) <!-- completed: -->
- [ ] `mise //cafleet:lint` green. <!-- completed: -->
- [ ] `mise //cafleet:typecheck` green. <!-- completed: -->
- [ ] `mise //cafleet:build` succeeds and the resulting wheel under `cafleet/dist/` contains `cafleet/db/alembic.ini` and `cafleet/db/alembic/**/*` (confirms the updated `include` globs bundle the relocated files). <!-- completed: -->
- [ ] Editable reinstall (`mise //cafleet:install`) so the global `cafleet` binary picks up the new resource path, then final removal-rule sweep: zero `files("cafleet") / "alembic.ini"` lookups and zero `src/cafleet/alembic` / `cafleet/src/cafleet/alembic` path references outside `design-docs/`. <!-- completed: -->

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-06-07 | Initial draft |
| 2026-06-07 | Review round 1: annotated site #4 (`_helpers.py:11`) as dead-but-retargeted; corrected Step-4 test-coverage claim (`test_db_init.py` over `_helpers.py`); added the docs-first design-doc commit task |
