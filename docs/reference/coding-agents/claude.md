---
icon: lucide/bot
---

# Claude members

Operational doc for cafleet member panes that run Claude Code (`claude`), the default backend — this file covers the claude-specific surface.

For the multi-backend overview and selection rules, see the [Coding agents](../../concepts/coding-agents.md) Concepts page.

## Overview

A claude member is a cafleet member whose `agent_placements.coding_agent` value is `"claude"`. claude is the **default** backend: `cafleet member create` spawns it when `--coding-agent` is omitted.

```bash
cafleet member create --fleet-id <fleet-id> --agent-id <director-agent-id> \
  --name Claude-A --description "<one-sentence purpose>" \
  --text "<spawn prompt>"
```

## Spawn flags

When `--coding-agent claude` is in effect (the default when `--coding-agent` is omitted), cafleet spawns the member pane with:

```
claude --permission-mode dontAsk --name <member-name> <prompt>
```

- `--permission-mode dontAsk` enables the Bash tool and auto-resolves routine permission prompts, so the member runs cafleet (and any other shell command) directly. This is the posture codex matches with `--ask-for-approval never --sandbox workspace-write` and opencode with its safety-floor ruleset.
- `--name <member-name>` sets the pane title to the member name. claude is the only backend that sets its pane title — codex and opencode panes are located via `cafleet member list` (the `pane_id` column is ground truth).
- `--model <m>` is appended immediately before the prompt when `cafleet member create --model <m>` is supplied (e.g. `claude --permission-mode dontAsk --name <member-name> --model opus <prompt>`). Any string passes through verbatim — the claude binary itself rejects unknown models, so newly released models work without a cafleet release. Example models (not enforced by cafleet): `fable`, `opus`, `sonnet`, `haiku`, `best`, `default`, `opusplan`, `sonnet[1m]`, `opus[1m]`. When the flag is omitted, no model tokens are emitted and claude uses the model set in its configuration.

If the `claude` binary is not on `PATH`, `cafleet member create` exits 1 with `Error: binary claude not found on PATH`. Install Claude Code, confirm with `claude --version`, and retry.

## cafleet usage from inside a claude pane

The cafleet CLI works unchanged from a claude pane; the usage convention — including how claude panes load skills directly — is documented on [Coding agents](../../concepts/coding-agents.md). For the full broker CLI reference, see [CLI options](../../spec/cli-options.md).

## The `!` shell-shortcut convention

Claude Code honors the leading-`!` shell shortcut that `cafleet member exec` uses — see [Coding agents](../../concepts/coding-agents.md).

## Verification recipe (manual smoke test)

Gated on local install of the `claude` binary. Run from inside a tmux or herdr session. The recipe pastes literal ids: fleet `1`, Director `2`, member `4` — your ids will differ.

```bash
cafleet fleet create --name claude-smoke --coding-agent claude
# Expect: a '<fleet_id> director=<director_id> admin=<admin_id>' line.
# Note the fleet and Director ids — the steps below use 1 and 2.

cafleet member create --fleet-id 1 --agent-id 2 \
  --name Claude-Smoke --description "claude smoke member" \
  --text "You are Claude-Smoke. Reply hello when polled."
# Expect: the backend defaults to claude (no --coding-agent needed).

cafleet member list --fleet-id 1 --all
# Expect: the new agent's row, backend column shows 'claude'; the pane title shows the member name.

cafleet message send --fleet-id 1 --agent-id 2 \
  --to 4 --text "ping"
# Expect: the claude pane receives the 2-line inline preview and the member ack-loops correctly.

cafleet member exec --fleet-id 1 \
  --member-id 4 "git status --short"
# Expect: '! git status --short' lands in the claude pane and the command runs.

cafleet member delete --fleet-id 1 --member-id 4
cafleet fleet delete --fleet-id 1
```

This recipe is not part of the automated test suite — it is the manual verification path before shipping changes that touch the claude backend.
