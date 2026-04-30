# `cafleet member exec` — extract bash dispatch into its own subcommand

**Status**: Approved
**Progress**: 22/28 tasks complete
**Last Updated**: 2026-04-30

## Overview

Extract the bash-dispatch primitive currently exposed as `cafleet member send-input --bash CMD` into its own Director-only subcommand `cafleet member exec CMD` (positional argument, no flag), and reject any `cafleet member send-input --freetext` value whose first non-whitespace character is `!` so the AskUserQuestion path cannot smuggle a Claude Code `!`-shortcut and bypass the new subcommand. Pure CLI surface split — no permission-aware logic, no allow/deny machinery, no aliases. The `--bash` flag is removed from `send-input` in the same change set per `.claude/rules/removal.md`.

## Success Criteria

- [ ] `cafleet member send-input` no longer accepts a `--bash` flag. Click rejects the old form with `Error: No such option: '--bash'.` (exit 2). Every literal mention of the old `--bash` flag on `send-input` is removed in the same change.
- [ ] `cafleet member exec CMD` exists as a new Director-only subcommand. CMD is a single required positional argument (`@click.argument("command")`); there is no `--bash` flag on this subcommand.
- [ ] `cafleet member exec` reuses `_load_authorized_member` for cross-Director boundary enforcement and `tmux.send_bash_command` for the `! CMD` + Enter keystroke pair — no new tmux helper, no new authorization code.
- [ ] `cafleet member send-input --freetext "<value>"` rejects any value whose first non-whitespace character (per `str.lstrip()` default) is `!`. Rejection wording: `Error: --freetext may not start with '!' — that triggers Claude Code's shell-execution shortcut. Use 'cafleet member exec' for shell dispatch instead.` Exit 2 (Click `UsageError`). Empty `--freetext ""` and whitespace-only values stay accepted (current behavior unchanged).
- [ ] `cafleet member exec` exit codes: `0` dispatch success, `2` for any input-validation failure (Click `UsageError`: missing positional, empty string, newline in command), `1` for runtime / IO / authorization failures (cross-Director rejection, missing placement, pending placement, tmux unavailable, tmux call failed). Empty-string and newline preflight live at the CLI handler and exit `2`; `tmux.send_bash_command`'s internal `TmuxError` preflight stays as defense-in-depth but the CLI handler never reaches it.
- [ ] Cross-Director, missing-placement, and pending-placement rejection wording on `cafleet member exec` reuses `_load_authorized_member`'s existing strings verbatim (mirroring `member capture` / `member send-input`).
- [ ] `tmux` unavailability on `cafleet member exec` exits `1` with the existing `cafleet member commands must be run inside a tmux session` wording (same surface as every other `member` subcommand).
- [ ] `.claude/settings.json` removes the three obsolete `ask` entries that scoped the old `--bash` flag form on `send-input` and adds one new `ask` entry: `Bash(cafleet --session-id * member exec *)`.
- [ ] Documentation is updated FIRST per `.claude/rules/design-doc-numbering.md`. Targets: `ARCHITECTURE.md`, `docs/spec/cli-options.md`, `README.md`, `skills/cafleet/SKILL.md`, `skills/cafleet/roles/director.md`, `skills/cafleet/roles/member.md`, `.claude/rules/bash-tool.md`, `.claude/settings.json`. Both the Director-side "for completeness" code block AND the operator-fallback paragraph in `bash-tool.md` § "When your Bash tool denies a command" point at `cafleet member exec`.
- [ ] `cafleet/tests/test_cli_member_send_input.py` is updated: `TestBashFlag` class deleted, `bash_recorder` fixture deleted, `--bash` removed from `TestFlagValidation` parametrizations, ONE regression test added asserting the old `--bash <cmd>` form errors with Click `No such option`, and tests added for the new bang-prefix rejection on `--freetext`.
- [ ] `cafleet/tests/test_cli_member_exec.py` is created: covers positional CMD dispatch via `tmux.send_bash_command`, cross-Director rejection, pending-placement rejection, missing-placement rejection, and empty-string / newline rejection.
- [ ] `mise //cafleet:test`, `mise //cafleet:lint`, `mise //cafleet:format`, `mise //cafleet:typecheck` all pass.

---

## Background

### Predecessor designs

| Design | Status | Relationship to this design |
|---|---|---|
| **0000034** `member-bash-via-director` | Complete | Introduced `cafleet member send-input --bash CMD` as the Director's dispatch primitive when a member's harness deny-list rejects a Bash invocation. Three `ask` rules in `.claude/settings.json` scope the operator confirmation prompt. **Superseded by this design** — the `--bash` flag is removed entirely. |
| **0000037** `permission-aware-shell-dispatch` | Approved (not merged) | Designed a richer replacement that re-reads `permissions.allow` / `permissions.deny` on every dispatch and decides allow / deny / ask via a glob matcher. The user opted not to merge that design. **This design is the deliberately reduced subset** — only the CLI surface split (no permission discovery, no glob matcher, no decide-allow-or-deny machinery). 0000037 stays available as the deferred fuller version when permission-aware control is needed. |

### Why the surface split now, without the permission-aware layer

The current `cafleet member send-input --bash CMD` overloads `send-input` with two unrelated input modes: AskUserQuestion replies (`--choice`, `--freetext`) and shell dispatch (`--bash`). The two modes share no validation rules, no key-sequence shape (`--bash` skips the AskUserQuestion `4` digit), and no operator-confirmation pattern (the three `ask` rules apply only to the `--bash` form). Splitting them into two subcommands makes each subcommand single-purpose, simplifies the flag-validation matrix, and creates a stable `cafleet member exec ...` invocation pattern that any future permission-aware layer (e.g. design 0000037) can hook into without further surface churn.

The bang-prefix guardrail on `--freetext` exists because Claude Code's `!`-shortcut would otherwise let an operator (or a misbehaving Director) smuggle a shell command through `--freetext "!ls"`, bypassing the surface split. Rejecting any `--freetext` value whose first non-whitespace character is `!` keeps the new boundary enforceable from the CLI alone.

---

## Specification

### 1. Subcommand surface

| Subcommand | Before | After |
|---|---|---|
| `cafleet member send-input` | accepts `--choice`, `--freetext`, `--bash` (mutually exclusive). `--freetext` accepts any non-newline string including `!`-prefixed values. | accepts `--choice`, `--freetext` (mutually exclusive). `--bash` is removed entirely. `--freetext` rejects values whose first non-whitespace character is `!`. |
| `cafleet member exec` | does not exist | accepts a single required positional `CMD`. Director-only. Reuses `_load_authorized_member` + `tmux.send_bash_command`. |

No aliasing. No deprecation period. No wrapper-shim. Per `.claude/rules/removal.md`, every literal mention of the old `--bash` flag on `cafleet member send-input` is removed in the same change set. **Callers must rewrite to `cafleet member exec CMD`. No migration shim, no deprecation period — the old form starts erroring immediately on merge.**

### 2. `cafleet member exec` Click signature and handler contract

```python
@member.command("exec")
@click.option("--agent-id", required=True, help="Director's agent ID")
@click.option("--member-id", required=True, help="Target member's agent ID")
@click.argument("command")
@click.pass_context
def member_exec(ctx, agent_id, member_id, command):
    """Dispatch a shell command into a member's pane via Claude Code's `!` shortcut.

    Director-only. Reuses _load_authorized_member for cross-Director enforcement
    and tmux.send_bash_command for the `! CMD` + Enter keystroke pair.
    """
```

#### Input-validation contract (CLI handler, before any tmux call)

| Input | Behavior |
|---|---|
| Missing positional `CMD` | Click built-in `Error: Missing argument 'COMMAND'.` (exit 2). |
| `command` with empty body after `.strip()` (i.e., `""` or whitespace-only) | `raise click.UsageError("command may not be empty.")` (exit 2). |
| `command` containing `\n` or `\r` | `raise click.UsageError("command may not contain newlines.")` (exit 2). |
| Compound CMD (pipes, `&&`, `;`, `$(...)`, backticks) | NOT special-cased. Treated as opaque string and forwarded to `tmux.send_bash_command` as-is. The `Bash(cafleet --session-id * member exec *)` `ask` rule in `.claude/settings.json` is the operator-confirmation surface. |

#### Authorization contract (post-validation, pre-dispatch)

Mirrors `member send-input` step-for-step:

1. `tmux.ensure_tmux_available()` — raises `tmux.TmuxError`, wrapped as `ClickException` exit 1 with existing `cafleet member commands must be run inside a tmux session` wording.
2. `_load_authorized_member(session_id, agent_id, member_id, placement_missing_msg=...)` — emits the existing missing-agent / missing-placement / cross-Director errors verbatim (exit 1).
3. If `placement["tmux_pane_id"] is None` (pending placement), exit 1 with `Error: member <member_id> has no pane yet (pending placement) — nothing to send.` (mirrors `member send-input`).

The `placement_missing_msg` parameter passed to `_load_authorized_member` reuses the `cafleet member send-input` wording verbatim: `agent {member_id} has no placement row; it was not spawned via 'cafleet member create'.`

#### Dispatch

```python
try:
    tmux.send_bash_command(target_pane_id=pane_id, command=command)
except tmux.TmuxError as exc:
    raise click.ClickException(f"send failed: {exc}") from exc
```

The CLI handler does not reach `tmux.send_bash_command`'s internal empty-string / newline preflight under normal use, because the CLI's `UsageError` preflight runs first. The internal preflight stays as defense-in-depth for direct callers of `tmux.send_bash_command` (currently none outside the CLI, but the helper is module-level).

#### Output

Text:

```
Sent bash command 'git log -1 --oneline' to member Claude-B (%7).
```

JSON (`cafleet --json ... member exec ...`):

```json
{
  "member_agent_id": "<uuid>",
  "pane_id": "%7",
  "command": "<command as-sent>"
}
```

Three keys: `member_agent_id`, `pane_id`, `command`. No `action` field (the subcommand name IS the action).

#### Exit code summary

| Outcome | Exit | Source |
|---|---|---|
| Dispatch success | `0` | normal return |
| Missing positional `CMD` | `2` | Click built-in |
| `command` empty / whitespace-only | `2` | `click.UsageError` raised by handler |
| `command` contains `\n` or `\r` | `2` | `click.UsageError` raised by handler |
| `tmux` unavailable / `TMUX` env var missing | `1` | `tmux.ensure_tmux_available()` → wrapped `ClickException` |
| Agent not found | `1` | `_load_authorized_member` → wrapped `ClickException` |
| Missing placement row | `1` | `_load_authorized_member` (existing wording) |
| Cross-Director (placement.director_agent_id mismatch) | `1` | `_load_authorized_member` (existing wording) |
| Pending placement (tmux_pane_id is None) | `1` | dedicated check in handler (existing wording) |
| `tmux send-keys` subprocess error | `1` | wrapped `ClickException` (`send failed: ...`) |

### 3. Bang-prefix guardrail on `cafleet member send-input --freetext`

Inserted at the top of `member_send_input` alongside the existing newline rejection.

```python
if freetext is not None and freetext.lstrip().startswith("!"):
    raise click.UsageError(
        "--freetext may not start with '!' — that triggers Claude Code's "
        "shell-execution shortcut. Use 'cafleet member exec' for shell dispatch instead."
    )
```

| Input | Result |
|---|---|
| `--freetext "!ls"` | Reject (exit 2). Bang is the first non-whitespace character. |
| `--freetext "  !ls"` | Reject (exit 2). After `lstrip()`, first character is `!`. |
| `--freetext "!"` | Reject (exit 2). Lone bang. |
| `--freetext "hi !"` | Accept. Bang is not in the leading position after `lstrip()`. |
| `--freetext "hi"` | Accept (current behavior unchanged). |
| `--freetext ""` | Accept (current behavior unchanged — submits an empty answer to the AskUserQuestion prompt). |
| `--freetext "   "` | Accept. After `lstrip()`, the string is empty, so `.startswith("!")` is `False`. Current behavior unchanged. |

Whether the bang-prefix check runs before or after the existing newline check is irrelevant for correctness — both `UsageError`s exit 2, and ordering only affects which error message surfaces when a value violates both rules. Place the bang-prefix check first (more specific).

`str.lstrip()`'s default whitespace set (any Unicode whitespace per `str.isspace()`) is sufficient; no custom whitespace class is needed.

### 4. `--bash` flag removal scope

Per `.claude/rules/removal.md`, every literal mention of the old `--bash` flag on `cafleet member send-input` is removed in the same change set:

| Surface | Action |
|---|---|
| `cafleet/src/cafleet/cli.py` | Remove the `@click.option("--bash", ...)` decorator and the `bash_command` parameter from `member_send_input`. Remove the `bash_command` arm of the dispatch logic. Update the `supplied != 1` check to count only `(choice, freetext)`. Remove `--bash` from the mutual-exclusion error wording. |
| `cafleet/src/cafleet/tmux.py` | `send_bash_command` is RETAINED — it is the dispatch primitive for the new `cafleet member exec` subcommand. No source changes. |
| `cafleet/tests/test_cli_member_send_input.py` | See §6 Test surface. |
| `docs/spec/cli-options.md` | Drop the `--bash` row from the `member send-input` flags table. Drop the `--bash` keystroke row from the key-sequence table. Drop the `--bash` validation rows from the validation table. Drop the `--bash` JSON output example. Add a new `### member exec` section. Update the error-message table. |
| `skills/cafleet/SKILL.md` | Rewrite the `Routing Bash via the Director` section to point at `cafleet member exec`. Drop every `--bash` reference. |
| `skills/cafleet/roles/director.md` | Replace `cafleet member send-input ... --bash "<command>"` examples with `cafleet member exec ... "<command>"`. Update the "What you MUST do" steps. |
| `skills/cafleet/roles/member.md` | Replace the `cafleet member send-input --bash <command>` reference in "WHEN YOUR BASH TOOL DENIES A COMMAND" with `cafleet member exec <command>`. |
| `.claude/rules/bash-tool.md` | Update BOTH the operator-fallback paragraph in §"When your Bash tool denies a command" AND the §"Director side (for completeness)" code block to point at `cafleet member exec`. |
| `.claude/settings.json` | Remove the three obsolete `ask` rules. Add `Bash(cafleet --session-id * member exec *)`. |
| `ARCHITECTURE.md` | Update any `member send-input --bash` mentions to `member exec`. |
| `README.md` | Update any `member send-input --bash` mentions to `member exec`. |
| Design doc 0000034 | NOT modified. The design doc is the historical record of the original `--bash` introduction; per `.claude/rules/removal.md`, "the design doc that authorized the [introduction] stays" (and this design doc, 0000038, is the canonical record of the removal). |

### 5. `.claude/settings.json` updates

Remove from `permissions.ask`:

```json
"Bash(cafleet --session-id * member send-input --bash *)",
"Bash(cafleet --session-id * member send-input * --bash)",
"Bash(cafleet --session-id * member send-input * --bash *)"
```

Add to `permissions.ask`:

```json
"Bash(cafleet --session-id * member exec *)"
```

The new pattern matches every `cafleet --session-id <uuid> member exec --agent-id <uuid> --member-id <uuid> '<cmd>'` invocation regardless of CMD content. The operator confirmation surface is preserved per-call.

### 6. Test surface

#### `cafleet/tests/test_cli_member_send_input.py` — updates

- **Delete** the `TestBashFlag` class entirely (the five tests that exercise the `--bash` happy path, mutual exclusion with `--choice`, mutual exclusion with `--freetext`, the no-flag-supplied error wording change, and the JSON audit log).
- **Delete** the `bash_recorder` fixture.
- **Update** `TestFlagValidation`:
  - Remove `--bash` mentions from `test_no_flag_supplied_exits_two` and `test_choice_and_freetext_combo_exits_two` (the error wording no longer enumerates `--bash`).
  - Keep all other tests in `TestFlagValidation` unchanged.
- **Add** `TestBashFlagRemoved` regression class with one test asserting `cafleet ... member send-input --bash "x"` exits 2 with Click's `No such option: '--bash'` error. The test guards against future re-introduction of the flag.
- **Add** `TestFreetextBangRejection` class:
  | Test name | Input | Expected exit | Expected message substring |
  |---|---|---|---|
  | `test_freetext_leading_bang_rejected` | `--freetext "!ls"` | 2 | `--freetext may not start with '!'` |
  | `test_freetext_whitespace_then_bang_rejected` | `--freetext "  !ls"` | 2 | `--freetext may not start with '!'` |
  | `test_freetext_lone_bang_rejected` | `--freetext "!"` | 2 | `--freetext may not start with '!'` |
  | `test_freetext_bang_not_in_leading_position_accepted` | `--freetext "hi !"` | 0 | (dispatched via `freetext_recorder`) |
  | `test_freetext_empty_still_accepted` | `--freetext ""` | 0 | (dispatched via `freetext_recorder`; current behavior unchanged) |
  | `test_freetext_whitespace_only_accepted` | `--freetext "   "` | 0 | (dispatched via `freetext_recorder`; `lstrip()` empties the string before the `startswith("!")` check) |

#### `cafleet/tests/test_cli_member_exec.py` — new

Mirror the structure of `test_cli_member_send_input.py`:

- Reuse the `_placement` / `_agent` / `_UNSET` / `session_id` / `runner` / `_stub_tmux_available` / `happy_path_agent` patterns.
- Add a `bash_recorder` fixture (recording calls to `tmux.send_bash_command`).
- Test classes:

| Class | Tests |
|---|---|
| `TestExecDispatch` | `test_positional_cmd_dispatched_with_pane_and_command` (asserts `tmux.send_bash_command` called once with `target_pane_id=PANE_ID` and matching `command`); `test_text_output`; `test_json_output_three_keys` (asserts `{member_agent_id, pane_id, command}` exactly). |
| `TestInputValidation` | `test_missing_positional_exits_two` (Click built-in `Missing argument`); `test_empty_command_exits_two` (`command may not be empty.`); `test_whitespace_only_command_exits_two` (`command may not be empty.`); `test_command_with_newline_exits_two` (parametrized over `\n`, `\r`, `\r\n`, leading `\n`, trailing `\n`). |
| `TestAuthorizationBoundary` | `test_missing_agent_exits_one`; `test_placement_none_exits_one_with_exact_message`; `test_cross_director_exits_one_with_exact_message`; `test_pending_pane_exits_one_with_exact_message`. Wording reuses the existing `_load_authorized_member` strings verbatim (mirrors `test_cli_member_send_input.py::TestAuthorizationBoundary`). |
| `TestTmuxUnavailable` | `test_tmux_not_available_exits_one` (monkeypatches `tmux.ensure_tmux_available` to raise `TmuxError`; asserts existing `cafleet member commands must be run inside a tmux session` wording). |

### 7. What this design deliberately does NOT do

- No permission discovery (`permissions.allow` / `permissions.deny` re-read at dispatch time). That is design 0000037, deferred.
- No glob matcher for `Bash(...)` patterns. That is design 0000037, deferred.
- No `--json` machine-readable allow/deny/ask outcome envelope. That is design 0000037, deferred.
- No alias from `cafleet member send-input --bash` to `cafleet member exec`. Hard rename per `.claude/rules/removal.md`.
- No deprecation warning. The old form errors immediately with Click's built-in `No such option`.

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

### Step 1: Documentation FIRST

Per `.claude/rules/design-doc-numbering.md`, every documentation surface is updated BEFORE any code or test changes.

- [x] Update `ARCHITECTURE.md`: replace any `member send-input --bash` mentions with `member exec` and add a one-line entry for the new subcommand. <!-- completed: 2026-04-30T11:15 -->
- [x] Update `docs/spec/cli-options.md`: drop the `--bash` row from the `member send-input` flags table; drop the `--bash` rows from the key-sequence and validation tables; drop the `--bash` JSON example; remove `--bash` from the error-messages table; add a new `### member exec` section mirroring the structure of `### member send-input` (flags table, key-sequence row, validation table, authorization-boundary subsection, output format, exit-code summary). Update the `--session-id` "Required for" list at the top of the file to include `member exec`. Update the agent-id-required subcommand list to include `member exec`. <!-- completed: 2026-04-30T11:25 -->
- [x] Update `README.md`: replace any `member send-input --bash` mentions with `member exec`. <!-- completed: 2026-04-30T11:30 -->
- [x] Rewrite `skills/cafleet/SKILL.md` § `Routing Bash via the Director`: replace every reference to `cafleet member send-input --bash` with `cafleet member exec`. Update the `### Member Send-Input` section to remove the `--bash` flag row, the `--bash` mode column wording, the `--bash` examples, the `--bash` JSON output, and the "Why two tmux calls for `--bash`" note (the new `### Member Exec` section captures it). Add a new `### Member Exec` section between `### Member Send-Input` and `### Server` documenting the new subcommand. <!-- completed: 2026-04-30T11:35 -->
- [x] Rewrite `skills/cafleet/roles/director.md`: replace the `cafleet member send-input ... --bash "<command>"` example with `cafleet member exec ... "<command>"`. Update step 2 ("If fulfilling, dispatch via …") and step 3 ("`--bash` flag mechanics") to describe `cafleet member exec` (positional CMD, no AskUserQuestion gate, mutually exclusive with nothing because `exec` has no other input modes). Update the cross-Director-boundary paragraph to reference `cafleet member exec`. <!-- completed: 2026-04-30T11:38 -->
- [x] Update `skills/cafleet/roles/member.md`: replace the `cafleet member send-input --bash <command>` reference in `WHEN YOUR BASH TOOL DENIES A COMMAND — RECONSIDER, THEN AUTO-ROUTE TO THE DIRECTOR` with `cafleet member exec <command>`. <!-- completed: 2026-04-30T11:40 -->
- [x] Update `.claude/rules/bash-tool.md`: update BOTH the operator-fallback paragraph in §"When your Bash tool denies a command" (the `please dispatch via cafleet member send-input --bash <command>` line) AND the §"Director side (for completeness)" code block (the `cafleet member send-input ... --bash "<command>"` example) to point at `cafleet member exec`. <!-- completed: 2026-04-30T11:50 -->
- [x] Update `.claude/settings.json`: remove the three obsolete `ask` entries that scoped the old `--bash` flag form on `send-input`; add `Bash(cafleet --session-id * member exec *)` to the `ask` list. <!-- completed: 2026-04-30T11:50 -->

### Step 2: Implementation — `cafleet/src/cafleet/cli.py`

- [x] Remove the `@click.option("--bash", "bash_command", ...)` decorator from `member_send_input`. Remove `bash_command` from the function signature. Remove the `bash_command is not None` arm of the dispatch logic. Update the `supplied = sum(...)` count to span `(choice, freetext)` only. Update the mutual-exclusion `UsageError` wording to `"--choice and --freetext are mutually exclusive; supply exactly one."`. Remove the `--bash` mention from the audit-log `action` enum (it is now only `"choice"` or `"freetext"`). Update the text-output `label` ladder (drop the `bash` arm). <!-- completed: 2026-04-30T13:30 -->
- [x] Add the bang-prefix guardrail at the top of `member_send_input`, BEFORE the existing newline rejection: `if freetext is not None and freetext.lstrip().startswith("!"): raise click.UsageError("--freetext may not start with '!' — that triggers Claude Code's shell-execution shortcut. Use 'cafleet member exec' for shell dispatch instead.")`. <!-- completed: 2026-04-30T13:30 -->
- [ ] Add the new `member_exec` subcommand. Place it directly after `member_send_input` to keep related Director-only commands clustered. Implementation outline:
  1. `_require_session_id(ctx)`.
  2. Validate `command`: `if not command.strip(): raise click.UsageError("command may not be empty.")`. `if "\n" in command or "\r" in command: raise click.UsageError("command may not contain newlines.")`.
  3. `tmux.ensure_tmux_available()` (wrap `TmuxError` as `ClickException`).
  4. `_load_authorized_member(session_id, agent_id, member_id, placement_missing_msg=...)`.
  5. `if pane_id is None: raise click.ClickException("member <id> has no pane yet (pending placement) — nothing to send.")` (existing wording).
  6. `tmux.send_bash_command(target_pane_id=pane_id, command=command)` wrapped in `try` / `except tmux.TmuxError as exc: raise click.ClickException(f"send failed: {exc}") from exc`.
  7. JSON output: `{"member_agent_id": member_id, "pane_id": pane_id, "command": command}`.
  8. Text output: `f"Sent bash command {command!r} to member {target['name']} ({pane_id})."`. <!-- completed: -->
- [ ] Run `mise //cafleet:lint` and `mise //cafleet:format` to confirm the new code is clean. <!-- completed: -->

### Step 3: Tests — `cafleet/tests/test_cli_member_send_input.py`

- [x] Delete the `TestBashFlag` class entirely. <!-- completed: 2026-04-30T13:06 -->
- [x] Delete the `bash_recorder` fixture. <!-- completed: 2026-04-30T13:06 -->
- [x] Update `TestFlagValidation::test_no_flag_supplied_exits_two`: remove the `assert "--bash" in out` assertion; update to assert the new wording `"--choice and --freetext are mutually exclusive"`. <!-- completed: 2026-04-30T13:06 -->
- [x] Update `TestFlagValidation::test_choice_and_freetext_combo_exits_two`: remove the `assert "--bash" in out` assertion. <!-- completed: 2026-04-30T13:06 -->
- [x] Add `TestBashFlagRemoved` class with one regression test asserting `cafleet --session-id <s> member send-input --agent-id <d> --member-id <m> --bash "x"` exits 2 with Click `No such option` error containing the literal substring `--bash`. <!-- completed: 2026-04-30T13:06 -->
- [x] Add `TestFreetextBangRejection` class with the six tests enumerated in §6 Test surface. <!-- completed: 2026-04-30T13:06 -->

### Step 4: Tests — `cafleet/tests/test_cli_member_exec.py` (new)

- [x] Create `cafleet/tests/test_cli_member_exec.py`. Mirror the fixture pattern from `test_cli_member_send_input.py`: `_placement`, `_agent`, `_UNSET`, `session_id`, `runner`, `_stub_tmux_available` (autouse), `happy_path_agent`, `bash_recorder` (records calls to `tmux.send_bash_command`). <!-- completed: 2026-04-30T13:14 -->
- [x] Add the `_invoke` helper for `cafleet --session-id <s> member exec --agent-id <d> --member-id <m> <command>`. <!-- completed: 2026-04-30T13:14 -->
- [x] Add `TestExecDispatch` class with: `test_positional_cmd_dispatched_with_pane_and_command`, `test_text_output`, `test_json_output_three_keys`. <!-- completed: 2026-04-30T13:14 -->
- [x] Add `TestInputValidation` class with: `test_missing_positional_exits_two` (Click `Missing argument`), `test_empty_command_exits_two`, `test_whitespace_only_command_exits_two`, `test_command_with_newline_exits_two` (parametrized over `\n`, `\r`, `\r\n`, leading `\n`, trailing `\n`). <!-- completed: 2026-04-30T13:14 -->
- [x] Add `TestAuthorizationBoundary` class with: `test_missing_agent_exits_one`, `test_placement_none_exits_one_with_exact_message`, `test_cross_director_exits_one_with_exact_message`, `test_pending_pane_exits_one_with_exact_message`. Wording mirrors `test_cli_member_send_input.py::TestAuthorizationBoundary` verbatim. <!-- completed: 2026-04-30T13:14 -->
- [x] Add `TestTmuxUnavailable` class with `test_tmux_not_available_exits_one`. <!-- completed: 2026-04-30T13:14 -->

### Step 5: Quality gates

- [ ] `mise //cafleet:test` passes (existing tests still pass after `--bash` removal; new tests pass). <!-- completed: -->
- [ ] `mise //cafleet:lint` passes. <!-- completed: -->
- [ ] `mise //cafleet:format` passes. <!-- completed: -->
- [ ] `mise //cafleet:typecheck` passes. <!-- completed: -->

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-04-30 | Initial draft. |
| 2026-04-30 | Approved by user. Status flipped to Approved. Last Updated refreshed. |
