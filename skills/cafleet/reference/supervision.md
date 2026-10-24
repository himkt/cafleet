# CAFleet Team Supervision

The Director owns the delegated task from assignment through evidence-backed completion. Use independent CAFleet sessions when work calls for subagents or agent teams; preserve the host's existing continuation, authorization and user gates.

## Core Principle

The host continues its own task and owns native tool/background work. CAFleet registers sessions, persists messages, delivers each message into its recipient's pane and submits authorized input. The Director retains outstanding work in host-owned task state and collects broker, artifact and pane evidence.

## Communication Model

Use point-to-point `cafleet message send` for coordination. Fan out with an individual send to each selected recipient, retaining each committed message ID and reporting recipient failures. Retrieve complete payloads with `--json`; long/multiline bodies use `--file <path>`.

Read every delivery, record any deferred action with its ID/artifact pointer in host-owned task state, then ACK it immediately. ACK means consumption; a pending user decision or unfinished assignment remains tracked outside the inbox. ACK every consumed delivery before ending the turn.

## Spawn Protocol

1. Read [Director policy](../roles/director.md), the executing backend in [coding-agents.md](coding-agents.md), BASE and supplied host rules. Resolve selected member backend/model/effort before writing its prompt.
2. Run `cafleet doctor --json` and resolve actual environment/asset/permission issues. Preserve the user's host continuation controls as supplied.
3. Register the current Director: `cafleet fleet create --name <name> --coding-agent <backend> --json`. Capture literal fleet and Director IDs. This performs one atomic registration without starting another process.
4. Write scoped role-by-path prompts under the supplied BASE, retaining host prerequisites and literal identity placeholders. Create each member with `cafleet member create --fleet-id <fleet-id> ... --file <absolute-path>`.
5. Receive each member's ready and ACK it. Dispatch that member's assignment as soon as its inputs exist; other members' readiness does not delay it.
6. End the turn after dispatch; a delivered line wakes you, and you follow [Director wake](#director-wake) until work resolves or an explicit user handoff occurs.

### Director wake

Every message addressed to you is delivered into your pane as one `[cafleet msg ID from SENDER TIMESTAMP]` line, led by Escape. Your pane is the one that ran `cafleet fleet create`, so register from your coding-agent session.

1. After dispatching work, continue independent authorized work or end your turn. Without an active host continuation, ending the turn with outstanding delegated work is the normal waiting state.
2. A delivered line wakes you. The inline body is the full message; a pointer line names the `cafleet message show ID --json` command for the full body.
3. On every wake and before ending a turn, run `cafleet message poll <director-member-id> --json`, which also surfaces messages whose delivery failed. Record deferred actions in host-owned task state, ACK each consumed delivery and act on it.
4. On every wake, run `cafleet member list <fleet-id> --json` and check each member's `unconfirmed_previews` and the assignments past their expected reply point, using `cafleet member capture` / `cafleet fleet capture` and artifacts as evidence.
5. For each ID in a member's `unconfirmed_previews`, capture the member pane. When the line is absent, or the capture cannot establish it, run `cafleet member prompt <member-id> "cafleet message poll <member-id> --json"`. The member ACKs the message normally, which removes it from the list.
6. Retain the operator's decision gates. A delivery that dismissed an open decision-surface question calls for re-asking it.

A delivery's Escape can interrupt your running turn; its resume clause tells you to continue that work. Text the operator typed but did not submit in your pane can merge with a delivered line; relay it to the operator rather than acting on the merged input.

#### Waiting under host continuation

A host continuation is an active Claude Code `/goal` condition or an active Codex thread objective. While one is active, the host starts another Director turn as soon as a turn ends with the goal unmet, and consecutive tool-free Claude Code turns reach the block cap. During an active host continuation, end each continued turn that has outstanding delegated work with one bounded foreground receive, then apply steps 3–6:

```bash
cafleet message wait <director-member-id> --timeout 30 --json
```

The 30-second timeout paces the loop and keeps every continued turn tool-bearing. A delivery that arrives during the wait still reaches your pane; the wait returns the same persisted message. Read host-specific evidence and limits in [coding-agents.md](coding-agents.md).

#### Sender follow-up

When your `cafleet message send` prints `Message N was persisted. …` on stderr, send exactly one follow-up to the same recipient with the body `message N was not delivered to your pane; poll your inbox`. Its drain delivers an unclaimed `N` first; for a durable claim, its line wakes the recipient, whose poll lists `N`. When the follow-up also prints `Message M was persisted. …`, inspect the target pane and continue with step 5's `member prompt`.

## Idle Semantics

Classify fresh captured evidence using the target's backend cues. Captures guide your next action only; delivery and dispatch never wait on them, and an idle composer does not establish task completion. A retained draft can merge with a later delivery or prompt.

| Evidence | Director response |
|---|---|
| Active tool, streaming or native shell work | End the turn; a delivered line or the operator wakes you |
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
5. End the turn while delegated work remains, until completion, a required user gate or explicit handoff.

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
3. Run `cafleet fleet delete <fleet-id>`. This deregisters the Director and registry without closing member panes.
4. Run `cafleet fleet list` and confirm the fleet is absent. Limit cleanup to fleets covered by this task's authorization.
