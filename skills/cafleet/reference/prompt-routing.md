# Routing a command through the Director

A member runs each command its host permits. A required unavailable command routes through the Director, whose host rules authorize the native pane action. Preserve the actual command and inspect native output.

## The Director's pane primitives

| Primitive | Responsibility |
|---|---|
| [member exec](../roles/director.md#member-exec) | Submit the actual one-line native shell command and return dispatch metadata |
| [member prompt](../roles/director.md#member-prompt) | Submit explicitly authorized composer input or post-command continuation |
| [member press](../roles/director.md#member-press) | Request explicitly authorized ordered manual keys |

Every writer shares the bounded pane-input lock. Wrapped prompt/exec input validates the pinned runner and durable receipt, preserves candidate/open interactions and requires a tested empty composer; it submits literal native input without Escape. Explicitly unwrapped prompt/exec input retains Escape-first multiplexer submission. Manual `member press` requests only its explicit keys through the outer pane, including wrapped members, with no implicit Escape or Enter. Its success establishes backend acknowledgment, while actual native effects and continuation require separate observation.

## Member-side: reconsider, then route

Follow [member command execution](../roles/member.md#command-execution). Try an authorized command, inspect its observed result, and reconsider incorrect flags/paths or an appropriate permitted alternative.

If the necessary command remains denied, send:

```bash
cafleet message send --from-member-id <my-member-id> --to-member-id <director-member-id> "Need to run: <command>. My harness denied it."
```

For hard-to-quote commands, write the exact one-line command under supplied BASE and name its path. End the turn so the Director can submit native input. A denied file edit needs a host allow rule or Director edit. If the broker send is also denied, report both observed failures to the operator.

Include the exact attempted command, absolute cwd, required environment overrides and observed refusal so the Director can authorize the actual action. A runner-observed permission request follows the [runtime procedure](member-runtime.md): the Director evaluates risk, decides allow/deny in its verified auto mode or obtains user permission, then uses a request-bound response or manual native input. A separately dispatched native exec remains a new action rather than an answer to that request.

Native output remains visible in the host transcript. Continue from it when the host resumes you or the Director sends an explicit continuation prompt. Report only output you actually observed.

## Director-side dispatch

Read the request, record the outstanding action in host-owned state, authorize its exact command, then dispatch and ACK consumption:

```bash
cafleet member exec <member-id> "<command>"
```

```bash
cafleet message ack <message-id>
```

After lock and placement revalidation, the CLI routes wrapped input through the healthy runner’s tested native-shell composer profile, or uses the legacy Escape lead for an explicitly unwrapped member. It types `! ` plus the actual command once. It returns submission metadata. The host owns environment, permissions, execution and visible output.

Escape-first input to an explicitly unwrapped member cancels an open tool approval or question in its pane. Capture that member's pane before `member exec` or `member prompt`; when the capture shows an open prompt, resolve it through the user relay first, or reach the member with `message send`, whose preview carries the dismissal clause.

Inspect the result and terminal state:

```bash
cafleet member capture <member-id> --lines 80
```

After completion evidence, continue a member that has not resumed:

```bash
cafleet member prompt <member-id> "Use the native command output and continue your assignment."
```

An uncertain dispatch requires capture/inspection and durable receipt reconciliation before a newly authorized action. Preserve known input IDs and use the [runtime procedure](member-runtime.md). The first writing call can partially type; a failure after Enter can mean the command ran. Preserve the pane and evidence instead of automatically resubmitting.

Refuse with a plain broker reply explaining why, then ACK. Relay a new user decision through the executing backend's decision surface. Track an authorized command/result/continuation in host-owned state while it remains outstanding, and check it on every [wake](supervision.md#director-wake). Native shell probes establish visibility; [backend notes](coding-agents.md) state the permission evidence limits.
