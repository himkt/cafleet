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

## Workspace configuration {#workspace-configuration}

CAFleet is designed to run inside a coding agent without per-command
permission prompts. Each backend has a different config file and permission
system:

| Backend | Config file | Manual configuration | Installed by `cafleet setup` | Reference |
|---|---|---|---|---|
| `claude` (Claude Code) | `~/.claude/settings.json` | The `permissions.allow` / `permissions.ask` entries below | The skills | The sub-section below |
| `codex` (OpenAI Codex CLI) | `~/.codex/config.toml` | The `[sandbox_workspace_write]` entries below | The skills, plus `~/.codex/rules/cafleet.rules` | [The `cafleet` rules file](../spec/coding-agent-backends.md#cafleet-rules-file) |
| `opencode` | none | none required | The skills, plus the `cafleet` agent preset at `~/.opencode/agents/cafleet.md` | [Opencode](../spec/coding-agent-backends.md#opencode) |

The paths above are defaults. `CLAUDE_CONFIG_DIR` and `CODEX_HOME` relocate
their skills and presets. `OPENCODE_CONFIG_DIR` relocates only the Opencode
preset; its skills remain under `~/.config/opencode/skills`. See
[Config-dir resolution](../spec/cli-options.md#config-dir-resolution).

The snippets below are the recommended starting points for the two backends
that need one.

### Claude Code

```json
{
  "permissions": {
    "allow": [
      "Bash(cafleet *)",
      "Skill(cafleet:cafleet)",
      "Skill(cafleet:cafleet-design-doc)"
    ],
    "ask": [
      "Bash(cafleet * member prompt *)"
    ]
  }
}
```

The `Bash(cafleet *)` pattern is the single allow-everything entry that the
literal integer-id convention enables —
one pattern covers every subcommand for every fleet. `cafleet member prompt *`
is moved to the `ask` list because it keystrokes arbitrary text or shell
commands into a member's pane; the operator should confirm each invocation.

### Codex

```toml
[sandbox_workspace_write]
network_access = true
writable_roots = ["/home/<you>/.local/share/cafleet"]
```

`network_access = true` is required because cafleet's multiplexer backends
(tmux and herdr) communicate over a local socket, which the Codex sandbox
classifies as network access — without it cafleet commands fail with
`Operation not permitted`. `writable_roots` grants write access to cafleet's
default SQLite DB directory. Use the absolute path matching
`CAFLEET_DATABASE_URL` or the default XDG location.

The Codex rules for `cafleet` commands allow every subcommand while keeping
`cafleet member prompt` prompting; the reference above covers their precedence
and where operator customizations belong.

### Trust the working directory

Coding agents ask for a trust confirmation the first time they start in a new
directory, and that first-run prompt stalls a freshly spawned member — it
ignores every incoming message until the prompt is cleared. Trust the
workspace in advance: launch your coding agent once in the working directory
the member panes will run in and accept the prompt, or add a trust entry to
the agent's configuration file (see your agent's reference page). Trust is
granted per directory, so each git worktree needs its own approval.


## Complete monitor prompts {#complete-monitor-prompts}

These generated examples use HOME `/home/cafleet-demo` and workspace
`/home/cafleet-demo/work/demo`. Replace both with your actual absolute paths.
Use `cafleet doctor --json` and [Config-dir resolution](../spec/cli-options.md#config-dir-resolution)
to locate the installed skill. Opencode skills stay under HOME/.config/opencode,
even when OPENCODE_CONFIG_DIR relocates its preset. Keep the four identity
placeholders unchanged: fleet creation substitutes them.

<!-- BEGIN BOOTSTRAP codex -->

```text
You are the monitor member in a CAFleet team.
ROLE DEFINITION: Open /home/cafleet-demo/.codex/skills/cafleet/roles/monitor.md BEFORE any other action. Follow that role definition.
Read /home/cafleet-demo/.codex/skills/cafleet/reference/coding-agent-overlays.md and resolve your own backend section, then load /home/cafleet-demo/.codex/skills/cafleet/SKILL.md as a member. Read /home/cafleet-demo/.codex/skills/cafleet/reference/base-dir.md before writing files. Do not start a nested workflow or team.
FLEET ID: {fleet_id}
DIRECTOR MEMBER ID: {director_member_id}
YOUR MEMBER ID: {member_id}
BASE: /home/cafleet-demo/work/demo
CODING AGENT: {coding_agent}
Send ready first. Launch the monitor loop in your own pane using the resolved backend lifecycle, retain its execution handle, and confirm the startup line before sending monitor live. Report a failed start without claiming live; the Director waits for monitor live before spawning ordinary members.
```

<!-- END BOOTSTRAP codex -->

<!-- BEGIN BOOTSTRAP opencode -->

```text
You are the monitor member in a CAFleet team.
ROLE DEFINITION: Open /home/cafleet-demo/.config/opencode/skills/cafleet/roles/monitor.md BEFORE any other action. Follow that role definition.
Read /home/cafleet-demo/.config/opencode/skills/cafleet/reference/coding-agent-overlays.md and resolve your own backend section, then load /home/cafleet-demo/.config/opencode/skills/cafleet/SKILL.md as a member. Read /home/cafleet-demo/.config/opencode/skills/cafleet/reference/base-dir.md before writing files. Do not start a nested workflow or team.
FLEET ID: {fleet_id}
DIRECTOR MEMBER ID: {director_member_id}
YOUR MEMBER ID: {member_id}
BASE: /home/cafleet-demo/work/demo
CODING AGENT: {coding_agent}
Send ready first. Launch the monitor loop in your own pane using the resolved backend lifecycle, retain its execution handle, and confirm the startup line before sending monitor live. Report a failed start without claiming live; the Director waits for monitor live before spawning ordinary members.
```

<!-- END BOOTSTRAP opencode -->
