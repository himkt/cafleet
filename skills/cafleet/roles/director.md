# Director Role — Bash Routing

You are a **Director** managing one or more members in a CAFleet team. Members spawn with `--permission-mode dontAsk`, so by default they run shell commands themselves via the Bash tool — no Director routing required.

The bash-via-Director protocol is the fallback when a member's Bash invocation is rejected by the Claude Code harness deny-list (destructive operations such as `git push`, `rm -rf`). In that case the member sends you a plain CAFleet message asking for the command. You decide whether to fulfill, and dispatch the command into the member's pane via one of two sibling subcommands picked by the smart-routing rule below: `cafleet member safe-exec CMD` (silent fast-path — re-reads the operator's `Bash(...)` allow / deny patterns and dispatches when an allow pattern matches) or `cafleet member exec CMD` (operator-prompted fallback — bare dispatch with no internal permission decision; Claude Code's outer `permissions.ask` on your invocation is the operator-confirmation surface).

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

2. **Smart-routing: try `safe-exec` first, fall back to `exec` on ask.**

   **Step 2a — try `cafleet member safe-exec CMD`:**

   ```bash
   cafleet --session-id <session-id> member safe-exec \
     --agent-id <director-agent-id> --member-id <member-agent-id> \
     "<command>"
   ```

   `safe-exec` is permission-aware: it re-reads three `settings.json` files (project-local → project shared → user, in matcher precedence order) on every call and matches the inner CMD against the operator's `Bash(...)` allow / deny patterns. The decision is one of three:

   - **Allow** (exit 0) — the inner CMD matches an allow pattern. The CLI prepends `! ` and appends `Enter` (two `tmux send-keys` calls: literal `! <command>`, then the `Enter` keystroke). Claude Code's `!` shortcut intercepts the line, runs the command via the harness's native CLI primitive (bypassing the Bash tool permission system), and prints the captured output back into the member's pane. The member's next prompt iteration sees the output as context. Done.
   - **Deny** (exit 2) — the inner CMD matches a deny pattern. Nothing is dispatched. See the "Handling deny" paragraph below. Do NOT attempt `exec` as a workaround — deny is intentional.
   - **Ask** (exit 3) — no allow pattern matches. Nothing is dispatched. Proceed to step 2b.

   **Step 2b — fall back to `cafleet member exec CMD` (only when safe-exec exited 3):**

   ```bash
   cafleet --session-id <session-id> member exec \
     --agent-id <director-agent-id> --member-id <member-agent-id> \
     "<command>"
   ```

   `exec` is a bare dispatcher — no internal permission decision. The outer Bash hook layer (Claude Code's `permissions.ask` on your invocation, configured via `Bash(cafleet --session-id * member exec *)` in the operator's `permissions.ask`) prompts the operator. On accept, the CLI keystrokes `! <command>` + Enter into the member's pane (exit 0). On cancel, Claude Code rejects the outer Bash invocation and `exec` never runs. This routes the inner-CMD decision through the operator per-call, surfacing commands the operator has not yet rule'd in `settings.json`.

3. **CMD argument shape:** both subcommands take a single positional `CMD` argument (no `--bash` flag). Empty strings, whitespace-only strings (after `.strip()`), and strings containing `\n` or `\r` are rejected with Click `UsageError` (exit 2). The legacy `--bash` form is gone — Click rejects it with `Error: No such option: '--bash'.` `member send-input` is unchanged (still AskUserQuestion-only with `--choice` / `--freetext`).

4. **Acknowledge the request.** ACK the member's message via `cafleet message ack --agent-id <director-agent-id> --task-id <task-id>` once you have dispatched (or refused, or relayed a deny / ask outcome to the operator). Leaving the message un-ACKed pollutes the inbox and breaks serialization.

5. **Refusing a request.** If you choose not to run the command, send a CAFleet message back to the member explaining why. The member is waiting on either `! <command>` output OR a follow-up message — silence breaks the workflow.

## Handling deny

When `safe-exec` exits 2, the inner CMD matched a `Bash(...)` deny pattern in one of the three `settings.json` files. The stderr block is structured for direct relay:

```
Error: command rejected by deny pattern Bash(<body>) declared in <file>. Offending command: <cmd>
```

Relay this stderr block (or the equivalent JSON `{outcome: "deny", matched_pattern, matched_file, offending_substring}` payload from `cafleet --json`) to the operator, plus a brief CAFleet message to the requesting member naming the deny pattern as the reason. The deny is intentional — the operator's `settings.json` is the source of truth. Do NOT attempt to bypass the deny by editing `settings.json` on the operator's behalf, by re-issuing the command with a tweaked spelling, or by routing through a different Director. If the operator believes the deny is a misconfiguration, the operator amends `settings.json` and the next `safe-exec` re-reads it (no caching).

## Handling ask

When `safe-exec` exits 3, no allow pattern matched the inner CMD. **The smart-routing rule says: fall back to `cafleet member exec ... CMD` automatically.** That single Bash invocation triggers Claude Code's outer `permissions.ask` prompt for the operator (configured via `Bash(cafleet --session-id * member exec *)`); on accept the CMD dispatches into the member pane (exit 0), on cancel Claude Code rejects the outer Bash and `exec` never runs.

Do not stall on `safe-exec`'s ask-block stderr. The stderr block lists the three resolved settings file paths and a suggested pattern:

```
Error: no allow pattern matches "<cmd>". Add a Bash(...) pattern to one of:
  - <project-local-path>
  - <project-path>
  - <user-path>
Files were re-read at this invocation. Suggested pattern: Bash(<first-token>:*)
```

Treat this output as informational — the suggested pattern is a hint the operator can adopt later if they want to silence future prompts on the same command. The immediate action is the `exec` fallback. Only ask the operator to amend `settings.json` directly when:

- The operator explicitly asked you to make a particular command silent, or
- The same `ask` block has surfaced repeatedly for routine commands and the operator has signaled annoyance.

If the operator cancels the outer prompt during the `exec` fallback, send a CAFleet message back to the requesting member explaining that the command remained unauthorized and the member should pick a different approach (correct the command, narrow the scope, or escalate to the operator directly).

## Serialization — process one request at a time

Concurrent member requests serialize through the broker queue. You MUST process command-request messages one at a time in the order returned by `cafleet message poll`:

1. Poll → take the first command-request in the returned list (the broker orders by `Task.status_timestamp.desc()` — newest-first).
2. Dispatch via the smart-routing rule (`safe-exec` first, `exec` on ask) — or refuse, or relay deny to the operator.
3. ACK.
4. Poll the next one.

Do not interleave or batch. The poll order (newest-first today) is the serialization mechanism — no separate queueing primitive is needed. Batching dispatches across multiple members can cause `! <command>` keystrokes to land in the wrong pane state if a member is mid-prompt.

## Cross-Director boundary

Both `cafleet member safe-exec` and `cafleet member exec` verify `placement.director_agent_id` matches `--agent-id` before any tmux call (and, for `safe-exec`, before reading any settings file). An attempt to dispatch into another Director's member exits 1 with `Error: agent <member-id> is not a member of your team (director_agent_id=<other-director>).` This is enforced at the broker; you do not need to re-check it, but you should not attempt cross-Director dispatch in the first place — it indicates a misconfigured monitoring loop or a confused team-graph.

## When you, as Director, want to run your own command

This protocol is **member → Director only**. Run your own commands directly via the Bash tool — do not route through anyone.

## Why this works

- **Members spawn with `--permission-mode dontAsk`**, so under the default flow they run cafleet (and any other shell command) themselves via the Bash tool. The bash-via-Director path fires only when the member's harness deny-list rejects the command.
- **Claude Code's `!` shortcut is the dispatch primitive** — on success, `cafleet member safe-exec` and `cafleet member exec` both keystroke `! <command>` + Enter into the member's pane, and Claude Code's `!` shortcut runs the command. The captured stdout/stderr lands in the member's next-turn context.
- **Two policy surfaces, one routing rule.** `safe-exec` consults the operator's `Bash(...)` allow / deny patterns silently (configured in `permissions.allow` so the outer Bash invocation never prompts). `exec` consults the operator at the outer Bash hook (configured in `permissions.ask` for `Bash(cafleet --session-id * member exec *)` so each invocation prompts). Smart-routing tries `safe-exec` first to keep routine pre-rule'd commands silent and falls back to `exec` only when `safe-exec` exits 3 (ask). The operator does not have to maintain a comprehensive allow list — unknown commands surface as outer-layer prompts via `exec`.
