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

Rerun `cafleet setup` after every upgrade, patch releases included, because
each version starts from a fresh database; see
[Upgrading cafleet](concepts/storage.md#upgrading-cafleet).

## Configure

CAFleet is designed to run inside a coding agent without per-command
permission prompts. Each backend has a different config file and permission
system:

| Backend | Config file | Manual configuration | Installed by `cafleet setup` |
|---|---|---|---|
| [`claude` (Claude Code)](#claude-code) | `~/.claude/settings.json` | The permission profile below that matches the Director's mode | The skills |
| [`codex` (OpenAI Codex CLI)](#codex) | `~/.codex/config.toml` | The `[sandbox_workspace_write]` entries below | The skills, plus `~/.codex/rules/cafleet.rules` |
| [`opencode`](#opencode) | — | — | The skills, plus the `cafleet` and `cafleet-ask` agent presets in `~/.config/opencode/agents/` |
| [`hermes` (Hermes Agent)](#hermes) | `~/.hermes/config.yaml` | Keep `approvals.mode` at its `smart` default | The skills, plus CAFleet's member variables merged into `terminal.client_env_passthrough` |

The paths above are defaults. `CLAUDE_CONFIG_DIR`, `CODEX_HOME` and
`HERMES_HOME` relocate their backend's installed assets. `OPENCODE_CONFIG_DIR` relocates only the OpenCode
presets; its skills remain under `~/.config/opencode/skills`. The
[config-dir resolution contract](https://github.com/himkt/cafleet/blob/main/SPEC.md#config-dir-resolution)
specifies validation and every install target.
Set the variable in the environment you run `cafleet setup` and
`cafleet member create` from: `member create` forwards it to the member, so
the member reads the configuration CAFleet installed into.

The sub-sections below give each backend's recommended starting point.

### Claude Code

Add one profile to your user-level `~/.claude/settings.json`: the
[auto mode profile](#auto-mode-profile) when your Director session runs in
`auto` mode, or the [other-modes profile](#other-modes-profile) for any other
mode. Members start in `auto` mode whichever mode the Director uses, so auto
mode must be available for your account and model. Both profiles pre-approve
what members need beyond their broker commands, and auto mode's classifier
reviews each other member call.

`cafleet member prompt`, `cafleet member exec` and `cafleet member press` send
explicit native input into a member's pane, and both profiles list them under
optional `ask` entries: keep them when you want to confirm each of the
Director's dispatches, or drop them to let the Director dispatch without a
prompt. Inside a member process every Director-only command refuses, which
catches a member's accidental invocation; this guard is not a security
boundary ([Command routing](concepts/members.md#command-routing)).

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
| The `member prompt`, `member exec` and `member press` prefixes under `ask` | Both, optional | The Director | The operator confirms each of the Director's dispatches |

`Edit` and `Write` apply to every session that reads the settings file, your
own included. To confine the approval, replace both with a path rule such as
`Edit(~/work/**)`, which covers every file-editing tool. When
`CLAUDE_CONFIG_DIR` relocates the config directory, write the `Read` entry
against that directory's `skills/**`.

A member whose shell command is denied or refused by auto mode asks its
Director to run it, and with the `ask` entries each such `cafleet member exec`
asks you to confirm.
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
environment?` dialog after a member's turn. Answer it with `Don't show again`
in an interactive session of that version that uses the members'
configuration directory; after that answer it does not return.

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
because IDs are literal integer arguments. `ask` entries outrank allow
entries, so the optional `ask` entries still confirm each explicit pane-input
dispatch of the Director.

### Codex

```toml
[sandbox_workspace_write]
network_access = true
writable_roots = ["/home/<you>/.local/share/cafleet"]
```

`network_access = true` is required because tmux and herdr communicate over a
local socket, which the Codex sandbox classifies as network access; without it
cafleet commands fail with `Operation not permitted`. `writable_roots` grants
write access to the directory of the selected database file, the default
`~/.local/share/cafleet` unless `CAFLEET_DATABASE_URL` selects another.

`cafleet setup` installs `~/.codex/rules/cafleet.rules`, which allows every
`cafleet` command. Codex members need it to run their broker commands, and
the Director-only commands refuse inside a member process on their own. Codex
applies the strictest decision across every `*.rules` file in that directory,
and setup overwrites its own file on every install, so put your own rules,
such as `prompt` rules to confirm the Director's commands, in a separate file
there. A rules directory maintained without the installed file, for example
from a dotfiles repository, needs at least this rule:

```text
prefix_rule(
    pattern = ["cafleet", "message"],
    decision = "allow",
    match = ["cafleet message poll 5", "cafleet message send --from-member-id 1 --to-member-id 2 ready"],
    not_match = ["cafleet member list 1", "cafleet doctor"],
)
```

That rule is enough for members. Keep the installed file for a Director
session: Codex has no classifier to review the other `cafleet` commands the
Director runs, so without a matching allow each of them follows the session's
own approval policy and sandbox.

### OpenCode

`cafleet setup` installs both agent presets; rerun it after upgrading CAFleet
to refresh them. A member runs under the `cafleet` preset by default, which
allows its listed commands and denies every other shell command without a
prompt. Choose `--permission-mode cafleet-ask` for a member whose other shell
commands should open a native permission form for the Director to answer.
Neither preset is an OS-level sandbox: keep every loaded OpenCode configuration
free of MCP servers, submit one shell command per invocation, and use the
`codex` backend when you need kernel-enforced isolation.

### Hermes

Hermes members use your existing Hermes configuration: your configured default
provider and model, and your profile when `active_profile` selects one. Each
member starts a fresh conversation. Run `cafleet setup --coding-agent hermes`
with `hermes` on `PATH`. Besides the skills, it adds CAFleet's member variables
(`CAFLEET_DATABASE_URL`, `CAFLEET_MEMBER_ID` and the tmux and herdr pane
variables) to `terminal.client_env_passthrough` in the file
`hermes config path` reports, keeping your own entries, their order and the
rest of the file. Hermes runs every session's terminal tools in one shared
gateway, and this list lets each member's tools see that member's own values,
including the `CAFLEET_MEMBER_ID` marker of the
[Director-only command guard](concepts/members.md#command-routing). Rerunning
setup adds only missing names, after your existing entries.
A plain `cafleet setup` on a host without `hermes` installs the skills and
skips this merge; run `cafleet setup --coding-agent hermes` after installing
Hermes.

`cafleet member create` reads your effective Hermes configuration and refuses
to start a member when a member variable is missing from the list.
`cafleet doctor` reports the list on its `hermes` summary line, for example
`error hermes: terminal.client_env_passthrough lacks CAFleet's member names: CAFLEET_MEMBER_ID — run 'cafleet setup --coding-agent hermes'`.

CAFleet needs no `approvals.deny` entries; keep every `cafleet …` glob out of
that list. Hermes applies `approvals.deny` to every session, so such a glob
also blocks the matching command in a Hermes Director's own session, which
Hermes reports as
`BLOCKED: this command matches the user-defined deny rule '<glob>' …`.
Inspect the list with `hermes config get --json approvals.deny`, then write it
back without those globs with
`hermes config set approvals.deny '<JSON list without them>'`. Under
`approvals.mode: smart`, the default, commands Hermes does not consider
dangerous run without review, and its guardian model reviews the rest. A
guardian escalation shows an approval prompt in the member's pane, which the
Director answers.

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
[Run a fleet](mixed-backend-team.md#manual-lifecycle).
