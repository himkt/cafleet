---
icon: lucide/cpu
---

# Coding agents

cafleet supports three coding-agent binaries inside member panes: `claude`
(Claude Code), `codex` (OpenAI Codex CLI), and `opencode`
(opencode.ai). The backend is selected per member with
`--coding-agent {claude,codex,opencode}`, and mixed-backend teams are allowed:
a single Director may spawn all three in the same fleet with no broker-level
differences. The value is recorded in the placement's `coding_agent` column.

The flag means slightly different things per command:

- `cafleet fleet create --coding-agent` is required, operator-declared
  metadata — cafleet does not spawn the root Director's process and cannot
  auto-detect what is already running in the calling pane, so the operator
  states the backend the Director is actually running on.
- `cafleet member create --coding-agent` both selects which backend is spawned
  and is recorded as placement metadata. When the flag is omitted, the member —
  every role — inherits the spawning Director's placement backend (an explicit
  value still wins).

Each backend is spawned with flags that enable its Bash tool and auto-resolve
routine permission prompts, so members run cafleet (and any shell command)
directly. The per-backend spawn argv, auto-approval posture, and sandbox
trade-offs are specified in
[Coding-agent backends](../spec/coding-agent-backends.md).

## cafleet usage from a member pane

The cafleet CLI works unchanged from any backend pane. Identity reaches a
member through its spawn prompt: `cafleet member create` renders the four
identity placeholders — `{fleet_id}`, `{member_id}`, `{director_member_id}`,
and `{coding_agent}` — to literals, so the member reads its ids as plain text
lines (e.g. `FLEET ID: 1`, `YOUR MEMBER ID: 4`) and passes them explicitly on
every command. The only environment variable forwarded into the pane is
`CAFLEET_DATABASE_URL`. claude panes load the Claude Code skills directly,
while codex and opencode panes read the cafleet skill files by absolute path.
All three honor a leading-`!` shell shortcut on the coding agent's input line,
so `cafleet member exec` works against any pane shape. For the full broker CLI
reference, see [CLI options](../spec/cli-options.md).

## Model selection

`cafleet member create --model <string>` forwards the value to the spawned
backend's own `--model` flag — e.g. `--model opus` for a `claude` member;
omit it and the binary uses its own default. Validation and the accepted
model-name format are per-backend and documented on the backend reference
pages linked above.

## Known asymmetries (intentional non-goals) {#known-asymmetries-intentional-non-goals}

- **Pane title.** Only the `claude` spawn argv carries `--name`, so `codex`
  and `opencode` panes do not display the member name in their pane title.
  The `pane_id` column of `cafleet member list` is ground truth for all three.
- **Sandbox isolation.** Only `codex` provides OS-level (kernel-enforced)
  isolation. `claude` and `opencode` rely on deny-list-only safety floors;
  operators who need kernel-enforced isolation should use the `codex` backend.
