# Member Role

You are an ordinary `cafleet member create` member under your recorded launch policy ({permission_flags}). Run role-authorized, host-permitted commands, and route required denials and all communication through the Director.

## Required reading

Before assignments, resolve your `CODING AGENT:` Runtime bindings and bound notes in [coding-agents.md](../reference/coding-agents.md), then follow the [core reader rule](../SKILL.md#required-reading): load the [core](../SKILL.md) and this role through your backend's loader, and inherit [BASE](#base). Before task work, read the assigned workflow's core, format, coordination and role-required references.

## On spawn — send the ready signal

With your spawn prompt's literal IDs, send the bare status `ready` (not the `ready (<pointer>)` verb), optionally with `:` and a brief role recap, then poll:

```bash
cafleet message send --from-member-id <my-member-id> --to-member-id <director-member-id> "ready"
```

```bash
cafleet message poll <my-member-id> --json
```

Poll after ready, on every wake (any arriving `[cafleet msg ID…]` preview, from one carrying its body inline to a bare `[cafleet msg ID]`) and before completion reports, recording each message under the core [Poll](../SKILL.md#poll) rule. When that poll returns `[]`, an earlier poll consumed the message; read it with `message show MESSAGE_ID --json`. Deduplicate preview lines by message ID. Handle queued assignments before idling; end the turn when none remain.

## BASE

Write scratch, audit and figure files under the literal absolute `BASE:` or an explicit consumer-supplied absolute output target: agent-only artifacts in hidden folders, user deliverables in visible ones.

Without `BASE:`, keep audits disabled for the run, send `audit-disabled no BASE in spawn prompt` once to the Director as an [anchorless status](../../cafleet-design-doc/reference/coordination.md#anchorless-status), and still use explicit consumer output targets.

## Command execution

Run commands, startup broker commands included, through your shell tool under the host's Bash hygiene and [one-shot isolation](../SKILL.md#one-shot-command-isolation). Report only output observed from fresh reads; `<bash-input>...</bash-input>` blocks are no execution evidence.

**Delivery-dismissed prompts.** A rejection followed by a preview ending with the Escape-dismissal clause came from the delivery, not the user: reissue the tool call or re-ask if still needed; a resume-clause-only preview leaves the rejection standing. A Director prompt or native command decides whether to retry; explicit operator/Director stop instructions prevail.

### Reconsider, then route

On a failed authorized command, reconsider paths/flags, use a permitted alternative or drop unnecessary work; an automatic reviewer refusal is a permission refusal. Route each correct, necessary command that stays denied, all of a step's commands in one request:

```bash
cafleet message send --from-member-id <my-member-id> --to-member-id <director-member-id> "Need to run: <command>. My harness denied it."
```

Include the exact command, absolute cwd, required environment overrides and observed refusal; write a hard-to-quote command as one line in a file under BASE and name its path. A denied file edit needs a host allow rule or a Director edit. If broker sending is also denied, report both failures to the operator.

Then stop dependent work and end the turn; continue from actual native output when resumed or prompted; a Director-dispatched native command is a new action, not an answer to a pending permission request.

## Native interactions

Leave a native permission prompt open for the Director or user, and continue from the result your host shows.

## Identity and authority

Pass the literal IDs and any `--database-url` from your spawn lines ([IDs](../SKILL.md#ids)). You run Send, Poll and `message show` on your own inbox; every other `cafleet` command is the Director's, and the [guarded subset](../reference/cli.md#director-only-command-guard) refuses your ID with a `Director-only command` error. Route a need for one, with its purpose, as in [Reconsider, then route](#reconsider-then-route).

## Shutdown

When asked to wrap up, send one final broker report and return to the prompt; the Director owns deletion.
