# Codex Members

Operational doc for cafleet member panes that run the OpenAI Codex CLI (`codex`) instead of Claude Code (`claude`). cafleet supports both binaries side-by-side; this file covers the codex-specific surface.

For the multi-backend overview and selection rules, see the [Coding agents](../../concepts/coding-agents.md) Concepts page.

## Overview

A codex member is a cafleet member whose `agent_placements.coding_agent` value is `"codex"`. The Director selects the backend at member-create time:

```bash
cafleet --fleet-id <fleet-id> member create --agent-id <director-agent-id> \
  --name Codex-A --description "<one-sentence purpose>" --coding-agent codex
```

The default is `--coding-agent claude`. A single Director may spawn `claude`, `codex`, and `opencode` members in the same fleet — the broker, message lifecycle, and tmux primitives behave identically for all three. See [Opencode members](opencode.md) for opencode-specific operational detail.

## Spawn flags

When `--coding-agent codex` is in effect, cafleet spawns the member pane with:

```
codex --ask-for-approval never --sandbox workspace-write <prompt>
```

- `--ask-for-approval never` disables interactive approval prompts. Combined with `--sandbox workspace-write`, this is the codex equivalent of Claude Code's `--permission-mode dontAsk`: routine permission prompts auto-resolve, the Bash tool is enabled, and the member runs cafleet (and any other shell command) directly.
- `--sandbox workspace-write` confines codex to writing files within the current workspace. See <https://developers.openai.com/codex/agent-approvals-security> for the upstream description of the approval / sandbox combo.

> [!IMPORTANT]
> Codex's `--sandbox workspace-write` blocks writes outside the workspace, including cafleet's default SQLite DB at `~/.local/share/cafleet/cafleet.db`. Operators must add the cafleet DB directory to `sandbox_workspace_write.writable_roots` in any `config.toml` codex reads, such as `~/.codex/config.toml`:
>
> ```toml
> [sandbox_workspace_write]
> writable_roots = ["/home/<you>/.local/share/cafleet"]
> ```
>
> Use the absolute path matching `CAFLEET_DATABASE_URL` or the default XDG location.

Refer to the same upstream page for the canonical write-up of the `--ask-for-approval` and `--sandbox` flags.

## Required codex CLI version

cafleet has been validated against `codex-cli 0.128.0`. Earlier versions may not accept the `--ask-for-approval` / `--sandbox` flags in the form cafleet uses; in that case the spawn will fail or the resulting pane will refuse non-interactive operation.

If `codex --version` reports an older version, upgrade per the upstream install instructions at <https://developers.openai.com/codex/>.

If the `codex` binary is not on `PATH`, `cafleet member create --coding-agent codex` exits 1 with `Error: binary codex not found on PATH`. Install `codex`, confirm with `codex --version`, and retry.

## cafleet usage from inside a codex pane

Codex does not load Claude Code's `Skill()` tool. **You read this file directly** instead — the spawn prompt tells you to. The same cafleet CLI surface works from a codex pane unchanged:

```bash
cafleet --fleet-id <fleet-id> message poll --agent-id <my-agent-id>
cafleet --fleet-id <fleet-id> message send --agent-id <my-agent-id> \
  --to <director-agent-id> --text "..."
cafleet --fleet-id <fleet-id> message ack --agent-id <my-agent-id> --task-id <task-id>
```

Substitute the literal ids handed to you in your spawn prompt. There is no env-var fallback.

For the full broker CLI reference (register, send, broadcast, poll, ack, cancel, show, agent listing, deregister, member commands), see `skills/cafleet/SKILL.md`.

## The `!` shell-shortcut convention

Codex CLI honors a leading-`!` shell shortcut on its input line, which cafleet's bash-via-Director fallback uses — see [Bash routing](../../concepts/bash-routing.md).

## Pane-title asymmetry

Only `claude` sets the pane title to the member name; locate `codex` panes via `cafleet member list` (the `pane_id` column is ground truth) — see [Coding agents](../../concepts/coding-agents.md).

## Verification recipe (manual smoke test)

Gated on local install of both `claude` and `codex` binaries. Run from inside a tmux session:

```bash
cafleet fleet create --label codex-smoke --coding-agent claude
# Capture: FLEET=<id>, DIRECTOR=<id> from the output.

cafleet --fleet-id $FLEET member create --agent-id $DIRECTOR \
  --name Claude-Smoke --description "claude smoke member" --coding-agent claude
cafleet --fleet-id $FLEET member create --agent-id $DIRECTOR \
  --name Codex-Smoke --description "codex smoke member" --coding-agent codex

cafleet --fleet-id $FLEET member list
# Expect: two rows, backend column shows 'claude' and 'codex' respectively.

cafleet --fleet-id $FLEET message send --agent-id $DIRECTOR \
  --to <codex-member-id> --text "ping"
# Expect: codex pane receives the poll trigger and the member ack-loops correctly.

cafleet --fleet-id $FLEET member exec \
  --member-id <codex-member-id> "git status --short"
# Expect: '! git status --short' lands in the codex pane and the command runs.

cafleet --fleet-id $FLEET member delete --member-id <codex-member-id>
cafleet --fleet-id $FLEET member delete --member-id <claude-member-id>
cafleet fleet delete $FLEET
```

This recipe is not part of the automated test suite — it is the manual verification path before shipping changes that touch the codex backend.
