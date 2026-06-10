# `--model` Option for `member create` and the `CodingAgent` Spawn Protocol

**Status**: Complete
**Progress**: 21/21 tasks complete
**Last Updated**: 2026-06-10

## Overview

Add an optional `--model <string>` flag to `cafleet member create` and a `model` keyword parameter to every coding-agent backend's `build_spawn_argv`, so a Director can pin the LLM a member runs on (e.g. `--model sonnet` for claude, `--model gpt-5.4-mini` for codex, `--model anthropic/claude-sonnet-4-6` for opencode). When the flag is omitted the spawn argv stays byte-identical to today and each binary uses its own default model.

## Success Criteria

- [x] `cafleet member create --model <m>` forwards `<m>` to the spawned backend binary via that binary's `--model` flag; omitting `--model` produces a spawn argv byte-identical to the pre-change argv for all three backends.
- [x] `--model` values violating the opencode `<provider-id>/<model-id>` format fail at create time with exit 2 and the documented error message, before any agent registration or tmux side effect.
- [x] claude and codex accept any string (pass-through; the binary itself rejects unknown models, so newly released models work without a cafleet release).
- [x] All listed docs and skills document `--model`, and `skills/cafleet/reference/director.md` carries the model-name-to-backend inference table so a Director can resolve "create a member with sonnet" to `--coding-agent claude --model sonnet`.
- [x] `mise //cafleet:lint`, `mise //cafleet:typecheck`, and `mise //cafleet:test` pass with the new tests in place.

---

## Background

`cafleet member create` spawns a member pane by calling `agent.build_spawn_argv(prompt, display_name=name)` (`cafleet/src/cafleet/cli.py:850`) on the backend selected via `--coding-agent {claude,codex,opencode}`. The `CodingAgent` Protocol (`cafleet/src/cafleet/coding_agent/base.py`) currently has no way to select the LLM; every member runs on the binary's default model. Operators who want a cheaper or stronger model per member have no per-spawn control.

`cafleet fleet create` is out of scope: its `--coding-agent` flag is operator-declared metadata only — it never spawns a process (see `docs/concepts/coding-agents.md`), so there is nothing for a model value to apply to.

Backend flag names were verified during drafting:

| Backend | Verified flag | Source |
|---|---|---|
| `claude` | `--model <model>` — alias (`fable`, `opus`, `sonnet`) or full name (`claude-fable-5`) | `claude --help` |
| `codex` | `-m, --model <MODEL>` — "Override the model set in configuration (for example `gpt-5.4`)" | <https://developers.openai.com/codex/cli/reference> |
| `opencode` | `-m, --model <provider/model>` — accepted by the bare TUI entry point (not only `opencode run`) | <https://opencode.ai/docs/cli/> |

The long form `--model` is used for all three backends; short aliases are never emitted.

---

## Specification

### CLI surface

New option on `member create` only:

```python
@click.option(
    "--model",
    "model",
    type=str,
    default=None,
    help="Model passed to the backend binary.",
)
```

The help string is deliberately a single short phrase: `tests/test_cli_help_budget.py` pins `member create --help` to a per-subcommand line budget (currently 13 lines — already at the cap) and the budgeted subcommands to a 4620-byte aggregate. In CliRunner's 80-column rendering the help column starts after the widest option spec (`--coding-agent [claude|codex|opencode]`), leaving roughly 37 characters per line, so the help text above (35 characters) must not grow. The new option line requires two budget bumps in `test_cli_help_budget.py` (tracked as a Step 4 task): `("member", "create")` from 13 to 14, and the aggregate byte budget from 4620 to the re-measured total (keep it tight — round the measured value up to the next multiple of 10). Narrative explanation of the flag lives in `docs/spec/cli-options.md`, per that test file's design.

| Property | Value |
|---|---|
| Scope | `member create` only — no change to `fleet create` or any other subcommand |
| Default | `None` (flag absent → no model tokens in the spawn argv) |
| Validation | Delegated to the selected backend via `validate_model` (see below) |
| Persistence | None — spawn-time only. NOT recorded in `agent_placements`, no Alembic migration, not shown in `member list` output |

The `member_create` body changes in two places:

1. Immediately after `agent = CODING_AGENTS[coding_agent]` (before `ensure_available()`, tmux context discovery, and `broker.register_agent`), call:

   ```python
   try:
       agent.validate_model(model)
   except ValueError as exc:
       raise click.UsageError(str(exc)) from exc
   ```

   Failing here means a rejected model value performs zero side effects — no registration to roll back, no tmux call.

2. The spawn call at `cli.py:850` becomes:

   ```python
   spawn_command = agent.build_spawn_argv(prompt, display_name=name, model=model)
   ```

### Protocol change (`cafleet/src/cafleet/coding_agent/base.py`)

Two changes to the `CodingAgent` Protocol:

```python
def validate_model(self, model: str | None) -> None:
    """Raise ValueError if ``model`` is not acceptable to this backend.

    ``None`` (flag omitted) is always valid. Called by ``member create``
    before any registration or tmux side effect.
    """
    ...

def build_spawn_argv(
    self, prompt: str, *, display_name: str, model: str | None = None
) -> list[str]:
    ...
```

- `model` is keyword-only with default `None` (per user decision — NOT required-no-default).
- When `model is None`, the returned argv is byte-identical to the current pinned argv (no model tokens emitted).
- When `model` is set, the backend appends `"--model", model` immediately before the prompt tokens.
- The Protocol is `runtime_checkable`, so the existing `isinstance(impl, CodingAgent)` contract test automatically requires `validate_model` on every impl.

### Per-backend behavior

| Backend | `validate_model` | argv with `model=None` (unchanged) | argv with `model=<m>` |
|---|---|---|---|
| `claude` | No-op — any string passes (including empty string) | `[claude, --permission-mode, dontAsk, --name, <display>, <prompt>]` | `[claude, --permission-mode, dontAsk, --name, <display>, --model, <m>, <prompt>]` |
| `codex` | No-op — any string passes (including empty string) | `[codex, --ask-for-approval, never, --sandbox, workspace-write, <prompt>]` | `[codex, --ask-for-approval, never, --sandbox, workspace-write, --model, <m>, <prompt>]` |
| `opencode` | `<provider-id>/<model-id>` format check (rule below) | `[opencode, --agent, cafleet, --prompt, <prompt>]` | `[opencode, --agent, cafleet, --model, <m>, --prompt, <prompt>]` |

For opencode, "immediately before the prompt" means before the `--prompt <prompt>` pair, since the prompt rides as a flag value rather than a positional.

### Validation policy

**claude / codex — pass-through.** Any string is forwarded verbatim, including empty and whitespace-only strings (user decision: no CLI-level emptiness check for any backend). The binary errors on unknown models — deliberately, so newly released models work without a cafleet release.

**opencode — `<provider-id>/<model-id>` format, first-slash rule.** `validate_model` rejects any non-`None` value that does not split on the **first** `/` into two non-empty segments:

```python
def validate_model(self, model: str | None) -> None:
    if model is None:
        return
    provider, sep, model_id = model.partition("/")
    if not sep or not provider or not model_id:
        raise ValueError(
            "--model for the opencode backend must be "
            f"'<provider-id>/<model-id>' (got '{model}')."
        )
```

| Input | Verdict | Reason |
|---|---|---|
| `anthropic/claude-sonnet-4-6` | accept | provider + model id |
| `a/b/c` | accept | first-slash split: provider `a`, model id `b/c` (model ids may themselves contain slashes) |
| `no-slash` | reject | no `/` |
| `/x` | reject | empty provider segment |
| `x/` | reject | empty model-id segment |
| `` (empty) | reject | no `/` |
| `None` (flag omitted) | accept | no validation when the flag is absent |

The CLI wraps the `ValueError` in `click.UsageError`, producing exit 2 with:

```
Error: --model for the opencode backend must be '<provider-id>/<model-id>' (got '<value>').
```

This row is added to the `docs/spec/cli-options.md` § Error Messages table.

### Known-model catalogs (examples, not enforced)

These lists appear in docs and skills as **examples only**. cafleet never validates against them — claude/codex accept any string, and opencode validates format, not catalog membership.

| Backend | Example models |
|---|---|
| `claude` | `fable`, `opus`, `sonnet` |
| `codex` | `gpt-5.5`, `gpt-5.4`, `gpt-5.4-mini`, `gpt-5.3-codex-spark` |
| `opencode` | any `<provider-id>/<model-id>` string, e.g. `anthropic/claude-sonnet-4-6`, `openai/gpt-5.5` |

### Model-name-to-backend inference table (for skills)

Lives in `skills/cafleet/reference/director.md`; `skills/cafleet/SKILL.md` gets the `--model` flag mention plus a pointer to it. The table lets a Director resolve a natural-language request to the correct flag pair:

| User says (model name shape) | Inferred backend | Flags to pass |
|---|---|---|
| `fable`, `opus`, `sonnet`, or a `claude-*` full name | `claude` | `--coding-agent claude --model <name>` (`--coding-agent claude` may be omitted — it is the default) |
| `gpt-*` (e.g. `gpt-5.5`, `gpt-5.4-mini`, `gpt-5.3-codex-spark`) | `codex` | `--coding-agent codex --model <name>` |
| Any name containing a `/` (e.g. `anthropic/claude-sonnet-4-6`, `openai/gpt-5.5`) | `opencode` | `--coding-agent opencode --model <provider-id>/<model-id>` |
| Anything else — no shape match (e.g. `gemini-2.5-pro`, `o3-mini`, any unfamiliar bare name) | none — do NOT infer | Ask the operator which backend to use (or for the explicit `--coding-agent` + `--model` pair) before spawning |

Examples to include verbatim in the skill: "please create a member with sonnet" → `--coding-agent claude --model sonnet`; "with gpt-5.4-mini" → `--coding-agent codex --model gpt-5.4-mini`; "with anthropic/claude-sonnet-4-6" → `--coding-agent opencode --model anthropic/claude-sonnet-4-6`.

### Documentation and skill surface

| File | Change |
|---|---|
| `docs/spec/cli-options.md` | `--model` row in the `member create` flag table; with-model variants in the "Spawn command per backend" table; opencode rejection row in § Error Messages |
| `docs/concepts/coding-agents.md` | Model-selection paragraph: per-member `--model`, pass-through vs format-validation policy, spawn-time-only (not persisted) |
| `docs/reference/coding-agents/codex.md` | `--model` in § Spawn flags with example catalog |
| `docs/reference/coding-agents/opencode.md` | `--model` in § Spawn flags with the `<provider-id>/<model-id>` format rule |
| `docs/get-started/quickstart.md` | Brief optional `--model` mention next to the `member create` demo (keep the minimal flow intact) |
| `README.md` | Sync with the above (README is a first-class doc target per `.claude/rules/design-doc-numbering.md`) |
| `skills/cafleet/SKILL.md` | `--model` mention in § Coding-agent backends + pointer to the inference table in `reference/director.md` |
| `skills/cafleet/reference/director.md` | `--model` row in the `member create` flag table + with-model variants in its own "Spawn command per backend" table (the twin of the one in `docs/spec/cli-options.md` — both must change in the same cycle) + the inference table above |
| `skills/cafleet-agent-team-supervision/SKILL.md` | `--model` mention wherever the spawn protocol documents `member create` flags |

The global `~/.claude/skills/cafleet-agent-team-supervision` mirror is outside this repository and is synced as an untracked follow-up, not an implementation task here.

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

### Step 1: Documentation and skills (first, per `.claude/rules/design-doc-numbering.md`)

- [x] Update `docs/concepts/coding-agents.md` with the model-selection paragraph <!-- completed: 2026-06-10T13:43 -->
- [x] Update `docs/spec/cli-options.md`: flag table row, spawn-command table variants, error-messages row <!-- completed: 2026-06-10T13:44 -->
- [x] Update `docs/reference/coding-agents/codex.md` § Spawn flags with `--model` + example catalog <!-- completed: 2026-06-10T13:45 -->
- [x] Update `docs/reference/coding-agents/opencode.md` § Spawn flags with `--model` + format rule <!-- completed: 2026-06-10T13:45 -->
- [x] Update `docs/get-started/quickstart.md` with the optional `--model` mention <!-- completed: 2026-06-10T13:46 -->
- [x] Update `README.md` to stay consistent with the docs changes <!-- completed: 2026-06-10T13:47 -->
- [x] Update `skills/cafleet/SKILL.md` (§ Coding-agent backends mention + pointer) <!-- completed: 2026-06-10T13:48 -->
- [x] Update `skills/cafleet/reference/director.md` (flag row + with-model variants in its "Spawn command per backend" table + inference table) <!-- completed: 2026-06-10T13:49 -->
- [x] Update `skills/cafleet-agent-team-supervision/SKILL.md` (spawn-protocol mention) <!-- completed: 2026-06-10T13:50 -->

### Step 2: Protocol and backends

- [x] `base.py`: add `model: str | None = None` keyword to `build_spawn_argv` and the new `validate_model` method to the `CodingAgent` Protocol, with docstrings covering the None-omits-tokens and None-is-always-valid contracts <!-- completed: 2026-06-10T13:52 -->
- [x] `claude.py`: `validate_model` no-op; emit `--model <m>` between `--name <display>` and the prompt when set <!-- completed: 2026-06-10T13:52 -->
- [x] `codex.py`: `validate_model` no-op; emit `--model <m>` between `workspace-write` and the prompt when set <!-- completed: 2026-06-10T13:52 -->
- [x] `opencode.py`: `validate_model` first-slash format check with the exact error message; emit `--model <m>` between `cafleet` and `--prompt` when set <!-- completed: 2026-06-10T13:52 -->

### Step 3: CLI wiring

- [x] `cli.py` `member_create`: add the `--model` option, call `agent.validate_model(model)` (wrapping `ValueError` in `click.UsageError`) before `ensure_available()` / registration, and pass `model=model` to `build_spawn_argv` <!-- completed: 2026-06-10T13:52 -->

### Step 4: Tests

- [x] `tests/test_coding_agent_protocol.py`: per backend, byte-exact argv with `model` set; byte-exact argv with `model` omitted AND `model=None` explicit (both equal to the current pinned list); claude/codex `validate_model` accepts arbitrary strings including `""` <!-- completed: 2026-06-10T13:48 -->
- [x] `tests/test_coding_agent_opencode.py`: `validate_model` accept/reject matrix from the Specification table (incl. `a/b/c` accept and `""` reject); byte-exact argv with `model` set <!-- completed: 2026-06-10T13:48 -->
- [x] `tests/test_cli_member.py`: `--model` wiring per backend (tokens present at the specified position); no `--model` → no `--model` token in the spawn argv; opencode invalid value → exit 2, exact message, and no agent registered (validation precedes registration); claude/codex empty-string pass-through <!-- completed: 2026-06-10T13:48 -->
- [x] `tests/test_cli_help_budget.py`: bump `("member", "create")` line budget 13 → 14 and re-measure the aggregate byte budget (4620 → 4320, the measured total rounded up to the next multiple of 10) <!-- completed: 2026-06-10T13:48 -->

### Step 5: Quality gates and manual verification

- [x] `mise //cafleet:lint`, `mise //cafleet:typecheck`, `mise //cafleet:test` all pass <!-- completed: 2026-06-10T13:52 -->
- [x] Manual smoke (inside tmux, with a fleet created via `cafleet fleet create`): run the quickstart-derived spawn `cafleet --fleet-id <fleet-id> member create --agent-id <director-id> --name "demo-member" --description "Demo member" --model sonnet -- "You are demo-member. Reply hello when polled."`. Pass: the member pane is running `claude --permission-mode dontAsk --name demo-member --model sonnet <prompt>` (verify the argv via the pane's process, e.g. `ps -o args= -t <pane-tty>`, or by observing the model name in the spawned session's status line via `cafleet member capture`). Clean up with `cafleet member delete` <!-- completed: 2026-06-10T14:00 -->
- [x] Natural-language Director test: start a fresh Claude Code session at the repo root AFTER the Step 1 skill updates land, so the updated `skills/cafleet/SKILL.md` and `skills/cafleet/reference/director.md` are what load; create a fleet, then tell the Director "please create a member with sonnet". Pass: the `member create` Bash invocation the Director constructs — observed in the session transcript / pane history at or before execution — targets the claude backend (explicit `--coding-agent claude` or omitted default) AND contains `--model sonnet`. Fail: any other backend, a missing `--model sonnet`, or the Director asking which backend to use (sonnet matches the claude row of the inference table, so no fallback question is warranted) <!-- completed: 2026-06-10T14:14 -->

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-06-10 | Initial draft |
| 2026-06-10 | Reviewer round 1: single-phrase `--model` help string + help-budget bump task; inference-table fallback row (no shape match → ask the operator); with-model variants in `director.md`'s spawn-command table; concrete procedure and pass criterion for both manual verification tasks |
| 2026-06-10 | Implementation complete: docs/skills (cc985f9), tests (1758bb0), implementation (98a3b0b), manual verification recorded (d3243c2); PR #104 reviewed by Copilot with zero comments; Status → Complete |
