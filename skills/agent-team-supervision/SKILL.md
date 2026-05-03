---
name: agent-team-supervision
description: Governance layer for CAFleet Directors. Loads agent-team-monitoring as a hard prerequisite. Defines Core Principle, Communication Model, Idle Semantics, Authorization-Scope Guard, Spawn Protocol, User Delegation, Stall Response (cross-reference), and Cleanup. Load both skills before any 'cafleet member create' call.
---

# CAFleet Agent Team Supervision

This skill builds on `Skill(agent-team-monitoring)`. Load monitoring first — it documents the cron-like mechanism that supervision is performed through. Supervision adds the always-applicable obligations and the Authorization-Scope Guard.

## Core Principle

**You are the instruction giver. If you stop giving instructions, the entire team stops.**

CAFleet members spawned via `cafleet member create` do not act autonomously. They respond to your messages and to the broker's auto-fired pane keystrokes. If you are not actively dispatching work, ACKing replies, and running supervision ticks, the team halts silently.

## Communication Model

Supervision happens over the CAFleet message broker. The flow:

1. The Director sends a message: `cafleet --session-id <session-id> message send --agent-id <director-agent-id> --to <member-agent-id> --text "..."`.
2. The broker persists the task and immediately keystrokes `cafleet --session-id <session-id> message poll --agent-id <member-agent-id>` into the recipient's tmux pane via `tmux send-keys` (the broker auto-fire).
3. The member's next turn picks up the polled task, processes it, and (when a reply is expected) sends a `cafleet message send` back to the Director.
4. The Director receives the reply on the next supervision tick (`/loop` for Claude Code, fallback driver for codex — see `Skill(agent-team-monitoring)` § Mechanism by backend) and ACKs it via `cafleet message ack`.

The Director never polls a member's pane via raw `tmux`. Inspection is via `cafleet member capture`; write is via `cafleet member send-input` / `cafleet member exec` / `cafleet member ping`. See `Skill(cafleet)` for the canonical command surface.

The Director's plain output is **not visible to members** — the only Director→member channel is `cafleet message send` (and the Director-only keystroke primitives above for special cases).

## Idle Semantics

**Members go idle after every turn. Idle is normal, not a stall.** A member that finished its turn and is awaiting the next instruction is doing exactly what it should.

- Idle members receive messages normally; the broker auto-fires a poll keystroke into the pane to wake them.
- Idle notifications are informational. Do not react to them unless you are ready to assign new work or to dispatch already-queued work (see Authorization-Scope Guard below).
- Do **not** nudge a member just because it went idle. Only nudge when idleness is **blocking your next step** AND health-check evidence (no recent message, no terminal forward progress) confirms a real stall.
- A member that has sent you a question and is awaiting your reply is idle by design — do not nudge it. Reply via `cafleet message send`.

Idleness alone is never a stop signal, never a stall, and never grounds for a passive-hold message. See the Authorization-Scope Guard below.

## Authorization-Scope Guard (CRITICAL)

**Absence of confirmation is not a stop signal.** When the user authorizes a
workstream ("execute the design doc", "process the review comments",
"continue with step N"), that authorization persists across `/loop` ticks
(claude), out-of-band scheduler firings (codex fallback), idle notifications,
and your own tool-result interpretations until the user issues an explicit
stop signal.

**You MUST dispatch queued work as soon as a teammate is idle and inputs are
available.** Examples of queued work that the Director MUST route immediately:

- Review comments waiting to be split across Programmer / Tester
- Next implementation step in the active design doc
- Reviewer feedback waiting to land at the Drafter
- A teammate's `cafleet message send` reply waiting to be ACK'd and acted on

**Do NOT** emit `Skipping. Holding for go.`, `Waiting for user confirmation
before dispatching`, or any equivalent passive-hold message in response to a
supervision tick or idle notification. The tick is a *health check*, not a
permission renewal request.

### What counts as a real stop signal

Only these gestures revoke prior authorization. Anything else is noise:

| Signal | Action |
|---|---|
| Explicit "stop", "wait", "halt", "pause" from the user | Stop dispatching; acknowledge; wait for the next instruction. |
| Profanity / frustration directed at your last action | Stop; acknowledge briefly; wait. Do not continue scheduled firings. |
| Repeated rejection of your tool calls (≥2 denials of the same operation in a row) | Stop the operation; surface the blocker; wait for guidance. |
| User typed `/clear` or restarted the session | Authorization is gone; do not resume from prior context without a new instruction. |

`/loop` cron firings, out-of-band scheduler firings, teammate idle
notifications, broker auto-fire receipts, and the absence of a fresh "go"
message are **not** stop signals. Treat them as inputs to evaluate, not gates
to pass through.

### When you genuinely need user input

If a queued action requires a *new* decision the user has not yet made
(choosing between options, approving a risky / remote-visible operation,
disambiguating a teammate's question), use `AskUserQuestion` — do **not**
emit a passive hold and wait. The hold message produces nothing; the question
unblocks you within seconds.

See `~/.claude/rules/skill-discovery.md` § *Authorization scope* and § *Stop
means stop* for the project-wide policy this section enforces.

## Spawn Protocol

Every time you spawn a member:

1. **Ensure the supervision mechanism is already running** — for Claude Code Directors, the `/loop` monitor must be active; for codex Directors, one of the fallbacks listed in `Skill(agent-team-monitoring)` § Mechanism by backend (out-of-band cron driver, MCP scheduling server, user-driven nudges, or no-active-monitor synchronous mode) must be in place. See `Skill(agent-team-monitoring)` § `/loop` Prompt Template for the canonical Claude Code setup.
2. **Spawn the member** via `cafleet --session-id <session-id> member create --agent-id <director-agent-id> --name <name> --description <desc> -- "<prompt>"`.
3. **Verify the member is active** by checking that `cafleet --session-id <session-id> member list --agent-id <director-agent-id>` shows the new member with a non-null `pane_id`.

Never spawn members without an active supervision mechanism. Never cancel the mechanism until all work is fully complete and the team is being shut down.

## User Delegation Protocol

CAFleet members never talk to the user directly — the Director relays. When a member sends a `cafleet message send` asking for user input:

1. **Classify the question shape:**
   - Choice among labelled options → `AskUserQuestion` with up to 4 options mirroring the member's labels; built-in "Other" handles custom text. Do NOT add an explicit "Write my own" option.
   - Open-ended / draft selection → `AskUserQuestion` with 2–4 complete candidate bodies so the user can compare wording side-by-side.
   - Yes/no → two-option `AskUserQuestion`.
2. **Ask the user.** No preamble sentence above the question — the conversation context plus the question text carry it.
3. **Relay the answer back** via `cafleet message send` to the originating member. Pass through the user's selection verbatim; do not substitute your own judgment. If the user chose "Other" and typed custom text, send the typed text.

**For `AskUserQuestion`-shaped pane prompts** (a member paused on the literal 4-option pane frame `1. … / 2. … / 3. … / 4. Type something`), follow the three-beat workflow in `Skill(cafleet)` § *Answer a member's AskUserQuestion prompt* (capture → user-facing decision prompt with shape-matched options → direct Bash invocation of the resolved `cafleet member send-input`). The pane-shapes table is canonical there; do not duplicate it.

**What you MUST NOT do:**

- Decide on the user's behalf, even when the answer looks obvious.
- Batch multiple members' questions into a single `AskUserQuestion` unless they are genuinely the same decision.
- Summarize or paraphrase the user's answer when relaying — pass it through.
- Print a fenced `bash` block of a `cafleet member send-input` invocation for the user to paste — invoke it via the Director's own Bash tool; the coding agent's per-call permission prompt is the consent surface.

## Stall Response

A member is stalled when it **blocks your next step** — not merely because it is idle. The 2-stage health check (`cafleet message poll` → `cafleet member capture`), the response-channels table, and the escalation rules live in `Skill(agent-team-monitoring)` § Stall Response. This skill does not duplicate them — load monitoring and follow that section.

## Cleanup Protocol

Cleanup follows `Skill(cafleet)` § Shutdown Protocol — that is the canonical teardown order (stop the supervision mechanism → `cafleet member delete` each member → verify roster empty → `cafleet session delete <session-id>` → `cafleet session list` sanity check).

The single rule supervision restates here: **stop the `/loop` cron (Claude Code) or fallback driver (codex) BEFORE deleting members.** A loop that keeps firing after `member delete` spams a tearing-down session, races with the delete path, and leaks cron output into the operator's terminal.

## Quick Reference

| Action | Primitive | Notes |
|---|---|---|
| Start the supervision tick | `/loop` (Claude Code) or fallback driver (codex) — see `Skill(agent-team-monitoring)` | First step — before any `cafleet member create` call |
| Spawn member | `cafleet --session-id <s> member create --agent-id <director> --name <n> --description <d> -- "<prompt>"` | Verify with `cafleet member list` |
| Message member | `cafleet --session-id <s> message send --agent-id <director> --to <member> --text "..."` | Broker auto-fires a `message poll` keystroke into the member's pane |
| ACK reply | `cafleet --session-id <s> message ack --agent-id <director> --task-id <task>` | Unacknowledged tasks accumulate; ACK every reply you act on |
| Inspect stalled member | `cafleet --session-id <s> member capture --agent-id <director> --member-id <member>` | Replaces raw `tmux capture-pane` |
| Manual inbox-poll nudge | `cafleet --session-id <s> member ping --agent-id <director> --member-id <member>` | Pre-approved; for missed auto-fires and post-`exec` chains |
| Shell-dispatch on member's behalf | `cafleet --session-id <s> member exec --agent-id <director> --member-id <member> "<cmd>"` | Per `Skill(cafleet)` § Routing Bash via the Director; follow with `member ping` |
| Answer 4-option pane prompt | `cafleet --session-id <s> member send-input --agent-id <director> --member-id <member> (--choice N \| --freetext "<text>")` | Delegate the decision via `AskUserQuestion` first; never decide silently |
| Relay user input | `AskUserQuestion` → `cafleet message send` | Pass-through; never substitute judgment |
| Shut down team | `Skill(cafleet)` § Shutdown Protocol | Stop loop → `member delete` each → `session delete` |
