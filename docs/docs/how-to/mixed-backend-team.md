# Run a fleet

Create members from the Director's coding-agent session inside tmux or herdr. Follow [Quickstart](../quickstart.md) for installation, permissions and workspace trust. Use literal returned IDs and one CAFleet command per shell-tool invocation.

## Manual lifecycle

This example assumes workspace `/home/demo/work/project` and installed skills under `/home/demo/.claude/skills`. Substitute actual absolute paths and returned IDs.

```bash
cafleet doctor --json
```

Resolve actual prerequisites, load the installed Director/backend procedure, then register the calling session:

```bash
cafleet fleet create --name demo --coding-agent claude
```

Example output: `1 director=2`. Creation registers the current pane without spawning another session. Arm one native receiver before depending on replies; require its ready record and retain the native handle. [Backend behavior](../spec/coding-agent-backends.md#native-reception) explains host capabilities; installed skills own concrete tool recipes.

### Create and dispatch a member

Save a compact prompt at a fresh absolute path such as `/home/demo/work/project/.prompts/review.md`:

```text
You are a CAFleet member. Read these absolute paths before assignment work:
 /home/demo/.claude/skills/cafleet/SKILL.md
 /home/demo/.claude/skills/cafleet/roles/member.md
 /home/demo/.claude/skills/cafleet/reference/coding-agents.md
FLEET ID: {fleet_id}
DIRECTOR MEMBER ID: {director_member_id}
YOUR MEMBER ID: {member_id}
CODING AGENT: {coding_agent}
Workspace: /home/demo/work/project
Report path: /home/demo/work/project/.reports/review.md
Read repository host instructions. The Director owns Git, user decisions and lifecycle.
Send ready as your first operational broker command:
cafleet message send --from-member-id {member_id} --to-member-id {director_member_id} "ready"
Poll full assignments, ACK consumption, perform scoped work and report evidence.
```

Only the four identity placeholders are substituted; double other literal braces.

```bash
cafleet member create --fleet-id 1 --name review --description "Independent review" --file /home/demo/work/project/.prompts/review.md
```

Suppose it returns member `3`. Read and ACK ready, then send the scoped assignment:

```bash
cafleet message send --from-member-id 2 --to-member-id 3 "Review the parser diff; report correctness findings with source paths and evidence. Keep source unchanged."
```

Explicit backend/model/effort requests govern launches; omitted backend inherits the Director's and omitted model/effort uses that backend's configured defaults. Use CLI help for accepted flags.

### Receive results

Read complete records from the same owned native receiver. Codex explicitly reads its retained session while work remains; Claude Monitor can deliver records as host events. On every batch/wake reconcile durable state:

```bash
cafleet message poll 2 --json
```

Read full bodies, record deferred actions and ACK each consumed delivery using its actual ID:

```bash
cafleet message ack 10
```

ACK means consumption. Keep unfinished work and user decisions in host-owned task state. Reconcile known/replayed IDs before acting; completed records need no second action/ACK. Broker events leave user gates pending.

Inspect roster activity and overdue work, using targeted capture and artifacts when needed:

```bash
cafleet member list 1 --json
```

Idle time and a quiet pane alone establish no task completion or failure. Request concrete status before replacing an overdue member.

### Recovery

A healthy empty receiver read retains the same process. Establish source-process exit before renewing a clean expired lease. Unexpected EOF/error requires inspecting exit, reconciling inbox and a fresh successful health/inbox check before replacement. Retain incomplete JSONL across chunks; discard incomplete final data while preserving unconsumed messages.

Where a native adapter is unavailable, state the reception limit and use manual poll or finite wait. Wait also retries eligible ordinary-member notifications:

```bash
cafleet message wait 2 --timeout 300 --json
```

Fully idle Codex reconciles on the next user/native activation; a background process alone provides no per-line wake. Preserve host goals, budgets and open decisions.

A failed notification follows an already committed send. Consume that message ID once; use a later send/wait for FIFO preview recovery. Diagnose multiplexer context/placement before destructive recovery. For a confirmed unusable member, preserve artifacts, completed/remaining work and messages, delete its owned pane/registration, then create a replacement with the handoff and a fresh ready gate.

### Run a denied command

The member reports its exact necessary command and observed denial. Authorize the command under the Director's host rules:

```bash
cafleet member exec 3 "mise //cafleet:test"
```

Dispatch establishes submission only. Inspect actual native output/completion:

```bash
cafleet member capture 3 --lines 80
```

Continue a member explicitly when the host has not resumed it:

```bash
cafleet member prompt 3 "Use the native command output and continue your assignment."
```

Inspect uncertain input before authorizing a retry; partial typing or execution may already have occurred. Native shell visibility establishes no second member-tool permission check.

### Close the fleet

Complete authorized work/user gates and consume the final inbox. Delete every owned ordinary member:

```bash
cafleet member delete 3
```

Verify the remaining roster, then deregister the fleet:

```bash
cafleet member list 1
```

```bash
cafleet fleet delete 1
```

Collect listener closure and observed native exit before retiring its handle. To retire reception while the fleet stays live, use verified cancellation with observed child cleanup or stop renewal and drain its finite lease. Fleet deletion retains history and closes no panes. Limit cleanup to owned resources.
