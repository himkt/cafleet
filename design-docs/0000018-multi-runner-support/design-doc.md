# Multi-Runner Support (Codex Integration)

**Status**: Approved
**Progress**: 6/18 tasks complete
**Last Updated**: 2026-04-12

## Overview

Add support for OpenAI Codex CLI as an alternative coding agent backend alongside Claude Code. Both agents run inside tmux panes; the change is limited to which binary and flags are passed to `tmux split-window`. A new `--coding-agent claude|codex` flag on `hikyaku member create` selects the backend, defaulting to `claude` for backward compatibility.

## Success Criteria

- [ ] `hikyaku member create --coding-agent codex` spawns a Codex process in a tmux pane with `--approval-mode auto-edit`
- [ ] `hikyaku member create` without `--coding-agent` behaves identically to the current implementation (spawns Claude)
- [ ] `hikyaku member list` displays which coding agent is running in each pane
- [ ] `hikyaku member capture` and `hikyaku member delete` work unchanged for both agent types
- [ ] `agent_placements` table tracks the coding agent type via a `coding_agent` column
- [ ] All existing tests pass without modification (backward compatibility)

---

## Background

Hikyaku currently hardcodes `claude` as the binary spawned in tmux panes for member agents. The `tmux.split_window()` function appends `["claude", claude_prompt]` to the tmux command, and the CLI has no mechanism to select an alternative. OpenAI's Codex CLI is a compatible coding agent that also runs interactively in a terminal, accepts a positional prompt argument, and shares the same tmux-based lifecycle as Claude Code.

**Prerequisite: Codex CLI tmux compatibility (verified by manual testing)**

- Codex runs inside a tmux pane the same way Claude Code does.
- `/exit` typed in the terminal cleanly shuts down a Codex session (confirmed by manual testing).
- `tmux capture-pane` captures Codex output identically to Claude Code output.

Since both agents share spawn, capture, and shutdown semantics within tmux, the integration requires only parameterizing the command construction.

---

## Specification

### CodingAgentConfig

A new module `client/src/hikyaku_client/coding_agent.py` encapsulates agent-specific details.

```python
from dataclasses import dataclass, field
import shutil


@dataclass(frozen=True)
class CodingAgentConfig:
    """Configuration for a coding agent binary that runs inside a tmux pane."""

    name: str                            # "claude" | "codex"
    binary: str                          # executable name on PATH
    extra_args: list[str] = field(default_factory=list)  # flags between binary and prompt
    default_prompt_template: str = ""    # str.format() template with {director_name}, {director_agent_id}

    def build_command(self, prompt: str) -> list[str]:
        return [self.binary, *self.extra_args, prompt]

    def ensure_available(self) -> None:
        if shutil.which(self.binary) is None:
            raise RuntimeError(f"'{self.binary}' binary not found on PATH")
```

Two built-in configurations:

| Field | `CLAUDE` | `CODEX` |
|-------|----------|---------|
| `name` | `"claude"` | `"codex"` |
| `binary` | `"claude"` | `"codex"` |
| `extra_args` | `[]` | `["--approval-mode", "auto-edit"]` |
| `default_prompt_template` | Current `_resolve_prompt` text (includes `Load Skill(hikyaku)`) | Codex-specific variant (no `Skill()` reference; explicit `hikyaku` CLI instructions) |

A `CODING_AGENTS: dict[str, CodingAgentConfig]` registry maps names to configs. A `get_coding_agent(name: str) -> CodingAgentConfig` helper raises `ValueError` for unknown names.

**Default prompt templates:**

Claude (existing behavior, moved to constant):
```
Load Skill(hikyaku). Your agent_id is $HIKYAKU_AGENT_ID.
You are a member of the team led by {director_name} ({director_agent_id}).
Wait for instructions via `hikyaku poll --agent-id $HIKYAKU_AGENT_ID`.
```

Codex (no Skill() mechanism; explicit CLI usage):
```
Your agent_id is $HIKYAKU_AGENT_ID.
You are a member of the team led by {director_name} ({director_agent_id}).
Check for instructions using `hikyaku poll --agent-id $HIKYAKU_AGENT_ID`.
Use `hikyaku ack --agent-id $HIKYAKU_AGENT_ID --task-id <id>` to acknowledge messages
and `hikyaku send --agent-id $HIKYAKU_AGENT_ID --to <id> --text "..."` to reply.
```

### tmux.py Changes

`split_window()` is generalized to accept an arbitrary command instead of a hardcoded Claude prompt.

| Before | After |
|--------|-------|
| `claude_prompt: str` parameter | `command: list[str]` parameter |
| `args += ["claude", claude_prompt]` | `args += command` |

No other functions change. `send_exit()`, `capture_pane()`, `select_layout()`, `director_context()`, and `ensure_tmux_available()` are agent-agnostic and remain untouched.

### CLI Changes

**`member create`** gains a `--coding-agent` option:

```python
@click.option(
    "--coding-agent",
    type=click.Choice(["claude", "codex"]),
    default="claude",
    show_default=True,
    help="Coding agent to spawn in the tmux pane",
)
```

The command flow changes:

1. Pre-flight adds `coding_agent_config.ensure_available()` after `tmux.ensure_tmux_available()`.
2. `_resolve_prompt()` accepts a `CodingAgentConfig` parameter. When no positional prompt is given, it fetches the director's name via `api.list_agents()` (existing behavior), then calls `coding_agent_config.default_prompt_template.format(director_name=..., director_agent_id=...)` to produce the final prompt string.
3. The `placement` dict sent to the registry includes `"coding_agent": coding_agent_config.name`.
4. `tmux.split_window()` receives `command=coding_agent_config.build_command(prompt)` instead of `claude_prompt=prompt`.

**`member delete`, `member capture`, `member list`** require no changes (tmux operations are agent-agnostic).

### Database Migration

Alembic migration `0005_add_coding_agent.py`:

```python
def upgrade() -> None:
    op.add_column(
        "agent_placements",
        sa.Column("coding_agent", sa.String(), nullable=False, server_default="claude"),
    )

def downgrade() -> None:
    op.drop_column("agent_placements", "coding_agent")
```

The `server_default="claude"` ensures existing rows are backfilled, maintaining backward compatibility.

### SQLAlchemy Model

`AgentPlacement` in `registry/src/hikyaku_registry/db/models.py` gains:

```python
coding_agent: Mapped[str] = mapped_column(String, nullable=False, server_default="claude")
```

### Pydantic Models

In `registry/src/hikyaku_registry/models.py`:

| Model | Change |
|-------|--------|
| `PlacementCreate` | Add `coding_agent: str = "claude"` |
| `PlacementView` | Add `coding_agent: str` |
| `PlacementPatch` | No change (still patches `tmux_pane_id` only) |

### Registry Store

In `registry/src/hikyaku_registry/registry_store.py`:

| Method | Change |
|--------|--------|
| `create_agent_with_placement()` | Store `placement.coding_agent` in the `AgentPlacement` row |
| `get_placement()` | Include `coding_agent` in the returned dict |
| `list_placements_for_director()` | Add `AgentPlacement.coding_agent` to the SELECT columns and include it in the returned placement dict |

### Registry API

In `registry/src/hikyaku_registry/api/registry.py`:

| Endpoint | Change |
|----------|--------|
| `POST /agents` | Pass through `coding_agent` from `PlacementCreate`; include in `PlacementView` response |
| `GET /agents/{id}` | Include `coding_agent` in `PlacementView` |
| `GET /agents` (with `director_agent_id`) | Include `coding_agent` in member `PlacementView` |
| `PATCH /agents/{id}/placement` | Include `coding_agent` in `PlacementView` response (no new patch field) |

### Output Formatting

In `client/src/hikyaku_client/output.py`:

| Function | Change |
|----------|--------|
| `format_member()` | Add `coding_agent` line: `backend:   codex` |
| `format_member_list()` | Add `backend` column to table header and rows, sourced from `placement.coding_agent` (default: `claude`) |

### Skill Documentation

| File | Change |
|------|--------|
| `.claude/skills/hikyaku/SKILL.md` | Document `--coding-agent` flag on `member create`. Add Codex example. Update `member list` output columns. |
| `.claude/skills/hikyaku-monitoring/SKILL.md` | Add note that monitoring protocol is agent-agnostic; `member capture` and `/exit` shutdown work for both agent types. |

### Future Work (Out of Scope)

- **Director-as-Codex**: Allow the Director itself to be a Codex instance. Requires investigation into whether Codex supports cron/loop capabilities needed for monitoring.
- **Additional coding agents**: The `CodingAgentConfig` dataclass and `CODING_AGENTS` registry make it straightforward to add new agents (e.g., Aider, Cursor CLI) by defining new configs without code changes to tmux/CLI/registry.

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

### Step 1: Documentation Updates

- [x] Update `ARCHITECTURE.md` with multi-runner support (CodingAgentConfig, --coding-agent flag) <!-- completed: 2026-04-12T15:00 -->
- [x] Update `docs/` with coding agent configuration details <!-- completed: 2026-04-12T15:00 -->
- [x] Update `README.md` to reflect new `--coding-agent` option <!-- completed: 2026-04-12T15:00 -->
- [x] Update `.claude/skills/hikyaku/SKILL.md` — document `--coding-agent` flag, add Codex example, update `member list` output format <!-- completed: 2026-04-12T15:00 -->
- [x] Update `.claude/skills/hikyaku-monitoring/SKILL.md` — add note that monitoring is agent-agnostic <!-- completed: 2026-04-12T15:00 -->

### Step 2: CodingAgentConfig Module

- [x] Create `client/src/hikyaku_client/coding_agent.py` with `CodingAgentConfig` dataclass, `CLAUDE`/`CODEX` constants, `CODING_AGENTS` registry, and `get_coding_agent()` helper <!-- completed: 2026-04-12T18:30 -->

### Step 3: Generalize tmux.split_window()

- [ ] Change `split_window()` parameter from `claude_prompt: str` to `command: list[str]` and replace `args += ["claude", claude_prompt]` with `args += command` <!-- completed: -->

### Step 4: Database Migration and Model

- [ ] Create Alembic migration `0005_add_coding_agent.py` adding `coding_agent TEXT NOT NULL DEFAULT 'claude'` to `agent_placements` <!-- completed: -->
- [ ] Add `coding_agent` column to `AgentPlacement` model in `registry/src/hikyaku_registry/db/models.py` <!-- completed: -->

### Step 5: Registry Layer (Models, Store, API)

- [ ] Update `PlacementCreate` and `PlacementView` in `registry/src/hikyaku_registry/models.py` <!-- completed: -->
- [ ] Update `create_agent_with_placement()`, `get_placement()`, and `list_placements_for_director()` in `registry/src/hikyaku_registry/registry_store.py` <!-- completed: -->
- [ ] Update `register_agent`, `get_agent_detail`, `list_agents`, and `patch_placement` endpoints in `registry/src/hikyaku_registry/api/registry.py` <!-- completed: -->

### Step 6: CLI Changes

- [ ] Add `--coding-agent` option to `member_create()` in `client/src/hikyaku_client/cli.py` <!-- completed: -->
- [ ] Update `_resolve_prompt()` to accept `CodingAgentConfig` and use its `default_prompt_template` <!-- completed: -->
- [ ] Update `member_create()` to call `coding_agent_config.ensure_available()`, pass `coding_agent` in placement, and use `build_command()` for `split_window()` <!-- completed: -->

### Step 7: Output Formatting

- [ ] Update `format_member()` and `format_member_list()` in `client/src/hikyaku_client/output.py` to display `coding_agent` <!-- completed: -->

### Step 8: Tests

- [ ] Add unit tests for `CodingAgentConfig` (build_command, ensure_available, get_coding_agent) <!-- completed: -->
- [ ] Update existing `split_window` and `member_create` tests for the new `command` parameter and `--coding-agent` flag <!-- completed: -->

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-04-12 | Initial draft |
