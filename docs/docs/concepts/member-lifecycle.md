# Member lifecycle

Register the calling session as Director, then create independent member panes and dispatch each assignment after its ready signal.

## Registration and readiness

`fleet create` registers the current Director atomically. `member create` inherits that Director's backend unless overridden, validates model/effort and prompt identity, then registers a pending placement, splits a pane and patches its owned pane ID. Creation compensates for failures while preserving the original cause and any cleanup error.

Each member reads its role and required host rules, sends `ready` as its first operational broker command, and immediately polls its inbox. The Director can assign that member as soon as its ready arrives. The member polls before completion reports and on every subsequent wake, then handles queued assignments before becoming idle.

## Assignment and receiving

Messages are point-to-point. Send individually to every selected recipient. Every recipient's messages, the Director's included, are delivered into its pane as one line with Escape first, and the recipient ACKs each delivered ID after acting on it. ACK every read delivery immediately, retaining unfinished actions in host-owned task state. See [run a fleet](../how-to/mixed-backend-team.md#manual-lifecycle).

## Command routing

A member runs commands its host permits and routes a required denied command to the Director. The Director authorizes `member exec`, which visibly submits the actual native shell command and returns submission metadata. Inspect native output with `member capture`; after completion, send an explicit `member prompt` to continue if the host has not resumed the member. A failed or uncertain dispatch requires inspection before any newly authorized retry.

The [native dispatch contract](../spec/cli-options.md#member-exec) owns validation, lock and uncertainty errors. File-edit permission gaps need a matching host rule or a Director edit.

## Inspection and recovery

Use `member list`, `member capture` and `fleet capture` to check roster and pane evidence. A timeout alone establishes neither failure nor completion. Preserve retained user drafts and relay them to the user. Replace an unusable member only under the existing authorization, carrying its assignment, messages and artifacts into the replacement prompt.

## Teardown

Delete owned ordinary members before deleting the fleet. Member deletion kills its owned pane and deregisters it; fleet deletion deregisters the registry without closing panes. Both serialize placement changes with pane input and fail without registry changes if lock contention lasts five seconds. The root Director is protected from individual member deletion.
