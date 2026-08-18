# CAFleet Team Supervision

Read this file for CAFleet team supervision — the Director-only governance and the `cafleet monitor` heartbeat mechanism it is performed through. Ordinary members and standalone agents never load it.

**Required reading — read AND resolve your overlay section first.** These instructions are backend-neutral and use `{placeholder}` tokens (`{decision_surface}`, `{permission_flags}`, `{monitor_model}`). Before acting on them, Read your overlay section [`coding-agent-overlays.md#<name>`](coding-agent-overlays.md) — `<name>` is the coding agent named on your spawn prompt's `CODING AGENT:` line — and **resolve** it per [`SKILL.md`](../SKILL.md) § *Resolve your overlay*.

## Core Principle

**You are the instruction giver. If you stop giving instructions, the entire team stops.**

CAFleet members spawned via `cafleet member create` do not act autonomously. They respond to your messages and to the broker's auto-fired pane keystrokes. If you are not actively dispatching work, ACKing replies, and running supervision ticks, the team halts silently.

## Communication Model

Supervision happens over the CAFleet message broker: the Director `cafleet message send`s a member → the broker keystrokes a 2-line inline preview into the member's pane (it processes the preview as a fresh user-turn; the full body is fetched via `cafleet message poll`) → the member acts and replies via `cafleet message send` → the broker keystrokes that reply into the Director's pane, which the Director ACKs (`cafleet message ack`). The inline-preview mechanics are canonical in [`SKILL.md`](../SKILL.md) § Send and [`multiplexer-backends.md`](../../../docs/docs/spec/multiplexer-backends.md#push-notifications).

**Long or multi-line bodies.** `message send` / `message broadcast` accept a `--file <path>` (or `--file -` for stdin) alternative to the inline positional `TEXT`. A long or multi-line body MUST be passed via `--file`, never inline, so it never lands on the command line and hits the shell's `ARG_MAX` limit. Short one-line bodies stay fine as the inline positional.

**Facilitation cue (load-bearing).** You are never nudged by a timer: your re-engagement channels are the broker auto-fire on every member `cafleet message send`, the monitor member's per-event messages, and the monitor's stalled-Director ping (fired only when you are confirmed quiet with un-acked deliveries — see § The monitor heartbeat). **Treat each of these — every inbound keystroke that re-opens your turn — as the cue to run the entire 5-step facilitation loop** (poll → ACK → dispatch → health-check → escalate), NOT to read the inbox and stop. Then honor the keystroke's closing clause where it carries one: resume your own work if something was still running when it landed.

Inspection is via `cafleet monitor scan` (the whole fleet at once) and `cafleet member capture` (one pane, deeper); write is via `cafleet member prompt` / `cafleet member ping` — never raw `tmux` ([`reference/recovery.md`](recovery.md) § *Shutdown Protocol*). See [`SKILL.md`](../SKILL.md) and [`reference/cli.md`](cli.md) for the canonical command surface.

The Director's plain output is **not visible to members** — the only Director→member channel is `cafleet message send` (and the Director-only keystroke primitives above for special cases).

## The monitor heartbeat

CAFleet members do not act autonomously. The team's periodic heartbeat is hosted by the fleet's dedicated **monitor member** — a cheap-model watcher spawned FIRST, by the `cafleet fleet create` bootstrap itself (§ *Spawn Protocol*). At startup the monitor member launches **`cafleet monitor`**, a per-fleet `scan → wake → sleep` loop, as a background task in its own pane (the loop-launch exclusivity rule is § *Spawn Protocol* → *Wait for the monitor gate*). Because the loop is just a backgrounded command, the heartbeat is **backend-agnostic** — a monitor member on `claude`, `codex`, or `opencode` gets the identical tick.

The wake is **unconditional and fleet-level**: once per wake interval (default **600 s**; `cafleet monitor <fleet-id> --interval N` / `CAFLEET_MONITOR_WAKE_INTERVAL`, `0` disables the wake while the loop keeps heartbeating) the loop keystrokes one `Esc`-first `[cafleet] tick:` payload into the **monitor member's own pane** — including when the fleet has no ordinary members yet. There is no per-member schedule and no per-member due computation.

On each wake the monitor member captures the fleet once (`cafleet monitor scan`), classifies each pane's content per the **target member's** backend overlay cues, confirms quiet across two consecutive wakes by capture sha, pings a confirmed-quiet ordinary member at most once per quiet period, pings **you** only when you are confirmed quiet AND your `unacked` count is greater than 0, and messages you per event (a member unchanged after its ping, a ping delivery failure, an `unknown` capture). Its full protocol is [`roles/monitor.md`](../roles/monitor.md) — the sole normative carrier; the wake payload points there and carries no protocol clauses itself.

See [`SKILL.md`](../SKILL.md) and the [Monitoring concepts page](https://himkt.github.io/cafleet/concepts/monitoring) for the full command surface and policy.

### How ordinary members are woken

The loop never keystrokes an ordinary member's pane. There are three paths:

1. **Primary** — the broker's inline-preview keystroke fired on every `cafleet message send` (`tmux.send_inline_preview`), landing the instant the Director or a teammate sends work.
2. **The monitor member's fixed ping** — one `cafleet member ping` per confirmed quiet period, per its role protocol ([`roles/monitor.md`](../roles/monitor.md)).
3. **Director recovery** — on a health check you may use `cafleet member ping` or send a new instruction, but every such re-engagement keystroke requires the target-specific fresh-capture gate below.

## Idle Semantics

**A member at rest between turns is normal, not a stall.** A member that finished its turn with no assigned work outstanding is doing exactly what it should — leave it. On each facilitation turn, capture and classify quiet members at your own discretion using the pre-ping capture gate below; the gate table's state → action rows govern what fires. The judgment the table cannot make:

- **You alone judge whether assigned work remains.** A `finished` member with outstanding assigned work is NOT left alone: dispatch the next step or re-engage it through the gate via `cafleet message send` / `cafleet member ping`. A `finished` member with nothing outstanding is at expected rest — the broker's inline preview wakes it when you have new work (each such send still routes through the gate).
- **Quiet member unchanged across two consecutive facilitation turns → re-engage it.** `stall_candidate` and `finished` are both quiet observations: when your fresh capture on this turn is byte-identical to the capture you took on the previous one, the member is confirmed quiet — fire `cafleet member ping` (or a specific `cafleet message send`) through the gate; your own conversation notes are the baseline between turns.
- **Pending deliveries and monitor events are context, not proof.** A member's un-acked delivery count and the monitor member's event messages annotate your health check and never by themselves authorize a ping.
- An immediate reply to a **reply-soliciting** message (a question or blocker) received from that member in the current facilitation turn is exempt from the gate: the member ended its turn to await this reply, so its pane is at rest with no live prompt — reply via `cafleet message send`. A reply to a progress-only status message ("still working", "ack") is NOT exempt — the member may still be mid-turn — and routes through the gate.

Idleness alone is never a stop signal (§ Authorization-Scope Guard below).

### The pre-ping capture gate

Every **Director-initiated** re-engagement keystroke at a member — `cafleet
member ping`, a non-exempt `cafleet message send`, and `cafleet message
broadcast` — is capture-gated immediately before firing. Classify from
content only using the **target member's** backend
overlay; mixed fleets make this target-specific. The normative gate capture
is the batch scan:

```bash
cafleet monitor scan <fleet-id>
```

One fresh scan at the default depth (20 lines per pane) satisfies the gate
for **every** member for that facilitation turn. A fresh single-member
`cafleet member capture` at default depth or deeper satisfies the gate for
that one member. Per-target freshness: a capture is *fresh* only within the
same facilitation turn and with no intervening keystroke into that pane —
once you keystroke a pane (`cafleet member ping`, a non-exempt `cafleet
message send`, a `cafleet member prompt`), its snapshot is stale, and a
further re-engagement of the same member needs a fresh capture (a
single-member `cafleet member capture` or a new scan).

| Capture classifies | Director action |
|---|---|
| `finished` | Fire the ping/send. |
| `stalled` (quiet, unchanged, no prompt, no in-flight work) | Fire the ping/send. |
| `awaiting_user` | **Skip this round.** Defer the entire send (nothing persisted, nothing keystroked). Do not relay the pane's prompt anywhere — the round is simply skipped. |
| `working` | **Skip this round.** Defer the entire send. The member surfaces its own result via `cafleet message send` when done. |
| `unknown` (dead / unreadable pane) | Do not ping. Enter the recovery path ([`reference/recovery.md`](recovery.md)) / § Stall Response → Escalation instead. |

The ambiguity tie-break: a capture that cannot distinguish `awaiting_user` from `finished` classifies `awaiting_user`. When in doubt between `stalled` and `working`, treat as `working` (skip the round) — a deferred ping costs one round; an `Esc` into an in-flight turn destroys work. For the gate, `stalled` means the capture shows a quiet pane with no pending prompt and no in-flight work, in a context where your own prior capture showed the same content; your conversation notes across facilitation turns are the baseline.

A `cafleet message broadcast` fires the same `Esc`-first preview into every recipient pane, and recipients cannot be skipped individually within one send — so the broadcast fires only when **every** recipient's fresh capture classifies `finished` or `stalled`; otherwise defer the entire broadcast, or replace it with per-recipient gated unicasts.

**Exempt from the gate:** the immediate reply to a reply-soliciting message (bullet above), and `cafleet member prompt --shell` — member-requested shell dispatch per [`reference/prompt-routing.md`](prompt-routing.md), where the member is blocked *expecting* the keystroke.

## Authorization-Scope Guard (CRITICAL)

**User authorization persists** across broker auto-fires, monitor pings and event messages, and teammate idle notifications until an explicit stop signal arrives — absence of confirmation is not a stop signal. The Director MUST dispatch queued work as soon as a teammate is idle and the inputs the work depends on are available; do NOT emit passive-hold messages in response to a supervision tick.

### Real stop signals (treat as halt; everything else is a tick to evaluate)

| Signal | Director response |
|---|---|
| User typed an explicit "stop" / "wait" / "pause" | Halt dispatch; wait for explicit re-authorization. |
| User typed profanity / frustration / a negative reaction | Halt dispatch; wait. Monitor pings and event messages during this state are read but not acted on. |
| User rejected your last 2+ tool calls | Halt dispatch; treat the rejections as a halt signal even if no profanity arrived. |
| User typed `/clear` or restarted the session | Authorization is gone; do not resume from prior context without a fresh instruction. |
| Member's reply contains a clear blocker; wait for guidance | Pause that one task only; continue dispatching to the rest of the team. |

Monitor pings and event messages, teammate idle notifications, broker auto-fire receipts, and the absence of a fresh "go" message are **not** stop signals. Treat them as inputs to evaluate, not gates to pass through.

### When you genuinely need user input

If a queued action requires a *new* decision the user has not yet made (choosing between options, approving a risky / remote-visible operation, disambiguating a teammate's question), use {decision_surface} (the canonical user-reaction rule is [`SKILL.md`](../SKILL.md) § *Soliciting user reactions*) — do **not** emit a passive hold and wait. The hold message produces nothing; the question unblocks you within seconds and produces a recorded answer.

## Spawn Protocol

**Fleet bootstrap (monitor included).** After the `cafleet doctor` env check, write the monitor member's spawn prompt to `${BASE}/.prompts/monitor-<UTC-compact>.md` (the standard pre-spawn audit convention; when `${BASE}` is `<unset>`, pass the prompt on stdin via `--monitor-file -` instead), then run `cafleet fleet create --name <n> --coding-agent <backend> --monitor-file <abs path> --monitor-model {monitor_model} --json`. One command atomically creates the fleet, registers the root Director bound to the current pane, registers the monitor member, and spawns the monitor's pane — any failure rolls everything back (no rows, no pane) and the command is retryable as-is. For `<backend>`, substitute the coding agent you are actually running on — a spawned agent's `CODING AGENT:` line names it; a standalone Director uses its own identity (e.g. Claude Code → `claude`); the monitor inherits it by construction. `{monitor_model}` is your overlay's value, mirroring the model list's *Monitor and reviewer defaults* table. Capture `fleet_id`, `director.member_id`, and `monitor.member_id` from the JSON response and carry them as literal integers on every later call; the literal-id rule and the positional-subject placement are canonical in [`SKILL.md`](../SKILL.md) § *Required ids*.

**Wait for the monitor gate.** At startup the monitor member sends `ready`, launches `cafleet monitor <fleet-id>` in its own pane, confirms the loop's startup line, and sends the gate signal **`monitor live`**. Wait for `ready`, then `monitor live`: that message gates your first ordinary `member create` (belt); the CLI's monitor-first guard backstops a Director that skips the wait (suspenders). The monitor member owns the loop launch and the startup-line confirmation ([`roles/monitor.md`](../roles/monitor.md)) — you do NOT launch the loop and do NOT confirm the startup line yourself. A monitor that instead reports a failed start (runtime-claim conflict, dead fleet) is resolved before spawning anyone. `cafleet member create --role monitor` is the mid-run recovery path for re-spawning a dead monitor — never the bootstrap path.

Every time you spawn a member:

1. **Verify env, then ensure supervision is running**:
   - **Pre-spawn env-check (gating)**: run `cafleet doctor`. It renders the three-section diagnosis (multiplexer, database, coding agents) and exits non-zero on **any** rendered issue — a multiplexer failure, a database-schema issue, or a stale/invalid coding-agent state; the not-installed state never counts. If it exits non-zero, ABORT the spawn protocol and surface the report — the gate deliberately catches, pre-spawn, what the stale-assets guard would reject at `member create` anyway, plus a behind-head schema. `cafleet doctor` is the canonical pane-identity probe, and `member create` owns the backend-binary `PATH` check (see [`cli-options.md`](../../../docs/docs/spec/cli-options.md#member-create)) — never a raw `tmux` / env probe, never a `<backend> --version` / `which` pre-probe.
   - **Monitor member live before any ordinary member** — `monitor live` received per *Wait for the monitor gate* (§ above); a monitor member that has since died is re-spawned with `--role monitor` before spawning anyone else.
2. **Spawn the member** via `cafleet member create --fleet-id <fleet-id> --name <name> --description <desc> --file <abs path to ${BASE}/.prompts/<role>-<UTC-compact>.md>` (the Director is auto-resolved from the fleet row). The pre-spawn file IS both the CLI input and the permanent audit artifact; the audit-file convention (with the `${BASE} == <unset>` guarded-skip + inline fallback), the `--model` flag, and the model-name→backend inference are canonical in [`reference/director.md`](director.md) § Member Create.
3. **Include the ready-signal directive in the spawn prompt.** Every spawn prompt MUST instruct the member, as its first Bash call, to send `cafleet message send … "ready"` (canonical wording in [`roles/member.md`](../roles/member.md) § *On spawn — send the ready signal*). It is the ONLY signal that the coding agent inside the pane has actually booted; a prompt missing it is a defect — fix and re-spawn.
4. **Verify the member is placed** by checking that `cafleet member list <fleet-id>` shows the new member with a non-null `pane_id`. This confirms the pane was created. Liveness of the coding agent inside the pane is confirmed asynchronously when the ready signal arrives — NOT by `member list`.
5. **End the active turn after spawn-and-verify.** The ready signal arrives via the re-engagement channels (§ Communication Model → *Facilitation cue*); you process it — ACK, dispatch first task — in your next active turn. See § *Asynchronous Wait Rule* below.

**Dispatch-on-ready.** When a member's ready signal arrives, ACK it and dispatch that member's first task in the same turn, provided the task's inputs exist. First-task dispatch is per-member: never hold a ready member's dispatch waiting for other members' ready signals or placements. A member whose first task genuinely depends on an input that does not yet exist (e.g. a deliverable another member has not produced) legitimately stays idle until that input lands — dispatch whatever is dispatchable, to whoever is ready.

Never spawn an ordinary member before the `monitor live` gate. Keep the monitor member alive until all work is fully complete and the team is being shut down.

### Asynchronous Wait Rule

The active turn consumes inputs that have already arrived and dispatches what is ready — then returns control. Waiting for things that have not yet arrived is the job of the re-engagement channels (§ Communication Model → *Facilitation cue*).

| Situation | Director action |
|---|---|
| Just spawned a member; ready signal not yet arrived | End the turn. Auto-fire delivers the ready signal as it lands; the monitor member is the backstop. When it lands, ACK and dispatch that member's first task in the same turn (§ Spawn Protocol → *Dispatch-on-ready*). |
| Just dispatched to a member; reply not yet arrived | End the turn. Same wake-up channels surface the reply. |
| Waiting on multiple members' replies before next step | End the turn. React to each arrival as its own wake-up, not all-at-once — never hold one member's dispatch waiting for another's arrival (§ Spawn Protocol → *Dispatch-on-ready*). |
| User asks "what's the status?" while members are working | Report the asynchronous truth (e.g. "Alice is processing X; her completion will surface in my next turn"). For a live snapshot, use `cafleet member capture`. |
| Turn finished dispatching and ACKing | End the turn. The next wake-up reopens the turn when there is something to act on. |

## Team-facilitation instructions

On every supervision tick — whether fired by inbound work arriving via the broker's inline-preview keystroke, by a monitor event message or stalled-Director ping, or executed inline within an active turn — the Director runs these five steps in order. The goal is to **facilitate the team in completing tasks**, not merely to detect stalls.

1. **Poll inbox.** `cafleet message poll <director-member-id>` returns only the un-acked (`input_required`) deliveries; ACKing each one (step 2) consumes it — the poll semantics are canonical at § Stall Response → Stage 1.
2. **ACK every message** that requires no further action: `cafleet message ack <message-id>`. Unacknowledged messages accumulate in the Director's inbox and obscure new arrivals.
3. **Dispatch queued work.** If a member is idle and inputs are available (a freshly-arrived ready signal whose first task's inputs exist, review comments to route, the next implementation step in a design doc, reviewer feedback waiting at the Drafter, a teammate reply waiting to be acted on), send the instruction immediately via `cafleet message send` — per-member, never held for other members' arrivals (§ Spawn Protocol → *Dispatch-on-ready*). **Do not wait for a fresh "go" from the user** — the user's original authorization persists across ticks; see § Authorization-Scope Guard.
4. **Run the health-check sequence** for any member that has not reported recent progress — cheapest, least-intrusive check first: (a) `cafleet member list` (enumerate members + pane status); (b) `cafleet message poll` (progress reports / help requests); (c) the facilitation turn's fresh `cafleet monitor scan <fleet-id>` — its section for the member, or a targeted `cafleet member capture` for deeper investigation — classified per § Idle Semantics → *The pre-ping capture gate* (a decision-prompt frame → see Stall Response for the decision-relay escape hatch); (d) `cafleet message send` a specific instruction — (c) is the gating precondition: (d) fires only for a member whose (c) capture classified `finished` or `stalled`; on `awaiting_user` or `working`, skip the round and defer the send; (e) once all members report completion, tell the user "All deliverables are ready for review."
5. **Escalate** to the user via {decision_surface} whenever a queued action requires a *new* user decision (option choice, risky/remote-visible operation, ambiguous teammate question); for the stall path (two fired sends with no progress) see § Stall Response → Escalation. The tick is a health check, not a permission renewal (§ Authorization-Scope Guard).

After the five steps, honor the resume clause of whatever keystroke re-opened your turn: if it landed while your own task was mid-flight, pick that task back up before ending the turn.

### Routing member bash requests

The workflow's spawned members run in workspace-scoped auto-approval mode ({permission_flags}; Bash tool enabled, permission prompts auto-resolve), so they run shell commands directly by default. When a member's harness denies a command (per-backend denial semantics canonical in [`reference/prompt-routing.md`](prompt-routing.md)), it auto-routes a plain shell-command request via `cafleet message send`, and you respond via `cafleet member prompt --shell`. Process such requests one at a time in poll order.

## Monitor Lifecycle

| Phase | Action |
|---|---|
| Spawn (before any ordinary member) | The `cafleet fleet create` bootstrap spawns the monitor member; wait for `ready` then `monitor live` per § *Spawn Protocol* → *Wait for the monitor gate* — that message (plus the CLI monitor-first guard) gates the first ordinary `cafleet member create`. Re-spawn a dead monitor with `member create --role monitor`. |
| Run work | The `Esc`-first wake lands in the **monitor member's** pane per wake interval (default 600 s); the monitor scans, classifies, pings a confirmed-quiet member once per quiet period, and messages you per event. Each inbound auto-fire, monitor event, or monitor ping is the cue to run the 5-step facilitation loop above. |
| User review | Keep the monitor member alive during the review cycle — revisions and re-reviews still count as in-progress work. |
| Teardown | Delete the monitor member FIRST (first-out); the full ordering is § *Cleanup Protocol*. |

**Lifecycle rule (non-negotiable):** The monitor member MUST stay alive from before the first ordinary `member create` through every phase, until the teardown above deletes it first-out.

## Stall Response

The monitor member's event messages and your own captures across facilitation turns are the evidence for the facilitation loop; they are never permission to bypass the fresh-capture gate — every Director re-engagement remains gate-preconditioned.

**What counts as stalled.** A member is stalled if it went idle without delivering expected output, without a meaningful progress update, or when a downstream task should have started but hasn't. Nudge a stalled member with a specific `cafleet message send` about what you expect next. Each workflow states its own wake sources — the turns on which you run this check — in its Director role file.

> **Bash request blocking case**: A member message asking for a shell command is dispatched per § *Team-facilitation instructions* → *Routing member bash requests*. The member blocks until the keystroke lands, so don't skip ahead to other inbox items while it waits.

### Stage 1 — Message-based check (`cafleet message poll`)

```bash
cafleet message poll <director-member-id>
```

`cafleet message poll` returns only the un-acked (`input_required`) deliveries addressed to the Director, newest first. ACKing a delivery consumes it, so a later poll surfaces only what has arrived since the last ACK — there is no last-tick timestamp to track. If the member has sent a progress report or help request via `cafleet message send`, you can act on it immediately without interrupting the member's work. This is non-intrusive and preferred.

### Stage 2 — Terminal capture fallback (`cafleet member capture`)

```bash
cafleet member capture <member-id>
```

The `cafleet member capture` default is `--lines 20`; bump `--lines` to show more of a stalled member's buffer.

If `cafleet message poll` shows no recent messages from the member, fall back to capturing the terminal buffer. This is non-intrusive (read-only inspection that works even when the member is mid-task) and replaces raw `tmux capture-pane`.

A Stage-2 `member capture` doubles as the gate capture for that member while still fresh (per the gate's freshness rule).

**Deferred sends.** `cafleet message send` both persists a broker message and fires the inline-preview keystroke; there is no persist-without-keystroke mode. A round the gate skips (`awaiting_user` / `working`) therefore defers the **entire send**: hold each deferred send as queued work and re-evaluate it with a fresh capture on the next facilitation tick, then fire or skip again. No additional wake channel exists for deferrals — the next keystroke that re-opens your turn is when a deferral re-evaluates; a deferred target that stays quiet also surfaces through the monitor's own fixed ping and unchanged-after-ping event.

> **The decision surface is a backend delta.** The concrete user-reaction surface by which the Director asks the user is backend-specific — see your overlay section ([`coding-agent-overlays.md#<name>`](coding-agent-overlays.md)). The canonical, backend-neutral user-reaction rule is [`SKILL.md`](../SKILL.md) § *Soliciting user reactions*.

### Escalation

If a member is still unresponsive after 2 **fired** re-engagement sends via `cafleet message send` AND `cafleet member capture` shows no forward progress in the terminal buffer, escalate to the user via {decision_surface} (per [`SKILL.md`](../SKILL.md) § *Soliciting user reactions*) with concrete options (e.g. re-send the instruction once more / re-spawn the member / drop its task). Only sends that actually fired count toward the threshold: a round the gate skipped never advances the count. A member that remains `awaiting_user` or `working` across many rounds is not "unresponsive" — it is parked on the user or making progress; keep skipping.

The unblock primitives and their ordering — non-intrusive `cafleet message poll` → read-only `cafleet member capture` → authoritative `cafleet message send` → `cafleet member ping` (missed auto-fire / required post-shell-dispatch follow-up) → `cafleet member prompt --shell "<cmd>"` (shell dispatch) → `cafleet member delete` (last resort, kills the pane immediately) → escalate to the user via {decision_surface} — are documented in [`reference/director.md`](director.md), [`reference/recovery.md`](recovery.md), [`reference/prompt-routing.md`](prompt-routing.md), and the § Quick Reference table below.

## User Delegation Protocol

CAFleet members never talk to the user directly — the Director relays. This is the relay-specific application of the canonical rule in [`SKILL.md`](../SKILL.md) § *Soliciting user reactions* (the question-shape taxonomy is in your overlay). When a member sends a `cafleet message send` asking for user input:

1. **Classify the question shape** per the question-shape taxonomy in your overlay (choice among labeled options, approve / yes-no, continue-or-abort, or open-ended / draft selection), and present it through {decision_surface}, mirroring the shape into options where the surface supports them. Follow your overlay for how the surface handles free-form text.
2. **Ask the user.** No preamble sentence above the question — the conversation context plus the question text carry it. One prompt per decision: batch multiple members' questions only when they are genuinely the same decision.
3. **Relay the answer back** via `cafleet message send` to the originating member. Pass through the user's selection verbatim; do not substitute your own judgment. If the user provided free-form text instead of a listed option, send that text.

A member that pauses on a decision-prompt pane frame awaiting a user reaction is the same delegation: put the decision to the user via {decision_surface}, then forward the answer with the decision-relay primitive your overlay describes, invoked through your own Bash tool — never printed as a fenced `bash` block for the user to paste; the coding agent's per-call permission prompt is the consent surface. The concrete surface, the three-beat workflow, and the pane-shapes table are backend deltas — see your overlay; the neutral pointer is [`reference/director.md`](director.md) § *Answering a member's relayed question*.

### Free-form replies — judging intent

When the user supplies free-form text instead of a listed option, use LLM reasoning to determine intent — not keyword matching. Interpret the user's text to distinguish between:

- **Abort intent** (the user wants to stop or cancel the process)
- **Non-abort intent** (the user is providing verbal feedback or asking a question)

On **abort intent**, run the Abort Flow: tear down the team per § *Cleanup Protocol*, ending in `cafleet fleet delete <fleet-id>`, which soft-deletes the fleet and sweeps the root Director in one transaction.

On **non-abort intent**, explain that feedback belongs in `COMMENT(` markers at the workflow's own feedback target, then re-prompt with the same option pattern. Each workflow names that target in its Director role file.

## Cleanup Protocol

Cleanup follows [`reference/recovery.md`](recovery.md) § Shutdown Protocol: the **monitor member is deleted FIRST** (first-out — the pane kill ends the wake source), then each remaining member, then the verification, `cafleet fleet delete`, and the final sanity check per that protocol.

A workflow that carries extra teardown — a precondition on when shutdown may begin, a roster-specific delete order, or non-CAFleet resources to release — runs those steps around this sequence and names them in its own Director role file.

## Quick Reference

| Action | Primitive | Notes |
|---|---|---|
| Verify Director pane env | `cafleet doctor` | Pre-spawn precondition; gating. Aborts the spawn protocol on any rendered issue — a multiplexer failure, a database-schema issue, or a stale/invalid coding-agent state (the not-installed state never counts). Replaces raw `tmux display-message` and `TMUX` env-var expansion. |
| Bootstrap fleet + monitor member | `cafleet fleet create --name <n> --coding-agent <backend> --monitor-file <abs path to ${BASE}/.prompts/monitor-<UTC-compact>.md> --monitor-model {monitor_model} --json` | One atomic command: fleet + root Director + monitor member (registration and pane); any failure rolls everything back. Wait for `ready` then `monitor live` before the first ordinary `member create`. |
| Re-spawn a dead monitor member | `cafleet member create --fleet-id <s> --role monitor --model {monitor_model} --name monitor --description <d> --file <abs path to ${BASE}/.prompts/monitor-<UTC-compact>.md>` | Mid-run recovery only; omit `--coding-agent`. Wait for `ready` then `monitor live` before re-engaging the team. |
| Fleet-wide pane snapshot | `cafleet monitor scan <s>` | One fresh scan per facilitation turn satisfies the pre-ping capture gate for every member (§ Idle Semantics → *The pre-ping capture gate*). |
| Spawn member | `cafleet member create --fleet-id <s> --name <n> --description <d> --file <abs path to ${BASE}/.prompts/<role>-<UTC-compact>.md>` | Pre-spawn file IS the audit artifact (see [`reference/director.md`](director.md) § *Member Create — Scratch and audit files*). Verify with `cafleet member list`. An inline positional `"<prompt>"` is still permitted for trivial one-line spawns. |
| Message member | `cafleet message send --from-member-id <director> --to-member-id <member> "..."` | Broker keystrokes an inline preview into the member's pane. Gated: fresh capture must classify finished/stalled (§ Idle Semantics → *The pre-ping capture gate*; reply-soliciting replies exempt) |
| ACK reply | `cafleet message ack <message>` | Unacknowledged messages accumulate; ACK every reply you act on |
| Inspect stalled member | `cafleet member capture <member>` | Targeted deeper investigation of a single pane; replaces raw `tmux capture-pane` |
| Manual inbox-poll | `cafleet member ping <member>` | Pre-approved; for missed auto-fires and post-`exec` chains. Gated: fresh capture must classify finished/stalled (§ Idle Semantics → *The pre-ping capture gate*) |
| Shell-dispatch on member's behalf | `cafleet member prompt <member> --shell "<cmd>"` | Per [`reference/prompt-routing.md`](prompt-routing.md); follow with `member ping` |
| Answer a member's relayed question | {decision_surface} → `cafleet message send` | Ask the user via {decision_surface} first, then relay the answer back to the member as a message; never decide silently |
| Relay user input | {decision_surface} → `cafleet message send` | Pass-through; never substitute judgment |
| Shut down team | [`reference/recovery.md`](recovery.md) § Shutdown Protocol | Delete the monitor member first (first-out) → `member delete` each remaining member → `fleet delete` |
