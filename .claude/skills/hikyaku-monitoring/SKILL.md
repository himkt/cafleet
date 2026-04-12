---
name: hikyaku-monitoring
description: Mandatory supervision protocol for a Director managing member agents via Hikyaku. Defines monitoring loop, spawn protocol, and stall response using hikyaku-native commands.
---

# Hikyaku Monitoring Skill

## Core Principle

**You are the instruction giver. If you stop giving instructions, the entire team stops.**

Members spawned via `hikyaku member create` do not act autonomously. They respond to your messages. If you are not actively monitoring and instructing, work halts silently.

## Monitoring Mandate

Before spawning **any** member, start a `/loop` monitor with a **3-minute interval**. The loop uses hikyaku-native commands exclusively.

`$DIRECTOR_ID` is a placeholder for the agent ID you received from `hikyaku register`. Substitute your actual agent ID in all commands below.

| Step | Command | Purpose |
|---|---|---|
| 1 | `hikyaku member list --agent-id $DIRECTOR_ID` | Enumerate all live members and their pane status |
| 2 | `hikyaku poll --agent-id $DIRECTOR_ID` | Check inbox for progress reports or help requests from members |
| 3 | For each member with no recent message: `hikyaku member capture --agent-id $DIRECTOR_ID --member-id $MEMBER_ID` | Terminal capture fallback -- inspect what the member is doing when it has not reported in |
| 4 | Based on findings, `SendMessage` to any stalled or idle member with a specific instruction | Drive the team forward |
| 5 | When all members have reported completion (via messages or visible in terminal output), report to the user: "All deliverables are ready for review." | Signal completion to user |

**Lifecycle rule:** The loop MUST stay active from the first `member create` until the final shutdown (`CronDelete` only in the cleanup step). It must run through all phases: research, compilation, review, revision, user approval.

## Spawn Protocol

Every time you spawn a member:

1. Ensure the `/loop` monitor is already running (set it up if not).
2. Spawn the member via `hikyaku member create --agent-id $DIRECTOR_ID --name <name> --description <desc> -- "<prompt>"`.
3. Verify the member is active by checking `hikyaku member list` output shows the new member with a non-null `pane_id`.

Never spawn members without an active monitor. Never cancel the monitor until all work is fully complete and the team is being shut down.

## Stall Response -- 2-Stage Health Check

When you receive any signal that a member may be stalled (loop check, idle notification, user nudge), evaluate using this 2-stage protocol:

### Stage 1 -- Message-based check (`hikyaku poll`)

```bash
hikyaku poll --agent-id $DIRECTOR_ID --since "2026-04-12T10:00:00Z"
```

The `--since` flag accepts an ISO 8601 timestamp. If the member has sent a progress report or help request via `hikyaku send`, you can act on it immediately without interrupting the member's work. This is non-intrusive and preferred.

### Stage 2 -- Terminal capture fallback (`hikyaku member capture`)

```bash
hikyaku member capture --agent-id $DIRECTOR_ID --member-id $MEMBER_ID --lines 200
```

If `hikyaku poll` shows no recent messages from the member, fall back to capturing the terminal buffer. This is non-intrusive (read-only inspection that works even when the member is mid-task) and replaces raw `tmux capture-pane`.

### Escalation

If a member is still unresponsive after 2 nudges via `SendMessage` AND `hikyaku member capture` shows no forward progress in the terminal buffer, escalate to the user.

| Channel | Type | When to use |
|---|---|---|
| `hikyaku poll` | Non-intrusive, message-based | First -- check if the member has reported in |
| `hikyaku member capture` | Non-intrusive, terminal snapshot | Second -- when no messages, inspect what the member is doing |
| `SendMessage` | Interactive, authoritative | Third -- send a specific instruction to unstick the member |
| Escalate to user | Last resort | After 2 nudges + no progress in terminal |

## `/loop` Prompt Template

Replace `<director-agent-id>` with the actual agent ID obtained from `hikyaku register` output.

```
Monitor team health (interval: 3 minutes). For each member spawned via hikyaku member create:

1. Run `hikyaku --json member list --agent-id <director-agent-id>` to get all members.
2. Run `hikyaku --json poll --agent-id <director-agent-id> --since "<ISO 8601 timestamp of last check>"` to check for incoming messages. ACK any progress reports.
3. For each member that has NOT sent a message since last check, run `hikyaku member capture --agent-id <director-agent-id> --member-id <member_id> --lines 200` to inspect their terminal.
4. If a member's terminal shows no forward progress since last check, send them a specific instruction via SendMessage: "Report your progress now. If blocked, state what is blocking you."
5. If all members have reported completion (via messages or visible in terminal output), report to the user: "All deliverables are ready for review."
6. If a member has been nudged 2 times with no progress, escalate to the user.
```
