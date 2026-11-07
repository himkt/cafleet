# Run a fleet

Create independent coding-agent sessions in tmux or herdr, assign work and receive durable replies through the Director's listener.

## Prerequisites

Follow Quickstart to [install](quickstart.md#install), [configure](quickstart.md#configure) and [trust the workspace](quickstart.md#trust-the-working-directory). Put each selected backend on PATH. Use literal returned IDs and one CAFleet command per shell-tool invocation.

## Prompt

```text
Use CAFleet to delegate independent reviews of this change, collect both reports, resolve their findings, and close the team.
```

Your agent loads the Director instructions, starts its listener, creates members and assigns each after its ready. The Director consumes broker events and checks evidence while preserving the host's supplied continuation and user gates.

## Manual lifecycle

This example uses HOME `/home/cafleet-demo` and workspace `/home/cafleet-demo/work/demo`. Substitute actual absolute paths and the installed skill root, including any [relocated config directory](quickstart.md#configure).

```bash
cafleet doctor --json
```

Resolve diagnosis issues. Load the Director role, backend reference and supervision procedure from the installed CAFleet skill, then register the calling session:

```bash
cafleet fleet create --name demo --coding-agent claude
```

Example output: `1 director=2`. Creation registers the current pane and its multiplexer server without starting another session; the backend flag declares the calling coding agent, and every later command targets the recorded pane and server. A Hermes Director passes its own pane, read with `! echo $TMUX_PANE` or `! echo $HERDR_PANE_ID`, as `--pane <pane-id>` ([Hermes setup](quickstart.md#hermes)); `--mux-server <server>` selects a server other than the one CAFleet finds by default. Before creating members, the Director starts its listener through the installed CAFleet skill's backend recipe; the listener's startup replay recovers replies that race with its own startup. The [message listen contract](https://github.com/himkt/cafleet/blob/main/SPEC.md#message-listen) specifies the listener's records, and the [host evidence contract](https://github.com/himkt/cafleet/blob/main/SPEC.md#host-evidence-and-native-input-authorization) states each backend's reception capabilities and evidence limits.

### Create and dispatch a member

Save this prompt as `/home/cafleet-demo/work/demo/.prompts/member.md` with a fresh filename. Keep identity placeholders and double other literal braces:

```text
You are an ordinary CAFleet member.
ROLE DEFINITION: Open /home/cafleet-demo/.claude/skills/cafleet/roles/member.md BEFORE any other action. Follow that role definition.
Read /home/cafleet-demo/.claude/skills/cafleet/reference/coding-agents.md and resolve your backend, then load /home/cafleet-demo/.claude/skills/cafleet/SKILL.md as an assigned member. Follow the BASE section in /home/cafleet-demo/.claude/skills/cafleet/roles/member.md and the supplied host instructions before task work.
FLEET ID: {fleet_id}
DIRECTOR MEMBER ID: {director_member_id}
YOUR MEMBER ID: {member_id}
BASE: /home/cafleet-demo/work/demo
CODING AGENT: {coding_agent}
DATABASE URL: {database_url}
When DATABASE URL is not default, pass --database-url <that URL> on every cafleet command.
As your first operational broker command, send: cafleet message send --from-member-id {member_id} --to-member-id {director_member_id} "ready"
Poll and ACK your assignment, perform it, poll before reporting completion and on every wake, then handle queued assignments before becoming idle.
```

```bash
cafleet member create --fleet-id 1 --name alice --description "Independent review" --file /home/cafleet-demo/work/demo/.prompts/member.md
```

Suppose the returned ID is `3`. Read its ready message, ACK its actual message ID, then dispatch its assignment:

```bash
cafleet message send --from-member-id 2 --to-member-id 3 "Review the change and report findings."
```

The send persists the assignment and reports how the runner delivered its preview ([delivery methods](concepts/members.md#deliver-generated-input)); the member's startup, report and wake polls also discover durable assignments.

### Receive results

The Director reads each message record from its listener; the installed CAFleet skill's backend recipe owns the tool calls, and reception sends no input into the Director's pane. On every wake, reconcile full pending payloads:

```bash
cafleet message poll 2 --json
```

ACK every consumed delivery before ending the turn, using its returned ID:

```bash
cafleet message ack 10
```

Keep deferred questions and approvals in host-owned task state after ACK, and reconcile replayed IDs before acting; broker events leave pending user decisions pending. The [message listen](https://github.com/himkt/cafleet/blob/main/SPEC.md#message-listen) and [Director consumption](https://github.com/himkt/cafleet/blob/main/SPEC.md#director-consumption-and-continuation) contracts own the full procedure.

The Director keeps its listener for the fleet's lifetime and renews one whose lease expired while the fleet exists. After an unexpected exit it reconciles the inbox and replaces the listener only once the old process has ended; unACKed messages survive damaged output. A Codex Director's `cafleet message listen --codex-thread` forwarder and the OpenCode and Hermes plugins deliver each record as a queued turn: they start a turn on an idle Director and wait for the end of a busy one ([host evidence](https://github.com/himkt/cafleet/blob/main/SPEC.md#host-evidence-and-native-input-authorization)).

On a backend whose [Director verdict](https://github.com/himkt/cafleet/blob/main/SPEC.md#role-eligibility) is `yes` but whose listener tool is unavailable, the Director states that reception limit and uses manual `message poll` or a finite receive within host permissions. A finite receive also retries ordinary-member previews:

```bash
cafleet message wait 2 --timeout 300 --json
```

### Handle an observed permission request

Handle the request as [Consume a runtime event](concepts/members.md#consume-a-runtime-event) describes. The Director decides allow or deny itself only when its own session runs in verified auto mode; otherwise it obtains your permission, and it answers manually on the member's native surface when answer coverage is unavailable.

### Run a command for a member

A member can ask `Need to run: mise //:test. My harness denied it.` Authorize and dispatch the exact command:

```bash
cafleet member exec 3 --from-member-id 2 "mise //:test"
```

The CLI returns submission metadata. Inspect the native command and result:

```bash
cafleet member capture 3 --lines 80
```

Once the command has finished, explicitly continue a member that has not resumed:

```bash
cafleet member prompt 3 --from-member-id 2 "Use the native command output and continue your assignment."
```

Inspect failed or uncertain input before authorizing any retry. [Command routing](concepts/members.md#command-routing) explains the evidence boundary.

### Close the fleet

Consume the final inbox and complete approval gates, then delete the fleet; [teardown](concepts/members.md#teardown) describes how it ends each remaining member:

```bash
cafleet fleet delete 1
```

```bash
cafleet fleet list
```

Fleet deletion closes the listener with reason `fleet_deleted` ([message listen](https://github.com/himkt/cafleet/blob/main/SPEC.md#message-listen)), the listener's only normal end; the Director confirms that the listener exited and starts no replacement. Pending messages stay available for later reads.

Fleet deletion retains history. When `cafleet fleet delete` exits 1, the fleet stays registered and the Director keeps its listener; resolve the reported cause and rerun `cafleet fleet delete`. Limit cleanup to the team you own.

## Backend variations

Use explicit member backend/model flags to create a mixed team. Resolve role defaults and models through [model choice](concepts/coding-agents.md#model-choice). OpenCode members perform assignments through their selected provider/model, workspace tools, broker and native shell path. Hermes members use your configured default Hermes model unless you name one, and need the [Hermes setup](quickstart.md#hermes) before their first spawn. Each member receives its own installed role/core paths, shared BASE and host prerequisites.
