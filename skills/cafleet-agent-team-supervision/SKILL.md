---
name: cafleet-agent-team-supervision
description: "Governance layer for CAFleet Directors. Loads agent-team-monitoring as a hard prerequisite. Defines Core Principle, Communication Model, Idle Semantics, Authorization-Scope Guard, Spawn Protocol, User Delegation, Stall Response (cross-reference), and Cleanup. Load both monitoring and supervision whenever you are about to spawn or manage CAFleet team members (any 'cafleet member create' call)."
---

# CAFleet Agent Team Supervision

This skill builds on the `cafleet-agent-team-monitoring` skill. Load monitoring first — it documents the `cafleet monitor` heartbeat that supervision is performed through. Supervision adds the always-applicable obligations and the Authorization-Scope Guard.

**Coding-agent overlay.** These instructions are backend-neutral; read your overlay at [`../cafleet/reference/coding-agent/<name>.md`](../cafleet/reference/coding-agent/<name>.md) — `<name>` is your coding agent, named by your spawn prompt's `CODING AGENT:` line — and apply its deltas on top of them.

## Core Principle

**You are the instruction giver. If you stop giving instructions, the entire team stops.**

CAFleet members spawned via `cafleet member create` do not act autonomously. They respond to your messages and to the broker's auto-fired pane keystrokes. If you are not actively dispatching work, ACKing replies, and running supervision ticks, the team halts silently.

## Communication Model

Supervision happens over the CAFleet message broker: the Director `cafleet message send`s a member → the broker keystrokes a 2-line inline preview into the member's pane (it processes the preview as a fresh user-turn; the full body is fetched via `cafleet message poll`) → the member acts and replies via `cafleet message send` → the broker keystrokes that reply into the Director's pane, which the Director ACKs (`cafleet message ack`). The inline-preview mechanics are canonical in the `cafleet` skill § Send and [tmux-push.md](../../docs/concepts/tmux-push.md).

**Facilitation cue (load-bearing).** The monitor loop does **not** wake the Director (it wakes only the monitoring member — firing whenever a watched agent, the root Director at 180 s or a member at 720 s, is due on its own interval; see the `cafleet-agent-team-monitoring` skill § The monitor heartbeat). The Director is re-engaged on demand: by the monitoring member's idle nudge (`cafleet member nudge`, which persists an ACKable broker task **and** fires the hardened, `Esc`-safeguarded inline preview, when the monitoring member finds the Director idle), and by the broker's inline-preview keystroke on every inbound `cafleet message send`. **Treat each such re-engagement as the cue to run the entire 5-step facilitation loop** (poll → ACK → dispatch → health-check → escalate), NOT to read the inbox and stop.

The Director never polls a member's pane via raw `tmux`. Inspection is via `cafleet member capture`; write is via `cafleet member exec` / `cafleet member ping` (plus the decision-relay primitive your overlay describes). See the `cafleet` skill for the canonical command surface.

The Director's plain output is **not visible to members** — the only Director→member channel is `cafleet message send` (and the Director-only keystroke primitives above for special cases).

## Idle Semantics

**Members go idle after every turn. Idle is normal, not a stall.** A member that finished its turn and is awaiting the next instruction is doing exactly what it should.

- Idle members receive messages normally — the broker's inline preview wakes them.
- Idle notifications are informational. Do not react to them unless you are ready to assign new work or to dispatch already-queued work (see Authorization-Scope Guard below).
- Do **not** nudge a member just because it went idle. Only nudge when idleness is **blocking your next step** AND health-check evidence (no recent message, no terminal forward progress) confirms a real stall.
- A member that has sent you a question and is awaiting your reply is idle by design — do not nudge it. Reply via `cafleet message send`.

Idleness alone is never a stop signal, never a stall, and never grounds for a passive-hold message. See the Authorization-Scope Guard below.

## Authorization-Scope Guard (CRITICAL)

**Absence of confirmation is not a stop signal.** User authorization persists across the monitoring member's idle nudges, broker auto-fires, and teammate idle notifications until an explicit stop signal arrives. The Director MUST dispatch queued work as soon as a teammate is idle and the inputs the work depends on are available; do NOT emit passive-hold messages in response to a supervision tick.

### Real stop signals (treat as halt; everything else is a tick to evaluate)

| Signal | Director response |
|---|---|
| User typed an explicit "stop" / "wait" / "pause" | Halt dispatch; wait for explicit re-authorization. |
| User typed profanity / frustration / a negative reaction | Halt dispatch; wait. The monitoring member's idle nudges during this state are skipped silently. |
| User rejected your last 2+ tool calls | Halt dispatch; treat the rejections as a halt signal even if no profanity arrived. |
| User typed `/clear` or restarted the session | Authorization is gone; do not resume from prior context without a fresh instruction. |
| Member's reply contains a clear blocker; wait for guidance | Pause that one task only; continue dispatching to the rest of the team. |

The monitoring member's idle nudges, teammate idle notifications, broker auto-fire receipts, and the absence of a fresh "go" message are **not** stop signals. Treat them as inputs to evaluate, not gates to pass through.

### When you genuinely need user input

If a queued action requires a *new* decision the user has not yet made (choosing between options, approving a risky / remote-visible operation, disambiguating a teammate's question), use {decision_surface} (the canonical user-reaction rule is the `cafleet` skill § *Soliciting user reactions*) — do **not** emit a passive hold and wait. The hold message produces nothing; the question unblocks you within seconds and produces a recorded answer.

## Spawn Protocol

**Spawn order (first-in): the monitoring member comes first.** The **first** `cafleet member create` in the fleet IS the dedicated monitoring member (`--role monitor --model {monitor_model}`); it starts the monitor and gates every ordinary `member create` behind its `ready: monitor live` handshake. The Director never runs `cafleet monitor start` itself. See the `cafleet-agent-team-monitoring` skill § The monitoring member for the canonical spawn prompt.

Every time you spawn a member:

1. **Verify env, then ensure supervision is running**:
   - **Pre-spawn env-check (gating)**: run `cafleet doctor`. If it exits non-zero or reports missing `TMUX` / `TMUX_PANE`, ABORT the spawn and surface the error — `cafleet member create` requires the Director inside a tmux pane. This is the canonical pane-identity probe; do NOT use raw `tmux display-message` / `TMUX` expansion. Backend-binary availability is NOT a separate step — `member create` does its own `PATH` check and errors if the binary is missing (see [`cli-options.md`](../../docs/spec/cli-options.md#member-create)); do NOT pre-probe with `<backend> --version` / `which`.
   - **Monitoring member up + monitor live before any ordinary member** — the spawn-gate is canonical in the `cafleet-agent-team-monitoring` skill § The monitoring member (the first `member create` is `--role monitor --model {monitor_model}`; its `ready: monitor live` handshake gates the first ordinary `member create`; wait on the message, do not block-poll status).
2. **Spawn the member** via `cafleet member create --fleet-id <fleet-id> --agent-id <director-agent-id> --name <name> --description <desc> --prompt-file <abs path to ${BASE}/prompts/<role>-<UTC-compact>.md>`. The pre-spawn file IS both the CLI input and the permanent audit artifact; the audit-file convention (with the `${BASE} == <unset>` guarded-skip + inline fallback), the `--model` flag, and the model-name→backend inference are canonical in the `cafleet` skill's `reference/director.md` § Member Create.
3. **Include the ready-signal directive in the spawn prompt.** Every spawn prompt MUST instruct the member, as its first Bash call, to send `cafleet message send … --text "ready"` (canonical wording in the `cafleet` skill's `roles/member.md` § *On Spawn — Send Ready Signal*). It is the ONLY signal that the coding agent inside the pane has actually booted; a prompt missing it is a defect — fix and re-spawn.
4. **Verify the member is placed** by checking that `cafleet member list --fleet-id <fleet-id>` shows the new member with a non-null `pane_id`. This confirms the pane was created. Liveness of the coding agent inside the pane is confirmed asynchronously when the ready signal arrives — NOT by `member list`.
5. **End the active turn after spawn-and-verify.** The ready signal arrives via broker auto-fire (member's `cafleet message send` → 2-line inline preview keystroked into your pane via `tmux.send_inline_preview`), with the monitoring member's idle nudge as the time-based backstop. You process it — ACK, dispatch first task — in your next active turn. See § *Asynchronous Wait Rule* below.

Never spawn ordinary members before the monitoring member's `ready: monitor live` handshake. Never stop the monitor (the monitoring member's `monitor start` background task) until all work is fully complete and the team is being shut down.

### Asynchronous Wait Rule

The active turn consumes inputs that have already arrived and dispatches what is ready — then returns control. Waiting for things that have not yet arrived is the job of the wake-up channels: broker auto-fire keystroke into the Director's pane on every member `cafleet message send`, plus the monitoring member's idle nudge on the monitor's scheduled cadence.

| Situation | Director action |
|---|---|
| Just spawned a member; ready signal not yet arrived | End the turn. Auto-fire delivers the ready signal as it lands; the monitoring member's idle nudge is the backstop. |
| Just dispatched to a member; reply not yet arrived | End the turn. Same wake-up channels surface the reply. |
| Waiting on multiple members' replies before next step | End the turn. React to each arrival as its own wake-up, not all-at-once. |
| User asks "what's the status?" while members are working | Report the asynchronous truth (e.g. "Alice is processing X; her completion will surface in my next turn"). For a live snapshot, use `cafleet member capture`. |
| Turn finished dispatching and ACKing | End the turn. The next wake-up reopens the turn when there is something to act on. |

## User Delegation Protocol

CAFleet members never talk to the user directly — the Director relays. This is the relay-specific application of the canonical rule in the `cafleet` skill § *Soliciting user reactions* (the question-shape taxonomy is in your overlay). When a member sends a `cafleet message send` asking for user input:

1. **Classify the question shape** per the question-shape taxonomy in your overlay (choice among labeled options, approve / yes-no, continue-or-abort, or open-ended / draft selection), and present it through {decision_surface} (mirror the shape into options where {decision_surface} supports them). Follow your overlay for how the surface handles free-form text.
2. **Ask the user.** No preamble sentence above the question — the conversation context plus the question text carry it.
3. **Relay the answer back** via `cafleet message send` to the originating member. Pass through the user's selection verbatim; do not substitute your own judgment. If the user provided free-form text instead of a listed option, send that text.

**For a member paused on a decision-prompt pane frame** awaiting a user reaction, follow your overlay's decision-relay workflow — the concrete pane frame, the three-beat capture/ask/relay, and the pane-shapes table are backend deltas. The neutral pointer is the `cafleet` skill's `reference/director.md` § *Answering a member's relayed question*.

**What you MUST NOT do:**

- Decide on the user's behalf, even when the answer looks obvious.
- Batch multiple members' questions into a single user prompt unless they are genuinely the same decision.
- Summarize or paraphrase the user's answer when relaying — pass it through.
- Print a fenced `bash` block of a pane-relay command for the user to paste — invoke any such primitive via the Director's own Bash tool; the coding agent's per-call permission prompt is the consent surface.

## Stall Response

See the `cafleet-agent-team-monitoring` skill § Stall Response. (A quiet ordinary member is never woken by the monitor loop — the monitoring member's idle assessment surfaces it to the Director, who re-engages it via `cafleet member ping` or `cafleet message send`; ordinary-member re-engagement is always Director-mediated.)

## Cleanup Protocol

Cleanup follows the `cafleet` skill § Shutdown Protocol (first-out): stop the monitor's `monitor start` task FIRST → `cafleet member delete` the monitoring member → each ordinary member → verify the roster is empty → `cafleet fleet delete <fleet-id>` → `cafleet fleet list`. The full stop mechanism (no `cafleet monitor stop`; message the monitoring member to stop the task) and the race rationale are canonical there.

## Quick Reference

| Action | Primitive | Notes |
|---|---|---|
| Verify Director pane env | `cafleet doctor` | Pre-spawn precondition; gating. Aborts the spawn protocol when `TMUX` / `TMUX_PANE` are missing. Replaces raw `tmux display-message` and `TMUX` env-var expansion. |
| Start the supervision tick | Spawn the monitoring member first: `cafleet member create --fleet-id <s> --agent-id <director> --name monitor --description <…> --role monitor --model {monitor_model} --prompt-file <…>`; it runs `cafleet monitor start` in its own pane — see the `cafleet-agent-team-monitoring` skill | Its `ready: monitor live` handshake gates the first ordinary `member create`. |
| Spawn member | `cafleet member create --fleet-id <s> --agent-id <director> --name <n> --description <d> --prompt-file <abs path to ${BASE}/prompts/<role>-<UTC-compact>.md>` | Pre-spawn file IS the audit artifact (see the `cafleet` skill's `reference/director.md` reference file § *Member Create — Scratch and audit files*). Verify with `cafleet member list`. Inline `-- "<prompt>"` is still permitted for trivial one-line spawns. |
| Message member | `cafleet message send --fleet-id <s> --agent-id <director> --to <member> --text "..."` | Broker keystrokes an inline preview into the member's pane |
| ACK reply | `cafleet message ack --fleet-id <s> --agent-id <director> --task-id <task>` | Unacknowledged tasks accumulate; ACK every reply you act on |
| Inspect stalled member | `cafleet member capture --fleet-id <s> --member-id <member>` | Replaces raw `tmux capture-pane` |
| Manual inbox-poll nudge | `cafleet member ping --fleet-id <s> --member-id <member>` | Pre-approved; for missed auto-fires and post-`exec` chains |
| Shell-dispatch on member's behalf | `cafleet member exec --fleet-id <s> --member-id <member> "<cmd>"` | Per the `cafleet` skill § Routing Bash via the Director; follow with `member ping` |
| Answer a member's relayed question | the decision-relay primitive your overlay describes (`../cafleet/reference/coding-agent/<name>.md`) | Delegate the decision to the user via {decision_surface} first; never decide silently |
| Relay user input | {decision_surface} → `cafleet message send` | Pass-through; never substitute judgment |
| Shut down team | the `cafleet` skill § Shutdown Protocol | Stop monitor → delete monitoring member first → `member delete` each ordinary → `fleet delete` |
