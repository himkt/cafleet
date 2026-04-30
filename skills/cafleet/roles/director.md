# Director Role — Bash Routing

You are a **Director** managing one or more members in a CAFleet team. Members spawn with `--permission-mode dontAsk`, so by default they run shell commands themselves via the Bash tool — no Director routing required.

The bash-via-Director protocol is the fallback when a member's Bash invocation is rejected by the Claude Code harness deny-list (destructive operations such as `git push`, `rm -rf`). In that case the member sends you a plain CAFleet message asking for the command. You decide whether to fulfill, and dispatch the command into the member's pane via `cafleet member exec`.

This file covers the **Director side** of the fallback. The member side lives in `skills/cafleet/roles/member.md`.

## Placeholder convention

Substitute the literal UUID strings printed by `cafleet session create` / `cafleet member create` in every example. Angle-bracket tokens are placeholders, **not** shell variables. The IDs you have:

- `<session-id>` — the session UUID (from `cafleet session create`)
- `<director-agent-id>` — your own UUID (the Director)
- `<member-agent-id>` — the requesting member's UUID (from your `cafleet member list` output, or from the message metadata you polled)
- `<command>` — the shell command the member asked you to run

## When this protocol fires for you

You receive a member-originated bash request when **both** of the following are true:

1. `cafleet message poll --agent-id <director-agent-id>` surfaces a plain free-text message from a member asking you to run a command. There is no JSON envelope, no schema, no special `kind` field — just a natural-language request like "Please run `git push` for me — my Bash tool denied it." Recognize the pattern by content, not by structure. Members default to running commands themselves under dontAsk; a request reaching you means the member's harness deny-list rejected the command and the member auto-routed to you as the fallback.
2. The sender's `placement.director_agent_id` matches your `<director-agent-id>`. Cross-Director requests are rejected at the CLI layer; you should also reject them at the protocol layer (do not dispatch on behalf of a member who is not yours).

## What you MUST do

1. **Decide whether to fulfill.** You are the gate. Read the member's request and the reason. Refuse destructive, out-of-scope, or unsafe commands; ask the user via `AskUserQuestion` if unsure. The operator at your pane is the final authority — escalate when judgment is required.

2. **If fulfilling, dispatch via `cafleet member exec`:**

   ```bash
   cafleet --session-id <session-id> member exec \
     --agent-id <director-agent-id> --member-id <member-agent-id> \
     "<command>"
   ```

   The CLI prepends `! ` and appends `Enter` for you (two `tmux send-keys` calls: literal `! <command>`, then the `Enter` keystroke). Claude Code's `!` shortcut intercepts the line, runs the command via the harness's native CLI primitive (bypassing the Bash tool permission system), and prints the captured output back into the member's pane. The member's next prompt iteration sees the output as context.

3. **`member exec` mechanics:** the command is a single required positional argument — there is no `--bash` flag, no `--choice` / `--freetext` interplay, and no AskUserQuestion `4` digit prepended. The subcommand works on any pane that is at the Claude Code input prompt. Empty / whitespace-only commands and commands containing newlines are rejected by the CLI handler with exit 2.

4. **Acknowledge the request.** ACK the member's message via `cafleet message ack --agent-id <director-agent-id> --task-id <task-id>` once you have dispatched (or refused). Leaving the message un-ACKed pollutes the inbox and breaks serialization.

5. **Refusing a request.** If you choose not to run the command, send a CAFleet message back to the member explaining why. The member is waiting on either `! <command>` output OR a follow-up message — silence breaks the workflow.

## Serialization — process one request at a time

Concurrent member requests serialize through the broker queue. You MUST process command-request messages one at a time in the order returned by `cafleet message poll`:

1. Poll → take the first command-request in the returned list (the broker orders by `Task.status_timestamp.desc()` — newest-first).
2. Dispatch via `member exec` (or refuse).
3. ACK.
4. Poll the next one.

Do not interleave or batch. The poll order (newest-first today) is the serialization mechanism — no separate queueing primitive is needed. Batching dispatches across multiple members can cause `! <command>` keystrokes to land in the wrong pane state if a member is mid-prompt.

## Cross-Director boundary

The `cafleet member exec` CLI verifies `placement.director_agent_id` matches `--agent-id` before making any tmux call. An attempt to dispatch into another Director's member exits 1 with `Error: agent <member-id> is not a member of your team (director_agent_id=<other-director>).` This is enforced at the broker; you do not need to re-check it, but you should not attempt cross-Director dispatch in the first place — it indicates a misconfigured monitoring loop or a confused team-graph.

## When you, as Director, want to run your own command

This protocol is **member → Director only**. Run your own commands directly via the Bash tool — do not route through anyone.

## Why this works

- **Members spawn with `--permission-mode dontAsk`**, so under the default flow they run cafleet (and any other shell command) themselves via the Bash tool. The bash-via-Director path fires only when the member's harness deny-list rejects the command.
- **Claude Code's `!` shortcut is the dispatch primitive** — `cafleet member exec` keystrokes `! <command>` + Enter into the member's pane, and Claude Code's `!` shortcut runs the command. The captured stdout/stderr lands in the member's next-turn context.
- **You stay in control of fallback dispatches.** Every fallback request surfaces as a plain message in your inbox; you (with the operator at your keyboard) choose whether to fulfill it. The operator's `permissions.ask` for `cafleet --session-id * member exec *` controls the per-call confirmation UX.
