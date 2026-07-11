# CAFleet Team Supervision

Read this file for CAFleet team supervision — the Director-only governance and the `cafleet monitor` heartbeat mechanism it is performed through. It defines the always-applicable obligations (Core Principle, Communication Model, Idle Semantics, Authorization-Scope Guard, Spawn Protocol), the heartbeat mechanism (the monitor loop, how members are woken, the monitoring member, the 5-step facilitation loop, Monitor Lifecycle), and the recovery surface (Stall Response, User Delegation, Cleanup, Quick Reference). Ordinary members and standalone agents never load it.

**Required reading — read AND resolve your overlay first.** These instructions are backend-neutral and use `{placeholder}` tokens (`{monitor_model}`, `{decision_surface}`, `{permission_flags}`). Before acting on them, Read your overlay [`coding-agent/<name>.md`](coding-agent/) — `<name>` is the coding agent named on your spawn prompt's `CODING AGENT:` line — then **resolve** it per [`SKILL.md`](../SKILL.md) § *Resolve your overlay*: materialize each token to its overlay value (or the documented default) and apply each bound note before you act. Skip resolution and you emit a literal `{monitor_model}` (spawning the monitoring member with `--model {monitor_model}` instead of its real model), guess a wrong/default value, or ignore a backend note.

## Core Principle

**You are the instruction giver. If you stop giving instructions, the entire team stops.**

CAFleet members spawned via `cafleet member create` do not act autonomously. They respond to your messages and to the broker's auto-fired pane keystrokes. If you are not actively dispatching work, ACKing replies, and running supervision ticks, the team halts silently.

## Communication Model

Supervision happens over the CAFleet message broker: the Director `cafleet message send`s a member → the broker keystrokes a 2-line inline preview into the member's pane (it processes the preview as a fresh user-turn; the full body is fetched via `cafleet message poll`) → the member acts and replies via `cafleet message send` → the broker keystrokes that reply into the Director's pane, which the Director ACKs (`cafleet message ack`). The inline-preview mechanics are canonical in [`SKILL.md`](../SKILL.md) § Send and [`multiplexer-backends.md`](../../../docs/spec/multiplexer-backends.md#push-notifications).

**Long or multi-line bodies.** `message send` / `message broadcast` accept a `--text-file <path>` (or `--text-file -` for stdin) alternative to inline `--text`. A long or multi-line body MUST be passed via `--text-file`, never inline `--text`, so it never lands on the command line and hits the shell's `ARG_MAX` limit. Short one-line bodies stay fine inline with `--text`.

**Facilitation cue (load-bearing).** The monitor loop does **not** wake the Director (it wakes only the monitoring member — firing whenever a watched member is due on its own interval; see § The monitor heartbeat). When the Director is re-engaged on demand (§ The monitor heartbeat defines the two channels), **treat each such re-engagement as the cue to run the entire 5-step facilitation loop** (poll → ACK → dispatch → health-check → escalate), NOT to read the inbox and stop.

The Director never polls a member's pane via raw `tmux`. Inspection is via `cafleet member capture`; write is via `cafleet member exec` / `cafleet member ping`. See [`SKILL.md`](../SKILL.md) and [`reference/cli.md`](cli.md) for the canonical command surface.

The Director's plain output is **not visible to members** — the only Director→member channel is `cafleet message send` (and the Director-only keystroke primitives above for special cases).

## The monitor heartbeat

CAFleet members do not act autonomously. The Director drives the team — and the Director needs a way to wake itself up periodically to check inboxes, dispatch queued work, and detect stalls. That heartbeat is supplied by **`cafleet monitor`**, a per-fleet `scan → wake → sleep` loop that the fleet's dedicated **monitoring member** runs as a **background task** in its own pane. Because it is just a backgrounded command, the heartbeat is **backend-agnostic** — a root Director on `claude`, `codex`, or `opencode` gets the identical tick. There is no per-backend scheduling asymmetry: the monitor is the one mechanism for every backend.

Each tick the loop scans the **watched set** — the root Director (default **180 s**) and every ordinary member (default **720 s**), each on its own per-member interval — and, when **≥ 1 watched member is due**, wakes the monitoring member **once**. The loop's only keystroke is into the monitoring member's own pane; it **never** keystrokes a watched pane (the Director or an ordinary member). The wake nudge does **not** lead with `Esc`: the monitoring member's pane runs a read-only routine under `dontAsk` and is never parked on a permission-approval prompt, so a leading `Esc` would merely self-interrupt an in-progress routine. (The `Esc` safeguard instead lives where a target may be on a prompt — the broker's message-delivery inline preview and `cafleet member ping`.) The dedicated monitoring member is the **watcher**, not a watched member — it carries no interval and is located by its `member_card_json.cafleet.kind == "monitoring-member"` marker, not by a `monitor_config` row. The keystroke is a single-line *wake nudge* that **names** each due member as `<role> <id> (<name>) [<reasons>]` (role `director` or `member`; reasons drawn from `interval`, `status:done`, `stall-check`) plus the Director id as the standing inspect-and-re-engage target, instructing the monitoring member to classify each named pane plus the Director on the five-state taxonomy and re-engage the Director when a due member is `stalled`/`finished` (see [`roles/monitor.md`](../roles/monitor.md)).

See [`SKILL.md`](../SKILL.md) and the [Monitoring concepts page](https://himkt.github.io/cafleet/concepts/monitoring/) for the full command surface and policy. **The monitoring member — not the Director — runs `cafleet monitor start`** (see § Monitor Lifecycle).

**The Director is never woken by the loop.** It is re-engaged only on demand: by the monitoring member's report (`cafleet message send`, which persists an ACKable broker task **and** fires the `Esc`-safeguarded inline preview, when the routine reports a `stalled`/`finished` member or classifies the Director's own pane `finished` with un-acked work — but **never** when the Director's pane is `awaiting_user`), and by the broker's inline-preview keystroke on every other inbound `cafleet message send`. The preview's leading `Esc` exists to stop the trailing `Enter` from blindly *confirming* a prompt; a pending `AskUserQuestion` is protected because the monitoring member never re-engages an `awaiting_user` pane at all — by policy, not by the keystroke. When woken by one of those, the Director runs the entire 5-step facilitation loop (poll → ACK → dispatch → health-check → escalate), not just an inbox read. The monitor decides only *when* to wake the monitoring member; this file defines *what* the Director does once re-engaged.

### How ordinary members are woken

Ordinary members are **watched** (each enrolled with its own 720 s interval), but the loop **never keystrokes a member pane**. When a member comes due, the loop wakes the *monitoring member*, which captures the member read-only and surfaces a stall to the Director. Member re-engagement is always Director-mediated, via two paths:

1. **Primary** — the broker's inline-preview keystroke fired on every `cafleet message send` (`tmux.send_inline_preview`), landing the instant the Director or a teammate sends work.
2. **Manual recovery** — the Director's `Esc`-safeguarded `cafleet member ping` (it reuses the `send_poll_trigger` helper, so it inherits the same `Esc` safeguard), for a member that missed its inline preview or looks stalled.

A member that has gone quiet is surfaced to the Director by the monitoring member's assessment; the Director then re-pings via `cafleet member ping` or re-sends the instruction. The monitoring member never keystrokes task instructions into a member's pane.

## The monitoring member

The monitoring member is a single, dedicated coding-agent member — spawned **first** in the fleet with `cafleet member create --role monitor --model {monitor_model}` — that owns the heartbeat and applies LLM judgment to the watched members' state (the Director **and** each freshly-due member). `--role monitor` sets `member_card_json.cafleet.kind == "monitoring-member"`; the monitoring member is **not** enrolled in `monitor_config` — it is the watcher, located by that kind marker (`find_monitoring_member`), and carries no interval of its own. Only one is allowed per fleet (a second `--role monitor` spawn is rejected). It is the **one** process that runs `cafleet monitor start` (the Director never runs it — see § Spawn Protocol).

The monitoring member's own first-person routine — its Startup (the `ready: monitor live` gate), the on-wake capture-classify-reengage steps, the wake-nudge it consumes, Teardown, and its canonical spawn prompt (the [`reference/director.md`](director.md) skeleton plus a per-role delta) — lives in [`roles/monitor.md`](../roles/monitor.md).

## Idle Semantics

**A member at rest between turns is normal, not a stall.** A member that finished its turn with no assigned work outstanding is doing exactly what it should — leave it. When a member goes quiet, what you do depends on the pane state the monitoring member reports (the five-state taxonomy in [`roles/monitor.md`](../roles/monitor.md)):

- **`finished` with outstanding assigned work → drive it forward (issue #174 bullet 3).** A member that completed its turn while the task you assigned is unfinished is NOT left alone: dispatch the next step or re-engage it via `cafleet message send` / `cafleet member ping`. You alone judge whether assigned work remains — the monitoring member reports `finished`, you decide. This corrects the old "do not nudge a member just because it went idle" reading.
- **`finished` with nothing outstanding → leave it.** Expected rest; idle notifications about it are informational, not a call to act. Idle members receive messages normally — the broker's inline preview wakes them when you have new work.
- **`stalled` mid-execution → re-engage (issue #174 bullet 2).** The monitoring member surfaces a member whose capture is unchanged across two consecutive stall-check observations; re-engage via `cafleet member ping` / `cafleet message send`.
- **`awaiting_user` → never keystroke it (issue #174 bullet 1).** Never `cafleet member ping` or `cafleet message send` a member you know to be awaiting a user answer while its prompt is up — both primitives keystroke `Esc` first, which would cancel the pending prompt. Relay the answer through the user ({decision_surface}) and let the member consume it.
- A member that sent you a question and sits at an **idle** prompt awaiting your reply is not `awaiting_user` (its pane shows no live prompt) — reply via `cafleet message send`; the reply's `Esc`-first keystroke cancels nothing.

Idleness alone is never a stop signal, never a stall, and never grounds for a passive-hold message. See the Authorization-Scope Guard below.

## Authorization-Scope Guard (CRITICAL)

**Absence of confirmation is not a stop signal.** User authorization persists across the monitoring member's nudges, broker auto-fires, and teammate idle notifications until an explicit stop signal arrives. The Director MUST dispatch queued work as soon as a teammate is idle and the inputs the work depends on are available; do NOT emit passive-hold messages in response to a supervision tick.

### Real stop signals (treat as halt; everything else is a tick to evaluate)

| Signal | Director response |
|---|---|
| User typed an explicit "stop" / "wait" / "pause" | Halt dispatch; wait for explicit re-authorization. |
| User typed profanity / frustration / a negative reaction | Halt dispatch; wait. The monitoring member's nudges during this state are skipped silently. |
| User rejected your last 2+ tool calls | Halt dispatch; treat the rejections as a halt signal even if no profanity arrived. |
| User typed `/clear` or restarted the session | Authorization is gone; do not resume from prior context without a fresh instruction. |
| Member's reply contains a clear blocker; wait for guidance | Pause that one task only; continue dispatching to the rest of the team. |

The monitoring member's nudges, teammate idle notifications, broker auto-fire receipts, and the absence of a fresh "go" message are **not** stop signals. Treat them as inputs to evaluate, not gates to pass through.

### When you genuinely need user input

If a queued action requires a *new* decision the user has not yet made (choosing between options, approving a risky / remote-visible operation, disambiguating a teammate's question), use {decision_surface} (the canonical user-reaction rule is [`SKILL.md`](../SKILL.md) § *Soliciting user reactions*) — do **not** emit a passive hold and wait. The hold message produces nothing; the question unblocks you within seconds and produces a recorded answer.

## Spawn Protocol

**Spawn order (first-in): the monitoring member comes first.** The **first** `cafleet member create` in the fleet IS the dedicated monitoring member (`--role monitor --model {monitor_model}`); it starts the monitor and gates every ordinary `member create` behind its `ready: monitor live` handshake. The Director never runs `cafleet monitor start` itself. See [`roles/monitor.md`](../roles/monitor.md) for the canonical spawn prompt and routine.

Every time you spawn a member:

1. **Verify env, then ensure supervision is running**:
   - **Pre-spawn env-check (gating)**: run `cafleet doctor`. If it exits non-zero or fails to resolve a multiplexer backend, ABORT the spawn and surface the error — `cafleet member create` requires the Director inside a tmux or herdr pane. This is the canonical pane-identity probe; do NOT use raw `tmux display-message` / `TMUX` expansion or any other backend-specific env probe. Backend-binary availability is NOT a separate step — `member create` does its own `PATH` check and errors if the binary is missing (see [`cli-options.md`](../../../docs/spec/cli-options.md#member-create)); do NOT pre-probe with `<backend> --version` / `which`.
   - **Monitoring member up + monitor live before any ordinary member** — the spawn-gate is canonical in [`roles/monitor.md`](../roles/monitor.md) (the first `member create` is `--role monitor --model {monitor_model}`; its `ready: monitor live` handshake gates the first ordinary `member create`; wait on the message, do not block-poll status).
2. **Spawn the member** via `cafleet member create --fleet-id <fleet-id> --name <name> --description <desc> --text-file <abs path to ${BASE}/.prompts/<role>-<UTC-compact>.md>` (the Director is auto-resolved from the fleet row). The pre-spawn file IS both the CLI input and the permanent audit artifact; the audit-file convention (with the `${BASE} == <unset>` guarded-skip + inline fallback), the `--model` flag, and the model-name→backend inference are canonical in [`reference/director.md`](director.md) § Member Create.
3. **Include the ready-signal directive in the spawn prompt.** Every spawn prompt MUST instruct the member, as its first Bash call, to send `cafleet message send … --text "ready"` (canonical wording in [`roles/member.md`](../roles/member.md) § *On spawn — send the ready signal*). It is the ONLY signal that the coding agent inside the pane has actually booted; a prompt missing it is a defect — fix and re-spawn.
4. **Verify the member is placed** by checking that `cafleet member list --fleet-id <fleet-id>` shows the new member with a non-null `pane_id`. This confirms the pane was created. Liveness of the coding agent inside the pane is confirmed asynchronously when the ready signal arrives — NOT by `member list`.
5. **End the active turn after spawn-and-verify.** The ready signal arrives via broker auto-fire (member's `cafleet message send` → 2-line inline preview keystroked into your pane via `tmux.send_inline_preview`), with the monitoring member's nudge as the time-based backstop. You process it — ACK, dispatch first task — in your next active turn. See § *Asynchronous Wait Rule* below.

Never spawn ordinary members before the monitoring member's `ready: monitor live` handshake. Never stop the monitor (the monitoring member's `monitor start` background task) until all work is fully complete and the team is being shut down.

### Asynchronous Wait Rule

The active turn consumes inputs that have already arrived and dispatches what is ready — then returns control. Waiting for things that have not yet arrived is the job of the wake-up channels: broker auto-fire keystroke into the Director's pane on every member `cafleet message send`, plus the monitoring member's nudge on the monitor's scheduled cadence.

| Situation | Director action |
|---|---|
| Just spawned a member; ready signal not yet arrived | End the turn. Auto-fire delivers the ready signal as it lands; the monitoring member's nudge is the backstop. |
| Just dispatched to a member; reply not yet arrived | End the turn. Same wake-up channels surface the reply. |
| Waiting on multiple members' replies before next step | End the turn. React to each arrival as its own wake-up, not all-at-once. |
| User asks "what's the status?" while members are working | Report the asynchronous truth (e.g. "Alice is processing X; her completion will surface in my next turn"). For a live snapshot, use `cafleet member capture`. |
| Turn finished dispatching and ACKing | End the turn. The next wake-up reopens the turn when there is something to act on. |

## Team-facilitation instructions

On every supervision tick — whether fired by the monitoring member's on-demand nudge, by inbound work arriving via the broker's inline-preview keystroke, or executed inline within an active turn — the Director runs these five steps in order. The goal is to **facilitate the team in completing tasks**, not merely to detect stalls.

1. **Poll inbox.** `cafleet message poll --fleet-id <fleet-id> --member-id <director-member-id>` returns only the un-acked (`input_required`) deliveries; ACKing each one (step 2) consumes it — the poll semantics are canonical at § Stall Response → Stage 1.
2. **ACK every message** that requires no further action: `cafleet message ack --fleet-id <fleet-id> --member-id <director-member-id> --task-id <task-id>`. Unacknowledged tasks accumulate in the Director's inbox and obscure new arrivals.
3. **Dispatch queued work.** If a member is idle and inputs are available (review comments to route, the next implementation step in a design doc, reviewer feedback waiting at the Drafter, a teammate reply waiting to be acted on), send the instruction immediately via `cafleet message send`. **Do not wait for a fresh "go" from the user** — the user's original authorization persists across ticks; see § Authorization-Scope Guard.
4. **Run the health-check sequence** for any member that has not reported recent progress — cheapest, least-intrusive check first: (a) `cafleet member list` (enumerate members + pane status); (b) `cafleet message poll` (progress reports / help requests); (c) for a member silent since the last check, `cafleet member capture` to inspect it (a decision-prompt frame → see Stall Response for the decision-relay escape hatch); (d) `cafleet message send` a specific instruction to any stalled/idle member; (e) once all members report completion, tell the user "All deliverables are ready for review."
5. **Escalate** to the user via {decision_surface} whenever a queued action requires a *new* user decision (option choice, risky/remote-visible operation, ambiguous teammate question); for the stall path (two nudges with no progress) see § Stall Response → Escalation. Do **not** emit passive-hold messages like `Skipping. Holding for go.` — the tick is a health check, not a permission renewal.

## Monitor Lifecycle

| Phase | Action |
|---|---|
| Spawn the monitoring member (first-in) | The **first** `cafleet member create` in the fleet IS the monitoring member: `cafleet member create --fleet-id <fleet-id> --name monitor --description <…> --role monitor --model {monitor_model} --text-file <rendered monitor prompt>`. It boots, launches `cafleet monitor start` as a background task in its own pane, confirms `monitor status`, and sends `ready: monitor live` to the Director. |
| Gate ordinary members | Wait for the monitoring member's `ready: monitor live` message before the first ordinary `cafleet member create`. The Director MAY run `cafleet monitor status --fleet-id <fleet-id>` itself as optional corroboration, but it waits on the handshake message rather than block-polling status (consistent with the async wait rule). |
| Run work | The monitor wakes the monitoring member whenever a watched member is due on its own interval (the root Director at 180 s, ordinary members at 720 s); do not intervene unless an escalation arrives. Each on-demand nudge from the monitoring member (or inbound work via inline preview) is the Director's cue to run the 5-step facilitation loop above. |
| User review | Keep the monitoring member and its `monitor start` task running during the review cycle — revisions and re-reviews still count as in-progress work. |
| Teardown (first-out) | Stop the monitor's background task FIRST, then delete the monitoring member before ordinary members. The authoritative full ordering is [`reference/recovery.md`](recovery.md) § *Shutdown Protocol*. |

**Lifecycle rule (non-negotiable):** The monitoring member MUST stay running (with its `monitor start` task live) from the first `member create` through every phase; at teardown the monitor is stopped FIRST (first-out) — a monitor still ticking after the monitoring member is deleted races the delete path. The full stop mechanism + ordering is canonical in [`reference/recovery.md`](recovery.md) § *Shutdown Protocol*.

## Stall Response

When you receive any signal that a member may be stalled (the monitoring member's nudge, idle notification, user nudge), evaluate using this 2-stage protocol. A quiet ordinary member is never woken by the monitor loop — the monitoring member's capture-and-classify assessment surfaces a `stalled`/`finished` member to the Director, who re-engages it via `cafleet member ping` or `cafleet message send`; ordinary-member re-engagement is always Director-mediated.

> **Bash request blocking case**: When `cafleet message poll` returns a member message asking for a shell command, dispatch via `cafleet member exec "<cmd>"` per [`reference/exec-routing.md`](exec-routing.md). Member blocks until the keystroke lands; process requests one at a time, don't skip ahead to other inbox items.

### Stage 1 — Message-based check (`cafleet message poll`)

```bash
cafleet message poll --fleet-id <fleet-id> --member-id <director-member-id>
```

`cafleet message poll` returns only the un-acked (`input_required`) deliveries addressed to the Director, newest first. ACKing a delivery consumes it, so a later poll surfaces only what has arrived since the last ACK — there is no last-tick timestamp to track. If the member has sent a progress report or help request via `cafleet message send`, you can act on it immediately without interrupting the member's work. This is non-intrusive and preferred.

### Stage 2 — Terminal capture fallback (`cafleet member capture`)

```bash
cafleet member capture --fleet-id <fleet-id> \
  --member-id <member-id>
```

The `cafleet member capture` default is `--lines 20`; bump `--lines` to show more of a stalled member's buffer.

If `cafleet message poll` shows no recent messages from the member, fall back to capturing the terminal buffer. This is non-intrusive (read-only inspection that works even when the member is mid-task) and replaces raw `tmux capture-pane`.

> **The decision surface is a backend delta.** The concrete user-reaction surface by which the Director asks the user is backend-specific — see your overlay ([`coding-agent/<name>.md`](coding-agent/)). The canonical, backend-neutral user-reaction rule is [`SKILL.md`](../SKILL.md) § *Soliciting user reactions*.

### Escalation

If a member is still unresponsive after 2 nudges via `cafleet message send` AND `cafleet member capture` shows no forward progress in the terminal buffer, escalate to the user via {decision_surface} (per [`SKILL.md`](../SKILL.md) § *Soliciting user reactions*) with concrete options (e.g. re-nudge once more / re-spawn the member / drop its task).

The unblock primitives and their ordering — non-intrusive `cafleet message poll` → read-only `cafleet member capture` → authoritative `cafleet message send` → `cafleet member ping` (missed auto-fire / required post-`exec` follow-up) → `cafleet member exec "<cmd>"` (shell dispatch) → `cafleet member delete --force` (last resort, never raw `tmux kill-pane`) → escalate to the user via {decision_surface} — are documented in [`reference/director.md`](director.md), [`reference/recovery.md`](recovery.md), [`reference/exec-routing.md`](exec-routing.md), and the § Quick Reference table below.

## User Delegation Protocol

CAFleet members never talk to the user directly — the Director relays. This is the relay-specific application of the canonical rule in [`SKILL.md`](../SKILL.md) § *Soliciting user reactions* (the question-shape taxonomy is in your overlay). When a member sends a `cafleet message send` asking for user input:

1. **Classify the question shape** per the question-shape taxonomy in your overlay (choice among labeled options, approve / yes-no, continue-or-abort, or open-ended / draft selection), and present it through {decision_surface}, mirroring the shape into options where the surface supports them. Follow your overlay for how the surface handles free-form text.
2. **Ask the user.** No preamble sentence above the question — the conversation context plus the question text carry it.
3. **Relay the answer back** via `cafleet message send` to the originating member. Pass through the user's selection verbatim; do not substitute your own judgment. If the user provided free-form text instead of a listed option, send that text.

**What you MUST NOT do:**

- Decide on the user's behalf, even when the answer looks obvious.
- Batch multiple members' questions into a single user prompt unless they are genuinely the same decision.
- Summarize or paraphrase the user's answer when relaying — pass it through.
- Print a fenced `bash` block of a pane command (`member exec` / `member ping`) for the user to paste — invoke any such primitive via the Director's own Bash tool; the coding agent's per-call permission prompt is the consent surface.

## Cleanup Protocol

Cleanup follows [`reference/recovery.md`](recovery.md) § Shutdown Protocol (first-out); the full ordering, the stop mechanism (no `cafleet monitor stop`; message the monitoring member to stop the task), and the race rationale are canonical there.

## Quick Reference

| Action | Primitive | Notes |
|---|---|---|
| Verify Director pane env | `cafleet doctor` | Pre-spawn precondition; gating. Aborts the spawn protocol when no supported multiplexer (tmux or herdr) resolves. Replaces raw `tmux display-message` and `TMUX` env-var expansion. |
| Start the supervision tick | Spawn the monitoring member first: `cafleet member create --fleet-id <s> --name monitor --description <…> --role monitor --model {monitor_model} --text-file <…>`; it runs `cafleet monitor start` in its own pane — see [`roles/monitor.md`](../roles/monitor.md) | Its `ready: monitor live` handshake gates the first ordinary `member create`. |
| Spawn member | `cafleet member create --fleet-id <s> --name <n> --description <d> --text-file <abs path to ${BASE}/.prompts/<role>-<UTC-compact>.md>` | Pre-spawn file IS the audit artifact (see [`reference/director.md`](director.md) § *Member Create — Scratch and audit files*). Verify with `cafleet member list`. Inline `--text "<prompt>"` is still permitted for trivial one-line spawns. |
| Message member | `cafleet message send --fleet-id <s> --from-member-id <director> --to-member-id <member> --text "..."` | Broker keystrokes an inline preview into the member's pane |
| ACK reply | `cafleet message ack --fleet-id <s> --member-id <director> --task-id <task>` | Unacknowledged tasks accumulate; ACK every reply you act on |
| Inspect stalled member | `cafleet member capture --fleet-id <s> --member-id <member>` | Replaces raw `tmux capture-pane` |
| Manual inbox-poll nudge | `cafleet member ping --fleet-id <s> --member-id <member>` | Pre-approved; for missed auto-fires and post-`exec` chains |
| Shell-dispatch on member's behalf | `cafleet member exec --fleet-id <s> --member-id <member> "<cmd>"` | Per [`reference/exec-routing.md`](exec-routing.md); follow with `member ping` |
| Answer a member's relayed question | {decision_surface} → `cafleet message send` | Ask the user via {decision_surface} first, then relay the answer back to the member as a message; never decide silently |
| Relay user input | {decision_surface} → `cafleet message send` | Pass-through; never substitute judgment |
| Shut down team | [`reference/recovery.md`](recovery.md) § Shutdown Protocol | Stop monitor → delete monitoring member first → `member delete` each ordinary → `fleet delete` |
