---
icon: lucide/cpu
---

# Coding agents

cafleet supports three coding-agent binaries inside member panes: `claude`
(Claude Code), `codex` (OpenAI Codex CLI), and `opencode` (opencode.ai). The
backend is selected at fleet-create and member-create time via
`--coding-agent {claude,codex,opencode}`; the default is `claude`.

## Backend resolution

| Backend | Spawn command | Notes |
|---|---|---|
| `claude` | `claude --permission-mode dontAsk --name <member-name> <prompt>` | Pane title derives from `--name` via Claude Code's terminal-title escape. |
| `codex`  | `codex --ask-for-approval never --sandbox workspace-write <prompt>` | No `--name` analog; pane title is whatever `codex` emits by default. Operators locate panes via `cafleet member list`. |
| `opencode` | `opencode --agent cafleet --prompt <prompt>` | No `--name` analog. Long-lived TUI bound to the `cafleet` agent definition at `~/.opencode/agents/cafleet.md`, written on first spawn if absent. Safety floor matches Claude Code's `dontAsk` posture (catch-all-allow + specific-deny ruleset enforced by opencode's own permission system) — NOT Codex's kernel sandbox. |

All three backends honor a leading-`!` shell shortcut on the coding agent's
input line, so `cafleet member exec` keystrokes `! <command>` + Enter into
any pane shape and the command runs natively. The `--coding-agent` value is
one of `claude` / `codex` / `opencode` and is recorded in the placement's
`coding_agent` column.

Mixed-backend teams are allowed: a single Director may spawn `claude`,
`codex`, and `opencode` members in the same fleet with no broker-level
differences.

For `cafleet fleet create`, the `--coding-agent` flag is operator-declared
metadata only — cafleet does not spawn the root Director's coding-agent
process and cannot auto-detect what is already running, so the operator
declares which binary is in the pane. For `cafleet member create`, the flag
both selects which backend is spawned AND is recorded as placement metadata.

## Model selection

`cafleet member create` accepts an optional `--model <string>` that is
forwarded to the spawned backend binary via that binary's `--model` flag
(e.g. `--model sonnet` for `claude`, `--model gpt-5.4-mini` for `codex`,
`--model anthropic/claude-sonnet-4-6` or `--model opencode/big-pickle`
for `opencode`). When the flag is
omitted, no model tokens are emitted and each binary uses its own default
model.

Validation is per-backend: `claude` and `codex` pass any string through
verbatim — the binary itself rejects unknown models, so newly released
models work without a cafleet release. `opencode` requires the
`<provider-id>/<model-id>` format and rejects anything else at create time
with exit 2, before any agent registration or tmux side effect.

The value is spawn-time only: it is NOT recorded in `agent_placements`,
requires no migration, and does not appear in `cafleet member list`
output.

## Known asymmetries (intentional non-goals)

- **Pane title.** Only the `claude` spawn argv carries `--name`, so `codex`
  and `opencode` panes do not display the member name in their pane title.
  Use `cafleet member list` to find a specific member's pane id; the
  `pane_id` column is ground truth for all three backends.
- **Bash-disable parity.** Neither codex nor opencode has a
  `--disallowedTools` analog, but this does not affect the routing protocol —
  see [Bash routing](bash-routing.md).
- **Sandbox isolation.** Only `codex` provides OS-level (kernel-enforced)
  isolation via `--sandbox workspace-write`. Both `claude` and `opencode`
  rely on deny-list-only safety floors; operators who need kernel-enforced
  isolation should use the `codex` backend.

Operational details for codex members — including the codex CLI version
pin, install pointer, and verification recipe — live at
[Codex members](../reference/coding-agents/codex.md). The equivalent for
opencode lives at [Opencode members](../reference/coding-agents/opencode.md),
including the preset materialization protocol and the refresh recipe for
upgrading the preset after a CAFleet release.
