---
name: cafleet-agent-team-supervision
description: "Governance layer for CAFleet Directors. Loads agent-team-monitoring as a hard prerequisite. Defines Core Principle, Communication Model, Idle Semantics, Authorization-Scope Guard, Spawn Protocol, User Delegation, Stall Response (cross-reference), and Cleanup. Load both monitoring and supervision whenever you are about to spawn or manage CAFleet team members (any 'cafleet member create' call)."
---

# CAFleet Agent Team Supervision

This skill builds on the `cafleet-agent-team-monitoring` skill. Load monitoring first — it documents the cron-like mechanism that supervision is performed through. Supervision adds the always-applicable obligations and the Authorization-Scope Guard.

## Core Principle

**You are the instruction giver. If you stop giving instructions, the entire team stops.**

CAFleet members spawned via `cafleet member create` do not act autonomously. They respond to your messages and to the broker's auto-fired pane keystrokes. If you are not actively dispatching work, ACKing replies, and running supervision ticks, the team halts silently.

## Communication Model

Supervision happens over the CAFleet message broker. The flow:

1. The Director sends a message: `cafleet --fleet-id <fleet-id> message send --agent-id <director-agent-id> --to <member-agent-id> --text "..."`.
2. The broker persists the task and immediately keystrokes a 2-line inline preview (`[cafleet msg …]` header + truncated body) into the recipient's pane via `tmux.send_inline_preview`. The recipient processes the preview as a fresh user-turn input — no `cafleet message poll` invocation is in the auto-fire path; to fetch the full body, the recipient calls `cafleet message poll` themselves.
3. The member's next turn picks up the polled task, processes it, and (when a reply is expected) sends a `cafleet message send` back to the Director.
4. The Director receives the reply on the next supervision tick (`/loop` for Claude Code, fallback driver for codex — see the `cafleet-agent-team-monitoring` skill § Mechanism by backend) and ACKs it via `cafleet message ack`.

The Director never polls a member's pane via raw `tmux`. Inspection is via `cafleet member capture`; write is via `cafleet member send-input` / `cafleet member exec` / `cafleet member ping`. See the `cafleet` skill for the canonical command surface.

The Director's plain output is **not visible to members** — the only Director→member channel is `cafleet message send` (and the Director-only keystroke primitives above for special cases).

## Idle Semantics

**Members go idle after every turn. Idle is normal, not a stall.** A member that finished its turn and is awaiting the next instruction is doing exactly what it should.

- Idle members receive messages normally; the broker keystrokes a 2-line inline preview into the pane via `tmux.send_inline_preview` to wake them.
- Idle notifications are informational. Do not react to them unless you are ready to assign new work or to dispatch already-queued work (see Authorization-Scope Guard below).
- Do **not** nudge a member just because it went idle. Only nudge when idleness is **blocking your next step** AND health-check evidence (no recent message, no terminal forward progress) confirms a real stall.
- A member that has sent you a question and is awaiting your reply is idle by design — do not nudge it. Reply via `cafleet message send`.

Idleness alone is never a stop signal, never a stall, and never grounds for a passive-hold message. See the Authorization-Scope Guard below.

## Authorization-Scope Guard (CRITICAL)

**Absence of confirmation is not a stop signal.** User authorization persists across `/loop` ticks, scheduler firings, broker auto-fires, and teammate idle notifications until an explicit stop signal arrives. The Director MUST dispatch queued work as soon as a teammate is idle and the inputs the work depends on are available; do NOT emit passive-hold messages in response to a supervision tick.

### Real stop signals (treat as halt; everything else is a tick to evaluate)

| Signal | Director response |
|---|---|
| User typed an explicit "stop" / "wait" / "pause" | Halt dispatch; wait for explicit re-authorization. |
| User typed profanity / frustration / a negative reaction | Halt dispatch; wait. Cron firings during this state are skipped silently. |
| User rejected your last 2+ tool calls | Halt dispatch; treat the rejections as a halt signal even if no profanity arrived. |
| User typed `/clear` or restarted the session | Authorization is gone; do not resume from prior context without a fresh instruction. |
| Member's reply contains a clear blocker; wait for guidance | Pause that one task only; continue dispatching to the rest of the team. |

`/loop` cron firings, out-of-band scheduler firings, teammate idle notifications, broker auto-fire receipts, and the absence of a fresh "go" message are **not** stop signals. Treat them as inputs to evaluate, not gates to pass through.

### When you genuinely need user input

If a queued action requires a *new* decision the user has not yet made (choosing between options, approving a risky / remote-visible operation, disambiguating a teammate's question), use `AskUserQuestion` — do **not** emit a passive hold and wait. The hold message produces nothing; the question unblocks you within seconds and produces a recorded answer.

## Spawn Protocol

Every time you spawn a member:

1. **Verify env, then ensure supervision is running**:
   - **Pre-spawn env-check (gating)**: run `cafleet doctor`. If it exits non-zero or reports missing `TMUX` / `TMUX_PANE`, ABORT the spawn protocol and surface the error to the user — `cafleet member create` requires the Director to be inside a tmux pane, and silently proceeding would fail later with a less-actionable error. This is the canonical pane-identity probe; do NOT reach for raw `tmux display-message` or `TMUX` env-var expansion. Backend binary availability (`claude` / `codex` / `opencode`) is NOT a separate pre-spawn step — `cafleet member create --coding-agent <backend>` performs its own `PATH` check and exits 1 with `Error: binary <name> not found on PATH` when missing. Do NOT run `<backend> --version` or `which <backend>` as a pre-spawn probe; trust the spawn-time check and let it surface the clean error.
   - **Ensure the supervision mechanism is already running** — for Claude Code Directors, the `/loop` monitor must be active; for codex or opencode Directors, one of the fallbacks listed in the `cafleet-agent-team-monitoring` skill § Mechanism by backend (out-of-band cron driver, MCP scheduling server for codex only, user-driven nudges, or no-active-monitor synchronous mode) must be in place. See the `cafleet-agent-team-monitoring` skill § `/loop` Prompt Template for the canonical Claude Code setup.
2. **Spawn the member** via `cafleet --fleet-id <fleet-id> member create --agent-id <director-agent-id> --name <name> --description <desc> --prompt-file <abs path to rendered prompt under ${BASE}/prompts/<role>-<UTC-compact>.md>`. The pre-spawn file IS both the CLI input AND the permanent audit artifact — see the `cafleet` skill's `reference/director.md` reference file § *Member Create — Scratch and audit files* for the canonical convention (including the `${BASE} == <unset>` guarded-skip + inline-positional fallback). Inline `-- "<prompt>"` is still permitted for trivial one-line ad-hoc spawns.
3. **Include the ready-signal directive in the spawn prompt.** Every spawn prompt MUST instruct the member, as its very first Bash call, to send `cafleet --fleet-id <fleet-id> message send --agent-id <my-agent-id> --to <director-agent-id> --text "ready"` (optionally `"ready: <brief role recap>"`). See the `cafleet` skill's `roles/member.md` reference file § *On Spawn — Send Ready Signal* for the canonical wording. A spawn prompt missing this directive is a defect — fix the prompt and re-spawn. The ready signal is the canonical "I am alive and accepting instructions" handshake; it is the ONLY signal that confirms the coding agent inside the pane has actually booted.
4. **Verify the member is placed** by checking that `cafleet --fleet-id <fleet-id> member list --agent-id <director-agent-id>` shows the new member with a non-null `pane_id`. This confirms the pane was created. Liveness of the coding agent inside the pane is confirmed asynchronously when the ready signal arrives — NOT by `member list`.
5. **End the active turn after spawn-and-verify.** The ready signal arrives via broker auto-fire (member's `cafleet message send` → 2-line inline preview keystroked into your pane via `tmux.send_inline_preview`), with `/loop` tick as the time-based backstop. You process it — ACK, dispatch first task — in your next active turn. See § *Asynchronous Wait Rule* below.

Never spawn members without an active supervision mechanism. Never cancel the mechanism until all work is fully complete and the team is being shut down.

### Asynchronous Wait Rule

The active turn consumes inputs that have already arrived and dispatches what is ready — then returns control. Waiting for things that have not yet arrived is the job of the wake-up channels: broker auto-fire keystroke into the Director's pane on every member `cafleet message send`, plus the `/loop` cron tick on its scheduled cadence.

| Situation | Director action |
|---|---|
| Just spawned a member; ready signal not yet arrived | End the turn. Auto-fire delivers the ready signal as it lands; `/loop` is the backstop. |
| Just dispatched to a member; reply not yet arrived | End the turn. Same wake-up channels surface the reply. |
| Waiting on multiple members' replies before next step | End the turn. React to each arrival as its own wake-up, not all-at-once. |
| User asks "what's the status?" while members are working | Report the asynchronous truth (e.g. "Alice is processing X; her completion will surface in my next turn"). For a live snapshot, use `cafleet member capture`. |
| Turn finished dispatching and ACKing | End the turn. The next wake-up reopens the turn when there is something to act on. |

## User Delegation Protocol

CAFleet members never talk to the user directly — the Director relays. When a member sends a `cafleet message send` asking for user input:

1. **Classify the question shape:**
   - Choice among labelled options → `AskUserQuestion` with up to 4 options mirroring the member's labels; built-in "Other" handles custom text. Do NOT add an explicit "Write my own" option.
   - Open-ended / draft selection → `AskUserQuestion` with 2–4 complete candidate bodies so the user can compare wording side-by-side.
   - Yes/no → two-option `AskUserQuestion`.
2. **Ask the user.** No preamble sentence above the question — the conversation context plus the question text carry it.
3. **Relay the answer back** via `cafleet message send` to the originating member. Pass through the user's selection verbatim; do not substitute your own judgment. If the user chose "Other" and typed custom text, send the typed text.

**For `AskUserQuestion`-shaped pane prompts** (a member paused on the literal 4-option pane frame `1. … / 2. … / 3. … / 4. Type something`), follow the three-beat workflow in the `cafleet` skill § *Answer a member's AskUserQuestion prompt* (capture → user-facing decision prompt with shape-matched options → direct Bash invocation of the resolved `cafleet member send-input`). The pane-shapes table is canonical there; do not duplicate it.

**What you MUST NOT do:**

- Decide on the user's behalf, even when the answer looks obvious.
- Batch multiple members' questions into a single `AskUserQuestion` unless they are genuinely the same decision.
- Summarize or paraphrase the user's answer when relaying — pass it through.
- Print a fenced `bash` block of a `cafleet member send-input` invocation for the user to paste — invoke it via the Director's own Bash tool; the coding agent's per-call permission prompt is the consent surface.

## Stall Response

See the `cafleet-agent-team-monitoring` skill § Stall Response.

## Cleanup Protocol

Cleanup follows the `cafleet` skill § Shutdown Protocol — that is the canonical teardown order (stop the supervision mechanism → `cafleet member delete` each member → verify roster empty → `cafleet fleet delete <fleet-id>` → `cafleet fleet list` sanity check).

The single rule supervision restates here: **stop the `/loop` cron (Claude Code) or fallback driver (codex) BEFORE deleting members.** A loop that keeps firing after `member delete` spams a tearing-down fleet, races with the delete path, and leaks cron output into the operator's terminal.

## Quick Reference

| Action | Primitive | Notes |
|---|---|---|
| Verify Director pane env | `cafleet doctor` | Pre-spawn precondition; gating. Aborts the spawn protocol when `TMUX` / `TMUX_PANE` are missing. Replaces raw `tmux display-message` and `TMUX` env-var expansion. |
| Start the supervision tick | `/loop` (Claude Code) or fallback driver (codex) — see the `cafleet-agent-team-monitoring` skill | Required before any `cafleet member create` call (after env-check). |
| Spawn member | `cafleet --fleet-id <s> member create --agent-id <director> --name <n> --description <d> --prompt-file <abs path to ${BASE}/prompts/<role>-<UTC-compact>.md>` | Pre-spawn file IS the audit artifact (see the `cafleet` skill's `reference/director.md` reference file § *Member Create — Scratch and audit files*). Verify with `cafleet member list`. Inline `-- "<prompt>"` is still permitted for trivial one-line spawns. |
| Message member | `cafleet --fleet-id <s> message send --agent-id <director> --to <member> --text "..."` | Broker keystrokes a 2-line inline preview into the member's pane via `tmux.send_inline_preview` |
| ACK reply | `cafleet --fleet-id <s> message ack --agent-id <director> --task-id <task>` | Unacknowledged tasks accumulate; ACK every reply you act on |
| Inspect stalled member | `cafleet --fleet-id <s> member capture --agent-id <director> --member-id <member>` | Replaces raw `tmux capture-pane` |
| Manual inbox-poll nudge | `cafleet --fleet-id <s> member ping --agent-id <director> --member-id <member>` | Pre-approved; for missed auto-fires and post-`exec` chains |
| Shell-dispatch on member's behalf | `cafleet --fleet-id <s> member exec --agent-id <director> --member-id <member> "<cmd>"` | Per the `cafleet` skill § Routing Bash via the Director; follow with `member ping` |
| Answer 4-option pane prompt | `cafleet --fleet-id <s> member send-input --agent-id <director> --member-id <member> (--choice N \| --freetext "<text>")` | Delegate the decision via `AskUserQuestion` first; never decide silently |
| Relay user input | `AskUserQuestion` → `cafleet message send` | Pass-through; never substitute judgment |
| Shut down team | the `cafleet` skill § Shutdown Protocol | Stop loop → `member delete` each → `fleet delete` |
