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

Explicit non-flags: cafleet does **not** pass `--interactive` (an internal `opencode run` flag, not a stable public surface) and does **not** pass `--dangerously-skip-permissions` (see below).

## Why we don't pass `--dangerously-skip-permissions`

`--dangerously-skip-permissions` is documented at <https://opencode.ai/docs/cli/> only as a flag of the `opencode run` subcommand, not as a bare-`opencode` flag. Passing it to the bare command would either be rejected by yargs `.strict()` parsing or silently ignored.

Independently, the flag is only consumed inside the non-interactive `execute()` path in opencode's source — the interactive code paths (which the cafleet pane uses) do not subscribe a `permission.asked` event handler that consults the flag. Even when accepted, the flag has no effect in any interactive code path.

The CAFleet pane is operated by a Director (an agent), not a human — the Director cannot drive a focus-based Allow-once / Allow-always / Reject UI reliably. The safety floor therefore pre-empts the `ask` state across the entire permission surface, so the TUI never shows a permission prompt in normal operation. This is the function of the `cafleet` agent definition's catch-all-allow + specific-deny ruleset: every permission check resolves to `allow` (catch-all) or `deny` (specific), and the auto-approve-once handler `--dangerously-skip-permissions` would provide is not needed.

## The `cafleet` agent preset

The `cafleet` agent definition is owned in CAFleet source as a frozen Python dataclass:

- **Module**: `cafleet/src/cafleet/coding_agent/opencode_preset.py`
- **Dataclasses**: `PermissionRuleset` (per-permission rules) and `OpencodeAgentDefinition` (frontmatter + body).
- **Preset constant**: `CAFLEET_AGENT` — the canonical CAFleet-spawned-member definition, with a catch-all `"*": "allow"` first for `bash` / `read` / `edit`, then specific dangerous patterns denied (`bash -c*`, `sudo*`, `rm -rf*`, `curl*`, `git push*`, `**/.env`, etc.), and Action-shorthand `"deny"` for `external_directory`, `webfetch`, `websearch`, `repo_clone`, `question`, `plan_enter`, `plan_exit`.
- **Rendering**: `OpencodeAgentDefinition.to_markdown()` emits a JSON-shaped frontmatter inside the `---` block followed by the markdown body. JSON is a strict subset of YAML 1.2, so opencode's frontmatter parser reads it correctly.

`OpencodeAgent.ensure_available()` calls `materialize_cafleet_agent(CAFLEET_AGENT)` on every spawn, which writes the rendered markdown to `~/.opencode/agents/cafleet.md` with **skip-if-exists semantics**: the file is written once on first spawn, and subsequent spawns are a cheap stat-then-no-op. Operators who customize the file (editing the deny-list, adding tools, changing the body) keep their edits — CAFleet never overwrites the file once it exists.

The catch-all + deny discipline matters because opencode's permission evaluator (`permission/evaluate.ts`) uses `findLast` to select the matching rule, so the order is "catch-all `*` FIRST, specific denies LATER" — the deny wins for dangerous patterns and the catch-all allow covers everything else. Re-ordering the rules breaks the safety floor.

## Refreshing the preset after a CAFleet upgrade

Skip-if-exists means a CAFleet upgrade that improves the deny-list (e.g. a new wrapper added to the bash deny patterns) does NOT propagate to a machine that already has `~/.opencode/agents/cafleet.md`. To pick up the latest preset:

```bash
rm ~/.opencode/agents/cafleet.md
cafleet --fleet-id <fleet-id> member create --agent-id <director-agent-id> \
  --name Opencode-Refresh --description "preset refresh" --coding-agent opencode
```

The next `OpencodeAgent.ensure_available()` call re-materializes the file from the current `CAFLEET_AGENT` constant. If you have local customizations you want to preserve, diff the current file against the new rendered output before deleting.

This trade-off favors respecting user customization over auto-applying upstream changes. The auto-refresh path is intentionally out of scope.

## Required opencode CLI version

cafleet has been validated against `opencode 1.15.5` (the version installed at the time Step 0 empirical verification was completed on 2026-05-19); `1.15.5` is also the minimum supported version. The bare-`opencode` TUI entry, the `--agent` flag, the `--prompt` flag, and the leading-`!` shell shortcut were all verified against this binary.

If `opencode --version` reports an older version that lacks any of these affordances, upgrade per the upstream install instructions at <https://opencode.ai/docs/>.

If the `opencode` binary is not on `PATH`, `cafleet member create --coding-agent opencode` exits 1 with `Error: binary opencode not found on PATH`. Install `opencode`, confirm with `opencode --version`, and retry.

If `~/.opencode/agents/cafleet.md` cannot be written (e.g. `$HOME` is read-only, `~/.opencode/` is owned by another user, or the disk is full), `materialize_cafleet_agent` wraps the underlying `OSError` / `PermissionError` as a `RuntimeError` (chained from the original) so `cafleet member create --coding-agent opencode` surfaces it via the existing spawn-failure path and aborts cleanly with no orphaned placement. Resolve the filesystem condition and retry.

## cafleet usage from inside an opencode pane

Opencode does not load Claude Code's `Skill()` tool. **You read this file directly** instead — the spawn prompt tells you to. The same cafleet CLI surface works from an opencode pane unchanged:

```bash
cafleet --fleet-id <fleet-id> message poll --agent-id <my-agent-id>
cafleet --fleet-id <fleet-id> message send --agent-id <my-agent-id> \
  --to <director-agent-id> --text "..."
cafleet --fleet-id <fleet-id> message ack --agent-id <my-agent-id> --task-id <task-id>
```

Substitute the literal UUIDs handed to you in your spawn prompt. There is no env-var fallback.

For the full broker CLI reference (register, send, broadcast, poll, ack, cancel, show, agent listing, deregister, member commands), see `skills/cafleet/SKILL.md`.

## The `!` shell-shortcut convention

Opencode's TUI input box honors a leading-`!` shell shortcut — typing `! <command>` runs the command natively, the same way Claude Code's and Codex CLI's `!` shortcuts work. cafleet's bash-via-Director fallback uses this convention:

- When your Bash tool denies a destructive command, send a plain CAFleet message to your Director asking for the command. The Director dispatches it via `cafleet member exec "<command>"`, which keystrokes `! <command>` + Enter into your pane. The command runs natively; its stdout/stderr lands in your next-turn context.
- You yourself never type `!`-prefixed commands manually. The shortcut is the dispatch mechanism the Director uses on your behalf.

## Pane-title asymmetry

`claude --name <member-name>` sets the tmux pane title via Claude Code's internal title-emit. **Neither `codex` nor `opencode` has an equivalent flag.** Opencode panes display whatever default title the binary emits. Pane discovery for all three backends goes through `cafleet member list`:

```bash
cafleet --fleet-id <fleet-id> member list --agent-id <director-agent-id>
```

The `pane_id` column is ground truth. For mixed-backend teams in particular, do NOT rely on tmux pane titles to find a specific member's pane.

## Permission-popup recovery posture

In normal operation the TUI never shows a permission popup — the catch-all-allow + specific-deny ruleset means every check resolves without `ask`. If a popup ever appears, it is a regression escape from the safety floor (e.g. opencode added a new tool category in a future release that uses `ask` semantics and the `cafleet` agent definition does not yet cover it), NOT a runtime decision-point.

The Director MUST escalate back to the user, capture pane state via `cafleet --fleet-id <s> member capture --member-id <opencode-member>` for diagnosis, and re-run Step 0-style empirical verification to extend the deny-list (which means updating `CAFLEET_AGENT` in source, shipping a new CAFleet release, and refreshing the preset per the recipe above). The Director MUST NOT wire `send_choice_key` against an opencode placement as an ad-hoc workaround — that defeats the safety floor invariant.

## Safety floor caveats

The opencode backend matches Claude Code's `dontAsk` posture: deny-list only, no OS-level sandbox. This is explicit user policy. The deny-list does NOT cover:

- **MCP-contributed tools.** Opencode's MCP integration does not route tool calls through the permission evaluator. CAFleet ships zero MCP stanzas, but a user-level `~/.config/opencode/opencode.json` that loads MCP servers will leak them into the cafleet spawn. **Operators MUST NOT add MCP servers to any opencode config their machine loads.**
- **Shell wrappers not enumerated in the deny-list.** The wrapper deny list covers `bash -c`, `sh -c`, `zsh -c`, `python -c`, `python3 -c`, `perl -e`, `node -e`, `node --eval`, `ruby -e`, `eval`, `exec`, and `osascript`. Any wrapper not on this list (`fish -c`, `dash -c`, `tclsh`, `lua`, etc.) bypasses the wrapper check. cafleet targets Linux/macOS workstations; the list is sized accordingly.
- **Language-specific eval bypasses.** `python script.py` where `script.py` calls `os.system(...)` passes the wrapper check (it is not `python -c`) and runs whatever the script wants. This is the fundamental limit of any allow-list-of-binaries permission model.
- **Side-channel egress.** DNS lookups, ICMP, kernel-level networking via `/proc`, NTP — none of these go through `bash` and are not gated. Codex's kernel sandbox WOULD block these; CAFleet's opencode backend does NOT.

Operators who need kernel-enforced isolation should use the `codex` backend with its `workspace-write` sandbox. This is the documented trade-off, not a bug.

## CAFleet writes one file under `$HOME`

The opencode backend introduces a new install-footprint behavior compared to `claude` and `codex`: CAFleet writes one file at `~/.opencode/agents/cafleet.md` from `OpencodeAgent.ensure_available()` on first spawn. Neither `claude` nor `codex` writes anywhere under `$HOME` from CAFleet code.

The write is scoped to that single well-known opencode path. CAFleet never writes anywhere else under `$HOME`. Skip-if-exists limits the risk of a malformed write to first spawn — if the file is somehow corrupted, delete it to recover and the next `OpencodeAgent.ensure_available()` call re-renders it from the in-source preset.

## Verification recipe (manual smoke test)

Gated on local install of `opencode`. Run from inside a tmux session:

```bash
rm -f ~/.opencode/agents/cafleet.md

cafleet fleet create --label opencode-smoke --coding-agent claude
# Capture: FLEET=<uuid>, DIRECTOR=<uuid> from the output.

cafleet --fleet-id $FLEET member create --agent-id $DIRECTOR \
  --name Opencode-Smoke --description "opencode smoke member" --coding-agent opencode
# Expect: ~/.opencode/agents/cafleet.md is materialized with the
# CAFLEET_AGENT preset (cat it and verify the JSON frontmatter).

cafleet --fleet-id $FLEET member list --agent-id $DIRECTOR
# Expect: backend column shows 'opencode' for the smoke member.

cafleet --fleet-id $FLEET message send --agent-id $DIRECTOR \
  --to <opencode-member-id> --text "ping"
# Expect: opencode pane receives the inline preview and the member ack-loops.

cafleet --fleet-id $FLEET member exec --agent-id $DIRECTOR \
  --member-id <opencode-member-id> "git status --short"
# Expect: '! git status --short' lands in the opencode pane and the
# command runs.

cafleet --fleet-id $FLEET member exec --agent-id $DIRECTOR \
  --member-id <opencode-member-id> "curl https://example.com"
# Expect: the deny-list blocks the curl command. If it does NOT, the
# safety floor is broken — STOP and re-run the agent-load smoke from
# the design doc's Step 0 GATE.

cafleet --fleet-id $FLEET member delete --agent-id $DIRECTOR --member-id <opencode-member-id>
cafleet fleet delete $FLEET
```

A second `cafleet member create --coding-agent opencode` invocation with the preset file already in place should leave the file unchanged (verify by capturing `stat --format=%Y ~/.opencode/agents/cafleet.md` before and after).

This recipe is not part of the automated test suite — it is the manual verification path before shipping changes that touch the opencode backend.
