# Opencode Members

Operational doc for cafleet member panes that run the [opencode](https://opencode.ai) TUI (`opencode`) instead of Claude Code (`claude`) or the OpenAI Codex CLI (`codex`). cafleet supports all three binaries side-by-side; this file covers the opencode-specific surface.

For the multi-backend overview and selection rules, see the [Coding agents](../../concepts/coding-agents.md) Concepts page.

## Overview

An opencode member is a cafleet member whose `agent_placements.coding_agent` value is `"opencode"`. The Director selects the backend at member-create time:

```bash
cafleet --fleet-id <fleet-id> member create --agent-id <director-agent-id> \
  --name Opencode-A --description "<one-sentence purpose>" --coding-agent opencode
```

The default is `--coding-agent claude`. A single Director may spawn `claude`, `codex`, and `opencode` members in the same fleet — the broker, message lifecycle, and tmux primitives behave identically for all three.

The opencode pane runs the bare `opencode` command, which per <https://opencode.ai/docs/cli/> is the documented TUI entry point ("The OpenCode CLI by default starts the TUI when run without any arguments"). The pane stays alive as a long-lived TUI you can scroll, switch to, and observe — matching the operator affordance of `claude` and `codex` panes. The `opencode run` subcommand (the documented headless / scripting entry) is **not** used.

## Spawn flags

When `--coding-agent opencode` is in effect, cafleet spawns the member pane with:

```
opencode --agent cafleet --prompt <prompt>
```

- `--agent cafleet` binds the spawn to the `cafleet` agent definition at `~/.opencode/agents/cafleet.md`. The definition carries an inline permission ruleset (catch-all `"*": "allow"` first, then specific `"deny"` patterns) that resolves every permission check to `allow` or `deny` — nothing falls through to opencode's `ask` state. This is the safety floor.
- `--prompt <prompt>` passes the initial prompt to the TUI. Per <https://opencode.ai/docs/cli/>, bare `opencode` takes `[project]` (an optional project-path positional), NOT `[message..]`. Passing the prompt as a positional would silently misinterpret it as a project path; `--prompt` is the documented flag for the initial-prompt path.

Explicit non-flags: cafleet does **not** pass `--interactive` (an internal `opencode run` flag) and does **not** pass `--dangerously-skip-permissions` — the bare-`opencode` TUI takes no skip-permissions flag, and the safety floor pre-empts the `ask` state instead.

## The `cafleet` agent preset

`--agent cafleet` binds the member to the `cafleet` agent definition at `~/.opencode/agents/cafleet.md`. cafleet writes this file on first spawn with **skip-if-exists semantics**: it is written once, and subsequent spawns are a cheap no-op. Operators who customize the file (editing the deny-list, adding tools, changing the body) keep their edits — cafleet never overwrites it once it exists.

The definition's ruleset lists a catch-all `"*": "allow"` first, then specific dangerous patterns denied (`bash -c*`, `sudo*`, `rm -rf*`, `curl*`, `git push*`, `**/.env`, and the like) plus a `deny` for categories such as external-directory access, web fetch, and web search. opencode selects the **last** matching rule, so the order — catch-all allow first, specific denies later — is what makes the deny win for dangerous patterns while everything else is allowed. Re-ordering the rules breaks the safety floor.

## Refreshing the preset after a CAFleet upgrade

Skip-if-exists means a CAFleet upgrade that improves the deny-list (e.g. a new wrapper added to the bash deny patterns) does NOT propagate to a machine that already has `~/.opencode/agents/cafleet.md`. To pick up the latest preset:

```bash
rm ~/.opencode/agents/cafleet.md
cafleet --fleet-id <fleet-id> member create --agent-id <director-agent-id> \
  --name Opencode-Refresh --description "preset refresh" --coding-agent opencode
```

The next spawn re-materializes the file from the current preset. If you have local customizations you want to preserve, diff the current file against the new rendered output before deleting.

This trade-off favors respecting user customization over auto-applying upstream changes. The auto-refresh path is intentionally out of scope.

## Required opencode CLI version

cafleet has been validated against `opencode 1.15.5` (the version installed at the time Step 0 empirical verification was completed on 2026-05-19); `1.15.5` is also the minimum supported version. The bare-`opencode` TUI entry, the `--agent` flag, the `--prompt` flag, and the leading-`!` shell shortcut were all verified against this binary.

If `opencode --version` reports an older version that lacks any of these affordances, upgrade per the upstream install instructions at <https://opencode.ai/docs/>.

If the `opencode` binary is not on `PATH`, `cafleet member create --coding-agent opencode` exits 1 with `Error: binary opencode not found on PATH`. Install `opencode`, confirm with `opencode --version`, and retry.

If `~/.opencode/agents/cafleet.md` cannot be written (e.g. `$HOME` is read-only, `~/.opencode/` is owned by another user, or the disk is full), `cafleet member create --coding-agent opencode` surfaces the error via the spawn-failure path and aborts cleanly with no orphaned placement. Resolve the filesystem condition and retry.

## cafleet usage from inside an opencode pane

Opencode does not load Claude Code's `Skill()` tool. **You read this file directly** instead — the spawn prompt tells you to. The same cafleet CLI surface works from an opencode pane unchanged:

```bash
cafleet --fleet-id <fleet-id> message poll --agent-id <my-agent-id>
cafleet --fleet-id <fleet-id> message send --agent-id <my-agent-id> \
  --to <director-agent-id> --text "..."
cafleet --fleet-id <fleet-id> message ack --agent-id <my-agent-id> --task-id <task-id>
```

Substitute the literal ids handed to you in your spawn prompt. There is no env-var fallback.

For the full broker CLI reference (register, send, broadcast, poll, ack, cancel, show, agent listing, deregister, member commands), see `skills/cafleet/SKILL.md`.

## The `!` shell-shortcut convention

Opencode's TUI input box honors a leading-`!` shell shortcut, which cafleet's bash-via-Director fallback uses — see [Bash routing](../../concepts/bash-routing.md).

## Pane-title asymmetry

Only `claude` sets the pane title to the member name; locate `opencode` panes via `cafleet member list` (the `pane_id` column is ground truth) — see [Coding agents](../../concepts/coding-agents.md).

## Permission-popup recovery posture

In normal operation the TUI never shows a permission popup — every check resolves to `allow` or `deny` without `ask`. If a popup ever appears, it is a regression escape from the safety floor, not a runtime decision-point: the Director MUST escalate to the user and capture pane state via `cafleet member capture --member-id <opencode-member>` for diagnosis, then extend the deny-list (a source change + new release + preset refresh). The Director MUST NOT answer the popup as an ad-hoc workaround — that defeats the safety-floor invariant.

## Safety floor caveats

The opencode backend matches Claude Code's `dontAsk` posture: deny-list only, no OS-level sandbox. This is explicit user policy. The deny-list cannot cover everything:

- **MCP-contributed tools.** opencode's MCP integration does not route tool calls through the permission evaluator. cafleet ships zero MCP stanzas, but a user-level opencode config that loads MCP servers will leak them into the cafleet spawn. **Operators MUST NOT add MCP servers to any opencode config their machine loads.**
- **Un-enumerated shell wrappers, in-language eval, and side-channel egress.** A shell wrapper not on the deny-list, a script that calls `os.system(...)`, or network egress that does not go through `bash` all bypass the deny-list. Codex's kernel sandbox WOULD block these; the opencode backend does NOT.

Operators who need kernel-enforced isolation should use the `codex` backend with its `workspace-write` sandbox. This is the documented trade-off, not a bug.

## CAFleet writes one file under `$HOME`

The opencode backend writes exactly one file — `~/.opencode/agents/cafleet.md` — on first spawn. Neither `claude` nor `codex` writes anywhere under `$HOME` from cafleet code. cafleet never writes anywhere else under `$HOME`; if the file is somehow corrupted, delete it and the next spawn re-renders it from the in-source preset.

## Verification recipe (manual smoke test)

Gated on local install of `opencode`. Run from inside a tmux session:

```bash
rm -f ~/.opencode/agents/cafleet.md

cafleet fleet create --label opencode-smoke --coding-agent claude
# Capture: FLEET=<id>, DIRECTOR=<id> from the output.

cafleet --fleet-id $FLEET member create --agent-id $DIRECTOR \
  --name Opencode-Smoke --description "opencode smoke member" --coding-agent opencode
# Expect: ~/.opencode/agents/cafleet.md is materialized with the
# cafleet preset (cat it and verify the JSON frontmatter).

cafleet --fleet-id $FLEET member list
# Expect: backend column shows 'opencode' for the smoke member.

cafleet --fleet-id $FLEET message send --agent-id $DIRECTOR \
  --to <opencode-member-id> --text "ping"
# Expect: opencode pane receives the inline preview and the member ack-loops.

cafleet --fleet-id $FLEET member exec \
  --member-id <opencode-member-id> "git status --short"
# Expect: '! git status --short' lands in the opencode pane and the
# command runs.

cafleet --fleet-id $FLEET member exec \
  --member-id <opencode-member-id> "curl https://example.com"
# Expect: the deny-list blocks the curl command. If it does NOT, the
# safety floor is broken — STOP and re-run the agent-load smoke from
# the design doc's Step 0 GATE.

cafleet --fleet-id $FLEET member delete --member-id <opencode-member-id>
cafleet fleet delete $FLEET
```

A second `cafleet member create --coding-agent opencode` invocation with the preset file already in place should leave the file unchanged (verify by capturing `stat --format=%Y ~/.opencode/agents/cafleet.md` before and after).

This recipe is not part of the automated test suite — it is the manual verification path before shipping changes that touch the opencode backend.
