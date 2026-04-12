# Project Rename: hikyaku → cafleet

**Status**: Approved
**Progress**: 18/36 tasks complete
**Last Updated**: 2026-04-12

## Overview

Rename the project from "hikyaku" to "cafleet" across the entire codebase. This is a comprehensive rename affecting the Python package, CLI command, environment variables, database paths, documentation, skills, plugins, CI/CD, and settings. The rename is a hard switch with no backward-compatibility period.

## Success Criteria

- [ ] Python package installs as `cafleet` and CLI command is `cafleet`
- [ ] All imports use `from cafleet` / `import cafleet`
- [ ] Environment variables use `CAFLEET_*` prefix
- [ ] Default database path is `~/.local/share/cafleet/registry.db`
- [ ] All documentation references use "CAFleet" (title case) or "cafleet" (code)
- [ ] `mise //cafleet:test` passes, `mise //cafleet:dev` starts the server
- [ ] CI/CD pipeline passes on the renamed codebase
- [ ] Skills and plugin definitions reference "cafleet"

---

## Background

The project is being renamed from "hikyaku" to "cafleet". The rename touches ~74 files across every layer of the codebase. Key decisions:

- **Root directory rename**: Out of scope (user handles manually)
- **GitHub repository rename**: Out of scope (handled separately)
- **Historical design documents** (0000001–0000018): Preserved as-is with original "hikyaku" name
- **PyPI publishing**: Not applicable (package is not published)
- **Backward compatibility**: None — hard switch for env vars, paths, and CLI
- **Database content**: No data migration needed (schema uses generic names)

### Naming Convention

| Context | Old | New |
|---------|-----|-----|
| Code / CLI / package | `hikyaku` | `cafleet` |
| Documentation headings | `Hikyaku` | `CAFleet` |
| Environment variables | `HIKYAKU_*` | `CAFLEET_*` |
| Default data directory | `~/.local/share/hikyaku/` | `~/.local/share/cafleet/` |

---

## Specification

### Scope Summary

The rename is organized into categories below. Each category lists every file that must change and the nature of the change.

### 1. Documentation Files

| File | Changes |
|------|---------|
| `README.md` | "Hikyaku" → "CAFleet", `hikyaku` → `cafleet` in code blocks, CLI examples, paths |
| `ARCHITECTURE.md` | Same pattern as README |
| `docs/spec/cli-options.md` | CLI command name, env var names, paths |
| `docs/spec/data-model.md` | Any "hikyaku" references |
| `docs/spec/registry-api.md` | Any "hikyaku" references |
| `docs/spec/a2a-operations.md` | No "hikyaku" references — no changes needed |
| `docs/spec/webui-api.md` | No "hikyaku" references — no changes needed |
| `CLAUDE.md` (root) | Project name, package paths, CLI command, skill references |
| `.claude/CLAUDE.md` | Project name, package paths, CLI command, skill references |
| `.claude/rules/commands.md` | `mise //hikyaku:*` → `mise //cafleet:*` |

### 2. Skill & Plugin Files

| File / Directory | Changes |
|------------------|---------|
| `.claude/skills/hikyaku/` → `.claude/skills/cafleet/` | Directory rename |
| `.claude/skills/hikyaku/SKILL.md` → `.claude/skills/cafleet/SKILL.md` | All CLI examples, env var names, paths |
| `.claude/skills/hikyaku-monitoring/` → `.claude/skills/cafleet-monitoring/` | Directory rename |
| `.claude/skills/hikyaku-monitoring/SKILL.md` → `.claude/skills/cafleet-monitoring/SKILL.md` | All CLI examples referencing `hikyaku` |
| `.claude/skills/update-readme/SKILL.md` | "Hikyaku" → "CAFleet" in project description, `hikyaku` → `cafleet` in CLI command, `mise //hikyaku:dev` → `mise //cafleet:dev`, `pip install hikyaku` → `pip install cafleet` |
| `.claude-plugin/plugin.json` | `"name": "hikyaku"` → `"name": "cafleet"`, skill path (repository URL left unchanged — updated with separate GitHub rename) |
| `.claude-plugin/marketplace.json` | `"name": "hikyaku"` → `"name": "cafleet"`, description |

### 3. Package Directory Structure

The workspace member directory and inner Python package must both be renamed:

```
Before:                          After:
hikyaku/                         cafleet/
  pyproject.toml                   pyproject.toml
  mise.toml                        mise.toml
  src/hikyaku/                     src/cafleet/
    __init__.py                      __init__.py
    cli.py                           cli.py
    server.py                        server.py
    config.py                        config.py
    ...                              ...
    alembic/                         alembic/
    alembic.ini                      alembic.ini
    api/                             api/
    db/                              db/
    webui/                           webui/
  tests/                           tests/
    conftest.py                      conftest.py
    test_*.py                        test_*.py
```

Rename sequence (order matters):
1. `git mv hikyaku/src/hikyaku hikyaku/src/cafleet` — inner Python package first
2. `git mv hikyaku cafleet` — outer workspace member directory

### 4. Package Configuration

| File | Changes |
|------|---------|
| Root `pyproject.toml` | `members = ["hikyaku"]` → `["cafleet"]`, `hikyaku = { workspace = true }` → `cafleet`, dev dependency, `tool.ty.src` include path |
| `cafleet/pyproject.toml` | `name = "hikyaku"` → `"cafleet"`, console script `hikyaku` → `cafleet`, hatch build `packages` and `include` paths |
| Root `mise.toml` | `config_roots` entry `"hikyaku"` → `"cafleet"`, env var `HIKYAKU_URL` → `CAFLEET_URL` |
| `cafleet/mise.toml` | `run = "uv run src/hikyaku/server.py"` → `"uv run src/cafleet/server.py"` |

### 5. Python Source Code — Imports

14 source files under `cafleet/src/cafleet/` contain "hikyaku" references (imports, paths, or string literals) and must be updated:

```python
# Before
from hikyaku.config import settings
from hikyaku.db.models import ...
import hikyaku.broker_client as api

# After
from cafleet.config import settings
from cafleet.db.models import ...
import cafleet.broker_client as api
```

**Files with references to update** (all under `cafleet/src/cafleet/` after rename):

- `server.py` — Multiple `from hikyaku.*` imports
- `cli.py` — `from hikyaku import broker_client as api`, other imports
- `config.py` — Database path `~/.local/share/hikyaku/` → `~/.local/share/cafleet/`, env var `HIKYAKU_DATABASE_URL` → `CAFLEET_DATABASE_URL`
- `executor.py` — `from hikyaku.*` imports
- `task_store.py` — `from hikyaku.*` imports
- `registry_store.py` — `from hikyaku.*` imports
- `webui_api.py` — `from hikyaku.*` imports
- `agent_card.py` — `from hikyaku.*` imports
- `coding_agent.py` — `from hikyaku.*` imports
- `tmux.py` — "hikyaku" in string literals
- `db/engine.py` — `from hikyaku.*` imports
- `db/models.py` — "hikyaku" in string literals or comments
- `api/registry.py` — `from hikyaku.*` imports
- `alembic/env.py` — `from hikyaku.config`, `from hikyaku.db.models`

### 6. Python Source Code — config.py

Special attention required for `config.py`:

```python
# Before
def _default_database_url() -> str:
    db_path = Path("~/.local/share/hikyaku/registry.db").expanduser()
    return f"sqlite+aiosqlite:///{db_path}"

class Settings(BaseSettings):
    database_url: str = Field(
        default_factory=_default_database_url,
        validation_alias="HIKYAKU_DATABASE_URL",
    )

# After
def _default_database_url() -> str:
    db_path = Path("~/.local/share/cafleet/registry.db").expanduser()
    return f"sqlite+aiosqlite:///{db_path}"

class Settings(BaseSettings):
    database_url: str = Field(
        default_factory=_default_database_url,
        validation_alias="CAFLEET_DATABASE_URL",
    )
```

### 7. Test Files

22 test files under `cafleet/tests/` contain "hikyaku" references and must be updated (`__init__.py` has none). Same mechanical replacement as source code.

Additionally, any test that references:
- The database path `~/.local/share/hikyaku/` (e.g., `test_db_engine.py`)
- The CLI command name `hikyaku` (e.g., `test_cli.py`, `test_cli_member.py`)
- Environment variable names `HIKYAKU_*` (e.g., `test_session_cli.py`)

### 8. Frontend Build Configuration

```typescript
// admin/vite.config.ts — Before
build: {
  outDir: '../hikyaku/src/hikyaku/webui',
}

// After
build: {
  outDir: '../cafleet/src/cafleet/webui',
}
```

### 9. CI/CD

```yaml
# .github/workflows/ci.yml — Before
working-directory: hikyaku

# After
working-directory: cafleet
```

### 10. Settings & Permissions

`.claude/settings.json` permission patterns must be updated:

```json
// Before
"Bash(mise //hikyaku*)",
"Bash(hikyaku *)",
"Bash(uv run hikyaku *)"

// After
"Bash(mise //cafleet*)",
"Bash(cafleet *)",
"Bash(uv run cafleet *)"
```

### 11. Alembic Migrations

Existing migration version files (`0001` through `0005`) do not contain "hikyaku" references and require no changes. The `alembic.ini` uses relative paths (`script_location = %(here)s/alembic`) which also need no changes beyond the directory rename. Only `alembic/env.py` has `from hikyaku.*` imports that need updating (covered by the bulk import update in Step 5).

### 12. Admin Frontend Source Code

4 TSX files in `admin/src/components/` contain "hikyaku" references:

| File | Line(s) | Reference | Type |
|------|---------|-----------|------|
| `Dashboard.tsx` | 16 | `hikyaku.sender.${sessionId}` | localStorage key |
| `Dashboard.tsx` | 50 | `Hikyaku —` | UI heading text |
| `Dashboard.tsx` | 83 | `hikyaku register` | CLI name in UI text |
| `SenderSelector.tsx` | 11 | `hikyaku.sender.${sessionId}` | localStorage key |
| `SessionPicker.tsx` | 27 | `Hikyaku — Sessions` | UI heading text |
| `SessionPicker.tsx` | 53 | `hikyaku-registry session create` | CLI name in UI text |
| `Sidebar.tsx` | 60 | `hikyaku register` | CLI name in UI text |

**localStorage key migration**: The localStorage keys change from `hikyaku.sender.*` to `cafleet.sender.*`. Existing stored sender preferences will be silently orphaned — this is acceptable. No migration logic is added; users simply re-select their sender on first use after the rename.

### 13. Plugin Repository URL

The `plugin.json` field `"repository"` currently points to `https://github.com/himkt/hikyaku`. Since the GitHub repository rename is out of scope for this design doc, the repository URL is **left unchanged** in this rename pass. It will be updated when the GitHub repository rename is performed separately. A `TODO` comment is not added — the repository URL will simply be updated as part of the separate GitHub rename effort.

### Out of Scope

- Root repository directory rename (user-managed)
- GitHub repository rename (handled separately)
- Historical design documents 0000001–0000018 (preserved as-is)
- PyPI publishing
- Database content migration (schema uses generic names)

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

### Step 1: Documentation Update

Per project rules, documentation is updated before code.

- [x] Update `ARCHITECTURE.md`: replace "Hikyaku" → "CAFleet", `hikyaku` → `cafleet` in paths/commands <!-- completed: 2026-04-12T10:00 -->
- [x] Update `README.md`: replace "Hikyaku" → "CAFleet", `hikyaku` → `cafleet` in paths/commands/examples <!-- completed: 2026-04-12T10:01 -->
- [x] Update `docs/spec/cli-options.md`: CLI command, env vars, paths <!-- completed: 2026-04-12T10:01 -->
- [x] Update `docs/spec/data-model.md`: any "hikyaku" references <!-- completed: 2026-04-12T10:01 -->
- [x] Update `docs/spec/registry-api.md`: any "hikyaku" references <!-- completed: 2026-04-12T10:01 -->
- [x] Verify `docs/spec/a2a-operations.md` and `docs/spec/webui-api.md` contain no "hikyaku" references (no-op) <!-- completed: 2026-04-12T10:02 -->
- [x] Update `CLAUDE.md` (root): project name, package, CLI, skill names <!-- completed: 2026-04-12T10:02 -->
- [x] Update `.claude/CLAUDE.md`: project name, package, CLI, skill names <!-- completed: 2026-04-12T10:02 -->
- [x] Update `.claude/rules/commands.md`: `mise //hikyaku:*` → `mise //cafleet:*` <!-- completed: 2026-04-12T10:02 -->

### Step 2: Skills & Plugin Files

- [x] Rename `.claude/skills/hikyaku/` → `.claude/skills/cafleet/` via `git mv` <!-- completed: 2026-04-12T10:10 -->
- [x] Update `.claude/skills/cafleet/SKILL.md`: all CLI commands, env vars, paths <!-- completed: 2026-04-12T10:11 -->
- [x] Rename `.claude/skills/hikyaku-monitoring/` → `.claude/skills/cafleet-monitoring/` via `git mv` <!-- completed: 2026-04-12T10:10 -->
- [x] Update `.claude/skills/cafleet-monitoring/SKILL.md`: all CLI references <!-- completed: 2026-04-12T10:11 -->
- [x] Update `.claude/skills/update-readme/SKILL.md`: project name, CLI command, mise task, pip install <!-- completed: 2026-04-12T10:11 -->
- [x] Update `.claude-plugin/plugin.json`: name, skill path (repository URL left unchanged — updated with separate GitHub rename) <!-- completed: 2026-04-12T10:12 -->
- [x] Update `.claude-plugin/marketplace.json`: name, description <!-- completed: 2026-04-12T10:12 -->

### Step 3: Package Directory Rename

- [x] `git mv hikyaku/src/hikyaku hikyaku/src/cafleet` (inner Python package) <!-- completed: 2026-04-12T10:20 -->
- [x] `git mv hikyaku cafleet` (outer workspace member directory) <!-- completed: 2026-04-12T10:20 -->

### Step 4: Package Configuration

- [ ] Update root `pyproject.toml`: workspace members, sources, dev deps, ty.src include <!-- completed: -->
- [ ] Update `cafleet/pyproject.toml`: package name, console script, hatch build paths <!-- completed: -->
- [ ] Update root `mise.toml`: monorepo config_roots, `HIKYAKU_URL` → `CAFLEET_URL` <!-- completed: -->
- [ ] Update `cafleet/mise.toml`: dev task server path <!-- completed: -->

### Step 5: Python Source Code

- [ ] Update all `from hikyaku` / `import hikyaku` to `from cafleet` / `import cafleet` across all 14 source files with references (includes `alembic/env.py`) <!-- completed: -->
- [ ] Update `config.py`: database path `~/.local/share/hikyaku/` → `~/.local/share/cafleet/`, env var `HIKYAKU_DATABASE_URL` → `CAFLEET_DATABASE_URL` <!-- completed: -->

### Step 6: Test Files

- [ ] Update all `from hikyaku` / `import hikyaku` to `from cafleet` / `import cafleet` across all 22 test files with references <!-- completed: -->
- [ ] Update any hardcoded database paths, CLI command names, and env var names in tests <!-- completed: -->

### Step 7: Frontend Build Configuration & Source Code

- [ ] Update `admin/vite.config.ts`: `outDir` path `../hikyaku/src/hikyaku/webui` → `../cafleet/src/cafleet/webui` <!-- completed: -->
- [ ] Update `admin/src/components/Dashboard.tsx`: localStorage key `hikyaku.sender.*` → `cafleet.sender.*`, heading "Hikyaku" → "CAFleet", CLI text `hikyaku register` → `cafleet register` <!-- completed: -->
- [ ] Update `admin/src/components/SenderSelector.tsx`: localStorage key `hikyaku.sender.*` → `cafleet.sender.*` <!-- completed: -->
- [ ] Update `admin/src/components/SessionPicker.tsx`: heading "Hikyaku" → "CAFleet", CLI text `hikyaku-registry session create` → `cafleet session create` <!-- completed: -->
- [ ] Update `admin/src/components/Sidebar.tsx`: CLI text `hikyaku register` → `cafleet register` <!-- completed: -->

### Step 8: CI/CD

- [ ] Update `.github/workflows/ci.yml`: `working-directory: hikyaku` → `cafleet` <!-- completed: -->

### Step 9: Settings & Permissions

- [ ] Update `.claude/settings.json`: replace `hikyaku` with `cafleet` in all allow/deny patterns <!-- completed: -->

### Step 10: Lock File & Verification

- [ ] Run `uv sync` to regenerate `uv.lock` with new package name <!-- completed: -->
- [ ] Run `mise //cafleet:test` to verify all tests pass <!-- completed: -->
- [ ] Run `mise //:lint` and `mise //:typecheck` to verify no lint/type errors <!-- completed: -->

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-04-12 | Initial draft |
| 2026-04-12 | Approved after 2 review rounds (admin frontend, alembic, localStorage, plugin URL, file counts) |
