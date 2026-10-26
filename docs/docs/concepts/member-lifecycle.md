# Member lifecycle

Register the calling session as Director, then create independent member panes and dispatch each assignment after its ready signal.

## Registration and readiness

`fleet create` registers the current Director atomically. `member create` inherits that Director's backend unless overridden, validates model/effort, native policy, evidence policy and prompt identity, then registers a pending placement/run and starts the owned PTY runner. Creation retains its guards through the prepared/start/started handshake. Creation compensates for failures while preserving the original cause and any cleanup error.

Each member reads its role and required host rules, sends `ready` as its first operational broker command, and immediately polls its inbox. The Director can assign that member as soon as its ready arrives. The member polls before completion reports and on every subsequent wake, then handles queued assignments before becoming idle.

## Assignment and receiving

Messages are point-to-point. Send individually to every selected recipient. Runner-managed members receive previews only through eligible native composer input; existing unwrapped members retain Escape-first previews. The root Director uses a persistent native inbox stream or explicit manual broker reads. Record deferred actions and ACK consumption, retaining unfinished work in host-owned task state. Arm Director reception before relying on ready replies and collect the receiver's exit during teardown. See [run a fleet](../how-to/mixed-backend-team.md#manual-lifecycle).

## Command routing

A member runs commands its host permits and routes a required denied command to the Director. The Director authorizes `member exec`, which visibly submits the actual native shell command and returns submission metadata. Inspect native output with `member capture`; after completion, send an explicit `member prompt` to continue if the host has not resumed the member. A held native interaction requires the human’s original-surface answer, followed by actual result and continuation checks. Inspect a failed or uncertain dispatch and its durable input receipt before any newly authorized retry. [Member permission events](member-permission-events.md) explains the complete event/answer/retry sequence.

The [native dispatch contract](../spec/cli-options.md#member-exec) owns validation, lock and uncertainty errors. File-edit permission gaps need a matching host rule or a Director edit.

## Inspection and recovery

Use `member list`, `member capture` and `fleet capture` to check roster and pane evidence. Capture about 20 lines with `member capture` (its default) and read the content directly; use `--lines` for more when it is insufficient. An empty composer does not mean the task is complete — judge completion from messages and artifacts. A timeout alone establishes neither failure nor completion. Replace an unusable member only under the existing authorization, carrying its assignment, messages and artifacts into the replacement prompt.

## Teardown

Delete owned ordinary members before deleting the fleet. Runner-managed deletion coordinates shutdown and event recovery under the existing pane locks, retaining unconfirmed child cleanup and recoverable artifacts explicitly. Member deletion kills its owned pane and deregisters it; fleet deletion deregisters the registry without closing panes. Both serialize placement changes with pane input and fail without registry changes if lock contention lasts five seconds. The root Director is protected from individual member deletion.
