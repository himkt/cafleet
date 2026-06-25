---
icon: lucide/bot
---

# Opencode members

Operational doc for cafleet member panes that run the [opencode](https://opencode.ai) TUI (`opencode`) instead of Claude Code (`claude`) or the OpenAI Codex CLI (`codex`) — this file covers the opencode-specific surface.

For the multi-backend overview and selection rules, see the [Coding agents](../../concepts/coding-agents.md) Concepts page.

## Overview

An opencode member is a cafleet member whose `agent_placements.coding_agent` value is `"opencode"`. The Director selects the backend at `agent spawn` time:

```bash
cafleet agent spawn --fleet-id <fleet-id> --agent-id <director-agent-id> \
  --name Opencode-A --description "<one-sentence purpose>" --coding-agent opencode
```

The default backend, mixed-backend teams, and the shared spawn framing are covered on [Coding agents](../../concepts/coding-agents.md).

The opencode pane runs the bare `opencode` command, which per <https://opencode.ai/docs/cli/> is the documented TUI entry point ("The OpenCode CLI by default starts the TUI when run without any arguments"). The pane stays alive as a long-lived TUI you can scroll, switch to, and observe — matching the operator affordance of `claude` and `codex` panes. The `opencode run` subcommand (the documented headless / scripting entry) is **not** used.

## Spawn flags

When `--coding-agent opencode` is in effect, cafleet spawns the member pane with:

```
opencode --agent cafleet --prompt <prompt>
```

- `--agent cafleet` binds the spawn to the `cafleet` agent definition at `~/.opencode/agents/cafleet.md`. The definition carries an inline permission ruleset (catch-all `"*": "allow"` first, then specific `"deny"` patterns) that resolves every permission check to `allow` or `deny` — nothing falls through to opencode's `ask` state. This is the safety floor.
- `--prompt <prompt>` passes the initial prompt to the TUI. Per <https://opencode.ai/docs/cli/>, bare `opencode` takes `[project]` (an optional project-path positional), NOT `[message..]`. Passing the prompt as a positional would silently misinterpret it as a project path; `--prompt` is the documented flag for the initial-prompt path.
- `--model <m>` is inserted between `--agent cafleet` and `--prompt` when `cafleet agent spawn --model <m>` is supplied (e.g. `opencode --agent cafleet --model anthropic/claude-sonnet-4-6 --prompt <prompt>`). The value must match the `<provider-id>/<model-id>` format: it is split on the **first** `/` into two non-empty segments, so model ids may themselves contain slashes (`a/b/c` is provider `a`, model id `b/c`). Violations fail at create time with exit 2 and `Error: --model for the opencode backend must be '<provider-id>/<model-id>' (got '<value>').`, before any agent registration or tmux side effect. When the flag is omitted, no model tokens are emitted and opencode uses its own default model. Example values (not a catalog cafleet enforces — only the format is validated): `anthropic/claude-sonnet-4-6`, `openai/gpt-5.5`, and entries from opencode's own built-in provider as reported by `opencode models` — `opencode/big-pickle`, `opencode/deepseek-v4-flash-free`, `opencode/mimo-v2.5-free`, `opencode/nemotron-3-ultra-free`, `opencode/north-mini-code-free`.

Explicit non-flags: cafleet does **not** pass `--interactive` (an internal `opencode run` flag) and does **not** pass `--dangerously-skip-permissions` — the bare-`opencode` TUI takes no skip-permissions flag, and the safety floor pre-empts the `ask` state instead.

## The `cafleet` agent preset

`--agent cafleet` binds the member to the `cafleet` agent definition at `~/.opencode/agents/cafleet.md`. cafleet writes this file on first spawn with **skip-if-exists semantics**: it is written once, and subsequent spawns are a cheap no-op. Operators who customize the file (editing the deny-list, adding tools, changing the body) keep their edits — cafleet never overwrites it once it exists.

The definition's ruleset lists a catch-all `"*": "allow"` first, then specific dangerous patterns denied (`bash -c*`, `sudo*`, `rm -rf*`, `curl*`, `git push*`, `**/.env`, and the like) plus a `deny` for categories such as external-directory access, web fetch, and web search. opencode selects the **last** matching rule, so the order — catch-all allow first, specific denies later — is what makes the deny win for dangerous patterns while everything else is allowed. Re-ordering the rules breaks the safety floor.

## Refreshing the preset after a CAFleet upgrade

Skip-if-exists means a CAFleet upgrade that improves the deny-list (e.g. a new wrapper added to the bash deny patterns) does NOT propagate to a machine that already has `~/.opencode/agents/cafleet.md`. To pick up the latest preset:

```bash
rm ~/.opencode/agents/cafleet.md
cafleet agent spawn --fleet-id <fleet-id> --agent-id <director-agent-id> \
  --name Opencode-Refresh --description "preset refresh" --coding-agent opencode
```

The next spawn re-materializes the file from the current preset. If you have local customizations you want to preserve, diff the current file against the new rendered output before deleting.

This trade-off favors respecting user customization over auto-applying upstream changes. The auto-refresh path is intentionally out of scope.

## Required opencode CLI version

cafleet has been validated against `opencode 1.15.5` (the version installed at the time Step 0 empirical verification was completed on 2026-05-19); `1.15.5` is also the minimum supported version. The bare-`opencode` TUI entry, the `--agent` flag, the `--prompt` flag, and the leading-`!` shell shortcut were all verified against this binary.

If `opencode --version` reports an older version that lacks any of these affordances, upgrade per the upstream install instructions at <https://opencode.ai/docs/>.

If the `opencode` binary is not on `PATH`, `cafleet agent spawn --coding-agent opencode` exits 1 with `Error: binary opencode not found on PATH`. Install `opencode`, confirm with `opencode --version`, and retry.

If `~/.opencode/agents/cafleet.md` cannot be written (e.g. `$HOME` is read-only, `~/.opencode/` is owned by another user, or the disk is full), `cafleet agent spawn --coding-agent opencode` surfaces the error via the spawn-failure path and aborts cleanly with no orphaned placement. Resolve the filesystem condition and retry.

## cafleet usage from inside an opencode pane

The cafleet CLI works unchanged from an opencode pane; the usage convention — including reading the cafleet skill files by absolute path — is documented on [Coding agents](../../concepts/coding-agents.md). For the full broker CLI reference, see [CLI options](../../spec/cli-options.md).

## The `!` shell-shortcut convention

Opencode's TUI input box honors the leading-`!` shell shortcut that `cafleet pane exec` uses — see [Coding agents](../../concepts/coding-agents.md).

## Pane-title asymmetry

Only `claude` sets the pane title to the member name; locate `opencode` panes via `cafleet agent list` (the `pane_id` column is ground truth) — see [Coding agents](../../concepts/coding-agents.md).

## Permission-popup recovery posture

In normal operation the TUI never shows a permission popup — every check resolves to `allow` or `deny` without `ask`. If a popup ever appears, it is a regression escape from the safety floor, not a runtime decision-point: the Director MUST escalate to the user and capture pane state via `cafleet pane capture --agent-id <opencode-member>` for diagnosis, then extend the deny-list (a source change + new release + preset refresh). The Director MUST NOT answer the popup as an ad-hoc workaround — that defeats the safety-floor invariant.

## Safety floor caveats

The opencode backend matches Claude Code's `dontAsk` posture: deny-list only, no OS-level sandbox. This is explicit user policy. The deny-list cannot cover everything:

- **MCP-contributed tools.** opencode's MCP integration does not route tool calls through the permission evaluator. cafleet ships zero MCP stanzas, but a user-level opencode config that loads MCP servers will leak them into the cafleet spawn. **Operators MUST NOT add MCP servers to any opencode config their machine loads.**
- **Un-enumerated shell wrappers, in-language eval, and side-channel egress.** A shell wrapper not on the deny-list, a script that calls `os.system(...)`, or network egress that does not go through `bash` all bypass the deny-list. Codex's kernel sandbox WOULD block these; the opencode backend does NOT.

Operators who need kernel-enforced isolation should use the `codex` backend with its `workspace-write` sandbox. This is the documented trade-off, not a bug.

## CAFleet writes one file under `$HOME`

The opencode backend writes exactly one file — `~/.opencode/agents/cafleet.md` — on first spawn. Neither `claude` nor `codex` writes anywhere under `$HOME` from cafleet code. cafleet never writes anywhere else under `$HOME`; if the file is somehow corrupted, delete it and the next spawn re-renders it from the in-source preset.

## Verification recipe (manual smoke test)

Gated on local install of `opencode`. Run from inside a tmux session. The recipe pastes literal ids: fleet `1`, Director `2`, member `4` — your ids will differ.

```bash
rm -f ~/.opencode/agents/cafleet.md

cafleet fleet create --label opencode-smoke --coding-agent claude
# Expect: a '<fleet_id> director=<director_id> admin=<admin_id>' line.
# Note the fleet and Director ids — the steps below use 1 and 2.

cafleet agent spawn --fleet-id 1 --agent-id 2 \
  --name Opencode-Smoke --description "opencode smoke member" --coding-agent opencode
# Expect: ~/.opencode/agents/cafleet.md is materialized with the
# cafleet preset (cat it and verify the JSON frontmatter).

cafleet agent list --fleet-id 1
# Expect: backend column shows 'opencode' for the smoke member.

cafleet message send --fleet-id 1 --agent-id 2 \
  --to 4 --text "ping"
# Expect: the opencode pane receives the inline preview and the member ack-loops.

cafleet pane exec --fleet-id 1 \
  --agent-id 4 "git status --short"
# Expect: '! git status --short' lands in the opencode pane and the
# command runs.

cafleet pane exec --fleet-id 1 \
  --agent-id 4 "curl https://example.com"
# Expect: the deny-list blocks the curl command. If it does NOT, the
# safety floor is broken — STOP and re-run the agent-load smoke from
# the design doc's Step 0 GATE.

cafleet agent deregister --fleet-id 1 --agent-id 4
cafleet fleet delete --fleet-id 1
```

A second `cafleet agent spawn --coding-agent opencode` invocation with the preset file already in place should leave the file unchanged (verify by capturing `stat --format=%Y ~/.opencode/agents/cafleet.md` before and after).

This recipe is not part of the automated test suite — it is the manual verification path before shipping changes that touch the opencode backend.
