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
| `codex` (OpenAI Codex CLI) | `~/.codex/config.toml` | The `[sandbox_workspace_write]` entries below | The skills, plus `~/.codex/rules/cafleet.rules` | The Codex sub-section below |
| `opencode` | none | none required | The skills, plus the `cafleet` and `cafleet-ask` agent presets at `~/.config/opencode/agents/cafleet.md` and `~/.config/opencode/agents/cafleet-ask.md` | The OpenCode sub-section below |

The paths above are defaults. `CLAUDE_CONFIG_DIR` and `CODEX_HOME` relocate
their skills and presets. `OPENCODE_CONFIG_DIR` relocates only the OpenCode
presets; its skills remain under `~/.config/opencode/skills`. The
[config-dir resolution contract](https://github.com/himkt/cafleet/blob/main/SPEC.md#config-dir-resolution)
specifies validation and every install target.
Set the variable in the environment you run `cafleet setup` and
`cafleet member create` from: `member create` forwards it to the member, so
the member reads the configuration CAFleet installed into.

The snippets below are the recommended starting points for the two backends
that need one.

### Claude Code

Add the profile below that matches the permission mode your Director session
runs in to your user-level `~/.claude/settings.json`. Members start in
`auto` mode whichever mode the Director uses, so auto mode must be available
for your account and model. Both profiles pre-approve what members need beyond
their broker commands, and auto mode's classifier reviews each other member
call.

| Behavior | Auto mode profile | Other-modes profile |
|---|---|---|
| Director session's permission mode | `auto` | Any other mode |
| The Director's `cafleet` commands | Reviewed by the auto mode classifier | Pre-approved by `Bash(cafleet *)` |
| A member's skill loads, skill-page reads, and file edits | Pre-approved by the entries both profiles share | Pre-approved by the entries both profiles share |
| `cafleet member prompt`, `cafleet member exec` and `cafleet member press` | Prompt the operator | Prompt the operator |

These three commands send explicit native input into a member's pane, so the
operator confirms each invocation.

A claude member needs no allow rule of its own for the broker commands:
cafleet passes `Bash(cafleet message *)` on the member's spawn command line. A
`deny` or `ask` rule that matches a broker command, the managed setting
`allowManagedPermissionRulesOnly` or a blocking `PreToolUse` hook still stops
a member.

| Entry | Profile | Needed by | Why |
|---|---|---|---|
| `Bash(cafleet *)` | Other modes | The Director | One pattern pre-approves every subcommand the Director runs |
| `Skill(cafleet)`, `Skill(cafleet-design-doc)` | Both | The Director and every member | Each loads the installed skills through the Skill tool |
| `Read(~/.claude/skills/**)` | Both | The Director and every member | Role and reference pages of an installed skill sit outside the working directory |
| `Edit`, `Write` | Both | Every member that writes files | A matching entry pre-approves the member's edits; no entry covers a protected path such as `.git` or `.claude` |
| The `member prompt`, `member exec` and `member press` prefixes under `ask` | Both | The Director | The operator confirms each dispatch |

`Edit` and `Write` apply to every session that reads the settings file, your
own included. To confine the approval, replace both with a path rule such as
`Edit(~/work/**)`, which covers every file-editing tool. When
`CLAUDE_CONFIG_DIR` relocates the config directory, write the `Read` entry
against that directory's `skills/**`.

A member whose shell command is denied or refused by auto mode asks its
Director to run it, and each such `cafleet member exec` asks you to confirm.
Add an allow entry for every project command members run as part of their
work, such as `Bash(npm test *)`, so those commands run without routing.

| Symptom | Cause | Resolution |
|---|---|---|
| A member reports a refused or denied shell command | Auto mode's classifier refused it, or a `deny` rule matched it | Add an allow entry for the command |
| A member reports a refused or denied file edit | No `Edit` entry matches, or the path is protected | Add `Edit` and `Write` or widen the path rule; for a protected path, the Director makes the edit |
| The Director is prompted for every `cafleet` command | The session runs in a non-auto mode with the auto mode profile | Switch the session to auto mode, or use the other-modes profile |
| The classifier blocks a Director `cafleet` command | Auto mode judged the action outside the request | Add an allow pattern for that subcommand |
| `defaultMode: "auto"` has no effect | It is set in a project or local settings file, or auto mode is unavailable for the organization or model | Move it to `~/.claude/settings.json`; when auto mode is unavailable, use the other-modes profile |

Claude Code 2.1.302 in auto mode can show a `Teach auto mode about your
environment?` dialog once per configuration directory, after a member's turn.
Answer it once in an interactive session of that version that uses the
members' configuration directory; `Don't show again` keeps it from returning.

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
      "Bash(cafleet member exec *)",
      "Bash(cafleet member press *)"
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
      "Bash(cafleet member exec *)",
      "Bash(cafleet member press *)"
    ]
  }
}
```

`Bash(cafleet *)` covers every subcommand the Director runs, for every fleet,
because IDs are literal integer arguments.

### Codex

```toml
[sandbox_workspace_write]
network_access = true
writable_roots = ["/home/<you>/.local/share/cafleet"]
```

`network_access = true` is required because tmux and herdr communicate over a
local socket, which the Codex sandbox classifies as network access; without it
cafleet commands fail with `Operation not permitted`. `writable_roots` grants
write access to the directory of the selected database file, the default XDG
location unless `CAFLEET_DATABASE_URL` selects another.

`cafleet setup` installs `~/.codex/rules/cafleet.rules`: it allows every
`cafleet` command and keeps the Director-owned commands prompting. Codex
applies the strictest decision across every `*.rules` file in that directory,
and setup overwrites its own file on every install, so put your own rules in a
separate file there. A rules directory maintained without the installed file,
for example from a dotfiles repository, needs at least these rules:

```text
prefix_rule(
    pattern = ["cafleet", "message"],
    decision = "allow",
    match = ["cafleet message poll 5", "cafleet message send --from-member-id 1 --to-member-id 2 ready"],
    not_match = ["cafleet member list 1", "cafleet doctor"],
)

prefix_rule(
    pattern = ["cafleet", "member", "prompt"],
    decision = "prompt",
    justification = "cafleet member prompt keystrokes arbitrary text into a member pane",
)

prefix_rule(
    pattern = ["cafleet", "member", "exec"],
    decision = "prompt",
    justification = "cafleet member exec runs an arbitrary shell command in a member pane",
)

prefix_rule(
    pattern = ["cafleet", "member", "press"],
    decision = "prompt",
    justification = "cafleet member press sends explicit native keys to a member pane",
)
```

That set is enough for members. Keep the installed file for a Director
session: Codex has no classifier to review what the minimum set leaves
unmatched.

### OpenCode

`cafleet setup` installs both agent presets; rerun it after upgrading CAFleet
to refresh them. A member runs under the `cafleet` preset by default, which
allows its listed commands and denies every other shell command without a
prompt. Choose `--permission-mode cafleet-ask` for a member whose other shell
commands should open a native permission form for the Director to answer.
Neither preset is an OS-level sandbox: keep every loaded OpenCode configuration
free of MCP servers, submit one shell command per invocation, and use the
`codex` backend when you need kernel-enforced isolation.

### Trust the working directory

Coding agents ask for a trust confirmation the first time they start in a new
directory, and that first-run prompt stalls a freshly spawned member — it
ignores every incoming message until the prompt is cleared. Trust the
workspace in advance: launch your coding agent once in the working directory
the member panes will run in and accept the prompt, or add a trust entry to
the agent's configuration file. For Codex, add the absolute workspace path to
`~/.codex/config.toml`:

```toml
[projects."/abs/path/to/workspace"]
trust_level = "trusted"
```

Trust is granted per directory, so each git worktree needs its own approval.

## Simple example — invoke from a coding agent

You can ask your agent: “Use the cafleet skill to create a team with two
members, exchange a message, then shut down the team.” The skill manages
the bootstrap and supervision protocol.

For the complete command sequence and backend variations, follow
[Run a fleet](how-to/mixed-backend-team.md#manual-lifecycle).
