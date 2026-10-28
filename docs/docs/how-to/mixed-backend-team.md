# Run a fleet

Create independent coding-agent sessions in tmux or herdr, assign work and receive durable replies through the Director's native inbox tools.

## Prerequisites

Follow Quickstart to [install](../quickstart.md#install), [configure](../quickstart.md#configure) and [trust the workspace](../quickstart.md#trust-the-working-directory). Put each selected backend on PATH. Use literal returned IDs and one CAFleet command per shell-tool invocation.

## Prompt

```text
Use CAFleet to delegate independent reviews of this change, collect both reports, resolve their findings, and close the team.
```

Your agent loads the Director instructions, arms native reception, creates members and assigns each after its ready. The Director consumes broker events and checks evidence while preserving the host's supplied continuation and user gates.

## Manual lifecycle

This example uses HOME `/home/cafleet-demo` and workspace `/home/cafleet-demo/work/demo`. Substitute actual absolute paths and the installed skill root. [Config-dir resolution](../spec/cli-options.md#config-dir-resolution) owns backend path overrides.

```bash
cafleet doctor --json
```

Resolve diagnosis issues. Load the Director role, backend reference and supervision procedure from the installed CAFleet skill, then register the calling session:

```bash
cafleet fleet create --name demo --coding-agent claude
```

Example output: `1 director=2`. Creation registers the current pane without starting another session. The backend flag declares the calling coding-agent host. Before creating members, arm one native receiver using the installed CAFleet skill's backend and supervision instructions. The [backend reference](../spec/coding-agent-backends.md) explains host capabilities and evidence limits. Require the receiver's ready record and retain its native handle; initial replay recovers replies racing with startup.

### Create and dispatch a member

Save this prompt as `/home/cafleet-demo/work/demo/.prompts/member.md` with a fresh filename. Keep identity placeholders and double other literal braces:

```text
You are an ordinary CAFleet member.
ROLE DEFINITION: Open /home/cafleet-demo/.claude/skills/cafleet/roles/member.md BEFORE any other action. Follow that role definition.
Read /home/cafleet-demo/.claude/skills/cafleet/reference/coding-agents.md and resolve your backend, then load /home/cafleet-demo/.claude/skills/cafleet/SKILL.md as an assigned member. Read /home/cafleet-demo/.claude/skills/cafleet/reference/base-dir.md and the supplied host instructions before task work.
FLEET ID: {fleet_id}
DIRECTOR MEMBER ID: {director_member_id}
YOUR MEMBER ID: {member_id}
BASE: /home/cafleet-demo/work/demo
CODING AGENT: {coding_agent}
As your first operational broker command, send: cafleet message send --from-member-id {member_id} --to-member-id {director_member_id} "ready"
Poll and ACK your assignment, perform it, poll before reporting completion and on every wake, then handle queued assignments before becoming idle.
```

```bash
cafleet member create --fleet-id 1 --name alice --description "Independent review" --file /home/cafleet-demo/work/demo/.prompts/member.md
```

Suppose the returned ID is `3`. Receive its ready, ACK its actual message ID, then dispatch its assignment:

```bash
cafleet message send --from-member-id 2 --to-member-id 3 "Review the change and report findings."
```

The send persists the assignment and submits a preview when the member’s recorded transport permits input. A runner holds input during candidate/open interactions, work, drafts, uncertain receipts or unknown coverage; explicitly unwrapped members retain Escape-first input. A later send or finite `message wait` rotation retries eligible pending previews. The member's startup/report/wake polls also discover durable assignments.

### Receive results

Read complete JSONL records from the same owned native receiver between events. Codex explicitly reads its retained session while delegated work remains; Claude Monitor may start a turn for each record. Backend recipes own the concrete tool calls. Native reception sends no automatic input into the Director's pane. On every batch/wake, reconcile full pending payloads:

```bash
cafleet message poll 2 --json
```

ACK every consumed delivery before ending the turn, using its returned ID:

```bash
cafleet message ack 10
```

Keep deferred questions/approvals in host-owned task state after ACK. Reconcile known/replayed IDs before acting; completed records need no second action or ACK. Broker events leave pending user decisions pending. On every batch/wake inspect roster, panes and artifacts; consume the Director's own pending IDs through broker reads.

A clean expired listener lease can renew after its native task exits while authorized work remains. Unexpected EOF or failure requires inspecting the exit, reconciling the inbox and a fresh successful broker health check before replacement. Retain incomplete JSONL across chunks and preserve unACKed messages after damaged output. Establish old-process termination before replacing a receiver. Fully idle Codex waits for the next user/native activation; a background process alone supplies no unsolicited per-line wake.

On a host satisfying the [Director requirements](../spec/coding-agent-backends.md#role-requirements), state an unavailable native adapter’s reception limit and use explicit manual poll or finite reception within host permissions. Finite wait also provides ordinary-member retry recovery:

```bash
cafleet message wait 2 --timeout 300 --json
```

### Handle an observed permission request

Use [Member permission events](../concepts/member-permission-events.md#consume-a-runtime-event) to verify the durable event/message link, record the unfinished action and ACK consumption. For a current permission request, evaluate its risk and use the Director session’s verified auto mode to decide allow/deny; otherwise obtain user permission. Submit the complete bound response command, give it to the user to execute, or use manual native input when answer coverage is unavailable. Inspect actual output and resumed work, consume/ACK available records, then explicitly invoke finite `message wait` to retry held previews; the listener alone performs no retry. A native command dispatched separately does not answer the original request.

### Run a command for a member

A member can ask `Need to run: mise //cafleet:test. My harness denied it.` Authorize and dispatch the exact command:

```bash
cafleet member exec 3 "mise //cafleet:test"
```

The CLI returns submission metadata. Inspect the native command and result:

```bash
cafleet member capture 3 --lines 80
```

Once the command has finished, explicitly continue a member that has not resumed:

```bash
cafleet member prompt 3 "Use the native command output and continue your assignment."
```

Inspect failed or uncertain input before authorizing any retry. [Command routing](../concepts/member-lifecycle.md#command-routing) explains the evidence boundary.

### Close the fleet

Consume the final inbox and complete approval gates. Delete each owned ordinary member, verify the remaining roster, then deregister:

```bash
cafleet member delete 3
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

Collect the listener's fleet-deleted closure and native task exit before retiring its handle. To stop reception while the fleet stays live, use verified native cancellation with observed termination or stop renewal and drain its finite lease. Preserve pending messages for later reads.

Fleet deletion retains history and closes no panes. Limit cleanup to the team you own.

## Backend variations

Use explicit member backend/model flags to create a mixed team. Resolve role defaults and models through [model choice](../concepts/coding-agents.md#model-choice). OpenCode members perform assignments through their selected provider/model, workspace tools, broker and native shell path. Each member receives its own installed role/core paths, shared BASE and host prerequisites.
