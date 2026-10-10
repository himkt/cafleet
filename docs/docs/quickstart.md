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
| `claude` (Claude Code) | `~/.claude/settings.json` | The permission profile below that matches the Director's mode | The skills | The sub-section below |
| `codex` (OpenAI Codex CLI) | `~/.codex/config.toml` | The `[sandbox_workspace_write]` entries below | The skills, plus `~/.codex/rules/cafleet.rules` | [The `cafleet` rules file](spec/coding-agent-backends.md#cafleet-rules-file) |
| `opencode` | none | none required | The skills, plus the `cafleet` agent preset at `~/.opencode/agents/cafleet.md` | [Opencode](spec/coding-agent-backends.md#opencode) |

The paths above are defaults. `CLAUDE_CONFIG_DIR` and `CODEX_HOME` relocate
their skills and presets. `OPENCODE_CONFIG_DIR` relocates only the Opencode
preset; its skills remain under `~/.config/opencode/skills`. See
[Config-dir resolution](spec/cli-options.md#config-dir-resolution).

The snippets below are the recommended starting points for the two backends
that need one.

### Claude Code

Add the profile below that matches the permission mode your Director session
runs in to your user-level `~/.claude/settings.json`. Members start in
`dontAsk` mode whichever mode the Director uses, so both profiles pre-approve
what members need beyond their broker commands.

| Behavior | Auto mode profile | Other-modes profile |
|---|---|---|
| Director session's permission mode | `auto` | Any other mode |
| The Director's `cafleet` commands | Reviewed by the auto mode classifier | Pre-approved by `Bash(cafleet *)` |
| A member's skill loads, skill-page reads, and file edits | Pre-approved by the entries both profiles share | Pre-approved by the entries both profiles share |
| `cafleet member prompt` and `cafleet member exec` | Prompt the operator | Prompt the operator |

`cafleet member prompt` and `cafleet member exec` are under `ask` in both
profiles because the first keystrokes arbitrary text into a member's pane and
the second runs an arbitrary command there; the operator confirms each
invocation.

A claude member needs no allow rule of its own for the broker commands:
cafleet passes them on the member's spawn command line. A `deny` or `ask` rule
that matches a broker command still blocks a member, and `cafleet doctor`
reports it — see
[Spawn-time allow rules](spec/coding-agent-backends.md#spawn-time-allow-rules).

`Edit` and `Write` let members write files. They apply to every session that
reads the settings file, your own included: each then edits files outside
Claude Code's protected paths without a prompt or classifier review.
[What each profile entry is for](spec/coding-agent-backends.md#claude-profile-entries)
names the role that needs each entry and gives the path rule that confines
this approval.
[What members can run](spec/coding-agent-backends.md#claude-member-permissions)
covers denied commands and the entries to add for your project's own task
commands.

#### Auto mode {#auto-mode-profile}

```json
{
  "permissions": {
    "defaultMode": "auto",
    "allow": [
      "Skill(cafleet)",
      "Skill(cafleet-design-doc)",
      "Read(~/.claude/skills/**)",
      "Edit",
      "Write"
    ],
    "ask": [
      "Bash(cafleet member prompt *)",
      "Bash(cafleet member exec *)"
    ]
  }
}
```

In auto mode a classifier reviews each Director command that no rule matches,
so the allow list carries no `Bash(cafleet ...)` entry. `defaultMode` selects
auto mode from user-level or managed settings; a project or local settings
file cannot select it.

#### Other permission modes {#other-modes-profile}

```json
{
  "permissions": {
    "allow": [
      "Bash(cafleet *)",
      "Skill(cafleet)",
      "Skill(cafleet-design-doc)",
      "Read(~/.claude/skills/**)",
      "Edit",
      "Write"
    ],
    "ask": [
      "Bash(cafleet member prompt *)",
      "Bash(cafleet member exec *)"
    ]
  }
}
```

`Bash(cafleet *)` is the single allow-everything entry that the literal
integer-id convention enables — one pattern covers every subcommand the
Director runs, for every fleet.

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
`cafleet member prompt` and `cafleet member exec` prompting; the reference
above covers their precedence and where operator customizations belong. A
rules directory maintained without the installed file needs the
[minimum rule set](spec/coding-agent-backends.md#minimum-rule-set).

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
