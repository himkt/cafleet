---
icon: lucide/bot
---

# Codex members

Operational doc for cafleet member panes that run the OpenAI Codex CLI (`codex`) instead of Claude Code (`claude`). cafleet supports both binaries side-by-side; this file covers the codex-specific surface.

For the multi-backend overview and selection rules, see the [Coding agents](../../concepts/coding-agents.md) Concepts page.

## Overview

A codex member is a cafleet member whose `agent_placements.coding_agent` value is `"codex"`. The Director selects the backend at member-create time:

```bash
cafleet member create --fleet-id <fleet-id> --agent-id <director-agent-id> \
  --name Codex-A --description "<one-sentence purpose>" --coding-agent codex
```

The default backend, mixed-backend teams, and the shared spawn framing are covered on [Coding agents](../../concepts/coding-agents.md).

## Spawn flags

When `--coding-agent codex` is in effect, cafleet spawns the member pane with:

```
codex --ask-for-approval never --sandbox workspace-write <prompt>
```

- `--ask-for-approval never` disables interactive approval prompts. Combined with `--sandbox workspace-write`, this is the codex equivalent of Claude Code's `--permission-mode dontAsk`: routine permission prompts auto-resolve, the Bash tool is enabled, and the member runs cafleet (and any other shell command) directly.
- `--sandbox workspace-write` confines codex to writing files within the current workspace. See <https://developers.openai.com/codex/agent-approvals-security> for the upstream description of the approval / sandbox combo.
- `--model <m>` is appended immediately before the prompt when `cafleet member create --model <m>` is supplied (e.g. `codex --ask-for-approval never --sandbox workspace-write --model gpt-5.4-mini <prompt>`). Any string passes through verbatim — the codex binary itself rejects unknown models, so newly released models work without a cafleet release. Example models (not enforced by cafleet): `gpt-5.5`, `gpt-5.4`, `gpt-5.4-mini`, `gpt-5.3-codex-spark`. When the flag is omitted, no model tokens are emitted and codex uses the model set in its configuration.

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

The cafleet CLI works unchanged from a codex pane; the usage convention — including reading the cafleet skill files by absolute path — is documented on [Coding agents](../../concepts/coding-agents.md). For the full broker CLI reference, see [CLI options](../../spec/cli-options.md).

## The `!` shell-shortcut convention

Codex honors the leading-`!` shell shortcut that `cafleet member exec` uses — see [Coding agents](../../concepts/coding-agents.md).

## Pane-title asymmetry

Only `claude` sets the pane title to the member name; locate `codex` panes via `cafleet member list` (the `pane_id` column is ground truth) — see [Coding agents](../../concepts/coding-agents.md).

## Verification recipe (manual smoke test)

Gated on local install of both `claude` and `codex` binaries. Run from inside a tmux session. The recipe pastes literal ids: fleet `1`, Director `2`, members `4` (claude) / `5` (codex) — your ids will differ.

```bash
cafleet fleet create --label codex-smoke --coding-agent claude
# Expect: a '<fleet_id> director=<director_id> admin=<admin_id>' line.
# Note the fleet and Director ids — the steps below use 1 and 2.

cafleet member create --fleet-id 1 --agent-id 2 \
  --name Claude-Smoke --description "claude smoke member" --coding-agent claude
cafleet member create --fleet-id 1 --agent-id 2 \
  --name Codex-Smoke --description "codex smoke member" --coding-agent codex

cafleet member list --fleet-id 1
# Expect: two rows, backend column shows 'claude' and 'codex' respectively.

cafleet message send --fleet-id 1 --agent-id 2 \
  --to 5 --text "ping"
# Expect: the codex pane receives the 2-line inline preview and the member ack-loops correctly.

cafleet member exec --fleet-id 1 \
  --member-id 5 "git status --short"
# Expect: '! git status --short' lands in the codex pane and the command runs.

cafleet member delete --fleet-id 1 --member-id 5
cafleet member delete --fleet-id 1 --member-id 4
cafleet fleet delete 1
```

This recipe is not part of the automated test suite — it is the manual verification path before shipping changes that touch the codex backend.
