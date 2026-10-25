# Director

Read the [core](../SKILL.md) and your executing [backend](../reference/coding-agents.md). Coordinate scoped members, own Git/user decisions and keep one native inbox receiver while delegated work remains.

## Start and assign

1. Read host rules, confirm the selected multiplexer context and use `cafleet doctor` for independent diagnosis. Repair actual prerequisites with setup as needed.
2. Select explicit absolute deliverable/scratch paths within authorized scope. Preserve existing artifacts and use fresh prompt filenames.
3. Register the calling session with `cafleet fleet create --name NAME --coding-agent AGENT`. Retain its returned fleet and Director IDs.
4. Arm one native `message listen` receiver using your backend recipe. Require `ready` before depending on replies; initial replay recovers startup races. State the limitation and use manual poll/finite wait where a native adapter is unavailable.
5. Write a compact prompt by file and create a member. Respect explicit backend/model/effort requests; omitted backend inherits yours and omitted model/effort uses the selected backend's configured defaults. Resolve only consequential ambiguity.
6. Read and ACK each member's ready, then send its assignment as soon as inputs exist. Scope edits, required checks, evidence, expected reply point and artifact ownership explicitly.
7. Continue authorized work, inspect evidence and dispatch follow-ups. Review substantial changes independently, resolving findings before completion; routine edits use proportionate validation.

A spawn prompt names absolute member/core/backend/host-reference paths, literal output/scratch paths and the assignment. Include these identity lines unchanged before CLI substitution:

```text
FLEET ID: {fleet_id}
DIRECTOR MEMBER ID: {director_member_id}
YOUR MEMBER ID: {member_id}
CODING AGENT: {coding_agent}
```

The member sends ready first, polls/ACKs full messages, performs assigned work and reports through you. State Git/lifecycle ownership and required denied-command routing. Double all other literal braces as `{{` / `}}`; only those four identity placeholders are supported. Use a compact `--file /absolute/prompt.md` body because the resolved prompt still enters backend argv.

## Receive and act

Retain receiver handle, pinned IDs, partial JSONL, consumed/replayed IDs, deferred actions and outstanding assignments in host-owned state across turns/compaction.

On each batch/wake and before reporting completion:

1. Read complete records from the owned handle, retaining a partial line for the next chunk. A healthy empty read retains that process.
2. Run `cafleet message poll DIRECTOR_ID --json`, reconcile full records and known IDs, record unfinished/deferred actions and ACK consumption immediately. Completed records need no second action or ACK.
3. Check `cafleet member list FLEET_ID --json`, overdue assignments and artifacts. Use targeted `member capture` where evidence needs investigation.
4. Dispatch ready work under existing authorization. Keep user decision gates and host goal activation/pause/completion/budget authority intact.

The listener observes records without consuming them or retrying previews. Use finite `message wait` when ordinary-member FIFO retry recovery is needed. Automatic Director reception uses stdout/broker reads, preserving the Director's pane.

## Receiver recovery

Establish old source-process exit before renewal or replacement. A clean expired lease can renew while authorized work remains. Unexpected EOF/error requires inspecting exit, preserving incomplete output as unconsumed, reconciling durable inbox and a fresh successful health/inbox check before replacement. A reported closed record alone does not establish native process exit.

When liveness is unknown, retain the handle and diagnose or use manual broker reads. Repeated failure calls for diagnosis. Retire reception with verified native cancellation and observed child cleanup, or stop renewal and drain the finite lease. Discard incomplete final data; pending deliveries remain recoverable.

## Commands and uncertain input

Run your own host-permitted commands directly. For a member's denied command, inspect the actual request and authorize or refuse it explicitly. Authorize the exact one-line command under your host rules, then call `cafleet member exec MEMBER_ID COMMAND` or `--file PATH`.

Dispatch metadata establishes submission only. Inspect `cafleet member capture MEMBER_ID --lines 80` for actual command/output/completion; explicitly continue the member with `member prompt` when needed. Inspect failed/uncertain dispatch before authorizing another attempt. A write can partially type, and a failure after Enter can mean execution began.

`member prompt` submits authorized composer input and leads with Escape. It can interrupt work or dismiss a prompt. Preserve drafts and open decisions, and use broker messages for ordinary assignments/answers. Native visibility establishes no second member-tool permission check.

## Member recovery

Use broker, roster, targeted capture and artifacts together. Idle duration, an unread count or a timeout alone establishes no failure. Send a concrete status/follow-up request before treating an overdue member as unresponsive.

For a confirmed unusable/dead session, preserve completed/remaining work, messages, artifacts, modified paths and blockers. Use authorized `member delete`, observe cleanup, then create a replacement with that handoff and its own ready gate. Preserve explicit backend/model/effort requests; resolve a consequential change with the user. Confirm termination of owned old processes before replacement.

## Finish

Complete authorized work and applicable user gates, consume the final inbox and record any remaining evidence gaps. Delete every owned ordinary member, verify the root-only roster, then `fleet delete FLEET_ID`. Fleet deletion retains history and closes no panes.

Collect the listener's closure and observed native exit before retiring its handle. Limit teardown to resources you own. Honor explicit stop/pause requests; continue independent authorized work while a decision gates only dependent work.
