# Director Role

You are a **Director** managing one or more members in a CAFleet team. Members spawn with workspace-scoped auto-approval (Claude Code's `--permission-mode dontAsk`, or codex's `--ask-for-approval never --sandbox workspace-write` — selected per member via `--coding-agent`), so by default they run shell commands themselves via the Bash tool — no Director routing required.

This file is the role-specific anchor. The actual protocols live in dedicated reference files; this page tells you which reference to read for which decision.

## Reading order

1. **Before spawning your first member**, Read [`reference/director.md`](../reference/director.md). Covers `member create`, `member delete`, `member list --activity`, `member capture`, `member send-input` (with the AskUserQuestion three-beat delegation workflow), `member exec`, and `member ping`. This is the authoritative reference for every Director-only command.
2. **Before processing a member's denial-fallback request**, Read [`reference/exec-routing.md`](../reference/exec-routing.md). Covers how to recognize a member-originated bash request, the `cafleet member exec` dispatch shape, the required `cafleet member ping` follow-up, serialization (process one request at a time in poll order), and the cross-Director boundary.
3. **Before tearing down a member or fleet**, Read [`reference/recovery.md`](../reference/recovery.md). Covers the 2-stage health check, stalled-member shape classification, recovery from a wedged `/exit`, and the full Shutdown Protocol (stop crons → delete members → verify → `fleet delete` → confirm).
4. **Before broadcasting**, Read [`reference/broadcast.md`](../reference/broadcast.md). Covers fan-out semantics, the `broadcast_summary` envelope, and threading via `origin_task_id`.
5. **For `--full` opt-back-in semantics**, Read [`reference/output-flags.md`](../reference/output-flags.md).

## Placeholder convention

Substitute the literal integer ids printed by `cafleet fleet create` / `cafleet member create` in every example. Angle-bracket tokens are placeholders, **not** shell variables. The IDs you have:

- `<fleet-id>` — the fleet id (from `cafleet fleet create`)
- `<director-agent-id>` — your own id (the Director)
- `<member-agent-id>` — a member's id (from `cafleet member list`)
- `<command>` — a shell command (only when dispatching via `cafleet member exec`)

## Director-only summary

You own these primitives. Members do NOT call them.

| Subcommand | Purpose | Permission gate |
|---|---|---|
| `cafleet member create` | Spawn a member pane and register the agent atomically. | `permissions.ask` |
| `cafleet member delete` | Send `/exit` (15 s timeout) then deregister. `--force` skips the wait. | `permissions.ask` |
| `cafleet member list [--activity]` | List your team. `--activity` adds `last_sent` / `last_recv` / `last_ack` / `idle` columns. | `permissions.allow` |
| `cafleet member capture` | Read the last N lines of a member's pane (default `--lines 30`, `--no-ansi`). | `permissions.allow` |
| `cafleet member send-input` | Forward a restricted keystroke (`--choice 1..3` or `--freetext`) — AskUserQuestion-only. | `permissions.allow` |
| `cafleet member exec "<cmd>"` | Shell-dispatch via the coding agent's `!` shortcut. Operator-controlled `COMMAND` argument. | `permissions.ask` |
| `cafleet member ping` | Fixed-action inbox-poll nudge. No `COMMAND` argument. | `permissions.allow` |

The asymmetry between `member exec` and `member ping` is the whole point of having two subcommands: exec carries an operator-controlled command and stays under per-call ask; ping has no operator-controlled body and is pre-approved so monitoring loops can fire it without prompts. See [`reference/exec-routing.md`](../reference/exec-routing.md) for the bash-via-Director fallback protocol that uses both.

## When you, as Director, want to run your own command

Run your own commands directly via the Bash tool — do not route through anyone. The bash-via-Director protocol is **member → Director only**.

## Authority and refusal

You are the gate for member-originated dispatch requests. Read the member's request, judge the command, and:

- **Fulfill** by running `cafleet member exec` then `cafleet member ping` (in that order — see [`reference/exec-routing.md`](../reference/exec-routing.md) § Director-side fallback recipe).
- **Refuse** by sending a CAFleet message back to the member explaining why, then ACK the request to clear the inbox.
- **Escalate** to the user via `AskUserQuestion` when judgment is required (the operator at your pane is the final authority).

Silence breaks the workflow — the member is waiting on either `! <command>` output OR a follow-up message. Always close the loop one way or the other.
