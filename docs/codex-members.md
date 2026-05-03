# Codex Members

Operational doc for cafleet member panes that run the OpenAI Codex CLI (`codex`) instead of Claude Code (`claude`). cafleet supports both binaries side-by-side; this file covers the codex-specific surface.

For the dual-backend overview and selection rules, see [ARCHITECTURE.md](../ARCHITECTURE.md) § Coding Agents.

## Overview

A codex member is a cafleet member whose `agent_placements.coding_agent` value is `"codex"`. The Director selects the backend at member-create time:

```bash
cafleet --session-id <session-id> member create --agent-id <director-agent-id> \
  --name Codex-A --description "<one-sentence purpose>" --coding-agent codex
```

The default is `--coding-agent claude`, so existing invocations are unchanged. A single Director may spawn `claude` and `codex` members in the same session — the broker, message lifecycle, and tmux primitives behave identically for both.

## Spawn flags

When `--coding-agent codex` is in effect, cafleet spawns the member pane with:

```
codex --ask-for-approval never --sandbox workspace-write <prompt>
```

- `--ask-for-approval never` disables interactive approval prompts. Combined with `--sandbox workspace-write`, this is the codex equivalent of Claude Code's `--permission-mode dontAsk`: routine permission prompts auto-resolve, the Bash tool is enabled, and the member runs cafleet (and any other shell command) directly.
- `--sandbox workspace-write` confines codex to writing files within the current workspace. See <https://developers.openai.com/codex/agent-approvals-security> for the upstream description of the approval / sandbox combo.

Refer to the same upstream page for the canonical write-up of the `--ask-for-approval` and `--sandbox` flags.

## Required codex CLI version

cafleet has been validated against `codex-cli 0.128.0`. Earlier versions may not accept the `--ask-for-approval` / `--sandbox` flags in the form cafleet uses; in that case the spawn will fail or the resulting pane will refuse non-interactive operation.

If `codex --version` reports an older version, upgrade per the upstream install instructions at <https://developers.openai.com/codex/>.

If the `codex` binary is not on `PATH`, `cafleet member create --coding-agent codex` exits 1 with `Error: binary codex not found on PATH`. Install `codex`, confirm with `codex --version`, and retry.

## cafleet usage from inside a codex pane

Codex does not load Claude Code's `Skill()` tool. **You read this file directly** instead — the spawn prompt tells you to. The same cafleet CLI surface works from a codex pane unchanged:

```bash
cafleet --session-id <session-id> message poll --agent-id <my-agent-id>
cafleet --session-id <session-id> message send --agent-id <my-agent-id> \
  --to <director-agent-id> --text "..."
cafleet --session-id <session-id> message ack --agent-id <my-agent-id> --task-id <task-id>
```

Substitute the literal UUIDs handed to you in your spawn prompt. There is no env-var fallback.

For the full broker CLI reference (register, send, broadcast, poll, ack, cancel, show, agent listing, deregister, member commands), see [skills/cafleet/SKILL.md](../skills/cafleet/SKILL.md).

## The `!` shell-shortcut convention

Codex CLI honors a leading-`!` shell shortcut on its input line — typing `! <command>` runs the command natively, the same way Claude Code's `!` shortcut works. cafleet's bash-via-Director fallback uses this convention:

- When your Bash tool denies a destructive command, send a plain CAFleet message to your Director asking for the command. The Director dispatches it via `cafleet member exec "<command>"`, which keystrokes `! <command>` + Enter into your pane. The command runs natively; its stdout/stderr lands in your next-turn context.
- You yourself never type `!`-prefixed commands manually. The shortcut is the dispatch mechanism the Director uses on your behalf.

## Pane-title asymmetry

`claude --name <member-name>` sets the tmux pane title via Claude Code's internal title-emit. **`codex` has no equivalent flag.** Codex panes display whatever default title `codex` emits (typically the binary name). This is intentional — pane discovery for both backends goes through `cafleet member list`:

```bash
cafleet --session-id <session-id> member list --agent-id <director-agent-id>
```

The `pane_id` column is ground truth. For mixed-backend teams in particular, do NOT rely on tmux pane titles to find a specific member's pane.

## Verification recipe (manual smoke test)

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

This recipe is not part of the automated test suite — it is the manual verification path before shipping changes that touch the codex backend.
