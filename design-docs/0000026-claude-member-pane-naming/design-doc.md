# Claude Member Pane Naming via `--name` at Spawn

**Status**: Draft
**Progress**: 0/15 tasks complete
**Last Updated**: 2026-04-15

## Overview

`cafleet member create` currently spawns a Claude Code instance with no display name, so the resulting tmux pane title is auto-derived from the prompt content (e.g. `⠐ Wait for cafleet poll instructions`). When several `cafleet-design-doc-create` runs spawn members concurrently, every Drafter / Reviewer pane shows a similar prompt-derived title, and operators cannot tell which pane belongs to which member at a glance. This design pipes the `--name` value already passed to `cafleet member create` into the spawned `claude` process via its `-n/--name` flag, so the tmux pane title becomes the member name (`⠐ Drafter`, `✳ Reviewer`) for the lifetime of the pane.

## Success Criteria

- [ ] `cafleet --session-id <s> member create --agent-id <d> --name "Drafter" --description "..." -- "<prompt>"` spawns claude with `--name Drafter` and the resulting tmux pane title contains the literal string `Drafter` (verified via `tmux display-message -p -t <pane> "#{pane_title}"`).
- [ ] The pane title still contains the member name after the member processes a `cafleet send` and produces a multi-step response (i.e. Claude Code's auto-derived topic does not overwrite the explicit `--name`).
- [ ] `cafleet member create --coding-agent codex` is unchanged: no `--name` flag is passed (codex does not support one — see Background).
- [ ] `CodingAgentConfig.build_command()` accepts a keyword-only `display_name: str | None = None` and is responsible for deciding whether to inject a name flag based on its own config; `cli.py` passes the member name unconditionally and stays agnostic to per-agent flag shapes.
- [ ] All four built-in `mise` checks (`mise //:lint`, `mise //:format`, `mise //:typecheck`, `mise //cafleet:test`) pass.
- [ ] `ARCHITECTURE.md`, `docs/spec/cli-options.md`, and `.claude/skills/cafleet/SKILL.md` reflect the new behaviour (pane title == member name for `claude` members).

---

## Background

### The original ask

The user observed that Claude Code accepts a `/rename` slash command at runtime to set a session display name, and asked whether spawned members could call it so that each tmux pane is identifiable.

### Why `/rename` at runtime is the wrong tool

Slash commands in Claude Code are user-input shortcuts processed by the TUI's input field. **Claude (the AI) cannot invoke them from its own output** — emitting the literal text `/rename Drafter` in a model turn does not trigger the command, it just renders as plain text. The Skill tool description explicitly excludes built-in CLI commands ("Do not use this tool for built-in CLI commands (like /help, /clear, etc.)"); `/rename` is in the same built-in category. The only way to drive `/rename` from outside the TUI is to inject it via `tmux send-keys`, which is fragile (timing-dependent, races against the TUI's startup splash and any in-progress prompt) and adds a new failure mode to `cafleet member create`.

### What `claude --name` actually does

`claude --help` documents the flag as:

```
-n, --name <name>   Set a display name for this session (shown in /resume and terminal title)
```

It is the same value `/rename` writes to at runtime, but set at process start. The display name is emitted via the standard terminal title escape sequence (OSC 0/2), which tmux captures into `#{pane_title}`. Because Claude Code keeps re-emitting the explicit name on every render, it is not overwritten by the auto-derived topic that would otherwise appear when a name is omitted.

### Verified behaviour (manual proof carried out on 2026-04-15)

A throwaway cafleet session was used to confirm the design before this doc was written:

| Step | Action | Observation |
|---|---|---|
| 1 | `cafleet member create --name "RenameTester" --description "..." -- "<prompt>"` (with `coding_agent.py` patched to inject `--name RenameTester`) | Pane spawned at `%10` |
| 2 | `tmux display-message -p -t %10 "#{pane_title}"` | `⠐ RenameTester` |
| 3 | `cafleet send --to <member> --text "Please write a haiku about cafleet ..."` (the member ran multiple `cafleet poll`/`ack`/`send` commands) | Member completed the work and replied |
| 4 | `tmux display-message -p -t %10 "#{pane_title}"` | `✳ RenameTester` (only the spinner glyph changed; the name persisted) |

The leading `⠐`/`✳` is Claude Code's own spinner / state indicator and is not part of the configured name. Operators can search by the trailing literal (`tmux list-panes -F "#{pane_id} #{pane_title}" | grep Drafter`).

### Why codex is excluded

`codex --help` (verified 2026-04-15) lists no `--name`, `-n`, `--label`, or terminal-title equivalent. A future codex release that adds one can be wired in by extending `CodingAgentConfig` (see Specification §A); this doc does not block on it.

### Current spawn flow

| File | Function | Behaviour today |
|---|---|---|
| `cafleet/src/cafleet/coding_agent.py:20-21` | `CodingAgentConfig.build_command(self, prompt) -> list[str]` | Returns `[self.binary, *self.extra_args, prompt]`. No knowledge of per-spawn metadata. |
| `cafleet/src/cafleet/cli.py:595` | `member_create` callsite | `command=coding_agent_config.build_command(prompt)` — only the resolved prompt is forwarded; the member's `name` is held in a local variable and never used after `register_agent`. |

Both lines are the only edit points required.

---

## Specification

### A. `CodingAgentConfig.build_command()` gains an optional `display_name`

The decision of *which* CLI flag (if any) carries a display name is per-agent metadata, not caller knowledge. Encode it once on the dataclass.

```python
# cafleet/src/cafleet/coding_agent.py
@dataclass(frozen=True)
class CodingAgentConfig:
    name: str
    binary: str
    extra_args: tuple[str, ...] = ()
    default_prompt_template: str = ""
    # NEW — per-agent flag template for "set the session display name", e.g.
    # ("--name",) for claude, () for codex (no equivalent today).
    display_name_args: tuple[str, ...] = ()

    def build_command(
        self, prompt: str, *, display_name: str | None = None
    ) -> list[str]:
        name_args: tuple[str, ...] = ()
        if display_name and self.display_name_args:
            name_args = (*self.display_name_args, display_name)
        return [self.binary, *self.extra_args, *name_args, prompt]
```

Concrete configs:

| Field | `CLAUDE` | `CODEX` |
|---|---|---|
| `display_name_args` | `("--name",)` | `()` |

When `display_name` is `None` or empty, `build_command()` is byte-identical to today (this preserves every existing test in `test_coding_agent.py::TestBuildCommand`).

When `display_name="Drafter"`:

| Config | Result |
|---|---|
| `CLAUDE.build_command("hello", display_name="Drafter")` | `["claude", "--name", "Drafter", "hello"]` |
| `CODEX.build_command("hello", display_name="Drafter")` | `["codex", "--approval-mode", "auto-edit", "hello"]` (unchanged — `display_name_args=()` ⇒ no-op) |

The flag is placed **before** the positional prompt. The verification run (Background §"Verified behaviour") confirms `claude --name <value> <prompt>` is the working invocation; placing options after the positional prompt is not a tested shape and is avoided.

### B. CLI wires the member name through

```python
# cafleet/src/cafleet/cli.py — sole change inside member_create
pane_id = tmux.split_window(
    target_window_id=director_ctx.window_id,
    env=fwd_env,
    command=coding_agent_config.build_command(prompt, display_name=name),
)
```

`name` is already a required CLI option (`@click.option("--name", required=True, ...)` at `cli.py:522`) and is the same string written to the `agents.name` column. Reusing it guarantees:

1. The pane title matches the member name shown in `cafleet member list`, the admin WebUI agent list, and every `cafleet send --to <id>` log line.
2. No new flag, env var, or config knob is introduced — the surface area of `cafleet member create` is unchanged.
3. The `display_name=name` call is unconditional. Codex spawns ignore it via `display_name_args=()` (Specification §A), so no per-agent branching lives in `cli.py`.

### C. Argument shape considerations

| Concern | Resolution |
|---|---|
| Member name with spaces (e.g. `--name "Code Reviewer"`) | `subprocess` receives a `list[str]`; whitespace inside an element is preserved verbatim. `tmux split-window <args...>` likewise tokenises on the explicit list. No quoting issues. |
| Member name containing `--` or other dash-prefixed strings | `claude` only treats the **first** unmatched token as the prompt; `--name <value>` is consumed as a name regardless of the value's content. Test case pins this. |
| Empty or whitespace-only name | Forbidden upstream — `register_agent` rejects empty names today via the existing `name` column constraint, and click's `required=True` catches the missing-flag case at parse time. No new validation is added in `build_command`. |
| Name longer than the terminal title can display | Cosmetic only; tmux truncates `#{pane_title}` to the pane width. No correctness impact. |

### D. Out of scope

| Item | Why deferred |
|---|---|
| Runtime `/rename` from inside the spawned Claude (e.g. via `tmux send-keys`) | Spawn-time `--name` already covers the use case; runtime injection adds startup-race complexity for no additional benefit. |
| Codex display-name support | Codex CLI does not expose a name flag today (verified). When it does, set `CODEX.display_name_args = (<flag>,)` — no other change required. |
| Renaming a member after spawn (e.g. when `cafleet member rename` is added) | No such CLI exists; out of scope. The agent-name column is treated as immutable for the pane's lifetime. |
| Auto-deriving a name when the user forgets `--name` | `--name` is already `required=True`; the only path here is "always required", so there is no defaulting branch to design. |

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

### Step 1: Documentation updates (per project convention — docs first)

- [ ] Update `ARCHITECTURE.md` Member Lifecycle section to note "the spawned `claude` process receives `--name <member-name>` so the tmux pane title shows the member name; codex panes use the auto-derived title". <!-- completed: -->
- [ ] Update `docs/spec/cli-options.md` `member create` table to add a row "*(spawn-side)* — for `--coding-agent claude`, the spawned process is invoked as `claude --name <member-name> <prompt>` so the pane title matches `--name`". <!-- completed: -->
- [ ] Update `.claude/skills/cafleet/SKILL.md` to mention "spawned `claude` panes show the member name as the tmux pane title; use `tmux list-panes -F '#{pane_id} #{pane_title}'` to find a specific member". <!-- completed: -->
- [ ] Verify `README.md` Member Lifecycle bullet (line 19) does not need editing — the high-level summary is already correct; only add a single clarifying clause if the surrounding paragraph misleads the reader. <!-- completed: -->

### Step 2: Code change — `coding_agent.py`

- [ ] Add `display_name_args: tuple[str, ...] = ()` field to `CodingAgentConfig` with default empty tuple (preserves frozen-dataclass invariants — see `test_coding_agent.py::TestCodingAgentConfig::test_frozen_dataclass`). <!-- completed: -->
- [ ] Set `CLAUDE.display_name_args = ("--name",)`; `CODEX.display_name_args` defaults to `()` (no override). <!-- completed: -->
- [ ] Modify `CodingAgentConfig.build_command()` signature to `build_command(self, prompt: str, *, display_name: str | None = None) -> list[str]` and inject `(*self.display_name_args, display_name)` between `extra_args` and the positional `prompt` when both `display_name` is truthy and `display_name_args` is non-empty. <!-- completed: -->

### Step 3: Code change — `cli.py`

- [ ] Modify the single `coding_agent_config.build_command(prompt)` call inside `member_create` (`cafleet/src/cafleet/cli.py:595`) to `coding_agent_config.build_command(prompt, display_name=name)`. No other call sites exist. <!-- completed: -->

### Step 4: Tests

- [ ] Extend `cafleet/tests/test_coding_agent.py::TestBuildCommand` with: (a) `test_display_name_kwarg_injects_for_claude` — `CLAUDE.build_command("p", display_name="Drafter") == ["claude", "--name", "Drafter", "p"]`; (b) `test_display_name_kwarg_no_op_for_codex` — `CODEX.build_command("p", display_name="Drafter") == ["codex", "--approval-mode", "auto-edit", "p"]`; (c) `test_display_name_none_matches_default` — calling without `display_name` keyword equals calling with `display_name=None` (parametrise both forms); (d) `test_display_name_with_spaces_preserved` — `CLAUDE.build_command("p", display_name="Code Reviewer") == ["claude", "--name", "Code Reviewer", "p"]`; (e) `test_display_name_args_field_default_empty_tuple` — a config built without `display_name_args` exposes `()`. <!-- completed: -->
- [ ] Add a `test_cli_member.py` (extend the existing file) test `test_member_create_passes_member_name_as_display_name` that monkeypatches `tmux.split_window` to capture the `command` kwarg, runs `member_create` via the click test runner with `--coding-agent claude --name "Drafter" -- "hello"`, and asserts the captured `command` list contains `"--name"` immediately followed by `"Drafter"` immediately followed by `"hello"`. <!-- completed: -->
- [ ] Add the codex-side regression: same setup but `--coding-agent codex`, asserting the captured `command` does NOT contain `"--name"`. <!-- completed: -->

### Step 5: Verification

- [ ] `mise //:lint` passes. <!-- completed: -->
- [ ] `mise //:format` passes. <!-- completed: -->
- [ ] `mise //:typecheck` passes. <!-- completed: -->
- [ ] `mise //cafleet:test` passes (all existing `test_coding_agent.py` cases must continue to pass — the `display_name` kwarg is keyword-only with a `None` default, so positional callers are unaffected). <!-- completed: -->

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-04-15 | Initial draft |
