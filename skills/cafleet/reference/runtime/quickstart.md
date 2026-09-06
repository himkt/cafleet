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

Configure and trust the workspace before spawning members; follow
[Workspace configuration](concepts/coding-agents.md#workspace-configuration)
for Claude, Codex, or Opencode. Then check the database, assets and multiplexer:

```bash
cafleet doctor
```

Resolve any reported issues before creating the fleet.

## Simple example — invoke from a coding agent

You can ask your agent: “Use the cafleet skill to create a team with two
members, exchange a message, then shut down the team.” The skill manages
the bootstrap and supervision protocol below.

## Raw CLI walkthrough

This example uses Claude; [complete Codex and Opencode prompts](concepts/coding-agents.md#complete-monitor-prompts)
follow the same template. The example HOME is `/home/cafleet-demo` and the
workspace is `/home/cafleet-demo/work/demo`. Replace those with your actual
absolute paths, using `cafleet doctor --json` and
[Config-dir resolution](spec/cli-options.md#config-dir-resolution) to locate
the installed skill. Save the following as `monitor-prompt.md` in the workspace.
Keep the four identity placeholders: fleet creation fills them in.

<!-- BEGIN BOOTSTRAP claude -->

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

<!-- END BOOTSTRAP claude -->

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
