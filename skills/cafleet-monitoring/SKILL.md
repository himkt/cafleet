---
name: cafleet-monitoring
description: Mandatory supervision protocol for a Director managing member agents via CAFleet. Defines monitoring loop, spawn protocol, and stall response using cafleet-native commands.
---

# CAFleet Monitoring Skill

## Core Principle

**You are the instruction giver. If you stop giving instructions, the entire team stops.**

Members spawned via `cafleet member create` do not act autonomously. They respond to your messages. If you are not actively monitoring and instructing, work halts silently.

## Placeholder convention

Every command below uses angle-bracket tokens (`<session-id>`, `<director-agent-id>`, `<member-agent-id>`) as **placeholders, not shell variables**. Substitute the literal UUID strings printed by `cafleet session create` (which returns both the session UUID and the root Director's `agent_id` — see the skill's `Typical Workflow` section for the exact output shape) directly into the command. Do **not** introduce shell variables for agent or session IDs — `permissions.allow` matches command strings literally, and shell expansion breaks that matching.

**Flag placement**: `--session-id` is a global flag (placed **before** the subcommand). `--agent-id` is a per-subcommand option (placed **after** the subcommand name). For example: `cafleet --session-id <session-id> message poll --agent-id <director-agent-id>`.

- `<session-id>` — the session UUID printed on line 1 of `cafleet session create` text output (or the `session_id` field in `--json` output)
- `<director-agent-id>` — the root Director's UUID printed on line 2 of `cafleet session create` text output (or `director.agent_id` in `--json` output). `cafleet session create` inside a tmux session now auto-bootstraps the root Director with its placement row — no separate `cafleet agent register` call is needed to obtain the Director's `agent_id`.
- `<member-agent-id>` — a target member's agent UUID (from `member create` / `member list`)

## Monitoring Mandate

Before spawning **any** member, start a `/loop` monitor with a **1-minute interval** (the `CronCreate` / `ScheduleWakeup` floor — sub-minute polling is not supported by the harness). The loop uses cafleet-native commands exclusively.

| Step | Command | Purpose |
|---|---|---|
| 1 | `cafleet --session-id <session-id> member list --agent-id <director-agent-id>` | Enumerate all live members and their pane status |
| 2 | `cafleet --session-id <session-id> message poll --agent-id <director-agent-id>` | Check inbox for progress reports or help requests from members |
| 3 | For each member with no recent message: `cafleet --session-id <session-id> member capture --agent-id <director-agent-id> --member-id <member-agent-id>` | Terminal capture fallback -- inspect what the member is doing when it has not reported in. If the capture shows an `AskUserQuestion`-style prompt, see Stage 2 below for the `member send-input` escape hatch. |
| 4 | Based on findings, `cafleet --session-id <session-id> message send --agent-id <director-agent-id> --to <member-agent-id> --text "..."` to any stalled or idle member with a specific instruction | Drive the team forward |
| 5 | When all members have reported completion (via messages or visible in terminal output), report to the user: "All deliverables are ready for review." | Signal completion to user |

**Lifecycle rule:** The loop MUST stay active from the first `member create` through every phase (research, compilation, review, revision, user approval). At teardown, **stop the loop BEFORE deleting members** — this is step 1 of the Shutdown Protocol in `Skill(cafleet)` and is non-negotiable. A loop that keeps firing after members are deleted spams `member list` / `message poll` against a tearing-down session, can race with the member-delete path, and (most visibly) leaks cron output into the operator's terminal after the team is ostensibly gone. Full teardown order: `CronDelete` each `/loop` job → `cafleet member delete` each member → `cafleet member list` to verify the roster is empty → `cafleet session delete <session-id>` → `cafleet session list` sanity check. `cafleet member delete` now blocks until the pane is actually gone (15 s default timeout). On timeout (exit 2), inspect + answer the prompt via `member capture` + `send-input`, or escalate to `--force`. All verification is via cafleet primitives; raw `tmux` write or inspect commands are NOT used. See `Skill(cafleet)` → "Shutdown Protocol" for the authoritative procedure. Do **not** attempt `cafleet agent deregister --agent-id <director-agent-id>` on the root Director — it is rejected with `Error: cannot deregister the root Director; use 'cafleet session delete' instead.`. `session delete` is the only supported teardown path and performs the Director deregister atomically.

## Spawn Protocol

Every time you spawn a member:

1. Ensure the `/loop` monitor is already running (set it up if not).
2. Spawn the member via `cafleet --session-id <session-id> member create --agent-id <director-agent-id> --name <name> --description <desc> -- "<prompt>"`.
3. Verify the member is active by checking `cafleet --session-id <session-id> member list --agent-id <director-agent-id>` output shows the new member with a non-null `pane_id`.

Never spawn members without an active monitor. Never cancel the monitor until all work is fully complete and the team is being shut down.

## Stall Response -- 2-Stage Health Check

When you receive any signal that a member may be stalled (loop check, idle notification, user nudge), evaluate using this 2-stage protocol:

> **Bash request blocking case**: When `cafleet message poll` returns a member message asking for a shell command, dispatch via `cafleet member exec "<cmd>"` per `Skill(cafleet)` § Routing Bash via the Director. Member blocks until the keystroke lands; process requests one at a time, don't skip ahead to other inbox items.

### Stage 1 -- Message-based check (`cafleet message poll`)

```bash
cafleet --session-id <session-id> message poll --agent-id <director-agent-id> \
  --since "2026-04-12T10:00:00+00:00"
```

The `--since` flag accepts an ISO 8601 timestamp. The broker stores `status_timestamp` via `datetime.now(UTC).isoformat()`, which renders as `YYYY-MM-DDTHH:MM:SS.ffffff+00:00` (microsecond precision, `+00:00` suffix — **not** `Z`), and the `--since` filter is applied as a raw SQLite TEXT comparison, so pass timestamps in the same `+00:00` form to get correct ordering. If the member has sent a progress report or help request via `cafleet message send`, you can act on it immediately without interrupting the member's work. This is non-intrusive and preferred.

### Stage 2 -- Terminal capture fallback (`cafleet member capture`)

```bash
cafleet --session-id <session-id> member capture --agent-id <director-agent-id> \
  --member-id <member-agent-id> --lines 120
```

`--lines 120` is the recommended default (matches the cafleet skill). Re-run with `--lines 200` as a fallback only if the first capture is truncated above the AskUserQuestion frame (the `1. …`, `2. …`, `3. …`, `4. Type something` rows are not all visible).

If `cafleet message poll` shows no recent messages from the member, fall back to capturing the terminal buffer. This is non-intrusive (read-only inspection that works even when the member is mid-task) and replaces raw `tmux capture-pane`.

If the terminal buffer shows the member paused on an `AskUserQuestion` prompt (a list of "1. …", "2. …", "3. …", "4. Type something" rows), the correct unblock is `cafleet member send-input` — never raw `tmux send-keys` — and the Director MUST delegate the decision to the user via its own `AskUserQuestion` tool call BEFORE invoking the wrapper. The Director never picks the `--choice` digit or drafts the `--freetext` body on its own judgment. The full three-beat workflow (capture → Director-side `AskUserQuestion` with shape-matched options → direct Bash invocation of the resolved `cafleet member send-input`, gated by Claude Code's native per-call permission prompt) and the pane-shapes table live in the cafleet skill's "Answer a member's AskUserQuestion prompt" section — that is canonical; do not duplicate the table here.

### Escalation

If a member is still unresponsive after 2 nudges via `cafleet message send` AND `cafleet member capture` shows no forward progress in the terminal buffer, escalate to the user.

| Channel | Type | When to use |
|---|---|---|
| `cafleet ... message poll --agent-id <director-agent-id>` | Non-intrusive, message-based | First -- check if the member has reported in |
| `cafleet ... member capture --agent-id <director-agent-id>` | Non-intrusive, terminal snapshot | Second -- when no messages, inspect what the member is doing |
| `cafleet ... message send --agent-id <director-agent-id> --to <member-agent-id> --text "..."` | Interactive, authoritative | Third -- send a specific instruction to unstick the member (broker auto-fires a `cafleet message poll` keystroke into the member's pane after persisting the message) |
| `cafleet ... member ping --agent-id <director-agent-id> --member-id <member-agent-id>` | Interactive, fixed-action keystroke | Director's pre-approved manual inbox-poll nudge — fires the same `cafleet message poll` keystroke the broker auto-fires, but as a manual entry-point. Use when a member appears stalled despite a recent `message send` (the broker auto-fire was missed or the pane was busy when it arrived) or after a long idle window. No positional argument, pre-approved in `permissions.allow`. Same authorization boundary as `capture` / `send-input` / `exec`. Failures surface as exit 1 (the auto-fire path swallows them silently). |
| `cafleet ... member send-input --agent-id <director-agent-id> --member-id <member-agent-id> (--choice N \| --freetext "<text>")` | Interactive, restricted keystroke | `--choice` / `--freetext` answer an `AskUserQuestion`-shaped prompt — delegate the decision to the user via the Director's own `AskUserQuestion` tool call FIRST, then invoke the resolved command via the Director's Bash tool (Claude Code's native per-call permission prompt is the consent surface; never print a fenced `bash` block for the user to paste). See the cafleet skill's "Answer a member's AskUserQuestion prompt" section for the canonical three-beat workflow + pane-shapes table. Same authorization boundary as `capture`. |
| `cafleet ... member exec --agent-id <director-agent-id> --member-id <member-agent-id> "<cmd>"` | Interactive, keystroke dispatch | Director-only shell-dispatch primitive — keystrokes `! <cmd>` + Enter into the member's pane via Claude Code's `!` shortcut. Shell-dispatch only — for inbox-poll-only nudges use `member ping`. Same authorization boundary as `capture` / `send-input`. See `Skill(cafleet)` § Routing Bash via the Director. |
| `cafleet ... member delete --agent-id <director-agent-id> --member-id <member-agent-id> --force` | Interactive, destructive | When `member delete` has already exited 2 and `capture` + `send-input` have failed to unblock the pane — forces an atomic `kill_pane` + deregister + layout rebalance. Never fall back to raw `tmux kill-pane`. |
| Process pending shell-command request from member | Blocking on member side | Dispatch via `cafleet member exec "<cmd>"` per `Skill(cafleet)` § Routing Bash via the Director. Don't skip past a member's request — the member sits idle until the keystroke lands. |
| Escalate to user | Last resort | After 2 nudges + no progress in terminal |

## `/loop` Prompt Template

Substitute the literal UUIDs into every `<session-id>`, `<director-agent-id>`, and `<member-agent-id>` placeholder before passing the prompt to `/loop`. The prompt must contain literal UUIDs, **not** shell variables — the `permissions.allow` matcher only allows literal command strings. Remember: `--session-id` goes before the subcommand, `--agent-id` goes after.

```
Monitor team health (interval: 1 minute). For each member spawned via `cafleet member create`:

1. Run `cafleet --session-id <session-id> --json member list --agent-id <director-agent-id>` to get all members.
2. Run `cafleet --session-id <session-id> --json message poll --agent-id <director-agent-id> --since "<ISO 8601 timestamp of last check>"` to check for incoming messages. ACK any progress reports.
3. For each member that has NOT sent a message since last check, run `cafleet --session-id <session-id> member capture --agent-id <director-agent-id> --member-id <member-agent-id> --lines 200` to inspect their terminal.
4. If a member's terminal shows no forward progress since last check, send them a specific instruction via `cafleet --session-id <session-id> message send --agent-id <director-agent-id> --to <member-agent-id> --text "Report your progress now. If blocked, state what is blocking you."` (the broker auto-fires a `cafleet message poll` keystroke into the member's pane to pick up the message).
5. If a member appears stalled despite a recent `message send` (the broker auto-fire was missed or arrived while the pane was busy), or after a long idle window with a queued message still unread, fire `cafleet --session-id <session-id> member ping --agent-id <director-agent-id> --member-id <member-agent-id>` to manually inject the same poll keystroke. This is the dedicated re-poke primitive — pre-approved in `permissions.allow`, no positional argument. Do NOT use `cafleet member exec` for this purpose; that is for shell dispatch only.
6. If all members have reported completion (via messages or visible in terminal output), report to the user: "All deliverables are ready for review."
7. If a member has been nudged 2 times with no progress, escalate to the user.
```
