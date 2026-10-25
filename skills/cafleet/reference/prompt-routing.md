# Routing a command through the Director

A member executes task commands under the backend's selected permission policy and routes an observed permission denial to the Director. Preserve native guards, explicit deny rules, managed policies and host permission/sandbox settings. The Director reviews the exact command and actual refusal under existing host and operator authority. Routing supplies no automatic override of a native or managed denial. The runtime prompt carries this behavioral contract and literal broker identities for every member-create path.

## The Director's pane primitives

| Primitive | Responsibility |
|---|---|
| [member exec](../roles/director.md#member-exec) | Submit the actual one-line native shell command and return dispatch metadata |
| [member prompt](../roles/director.md#member-prompt) | Submit explicitly authorized composer input or post-command continuation |

Every writer shares the bounded pane-input lock and leads with Escape; no writer checks pane readiness. Successful submission establishes no command exit status.

## Member-side: observed denial

Follow [member command execution](../roles/member.md#command-execution). Use native readers for prerequisite files and permitted shell reads when shell is the only reader. Inspect each command's actual result. An ordinary nonzero result follows normal recovery; an observed permission denial routes with its exact command, working directory, required environment and actual refusal reason. The same route is available before ready when a prerequisite is denied.

After a returned permission denial, send:

```bash
cafleet message send --from-member-id <my-member-id> --to-member-id <director-member-id> "Need to run: <exact command>. Cwd: <absolute working directory>. Environment: <required overrides or none>. Refusal: <actual permission denial>."
```

For hard-to-quote commands, use a permitted native editor to write the exact one-line command and refusal evidence under supplied BASE and name its path. End the turn for Director review. A denied file edit needs an authorized host-rule change or Director edit. If broker communication itself fails, report the observed failure to the operator.

Surface unresolved approval to the Director. When automatic review is selected, surface an unavailable automatic mode to the Director. Preserve the actual pending request rather than inferring consent. An interactive prompt can prevent a member from sending; the Director's supervision must inspect that state. The prompt-level routing contract provides no deterministic refusal-event interception.

Await an actual result or explicit continuation before continuing dependent work. Use the visible host transcript when the host resumes you or the Director sends a continuation prompt. Report only observed output and preserve the original exit status; a dispatch receipt establishes submission only.

## Director-side dispatch

Read the command/cwd/environment/refusal evidence and record the outstanding action in host-owned state. Resolve the request under existing host and operator authority; an effective native or managed denial retains that authority. When a native dispatch is authorized, submit its exact command and ACK consumption:

```bash
cafleet member exec <member-id> "<command>"
```

```bash
cafleet message ack <message-id>
```

After Escape, lock and revalidation, the CLI types `! ` plus the actual command once. It returns submission metadata. The host owns environment, permissions, execution and visible output.

Inspect the result and terminal state:

```bash
cafleet member capture <member-id> --lines 80
```

After completion evidence, continue a member that has not resumed:

```bash
cafleet member prompt <member-id> "Use the native command output and continue your assignment."
```

An uncertain dispatch requires capture/inspection before a newly authorized retry. The first writing call can partially type; a failure after Enter can mean the command ran. Preserve the pane and evidence instead of automatically resubmitting.

Refuse with a plain broker reply explaining why, then ACK. Relay a new user decision through the executing backend's decision surface. Track an authorized command/result/continuation in host-owned state while it remains outstanding, and check it on every [wake](supervision.md#director-wake). Native shell probes establish visibility; [backend notes](coding-agents.md) state the permission evidence limits.
