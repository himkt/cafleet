# Routing Bash via the Director

The bash-via-Director protocol is the **fallback** for the harness deny-list. Members run cafleet and any shell command directly via the Bash tool by default (workspace-scoped auto-approval — see [`roles/member.md`](../roles/member.md) and [Bash routing](../../../docs/concepts/bash-routing.md)). The fallback fires only when the coding-agent harness deny-list rejects a Bash invocation (e.g. `git push`, `rm -rf`): the member auto-routes a plain CAFleet message to its Director, which dispatches the command into the member's pane via `cafleet member exec` (keystrokes literal `! <cmd>` + `Enter`, honored by `claude` / `codex` / `opencode`).

## The two primitives

| Primitive | Purpose | Permission gate |
|---|---|---|
| [`cafleet member exec`](director.md#member-exec) | Shell-dispatch with operator-controlled `COMMAND`. Keystrokes `! <cmd>` + `Enter`. | `permissions.ask` |
| [`cafleet member ping`](director.md#member-ping) | Fixed-action inbox-poll nudge; no operator-controlled body. | `permissions.allow` |

## Member-side: reconsider, then route

Most denials are a wrong flag, wrong path, or an unnecessary command — fix or drop it yourself first. Only a genuinely-correct, genuinely-needed, still-denied command gets routed:

1. Send a plain CAFleet message to the Director:
   ```bash
   cafleet message send --fleet-id <fleet-id> --agent-id <my-agent-id> \
     --to <director-agent-id> \
     --text "Need to run: <command>. My harness denied it (<reason if known>)."
   ```
2. **Wait** for the Director's `member exec` dispatch to land in your pane; the bang-shortcut output appears in your next-turn context.
3. Process the output; reply to the Director if a follow-up is expected.

The forbidden behaviors (never fake `<bash-input>` markup or fabricate output, never answer from stale context, never assume Bash is denied without trying) are canonical in [`roles/member.md`](../roles/member.md#what-you-must-never-do). Two routing-specific additions: never route without reconsidering first, and never offer the operator a list of routing options — the operator already asked for the command to run; routing is implementation. If your `cafleet message send` is *also* harness-denied, tell the operator both are denied (the only time you ask the operator for help); otherwise route silently.

## Director-side dispatch

On a member's denial-fallback request: read it (`cafleet message poll` / `message show`), verify it is reasonable and from a real team member, then dispatch and follow up — **in this order**:

```bash
cafleet member exec --fleet-id <fleet-id> --member-id <member-agent-id> "<command>"
cafleet member ping --fleet-id <fleet-id> --member-id <member-agent-id>
cafleet message ack --fleet-id <fleet-id> --agent-id <director-agent-id> --task-id <task-id>
```

The `member ping` is required — `member exec` only stages the bang output; the ping advances the member's turn so it consumes the output (see [`reference/director.md`](director.md#member-exec) § Required follow-up).

**Serialize.** Process requests in poll order, one at a time: `exec → ping → ack → next`. Two `member exec` dispatches firing concurrently against the same pane race the keystroke sequence and corrupt output. Within one member, `exec → ping → exec → ping`; never `exec → exec`.

**Cross-fleet boundary.** `member exec` reaches any member of the same `--fleet-id` (no caller-auth check); a `--member-id` outside the fleet returns "not found" (see [`cli-options.md`](../../../docs/spec/cli-options.md#member-exec)). A request naming a member outside your fleet cannot be served — reply with a plain `cafleet message send` explaining the mismatch.
