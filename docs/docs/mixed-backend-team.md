# Run a fleet

Create independent coding-agent sessions in tmux or herdr, assign work and receive durable replies as previews typed into the Director's pane.

## Prerequisites

Follow Quickstart to [install](quickstart.md#install) and [configure](quickstart.md#configure), then resolve any native [working-directory consent](concepts/coding-agents.md#working-directory-trust). Put each selected backend on PATH. Use literal returned IDs and one CAFleet command per shell-tool invocation.

## Prompt

```text
Use CAFleet to delegate independent reviews of this change, collect both reports, resolve their findings, and close the team.
```

Your agent loads the Director instructions, creates members and assigns each after its ready, then consumes broker events and checks evidence, preserving the host's supplied continuation and user gates.

## Manual lifecycle

This example uses HOME `/home/cafleet-demo` and workspace `/home/cafleet-demo/work/demo`. Substitute actual absolute paths and the installed skill root, including any [relocated config directory](quickstart.md#configure).

```bash
cafleet doctor --json
```

Resolve diagnosis issues. Load the Director role, backend reference and supervision procedure from the installed CAFleet skill, then register the calling session:

```bash
cafleet fleet create --name demo --coding-agent claude
```

Example output: `1 director=2`. Creation registers the current pane and its multiplexer server without starting another session, and the backend flag declares the calling coding agent; every later command targets that pane and server. When CAFleet cannot confirm the Director's pane, as for a Hermes Director and usually a Codex Director, it asks for the pane ID and server, and the Director reruns with `--pane <pane-id> --mux-server <server>` ([Director pane verification](concepts/members.md#director-pane-verification)). From then on, the fleet's member runners type each message addressed to the Director into this pane; the [host evidence contract](https://github.com/himkt/cafleet/blob/main/SPEC.md#host-evidence-and-native-input-authorization) states how each backend takes a preview during a running turn.

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
Poll your assignment and record it, perform it, poll before reporting completion and on every wake, then handle queued assignments before becoming idle.
```

```bash
cafleet member create --fleet-id 1 --name alice --description "Independent review" --file /home/cafleet-demo/work/demo/.prompts/member.md
```

Suppose the returned ID is `3`. Receive its ready message through your poll, then dispatch the assignment:

```bash
cafleet message send --from-member-id 2 --to-member-id 3 "Review the change and report findings."
```

The send persists the assignment and reports how the runner delivered its preview ([delivery methods](concepts/members.md#deliver-generated-input)); the member's startup, report and wake polls also discover durable assignments.

### Receive results

Each message addressed to the Director arrives in its pane as one preview opening with `[cafleet msg ID …]`, typed by a live member runner of the fleet, oldest first across senders. The preview is the full message when the pane fits it, and otherwise a shorter form down to `[cafleet msg ID]` alone ([Pane size](concepts/members.md#pane-size)). A preview starts a turn when the Director is idle, and on each one, whatever its form, the Director runs its poll once:

```bash
cafleet message poll 2 --json
```

The poll consumes every message it returns, so the Director records each one's sender, required action and artifact pointers before running any other command. A preview whose message an earlier poll already consumed returns `[]`, which is normal. Keep deferred questions and approvals in host-owned task state; broker events leave pending user decisions pending. `message show MESSAGE_ID --json` reads a consumed message again. The [Director consumption](https://github.com/himkt/cafleet/blob/main/SPEC.md#director-consumption-and-continuation) contract owns the full procedure.

A draft in the Director's composer, or an open approval or question dialog in its pane, holds every preview until you clear the draft or answer the dialog, and the oldest held message blocks newer ones. Keep the composer clear between turns. [Director requirements](concepts/coding-agents.md#director-requirements) lists how each backend takes a preview during a running turn.

Previews need at least one live member runner; messages sent while none is live stay pending until a runner starts. The Director therefore polls by hand at these points:

| Point | Reason |
|---|---|
| After a `member delete` that leaves the fleet with no active member | The deleted member's last report has no runner to deliver it |
| Before `fleet delete` | Final inbox consumption |
| At each wake's overdue-assignment check | An unexpected exit of the last member surfaces at the next Director turn |
| After a Director session restart or context loss | Previews typed before the restart may be missing from the new context |

On a backend whose Director screen pane delivery does not recognize, nothing is typed into the Director's pane, and the Director reads by manual `message poll` within host permissions. Each member's runner retries that member's undelivered previews on its own, so no Director command is needed for them.

### Handle an observed permission request

Handle the request as [Consume a runtime event](concepts/members.md#consume-a-runtime-event) describes. The Director decides allow or deny itself only when its own session runs in verified auto mode, otherwise it obtains your permission; without answer coverage the request is answered manually on the member's native surface.

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

Run a final `message poll`, complete approval gates, then delete the fleet; [teardown](concepts/members.md#teardown) describes how it ends each remaining member:

```bash
cafleet fleet delete 1
```

```bash
cafleet fleet list
```

Fleet deletion retains history, and pending messages stay available for later reads. When `cafleet fleet delete` exits 1, resolve the reported cause and rerun it. Limit cleanup to the team you own.

## Backend variations

Create a mixed team with explicit member backend/model flags, resolving role defaults and models through [model choice](concepts/coding-agents.md#model-choice). OpenCode members work through their selected provider/model, workspace tools, broker and native shell path. Hermes members use your configured default Hermes model unless you name one ([Hermes configuration](concepts/coding-agents.md#hermes-configuration)). pi members likewise use your configured pi default model, refuse a `:<thinking>` suffix in `--model`, and run under the CAFleet member gate installed by shared setup ([pi configuration](concepts/coding-agents.md#pi-configuration)). Each member receives its own installed role/core paths, shared BASE and host prerequisites.
