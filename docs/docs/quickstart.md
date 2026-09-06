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
the bootstrap and supervision protocol below.

## Raw CLI walkthrough

This example uses Claude; [Codex and Opencode skill paths](concepts/coding-agents.md#complete-monitor-prompts)
follow the same template. The example HOME is `/home/cafleet-demo` and the
workspace is `/home/cafleet-demo/work/demo`. Replace those with your actual
absolute paths, using `cafleet doctor --json` and
[Config-dir resolution](spec/cli-options.md#config-dir-resolution) to locate
the installed skill. Save the following as `monitor-prompt.md` in the workspace.
Keep the four identity placeholders: fleet creation fills them in.

```text
You are the monitor member in a CAFleet team.
ROLE DEFINITION: Open /home/cafleet-demo/.claude/skills/cafleet/roles/monitor.md BEFORE any other action. Follow that role definition.
Read /home/cafleet-demo/.claude/skills/cafleet/reference/coding-agent-overlays.md and resolve your own backend section, then load /home/cafleet-demo/.claude/skills/cafleet/SKILL.md as a member. Read /home/cafleet-demo/.claude/skills/cafleet/reference/base-dir.md before writing files. Do not start a nested workflow or team.
FLEET ID: {fleet_id}
DIRECTOR MEMBER ID: {director_member_id}
YOUR MEMBER ID: {member_id}
BASE: /home/cafleet-demo/work/demo
CODING AGENT: {coding_agent}
Send ready first. Launch the monitor loop in your own pane using the resolved backend lifecycle, retain its execution handle, and confirm the startup line before sending monitor live. Report a failed start without claiming live; the Director waits for monitor live before spawning ordinary members.
```

```bash
cafleet fleet create --name demo --coding-agent claude --monitor-file /home/cafleet-demo/work/demo/monitor-prompt.md
```

The result contains the fleet, Director and monitor IDs. In the following
example they are 1, 2 and 3. The monitor sends `ready`, starts the loop in
its own pane, checks the startup line, then sends `monitor live`. Wait for
that confirmed `monitor live` before creating an ordinary member. Codex
retains the managed session and applies its bounded startup check; all
backend lifecycle details remain in the installed monitor role.

```bash
cafleet member create --fleet-id 1 --name demo-member --description "Demo member" "Load the cafleet skill as a member and read its roles/member.md before acting.
FLEET ID: {fleet_id}
DIRECTOR MEMBER ID: {director_member_id}
YOUR MEMBER ID: {member_id}
CODING AGENT: {coding_agent}
Send ready to your Director, then wait for instructions."
```

Wait for the member's `ready`, then take a fresh capture and confirm the
member is ready to receive work under the [capture gate](concepts/monitoring.md).
If the returned member ID is 4, send it work through the broker:

```bash
cafleet message send --from-member-id 2 --to-member-id 4 "Please reply hello."
```

The member receives an inline preview, polls, acknowledges and replies.
Read the Director inbox and acknowledge the returned message ID:

```bash
cafleet message poll 2
```

```bash
cafleet message ack 10
```

Here 10 is an example: use the actual message ID. Follow the
[Monitoring](concepts/monitoring.md) capture gate before further dispatch.

## Shutdown

Delete the monitor first to stop its wake source, then the ordinary member.
Verify only the Director remains before deleting the fleet:

```bash
cafleet member delete 3
```

```bash
cafleet member delete 4
```

```bash
cafleet member list 1
```

```bash
cafleet fleet delete 1
```

```bash
cafleet fleet list
```

See [CLI options](spec/cli-options.md) for command contracts and
[Mixed-backend teams](how-to/mixed-backend-team.md) for a larger example.
