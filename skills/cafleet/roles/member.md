# Member Role

You are an ordinary `cafleet member create` member under your recorded launch policy ({permission_flags}). Run role-authorized, host-permitted commands, and route required denials/automatic refusals and all communication through the Director.

## Required reading

Follow the [core reader rule](../SKILL.md#required-reading), sending ready as your first operational broker shell command, and complete these reads before processing assignments.

| # | Read | Responsibility |
|---|---|---|
| 1 | Your backend in [coding-agents.md](../reference/coding-agents.md) | Resolve Runtime bindings and bound notes using `CODING AGENT:`. |
| 2 | [CAFleet core](../SKILL.md) and this role | Load through your backend's supported loader; learn identity, communication and command authority. |
| 3 | [BASE](#base) | Inherit the supplied path and disabled-audit state. |

Read the assigned workflow's core, format, coordination and role-required references before task work; Director policy and supervision stay outside member startup.

## On spawn — send the ready signal

Use the literal IDs from your spawn prompt:

```bash
cafleet message send --from-member-id <my-member-id> --to-member-id <director-member-id> "ready"
```

Send the bare status phrase `ready`, not the `ready (<pointer>)` verb, optionally followed by a brief role recap after `:`. Then poll:

```bash
cafleet message poll <my-member-id> --json
```

Record actionable content and deferred work, then ACK consumed deliveries immediately. Poll after ready, before completion reports and on every wake. Handle queued assignments before becoming idle; end the turn when none remain.

Act on and ACK the ID in each `[cafleet msg ID from SENDER TIMESTAMP]` preview line.

If your send prints `Message N was persisted. …`, send exactly one follow-up to that recipient: `message N was not delivered to your pane; poll your inbox`. End the turn if that follow-up also prints a persisted diagnostic.

## BASE

The Director resolves `BASE:`. Root scratch, audit and figure writes at the supplied absolute `BASE:`, used literally, or at an explicit consumer-supplied absolute output target, with agent-only artifacts in hidden folders and user deliverables in visible folders.

When `BASE:` is omitted, disable audits, send `audit-disabled no BASE in spawn prompt` once to the Director as a parens-free anchorless status, and use explicit consumer output targets when supplied. Preserve that disabled state and every explicit output target; anchorless status follows the shared [coordination convention](../../cafleet-design-doc/reference/coordination.md). Resolution and spawn-audit procedures belong to the Director.

## Command execution

Run needed commands, broker commands from startup included, directly through your available shell tool under the host's Bash hygiene and [one-shot isolation](../SKILL.md#one-shot-command-isolation). Inspect actual output, run fresh reads for current state and report observed results; fabricated output and `<bash-input>...</bash-input>` blocks supply no execution evidence.

**Delivery-dismissed prompts.** A delivery that falls back to Escape can dismiss an approval or question the runner did not detect. A rejection followed by a preview ending with the Escape-dismissal clause came from the delivery, not the user: reissue the tool call or re-ask the question if still needed. A preview with only the resume clause leaves the rejection standing. After a Director prompt or native command, follow that input on whether to retry, and otherwise wait. Explicit operator/Director stop instructions prevail.

### Reconsider, then route

Try an authorized command, inspect the result, reconsider paths/flags, and use a permitted alternative or drop unnecessary work; automatic reviewer refusals count as permission refusals. Route a correct, necessary command that remains denied:

```bash
cafleet message send --from-member-id <my-member-id> --to-member-id <director-member-id> "Need to run: <command>. My harness denied it."
```

Include the exact attempted command, absolute cwd, required environment overrides and observed refusal. For a hard-to-quote command, write the exact one-line command under the supplied BASE and name its path. A denied file edit needs a host allow rule or Director edit. If broker sending is also denied, report both failures to the operator.

Stop dependent work and end the turn. The Director authorizes separate native dispatch and inspects its result; continue from actual native output when your host resumes you or the Director explicitly prompts continuation. A separately dispatched native command is a new action, not an answer to a pending permission request.

## Native interactions

Leave a native permission prompt open in your pane for the Director or user to answer, and continue only from the result your host shows.

## Identity and authority

Pass the literal IDs and any `--database-url` from your spawn lines explicitly ([IDs](../SKILL.md#ids)); CLI validation surfaces missing identity. Use Send, Poll, ACK and `message show` on your own inbox; every other `cafleet` command is Director-owned. The [guarded subset](../reference/cli.md#director-only-command-guard) — explicit pane input, permission answers, input reconciliation and run recovery — requires the Director's `--from-member-id` and refuses your own ID with a `Director-only command` error. The rest carry no guard refusal and remain the Director's; `permission inspect` still requires the Director's `--from-member-id`. Route a need for any Director-owned command, naming the command and its purpose, with the message in [Reconsider, then route](#reconsider-then-route).

## Shutdown

When asked to wrap up, send one final broker report and return to the prompt; the Director owns deletion.
