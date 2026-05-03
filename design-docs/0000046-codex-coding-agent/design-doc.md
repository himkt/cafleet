# Codex Coding Agent

**Status**: Approved
**Progress**: 15/29 tasks complete
**Last Updated**: 2026-05-03

## Overview

Add the OpenAI `codex` CLI as a second supported coding-agent binary alongside `claude`. Operators select the binary at session-create time (for the root Director) and at member-create time (per member) via a new `--coding-agent {claude,codex}` flag; default `claude` preserves existing behavior. The CLI, prompt template, and supporting docs become backend-neutral; the broker, message lifecycle, and tmux primitives remain unchanged.

## Success Criteria

- [ ] `cafleet session create --coding-agent codex` records `placement.coding_agent = "codex"` for the root Director and is reflected in `cafleet session create --json` output.
- [ ] `cafleet member create --coding-agent codex` spawns a `codex --ask-for-approval never --sandbox workspace-write <prompt>` process in the new pane and records `placement.coding_agent = "codex"`.
- [ ] `cafleet member create --coding-agent claude` continues to spawn a `claude --permission-mode dontAsk --name <name> <prompt>` process unchanged.
- [ ] Mixed-backend teams work: a Director may spawn one `claude` member and one `codex` member in the same session with no broker / tmux interaction differences.
- [ ] `cafleet member exec` dispatches `! <command>` keystrokes into both `claude` and `codex` panes successfully (the `!` shell shortcut is honored by both binaries).
- [ ] `--coding-agent codex` exits with `binary codex not found on PATH` when the `codex` binary is absent; `--coding-agent claude` exits with `binary claude not found on PATH` when the `claude` binary is absent.
- [ ] `member send-input --freetext` continues to reject values whose first non-whitespace character is `!` for both backends.
- [ ] `ARCHITECTURE.md`, `README.md`, `docs/codex-members.md` (new), `docs/spec/cli-options.md` (if it exists), and every affected `skills/*/SKILL.md` describe the current dual-backend surface before code lands.
- [ ] Unit tests cover the command builders (`claude`, `codex`) and the binary-not-found guard for both.
- [ ] §11 manual smoke recipe runs end-to-end on a workstation with both `claude` and `codex` installed.

---

## Background

`cafleet` originally supported only `claude` (Claude Code) as the coding-agent binary that runs inside each member's tmux pane. Two prior design cycles touched multi-backend support:

| Design | Outcome |
|---|---|
| `0000018` (Complete, superseded) | Introduced a `CodingAgentConfig` abstraction with `CLAUDE` / `CODEX` configs and a `--coding-agent` flag. |
| `0000034 §15` (Approved, Complete) | Removed `codex` entirely. Reason: the bash-via-Director enforcement protocol assumed a `--disallowedTools` flag that `codex` did not ship at the time. |
| `0000041 §B` (Complete) | Inlined `CodingAgentConfig` into `cli.py`. The `coding_agent.py` module was deleted. The codebase now hardcodes `claude`. |

Two facts have changed since `0000034`:

1. The user has decided that the original deprecation reason — bash-via-Director enforcement asymmetry — no longer blocks adoption. cafleet members already run with the Bash tool enabled (see `skills/cafleet/SKILL.md`). The bash-via-Director protocol is the fallback for harness-deny-listed commands (e.g. `git push`), not a Tool-permission asymmetry. Codex's lack of a `--disallowedTools` analog is acceptable.
2. The user has chosen a workspace-scoped auto-approval combo for codex — `codex --ask-for-approval never --sandbox workspace-write` — which is the codex equivalent of Claude Code's `--permission-mode dontAsk` (per <https://developers.openai.com/codex/agent-approvals-security>). Codex CLI also accepts the leading-`!` shell shortcut, so `cafleet member exec` and the `tmux.send_bash_command` keystroke recipe work without modification.

This redesign is a clean re-introduction tailored to the codebase as it stands today, **not** a restoration of the `0000018` `CodingAgentConfig` abstraction or the `0000034 §13` restoration plan. There is no `CodingAgentConfig` class, no module-level config registry, no "multi-runner" framing in user-facing docs. The added surface is a single CLI flag plus two thin helpers.

---

## Specification

### 1. Backend selection — when and where

| Choice point | Flag | Default | Effect |
|---|---|---|---|
| `cafleet session create` | `--coding-agent {claude,codex}` | `claude` | Sets `placement.coding_agent` for the root Director. Replaces the hardcoded `_ROOT_DIRECTOR_CODING_AGENT = "unknown"` constant in `cafleet/broker.py`. |
| `cafleet member create` | `--coding-agent {claude,codex}` | `claude` | Sets `placement.coding_agent` for the new member and selects the spawn-command builder. |

- `--coding-agent` is validated via `click.Choice(["claude", "codex"])` at the CLI layer.
- Click `--help` text on both subcommands: `Coding-agent binary to spawn / declare for the placement (default: claude).`
- The placement column remains free-text `String` at the DB layer (`cafleet/db/models.py:60-62`); the enum is enforced only at input.
- There is **no** environment-variable fallback (e.g. `CAFLEET_DEFAULT_CODING_AGENT`). cafleet's literal-flag philosophy (every invocation that touches agents/messages must carry literal flags) is incompatible with implicit env-var defaults that change behavior. The default is hardcoded at the `click.Option` level.
- Mixed-backend teams are allowed: a Director may spawn `claude` and `codex` members side-by-side in one session with no broker-level restrictions.
- **Semantics differ between subcommands.** For `member create`, `--coding-agent` both selects the spawn-command builder AND is recorded as placement metadata — cafleet itself spawns the member, so the flag drives the actual binary launched. For `session create`, the flag is **operator-declared** metadata only: cafleet does not spawn the root Director and cannot auto-detect what is already running in the calling pane, so the operator declares which binary they are running. Supplying the wrong value at `session create` does not break anything functionally — it simply mislabels the placement row.

### 2. Spawn-command shape per backend

Both backends spawn via `tmux.split_window(... command=<list[str]>)`. The list is built by a per-backend helper; the rest of `member_create` is unchanged.

| Backend | Spawn command (list form) | Notes |
|---|---|---|
| `claude` | `["claude", "--permission-mode", "dontAsk", "--name", <display_name>, <prompt>]` | Unchanged from today (`cafleet/cli.py:36-43`). |
| `codex`  | `["codex", "--ask-for-approval", "never", "--sandbox", "workspace-write", <prompt>]` | No `--name` analog exists in codex — pane title plumbing is intentionally skipped (see §3). |

Helpers:

```python
_CLAUDE_BINARY = "claude"
_CODEX_BINARY = "codex"

def _build_claude_command(prompt: str, *, display_name: str) -> list[str]:
    return [_CLAUDE_BINARY, "--permission-mode", "dontAsk", "--name", display_name, prompt]

def _build_codex_command(prompt: str) -> list[str]:
    return [_CODEX_BINARY, "--ask-for-approval", "never", "--sandbox", "workspace-write", prompt]

def _ensure_coding_agent_available(binary_name: str) -> None:
    if shutil.which(binary_name) is None:
        raise RuntimeError(f"binary {binary_name} not found on PATH")
```

`_ensure_claude_available` is removed. The single generic helper covers both backends and any future addition.

### 3. Pane-title asymmetry (intentional non-goal)

`claude --name <display_name>` sets the tmux pane title via Claude Code's internal `set_pane_title` call. `codex` has no equivalent flag.

- Codex panes display whatever default title `codex` emits (typically the binary name).
- Operators locate a specific member's pane via `cafleet member list --agent-id <director-agent-id>` (the `pane_id` column is ground truth — this is already documented in `skills/cafleet/SKILL.md` § Member Create).
- For mixed-backend teams in particular, title-based pane scanning is not reliable since codex panes do not carry the member's name. `cafleet member list` (text or `--json`) is the canonical pane-discovery surface for both homogeneous and mixed teams.
- `cafleet` does **not** synthesize a pane title for codex via `tmux select-pane -T <name>`. Adding it would mean a second tmux call inside `member_create`, and the existing pane-id-based discovery flow already works.
- This asymmetry is recorded once in `docs/codex-members.md` and once in the new `skills/cafleet/SKILL.md` § For codex members.

### 4. Prompt template (backend-neutral)

The prompt template is shared. Today's `_CLAUDE_PROMPT_TEMPLATE` is renamed `_MEMBER_PROMPT_TEMPLATE` and its `Load Skill(cafleet).` line is replaced with backend-neutral phrasing — Claude Code reads the natural-language instruction and uses its `Skill()` tool; Codex reads it and consults the canonical doc file shipped at `docs/codex-members.md`.

```python
_MEMBER_PROMPT_TEMPLATE = (
    "Load the 'cafleet' skill (Claude Code: via the Skill tool; Codex: read "
    "docs/codex-members.md in the cafleet repo). Your session_id is {session_id} "
    "and your agent_id is {agent_id}.\n"
    "You are a member of the team led by {director_name} ({director_agent_id}).\n"
    "Wait for instructions via "
    "`cafleet --session-id {session_id} message poll --agent-id {agent_id}`.\n"
    "Your harness runs in workspace-scoped auto-approve mode — your Bash tool is\n"
    "enabled and routine permission prompts auto-resolve, so call cafleet (and any\n"
    "other shell command) directly via the Bash tool."
)
```

The `str.format()` call sites in `_resolve_prompt` are unchanged. The four substituted placeholders (`session_id`, `agent_id`, `director_name`, `director_agent_id`) are still injected by `cafleet.cli._resolve_prompt`.

### 5. `member exec` and `member send-input` are backend-neutral

| Surface | Behavior with `claude` | Behavior with `codex` | Notes |
|---|---|---|---|
| `cafleet member exec <cmd>` | Keystrokes `! <cmd>` + Enter; Claude Code's `!` shortcut runs the command. | Keystrokes `! <cmd>` + Enter; Codex CLI also routes leading `!` to its shell shortcut. | No change to `tmux.send_bash_command`. |
| `cafleet member send-input --freetext <text>` | Rejects `!`-prefixed text. | Rejects `!`-prefixed text. | The guard prevents AskUserQuestion `--freetext` from smuggling a shell-shortcut into either backend. |
| `cafleet member ping` | Keystrokes `cafleet ... message poll ...` + Enter. | Same. | Backend-agnostic. |

The `--freetext`-rejects-`!` help text in `cafleet/cli.py:1001-1002` is softened from "Claude Code's shell-execution shortcut" to "the coding agent's shell-execution shortcut" so the message is accurate for both backends. The validation rule itself is unchanged.

### 6. Non-Goals

- **No bash-disable parity.** Codex has no `--disallowedTools` analog. cafleet members already run with the Bash tool enabled by default; the bash-via-Director protocol is the fallback for harness-deny-listed destructive commands (e.g. `git push`), not a Tool-permission gate. The redesign explicitly does **not** attempt to disable bash on either backend.
- **No `CodingAgentConfig` abstraction.** A per-binary helper function pair (`_build_claude_command` / `_build_codex_command` plus a shared `_ensure_coding_agent_available`) is the entire abstraction surface. No registry, no plugin interface, no class hierarchy.
- **No env-var override.** No `CAFLEET_DEFAULT_CODING_AGENT` (or similar). The default is hardcoded at `click.Option` level for both `session create` and `member create`.
- **No pane-title parity for codex.** Operators discover panes via `cafleet member list`, not via tmux pane titles.
- **No new schema migration.** The existing `placement.coding_agent` column already accepts free-text values; `server_default='claude'` is retained to keep the no-flag path correct.
- **No "multi-runner" / "backend selector" framing.** Per `~/.claude/rules/removal.md`, user-facing docs describe the dual-backend surface as "supported coding agents: `claude` and `codex`" — not as a pluggable runner abstraction.

### 7. Data model

No schema changes. The `agent_placements.coding_agent` column already exists with `server_default='claude'` and accepts arbitrary strings. The CLI's `click.Choice` validation is the only enforcement of the `{claude, codex}` enum.

| Aspect | Value | Rationale |
|---|---|---|
| Column type | `String` (unchanged) | Free-text at DB; CLI enforces the enum. New backends can land without a migration. |
| `server_default` | `'claude'` (unchanged) | Pre-existing member rows that took the column default continue to read `'claude'`; pre-existing root-Director rows (inserted with the now-removed `_ROOT_DIRECTOR_CODING_AGENT = "unknown"` literal) continue to read `'unknown'`. No backfill happens; that is intentional — the value is metadata, not a runtime selector for already-spawned panes. The CLI default is also `'claude'`, so the column default is never relied on at insert time after this change — it remains as a backstop. |
| New column? | No | All required state already fits in the existing column. |
| Alembic migration? | None | No DDL changes. |

### 8. CLI surface (delta only)

```text
cafleet [--session-id <uuid>] [--json] session create [--label <str>] [--coding-agent {claude,codex}]
cafleet --session-id <uuid> [--json] member create --agent-id <director-uuid> --name <str> --description <str> \
                                                  [--coding-agent {claude,codex}] [-- <prompt>]
```

Exit codes and existing flags are unchanged. `--coding-agent` is additive on both subcommands.

### 9. JSON output deltas

`cafleet session create --json` now reflects the chosen backend:

```json
{
  "session_id": "...",
  "director": {
    "placement": {
      "coding_agent": "codex"
    }
  }
}
```

The string `"unknown"` no longer appears for newly-created sessions — `--coding-agent` always supplies a value (`"claude"` by default). Pre-existing sessions on disk retain whatever value they had.

`cafleet member create --json` and `cafleet member list --json` already include `placement.coding_agent`; the value emits `"codex"` for codex members with no other shape change.

### 10. Error messages

| Trigger | Exit | Message |
|---|---|---|
| `--coding-agent codex` and `codex` not on PATH | 1 | `Error: binary codex not found on PATH` |
| `--coding-agent claude` and `claude` not on PATH | 1 | `Error: binary claude not found on PATH` |
| `--coding-agent foo` (unrecognized) | 2 | Click built-in: `Error: Invalid value for '--coding-agent': 'foo' is not one of 'claude', 'codex'.` |
| `member send-input --freetext '! pwd'` (any backend) | 2 | `Error: --freetext may not start with '!' — that triggers the coding agent's shell-execution shortcut. Use 'cafleet member exec' for shell dispatch instead.` |

### 11. Verification recipe (manual smoke test)

Gated on local install of both `claude` and `codex` binaries. Run from inside a tmux session:

```bash
cafleet session create --label codex-smoke --coding-agent claude
# Capture: SESSION=<uuid>, DIRECTOR=<uuid> from the output.

cafleet --session-id $SESSION member create --agent-id $DIRECTOR \
  --name Claude-Smoke --description "claude smoke member" --coding-agent claude
cafleet --session-id $SESSION member create --agent-id $DIRECTOR \
  --name Codex-Smoke --description "codex smoke member" --coding-agent codex

cafleet --session-id $SESSION member list --agent-id $DIRECTOR
# Expect: two rows, backend column shows 'claude' and 'codex' respectively.

cafleet --session-id $SESSION message send --agent-id $DIRECTOR \
  --to <codex-member-id> --text "ping"
# Expect: codex pane receives the poll trigger and the member ack-loops correctly.

cafleet --session-id $SESSION member exec --agent-id $DIRECTOR \
  --member-id <codex-member-id> "git status --short"
# Expect: '! git status --short' lands in the codex pane and the command runs.

cafleet --session-id $SESSION member delete --agent-id $DIRECTOR --member-id <codex-member-id>
cafleet --session-id $SESSION member delete --agent-id $DIRECTOR --member-id <claude-member-id>
cafleet session delete $SESSION
```

This recipe lives in `docs/codex-members.md` § Verification and is referenced from `ARCHITECTURE.md` § Coding agents. It is not part of the automated test suite.

### 12. Codex CLI version requirement

The redesign assumes a `codex` CLI version that supports `--ask-for-approval` and `--sandbox`. The exact minimum version is recorded as **"latest as of 2026-05"** in `docs/codex-members.md` and in the binary-not-found-path error guidance; the Implementer fills in the precise version string at execute time after running `codex --version` against a workstation install. `docs/codex-members.md` also includes a pointer to the upstream codex install instructions (<https://developers.openai.com/codex/>) so operators can find and install the binary; the Implementer verifies the URL is still canonical at execute time.

---

## Implementation

> Documentation precedes code per `.claude/rules/design-doc-numbering.md`.
> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`. Check the box and stamp the timestamp in the same edit.

### Step 1: Documentation

- [x] Update `ARCHITECTURE.md` — add a "Coding agents" subsection that lists `claude` and `codex` as the two supported binaries, summarizes spawn flags, points at `docs/codex-members.md`, and notes the pane-title and bash-disable asymmetries as known non-goals. <!-- completed: 2026-05-03T17:30 -->
- [x] Create `docs/codex-members.md` — operational doc for codex members. Sections: Overview; Spawn flags (`--ask-for-approval never --sandbox workspace-write`); cafleet usage from inside a codex pane (Bash-tool driven, no `Skill()` tool — read this file directly); the `!` shell-shortcut convention used by `cafleet member exec`; pane-title asymmetry note; required `codex` CLI version ("latest as of 2026-05" — exact version filled in at execute time); Verification recipe (§11 above). <!-- completed: 2026-05-03T17:32 -->
- [x] Update `docs/spec/cli-options.md` (if present) — add `--coding-agent` rows for `session create` and `member create`. Drop any stale "claude-only" wording; do **not** add a "Removed flags" or "Deprecated" callout per `~/.claude/rules/removal.md`. <!-- completed: 2026-05-03T17:38 -->
- [x] Update `README.md` to match `ARCHITECTURE.md` and `docs/`. Mention `cafleet session create --coding-agent {claude,codex}` and `cafleet member create --coding-agent {claude,codex}` in the CLI quick reference; mention codex support in the feature list. Use the `/update-readme` skill if the change surface is large. <!-- completed: 2026-05-03T17:40 -->
- [x] Update `skills/cafleet/SKILL.md` — add a "For codex members" section (Director-side orientation; how to read the placement.coding_agent column for mixed teams; pointer to `docs/codex-members.md`). Update the `member create` example block to show the new `--coding-agent` flag. Update the `session create` example block to show the new `--coding-agent` flag. Update the `member send-input --freetext` "no leading `!`" rationale wording to "the coding agent's shell-execution shortcut" so it reads correctly for both backends. <!-- completed: 2026-05-03T17:48 -->
- [x] Update every other affected `skills/*/SKILL.md` — search for literal mentions of `claude` as the coding agent and update to neutral phrasing where the surrounding sentence is backend-agnostic (skill list at minimum: `skills/cafleet-monitoring/SKILL.md`, `skills/cafleet/roles/member.md`, `skills/cafleet/roles/director.md`). Do NOT introduce a "supports multiple runners" callout. <!-- completed: 2026-05-03T17:52 -->
- [x] Update `cafleet/CLAUDE.md` and `cafleet/.claude/CLAUDE.md` design-doc index lines (add the new `0000046-codex-coding-agent/design-doc.md` entry once status reaches Approved). <!-- completed: 2026-05-03T18:00 -->

### Step 2: Tests (TDD — write first)

> These tests are written first and are expected to fail until Steps 3-5 land. Each test references the symbol/CLI surface it covers; the implementation steps satisfy them. Step 6 (Verification) is the green-bar checkpoint.

- [x] Add unit tests for `_build_claude_command` and `_build_codex_command` asserting the exact list shape (binary name, flags, ordering, prompt placement). <!-- completed: 2026-05-03T17:30 -->
- [x] Add a unit test for `_ensure_coding_agent_available` that monkeypatches `shutil.which` to return `None` and asserts the `RuntimeError` message contains `binary <name> not found on PATH` for both backends (parametrized over `["claude", "codex"]`). <!-- completed: 2026-05-03T17:30 -->
- [x] Add a CLI-level test for `cafleet member create --coding-agent codex` that monkeypatches `shutil.which` to fake codex presence and asserts the spawn-command list passed to `tmux.split_window` matches the codex shape. <!-- completed: 2026-05-03T17:30 -->
- [x] Add a CLI-level test for `cafleet session create --coding-agent codex` that asserts the resulting `placement.coding_agent` is `"codex"` (no real codex binary needed — `session create` does not spawn). <!-- completed: 2026-05-03T17:30 -->
- [x] Add a CLI-level test that `cafleet member send-input --freetext '! pwd'` is rejected with the softened error wording, regardless of backend (member is fixture-injected). <!-- completed: 2026-05-03T17:30 -->
- [x] No end-to-end member-create test with a real codex binary. The integration shape is identical to claude's; the §11 manual smoke recipe is the verification path. <!-- completed: 2026-05-03T17:30 -->
- [x] Update existing broker test fixtures and any direct `broker.create_session(...)` callers to pass the new `coding_agent=` kwarg. The signature change in Step 5 will otherwise break test collection. <!-- completed: 2026-05-03T17:30 -->
- [x] Update existing tests that import or assert on `_CLAUDE_PROMPT_TEMPLATE` (now `_MEMBER_PROMPT_TEMPLATE`) and on the `Load Skill(cafleet).` literal substring. Rename symbol references and update body assertions to match the new backend-neutral phrasing in §4. <!-- completed: 2026-05-03T17:30 -->

### Step 3: CLI surface

- [ ] Add `--coding-agent` (Click `Choice(["claude", "codex"])`, default `"claude"`) to the `session create` subcommand in `cafleet/src/cafleet/cli.py`. Plumb the value through to `broker.create_session(...)` via a new `coding_agent: str` keyword argument. <!-- completed: -->
- [ ] Add `--coding-agent` (Click `Choice(["claude", "codex"])`, default `"claude"`) to the `member create` subcommand. Branch on the value to select the spawn-command builder; pass the value through to the placement insert. <!-- completed: -->
- [ ] Replace the hardcoded `placement={"coding_agent": _CLAUDE_BINARY}` literal at `cafleet/src/cafleet/cli.py:700` with the value from `--coding-agent`. <!-- completed: -->
- [ ] Soften the `member send-input --freetext` `!`-rejection error message from "Claude Code's shell-execution shortcut" to "the coding agent's shell-execution shortcut". Validation logic unchanged. <!-- completed: -->

### Step 4: Helpers

- [ ] Add `_CODEX_BINARY = "codex"` alongside `_CLAUDE_BINARY = "claude"` in `cafleet/src/cafleet/cli.py`. <!-- completed: -->
- [ ] Add `_build_codex_command(prompt: str) -> list[str]` returning `[_CODEX_BINARY, "--ask-for-approval", "never", "--sandbox", "workspace-write", prompt]`. <!-- completed: -->
- [ ] Replace `_ensure_claude_available()` with `_ensure_coding_agent_available(binary_name: str)` that raises `RuntimeError(f"binary {binary_name} not found on PATH")` when `shutil.which(binary_name)` is `None`. Update the single call site in `member_create` to pass the chosen backend's binary name. <!-- completed: -->
- [ ] Rename `_CLAUDE_PROMPT_TEMPLATE` → `_MEMBER_PROMPT_TEMPLATE` and replace `"Load Skill(cafleet)."` with the backend-neutral phrasing in §4. The substitution placeholder set is unchanged. <!-- completed: -->

### Step 5: Broker

- [ ] Drop `_ROOT_DIRECTOR_CODING_AGENT = "unknown"` from `cafleet/src/cafleet/broker.py:17`. <!-- completed: -->
- [ ] Extend `broker.create_session(...)` to accept a `coding_agent: str` keyword (no default at the broker layer — the CLI is the only caller and always supplies it). Use the value when building `director_placement["coding_agent"]`. <!-- completed: -->

### Step 6: Verification

- [ ] Run `mise //cafleet:test`. All tests pass. <!-- completed: -->
- [ ] Run `mise //cafleet:lint`. Clean. <!-- completed: -->
- [ ] Run `mise //cafleet:typecheck`. Clean. <!-- completed: -->
- [ ] Manually run the §11 smoke recipe on a workstation with both `claude` and `codex` installed. Capture the output and attach to the PR. <!-- completed: -->

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-05-03 | Initial draft. |
| 2026-05-03 | Reviewer round 1: corrected task count (0/27 → 0/29), reworded §7 `server_default` rationale, added Click `--help` text in §1, added root-Director vs member semantic split in §1, added pane-discovery reinforcement for mixed teams in §3, added codex install-instructions pointer in §12, added two Step 5 tasks for broker test-fixture and prompt-template-symbol/body update. |
| 2026-05-03 | User comment (TDD): moved Tests step from Step 5 to Step 2; renumbered Steps 2-5 accordingly. Task count unchanged at 0/29. |
| 2026-05-03 | User approved. Status: Draft -> Approved. |
