# Routing Bash via the Director

The bash-via-Director protocol is the **fallback** for the harness deny-list. Members spawn with workspace-scoped auto-approval enabled — `claude` uses `--permission-mode dontAsk`, `codex` uses `--ask-for-approval never --sandbox workspace-write`, and `opencode` uses `--agent cafleet` to bind the in-source `CAFLEET_AGENT` permission ruleset (catch-all-allow + specific-deny) — so the Bash tool is enabled and routine permission prompts auto-resolve silently. Members run cafleet (and any other shell command) directly via the Bash tool by default. **No prefix, no Director routing required by default.**

The fallback fires only when the coding-agent harness deny-list rejects a Bash invocation (e.g. `git push`, `rm -rf`). In that case the member auto-routes: it sends a plain CAFleet message to its Director asking for the command, and the Director dispatches the command into the member's pane via `cafleet member exec`, which keystrokes literal `! <cmd>` + `Enter` and triggers the coding agent's `!` CLI shortcut on the receiving side (honored by `claude`, `codex`, and `opencode`). No new broker primitives, no extra helper machinery — just the existing message-passing + tmux-keystroke infrastructure plus the dedicated `member exec` subcommand.

## The two primitives

| Primitive | Purpose | Permission gate |
|---|---|---|
| [`cafleet member exec`](director.md#member-exec) | **Shell-dispatch** with operator-controlled `COMMAND` argument. Keystrokes `! <cmd>` + `Enter` into the member's pane via the coding agent's `!` shortcut. | `permissions.ask` — every invocation prompts the operator. |
| [`cafleet member ping`](director.md#member-ping) | **Inbox-poll-only** nudge. Keystrokes `cafleet --fleet-id <s> message poll --agent-id <m>` + `Enter` into the member's pane. Action is fixed by the subcommand name; no operator-controlled keystroke body. | `permissions.allow` — pre-approved. |

`member exec` is for shell dispatch. `member ping` is for nudging a member that missed an inline-preview keystroke (e.g. its TUI was in a non-input state when the preview arrived). Do not conflate them.

## Reconsider before routing

Most denials happen because the underlying command is wrong — wrong flag, wrong path, or unnecessary altogether. Before routing, the member MUST reconsider. Only route a command that is **genuinely correct AND genuinely needed AND still rejected** by the harness.

| Step | Member action |
|---|---|
| 1 | Bash invocation denied → re-examine the command. Was a flag wrong? Path wrong? Was the command needed at all? Most denials resolve here without any routing. |
| 2 | If the command was wrong, fix it and try again via Bash. |
| 3 | If the command was unnecessary, drop it. Tell the user / Director plainly that you have the answer already. |
| 4 | If the command is genuinely correct AND genuinely needed AND still denied, **and only then**, send a CAFleet message to the Director asking for the command. |

## Member-side fallback recipe

When a member decides to route after reconsidering:

1. Send a plain CAFleet message to the Director:
   ```bash
   cafleet --fleet-id <fleet-id> message send --agent-id <my-agent-id> \
     --to <director-agent-id> \
     --text "Need to run: <command>. My harness denied it (<reason if known>)."
   ```
2. **Wait** for the Director's `member exec` dispatch to land in the member's pane. The Director's `member exec` will keystroke `! <command>` + Enter; the bang-shortcut output appears in the member's next-turn context.
3. Process the output as fresh context. Reply to the Director if a follow-up is expected.

### MUST NOT (member side)

- Emit `<bash-input>...</bash-input>` markup, fenced ```bash result blocks, or any text that looks like a command result without actually running the command. That markup is meaningful only when produced by the harness; emitting it yourself produces no execution and misleads the operator into thinking the command ran.
- Fabricate output. If you have not actually run the command and seen real output, you do not know the result. Say so plainly.
- "Just answer from prior context" when a fresh command result is needed. Working-tree state changes between turns; only a real execution gives a true answer.
- Refuse to run a command on the assumption that Bash is denied. Bash is enabled under workspace-scoped auto-approval. Try it.
- Route a denied command to the Director without first reconsidering it. Most denials happen because the command is wrong (typo, wrong flag, wrong path) or unnecessary altogether. Fix the command yourself before asking anyone else to run it.
- Offer the operator a list of routing options ("(1) you run it via `!`, (2) route through Director, (3) skip"). The operator already implicitly answered by asking you to run the command — they wanted it run. Routing is implementation. The bash-via-Director protocol exists precisely to handle the harness-denied case without operator interaction.

If the `cafleet message send` itself is also denied by the harness, surface that to the operator as a single fact ("my Bash and my cafleet are both denied; please dispatch via `cafleet member exec <command>` from your Director pane") — but that is the **only** time you ask the operator for help. Anything else, route through the Director silently.

## Director-side fallback recipe

When a Director receives a member's denial-fallback request:

1. Read the message via `cafleet message poll` / `cafleet message show --task-id <task-id>`.
2. Verify the request makes sense (the command is reasonable, comes from a real member of your team, and has not already been served).
3. Dispatch via [`cafleet member exec`](director.md#member-exec):
   ```bash
   cafleet --fleet-id <fleet-id> member exec \
     --member-id <member-agent-id> \
     "<command>"
   ```
4. **Immediately follow up with `cafleet member ping`** so the member's TUI advances its turn and consumes the bang-output as fresh context (see `member exec` § Required follow-up in `reference/director.md`):
   ```bash
   cafleet --fleet-id <fleet-id> member ping \
     --member-id <member-agent-id>
   ```
5. ACK the member's request message via `cafleet message ack --task-id <task-id>` so it does not re-fire on the next poll.

### Serialization

Process member denial-fallback requests in **poll order, one at a time**. Two `member exec` dispatches firing concurrently against the same pane race the keystroke sequence and corrupt output. If you have multiple denial-fallback requests outstanding from different members, dispatch them sequentially: exec → ping → ack → next request. Inside a single member, exec → ping → exec → ping; never exec → exec.

### Cross-fleet boundary

`cafleet member exec` reaches any member of the same `--fleet-id` — there is no caller-auth check. The only boundary is fleet isolation: a `--member-id` that does not belong to `--fleet-id` exits 1 with `Error: Agent <member-id> not found`. A denial-fallback request from a member of your own fleet is always dispatchable; a request that names a member outside your fleet cannot be served (the `member exec` will return "not found"), so reply to the sender with a plain `cafleet message send` explaining the mismatch instead.

## Why no operator-prompts-for-routing

When a member offers the operator a list of options ("(1) you run it via `!`, (2) route through Director, (3) skip"), it pushes a routing decision back to the operator that they already implicitly answered by asking the member to run the command. They wanted it run. Routing is implementation. The bash-via-Director protocol exists precisely to handle the harness-denied case without operator interaction.

## Role-specific reading

The fallback has two perspectives. Read **only the file matching your role**:

- **If you are a member** (spawned by `cafleet member create`) → read [`roles/member.md`](../roles/member.md). Covers the default "run it yourself via Bash" path, the reconsider-then-route protocol when Bash is denied, and forbidden behaviors.
- **If you are a Director** → read [`roles/director.md`](../roles/director.md). Covers how to recognize a member's denial-fallback request, the `cafleet member exec` dispatch, the ping follow-up, and the cross-fleet boundary.
