# CAFleet Team Supervision

The Director owns the delegated task from assignment through evidence-backed completion. Use independent CAFleet sessions when work calls for subagents or agent teams; preserve the host's existing continuation, authorization and user gates.

## Core Principle

The host continues its own task and owns native tool/background work. CAFleet registers sessions and persists messages. Ordinary members receive automatic pane previews; the root Director receives full broker records through native stdout tools. The Director retains its receiver handle and outstanding work in host-owned task state and collects broker, artifact and pane evidence.

## Communication Model

Use point-to-point `cafleet message send` for coordination. Fan out with an individual send to each selected recipient, retaining each committed message ID and reporting recipient failures. Retrieve complete payloads with `--json`; long/multiline bodies use `--file <path>`.

Read every delivery, record any deferred action with its ID/artifact pointer in host-owned task state, then ACK it immediately. ACK means consumption; a pending user decision or unfinished assignment remains tracked outside the inbox. Reconcile replayed or already-known IDs through `poll --json` or `show --json` before acting; completed records need no second action or ACK. Stream output and native event delivery leave consumption unchanged.

## Spawn Protocol

1. Read [Director policy](../roles/director.md), the executing backend in [coding-agents.md](coding-agents.md), BASE and supplied host rules. Resolve selected member backend/model/effort before writing its prompt.
2. Run `cafleet doctor --json` and resolve actual environment/asset/permission issues. Preserve the user's host continuation controls as supplied.
3. Register the current Director: `cafleet fleet create --name <name> --coding-agent <backend> --json`. Capture literal fleet and Director IDs. This performs one atomic registration without starting another process.
4. Arm one native `message listen` receiver using the executing backend's [runtime recipe](coding-agents.md). Retain its handle and require `ready` before depending on replies; initial replay recovers messages racing with startup. When a verified native adapter is unavailable, state the limitation and use explicit manual `poll`/finite `wait` within host permissions.
5. Write scoped role-by-path prompts under the supplied BASE, retaining host prerequisites and literal identity placeholders. Create each member with `cafleet member create --fleet-id <fleet-id> ... --file <absolute-path>`.
6. Receive each member's ready and ACK it. Dispatch that member's assignment as soon as its inputs exist; other members' readiness does not delay it.
7. Keep native reception active and follow [Director wake](#director-wake) until work resolves or an explicit user handoff occurs.

### Director wake

Receive Director messages through broker reads and stdout only. Automatic reception sends no Escape, text, Enter or queued composer input to the Director, including on startup failure, receiver loss or missing native tools. Backend recipes own concrete tool calls; preserve the supplied host continuation state and user decision gates.

1. Retain one owned native handle while assignments or an authorized user-feedback phase remain outstanding. Store its pinned member/fleet IDs, receive status, incomplete JSONL and unfinished actions across context compaction.
2. Consume complete `ready`, `message` and `closed` records from that handle. Preserve incomplete chunks for the next read. A healthy live read with no output keeps the same process; receiving a message supplies no reason to replace it.
3. On every received batch/wake and before reporting completion, run `cafleet message poll <director-member-id> --json`. Read full bodies, reconcile known/replayed IDs, record deferred actions, ACK consumed deliveries and dispatch work whose inputs are ready.
4. On every wake, run `cafleet member list <fleet-id> --json` and check each member's `unconfirmed_previews` and the assignments past their expected reply point, using `cafleet member capture` / `cafleet fleet capture` and artifacts as evidence.
5. For each ID in an ordinary member's `unconfirmed_previews`, capture its pane. When the line is absent, or the capture cannot establish it, run `cafleet member prompt <member-id> "cafleet message poll <member-id> --json"`. Consume the Director's own pending IDs through broker reads. Explicit finite `wait` remains available when ordinary-member retry rotation is needed; the listener performs no preview retries.
6. Keep required user decisions pending until the operator answers. Incoming broker records are events, not user answers. Continue independent authorized work and reception where the host permits it; when a decision blocks host tools, reconcile the inbox on the next authorized activation.

#### Receiver recovery

| Observed state | Director response |
|---|---|
| Live handle, empty read | Retain the handle and read it again when appropriate |
| Clean `closed` with `timeout` | Establish task exit, then renew only while authorized delegated work remains |
| `member_inactive` / `fleet_deleted` | Retire the handle; resume only with an explicitly established new task/identity |
| Unexpected EOF, failed native call or receiver exit | Inspect output/exit and complete a fresh successful broker health/inbox check before replacing the receiver |
| Fatal database/schema/assets error | Surface the cause and follow the existing repair/authorization procedure before restarting |
| Truncated output or incomplete JSONL | Preserve complete records, leave incomplete messages unACKed and retrieve full output or reconcile/replay through the broker |
| Old receiver liveness unknown | Preserve its handle and diagnose or use explicit manual reads; establish termination before replacing it |
| Native stream tool unavailable or denied | State the limitation and use manual `poll`/finite `wait` within host permissions |

Retain pending messages across recovery. Repeated receiver failures require diagnosis; each replacement requires observed termination and a fresh successful health/inbox check. Preserve the host's goal activation, pause, completion and budget authority: reception never calls a goal lifecycle tool, starts a goal, resumes a paused goal or changes its budget. Follow the backend's native receive lifecycle independently of whether a host goal was supplied.

#### Sender follow-up

When an ordinary-recipient `cafleet message send` prints `Message N was persisted. …` on stderr, send exactly one follow-up to the same recipient with the body `message N was not delivered to your pane; poll your inbox`. Its drain delivers an unclaimed `N` first; for a durable claim, its line wakes the recipient, whose poll lists `N`. When the follow-up also prints `Message M was persisted. …`, inspect the target pane and continue with step 5's `member prompt`.

## Idle Semantics

Classify fresh captured evidence using the target's backend cues. Captures guide your next action only; delivery and dispatch never wait on them, and an idle composer does not establish task completion. A retained draft can merge with a later delivery or prompt.

| Evidence | Director response |
|---|---|
| Active tool, streaming or native shell work | Continue independent work and the owned native receive lifecycle |
| Explicit question/approval | Relay a received question or inspect the complete prompt |
| Idle empty composer with unfinished work | Check messages/artifacts and send the next concrete instruction |
| Retained draft | Relay the capture and ask the user how to preserve or submit it before prompting |
| Unknown/truncated/unrecognized capture | Inspect more deeply before choosing an action |
| Missing/dead pane or concrete failure | Recover within existing authorization |

A timeout, idle duration or unread count alone proves no failure. Expected ready/reply deadlines belong to the assignment and are checked by the Director.

## Authorization-Scope Guard (CRITICAL)

Carry the user's authorization forward across wakes and checks. Dispatch already authorized work when inputs arrive; an inbox check renews no permission. Keep explicit user review/approval gates authoritative. Relay a new decision only when its meaning, scope or risk requires the operator.

### Real stop signals (treat as halt; everything else is a tick to evaluate)

Explicit cancellation, a required user decision, a concrete unrecoverable failure or a workflow gate changes what can proceed. Track that state and continue independent authorized work when available. Preserve the host's objective, activation authority and budget as supplied.

### When you genuinely need user input

Use the executing backend's decision surface for a concrete choice. State the evidence and pending action. A member routes its question through the Director; the Director relays the answer faithfully.

## Team-facilitation instructions

At every wake and active workflow boundary:

1. Read full broker payloads and artifacts; ACK every consumed message.
2. Record deferred user decisions and remaining actions in host-owned state.
3. Dispatch ready work immediately.
4. Inspect overdue/failed work using roster, `unconfirmed_previews`, targeted captures and artifacts.
5. Keep the backend's native receive lifecycle active until completion, a required user gate or explicit handoff.

### Routing member command requests

A member runs host-permitted commands and sends required denied commands through [prompt-routing](prompt-routing.md). Authorize native dispatch, inspect the actual result and explicitly continue the member after completion evidence when its host has not resumed it. Process requests in inbox order and close every request with an action or reasoned reply.

## Stall Response

Start with your inbox and assignment/artifact state, then use a read-only targeted capture. A quiet/ambiguous pane calls for inspection rather than an inferred failure.

### Stage 1 — Message-based check (`cafleet message poll`)

Use `cafleet message poll <director-member-id> --json`, record unfinished actions, ACK each delivery and act on it.

### Stage 2 — Terminal capture fallback (`cafleet member capture`)

Use `cafleet member capture <member-id> --lines <depth>` for complete backend UI evidence. `cafleet fleet capture <fleet-id>` supplies a one-shot roster-wide diagnostic. Capture is safe while the member works.

### Escalation

When two actually delivered concrete re-engagement messages produce no progress and captures establish an unresponsive session, relay the evidence and recovery choices through the host decision surface. Pending previews and `unconfirmed_previews` entries do not count as delivered. Active work and pending user decisions retain their own handling.

## User Delegation Protocol

Relay explicit member questions through the executing backend's decision surface, then send the user's answer verbatim. A capture alone supplies evidence, not authority to answer a prompt. Interpret free-form replies by intent; record ordinary revision requests through the consuming workflow's coordination format and preserve its gates.

## Cleanup Protocol

Read [Shutdown](#shutdown) immediately before cleanup. Delete authorized ordinary members, verify the root-only roster, deregister the fleet and confirm closure. Preserve workflow-specific review/approval preconditions.

## Recovery

Read this section immediately before recovery. Run `cafleet doctor` and check inbox/roster/pane/artifact evidence. Use `cafleet member capture` before deciding that an agent is dead or unusable. Preserve retained drafts and relay them to the user.

Once evidence confirms a missing/dead pane or an authorized unusable-session replacement, use `cafleet member delete`, then `cafleet member create` with a context-preserving prompt. Carry the assignment, full message IDs, completed/remaining work, artifacts and blocker context. The replacement gets a new ID and its own ready gate. Persistently unknown input stays untouched while inspected or relayed.

A multiplexer failure calls for diagnosis of the calling context and target placement. Registry existence alone proves no physical pane presence. Resolve connectivity before selecting destructive recovery.

## Shutdown

Use CAFleet primitives within the task's authorized scope.

1. Delete every owned ordinary member with `cafleet member delete <member-id>`. The operation kills its pane and deregisters it while serializing input ownership.
2. Run `cafleet member list <fleet-id>`; only the root Director should remain. Resolve failed member teardown before proceeding.
3. Consume the final inbox and complete the workflow's approval gates, then run `cafleet fleet delete <fleet-id>`. This deregisters the Director and registry without killing its pane.
4. Collect the listener's `fleet_deleted` closure and native task exit, then retire its handle. If retiring reception while the fleet stays live, use verified native cancellation with observed termination, or stop renewal and drain its finite lease. Discard an incomplete final line and preserve pending messages for later reads.
5. Run `cafleet fleet list` and confirm the fleet is absent. Limit cleanup to fleets covered by this task's authorization.
