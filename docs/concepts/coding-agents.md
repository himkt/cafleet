---
icon: lucide/cpu
---

# Coding agents

cafleet supports three coding-agent binaries inside member panes: `claude`
(Claude Code), `codex` (OpenAI Codex CLI), and `opencode` (opencode.ai). The
backend is selected at `fleet create` and `member create` time via
`--coding-agent {claude,codex,opencode}`; the default is `claude`.

## Backend resolution

| Backend | Spawn command | Notes |
|---|---|---|
| `claude` | `claude --permission-mode dontAsk --name <member-name> <prompt>` | Pane title derives from `--name` via Claude Code's terminal-title escape. |
| `codex`  | `codex --ask-for-approval never --sandbox workspace-write <prompt>` | No `--name` analog; pane title is whatever `codex` emits by default. Operators locate panes via `cafleet member list`. |
| `opencode` | `opencode --agent cafleet --prompt <prompt>` | No `--name` analog. Long-lived TUI bound to the `cafleet` agent definition at `~/.opencode/agents/cafleet.md`, written on first spawn if absent. Safety floor matches Claude Code's `dontAsk` posture (catch-all-allow + specific-deny ruleset enforced by opencode's own permission system) — NOT Codex's kernel sandbox. |

The `--coding-agent` value (`claude` / `codex` / `opencode`) is recorded in
the placement's `coding_agent` column. Mixed-backend teams are allowed: a
single Director may spawn all three in the same fleet with no broker-level
differences.

## cafleet usage from a member pane

The cafleet CLI works unchanged from any backend pane. Identity reaches a
member through its spawn prompt: `cafleet member create` runs `str.format`
over the resolved prompt, rendering the four identity placeholders —
`{fleet_id}`, `{agent_id}` (the spawned member's own newly-allocated id),
`{director_agent_id}`, and `{coding_agent}` — to literals, so the member reads
its ids as plain text lines in its prompt (e.g. `FLEET ID: 1`,
`YOUR AGENT ID: 4`). The only environment variable forwarded into the pane is
`CAFLEET_DATABASE_URL`. The member passes its ids explicitly on every command:
a poll is `cafleet message poll --fleet-id 1 --agent-id 4` and a
self-attributed send is
`cafleet message send --fleet-id 1 --agent-id 4 --to <director-agent-id> --text ...`.
claude panes load the Claude Code skills
directly, while codex and opencode panes read the cafleet skill files by
absolute path. All three honor a leading-`!` shell shortcut on the coding
agent's input line, so `cafleet member exec` keystrokes `! <command>` + Enter
into any pane shape and the command runs natively. For the full broker CLI
reference, see [CLI options](../spec/cli-options.md).

## Backend selection by `--coding-agent`

For `cafleet fleet create`, the `--coding-agent` flag is operator-declared
metadata only — cafleet does not spawn the root Director's coding-agent
process and cannot auto-detect what is already running, so the operator
declares which binary is in the pane. For `cafleet member create`, the flag
both selects which backend is spawned AND is recorded as placement metadata.
For `--role monitor`, omitting `--coding-agent` inherits the spawning
Director's backend (read from its placement row) so the monitoring member
runs on — and reads the overlay of — the same backend it watches; an
explicit `--coding-agent` still wins. Ordinary members keep the `claude`
default when the flag is omitted.

## Model selection

`cafleet member create --model <string>` forwards the value to the spawned
backend's own `--model` flag (e.g. `--model sonnet` for `claude`,
`--model gpt-5.4-mini` for `codex`, `--model anthropic/claude-sonnet-4-6` for
`opencode`); omit it and each binary uses its own default. Validation is
per-backend — `claude` and `codex` pass any string through verbatim (the
binary rejects unknown models), while `opencode` requires the
`<provider-id>/<model-id>` format and rejects anything else at create time.
The value is spawn-time only: it is not recorded in `agent_placements` and does
not appear in `cafleet member list`. The exhaustive `--model` flag detail is in
[CLI options](../spec/cli-options.md); per-backend model examples live on the
backend reference pages linked below.

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

Operational details for each backend live on its reference page:
[Claude members](../reference/coding-agents/claude.md) (the default backend,
its spawn flags, and model examples), [Codex members](../reference/coding-agents/codex.md)
(the codex CLI version pin, install pointer, and verification recipe), and
[Opencode members](../reference/coding-agents/opencode.md) (the preset
materialization protocol and the refresh recipe for upgrading the preset after
a CAFleet release).
