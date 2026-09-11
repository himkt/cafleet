# Run a fleet

Create a supervised team from your coding-agent pane inside tmux or herdr.
A Director can use one backend or mix Claude, Codex and OpenCode members;
all exchange messages through the same broker.

## Prerequisites

Follow Quickstart to [install](../quickstart.md#install),
[configure](../quickstart.md#configure) and
[trust the working directory](../quickstart.md#trust-the-working-directory).
Put each selected backend binary on PATH. Run every CAFleet command in its
own shell-tool invocation and use the literal IDs returned by the CLI.

## Prompt

```text
Create a CAFleet team for this repo with three members — one on claude,
one on codex, and one on opencode. Name them alice, bob, and carol.
Send each member a message asking it to report its backend, confirm
all three replies, then shut down the team.
```

Your agent loads the CAFleet Director instructions, starts the monitor,
and dispatches each member after its own ready signal. Messages appear as
[inline previews](../spec/multiplexer-backends.md#push-notifications) in
the member panes and remain available through the broker.

## Manual lifecycle

This baseline uses a Claude Director, example HOME `/home/cafleet-demo`
and workspace `/home/cafleet-demo/work/demo`. Substitute your actual
absolute paths and installed skill root. Run the diagnostic first; resolve
any reported issue before bootstrap:

```bash
cafleet doctor --json
```

Use [Config-dir resolution](../spec/cli-options.md#config-dir-resolution)
for overrides. The default skill roots are:

| Backend | CAFleet skill root |
|---|---|
| `claude` | `~/.claude/skills/cafleet` |
| `codex` | `~/.codex/skills/cafleet` |
| `opencode` | `~/.config/opencode/skills/cafleet` |

The Director loads its own backend instructions and the installed generic
Director role and supervision protocol before orchestration. When composing
a spawn, it reads the selected backend's model catalog, defaults and
capabilities. Pane classification uses the observed member's backend cues.

### Bootstrap the monitor

Save this complete prompt as
`/home/cafleet-demo/work/demo/.prompts/monitor.md`, creating the directory
and choosing a new filename if one already exists. Keep the four identity
placeholders for CAFleet to fill during creation; double any other literal
braces added to the prompt.

```text
You are the monitor member in a CAFleet team.
ROLE DEFINITION: Open /home/cafleet-demo/.claude/skills/cafleet/roles/monitor.md BEFORE any other action. Follow that role definition.
Read /home/cafleet-demo/.claude/skills/cafleet/reference/coding-agents.md and resolve your own backend section, then load /home/cafleet-demo/.claude/skills/cafleet/SKILL.md as an assigned member. Read /home/cafleet-demo/.claude/skills/cafleet/reference/base-dir.md before writing files.
FLEET ID: {fleet_id}
DIRECTOR MEMBER ID: {director_member_id}
YOUR MEMBER ID: {member_id}
BASE: /home/cafleet-demo/work/demo
CODING AGENT: {coding_agent}
Complete the role's prerequisite reads, send ready to the Director, then launch the monitor loop in your own pane using the resolved backend lifecycle. Retain its execution handle and confirm the startup line before sending monitor live. Report a failed start without claiming live; the Director waits for monitor live before spawning an ordinary member.
```

```bash
cafleet fleet create --name demo --coding-agent claude --monitor-model haiku --monitor-file /home/cafleet-demo/work/demo/.prompts/monitor.md
```

For example, the command returns `1 director=2 monitor=3`. The
`--coding-agent` value states the backend already running in the Director's
pane; the monitor inherits it. Wait for the monitor's `ready`, then its
confirmed `monitor live` before creating an ordinary member. Registration
alone does not establish startup. Codex retains a managed execution session
and performs bounded startup and later-wake liveness checks; each backend's
installed monitor instructions define its execution mechanism.

### Create and dispatch a member

Save this prompt as a new file
`/home/cafleet-demo/work/demo/.prompts/member.md`:

```text
You are an ordinary CAFleet member.
ROLE DEFINITION: Open /home/cafleet-demo/.claude/skills/cafleet/roles/member.md before any other action. Follow that role definition.
Read /home/cafleet-demo/.claude/skills/cafleet/reference/coding-agents.md and resolve your own backend section, then load /home/cafleet-demo/.claude/skills/cafleet/SKILL.md. Read /home/cafleet-demo/.claude/skills/cafleet/reference/base-dir.md and follow the supplied BASE contract.
FLEET ID: {fleet_id}
DIRECTOR MEMBER ID: {director_member_id}
YOUR MEMBER ID: {member_id}
BASE: /home/cafleet-demo/work/demo
CODING AGENT: {coding_agent}
Use an available text reader for prerequisites; shell-only prerequisite reads may precede ready. As your first operational broker shell command, send: cafleet message send --from-member-id {member_id} --to-member-id {director_member_id} "ready"
Poll and ACK your assignment, then act on the Director's instructions. Wait at the prompt when no assignment remains.
```

```bash
cafleet member create --fleet-id 1 --name alice --description "Demo member" --file /home/cafleet-demo/work/demo/.prompts/member.md
```

Suppose the returned member ID is `4`. Wait for that member's `ready`, then
take a fresh capture and apply the [capture gate](../concepts/monitoring.md)
before dispatching work:

```bash
cafleet member capture 4
```

When the capture shows the member ready to receive its assignment:

```bash
cafleet message send --from-member-id 2 --to-member-id 4 "Please reply hello."
```

The member receives the preview, polls, acknowledges and replies. Read the
Director inbox and ACK each consumed delivery using its actual message ID:

```bash
cafleet message poll 2 --json
```

```bash
cafleet message ack 10
```

Here `10` is an example. Repeat the capture gate before further dispatch;
working or awaiting-user panes defer the send, and an unknown capture needs
diagnosis. The monitor's live gate and each ordinary member's ready gate
serve different purposes.

### Close the fleet

Delete the monitor first to stop its wake source, then delete the ordinary
member. After each command succeeds, verify that only the root Director
remains before deleting the fleet:

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

Confirm fleet `1` is absent. If another fixture-owned member remains, finish
its member deletion before fleet deletion; fleet deletion alone does not
close panes. Apply cleanup only to the team you are authorized to manage.

## Backend variations

Use the same lifecycle for a Codex or OpenCode Director, declaring its actual
backend at fleet creation and using its installed monitor paths and selected
monitor model. The monitor inherits the Director's backend. Resolve model
choices through [Model choice](../concepts/coding-agents.md#model-choice).

For a mixed team, save separate ordinary-member prompts with the appropriate
installed role/core/backend paths and a shared workspace BASE. After monitor
live, add members with explicit backend flags:

```bash
cafleet member create --fleet-id 1 --name bob --description "Codex member" --coding-agent codex --file /home/cafleet-demo/work/demo/.prompts/bob.md
```

```bash
cafleet member create --fleet-id 1 --name carol --description "OpenCode member" --coding-agent opencode --file /home/cafleet-demo/work/demo/.prompts/carol.md
```

Dispatch each member after its own ready and fresh capture, using the actual
returned ID. List members to find their panes: only Claude sets the member
name as its pane title. During shutdown, delete every added member after the
monitor and before the root-only registry check. Exact flags, outputs and
failure distinctions are in [CLI options](../spec/cli-options.md).
