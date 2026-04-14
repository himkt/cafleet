---
name: cafleet-monitoring
description: Mandatory supervision protocol for a Director managing member agents via CAFleet. Defines monitoring loop, spawn protocol, and stall response using cafleet-native commands.
---

# CAFleet Monitoring Skill

## Core Principle

**You are the instruction giver. If you stop giving instructions, the entire team stops.**

Members spawned via `cafleet member create` do not act autonomously. They respond to your messages. If you are not actively monitoring and instructing, work halts silently.

**Agent-agnostic monitoring**: This protocol works identically for all coding agent backends (Claude, Codex, etc.). `cafleet member capture` captures terminal output and `cafleet member delete` sends `/exit` regardless of which coding agent is running in the pane. The `--coding-agent` flag only affects `member create`; all other member commands are backend-agnostic.

## Placeholder convention

Every command below uses angle-bracket tokens (`<session-id>`, `<director-agent-id>`, `<member-agent-id>`) as **placeholders, not shell variables**. Substitute the literal UUID strings printed by `cafleet session create` and `cafleet register` directly into the command. Do **not** introduce shell variables (`$DIRECTOR_ID`, `$MEMBER_ID`, etc.) — `permissions.allow` matches command strings literally and shell expansion breaks that matching.

- `<session-id>` — the session UUID printed by `cafleet session create` (a single value reused across every command in this Director's run)
- `<director-agent-id>` — the Director's agent UUID returned by `cafleet ... register`
- `<member-agent-id>` — a target member's agent UUID (from `member create` / `member list`)

## Monitoring Mandate

Before spawning **any** member, start a `/loop` monitor with a **3-minute interval**. The loop uses cafleet-native commands exclusively.

| Step | Command | Purpose |
|---|---|---|
| 1 | `cafleet --session-id <session-id> --agent-id <director-agent-id> member list` | Enumerate all live members and their pane status |
| 2 | `cafleet --session-id <session-id> --agent-id <director-agent-id> poll` | Check inbox for progress reports or help requests from members |
| 3 | For each member with no recent message: `cafleet --session-id <session-id> --agent-id <director-agent-id> member capture --member-id <member-agent-id>` | Terminal capture fallback -- inspect what the member is doing when it has not reported in |
| 4 | Based on findings, `cafleet --session-id <session-id> --agent-id <director-agent-id> send --to <member-agent-id> --text "..."` to any stalled or idle member with a specific instruction | Drive the team forward |
| 5 | When all members have reported completion (via messages or visible in terminal output), report to the user: "All deliverables are ready for review." | Signal completion to user |

**Lifecycle rule:** The loop MUST stay active from the first `member create` until the final shutdown cleanup step. Keep it running through all phases: research, compilation, review, revision, and user approval, and only stop it after deleting members with `cafleet ... member delete ...` and then deregistering the Director with `cafleet ... deregister ...`.

## Spawn Protocol

Every time you spawn a member:

1. Ensure the `/loop` monitor is already running (set it up if not).
2. Spawn the member via `cafleet --session-id <session-id> --agent-id <director-agent-id> member create --name <name> --description <desc> -- "<prompt>"`.
3. Verify the member is active by checking `cafleet --session-id <session-id> --agent-id <director-agent-id> member list` output shows the new member with a non-null `pane_id`.

Never spawn members without an active monitor. Never cancel the monitor until all work is fully complete and the team is being shut down.

## Stall Response -- 2-Stage Health Check

When you receive any signal that a member may be stalled (loop check, idle notification, user nudge), evaluate using this 2-stage protocol:

### Stage 1 -- Message-based check (`cafleet poll`)

```bash
cafleet --session-id <session-id> --agent-id <director-agent-id> poll \
  --since "2026-04-12T10:00:00Z"
```

The `--since` flag accepts an ISO 8601 timestamp. If the member has sent a progress report or help request via `cafleet send`, you can act on it immediately without interrupting the member's work. This is non-intrusive and preferred.

### Stage 2 -- Terminal capture fallback (`cafleet member capture`)

```bash
cafleet --session-id <session-id> --agent-id <director-agent-id> member capture \
  --member-id <member-agent-id> --lines 200
```

If `cafleet poll` shows no recent messages from the member, fall back to capturing the terminal buffer. This is non-intrusive (read-only inspection that works even when the member is mid-task) and replaces raw `tmux capture-pane`.

### Escalation

If a member is still unresponsive after 2 nudges via `cafleet send` AND `cafleet member capture` shows no forward progress in the terminal buffer, escalate to the user.

| Channel | Type | When to use |
|---|---|---|
| `cafleet ... poll` | Non-intrusive, message-based | First -- check if the member has reported in |
| `cafleet ... member capture` | Non-intrusive, terminal snapshot | Second -- when no messages, inspect what the member is doing |
| `cafleet ... send --to <member-agent-id> --text "..."` | Interactive, authoritative | Third -- send a specific instruction to unstick the member (push notification triggers the member's pane to poll) |
| Escalate to user | Last resort | After 2 nudges + no progress in terminal |

## `/loop` Prompt Template

Substitute the literal UUIDs into every `<session-id>`, `<director-agent-id>`, and `<member-agent-id>` placeholder before passing the prompt to `/loop`. The prompt must contain literal UUIDs, **not** shell variables — the `permissions.allow` matcher only allows literal command strings.

```
Monitor team health (interval: 3 minutes). For each member spawned via `cafleet member create`:

1. Run `cafleet --session-id <session-id> --json --agent-id <director-agent-id> member list` to get all members.
2. Run `cafleet --session-id <session-id> --json --agent-id <director-agent-id> poll --since "<ISO 8601 timestamp of last check>"` to check for incoming messages. ACK any progress reports.
3. For each member that has NOT sent a message since last check, run `cafleet --session-id <session-id> --agent-id <director-agent-id> member capture --member-id <member-agent-id> --lines 200` to inspect their terminal.
4. If a member's terminal shows no forward progress since last check, send them a specific instruction via `cafleet --session-id <session-id> --agent-id <director-agent-id> send --to <member-agent-id> --text "Report your progress now. If blocked, state what is blocking you."` (the push notification will trigger the member's pane to poll and pick up the message).
5. If all members have reported completion (via messages or visible in terminal output), report to the user: "All deliverables are ready for review."
6. If a member has been nudged 2 times with no progress, escalate to the user.
```
