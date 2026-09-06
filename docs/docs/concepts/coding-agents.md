# Coding agents

cafleet supports three coding-agent binaries inside member panes. The backend
is selected per member with `--coding-agent {claude,codex,opencode}`, and
mixed-backend teams are allowed: a single Director may spawn all three in the
same fleet with no broker-level differences. The value is recorded in the
placement's `coding_agent` column.

| Backend | Product | How the pane loads the cafleet skill |
|---|---|---|
| `claude` | Claude Code | Loads the Claude Code skills directly |
| `codex` | OpenAI Codex CLI | Reads the cafleet skill files by absolute path |
| `opencode` | opencode.ai | Reads the cafleet skill files by absolute path |

Plain `cafleet setup` installs the skills and preset for all three backends
in one run; `--coding-agent` narrows the selection — see
[CLI options § cafleet setup](../spec/cli-options.md#cafleet-setup).

The flag means slightly different things per command:

- `cafleet fleet create --coding-agent` is required, operator-declared
  metadata — cafleet does not spawn the root Director's process and cannot
  auto-detect what is already running in the calling pane, so the operator
  states the backend the Director is actually running on.
- `cafleet member create --coding-agent` both selects which backend is spawned
  and is recorded as placement metadata. When the flag is omitted, the member —
  every role — inherits the spawning Director's placement backend (an explicit
  value still wins).

Each backend is spawned with flags that enable its Bash tool with no runtime
permission prompts. The per-backend spawn argv, shell-command posture, and
sandbox trade-offs are specified in
[Coding-agent backends § Spawn argv](../spec/coding-agent-backends.md#spawn-argv).

## cafleet usage from a member pane

The cafleet CLI works unchanged from any backend pane. Identity reaches a
member through its spawn prompt: `cafleet member create` renders four identity
placeholders to literals, so the member reads its ids as plain text lines
(e.g. `FLEET ID: 1`, `YOUR MEMBER ID: 4`) and passes them explicitly on every
command. Each placeholder and its label line is in
[CLI options § Spawn-prompt substitution](../spec/cli-options.md#spawn-prompt-substitution). The only environment variable forwarded into the pane is
`CAFLEET_DATABASE_URL`.
All three honor a leading-`!` shell shortcut on the coding agent's input line,
so `cafleet member prompt --shell` works against any pane shape. For the full
broker CLI reference, see [CLI options](../spec/cli-options.md).

## One-shot command isolation {#one-shot-command-isolation}

Run every one-shot `cafleet` command as the only command of its shell-tool
invocation, and run a sequence of CAFleet operations as separate shell-tool
calls. This rule is backend-neutral and applies identically to `claude`,
`codex`, and `opencode` panes.

The reason is the pane push channel: a compound invocation — a one-shot
CAFleet command placed beside another command with a newline, `;`, `&&`, a
pipe, or shell `&` — keeps the coding agent's shell tool occupied after the
CAFleet process exits. While the tool is occupied, the pane cannot consume an
inbound inline-preview keystroke, and a notification aimed at that pane can
fail even though the message itself was durably persisted. Isolated
invocations return the pane to the composer between operations, which is what
the push-notification channel depends on.

Leading `NAME=value` environment assignments immediately preceding the
`cafleet` executable are allowed: they set the CAFleet process's environment
without starting another process. Shell redirection does not authorize another
process either; a command that needs a long body should use the positional
argument or `--file <path>` rather than a pipe.

The sole exception is the long-lived `cafleet monitor` process. Its invocation
still contains only that monitor process, but the monitor member hosts it with
its backend-resolved long-lived-execution mechanism — see
[Monitoring](monitoring.md) for the monitor lifecycle.

## Model selection

`cafleet member create --model <string>` forwards the value to the spawned
backend's own `--model` flag — e.g. `--model opus` for a `claude` member;
omit it and the binary uses its own default. Per-backend accepted formats and
create-time validation are in
[Coding-agent backends § Model selection](../spec/coding-agent-backends.md#model-selection).

## Reasoning effort

`cafleet member create --effort <level>` forwards a reasoning-effort level to
the spawned backend binary. Unlike `--model`, the accepted level set is
validated per backend at create time, before any registration or multiplexer
side effect; omit the flag and the binary uses its own default. Per-backend
accepted levels, forwarding forms, and rejection strings are in
[Coding-agent backends § Reasoning effort](../spec/coding-agent-backends.md#reasoning-effort).

## Known asymmetries (intentional non-goals) {#known-asymmetries-intentional-non-goals}

| Dimension | `claude` | `codex` | `opencode` |
|---|---|---|---|
| Reasoning effort | supported | supported | not supported |
| Pane title | supported, via `--name` | not supported | not supported |
| Sandbox isolation | not supported — a deny-list safety floor | supported — OS-level, kernel-enforced | not supported — a deny-by-default bash allowlist |

`--effort` with the `opencode` backend exits 2 with
`opencode does not support reasoning effort.` before any side effect. Because
`codex` and `opencode` panes do not display the member name, the `pane_id`
column of `cafleet member list` is ground truth for all three. Operators who
need kernel-enforced isolation should use the `codex` backend.

## Complete monitor prompts {#complete-monitor-prompts}

Use the [Quickstart monitor prompt](../quickstart.md#raw-cli-walkthrough),
substituting the skill path for your backend. Codex uses
`~/.codex/skills/cafleet`; Opencode uses `~/.config/opencode/skills/cafleet`.
