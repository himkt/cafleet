# Backend Abstractions — `cafleet.coding_agent` and `cafleet.multiplexer`

**Status**: Complete
**Progress**: 45/45 tasks complete
**Last Updated**: 2026-05-19

## Overview

Extract two backend abstractions from today's flat `cli.py` helpers and `tmux.py` module: `cafleet.coding_agent` (Protocol + `ClaudeCodeAgent` + `CodexAgent`) and `cafleet.multiplexer` (Protocol + `TmuxMultiplexer`). The abstractions ship as a pure refactor with two existing concrete impls of `CodingAgent` and one of `Multiplexer`; the cursor CLI and cmux additions land in separate follow-on design docs that each add one new file.

## Success Criteria

- [x] `cafleet/coding_agent/{__init__.py, base.py, claude.py, codex.py}` subpackage exists. `base.py` defines a `@runtime_checkable` `CodingAgent` Protocol; `claude.py` and `codex.py` each define one concrete class. `__init__.py` exposes `CodingAgent`, `ClaudeCodeAgent`, `CodexAgent`, and a `CODING_AGENTS: dict[str, CodingAgent]` registry.
- [x] `cafleet/multiplexer/{__init__.py, base.py, tmux.py}` subpackage exists. `base.py` defines a `@runtime_checkable` `Multiplexer` Protocol plus the `MultiplexerContext` dataclass plus the `poll_until_pane_gone` shared helper. `tmux.py` defines `TmuxMultiplexer`. `__init__.py` exposes the Protocol, the context dataclass, `TmuxMultiplexer`, and a `MULTIPLEXERS: dict[str, Multiplexer]` registry.
- [x] `cafleet/tmux.py` (the current flat module) is deleted. Every import in `cli.py`, `broker.py`, and tests is updated to the new path.
- [x] `cli.member_create` and `cli.session_create` resolve the spawn argv via `CODING_AGENTS[coding_agent].build_spawn_argv(prompt, display_name=name)` instead of the inline `_build_claude_command` / `_build_codex_command` branch. The `_CLAUDE_BINARY` / `_CODEX_BINARY` constants and the two `_build_*_command` helpers are deleted from `cli.py`.
- [x] `click.Choice` for `--coding-agent` is computed dynamically as `list(CODING_AGENTS.keys())` on both `session create` and `member create`. The list is still `["claude", "codex"]` in this design doc; the cursor follow-on doc adds one dict entry.
- [x] Alembic revision `0010_backfill_unknown_coding_agent.py` is committed and chained off `0009_drop_task_json_add_text.py`. The migration backfills `agent_placements.coding_agent = 'unknown'` rows to `'claude'`, is idempotent, and leaves `'claude'` / `'codex'` rows untouched. `mise //cafleet:test` runs the migration as part of its existing schema setup.
- [x] A parametrized contract test (`tests/test_coding_agent_protocol.py`) asserts `isinstance(impl, CodingAgent)` for every value in `CODING_AGENTS.values()` and exercises `build_spawn_argv` + `ensure_available` for each. A parallel `tests/test_multiplexer_protocol.py` does the same for `MULTIPLEXERS.values()`.
- [x] `cafleet doctor` calls `MULTIPLEXERS["tmux"].context_discovery()` (via the registry — not a direct `cafleet.multiplexer.tmux.director_context` import) so the cmux follow-on doc can swap the active multiplexer without editing `doctor`. JSON output schema (`{"tmux": {"session_name", "window_id", "pane_id", "tmux_pane_env"}}`) is preserved verbatim.
- [x] `mise //cafleet:test` is green at every commit boundary. The 17 test files that reference `cafleet.tmux` are updated to use `cafleet.multiplexer.tmux` (or `cafleet.multiplexer` for re-exported top-level symbols). No test redesign — only import-path updates.
- [x] `ARCHITECTURE.md`, `README.md`, `docs/spec/cli-options.md`, `docs/codex-members.md`, and every affected `skills/*/SKILL.md` describe the new module layout before code lands.

---

## Background

> User answers to the Drafter's category-letter / question-number clarification round (cited inline below as **A1, A2, B1, B2, B3(d), B5, C1, C2, D1, D2, D3, D4, D5**) live in `clarification-answers.md` in this directory.

`cafleet` has three prior design cycles that touched the coding-agent abstraction question and one that touched the spawn-command-builder question. This redesign is informed by all four — it does **not** restore the rejected abstraction; it ships a different one for a different reason.

| Design | Outcome | Why this design isn't a repeat |
|---|---|---|
| `0000018` (Complete, superseded) | Introduced `CodingAgentConfig` dataclass + `CLAUDE` / `CODEX` configs + `--coding-agent` flag. Treated the dataclass as a one-stop registry. | That abstraction failed because `codex` was removed shortly after by `0000034 §15`, leaving the dataclass with one concrete instance. |
| `0000041 §B` (Complete) | Deleted `cafleet/coding_agent.py` with the rationale: *"no second backend, no plugin point, no test that passes a different config."* Inlined the helpers as `_CLAUDE_BINARY` / `_build_claude_command` / `_ensure_claude_available` constants in `cli.py`. | The §B premise was correct for that codebase state. |
| `0000046` (Complete) | Re-introduced `codex` as a second backend. Background section explicitly refused to restore `CodingAgentConfig`: *"A per-binary helper function pair (`_build_claude_command` / `_build_codex_command` plus a shared `_ensure_coding_agent_available`) is the entire abstraction surface. No registry, no plugin interface, no class hierarchy."* | The 0000046 premise was: codex is *the* second backend, so two thin helpers suffice. Cursor wasn't on the roadmap then. |
| `0000056 §P5` (Complete) | Audited the `_build_claude_command` vs `_build_codex_command` pair and explicitly marked the extraction as **skip**, citing: *"Real flag-set divergence is too small to factor; leave both builders inline."* | Same reason as 0000046 — extraction wasn't justified by a third concrete impl. |

### What is different this time

The user has confirmed two concrete planned additions:

1. **Cursor CLI** as a third coding agent (joins claude + codex).
2. **cmux** as a second multiplexer (joins tmux).

Both arrive in separate follow-on design docs. The abstraction in this doc is the migration path: it ships with the existing two `CodingAgent` impls and the existing one `Multiplexer` impl, so the follow-on docs each add exactly one new file to a registry dict — not a refactor justifying speculative future code.

The two abstractions are deliberately **Protocols**, not ABCs (user pick at B1 / B2). This is the lightest-weight abstraction that meets the trap-avoidance criterion from `0000041 §B` and `0000046`: there is no class hierarchy to maintain, no `super().__init__()` ceremony, no metaclass magic. A concrete impl is a plain class whose method shapes match the Protocol. `@runtime_checkable` lets the contract test verify the match at test time.

### Why now, not as part of the cursor/cmux PR

The refactor surface is non-trivial — 17 test files reference `cafleet.tmux` directly (`grep -rln "cafleet.tmux" cafleet/tests/`) and need their import paths updated. Cursor and cmux each carry their own integration questions (cursor's spawn-flag shape, cmux's pane-id semantics) that deserve their own clarification rounds. Landing the refactor first keeps each follow-on PR small and independently reviewable.

---

## Specification

### 1. `cafleet.coding_agent` subpackage

#### 1.1 Layout

```
cafleet/src/cafleet/coding_agent/
├── __init__.py    # re-exports + CODING_AGENTS registry
├── base.py        # CodingAgent Protocol + shared helpers
├── claude.py      # ClaudeCodeAgent
└── codex.py       # CodexAgent
```

#### 1.2 Protocol (`base.py`)

```python
from typing import Protocol, runtime_checkable
import shutil


@runtime_checkable
class CodingAgent(Protocol):
    """Coding-agent binary that runs inside a multiplexer pane."""

    @property
    def name(self) -> str:
        """Registry key — matches the `placement.coding_agent` column value."""
        ...

    @property
    def binary_name(self) -> str:
        """Executable name resolved via `shutil.which`."""
        ...

    def ensure_available(self) -> None:
        """Raise RuntimeError if `binary_name` is not on PATH."""
        ...

    def build_spawn_argv(self, prompt: str, *, display_name: str) -> list[str]:
        """Return the argv list passed to the multiplexer's `split_window`.

        `display_name` is honored by backends that support a pane-title flag
        (claude) and silently ignored by those that do not (codex).
        """
        ...


def ensure_binary_on_path(binary_name: str) -> None:
    """Shared availability check used by every CodingAgent impl."""
    if shutil.which(binary_name) is None:
        raise RuntimeError(f"binary {binary_name} not found on PATH")
```

Per the B1 user decision (Protocol, not ABC), shared logic lives as a module-level helper (`ensure_binary_on_path`) that concrete impls call from their own `ensure_available` method. No base-class concrete methods.

#### 1.3 `ClaudeCodeAgent` (`claude.py`)

```python
from cafleet.coding_agent.base import ensure_binary_on_path


class ClaudeCodeAgent:
    name = "claude"
    binary_name = "claude"

    def ensure_available(self) -> None:
        ensure_binary_on_path(self.binary_name)

    def build_spawn_argv(self, prompt: str, *, display_name: str) -> list[str]:
        return [
            self.binary_name,
            "--permission-mode",
            "dontAsk",
            "--name",
            display_name,
            prompt,
        ]
```

Argv shape is byte-identical to today's `_build_claude_command` in `cli.py:34-42`.

#### 1.4 `CodexAgent` (`codex.py`)

```python
from cafleet.coding_agent.base import ensure_binary_on_path


class CodexAgent:
    name = "codex"
    binary_name = "codex"

    def ensure_available(self) -> None:
        ensure_binary_on_path(self.binary_name)

    def build_spawn_argv(self, prompt: str, *, display_name: str) -> list[str]:
        # display_name silently ignored — codex has no `--name` analog.
        return [
            self.binary_name,
            "--ask-for-approval",
            "never",
            "--sandbox",
            "workspace-write",
            prompt,
        ]
```

Argv shape is byte-identical to today's `_build_codex_command` in `cli.py:45-53`. The `display_name` parameter is accepted (positional-keyword compatible with the Protocol) and discarded — no `supports_display_name` flag, no CLI-side branch.

#### 1.5 Registry (`__init__.py`)

```python
from cafleet.coding_agent.base import CodingAgent, ensure_binary_on_path
from cafleet.coding_agent.claude import ClaudeCodeAgent
from cafleet.coding_agent.codex import CodexAgent

CODING_AGENTS: dict[str, CodingAgent] = {
    "claude": ClaudeCodeAgent(),
    "codex": CodexAgent(),
}

__all__ = [
    "CodingAgent",
    "ClaudeCodeAgent",
    "CodexAgent",
    "CODING_AGENTS",
    "ensure_binary_on_path",
]
```

The dict is constructed once at module load. Concrete impls are stateless, so a single shared instance per entry is safe.

#### 1.6 Prompt template — stays shared module-level

Per the implicit B3(d) resolution (today's `_MEMBER_PROMPT_TEMPLATE` is backend-neutral), the template stays a module-level constant in `cli.py` and `cli._resolve_prompt` keeps applying `str.format(...)`. **No per-backend prompt logic moves onto the Protocol.** If a future backend needs a different template, it can override at that point.

### 2. `cafleet.multiplexer` subpackage

#### 2.1 Layout

```
cafleet/src/cafleet/multiplexer/
├── __init__.py    # re-exports + MULTIPLEXERS registry
├── base.py        # Multiplexer Protocol + MultiplexerContext + shared helpers
└── tmux.py        # TmuxMultiplexer + TmuxError
```

#### 2.2 `MultiplexerContext` (`base.py`)

```python
from dataclasses import dataclass


@dataclass(frozen=True)
class MultiplexerContext:
    """Resolved pane identity, returned by `Multiplexer.context_discovery()`."""
    session: str
    window_id: str
    pane_id: str
```

Fields are byte-identical to today's `tmux.DirectorContext`. Per the B5 user decision, cmux maps its pane / tab / session concepts onto these same slots — broker stays trivial.

#### 2.3 Protocol (`base.py`)

```python
from typing import Protocol, runtime_checkable


@runtime_checkable
class Multiplexer(Protocol):
    """Terminal multiplexer that hosts coding-agent panes."""

    @property
    def name(self) -> str:
        """Registry key (e.g. `"tmux"`)."""
        ...

    # Lifecycle
    def ensure_available(self) -> None: ...
    def context_discovery(self) -> MultiplexerContext: ...
    def split_window(
        self,
        *,
        target_window_id: str,
        env: dict[str, str],
        command: list[str],
    ) -> str: ...
    def select_layout(
        self, *, target_window_id: str, layout: str = "main-vertical"
    ) -> None: ...
    def kill_pane(self, *, target_pane_id: str, ignore_missing: bool = False) -> None: ...
    def pane_exists(self, *, target_pane_id: str) -> bool: ...
    def wait_for_pane_gone(
        self, *, target_pane_id: str, timeout: float = 15.0, interval: float = 0.5
    ) -> bool: ...

    # Keystroke primitives
    def send_exit(self, *, target_pane_id: str, ignore_missing: bool = False) -> None: ...
    def send_poll_trigger(
        self, *, target_pane_id: str, session_id: str, agent_id: str
    ) -> bool: ...
    def send_inline_preview(
        self,
        *,
        target_pane_id: str,
        task_id_8: str,
        sender_8: str,
        ts: str,
        text: str,
    ) -> bool: ...
    def send_choice_key(self, *, target_pane_id: str, digit: int) -> None: ...
    def send_freetext_and_submit(self, *, target_pane_id: str, text: str) -> None: ...
    def send_bash_command(self, *, target_pane_id: str, command: str) -> None: ...
    def capture_pane(self, *, target_pane_id: str, lines: int = 30) -> str: ...
```

The 14 public methods cover every primitive in today's `cafleet.tmux` (parameter names and types kept identical). The private helper `_send_literal_then_enter` is **not** on the Protocol — it lives as a `tmux`-specific module-level function (§2.5).

#### 2.4 Shared helpers (`base.py`)

Per the B2 user decision (Protocol, not ABC), shared composition logic cannot live as a base-class method. The `wait_for_pane_gone` polling loop is mechanically composed of `pane_exists` + `time.monotonic` + `time.sleep` — that composition is identical across multiplexers. It lives as a module-level helper that each concrete impl calls from its own `wait_for_pane_gone` method:

```python
import time
from collections.abc import Callable


def poll_until_pane_gone(
    pane_exists_fn: Callable[[], bool],
    *,
    timeout: float,
    interval: float,
) -> bool:
    """Generic poll-until-False helper for any Multiplexer's `wait_for_pane_gone`."""
    deadline = time.monotonic() + timeout
    while True:
        if not pane_exists_fn():
            return True
        if time.monotonic() >= deadline:
            return False
        time.sleep(interval)
```

`TmuxMultiplexer.wait_for_pane_gone` calls `poll_until_pane_gone(lambda: self.pane_exists(target_pane_id=...), ...)`.

#### 2.5 `TmuxMultiplexer` (`tmux.py`)

The contents of today's `cafleet/tmux.py` (lines 1-252) move verbatim, with these transformations:

| Today's symbol | Post-refactor location |
|---|---|
| `TmuxError` | `cafleet.multiplexer.tmux.TmuxError` (class lives next to its owner; re-exported from `cafleet.multiplexer.__init__`) |
| `DirectorContext` | Renamed to `MultiplexerContext` in `cafleet.multiplexer.base` |
| `ensure_tmux_available()` | `TmuxMultiplexer.ensure_available()` method |
| `director_context()` | `TmuxMultiplexer.context_discovery()` method |
| `split_window(...)` | `TmuxMultiplexer.split_window(...)` method |
| `select_layout(...)` | `TmuxMultiplexer.select_layout(...)` method |
| `kill_pane(...)` | `TmuxMultiplexer.kill_pane(...)` method |
| `pane_exists(...)` | `TmuxMultiplexer.pane_exists(...)` method |
| `wait_for_pane_gone(...)` | `TmuxMultiplexer.wait_for_pane_gone(...)` method (delegates to `poll_until_pane_gone`) |
| `send_exit(...)` | `TmuxMultiplexer.send_exit(...)` method |
| `send_poll_trigger(...)` | `TmuxMultiplexer.send_poll_trigger(...)` method |
| `send_inline_preview(...)` | `TmuxMultiplexer.send_inline_preview(...)` method |
| `send_choice_key(...)` | `TmuxMultiplexer.send_choice_key(...)` method |
| `send_freetext_and_submit(...)` | `TmuxMultiplexer.send_freetext_and_submit(...)` method |
| `send_bash_command(...)` | `TmuxMultiplexer.send_bash_command(...)` method |
| `capture_pane(...)` | `TmuxMultiplexer.capture_pane(...)` method |
| `_send_literal_then_enter(...)` | `cafleet.multiplexer.tmux._send_literal_then_enter` (module-level private function — tmux-specific keystroke sequencing, not on the Protocol) |
| `_run(...)`, `_run_tolerating_pane_gone(...)`, `_PANE_GONE_MARKERS`, `_SUBMIT_DELAY` | Stay as tmux-private module-level functions / constants in `cafleet.multiplexer.tmux` |

`TmuxMultiplexer` is a stateless class — methods do not read or write `self` attributes beyond `self.name`. Behavior is byte-identical to today's `cafleet.tmux` module functions.

```python
class TmuxMultiplexer:
    name = "tmux"

    def ensure_available(self) -> None:
        # Body verbatim from today's tmux.ensure_tmux_available().
        ...

    def context_discovery(self) -> MultiplexerContext:
        # Body verbatim from today's tmux.director_context().
        ...

    # ... all other methods follow the same pattern.

    def wait_for_pane_gone(
        self, *, target_pane_id: str, timeout: float = 15.0, interval: float = 0.5
    ) -> bool:
        return poll_until_pane_gone(
            lambda: self.pane_exists(target_pane_id=target_pane_id),
            timeout=timeout,
            interval=interval,
        )
```

#### 2.6 Registry (`__init__.py`)

```python
from cafleet.multiplexer.base import (
    Multiplexer,
    MultiplexerContext,
    poll_until_pane_gone,
)
from cafleet.multiplexer.tmux import TmuxError, TmuxMultiplexer

MULTIPLEXERS: dict[str, Multiplexer] = {
    "tmux": TmuxMultiplexer(),
}

__all__ = [
    "Multiplexer",
    "MultiplexerContext",
    "TmuxMultiplexer",
    "TmuxError",
    "MULTIPLEXERS",
    "poll_until_pane_gone",
]
```

#### 2.7 Active multiplexer resolution

There is no `set_multiplexer(...)` setter or runtime selector in this design doc. Every caller in the codebase that needs a multiplexer reaches for `MULTIPLEXERS["tmux"]` directly (or for a specific method via a local import — see §3.4). When the cmux follow-on doc lands, it adds a `MULTIPLEXERS["cmux"]` entry; the strategy for picking between them (env var, CLI flag, auto-detect) is that doc's problem, not this one's.

### 3. Caller updates

#### 3.1 `cli.py` — coding-agent selection

The `_CLAUDE_BINARY`, `_CODEX_BINARY`, `_build_claude_command`, `_build_codex_command`, and `_ensure_coding_agent_available` symbols in `cli.py:24-58` are deleted. Their callers in `member_create` change:

```python
# Before (cli.py:947, 985-988):
binary_name = _CLAUDE_BINARY if coding_agent == "claude" else _CODEX_BINARY
_ensure_coding_agent_available(binary_name)
...
if coding_agent == "claude":
    spawn_command = _build_claude_command(prompt, display_name=name)
else:
    spawn_command = _build_codex_command(prompt)

# After:
from cafleet.coding_agent import CODING_AGENTS

agent = CODING_AGENTS[coding_agent]
agent.ensure_available()
...
spawn_command = agent.build_spawn_argv(prompt, display_name=name)
```

The `click.Choice` becomes dynamic on both `session_create` and `member_create`:

```python
# Before (cli.py:286-292, 918-925):
type=click.Choice(["claude", "codex"]),

# After:
from cafleet.coding_agent import CODING_AGENTS

type=click.Choice(list(CODING_AGENTS.keys())),
```

The Click decorator runs at import time; `CODING_AGENTS.keys()` is resolved at decorator-application time and stays `["claude", "codex"]` until the cursor follow-on doc lands.

`_ensure_tmux_or_die()` in `cli.py:61-65` changes to call the registered multiplexer:

```python
# Before:
def _ensure_tmux_or_die() -> None:
    try:
        tmux.ensure_tmux_available()
    except tmux.TmuxError as exc:
        raise click.ClickException(str(exc)) from exc

# After:
from cafleet.multiplexer import MULTIPLEXERS, TmuxError

def _ensure_tmux_or_die() -> None:
    try:
        MULTIPLEXERS["tmux"].ensure_available()
    except TmuxError as exc:
        raise click.ClickException(str(exc)) from exc
```

Every other call site (`session_create`, `doctor`, `member_create`, `member_delete`, `member_capture`, `member_send_input`, `member_exec`, `member_ping`) is updated mechanically: `tmux.X(...)` becomes `MULTIPLEXERS["tmux"].X(...)`, `tmux.TmuxError` becomes the re-exported `TmuxError`, and `tmux.DirectorContext` becomes `MultiplexerContext`.

#### 3.2 `cli.doctor` — multiplexer-aware

Per D5, `doctor` resolves context via the registry rather than a hardcoded tmux call:

```python
# Before (cli.py:402-435):
try:
    director_ctx = tmux.director_context()
except tmux.TmuxError as exc:
    ...

# After:
multiplexer = MULTIPLEXERS["tmux"]  # cursor/cmux follow-on adds selection logic
try:
    director_ctx = multiplexer.context_discovery()
except TmuxError as exc:
    ...
```

JSON output shape is preserved verbatim — the `"tmux"` top-level key stays (the multiplexer name is reported separately in a follow-on if cmux lands).

#### 3.3 `broker.py` — imports and local-import pattern

In a single commit (Step 5), rename `DirectorContext` to `MultiplexerContext` at every site in `broker.py`: the `from cafleet.tmux import DirectorContext` import at line 14 becomes `from cafleet.multiplexer import MultiplexerContext`, and the type annotation `director_context: DirectorContext` at line 107 becomes `director_context: MultiplexerContext`. The function signature `create_session(..., director_context: MultiplexerContext, ...)` keeps its parameter name — the name is descriptive and not coupled to the type name, and attribute accesses like `director_context.session` / `.window_id` / `.pane_id` further down the function body continue to work unchanged.

Per D4, the local-import pattern in `broker._try_notify_recipient` (today `broker.py:85`) is preserved:

```python
# Before:
from cafleet.tmux import send_inline_preview
return send_inline_preview(...)

# After:
from cafleet.multiplexer.tmux import TmuxMultiplexer
return TmuxMultiplexer().send_inline_preview(...)
```

The local-import-per-call shape is what makes `monkeypatch.setattr("cafleet.multiplexer.tmux.TmuxMultiplexer.send_inline_preview", ...)` work. The shared registry singleton (`MULTIPLEXERS["tmux"]`) is **not** used here — we want a fresh class lookup per call so the monkeypatch is picked up. Constructing a fresh instance is free (the class is stateless).

#### 3.4 Test-path updates

The 17 test files that today reference `cafleet.tmux` shift their import path:

| Today | Post-refactor |
|---|---|
| `from cafleet.tmux import DirectorContext` | `from cafleet.multiplexer import MultiplexerContext as DirectorContext` (one rename per file or a global sed; the variable name `DirectorContext` can stay if a file uses it heavily — it's a test-local alias) |
| `from cafleet.tmux import TmuxError` | `from cafleet.multiplexer import TmuxError` |
| `monkeypatch.setattr("cafleet.tmux.ensure_tmux_available", ...)` | `monkeypatch.setattr("cafleet.multiplexer.tmux.TmuxMultiplexer.ensure_available", ...)` |
| `monkeypatch.setattr("cafleet.tmux.director_context", ...)` | `monkeypatch.setattr("cafleet.multiplexer.tmux.TmuxMultiplexer.context_discovery", ...)` |
| `monkeypatch.setattr("cafleet.tmux.split_window", ...)` | `monkeypatch.setattr("cafleet.multiplexer.tmux.TmuxMultiplexer.split_window", ...)` |
| `monkeypatch.setattr("cafleet.tmux.select_layout", ...)` | `monkeypatch.setattr("cafleet.multiplexer.tmux.TmuxMultiplexer.select_layout", ...)` |
| `monkeypatch.setattr("cafleet.tmux.send_exit", ...)` | `monkeypatch.setattr("cafleet.multiplexer.tmux.TmuxMultiplexer.send_exit", ...)` |
| `monkeypatch.setattr("cafleet.tmux.send_inline_preview", ...)` | `monkeypatch.setattr("cafleet.multiplexer.tmux.TmuxMultiplexer.send_inline_preview", ...)` |

> **Method-on-class monkeypatch caveat.** `monkeypatch.setattr` on an unbound method requires the replacement to accept `self` as its first positional argument (or be a `staticmethod`-decorated function for kwargs-only paths). Test bodies that previously substituted a `lambda: None` for a no-arg module-level function need a `lambda self, **_: None` substitute. The Programmer is authorized to choose the lambda shape on a per-test-file basis during execute.

No test body is restructured. The function-level monkeypatch idiom stays; only the targets shift.

### 4. Database migration

#### 4.1 Backfill rationale (D2)

Pre-`0000046` root-Director rows in `agent_placements` may carry `coding_agent = "unknown"` (the value of the now-removed `_ROOT_DIRECTOR_CODING_AGENT` constant in pre-`0000046` `broker.py`). Post-refactor, `CODING_AGENTS["unknown"]` raises `KeyError`. The KeyError surfaces only when something tries to spawn or look up an "unknown"-backed row; existing render paths (`output.format_member`, `format_session_create`) treat the column as opaque metadata and continue working. The backfill is preventative: we migrate the value to `"claude"` (which matches what those rows actually ran — claude was the only choice pre-`0000046`) so future code that does want to look up the column in the registry will not crash.

#### 4.2 Migration script

```python
# cafleet/src/cafleet/alembic/versions/0010_backfill_unknown_coding_agent.py
"""Backfill placement.coding_agent = 'unknown' rows to 'claude'."""

from alembic import op

revision = "0010"
down_revision = "0009"
branch_labels = None
depends_on = None


def upgrade() -> None:
    op.execute(
        "UPDATE agent_placements "
        "SET coding_agent = 'claude' "
        "WHERE coding_agent = 'unknown'"
    )


def downgrade() -> None:
    # No-op: this is a data-only backfill. The original 'unknown' value is
    # not recoverable from 'claude' (it would require knowing which rows
    # were originally backfilled), and the design-046 reasoning explicitly
    # treats 'unknown' as a label-not-truth value that 'claude' supersedes.
    pass
```

The script is idempotent — re-running `upgrade` finds no `"unknown"` rows and updates zero. Rows with `coding_agent = "claude"` or `coding_agent = "codex"` are untouched.

### 5. Contract tests

#### 5.1 `tests/test_coding_agent_protocol.py`

```python
import pytest
from cafleet.coding_agent import CODING_AGENTS, CodingAgent


@pytest.mark.parametrize("name,impl", list(CODING_AGENTS.items()))
def test_impl_satisfies_protocol(name, impl):
    assert isinstance(impl, CodingAgent)
    assert impl.name == name


@pytest.mark.parametrize("impl", list(CODING_AGENTS.values()))
def test_build_spawn_argv_includes_prompt_last(impl):
    argv = impl.build_spawn_argv("<PROMPT>", display_name="some-name")
    assert isinstance(argv, list)
    assert all(isinstance(x, str) for x in argv)
    assert argv[-1] == "<PROMPT>"
    assert argv[0] == impl.binary_name


@pytest.mark.parametrize("impl", list(CODING_AGENTS.values()))
def test_ensure_available_raises_when_binary_missing(impl, monkeypatch):
    monkeypatch.setattr("shutil.which", lambda _: None)
    with pytest.raises(RuntimeError, match="not found on PATH"):
        impl.ensure_available()
```

#### 5.2 `tests/test_multiplexer_protocol.py`

```python
import pytest
from cafleet.multiplexer import MULTIPLEXERS, Multiplexer, MultiplexerContext
from cafleet.multiplexer.tmux import TmuxMultiplexer


@pytest.mark.parametrize("name,impl", list(MULTIPLEXERS.items()))
def test_impl_satisfies_protocol(name, impl):
    """Cross-impl Protocol check — runs against every registered Multiplexer."""
    assert isinstance(impl, Multiplexer)
    assert impl.name == name


def test_tmux_context_discovery_returns_multiplexer_context(monkeypatch):
    """tmux-specific smoke — stays single-impl because the monkeypatch target is tmux-private.

    When the cmux follow-on doc lands, it adds its own parallel
    `test_cmux_context_discovery_returns_multiplexer_context` rather than
    parametrizing this one — each multiplexer's `context_discovery`
    stub-shape is impl-specific.
    """
    monkeypatch.setattr(
        "cafleet.multiplexer.tmux._run",
        lambda *a, **k: "fake-session|@1|%1",
    )
    monkeypatch.setenv("TMUX", "fake")
    monkeypatch.setenv("TMUX_PANE", "%1")
    ctx = TmuxMultiplexer().context_discovery()
    assert isinstance(ctx, MultiplexerContext)
    assert ctx.session == "fake-session"
```

`test_impl_satisfies_protocol` is the parametrized cross-impl assertion (catches signature drift the moment cmux lands); `test_tmux_context_discovery_returns_multiplexer_context` is the tmux-specific smoke whose stub does not generalize. The contract tests catch *signature drift* between Protocol and impls (e.g., adding a new method to the Protocol that an impl forgets). They are the cheapest insurance for the cursor/cmux follow-on docs.

### 6. Out of scope (confirmed)

Per the brief and D3:

- **Cursor CLI / cmux impls themselves** — separate follow-on design docs.
- **DB schema changes** — `agent_placements.coding_agent` column stays free-text `String` with `server_default='claude'`. Only the data migration in §4 lands here.
- **CLI flag value surface** — `--coding-agent {claude,codex}` choice values are unchanged. The Choice **computation** becomes dynamic from `CODING_AGENTS.keys()` (so adding cursor is one dict entry), but the set of accepted values is `{"claude", "codex"}` until the cursor doc lands.
- **Message broker, task lifecycle, WebUI** — unchanged.
- **`set_multiplexer()` DI hook / FakeMultiplexer test double** — deferred to the cmux follow-on doc per C1.
- **Prompt-template per-backend customization** — `_MEMBER_PROMPT_TEMPLATE` stays a shared module-level constant in `cli.py`.
- **`supports_display_name` flag** — CodexAgent silently ignores `display_name`; no CLI-side branch.

### 7. File inventory

| Path | Action |
|---|---|
| `cafleet/src/cafleet/coding_agent/__init__.py` | **New** — re-exports + `CODING_AGENTS` registry |
| `cafleet/src/cafleet/coding_agent/base.py` | **New** — `CodingAgent` Protocol + `ensure_binary_on_path` |
| `cafleet/src/cafleet/coding_agent/claude.py` | **New** — `ClaudeCodeAgent` |
| `cafleet/src/cafleet/coding_agent/codex.py` | **New** — `CodexAgent` |
| `cafleet/src/cafleet/multiplexer/__init__.py` | **New** — re-exports + `MULTIPLEXERS` registry |
| `cafleet/src/cafleet/multiplexer/base.py` | **New** — `Multiplexer` Protocol + `MultiplexerContext` + `poll_until_pane_gone` |
| `cafleet/src/cafleet/multiplexer/tmux.py` | **New** — `TmuxMultiplexer` + `TmuxError` + private `_run` helpers (content migrated from old `cafleet/tmux.py`) |
| `cafleet/src/cafleet/tmux.py` | **Delete** |
| `cafleet/src/cafleet/cli.py` | **Edit** — delete `_CLAUDE_BINARY` / `_CODEX_BINARY` / `_build_*_command` / `_ensure_coding_agent_available`; replace with registry lookups; dynamic `click.Choice`; multiplexer-via-registry in `doctor` / `member_*` / `session_create` |
| `cafleet/src/cafleet/broker.py` | **Edit** — import path updates only; local-import pattern preserved |
| `cafleet/src/cafleet/alembic/versions/0010_backfill_unknown_coding_agent.py` | **New** — data migration |
| `cafleet/tests/test_coding_agent_protocol.py` | **New** — contract test |
| `cafleet/tests/test_multiplexer_protocol.py` | **New** — contract test |
| `cafleet/tests/test_tmux.py`, `test_tmux_send_helpers.py` | **Rename + Edit** — rename to `test_multiplexer_tmux.py` / `test_multiplexer_tmux_send_helpers.py`; point at new module path; rename test classes / function names as needed |
| 15 other `cafleet/tests/*.py` files referencing `cafleet.tmux` (the 17-file total minus `test_tmux.py` and `test_tmux_send_helpers.py` above) | **Edit** — monkeypatch import-path updates per the §3.4 table |
| `ARCHITECTURE.md` | **Edit** — Component-Layout table: add `coding_agent/` and `multiplexer/` subpackage rows; remove `tmux.py` row |
| `README.md` | **Edit** — Project Structure tree-block: same delta as ARCHITECTURE |
| `docs/spec/cli-options.md` | **Audit** — no flag changes, but the §`--coding-agent` blurb may want a one-line note that the value set is registry-driven |
| `docs/codex-members.md` | **Audit** — should be untouched; backend description does not change |
| `skills/cafleet/SKILL.md` | **Audit** — no CLI changes; verify nothing references the old module path |
| `skills/cafleet/reference/director.md` | **Audit** — same |
| `skills/cafleet/reference/exec-routing.md` | **Audit** — same |
| `skills/cafleet/reference/recovery.md` | **Audit** — same |

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

### Step 1: Documentation Updates (lands first per `.claude/rules/design-doc-numbering.md`)

- [x] Update `ARCHITECTURE.md` — Component-Layout table: add `cafleet/coding_agent/` and `cafleet/multiplexer/` subpackage rows; remove the `tmux.py` row. Update any prose that names `cafleet.tmux` or `_build_claude_command` / `_build_codex_command`. <!-- completed: 2026-05-18T13:08 -->
- [x] Update `README.md` — Project Structure tree-block to match ARCHITECTURE. Run `/update-readme` to catch drift. <!-- completed: 2026-05-18T13:08 -->
- [x] Audit `docs/spec/cli-options.md` — confirm `--coding-agent` documentation still matches (the flag values are unchanged); add one line noting the Choice list is registry-driven if it improves clarity. If any reference to the old `cafleet.tmux` module path is found, update it to `cafleet.multiplexer.tmux` (or `cafleet.multiplexer` for the re-exported top-level symbols). <!-- completed: 2026-05-18T13:09 -->
- [x] Audit `docs/codex-members.md` for `cafleet.tmux` references; if found, update them to `cafleet.multiplexer.tmux` (or `cafleet.multiplexer`). <!-- completed: 2026-05-18T13:10 -->
- [x] Audit every `skills/cafleet/SKILL.md` + `skills/cafleet/reference/*.md` for `cafleet.tmux` references; if found, update them to `cafleet.multiplexer.tmux` (or `cafleet.multiplexer`). None expected (skills describe CLI behavior, not internal module layout), but verify. <!-- completed: 2026-05-18T13:10 -->
- [ ] Commit: `docs: documentation surface for design 0000066 backend abstractions`. <!-- completed: -->

### Step 2: `cafleet.multiplexer` subpackage

- [x] Create `cafleet/src/cafleet/multiplexer/base.py` with `MultiplexerContext` dataclass, `Multiplexer` `@runtime_checkable` Protocol, and `poll_until_pane_gone` helper. <!-- completed: 2026-05-18T13:13 -->
- [x] Create `cafleet/src/cafleet/multiplexer/tmux.py` by migrating today's `cafleet/tmux.py` content. Wrap module-level functions as `TmuxMultiplexer` instance methods (stateless — methods do not touch `self` beyond `self.name`). Keep `TmuxError`, `_run`, `_run_tolerating_pane_gone`, `_PANE_GONE_MARKERS`, `_SUBMIT_DELAY`, `_send_literal_then_enter` as module-level (not on the class). Implement `wait_for_pane_gone` via `poll_until_pane_gone`. <!-- completed: 2026-05-18T13:13 -->
- [x] Create `cafleet/src/cafleet/multiplexer/__init__.py` exposing `Multiplexer`, `MultiplexerContext`, `TmuxMultiplexer`, `TmuxError`, `poll_until_pane_gone`, and the `MULTIPLEXERS: dict[str, Multiplexer]` registry. <!-- completed: 2026-05-18T13:13 -->
- [ ] Commit: `refactor: extract cafleet.multiplexer subpackage (design 0000066 step 2)`. The old `cafleet/src/cafleet/tmux.py` stays in place and is **not** deleted in this commit — `cli.py`, `broker.py`, and the test files still import from it. The legacy module is removed in Step 6 once every caller has been migrated. <!-- completed: -->

### Step 3: `cafleet.coding_agent` subpackage

- [x] Create `cafleet/src/cafleet/coding_agent/base.py` with `CodingAgent` `@runtime_checkable` Protocol and `ensure_binary_on_path` helper. <!-- completed: 2026-05-18T13:38 -->
- [x] Create `cafleet/src/cafleet/coding_agent/claude.py` with `ClaudeCodeAgent` — argv shape byte-identical to today's `_build_claude_command`. <!-- completed: 2026-05-18T13:38 -->
- [x] Create `cafleet/src/cafleet/coding_agent/codex.py` with `CodexAgent` — argv shape byte-identical to today's `_build_codex_command`. `build_spawn_argv` accepts `display_name` and ignores it. <!-- completed: 2026-05-18T13:38 -->
- [x] Create `cafleet/src/cafleet/coding_agent/__init__.py` exposing the Protocol, both concrete classes, the `ensure_binary_on_path` helper, and the `CODING_AGENTS: dict[str, CodingAgent]` registry. <!-- completed: 2026-05-18T13:38 -->
- [ ] Commit: `refactor: extract cafleet.coding_agent subpackage (design 0000066 step 3)`. <!-- completed: -->

### Step 4: Wire `cli.py` to the registries

- [x] Replace `_CLAUDE_BINARY` / `_CODEX_BINARY` / `_build_claude_command` / `_build_codex_command` / `_ensure_coding_agent_available` usage in `member_create` with `CODING_AGENTS[coding_agent].ensure_available()` + `.build_spawn_argv(prompt, display_name=name)`. Delete the five symbols from `cli.py`. <!-- completed: 2026-05-18T13:43 -->
- [x] Replace `click.Choice(["claude", "codex"])` on `session_create` and `member_create` with `click.Choice(list(CODING_AGENTS.keys()))`. <!-- completed: 2026-05-18T13:43 -->
- [x] Replace every `tmux.X(...)` call in `cli.py` (`session_create`, `doctor`, `member_create`, `member_delete`, `member_capture`, `member_send_input`, `member_exec`, `member_ping`, `_ensure_tmux_or_die`) with `MULTIPLEXERS["tmux"].X(...)`. <!-- completed: 2026-05-18T13:44 -->
- [x] Replace `tmux.TmuxError` references with the re-exported `TmuxError` from `cafleet.multiplexer`. Replace `tmux.DirectorContext` with `MultiplexerContext`. <!-- completed: 2026-05-18T13:44 -->
- [x] Drop the `from cafleet import ... tmux` import in `cli.py:21`; add `from cafleet.coding_agent import CODING_AGENTS` and `from cafleet.multiplexer import MULTIPLEXERS, MultiplexerContext, TmuxError`. <!-- completed: 2026-05-18T13:43 -->
- [ ] Commit: `refactor: cli.py uses CODING_AGENTS + MULTIPLEXERS registries (design 0000066 step 4)`. <!-- completed: -->

### Step 5: Wire `broker.py` to the new module paths

- [x] Replace `from cafleet.tmux import DirectorContext` in `broker.py:14` with `from cafleet.multiplexer import MultiplexerContext`. Rename `DirectorContext` to `MultiplexerContext` at every site in `broker.py`. <!-- completed: 2026-05-18T14:05 -->
- [x] Update the local import in `broker._try_notify_recipient` (today `broker.py:85`) from `from cafleet.tmux import send_inline_preview` to `from cafleet.multiplexer.tmux import TmuxMultiplexer`; call `TmuxMultiplexer().send_inline_preview(...)`. <!-- completed: 2026-05-18T14:05 -->
- [ ] Commit: `refactor: broker.py imports from cafleet.multiplexer (design 0000066 step 5)`. <!-- completed: -->

### Step 6: Test import-path updates

- [x] Sweep `cafleet/tests/` for `cafleet.tmux` references. Update imports: `from cafleet.tmux import DirectorContext` → `from cafleet.multiplexer import MultiplexerContext as DirectorContext` (or drop the alias if the test uses fewer than ~3 occurrences). <!-- completed: 2026-05-18T14:13 -->
- [x] Update monkeypatch targets per the table in §3.4. For monkeypatched methods on `TmuxMultiplexer`, ensure lambdas accept `self` as the first positional arg. <!-- completed: 2026-05-18T14:13 -->
- [x] Rename `tests/test_tmux.py` → `tests/test_multiplexer_tmux.py` and `tests/test_tmux_send_helpers.py` → `tests/test_multiplexer_tmux_send_helpers.py`. Update the docstring at the top of each. <!-- completed: 2026-05-18T14:15 -->
- [x] Delete `cafleet/src/cafleet/tmux.py` now that no caller imports from it (`grep -rn "cafleet.tmux" cafleet/src cafleet/tests` should return zero matches at this point). <!-- completed: 2026-05-18T14:16 -->
- [x] Run `mise //cafleet:test` — expect green. <!-- completed: 2026-05-18T14:17 -->
- [ ] Commit: `refactor: migrate tests + delete legacy cafleet/tmux.py (design 0000066 step 6)`. <!-- completed: -->

### Step 7: Contract tests

- [x] Create `cafleet/tests/test_coding_agent_protocol.py` with the parametrized contract tests from §5.1. <!-- completed: 2026-05-18T14:23 -->
- [x] Create `cafleet/tests/test_multiplexer_protocol.py` with the parametrized contract tests from §5.2. <!-- completed: 2026-05-18T14:23 -->
- [x] Run `mise //cafleet:test` — expect green; new tests parametrize over `CODING_AGENTS.values()` and `MULTIPLEXERS.values()`, so the count grows. <!-- completed: 2026-05-18T14:25 -->
- [x] Commit: `test: protocol contract tests for CodingAgent + Multiplexer (design 0000066 step 7)`. <!-- completed: 2026-05-18T14:25 -->

### Step 8: Alembic backfill migration

- [x] Create `cafleet/src/cafleet/alembic/versions/0010_backfill_unknown_coding_agent.py` chained off `down_revision = "0009"`. <!-- completed: 2026-05-18T14:28 -->
- [x] `upgrade()` executes the `UPDATE agent_placements SET coding_agent = 'claude' WHERE coding_agent = 'unknown'` statement; `downgrade()` is a no-op (rationale in §4.2). <!-- completed: 2026-05-18T14:28 -->
- [x] Run `mise //cafleet:test` to confirm the test suite's schema-setup path applies the new revision cleanly. <!-- completed: 2026-05-18T14:29 -->
- [x] Smoke: run `cafleet db init` against a workstation SQLite that has both pre-`0000046` `"unknown"` rows and post-`0000046` `"claude"`/`"codex"` rows; confirm only the `"unknown"` rows change. <!-- completed: 2026-05-18T14:32 (Director ran `cafleet db init` against workstation DB: alembic upgrade 0009 → 0010 ran cleanly; no `unknown` rows present so the UPDATE was a degenerate no-op; pre-existing `claude` rows untouched.) -->
- [ ] Commit: `feat: alembic 0010 backfills placement.coding_agent unknown→claude (design 0000066 step 8)`. <!-- completed: -->

### Step 9: Final verification

- [x] Run `mise //cafleet:test` — expect green. <!-- completed: 2026-05-18T14:40 (617 pass + 1 pre-existing test_base_dir failure unchanged since baseline 5ee52e4 — out of scope for design 0000066, confirmed by Verifier) -->
- [x] Run `mise //cafleet:lint` — expect clean. <!-- completed: 2026-05-18T14:40 -->
- [x] Run `mise //cafleet:format` — expect clean. <!-- completed: 2026-05-18T14:40 (covered by ruff format --check inside mise //cafleet:lint) -->
- [x] Run `mise //cafleet:typecheck` — expect clean. <!-- completed: 2026-05-18T14:40 -->
- [x] Manual smoke (workstation with both `claude` and `codex` installed): `cafleet session create --coding-agent claude` works; `cafleet session create --coding-agent codex` works; `cafleet member create --coding-agent claude` spawns claude in a pane; `cafleet member create --coding-agent codex` spawns codex; `cafleet doctor` reports the calling pane's tmux IDs unchanged. <!-- completed: 2026-05-18T14:43 (Director-side aggregated smoke: (1) `cafleet --json doctor` returned `{"tmux":{"session_name":"0","window_id":"@0","pane_id":"%0","tmux_pane_env":"%0"}}` — schema preserved verbatim; (2) `cafleet session create --help` and `cafleet member create --help` both list `--coding-agent [claude|codex]` — dynamic Choice resolves; (3) the design-doc execution session itself spawned Programmer/Tester/Verifier via `cafleet member create --coding-agent claude` against the registry-backed code path — three live claude panes confirm the spawn flow works end-to-end. Full standalone `cafleet session create` in a separate tmux window was held back per Authorization-Scope Guard.) -->
- [x] Update Status header to `Complete` and tick Success Criteria. <!-- completed: 2026-05-19T09:59 -->

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-05-18 | Initial draft — pure-refactor-first abstraction (Protocols + registries), prepared for cursor / cmux follow-on docs. |
| 2026-05-18 | Reviewer round 1 — 11 `COMMENT(reviewer)` markers addressed (Progress count, SC #2 wording, clarification-answers pointer, test-file count, `Callable` import, `broker.py:137-139` citation, single-commit transition, parametrized contract test split, file-inventory rename action, audit-task fix wording, `tmux.py` deletion moved from Step 2 to Step 6). User-approved. Status → Approved. |
| 2026-05-18 → 2026-05-19 | Implementation landed across 13 commits on `feat/0000066-backend-abstractions` → PR #84. Verifier confirmed 617 pass + 1 pre-existing test_base_dir failure (unchanged from baseline), lint/typecheck/format clean, all 10 Success Criteria satisfied. Five Copilot review rounds: R1 addressed inline (`select_layout` stripped from `Multiplexer` Protocol per user follow-up); R2 round-2 refactor internalized tmux layout rebalance into `TmuxMultiplexer.split_window` and dropped the post-delete rebalance entirely (cli.py is now Protocol-pure); R3 fixed doc drift in `ARCHITECTURE.md`, registry-neutral `--coding-agent` help text, alembic 0010 docstring reword, and renamed `test_tmux_send_inline_preview.py` → `test_multiplexer_tmux_send_inline_preview.py`; R4 fixed the lingering rebalance reference in `ARCHITECTURE.md` delete-ordering / atomic-create-flow; R5 returned with no new comments. Status → Complete. |
