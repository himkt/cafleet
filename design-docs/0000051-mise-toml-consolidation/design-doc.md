# mise.toml Consolidation under uv Workspace

**Status**: Approved
**Progress**: 11/23 tasks complete
**Last Updated**: 2026-05-10

## Overview

Consolidate the duplicated and drifted mise task layout across `mise.toml`, `cafleet/mise.toml`, and `admin/mise.toml` now that the repository has adopted uv workspace. After this change, the root `mise.toml` holds only monorepo configuration plus the shared `[tools]` block, all `//cafleet:*` tasks live in a single thin `cafleet/mise.toml` (using workspace-aware `uv run --package cafleet ...`), and `admin/mise.toml` stays separate because admin is a Bun project outside the uv workspace.

## Success Criteria

Each `//cafleet:*` and `//admin:*` task is verified at the strongest level its semantics allow. The matrix below pins the verification mode per task so success is testable; full task-by-task verification commands live in Step 5.

| Task | Verification mode |
|---|---|
| `//cafleet:test`, `//cafleet:lint`, `//cafleet:format`, `//cafleet:typecheck`, `//cafleet:sync`, `//cafleet:build`, `//admin:lint`, `//admin:build` | **Run to completion** from the repository root with exit code 0. |
| `//cafleet:dev`, `//admin:dev` | **Resolve and start** (mise dispatches the task body and the underlying server begins listening). Verified by successful startup, not termination. |
| `//cafleet:install`, `//admin:install` | **Resolve** (`mise tasks --all` lists them with the expected body). Skipped from execution to avoid mutating the developer's global uv tool registry / `node_modules`. |
| `//cafleet:publish` | **Chain resolves** (`mise tasks --all` lists it; every `{ task = "..." }` subtask reference points at a defined task). Skipped from execution because `uv publish` would push a real wheel to PyPI. |

- [ ] Every task in the matrix above passes its declared verification mode
- [ ] Root `mise.toml` defines no `[tasks.*]` sections — only `[monorepo]` and `[tools]`
- [ ] `cafleet/mise.toml` is the single source for every `//cafleet:*` task
- [ ] `mise //cafleet:sync` is a real, executable task (closing the existing documentation drift in `.claude/rules/commands.md`)
- [ ] No `dir = ...` overrides on cafleet tasks — they rely on mise's monorepo cwd auto-set
- [ ] `cafleet/mise.toml` task bodies use the workspace-aware `uv run --package cafleet ...` form
- [ ] `.claude/rules/commands.md` reflects the final task surface; no references to nonexistent tasks remain
- [ ] `README.md:110` no longer uses the about-to-be-deleted `mise uv-sync` short-form (replaced by `mise //cafleet:sync`)

---

## Background

The repository currently has three `mise.toml` files with overlapping responsibilities and one documentation drift bug:

| File | Tasks defined | Notes |
|---|---|---|
| `mise.toml` (root) | `build`, `publish`, `uv-sync` | Plus `[monorepo]` config and `[tools]` block (uv, bun) |
| `cafleet/mise.toml` | `dev`, `test`, `lint`, `format`, `typecheck`, `install` | All use `uv run ...` (no `--package` flag) — relies on cwd being set to `cafleet/` |
| `admin/mise.toml` | `lint`, `dev`, `install`, `build` | Bun-based; not a uv workspace member |

Two structural issues motivated this design:

1. **Split cafleet task surface.** The `build` and `publish` tasks live at the root but operate solely on the `cafleet` package (`uv build --wheel --package cafleet`, `uv publish`). The `uv-sync` task is workspace-wide but documented in `.claude/rules/commands.md` as `mise //cafleet:sync` — a task name that does not exist. Authors and agents are forced to remember which file each task lives in.
2. **uv workspace adoption.** The root `pyproject.toml` already declares `[tool.uv.workspace] members = ["./cafleet"]`. Workspace-aware invocations (`uv run --package cafleet ...`) make the package targeted by each command explicit, removing implicit dependence on cwd and surfacing intent in the task body.

Two constraints from prior decisions apply:

- The `mise //<package>:<task>` full-path notation (design 0000009) is a **hard requirement** — every reference in `.claude/settings.json`, `.claude/rules/commands.md`, both `CLAUDE.md` files, and downstream skill docs is encoded against it. mise's full-path resolution requires the package to appear in `monorepo.config_roots` **and** to have its own `mise.toml`. Therefore `cafleet/mise.toml` must continue to exist; the question is only what it contains.
- `admin/` is a Bun project (`bun lint`, `bun dev`, `bun install --frozen-lockfile`, `bun run build`). Its tasks invoke `bun`, which reads `package.json` and `bun.lock` from the working directory. A separate `admin/mise.toml` is the cleanest way to keep cwd correct for these commands without inventing per-task `dir = "admin"` overrides at root. This design therefore leaves `admin/mise.toml` untouched.

---

## Specification

### Target file shape

| File | Contents after this design |
|---|---|
| `mise.toml` (root) | `experimental_monorepo_root = true`, `[monorepo]` (unchanged), `[tools]` (unchanged). **No `[tasks.*]`.** |
| `cafleet/mise.toml` | All nine `//cafleet:*` tasks: `dev`, `test`, `lint`, `format`, `typecheck`, `install`, `build`, `publish`, `sync`. Bodies use `uv run --package cafleet ...` where they invoke a uv-managed binary. |
| `admin/mise.toml` | **Unchanged.** |

### Task migration map

| Old location | Old task body | New location | New task body |
|---|---|---|---|
| `mise.toml` `[tasks.build]` | `uv build --wheel --package cafleet` | `cafleet/mise.toml` `[tasks.build]` | `uv build --wheel --package cafleet --out-dir ./dist` (added `--out-dir ./dist`; without it, `uv build` writes the wheel to the uv-workspace root, not `cafleet/dist/`) |
| `mise.toml` `[tasks.publish]` | chained `//admin:install` → `//admin:build` → `build` → `uv publish` | `cafleet/mise.toml` `[tasks.publish]` | chained `//admin:install` → `//admin:build` → `//cafleet:build` → `uv publish` |
| `mise.toml` `[tasks.uv-sync]` | `uv sync --all-groups --all-packages --frozen` | `cafleet/mise.toml` `[tasks.sync]` | `uv sync --all-groups --all-packages --frozen` (unchanged body; renamed from `uv-sync` → `sync`) |
| `cafleet/mise.toml` `[tasks.dev]` | `uv run uvicorn cafleet.server:app --host 127.0.0.1 --port 8000` | same | `uv run --package cafleet uvicorn cafleet.server:app --host 127.0.0.1 --port 8000` |
| `cafleet/mise.toml` `[tasks.test]` | `uv run python -m pytest` | same | `uv run --package cafleet python -m pytest` |
| `cafleet/mise.toml` `[tasks.lint]` | `uv run ruff check .` + `uv run ruff format --check .` | same | `uv run --package cafleet ruff check .` + `uv run --package cafleet ruff format --check .` |
| `cafleet/mise.toml` `[tasks.format]` | `uv run ruff format .` | same | `uv run --package cafleet ruff format .` |
| `cafleet/mise.toml` `[tasks.typecheck]` | `uv run ty check` | same | `uv run --package cafleet ty check` |
| `cafleet/mise.toml` `[tasks.install]` | `uv tool install --reinstall --editable .` | same | `uv tool install --reinstall --editable .` (no `--package` — `uv tool install` is workspace-agnostic; the `.` resolves via mise's auto-cwd) |

### Final root `mise.toml`

```toml
experimental_monorepo_root = true

[monorepo]
config_roots = [
  "cafleet",
  "admin",
]

[tools]
"aqua:astral-sh/uv" = "latest"
"core:bun" = "latest"
```

### Final `cafleet/mise.toml`

```toml
[tasks.dev]
run = "uv run --package cafleet uvicorn cafleet.server:app --host 127.0.0.1 --port 8000"
description = "Start the admin WebUI server on 127.0.0.1:8000 (no hot-reload; restart manually between edits)"

[tasks.test]
run = "uv run --package cafleet python -m pytest"
description = "Run all tests"

[tasks.lint]
run = [
  "uv run --package cafleet ruff check .",
  "uv run --package cafleet ruff format --check .",
]
description = "Run ruff linter and format check"

[tasks.format]
run = "uv run --package cafleet ruff format ."
description = "Format code with ruff"

[tasks.typecheck]
run = "uv run --package cafleet ty check"
description = "Run ty type checker"

[tasks.install]
run = "uv tool install --reinstall --editable ."
description = "Install the cafleet CLI as an editable uv tool (source edits take effect without a second reinstall)"

[tasks.build]
run = "uv build --wheel --package cafleet --out-dir ./dist"
description = "Build the cafleet wheel into cafleet/dist/ (the explicit --out-dir prevents uv from writing to the uv-workspace root by default)"

[tasks.sync]
run = "uv sync --all-groups --all-packages --frozen"
description = "Sync uv workspace dependencies (all groups, all packages, frozen lockfile)"

[tasks.publish]
run = [
  { task = "//admin:install" },
  { task = "//admin:build" },
  { task = "//cafleet:build" },
  "uv publish",
]
description = "Publish cafleet to PyPI (builds admin assets, builds the wheel, then uv publish)"
```

### `admin/mise.toml` — left untouched

```toml
[tasks.lint]
run = "bun lint"

[tasks.dev]
run = "bun dev"

[tasks.install]
run = "bun install --frozen-lockfile"

[tasks.build]
run = "bun run build"
```

### Why no `dir =` on cafleet tasks

mise's monorepo task resolution sets the working directory to the config root of the file containing the task definition. A task in `cafleet/mise.toml` is invoked with `cwd = cafleet/` regardless of where the developer ran `mise` from. This auto-cwd is what lets:

- `uv tool install --reinstall --editable .` resolve `.` to `cafleet/` (uv tool install is not workspace-aware and needs a path)
- `uv run --package cafleet ty check` (no path arg) discover `[tool.ty]` config from `cafleet/pyproject.toml`
- `uv run --package cafleet ruff check .` apply `cafleet/pyproject.toml`'s `[tool.ruff.lint]` to the right tree

Since the auto-cwd already does the right thing, no task in this design needs an explicit `dir = ...` override.

### What genuinely must stay per-package

The investigation called out by Q8 surfaced three things that legitimately need a per-package home, and one that does not but is worth recording:

| Item | Per-package? | Why |
|---|---|---|
| `cafleet/mise.toml` itself | Yes (must exist) | Required by design 0000009's `mise //<package>:<task>` notation: full-path resolution requires both a `monorepo.config_roots` entry **and** a per-package `mise.toml`. Folding into root would break the `Bash(mise //cafleet*)` allow pattern in `.claude/settings.json` and every doc/skill mention. |
| `admin/mise.toml` | Yes (stays separate) | admin is a Bun project, not a uv workspace member. Bun reads `package.json` / `bun.lock` from cwd; mise's per-package `mise.toml` provides cwd auto-set to `admin/`. Folding into root would require explicit `dir = "admin"` on every admin task. The Bun toolchain has no `--package <name>` analog to make commands cwd-independent. |
| `[tool.ty]` config discovery | Per-package implicitly | ty discovers config from cwd's `pyproject.toml`. The `cafleet/mise.toml` location is what gives `cwd = cafleet/`, which is what makes `ty check` find `cafleet/pyproject.toml`. This is a behavioral dependency of *the cafleet/mise.toml location*, not a separate per-package settings file. |
| Per-package `[tools]` pins | No (single root `[tools]` is sufficient) | Per Q7, the user does not want per-package tool pins reserved. Both packages share `aqua:astral-sh/uv` and `core:bun` from the root `[tools]` block. Future divergence can be revisited as a separate design. |
| Per-package `[env]` | No (none today) | No package currently sets task-scoped env vars. `CAFLEET_DATABASE_URL`, `CAFLEET_BROKER_HOST`, `CAFLEET_BROKER_PORT` are all developer-supplied at runtime, not declared in mise. Nothing to migrate. |

### Documentation drift to fix in this design

- `.claude/rules/commands.md` line 10 references `mise //cafleet:sync`, but no `sync` task exists today (root has `uv-sync`, which is unreachable via `//cafleet:sync`). After this design, `//cafleet:sync` is a real task and the doc reference becomes accurate without further edits to that line.

### `.claude/settings.json` — no change needed

The current allow patterns `Bash(mise //admin*)` and `Bash(mise //cafleet*)` are wildcards that already cover the new `//cafleet:build`, `//cafleet:publish`, `//cafleet:sync` tasks. The deny entries for short-form `mise build` / `mise build *` continue to apply to the old root-level short forms (which are removed by this design). No allow / deny edits are required.

The pre-existing `Bash(uv run --package *)` deny entry remains correct: agents must continue to invoke commands via `mise //cafleet:*`, not by calling `uv run --package cafleet ...` directly. The fact that mise tasks now embed `uv run --package cafleet` internally is invisible to the permission layer (mise spawns the child process; Claude Code only matches the outer Bash invocation).

### Out of scope

- No changes to `admin/mise.toml`. Bun toolchain remains untouched.
- No changes to `pyproject.toml` files (root or `cafleet/`). uv workspace config is already correct.
- No changes to `.claude/settings.json` (wildcards already cover the new task names).
- No changes to `ARCHITECTURE.md` — it does not reference the mise task surface. (`README.md:110` **does** reference `mise uv-sync` and is updated in Step 1 — see below.)
- No new per-package `[tools]` pins or `[env]` blocks (per Q7 / Q8 user direction).
- No revisit of design 0000009's full-path convention. This design preserves it.

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

### Step 1: Update documentation

Per project rule (`.claude/rules/design-doc-numbering.md` § Implementation Order): documentation lands before code.

- [x] Confirm the `Sync dependencies: mise //cafleet:sync` line in `.claude/rules/commands.md` is unchanged (it becomes accurate after Step 2 lands) <!-- completed: 2026-05-10T22:01 -->
- [x] Add `Build cafleet wheel: mise //cafleet:build` and `Publish cafleet: mise //cafleet:publish` rows to `.claude/rules/commands.md` <!-- completed: 2026-05-10T22:03 -->
- [x] Update `README.md:110` from `mise uv-sync` to `mise //cafleet:sync` (the short-form root task is deleted in Step 4; this is also the only `README.md` reference that goes stale) <!-- completed: 2026-05-10T22:01 -->
- [x] Audit both `CLAUDE.md` files (root + `.claude/CLAUDE.md`) for any direct mention of `mise build`, `mise publish`, or `mise uv-sync` short-forms — none expected, but confirm and fix if found <!-- completed: 2026-05-10T22:01 -->
- [x] Audit `skills/*/SKILL.md` for any mise task examples that would need updating (expected: none — skills target the `cafleet` CLI surface, not dev tasks) <!-- completed: 2026-05-10T22:01 -->

### Step 2: Move root tasks into `cafleet/mise.toml`

- [x] Add `[tasks.build]`, `[tasks.publish]`, `[tasks.sync]` to `cafleet/mise.toml` per the Specification §Final cafleet/mise.toml block <!-- completed: 2026-05-10T22:04 -->
- [x] In the new `[tasks.publish]`, replace the `{ task = "build" }` step with `{ task = "//cafleet:build" }` (the `build` task is no longer at root) <!-- completed: 2026-05-10T22:04 -->

### Step 3: Rewrite cafleet task bodies to use `uv run --package cafleet`

- [x] Update `[tasks.dev]`, `[tasks.test]`, `[tasks.lint]` (both run-list entries), `[tasks.format]`, `[tasks.typecheck]` in `cafleet/mise.toml` to prepend `--package cafleet` to every `uv run` invocation <!-- completed: 2026-05-10T22:05 -->
- [x] Leave `[tasks.install]` body unchanged — `uv tool install` is not workspace-aware <!-- completed: 2026-05-10T22:05 -->

### Step 4: Strip tasks from root `mise.toml`

- [x] Delete `[tasks.build]`, `[tasks.publish]`, `[tasks.uv-sync]` from root `mise.toml` <!-- completed: 2026-05-10T22:06 -->
- [x] Confirm root `mise.toml` retains exactly: `experimental_monorepo_root = true`, `[monorepo]` block, `[tools]` block <!-- completed: 2026-05-10T22:06 -->

### Step 5: Verification

Per the Success Criteria matrix, each task is verified at the strongest level its semantics allow. Tasks that mutate global state or never terminate are verified by *resolution* only.

**Run-to-completion checks** (exit code 0 expected):

- [ ] Run `mise tasks --all` and confirm output lists exactly: `//admin:build`, `//admin:dev`, `//admin:install`, `//admin:lint`, `//cafleet:build`, `//cafleet:dev`, `//cafleet:format`, `//cafleet:install`, `//cafleet:lint`, `//cafleet:publish`, `//cafleet:sync`, `//cafleet:test`, `//cafleet:typecheck` (13 tasks total; no root-level tasks) <!-- completed: -->
- [ ] Run `mise //cafleet:test` from the repository root — passes <!-- completed: -->
- [ ] Run `mise //cafleet:lint` from the repository root — passes <!-- completed: -->
- [ ] Run `mise //cafleet:format` from the repository root on a clean tree — leaves the tree clean (catches regressions in the `uv run --package cafleet ruff format .` rewrite from Step 3) <!-- completed: -->
- [ ] Run `mise //cafleet:typecheck` from the repository root — passes <!-- completed: -->
- [ ] Run `mise //cafleet:sync` from the repository root — succeeds (real task, not the previously-undefined doc reference) <!-- completed: -->
- [ ] Run `mise //cafleet:build` from the repository root — produces a wheel under `cafleet/dist/` (the `--out-dir ./dist` flag combined with mise's auto-cwd `cafleet/` keeps the artifact out of the uv-workspace root) <!-- completed: -->
- [ ] Run `mise //admin:lint` and `mise //admin:build` to confirm admin tasks remain unaffected <!-- completed: -->

**Resolution-only checks** (intentionally not executed; rationale for skipping execution given inline):

- [ ] Confirm `mise //cafleet:dev` resolves and starts (binds `127.0.0.1:8000`); kill with Ctrl+C. *Execution skipped to completion because the uvicorn server never terminates.* <!-- completed: -->
- [ ] Confirm `mise //admin:dev` resolves and starts (Vite dev server prints its listen URL); kill with Ctrl+C. *Execution skipped to completion: long-running dev server.* <!-- completed: -->
- [ ] Confirm `mise //cafleet:install` and `mise //admin:install` appear in `mise tasks --all` with the expected task bodies. *Execution skipped to avoid mutating the developer's global uv-tool registry / `admin/node_modules`.* <!-- completed: -->
- [ ] Confirm `mise //cafleet:publish` appears in `mise tasks --all`, and that every `{ task = "..." }` subtask reference inside its body (`//admin:install`, `//admin:build`, `//cafleet:build`) resolves to a defined task. *Execution skipped because `uv publish` would push a real wheel to PyPI.* <!-- completed: -->

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-05-09 | Initial draft |
| 2026-05-09 | Reviewer revision: Success Criteria reframed as a per-task verification-mode matrix; `README.md:110` (`mise uv-sync`) brought into scope and added to Step 1; first Step 1 task split into two factual tasks; Step 5 split into run-to-completion vs resolution-only checks with `//cafleet:format` added and rationale for every skip recorded |
| 2026-05-10 | User revision: `[tasks.build]` body changed from `uv build --wheel --package cafleet` to `uv build --wheel --package cafleet --out-dir ./dist` so the wheel lands in `cafleet/dist/` instead of the uv-workspace root; migration map and Step 5 wheel-location check updated to match |
| 2026-05-10 | Approved |
