# CAFleet Team Supervision

Read this file for CAFleet team supervision — the Director-only governance and the `cafleet monitor` heartbeat mechanism it is performed through. It defines the always-applicable obligations (Core Principle, Communication Model, Idle Semantics, Authorization-Scope Guard, Spawn Protocol), the heartbeat mechanism (the Director-hosted monitor loop, the periodic wake, the 5-step facilitation loop, Monitor Lifecycle), and the recovery surface (Stall Response, User Delegation, Cleanup, Quick Reference). Ordinary members and standalone agents never load it.

**Required reading — read AND resolve your overlay first.** These instructions are backend-neutral and use `{placeholder}` tokens (`{bg_run}`, `{bg_stop}`, `{decision_surface}`, `{permission_flags}`). Before acting on them, Read your overlay [`coding-agent/<name>-overlay.md`](coding-agent/) — `<name>` is the coding agent named on your spawn prompt's `CODING AGENT:` line — then **resolve** it per [`SKILL.md`](../SKILL.md) § *Resolve your overlay*: materialize each token to its overlay value (or the documented default) and apply each bound note before you act. Skip resolution and you emit a literal `{bg_run}` (describing the launch primitive instead of using it), guess a wrong/default value, or ignore a backend note.

## Core Principle

**You are the instruction giver. If you stop giving instructions, the entire team stops.**

CAFleet members spawned via `cafleet member create` do not act autonomously. They respond to your messages and to the broker's auto-fired pane keystrokes. If you are not actively dispatching work, ACKing replies, and running supervision ticks, the team halts silently.

## Communication Model

Supervision happens over the CAFleet message broker: the Director `cafleet message send`s a member → the broker keystrokes a 2-line inline preview into the member's pane (it processes the preview as a fresh user-turn; the full body is fetched via `cafleet message poll`) → the member acts and replies via `cafleet message send` → the broker keystrokes that reply into the Director's pane, which the Director ACKs (`cafleet message ack`). The inline-preview mechanics are canonical in [`SKILL.md`](../SKILL.md) § Send and [`multiplexer-backends.md`](../../../docs/docs/spec/multiplexer-backends.md#push-notifications).

**Long or multi-line bodies.** `message send` / `message broadcast` accept a `--text-file <path>` (or `--text-file -` for stdin) alternative to inline `--text`. A long or multi-line body MUST be passed via `--text-file`, never inline `--text`, so it never lands on the command line and hits the shell's `ARG_MAX` limit. Short one-line bodies stay fine inline with `--text`.

**Facilitation cue (load-bearing).** The monitor loop wakes **you** directly: once per wake interval it keystrokes the `[cafleet] tick:` payload into your own pane (see § The monitor heartbeat). **Treat each wake — and each inbound broker auto-fire — as the cue to run the entire 5-step facilitation loop** (poll → ACK → dispatch → health-check → escalate), NOT to read the inbox and stop. Then honor the wake's closing clause: resume your own work if something was still running when the keystroke landed.

The Director never polls a member's pane via raw `tmux`. Inspection is via `cafleet monitor capture`; write is via `cafleet member prompt` / `cafleet member ping`. See [`SKILL.md`](../SKILL.md) and [`reference/cli.md`](cli.md) for the canonical command surface.

The Director's plain output is **not visible to members** — the only Director→member channel is `cafleet message send` (and the Director-only keystroke primitives above for special cases).

## The monitor heartbeat

CAFleet members do not act autonomously. The Director drives the team — and the Director needs a way to wake itself up periodically to check inboxes, dispatch queued work, and detect stalls. That heartbeat is supplied by **`cafleet monitor`**, a per-fleet `scan → wake → sleep` loop that **you launch as a background task in your own pane** ({bg_run}) immediately after `cafleet fleet create`. Because it is just a backgrounded command, the heartbeat is **backend-agnostic** — a root Director on `claude`, `codex`, or `opencode` gets the identical tick.

The wake is **unconditional and fleet-level**: once per wake interval (default **600 s**; `monitor start --interval N` / `CAFLEET_MONITOR_WAKE_INTERVAL`, `0` disables the wake while the loop keeps heartbeating) the loop keystrokes one `Esc`-first payload into the Director's own pane — including when the fleet has no other members yet. There is no per-member schedule and no per-member due computation.

The byte-identical tmux/herdr wake is a **pure trigger**: it names every active member as `<member-id> (<name>; coding_agent=<agent>; unacked=<pending-count>)`, tells you to poll your inbox, ACK, and dispatch, and closes with the resume clause — `Resume your work if something was still running.` — the remedy for a keystroke that lands mid-turn. The loop never keystrokes a member's pane; `cafleet member ping` stays a Director-only manual primitive.

See [`SKILL.md`](../SKILL.md) and the [Monitoring concepts page](https://himkt.github.io/cafleet/concepts/monitoring) for the full command surface and policy. **You — the Director — run `cafleet monitor start`** (see § Monitor Lifecycle).

The periodic wake plus the broker auto-fire on every member `cafleet message send` are the two Director re-engagement channels.

### How ordinary members are woken

The loop never keystrokes a member pane. There are two paths:

1. **Primary** — the broker's inline-preview keystroke fired on every `cafleet message send` (`tmux.send_inline_preview`), landing the instant the Director or a teammate sends work.
2. **Director recovery** — on an on-tick health check you may use `cafleet member ping` or send a new instruction, but every such re-engagement keystroke requires the target-specific fresh-capture gate below.

## Idle Semantics

**A member at rest between turns is normal, not a stall.** A member that finished its turn with no assigned work outstanding is doing exactly what it should — leave it. On each wake, capture and classify quiet members at your own discretion using the pre-ping capture gate below; what you do depends on the pane state:

- **`finished` with outstanding assigned work → drive it forward.** A member that completed its turn while the task you assigned is unfinished is NOT left alone: dispatch the next step or re-engage it — through the pre-ping capture gate below — via `cafleet message send` / `cafleet member ping`. You alone judge whether assigned work remains.
- **`finished` with nothing outstanding → leave it.** Expected rest. Idle members receive messages normally — the broker's inline preview wakes them when you have new work (each such send still routes through the gate).
- **Quiet member unchanged across two consecutive wakes → re-engage it.** `stall_candidate` and `finished` are both quiet observations: when your fresh capture on this wake is byte-identical to the capture you took on the previous wake, the member is confirmed quiet — an idle member and a stalled one get the same treatment up to the re-engagement. Fire `cafleet member ping` (or a specific `cafleet message send`) through the gate; your own conversation notes are the baseline between wakes.
- **`unacked` is context, not proof.** The wake payload's per-member `unacked` count annotates your health check and never by itself authorizes a ping. A `working` member with pending deliveries is still `working` and non-actionable.
- **`awaiting_user` or `working` → skip the round.** The gate below defers the entire send; a pending user prompt is never destroyed and an in-flight turn is never interrupted.
- An immediate reply to a **reply-soliciting** message (a question or blocker) received from that member in the current facilitation turn is exempt from the gate: the member ended its turn to await this reply, so its pane is at rest with no live prompt — reply via `cafleet message send`; the reply's `Esc`-first keystroke cancels nothing. A reply to a progress-only status message ("still working", "ack") is NOT exempt — the member may still be mid-turn — and routes through the gate.

Idleness alone is never a stop signal, never a stall, and never grounds for a passive-hold message. See the Authorization-Scope Guard below.

### The pre-ping capture gate

Every **Director-initiated** re-engagement keystroke at a member — `cafleet
member ping`, a non-exempt `cafleet message send`, and `cafleet message
broadcast` — is capture-gated immediately before firing. Classify from
content only using the **target member's** backend
overlay; mixed fleets make this target-specific. The gate capture depth is
normative:

```bash
cafleet monitor capture --fleet-id <fleet-id> --member-id <target-member-id> --lines 120
```

| Capture classifies | Director action |
|---|---|
| `finished` | Fire the ping/send. |
| `stalled` (quiet, unchanged, no prompt, no in-flight work) | Fire the ping/send. |
| `awaiting_user` | **Skip this round.** Defer the entire send (nothing persisted, nothing keystroked). Do not relay the pane's prompt anywhere — the round is simply skipped. |
| `working` | **Skip this round.** Defer the entire send. The member surfaces its own result via `cafleet message send` when done. |
| `unknown` (dead / unreadable pane) | Do not ping. Enter the recovery path ([`reference/recovery.md`](recovery.md)) / § Stall Response → Escalation instead. |

The ambiguity tie-break: a capture that cannot distinguish `awaiting_user` from `finished` classifies `awaiting_user`. When in doubt between `stalled` and `working`, treat as `working` (skip the round) — a deferred ping costs one tick; an `Esc` into an in-flight turn destroys work. For the gate, `stalled` means the capture shows a quiet pane with no pending prompt and no in-flight work, in a context where your own prior capture showed the same content; your conversation notes across wakes are the baseline.

The gate is judgment applied at use time: knowledge from an earlier capture is *stale* and never substitutes for the fresh capture immediately before the keystroke.

A `cafleet message broadcast` fires the same `Esc`-first preview into every recipient pane, and recipients cannot be skipped individually within one send — so the broadcast fires only when **every** recipient's fresh capture classifies `finished` or `stalled`; otherwise defer the entire broadcast, or replace it with per-recipient gated unicasts.

**Exempt from the gate:** the immediate reply to a reply-soliciting message (bullet above), and `cafleet member prompt --shell` — member-requested shell dispatch per [`reference/prompt-routing.md`](prompt-routing.md), where the member is blocked *expecting* the keystroke.

## Authorization-Scope Guard (CRITICAL)

**Absence of confirmation is not a stop signal.** User authorization persists across periodic wakes, broker auto-fires, and teammate idle notifications until an explicit stop signal arrives. The Director MUST dispatch queued work as soon as a teammate is idle and the inputs the work depends on are available; do NOT emit passive-hold messages in response to a supervision tick.

### Real stop signals (treat as halt; everything else is a tick to evaluate)

| Signal | Director response |
|---|---|
| User typed an explicit "stop" / "wait" / "pause" | Halt dispatch; wait for explicit re-authorization. |
| User typed profanity / frustration / a negative reaction | Halt dispatch; wait. Periodic wakes during this state are read but not acted on. |
| User rejected your last 2+ tool calls | Halt dispatch; treat the rejections as a halt signal even if no profanity arrived. |
| User typed `/clear` or restarted the session | Authorization is gone; do not resume from prior context without a fresh instruction. |
| Member's reply contains a clear blocker; wait for guidance | Pause that one task only; continue dispatching to the rest of the team. |

Periodic wakes, teammate idle notifications, broker auto-fire receipts, and the absence of a fresh "go" message are **not** stop signals. Treat them as inputs to evaluate, not gates to pass through.

### When you genuinely need user input

If a queued action requires a *new* decision the user has not yet made (choosing between options, approving a risky / remote-visible operation, disambiguating a teammate's question), use {decision_surface} (the canonical user-reaction rule is [`SKILL.md`](../SKILL.md) § *Soliciting user reactions*) — do **not** emit a passive hold and wait. The hold message produces nothing; the question unblocks you within seconds and produces a recorded answer.

## Spawn Protocol

**Launch the heartbeat first.** Immediately after `cafleet fleet create` and **before** the first `cafleet member create`, launch `cafleet monitor start --fleet-id <fleet-id>` as a background task in your own pane ({bg_run}) and confirm the startup line the loop prints immediately after claiming the runtime row — `monitor loop started (fleet <fleet_id>, tick <tick>s, pid <pid>)` — in the task output. That confirmation is the gate on the first `member create`. A loop task that exits instead (runtime-claim conflict, dead fleet) is a failed start — resolve it before spawning anyone.

**Fleet bootstrap.** `cafleet fleet create --coding-agent <backend> --json` atomically creates the fleet, registers the root Director bound to the current pane, and writes its placement row in one transaction. Capture `fleet_id` and `director.member_id` from the JSON response and carry them as literal integers on every later call; the literal-id rule and the per-subcommand flag placement are canonical in [`SKILL.md`](../SKILL.md) § *Required Flags*.

Every time you spawn a member:

1. **Verify env, then ensure supervision is running**:
   - **Pre-spawn env-check (gating)**: run `cafleet doctor`. It reports the resolved multiplexer backend and the pane's session/window/pane identifiers. If it exits non-zero or fails to resolve a multiplexer backend, ABORT the spawn and surface the error — `cafleet member create` requires the Director inside a tmux or herdr pane. This is the canonical pane-identity probe; do NOT use raw `tmux display-message` / `TMUX` expansion or any other backend-specific env probe. Backend-binary availability is NOT a separate step — `member create` does its own `PATH` check and errors if the binary is missing (see [`cli-options.md`](../../../docs/docs/spec/cli-options.md#member-create)); do NOT pre-probe with `<backend> --version` / `which`.
   - **Monitor loop live before any member** — the `monitor loop started` startup line confirmed in your background task's output (§ above). A loop that has since exited is re-launched before spawning.
2. **Spawn the member** via `cafleet member create --fleet-id <fleet-id> --name <name> --description <desc> --text-file <abs path to ${BASE}/.prompts/<role>-<UTC-compact>.md>` (the Director is auto-resolved from the fleet row). The pre-spawn file IS both the CLI input and the permanent audit artifact; the audit-file convention (with the `${BASE} == <unset>` guarded-skip + inline fallback), the `--model` flag, and the model-name→backend inference are canonical in [`reference/director.md`](director.md) § Member Create.
3. **Include the ready-signal directive in the spawn prompt.** Every spawn prompt MUST instruct the member, as its first Bash call, to send `cafleet message send … --text "ready"` (canonical wording in [`roles/member.md`](../roles/member.md) § *On spawn — send the ready signal*). It is the ONLY signal that the coding agent inside the pane has actually booted; a prompt missing it is a defect — fix and re-spawn.
4. **Verify the member is placed** by checking that `cafleet member list --fleet-id <fleet-id>` shows the new member with a non-null `pane_id`. This confirms the pane was created. Liveness of the coding agent inside the pane is confirmed asynchronously when the ready signal arrives — NOT by `member list`.
5. **End the active turn after spawn-and-verify.** The ready signal arrives via broker auto-fire (member's `cafleet message send` → 2-line inline preview keystroked into your pane via `tmux.send_inline_preview`), with the periodic wake as the backstop when a member goes quiet without reporting. You process it — ACK, dispatch first task — in your next active turn. See § *Asynchronous Wait Rule* below.

Never spawn members before the startup-line confirmation. Keep the loop alive until all work is fully complete and the team is being shut down.

### Asynchronous Wait Rule

The active turn consumes inputs that have already arrived and dispatches what is ready — then returns control. Waiting for things that have not yet arrived is the job of the wake-up channels: broker auto-fire keystroke into the Director's pane on every member `cafleet message send`, plus the periodic monitor wake.

| Situation | Director action |
|---|---|
| Just spawned a member; ready signal not yet arrived | End the turn. Auto-fire delivers the ready signal as it lands; the periodic wake is the backstop. |
| Just dispatched to a member; reply not yet arrived | End the turn. Same wake-up channels surface the reply. |
| Waiting on multiple members' replies before next step | End the turn. React to each arrival as its own wake-up, not all-at-once. |
| User asks "what's the status?" while members are working | Report the asynchronous truth (e.g. "Alice is processing X; her completion will surface in my next turn"). For a live snapshot, use `cafleet monitor capture`. |
| Turn finished dispatching and ACKing | End the turn. The next wake-up reopens the turn when there is something to act on. |

## Team-facilitation instructions

On every supervision tick — whether fired by the periodic monitor wake, by inbound work arriving via the broker's inline-preview keystroke, or executed inline within an active turn — the Director runs these five steps in order. The goal is to **facilitate the team in completing tasks**, not merely to detect stalls.

1. **Poll inbox.** `cafleet message poll --fleet-id <fleet-id> --member-id <director-member-id>` returns only the un-acked (`input_required`) deliveries; ACKing each one (step 2) consumes it — the poll semantics are canonical at § Stall Response → Stage 1.
2. **ACK every message** that requires no further action: `cafleet message ack --fleet-id <fleet-id> --member-id <director-member-id> --message-id <message-id>`. Unacknowledged messages accumulate in the Director's inbox and obscure new arrivals.
3. **Dispatch queued work.** If a member is idle and inputs are available (review comments to route, the next implementation step in a design doc, reviewer feedback waiting at the Drafter, a teammate reply waiting to be acted on), send the instruction immediately via `cafleet message send`. **Do not wait for a fresh "go" from the user** — the user's original authorization persists across ticks; see § Authorization-Scope Guard.
4. **Run the health-check sequence** for any member that has not reported recent progress — cheapest, least-intrusive check first: (a) `cafleet member list` (enumerate members + pane status); (b) `cafleet message poll` (progress reports / help requests); (c) for a member silent since the last check, a fresh `cafleet monitor capture --lines 120` classified per § Idle Semantics → *The pre-ping capture gate* (a decision-prompt frame → see Stall Response for the decision-relay escape hatch); (d) `cafleet message send` a specific instruction — (c) is the gating precondition: (d) fires only for a member whose (c) capture classified `finished` or `stalled`; on `awaiting_user` or `working`, skip the round and defer the send; (e) once all members report completion, tell the user "All deliverables are ready for review."
5. **Escalate** to the user via {decision_surface} whenever a queued action requires a *new* user decision (option choice, risky/remote-visible operation, ambiguous teammate question); for the stall path (two fired sends with no progress) see § Stall Response → Escalation. Do **not** emit passive-hold messages like `Skipping. Holding for go.` — the tick is a health check, not a permission renewal.

After the five steps, honor the wake's resume clause: if the keystroke landed while your own task was mid-flight, pick that task back up before ending the turn.

### Routing member bash requests

The workflow's spawned members run in workspace-scoped auto-approval mode ({permission_flags}; Bash tool enabled, permission prompts auto-resolve), so they run shell commands directly by default. The bash-via-Director protocol is the fallback when a member's Bash invocation is denied by its coding-agent harness (destructive operations such as `git push` on claude/codex; any command outside the preset's deny-by-default allowlist on opencode). The member auto-routes by sending a plain shell-command request via `cafleet message send`, and you respond by sending `! <command>` keystrokes through `cafleet member prompt --shell`. Process such requests one at a time in poll order. Full invocation + flag layout in [`reference/prompt-routing.md`](prompt-routing.md).

## Monitor Lifecycle

| Phase | Action |
|---|---|
| Launch (before any member) | Immediately after `cafleet fleet create`, launch `cafleet monitor start --fleet-id <fleet-id>` as a background task in your own pane ({bg_run}) and confirm the startup line — `monitor loop started (fleet <fleet_id>, tick <tick>s, pid <pid>)` — in the task output. That confirmation gates the first `cafleet member create`. |
| Run work | One `Esc`-first wake lands in your pane per wake interval (default 600 s), naming every member and its `unacked` count; each wake (or inbound work via inline preview) is the cue to run the 5-step facilitation loop above. |
| User review | Keep the loop's background task running during the review cycle — revisions and re-reviews still count as in-progress work. |
| Teardown | Stop the background task FIRST ({bg_stop}) — the loop's signal handler runs its ownership-checked runtime clear — then delete members. The full ordering is § *Cleanup Protocol*. |

**Lifecycle rule (non-negotiable):** The monitor loop MUST stay running from before the first `member create` through every phase, until the teardown above stops it.

## Stall Response

The wake payload's per-member `unacked` counts and your own captures across wakes are the evidence for the facilitation loop; they are never permission to bypass the fresh-capture gate — every Director re-engagement remains gate-preconditioned.

**What counts as stalled.** A member is stalled if it went idle without delivering expected output, without a meaningful progress update, or when a downstream task should have started but hasn't. Nudge a stalled member with a specific `cafleet message send` about what you expect next. Each workflow states its own wake sources — the turns on which you run this check — in its Director role file.

> **Bash request blocking case**: A member message asking for a shell command is dispatched per § *Team-facilitation instructions* → *Routing member bash requests*. The member blocks until the keystroke lands, so don't skip ahead to other inbox items while it waits.

### Stage 1 — Message-based check (`cafleet message poll`)

```bash
cafleet message poll --fleet-id <fleet-id> --member-id <director-member-id>
```

`cafleet message poll` returns only the un-acked (`input_required`) deliveries addressed to the Director, newest first. ACKing a delivery consumes it, so a later poll surfaces only what has arrived since the last ACK — there is no last-tick timestamp to track. If the member has sent a progress report or help request via `cafleet message send`, you can act on it immediately without interrupting the member's work. This is non-intrusive and preferred.

### Stage 2 — Terminal capture fallback (`cafleet monitor capture`)

```bash
cafleet monitor capture --fleet-id <fleet-id> \
  --member-id <member-id>
```

The `cafleet monitor capture` default is `--lines 20`; bump `--lines` to show more of a stalled member's buffer.

If `cafleet message poll` shows no recent messages from the member, fall back to capturing the terminal buffer. This is non-intrusive (read-only inspection that works even when the member is mid-task) and replaces raw `tmux capture-pane`.

A Stage-2 capture doubles as the gate capture only when it was taken at `--lines 120` and is still fresh (same facilitation turn, no intervening keystroke into the pane); the default `--lines 20` capture does not satisfy the gate.

**Deferred sends.** `cafleet message send` both persists a broker message and fires the inline-preview keystroke; there is no persist-without-keystroke mode. A round the gate skips (`awaiting_user` / `working`) therefore defers the **entire send**: hold each deferred send as queued work and re-evaluate it with a fresh capture on the next facilitation tick, then fire or skip again. No additional wake channel exists for deferrals — the next periodic wake re-opens your turn, so a deferral resolves within at most one wake interval once the pane clears.

> **The decision surface is a backend delta.** The concrete user-reaction surface by which the Director asks the user is backend-specific — see your overlay ([`coding-agent/<name>-overlay.md`](coding-agent/)). The canonical, backend-neutral user-reaction rule is [`SKILL.md`](../SKILL.md) § *Soliciting user reactions*.

### Escalation

If a member is still unresponsive after 2 **fired** re-engagement sends via `cafleet message send` AND `cafleet monitor capture` shows no forward progress in the terminal buffer, escalate to the user via {decision_surface} (per [`SKILL.md`](../SKILL.md) § *Soliciting user reactions*) with concrete options (e.g. re-send the instruction once more / re-spawn the member / drop its task). Only sends that actually fired count toward the threshold: a round the gate skipped never advances the count. A member that remains `awaiting_user` or `working` across many rounds is not "unresponsive" — it is parked on the user or making progress; keep skipping.

The unblock primitives and their ordering — non-intrusive `cafleet message poll` → read-only `cafleet monitor capture` → authoritative `cafleet message send` → `cafleet member ping` (missed auto-fire / required post-shell-dispatch follow-up) → `cafleet member prompt --shell "<cmd>"` (shell dispatch) → `cafleet member delete` (last resort, kills the pane immediately, never raw `tmux kill-pane`) → escalate to the user via {decision_surface} — are documented in [`reference/director.md`](director.md), [`reference/recovery.md`](recovery.md), [`reference/prompt-routing.md`](prompt-routing.md), and the § Quick Reference table below.

## User Delegation Protocol

CAFleet members never talk to the user directly — the Director relays. This is the relay-specific application of the canonical rule in [`SKILL.md`](../SKILL.md) § *Soliciting user reactions* (the question-shape taxonomy is in your overlay). When a member sends a `cafleet message send` asking for user input:

1. **Classify the question shape** per the question-shape taxonomy in your overlay (choice among labeled options, approve / yes-no, continue-or-abort, or open-ended / draft selection), and present it through {decision_surface}, mirroring the shape into options where the surface supports them. Follow your overlay for how the surface handles free-form text.
2. **Ask the user.** No preamble sentence above the question — the conversation context plus the question text carry it.
3. **Relay the answer back** via `cafleet message send` to the originating member. Pass through the user's selection verbatim; do not substitute your own judgment. If the user provided free-form text instead of a listed option, send that text.

A member that pauses on a decision-prompt pane frame awaiting a user reaction is the same delegation: put the decision to the user via {decision_surface}, then forward the answer with the decision-relay primitive your overlay describes, invoked through your own Bash tool. The concrete surface, the three-beat workflow, and the pane-shapes table are backend deltas — see your overlay; the neutral pointer is [`reference/director.md`](director.md) § *Answering a member's relayed question*.

### Free-form replies — judging intent

When the user supplies free-form text instead of a listed option, use LLM reasoning to determine intent — not keyword matching. Interpret the user's text to distinguish between:

- **Abort intent** (the user wants to stop or cancel the process)
- **Non-abort intent** (the user is providing verbal feedback or asking a question)

On **abort intent**, run the Abort Flow: tear down the team per § *Cleanup Protocol*, ending in `cafleet fleet delete --fleet-id <fleet-id>`, which soft-deletes the fleet and sweeps the root Director in one transaction.

On **non-abort intent**, explain that feedback belongs in `COMMENT(` markers at the workflow's own feedback target, then re-prompt with the same option pattern. Each workflow names that target in its Director role file.

**What you MUST NOT do:**

- Decide on the user's behalf, even when the answer looks obvious.
- Batch multiple members' questions into a single user prompt unless they are genuinely the same decision.
- Summarize or paraphrase the user's answer when relaying — pass it through.
- Print a fenced `bash` block of a pane command (`member prompt` / `member ping`) for the user to paste — invoke any such primitive via the Director's own Bash tool; the coding agent's per-call permission prompt is the consent surface.

## Cleanup Protocol

Cleanup follows [`reference/recovery.md`](recovery.md) § Shutdown Protocol, in order: stop the monitor loop's background task FIRST ({bg_stop}; the loop's signal handler runs its ownership-checked runtime clear, nulling the process fields and preserving `tick_seconds` and `last_wake_at`) → `cafleet member delete` each member (each kills the pane immediately) → `cafleet member list` verification that only the root Director's row remains → `cafleet fleet delete --fleet-id <fleet-id>` → `cafleet fleet list` sanity check.

A workflow that carries extra teardown — a precondition on when shutdown may begin, a roster-specific delete order, or non-CAFleet resources to release — runs those steps around this sequence and names them in its own Director role file.

## Quick Reference

| Action | Primitive | Notes |
|---|---|---|
| Verify Director pane env | `cafleet doctor` | Pre-spawn precondition; gating. Aborts the spawn protocol when no supported multiplexer (tmux or herdr) resolves. Replaces raw `tmux display-message` and `TMUX` env-var expansion. |
| Start the supervision tick | {bg_run} `cafleet monitor start --fleet-id <s>` in your own pane, immediately after `fleet create` | Confirm the `monitor loop started (…)` startup line in the task output before the first `member create`. |
| Spawn member | `cafleet member create --fleet-id <s> --name <n> --description <d> --text-file <abs path to ${BASE}/.prompts/<role>-<UTC-compact>.md>` | Pre-spawn file IS the audit artifact (see [`reference/director.md`](director.md) § *Member Create — Scratch and audit files*). Verify with `cafleet member list`. Inline `--text "<prompt>"` is still permitted for trivial one-line spawns. |
| Message member | `cafleet message send --fleet-id <s> --from-member-id <director> --to-member-id <member> --text "..."` | Broker keystrokes an inline preview into the member's pane. Gated: fresh capture must classify finished/stalled (§ Idle Semantics → *The pre-ping capture gate*; reply-soliciting replies exempt) |
| ACK reply | `cafleet message ack --fleet-id <s> --member-id <director> --message-id <message>` | Unacknowledged messages accumulate; ACK every reply you act on |
| Inspect stalled member | `cafleet monitor capture --fleet-id <s> --member-id <member>` | Replaces raw `tmux capture-pane` |
| Manual inbox-poll | `cafleet member ping --fleet-id <s> --member-id <member>` | Pre-approved; for missed auto-fires and post-`exec` chains. Gated: fresh capture must classify finished/stalled (§ Idle Semantics → *The pre-ping capture gate*) |
| Shell-dispatch on member's behalf | `cafleet member prompt --fleet-id <s> --member-id <member> --shell "<cmd>"` | Per [`reference/prompt-routing.md`](prompt-routing.md); follow with `member ping` |
| Answer a member's relayed question | {decision_surface} → `cafleet message send` | Ask the user via {decision_surface} first, then relay the answer back to the member as a message; never decide silently |
| Relay user input | {decision_surface} → `cafleet message send` | Pass-through; never substitute judgment |
| Shut down team | [`reference/recovery.md`](recovery.md) § Shutdown Protocol | Stop the monitor loop's background task first ({bg_stop}) → `member delete` each member → `fleet delete` |
