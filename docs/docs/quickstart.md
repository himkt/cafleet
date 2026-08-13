# Quickstart

This page is a one-screen walkthrough that creates a CAFleet fleet, spawns
two member panes, and sends a message between them. It starts from a clean
machine: install the CLI, configure your coding agent, then run the
walkthrough.

## Install

Prerequisites:

| Requirement | Accepted | Notes |
|---|---|---|
| Platform | macOS (Apple Silicon) or Linux (x86_64 / aarch64) | One prebuilt binary per target — see the install block below |
| Terminal multiplexer | tmux or herdr | Auto-detected — see [Multiplexer backends](spec/multiplexer-backends.md) |
| Coding agent | `claude` (Claude Code), `codex` (OpenAI Codex CLI), or `opencode` | At least one; a mixed fleet may use all three — see [Coding agents](concepts/coding-agents.md) |

Install the CLI with Homebrew, then run the setup:

```bash
brew install himkt/tap/cafleet
cafleet setup
```

Alternatively, download the archive for your platform from
[GitHub Releases](https://github.com/himkt/cafleet/releases) —
`cafleet-v<version>-<target>.tar.gz` for `aarch64-apple-darwin`,
`x86_64-unknown-linux-musl`, or `aarch64-unknown-linux-musl` — then extract
the single `cafleet` binary onto your `PATH` and run the same setup:

```bash
tar -xzf cafleet-v<version>-<target>.tar.gz
mv cafleet ~/.local/bin/
cafleet setup
```

## Configure

CAFleet is designed to run inside a coding agent without per-command
permission prompts. Each backend has a different config file and permission
system:

| Backend | Config file | Manual configuration | Installed by `cafleet setup` | Reference |
|---|---|---|---|---|
| `claude` (Claude Code) | `~/.claude/settings.json` | The `permissions.allow` / `permissions.ask` entries below | The skills | The sub-section below |
| `codex` (OpenAI Codex CLI) | `~/.codex/config.toml` | The `[sandbox_workspace_write]` entries below | The skills, plus `~/.codex/rules/cafleet.rules` | [The `cafleet` rules file](spec/coding-agent-backends.md#cafleet-rules-file) |
| `opencode` | none | none required | The skills, plus the `cafleet` agent preset at `~/.opencode/agents/cafleet.md` | [Opencode](spec/coding-agent-backends.md#opencode) |

The paths above are the defaults: the backend config-location variables
`CLAUDE_CONFIG_DIR`, `CODEX_HOME`, and `OPENCODE_CONFIG_DIR` relocate them,
and `cafleet setup` installs to the same resolved directories — see
[Config-dir resolution](spec/cli-options.md#config-dir-resolution).

The snippets below are the recommended starting points for the two backends
that need one.

### Claude Code

```json
{
  "permissions": {
    "allow": [
      "Bash(cafleet *)",
      "Skill(cafleet:cafleet)",
      "Skill(cafleet:cafleet-design-doc)",
      "Skill(cafleet:cafleet-research)"
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

## Simple example — invoke from a coding agent

For the simplest path, ask Claude Code or Codex to demonstrate CAFleet for
you. Inside an agent, send the following prompt:

```text
I want to see how cafleet works.
Please create a new team with two members using cafleet and let them ping-pong each other.
After the demonstration, please shutdown the team.
```

The coding agent will invoke the `cafleet` skill, which guides it through the
fleet-create / member-create / message-send loop and finally tears the team
down.

## Raw CLI walkthrough

If you would rather drive CAFleet from the shell directly, the commands below
mirror what the skill does internally. Run them inside a tmux or herdr session —
the `fleet create` and `member create` commands require one.

:::details Expand the walkthrough

The walkthrough pastes literal integer ids: fleet `1`, root Director `2`,
members `3` and `4`, message `10`. Your ids will differ — substitute the
integers your own commands print.

Create a fleet. This records your current pane as the root Director's
pane; `--coding-agent` is required — pass the backend the Director is
actually running on (here, Claude Code):

```bash
cafleet fleet create --name "demo" --coding-agent claude
```

```
1 director=2
```

The line carries the fleet id (`1`) and the root Director's member id
(`2`). If it scrolls away, run
`cafleet fleet list` — it re-prints the fleet id and the Director id (the
`DIRECTOR` column).

Spawn two member panes. Each member's prompt is just a one-line greeting.
Optional: add `--model <m>` (e.g. `--model sonnet`) to pin a member's LLM;
omitted, the backend binary uses its own default model:

```bash
cafleet member create --fleet-id 1 \
  --name "demo-member" \
  --description "Demo member" \
  "You are demo-member. Reply hello when polled."
```

```
3 demo-member backend=claude pane=%7
```

```bash
cafleet member create --fleet-id 1 \
  --name "reviewer" \
  --description "Reviewer member" \
  "You are reviewer. Reply hello when polled."
```

```
4 reviewer backend=claude pane=%8
```

List the fleet's roster — the new members' ids (`3` and `4`) appear
alongside the Director:

```bash
cafleet member list 1
```

```
3 members:
  member_id  name           kind      backend   pane_id  idle
  ---------  -------------  --------  --------  -------  ----
  2          Director       director  claude    %0       -
  3          demo-member    member    claude    %7       -
  4          reviewer       member    claude    %8       -
```

Send a message between the members — `demo-member` (`3`) messages
`reviewer` (`4`):

```bash
cafleet message send --from-member-id 3 --to-member-id 4 "hi"
```

```
Message sent.
[10 | from:3 | 2026-06-11T09:00:00.123456+00:00]
hi
```

`reviewer` receives the message as a 2-line inline preview pushed into its
tmux pane and the message lands in the broker queue. From here, the typical
flow is `cafleet message poll` from the recipient and `cafleet message ack`
once it has consumed the message.

When you are done, tear the fleet down:

```bash
cafleet member delete 3
cafleet member delete 4
cafleet fleet delete 1
```

```
Deleted fleet 1. Deregistered 1 members.
```

:::

Where to go next:

- [CLI options](spec/cli-options.md) — every subcommand and flag.
- [How-to guides](how-to/mixed-backend-team.md) — prompt-first task guides.
