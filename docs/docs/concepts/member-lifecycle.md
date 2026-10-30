# Member lifecycle

Register the calling session as Director, then create independent member panes and dispatch each assignment after its ready signal.

## Registration and readiness

`fleet create` registers the current Director atomically. `member create` inherits that Director's backend unless overridden, validates model/effort, native policy, evidence policy and prompt identity, then registers a pending placement/run and starts the owned PTY runner. Creation retains its guards through the prepared/start/started handshake. Creation compensates for failures while preserving the original cause and any cleanup error.

Members confirm readiness, then poll their inbox. The Director can assign a member as soon as its ready arrives.

## Assignment and receiving

Messages are point-to-point. Send individually to every selected recipient. Runner-managed members receive previews through their runner: profile delivery into an empty composer, Escape-first delivery fallback otherwise, or a queue the runner drains after an open native interaction closes ([delivery methods](member-permission-events.md#deliver-generated-input)). Explicitly unwrapped members receive Escape-first previews. The root Director receives replies through its native inbox tools; [run a fleet](../how-to/mixed-backend-team.md#manual-lifecycle) explains reception and consumption.

## Command routing

A member runs commands its host permits and routes a required denied command to the Director. The Director authorizes `member exec`, which visibly submits the actual native shell command and returns submission metadata. Inspect native output with `member capture`; after completion, send an explicit `member prompt` to continue if the host has not resumed the member. A held permission request uses the Director’s effective mode: verified auto permits its allow/deny decision, while other/unknown modes require user permission. Submit only a current request-bound response or use manual native input, then inspect actual results and continuation. Inspect a failed or uncertain dispatch and its durable input receipt before any newly authorized retry. [Member permission events](member-permission-events.md) explains the complete event/answer/retry sequence.

The [native dispatch contract](https://github.com/himkt/cafleet/blob/main/SPEC.md#member-exec) owns validation, lock and uncertainty errors. File-edit permission gaps need a matching host rule or a Director edit.

## Inspection and recovery

Use `member list` and `member capture` to check roster and pane evidence. Capture about 20 lines with `member capture` (its default) and read the content directly; use `--lines` for more when it is insufficient. An empty composer does not mean the task is complete — judge completion from messages and artifacts. A timeout alone establishes neither failure nor completion. Replace an unusable member only under the existing authorization, carrying its assignment, messages and artifacts into the replacement prompt.

## Teardown

Runner-managed deletion coordinates shutdown and event recovery under the existing pane locks, retaining unconfirmed child cleanup and recoverable artifacts explicitly. Member deletion kills its owned pane and deregisters it. Fleet deletion ends every remaining ordinary member the same way, reporting each pane outcome, then deregisters the root Director and the fleet while leaving the Director's pane open; a member that cannot be ended stops it with the fleet still registered, and a rerun resumes. Both serialize placement changes with pane input and fail without registry changes if lock contention lasts five seconds. The root Director is protected from individual member deletion.
