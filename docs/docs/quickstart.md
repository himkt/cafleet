# Quickstart

Create a fleet, send a message, and close the team from your coding-agent
pane inside tmux or herdr. Use the literal IDs returned by each command.
Run each CAFleet command in its own shell-tool invocation.

## Install

```bash
brew install himkt/tap/cafleet
```

```bash
cafleet setup
```

Alternatively, extract the binary for your platform from
[GitHub Releases](https://github.com/himkt/cafleet/releases) onto PATH, then
run setup. Setup installs embedded skills and presets offline; use
`--coding-agent` to select a backend.

## Configure

CAFleet is designed to run inside a coding agent without per-command
permission prompts. Each backend has a different config file and permission
system:

| Backend | Config file | Manual configuration | Installed by `cafleet setup` | Reference |
|---|---|---|---|---|
| `claude` (Claude Code) | `~/.claude/settings.json` | The `permissions.allow` / `permissions.ask` entries below | The skills | The sub-section below |
| `codex` (OpenAI Codex CLI) | `~/.codex/config.toml` | The `[sandbox_workspace_write]` entries below | The skills, plus `~/.codex/rules/cafleet.rules` | [The `cafleet` rules file](spec/coding-agent-backends.md#cafleet-rules-file) |
| `opencode` | none | none required | The skills, plus the `cafleet` agent preset at `~/.opencode/agents/cafleet.md` | [Opencode](spec/coding-agent-backends.md#opencode) |

The paths above are defaults. `CLAUDE_CONFIG_DIR` and `CODEX_HOME` relocate
their skills and presets. `OPENCODE_CONFIG_DIR` relocates only the Opencode
preset; its skills remain under `~/.config/opencode/skills`. See
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

## Simple example — invoke from a coding agent

You can ask your agent: “Use the cafleet skill to create a team with two
members, exchange a message, then shut down the team.” The skill manages
the bootstrap and supervision protocol.

For the complete command sequence and backend variations, follow
[Run a fleet](how-to/mixed-backend-team.md#manual-lifecycle).
