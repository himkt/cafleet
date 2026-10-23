# Routing a command through the Director

A member runs each command its host permits. A required unavailable command routes through the Director, whose host rules authorize the native pane action. Preserve the actual command and inspect native output.

## The Director's pane primitives

| Primitive | Responsibility |
|---|---|
| [member exec](../roles/director.md#member-exec) | Submit the actual one-line native shell command and return dispatch metadata |
| [member prompt](../roles/director.md#member-prompt) | Submit explicitly authorized composer input or post-command continuation |
| member ping | Submit a fixed inbox-poll prompt to an idle member |

Every writer shares input locking and fresh readiness checks. Successful submission establishes no command exit status.

## Member-side: reconsider, then route

Follow [member command execution](../roles/member.md#command-execution). Try an authorized command, inspect its observed result, and reconsider incorrect flags/paths or an appropriate permitted alternative.

If the necessary command remains denied, send:

```bash
cafleet message send --from-member-id <my-member-id> --to-member-id <director-member-id> "Need to run: <command>. My harness denied it."
```

For hard-to-quote commands, write the exact one-line command under supplied BASE and name its path. End the turn so the Director can submit native input. A denied file edit needs a host allow rule or Director edit. If the broker send is also denied, report both observed failures to the operator.

Native output remains visible in the host transcript. Continue from it when the host resumes you or the Director sends an explicit continuation prompt. Report only output you actually observed.

## Director-side dispatch

Read the request, record the outstanding action in host-owned state, authorize its exact command, then dispatch and ACK consumption:

```bash
cafleet member exec <member-id> "<command>"
```

```bash
cafleet message ack <message-id>
```

The CLI types `! ` plus the actual command once after lock/revalidation/idle capture. It returns submission metadata. The host owns environment, permissions, execution and visible output.

Inspect the result and terminal state:

```bash
cafleet member capture <member-id> --lines 80
```

After completion and idle evidence, continue a member that has not resumed:

```bash
cafleet member prompt <member-id> "Use the native command output and continue your assignment."
```

An uncertain dispatch requires capture/inspection before a newly authorized retry. The first writing call can partially type; a failure after Enter can mean the command ran. Preserve the pane and evidence instead of automatically resubmitting.

Refuse with a plain broker reply explaining why, then ACK. Relay a new user decision through the executing backend's decision surface. Keep live receiving while an authorized command/result/continuation remains outstanding. Native shell probes establish visibility; [backend notes](coding-agents.md) state the permission evidence limits.
