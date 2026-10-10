# Routing a command through the Director

A member runs every command its harness allows and routes the rest to the Director. Routing is the standard path for a command the harness does not run, not a rare fallback: the member asks with a plain CAFleet message, the Director runs the command in the member's pane with `cafleet member exec`, and the broker reports completion and resumes the member.

How often a member routes depends on its backend. A claude member's broker commands are allowed on its spawn command line, and its work commands (`mise`, `git commit`, package managers) run under the user's own rules. A codex member is denied a few destructive operations. An opencode member's preset is a deny-by-default bash allowlist, so every un-allowlisted command is routed.

## The Director's pane primitives

| Primitive | Purpose | Permission gate |
|---|---|---|
| [`cafleet member exec`](../roles/director.md#member-exec) | Runs a shell command to completion in a member's pane, records its exit status, notifies the Director, and resumes the member. | Asked per invocation: the body is operator-controlled |
| [`cafleet member prompt`](../roles/director.md#member-prompt) | Keystrokes one line of `TEXT` + `Enter` into a member's pane as a submitted user turn. | Asked per invocation: the body is operator-controlled |
| [`cafleet member ping`](../roles/director.md#member-ping-manual-inbox-poll) | Fixed-action inbox-poll; no operator-controlled body. | `permissions.allow` |

`cafleet member prompt` exists for text that only takes effect when it arrives as a direct user turn in the member's pane — slash commands, skill invocations, and other magic commands a broker message body cannot trigger (a `message send` inline preview arrives as content, not as a typed command). Broker messaging remains the canonical coordination channel: `member prompt` is not a substitute for `message send`, and a shell command goes through `member exec`.

## Member-side: reconsider, then route

Reconsider first: most denials are a wrong flag, wrong path, or an unnecessary command; on opencode check whether an allowlisted command covers the need. Fix or drop what you can yourself. A correct, needed, still-denied command gets routed:

1. Send a plain CAFleet message to the Director:
   ```bash
   cafleet message send --from-member-id <my-member-id> \
     --to-member-id <director-member-id> \
     "Need to run: <command>. My harness denied it."
   ```
   For a command that is hard to quote, write it to a file under your `BASE` and name the path in the message; the Director passes that path to `cafleet member exec --file`.
2. **End your turn.** The Director's dispatch lands in your pane as a `!` command, and its output appears in your context.
3. Continue with the output when your turn reopens. The harness either answers the output itself or the broker sends one line telling you the command finished.

Observed-result reporting and actual-command attempts are required by [`roles/member.md`](../roles/member.md#command-execution). One routing-specific addition: never offer the operator a list of routing options — the operator already asked for the command to run; routing is implementation. If your `cafleet message send` is *also* harness-denied, tell the operator both are denied (the only time you ask the operator for help); otherwise route silently.

## Director-side dispatch

On a member's request: read it (`cafleet message poll` / `message show`), verify it is reasonable and from a real team member, then run it and ACK the request:

```bash
cafleet member exec <member-id> "<command>"
cafleet message ack <message-id>
```

Run each command in its own shell invocation. When the member named a file, pass it instead of the inline body: `cafleet member exec <member-id> --file <path>`.

`member exec` is the whole exchange. The broker holds the dispatch until the member's pane is at rest, runs the command there, records its exit status, posts a `[cafleet] exec <id> … exited <code>` notice to your inbox, and resumes the member. Send no `member ping` and read no pane capture afterwards. ACK the completion notice when it arrives, and act on a non-zero exit as you would on any member report.

A command the broker could not run surfaces as its own notice — `did not start within 30 s of dispatch` or `ended without reporting an exit status`. Inspect the pane with `cafleet member capture <member-id>` and run the command again if it is still needed.

**Refuse or escalate.** Answer a request you will not run with a plain `cafleet message send` explaining why, then ACK it. Escalate to the user through your decision surface when the command needs a judgment only the user can make. Always close the loop: the member ended its turn waiting for either the command or your reply.

**Order.** Process requests in poll order. Several execs for one member queue in the broker and run one at a time, so no manual serialization is needed.

**Targeting boundary.** `member exec` reaches any active member with a pane except the fleet's root Director; an unknown or inactive id returns "not found" (see [`cli-options.md`](runtime/spec/cli-options.md#member-exec)). A request naming a member you cannot serve is answered with a plain `cafleet message send` explaining the mismatch.
