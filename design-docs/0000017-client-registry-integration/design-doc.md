# Merge Client and Registry into a Single Package

**Status**: Approved
**Progress**: 12/32 tasks complete
**Last Updated**: 2026-04-12

## Overview

Consolidate the two Python packages (`hikyaku-registry` and `hikyaku-client`) into a single `hikyaku` package with a flat module namespace and unified CLI entry point. The primary motivation is deployment simplicity: one `pip install hikyaku` gives users both the broker server and the agent CLI.

## Success Criteria

- [ ] Single installable package `hikyaku` ships both server and CLI functionality
- [ ] Unified `hikyaku` CLI replaces both `hikyaku` (client) and `hikyaku-registry` (admin) commands
- [ ] All existing tests pass from a single `tests/` directory at the project root
- [ ] `admin/` remains an independent workspace member building into `hikyaku/src/hikyaku/webui/`
- [ ] `mise` tasks (`//hikyaku:test`, `//hikyaku:dev`, `//hikyaku:lint`) work correctly
- [ ] CI pipeline passes with updated paths

---

## Background

The Hikyaku monorepo currently ships two Python packages via uv workspace:

| Package | Import Namespace | Console Script | Dependencies | Files |
|---|---|---|---|---|
| `hikyaku-registry` | `hikyaku_registry` | `hikyaku-registry` | FastAPI, SQLAlchemy, aiosqlite, Alembic, a2a-sdk, click, pydantic, pydantic-settings | 20 source + 16 test |
| `hikyaku-client` | `hikyaku_client` | `hikyaku` | click, httpx, a2a-sdk | 5 source + 4 test |

The two packages share zero code and have no cross-imports. The separation was a reasonable starting point but adds friction: two `pyproject.toml` files, two test suites, two mise task namespaces, and two packages to install. Since the project has no external consumers requiring lightweight client-only installs, merging simplifies everything.

---

## Specification

### Target Package Structure

```
hikyaku/                          # uv workspace member (was registry/ + client/)
├── pyproject.toml                # merged dependencies, single console script
├── mise.toml                     # merged mise tasks
├── tests/                        # merged test suite (was registry/tests/ + client/tests/)
│   ├── conftest.py
│   ├── test_auth.py
│   ├── test_db_engine.py
│   ├── test_db_init.py
│   ├── test_db_models.py
│   ├── test_alembic_smoke.py
│   ├── test_alembic_0002_upgrade.py
│   ├── test_registry_api.py
│   ├── test_registry_store.py
│   ├── test_executor.py
│   ├── test_task_store.py
│   ├── test_session_cli.py
│   ├── test_webui_api.py
│   ├── test_webui_mount.py
│   ├── test_a2a.py
│   ├── test_cli.py               # merged from client/tests/test_cli.py
│   ├── test_cli_register.py
│   ├── test_cli_member.py
│   └── test_tmux.py
└── src/
    └── hikyaku/
        ├── __init__.py
        ├── server.py              # was main.py — ASGI app entry point
        ├── config.py              # settings (from registry)
        ├── auth.py                # session + agent-id resolution (from registry)
        ├── cli.py                 # unified CLI (merged from both)
        ├── models.py              # Pydantic request/response models (from registry)
        ├── executor.py            # BrokerExecutor (from registry)
        ├── task_store.py          # TaskStore (from registry)
        ├── agent_card.py          # broker Agent Card (from registry)
        ├── registry_store.py      # agent + session CRUD (from registry)
        ├── webui_api.py           # WebUI API router (from registry)
        ├── broker_client.py       # httpx helpers (was client/api.py — renamed to avoid api/ conflict)
        ├── output.py              # CLI output formatting (from client)
        ├── tmux.py                # tmux subprocess helper (from client)
        ├── db/
        │   ├── __init__.py
        │   ├── engine.py
        │   └── models.py          # SQLAlchemy declarative models
        ├── api/
        │   ├── __init__.py
        │   └── registry.py        # Registry REST router
        ├── alembic.ini            # bundled in wheel
        ├── alembic/
        │   ├── env.py
        │   └── versions/
        │       ├── 0001_initial_schema.py
        │       ├── 0002_local_simplification.py
        │       ├── 0003_add_origin_task_id.py
        │       └── 0004_add_agent_placements.py
        └── webui/                 # admin build output (populated by mise //admin:build)
```

### Module Rename Mapping

All imports change from `hikyaku_registry.*` / `hikyaku_client.*` to `hikyaku.*`.

| Old Module | New Module | Rename Reason |
|---|---|---|
| `hikyaku_registry.main` | `hikyaku.server` | Clarify purpose: this is the ASGI server entry point |
| `hikyaku_client.api` | `hikyaku.broker_client` | Avoid collision with `hikyaku.api/` package |
| `hikyaku_registry.cli` + `hikyaku_client.cli` | `hikyaku.cli` | Merged into single CLI module |
| All other `hikyaku_registry.*` | `hikyaku.*` | Namespace flattening (no rename) |
| All other `hikyaku_client.*` | `hikyaku.*` | Namespace flattening (no rename) |

### Unified CLI Design

Single entry point `hikyaku` with the following command tree:

```
hikyaku [--json]
├── env                                                # prints HIKYAKU_URL and HIKYAKU_SESSION_ID
├── db init                                            # from hikyaku-registry
├── session create [--label] [--json]                  # from hikyaku-registry
├── session list [--json]                              # from hikyaku-registry
├── session show SESSION_ID [--json]                   # from hikyaku-registry
├── session delete SESSION_ID                          # from hikyaku-registry
├── register --name --description [--skills]           # from hikyaku client
├── send --agent-id --to --text                        # from hikyaku client
├── broadcast --agent-id --text                        # from hikyaku client
├── poll --agent-id [--since] [--page-size]            # from hikyaku client
├── ack --agent-id --task-id                           # from hikyaku client
├── cancel --agent-id --task-id                        # from hikyaku client
├── get-task --agent-id --task-id                      # from hikyaku client
├── agents --agent-id [--id]                           # from hikyaku client
├── deregister --agent-id                              # from hikyaku client
├── member create --agent-id --name --description [PROMPT...]  # from hikyaku client
├── member delete --agent-id --member-id               # from hikyaku client
├── member list --agent-id                             # from hikyaku client
└── member capture --agent-id --member-id [--lines]    # from hikyaku client
```

**CLI merge strategy**: The registry CLI (`main` group with `db` and `session` subgroups) and client CLI (`cli` group with top-level commands and `member` subgroup) merge into a single click group. The `--json` global flag and `HIKYAKU_SESSION_ID`/`HIKYAKU_URL` env var handling from the client CLI apply to the merged root group. The `db` and `session` subgroups do not use these env vars (they access SQLite directly via `config.settings`).

### Merged pyproject.toml (`hikyaku/pyproject.toml`)

```toml
[project]
name = "hikyaku"
version = "0.1.0"
description = "A2A-native message broker and agent registry for coding agents"
requires-python = ">=3.12"
dependencies = [
    "fastapi",
    "uvicorn[standard]",
    "a2a-sdk",
    "sqlalchemy>=2.0",
    "alembic",
    "aiosqlite",
    "click",
    "pydantic",
    "pydantic-settings",
    "httpx",
]

[project.scripts]
hikyaku = "hikyaku.cli:cli"

[build-system]
requires = ["hatchling"]
build-backend = "hatchling.build"

[tool.hatch.build.targets.wheel]
packages = ["src/hikyaku"]
include = [
  "src/hikyaku/alembic.ini",
  "src/hikyaku/alembic/**/*",
  "src/hikyaku/webui/**/*",
]

[tool.pytest.ini_options]
asyncio_mode = "auto"
```

### Root pyproject.toml Changes

The root `pyproject.toml` becomes a **virtual workspace root** (no `[project]` table) to avoid a naming collision with the `hikyaku` member package. The `[project]` section with `name = "hikyaku"` and `version = "0.1.0"` is removed entirely.

```toml
# No [project] table — virtual workspace root

[tool.uv.workspace]
members = ["hikyaku", "admin"]

[tool.uv.sources]
hikyaku = { workspace = true }

[dependency-groups]
dev = [
    "hikyaku",
    "pytest>=9.0.2",
    "pytest-asyncio>=1.3.0",
    "ruff>=0.11.0",
    "ty>=0.0.26",
]

[tool.ty]

[tool.ty.environment]
python-version = "3.12"

[tool.ty.src]
include = ["hikyaku/src"]

[tool.ty.analysis]
allowed-unresolved-imports = [
    "a2a.*",
    "pydantic_settings.*",
    "starlette.*",
    "fastapi.*",
    "uvicorn",
    "click.*",
    "httpx.*",
]

[tool.ruff]
exclude = ["vendor/"]
```

### Mise Task Definitions

Root `mise.toml` — update monorepo config_roots:

```toml
experimental_monorepo_root = true

[monorepo]
config_roots = [
  "hikyaku",
  "admin",
]

[tasks.lint]
run = "uv run ruff check ."
description = "Run ruff linter"

[tasks.format]
run = "uv run ruff format --check ."
description = "Check code formatting with ruff"

[tasks.typecheck]
run = "uv run ty check"
description = "Run ty type checker"

[env]
HIKYAKU_URL = "http://localhost:8000"
```

`hikyaku/mise.toml` — replaces both `registry/mise.toml` and `client/mise.toml`:

```toml
[tasks.dev]
run = "uv run src/hikyaku/server.py"
description = "Start the broker server"

[tasks.test]
run = "uv run python -m pytest"
description = "Run all tests"

[tasks.lint]
run = [
  "uv --preview-features format format --diff",
  "uvx ruff check",
  "uv run mypy src",
]
description = "Check formatting, lint, and type-check"

[tasks.format]
run = [
  "uv --preview-features format format",
  "uvx ruff check --fix",
]
description = "Auto-format and fix lint issues"
```

The `mypy` and `uv format` commands are retained from the existing `registry/mise.toml` and `client/mise.toml` (both had identical lint/format task definitions).

### Admin Build Path Update

In `admin/vite.config.ts`, change the `outDir`:

```typescript
// Before
outDir: '../registry/src/hikyaku_registry/webui',
// After
outDir: '../hikyaku/src/hikyaku/webui',
```

### Import Rewrite Summary

Every Python file that imports from `hikyaku_registry` or `hikyaku_client` must be updated:

```python
# Before
from hikyaku_registry.config import settings
from hikyaku_registry.db.models import Base
from hikyaku_client.api import send_message
from hikyaku_client import output

# After
from hikyaku.config import settings
from hikyaku.db.models import Base
from hikyaku.broker_client import send_message
from hikyaku import output
```

The `alembic/env.py` imports also change:

```python
# Before
from hikyaku_registry.config import settings
from hikyaku_registry.db.models import Base

# After
from hikyaku.config import settings
from hikyaku.db.models import Base
```

The `importlib.resources` reference in `cli.py` changes:

```python
# Before
importlib.resources.files("hikyaku_registry") / "alembic.ini"

# After
importlib.resources.files("hikyaku") / "alembic.ini"
```

### Files to Delete After Migration

```
registry/                  # entire directory
client/                    # entire directory
```

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

### Step 1: Update Documentation

- [x] Update `ARCHITECTURE.md` to reflect single-package structure, new module paths, unified CLI <!-- completed: 2026-04-12T12:00 -->
- [x] Update `README.md` with new install/usage instructions <!-- completed: 2026-04-12T12:00 -->
- [x] Update `docs/` if any files reference old package names or CLI commands <!-- completed: 2026-04-12T12:00 -->
- [x] Update `.claude/CLAUDE.md` and `.claude/rules/commands.md` with new mise task paths <!-- completed: 2026-04-12T12:00 -->
- [x] Update affected `SKILL.md` files (hikyaku skill, update-readme skill) with new CLI surface <!-- completed: 2026-04-12T12:00 -->

### Step 2: Create Package Directory and Config

- [x] Create `hikyaku/` directory with `pyproject.toml` (merged dependencies, single console script) <!-- completed: 2026-04-12T09:17 -->
- [x] Create `hikyaku/mise.toml` (merged lint, format, dev, test tasks) <!-- completed: 2026-04-12T09:17 -->
- [x] Create `hikyaku/src/hikyaku/__init__.py` <!-- completed: 2026-04-12T09:17 -->

### Step 3: Move Registry Source Files

- [x] Move all files from `registry/src/hikyaku_registry/` to `hikyaku/src/hikyaku/`, renaming `main.py` to `server.py` <!-- completed: 2026-04-12T09:20 -->
- [x] Move `db/`, `api/`, `alembic/`, `alembic.ini`, `webui/` subdirectories <!-- completed: 2026-04-12T09:20 -->

### Step 4: Move Client Source Files

- [x] Move `client/src/hikyaku_client/api.py` to `hikyaku/src/hikyaku/broker_client.py` <!-- completed: 2026-04-12T09:21 -->
- [x] Move `output.py` and `tmux.py` to `hikyaku/src/hikyaku/` <!-- completed: 2026-04-12T09:21 -->

### Step 5: Merge CLIs

- [ ] Merge registry CLI (`db`, `session` subgroups) and client CLI (`env` command, top-level commands, `member` subgroup) into single `hikyaku/src/hikyaku/cli.py` <!-- completed: -->

### Step 6: Rewrite All Internal Imports

- [ ] Replace `from hikyaku_registry` with `from hikyaku` in all source files <!-- completed: -->
- [ ] Replace `from hikyaku_client` with `from hikyaku` in all source files, changing `api` references to `broker_client` <!-- completed: -->
- [ ] Update `importlib.resources.files("hikyaku_registry")` to `importlib.resources.files("hikyaku")` <!-- completed: -->

### Step 7: Update Alembic Configuration

- [ ] Update `alembic/env.py` imports to use `hikyaku.*` namespace <!-- completed: -->
- [ ] Verify `alembic.ini` `script_location` still resolves correctly within the new package <!-- completed: -->

### Step 8: Update Root Workspace Config

- [ ] Update root `pyproject.toml`: remove `[project]` table (virtual workspace root), update workspace members, uv sources, dev dependencies, ty src paths <!-- completed: -->
- [ ] Update root `mise.toml`: monorepo config_roots <!-- completed: -->

### Step 9: Update Admin Build Path

- [ ] Change `outDir` in `admin/vite.config.ts` from `'../registry/src/hikyaku_registry/webui'` to `'../hikyaku/src/hikyaku/webui'` <!-- completed: -->

### Step 10: Merge Tests

- [ ] Move `registry/tests/*.py` to `hikyaku/tests/` <!-- completed: -->
- [ ] Move `client/tests/*.py` to `hikyaku/tests/`, resolving any filename conflicts <!-- completed: -->
- [ ] Update all test imports from `hikyaku_registry`/`hikyaku_client` to `hikyaku` <!-- completed: -->

### Step 11: Remove Old Directories

- [ ] Delete `registry/` directory <!-- completed: -->
- [ ] Delete `client/` directory <!-- completed: -->

### Step 12: Verify

- [ ] Run `uv sync` from project root <!-- completed: -->
- [ ] Run `mise //hikyaku:test` — all tests pass <!-- completed: -->
- [ ] Run `mise //:lint` — no lint errors <!-- completed: -->
- [ ] Run `mise //:format` — formatting clean <!-- completed: -->
- [ ] Run `mise //:typecheck` — no type errors <!-- completed: -->
- [ ] Run `mise //admin:build` — admin builds into new path <!-- completed: -->

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-04-12 | Initial draft |
