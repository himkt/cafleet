# CAFleet Team Supervision

Read this file for CAFleet team supervision — the Director-only governance and the `cafleet monitor` heartbeat mechanism it is performed through. Ordinary members and standalone agents never load it.

**Required reading — read AND resolve your overlay section first.** These instructions are backend-neutral and use `{placeholder}` tokens (`{decision_surface}`, `{permission_flags}`, `{monitor_model}`). Before acting on them, Read your overlay section [`coding-agents.md#<name>`](coding-agents.md) — `<name>` is the coding agent named on your spawn prompt's `CODING AGENT:` line — and **resolve** your runtime bindings and the inherited monitor backend's Role defaults per [`SKILL.md`](../SKILL.md) § *Resolve your overlay*.

## Core Principle

**You are the instruction giver. If you stop giving instructions, the entire team stops.**

CAFleet members spawned via `cafleet member create` do not act autonomously. They respond to your messages and to the broker's auto-fired pane keystrokes. If you are not actively dispatching work, ACKing replies, and running supervision ticks, the team halts silently.

## Communication Model

Supervision happens over the CAFleet message broker: the Director `cafleet message send`s a member → the broker keystrokes a 2-line inline preview into the member's pane once that pane is at rest (the member processes the preview as a fresh user-turn; the full body is fetched via `cafleet message poll`) → the member acts and replies via `cafleet message send` → the broker keystrokes that reply into the Director's pane, which the Director ACKs (`cafleet message ack`). The inline-preview mechanics are canonical in [`SKILL.md`](../SKILL.md) § Send and [`multiplexer-backends.md`](runtime/spec/multiplexer-backends.md#push-notifications).

**Long or multi-line bodies.** `message send` / `message broadcast` accept a `--file <path>` (or `--file -` for stdin) alternative to the inline positional `TEXT`. A long or multi-line body MUST be passed via `--file`, never inline, so it never lands on the command line and hits the shell's `ARG_MAX` limit. Short one-line bodies stay fine as the inline positional.

**Facilitation cue (load-bearing).** You are never nudged by a timer: your re-engagement channels are the broker's inline preview of every member `cafleet message send`, the broker's own notices (§ *Broker-held delivery*), the monitor member's per-event messages, and the monitor's stalled-Director ping (fired only when you are confirmed quiet with un-acked deliveries — see § The monitor heartbeat). **Treat each of these — every inbound keystroke that re-opens your turn — as the cue to run the entire 5-step facilitation loop** (poll → ACK → dispatch → health-check → escalate), NOT to read the inbox and stop. Then honor the keystroke's closing clause where it carries one: resume your own work if something was still running when it landed.

Inspect through `cafleet monitor scan` (the fleet) and `cafleet member capture` (one pane, deeper); use role-authorized `member exec` / `member prompt` / `member ping` for pane writes. CAFleet primitives own multiplexer interactions, including [Shutdown](#shutdown). The [core command index](../SKILL.md#command-index) links exact command contracts.

The Director's plain output is **not visible to members** — the only Director→member channel is `cafleet message send` (and the Director-only keystroke primitives above for special cases).

## The monitor heartbeat

CAFleet members do not act autonomously. The team's periodic heartbeat is **`cafleet monitor`**, a per-fleet `deliver → wake → sleep` loop. `cafleet fleet create` starts it as a detached process and returns only once it is live, so the loop depends on no member's shell tool and you never launch or poll it. It wakes the fleet's dedicated **monitor member** — a cheap-model watcher spawned FIRST, by the same bootstrap (§ *Spawn Protocol*).

The wake is **unconditional and fleet-level**: once per wake interval (default **600 s**; `0` disables the wake while the loop keeps heartbeating and delivering) the loop keystrokes one `Esc`-first `[cafleet] tick:` payload into the **monitor member's own pane** — including when the fleet has no ordinary members yet. There is no per-member schedule and no per-member due computation.

On each wake the monitor member captures the fleet once (`cafleet monitor scan`), classifies each pane's content per the **target member's** backend overlay cues, confirms quiet across two consecutive wakes by capture sha, pings a confirmed-quiet ordinary member at most once per quiet period, pings **you** only when you are confirmed quiet AND your `unacked` count is greater than 0, and messages you per event (a member unchanged after its ping, a ping delivery failure, an `unknown` capture, one of its own commands being denied). Its full protocol is [`roles/monitor.md`](../roles/monitor.md) — the sole normative carrier; the wake payload points there and carries no protocol clauses itself.

See [`SKILL.md`](../SKILL.md) and the [Monitoring concepts page](https://himkt.github.io/cafleet/concepts/monitoring) for the full command surface and policy.

### Broker-held delivery

The broker sends a keystroke only into a pane that is at rest. A message for a member that is mid-turn, or that shows a permission prompt, is **held**: the row is persisted at once, the pane is left alone, and the monitor loop delivers the preview within one tick of the pane coming to rest. The same rule covers your own pane, so a member's message never dismisses a prompt you are answering.

This is why you send without checking the target first:

- **Send when you have something to send.** `cafleet message send` and `cafleet message broadcast` need no capture beforehand. A held send exits 0; its output shows `notification_sent: false` (unicast, `--json`) or a `delivered` count below `recipients` (broadcast).
- **A hold is bounded.** After the hold timeout (`CAFLEET_DELIVERY_HOLD_TIMEOUT`, default 300 s) the broker forces the delivery, with a line telling the recipient what it interrupted. A forced preview that names a dismissed prompt means the rejection came from the broker, not from the user: re-issue the tool call.
- **`cafleet member ping` is gated by the broker too.** It keystrokes a pane at rest or a quiet pane it cannot classify, and skips a busy one with exit 0 and a reason.
- **`cafleet member prompt` is your deliberate direct turn** and is sent at once.

The broker also writes **notices** into your inbox — a finished or lost `member exec`, and a spawned member that sent no message within 180 s. Each is an ordinary message starting `[cafleet] `; the actions are in [`roles/director.md`](../roles/director.md#broker-notices).

A `cafleet message send` or `broadcast` that exits 1 with `… was persisted, but monitor loop for fleet <id> did not start; see <log path>` has committed the message. Send the body once only: read the named log, run `cafleet doctor`, and the next command that leaves work owed starts the loop again.

### How ordinary members are woken

1. **Primary** — the broker's inline preview of a `cafleet message send`: at once into a pane at rest, otherwise by the loop when the pane comes to rest.
2. **The monitor member's fixed ping** — one `cafleet member ping` per confirmed quiet period, per its role protocol ([`roles/monitor.md`](../roles/monitor.md)).
3. **Director recovery** — on a health check you may use `cafleet member ping` or send a new instruction (§ *Stall Response*).

## Idle Semantics

**A member at rest between turns is normal, not a stall.** A member that finished its turn with no assigned work outstanding is doing exactly what it should — leave it. On a health check, capture and classify a quiet member with the **target member's** backend overlay cues:

- **You alone judge whether assigned work remains.** A `finished` member with outstanding assigned work is NOT left alone: dispatch the next step via `cafleet message send`. A `finished` member with nothing outstanding is at expected rest — the broker's inline preview wakes it when you have new work.
- **Confirm a stall candidate across two consecutive facilitation turns.** `stall_candidate` and `finished` are both quiet observations: when your capture on this turn is byte-identical to the capture you took on the previous one, the member is confirmed quiet. Re-engage a confirmed stall candidate with a specific `cafleet message send` or a `cafleet member ping`; a finished member with no outstanding assignment remains at expected rest. Your own conversation notes are the baseline between turns. The monitor member's separate bounded-ping policy remains in its role file.
- **`working` and `awaiting_user` need nothing from you.** A working member surfaces its own result when done, and a message you send it meanwhile is held and delivered afterwards. A prompt seen only in a capture is the member's to resolve; relay only a question the member sent you.
- **Pending deliveries and monitor events are context, not proof.** A member's un-acked delivery count and the monitor member's event messages annotate your health check; they do not by themselves establish a stall.
- **An `unknown` capture (dead or unreadable pane) calls for [Recovery](#recovery)**, not a ping.

Idleness alone is never a stop signal (§ Authorization-Scope Guard below).

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

**Fleet bootstrap (monitor included).** After the `cafleet doctor` env check, write the monitor member's spawn prompt to `${BASE}/.prompts/monitor-<UTC-compact>.md` (the standard pre-spawn audit convention; when `${BASE}` is `<unset>`, pass the prompt on stdin via `--monitor-file -` instead), then run `cafleet fleet create --name <n> --coding-agent <backend> --monitor-file <abs path> --monitor-model {monitor_model} --monitor-effort {monitor_effort} --json` for Claude or Codex. Omit `--monitor-effort` for OpenCode. One command creates the fleet, root Director bound to the current pane, and monitor member in a DB transaction, owns the spawned monitor pane until success, and then starts the fleet's monitor loop as a detached process. It returns only after the loop is live and reports the loop's pid (`monitor_loop.pid` in JSON). On failure it attempts DB/pane compensation; inspect any cleanup-failure or unknown-pane diagnostic before retrying, since those outcomes do not confirm a complete rollback. When the loop does not start, the command kills the monitor pane, soft-deletes the fleet, prints no ids, and exits 1 naming a log file: read that log, fix the cause, and run `fleet create` again. For `<backend>`, substitute the coding agent you are actually running on — a spawned agent's `CODING AGENT:` line names it; a standalone Director uses its own identity (e.g. Claude Code → `claude`); the monitor inherits it by construction. The monitor's model and effort come from that backend's Role defaults; recovery passes the same values as `--model` and `--effort` with backend inheritance. Capture `fleet_id`, `director.member_id`, and `monitor.member_id` from the JSON response and carry those literal integers on every later call; the literal-id rule and the positional-subject placement are canonical in [`SKILL.md`](../SKILL.md) § *Required ids*.

**Reuse a running fleet.** If you already have a running fleet (e.g. an outer orchestration), reuse its `fleet_id` and its root Director's `member_id` instead of creating a new fleet — the root Director from `fleet create` is the team lead.

**Wait for the monitor's ready.** At startup the monitor member sends `ready` and ends its turn. That `ready` gates your first ordinary `member create` (belt); the CLI's monitor-first guard backstops a Director that skips the wait (suspenders). The loop is already running when `fleet create` returns, so the monitor member has nothing to launch ([`roles/monitor.md`](../roles/monitor.md)). `cafleet member create --role monitor` is the mid-run recovery path for re-spawning a dead monitor — the bootstrap path is always `fleet create`.

Every time you spawn a member:

1. **Verify env, then ensure supervision is running**:
   - **Pre-spawn env-check (gating)**: run `cafleet doctor`. It renders the four-section diagnosis (multiplexer, database, coding agents, member permissions) and exits non-zero on **any** rendered issue — a multiplexer failure, a database-schema issue, a stale/invalid coding-agent state, or a Claude Code `deny` / `ask` rule that would block a member's broker commands; the not-installed state never counts. If it exits non-zero, ABORT the spawn protocol and surface the report — the gate deliberately catches, pre-spawn, what the stale-assets guard would reject at `member create` anyway, plus a behind-head schema and a member that could not reach you. `cafleet doctor` is the canonical pane-identity probe, and `member create` owns the backend-binary `PATH` check (see [`cli-options.md`](runtime/spec/cli-options.md#member-create)) — never a raw `tmux` / env probe, never a `<backend> --version` / `which` pre-probe.
   - **Monitor member ready before any ordinary member** — its `ready` received per *Wait for the monitor's ready* (§ above); a monitor member that has since died is re-spawned with `--role monitor` before spawning anyone else.
2. **Spawn the member** via `cafleet member create --fleet-id <fleet-id> --name <name> --description <desc> --file <abs path to ${BASE}/.prompts/<role>-<UTC-compact>.md>` (the Director is auto-resolved from the fleet row). The pre-spawn file IS both the CLI input and the permanent audit artifact; the audit-file convention (with the `${BASE} == <unset>` guarded-skip + inline fallback), the `--model` flag, and the model-name→backend inference are canonical in [`roles/director.md`](../roles/director.md) § Member Create.
3. **Carry the skeleton's ready-signal line.** Every spawn prompt carries the fixed ready-signal line of the canonical spawn-prompt skeleton ([`roles/director.md`](../roles/director.md) § *Canonical spawn-prompt skeleton*), instructing the member, as its first operational broker shell command, to send its `ready` message (member-side protocol: [`roles/member.md`](../roles/member.md) § *On spawn — send the ready signal*). A skeleton render inherits the line automatically; a hand-written prompt must include it explicitly. It is the ONLY signal that the coding agent inside the pane has actually booted; a prompt missing the line is a defect — fix and re-spawn.
4. **Verify the member is placed** by checking that `cafleet member list <fleet-id>` shows the new member with a non-null `pane_id`. This confirms the pane was created; a missing or pending row means that spawn failed — retry it. The placement audit gates nothing — first-task dispatch rides each member's ready signal (*Dispatch-on-ready* below), and liveness of the coding agent inside the pane is confirmed by that signal, NOT by `member list`.
5. **End the active turn after spawn-and-verify.** The ready signal arrives via the re-engagement channels (§ Communication Model → *Facilitation cue*); you process it — ACK, dispatch first task — in your next active turn. See § *Asynchronous Wait Rule* below.

**Dispatch-on-ready.** When a member's ready signal arrives, ACK it and dispatch that member's first task in the same turn, provided the task's inputs exist. The dispatch is unconditional: a member that sent `ready` and kept reading is still mid-turn, and the broker holds the assignment and delivers it when the member goes idle, with no second signal from the member and no later action from you. First-task dispatch is per-member: never hold a ready member's dispatch waiting for other members' ready signals or placements. A member whose first task genuinely depends on an input that does not yet exist (e.g. a deliverable another member has not produced) legitimately stays idle until that input lands — dispatch whatever is dispatchable, to whoever is ready.

Spawn an ordinary member only after the monitor member's `ready`. Keep the monitor member alive until all work is fully complete and the team is being shut down.

### Asynchronous Wait Rule

The active turn consumes inputs that have already arrived and dispatches what is ready — then returns control. Waiting for things that have not yet arrived is the job of the re-engagement channels (§ Communication Model → *Facilitation cue*).

> **An asynchronous handoff is a turn boundary.** After a Director dispatches work to a member, the Director MUST end or yield its active turn. It MUST NOT remain active waiting for completion by running `sleep`, repeated or periodic `cafleet message poll`, a busy-wait loop, or any equivalent timer/polling loop. A CAFleet/monitor notification resumes the workflow in a later turn. A user-requested one-off status check is allowed, but MUST NOT become recurring polling.

This boundary applies after spawn dispatch, ordinary assignments, review routes, and every other asynchronous member handoff. It does not prohibit the on-demand inbox poll performed when an inbound notification has already reopened a later turn, and it does not prohibit one user-requested status snapshot. After acting on already-arrived inputs and dispatching new work, the Director returns control again. One-shot command isolation ([`SKILL.md`](../SKILL.md) § *One-shot command isolation*) complements this rule; neither replaces the other.

| Situation | Director action |
|---|---|
| Just spawned a member; ready signal not yet arrived | End the turn. The broker delivers the ready signal once your pane is at rest; the monitor member and the broker's silent-member notice are the backstops. When it lands, ACK and dispatch that member's first task in the same turn (§ Spawn Protocol → *Dispatch-on-ready*). |
| Just dispatched to a member; reply not yet arrived | End the turn. Same wake-up channels surface the reply. |
| Waiting on multiple members' replies before next step | End the turn. React to each arrival as its own wake-up, not all-at-once — never hold one member's dispatch waiting for another's arrival (§ Spawn Protocol → *Dispatch-on-ready*). |
| User asks "what's the status?" while members are working | Report the asynchronous truth (e.g. "Alice is processing X; her completion will surface in my next turn"). For a live snapshot, use `cafleet member capture`. |
| Turn finished dispatching and ACKing | End the turn. The next wake-up reopens the turn when there is something to act on. |

## Team-facilitation instructions

On every supervision tick — whether fired by inbound work arriving via the broker's inline-preview keystroke, by a monitor event message or stalled-Director ping, or executed inline within an active turn — the Director runs these five steps in order. The goal is to **facilitate the team in completing tasks**, not merely to detect stalls.

1. **Poll inbox.** `cafleet message poll <director-member-id>` returns only the un-acked (`input_required`) deliveries; ACKing each one (step 2) consumes it — the poll semantics are canonical at § Stall Response → Stage 1.
2. **ACK every message** that requires no further action: `cafleet message ack <message-id>`. Unacknowledged messages accumulate in the Director's inbox and obscure new arrivals.
3. **Dispatch queued work.** If a member is idle and inputs are available (a freshly-arrived ready signal whose first task's inputs exist, review comments to route, the next implementation step in a design doc, reviewer feedback waiting at the Drafter, a teammate reply waiting to be acted on), send the instruction immediately via `cafleet message send` — per-member, never held for other members' arrivals (§ Spawn Protocol → *Dispatch-on-ready*). **Do not wait for a fresh "go" from the user** — the user's original authorization persists across ticks; see § Authorization-Scope Guard.
4. **Run the health-check sequence** for any member that has not reported recent progress — cheapest, least-intrusive check first: (a) `cafleet member list` (enumerate members + pane status); (b) `cafleet message poll` (progress reports / help requests); (c) `cafleet monitor scan <fleet-id>` — its section for the member, or a targeted `cafleet member capture` for deeper investigation — classified per § Idle Semantics; (d) `cafleet message send` a specific instruction to a member that has outstanding work or is a confirmed stall — the broker holds the keystroke while the member is busy, so the send itself needs no precondition; (e) once all members report completion, tell the user "All deliverables are ready for review."
5. **Escalate** to the user via {decision_surface} whenever a queued action requires a *new* user decision (option choice, risky/remote-visible operation, ambiguous teammate question); for the stall path (two fired sends with no progress) see § Stall Response → Escalation. The tick is a health check, not a permission renewal (§ Authorization-Scope Guard).

After the five steps, honor the resume clause of whatever keystroke re-opened your turn: if it landed while your own task was mid-flight, pick that task back up before ending the turn.

### Routing member command requests

The workflow's spawned members run in workspace-scoped auto-approval mode ({permission_flags}; Bash tool enabled, broker commands allowed from spawn). A member runs what its harness allows and routes the rest: it sends `Need to run: <command>. My harness denied it.` via `cafleet message send` and ends its turn, and you run the command with `cafleet member exec <member-id> "<command>"`, then ACK the request. The broker reports the exit status as a notice and resumes the member — no ping and no capture follow. Routing is the standard path for a command the harness does not run, so expect it routinely; process requests in poll order ([`reference/prompt-routing.md`](prompt-routing.md)).

## Monitor Lifecycle

| Phase | Action |
|---|---|
| Spawn (before any ordinary member) | The `cafleet fleet create` bootstrap spawns the monitor member and starts the loop; wait for the monitor's `ready` per § *Spawn Protocol* → *Wait for the monitor's ready* — that message (plus the CLI monitor-first guard) gates the first ordinary `cafleet member create`. Re-spawn a dead monitor with `member create --role monitor`. |
| Run work | Every tick the loop delivers the keystrokes the broker is holding. The `Esc`-first wake lands in the **monitor member's** pane per wake interval (default 600 s); the monitor scans, classifies, pings a confirmed-quiet member once per quiet period, and messages you per event. Each inbound preview, broker notice, monitor event, or monitor ping is the cue to run the 5-step facilitation loop above. |
| User review | Keep the monitor member alive during the review cycle — revisions and re-reviews still count as in-progress work. |
| Teardown | Delete the monitor member FIRST (first-out); the full ordering is § *Cleanup Protocol*. |

**Lifecycle rule (non-negotiable):** The monitor member MUST stay alive from before the first ordinary `member create` through every phase, until the teardown above deletes it first-out.

## Stall Response

The monitor member's event messages, the broker's notices, and your own captures across facilitation turns are the evidence for the facilitation loop.

**What counts as stalled.** A member is stalled if it went idle without delivering expected output, without a meaningful progress update, or when a downstream task should have started but hasn't. Nudge a stalled member with a specific `cafleet message send` about what you expect next. Each workflow states its own wake sources — the turns on which you run this check — in its Director role file.

> **Command request blocking case**: A member message asking for a command is run per § *Team-facilitation instructions* → *Routing member command requests*. The member ended its turn to wait for it, so run it before moving on to other inbox items.

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

> **The decision surface is a backend delta.** The concrete user-reaction surface by which the Director asks the user is backend-specific — see your overlay section ([`coding-agents.md#<name>`](coding-agents.md)). The canonical, backend-neutral user-reaction rule is [`SKILL.md`](../SKILL.md) § *Soliciting user reactions*.

### Escalation

If a member is still unresponsive after 2 **delivered** re-engagement sends via `cafleet message send` AND `cafleet member capture` shows no forward progress in the terminal buffer, escalate to the user via {decision_surface} (per [`SKILL.md`](../SKILL.md) § *Soliciting user reactions*) with concrete options (e.g. re-send the instruction once more / re-spawn the member / drop its task). Only sends whose preview actually landed count toward the threshold: a send the broker is still holding does not advance the count. A member that remains `awaiting_user` or `working` across many rounds is not "unresponsive" — it is parked on the user or making progress.

The unblock primitives and their ordering — non-intrusive `cafleet message poll` → read-only `cafleet member capture` → authoritative `cafleet message send` → `cafleet member ping` (a quiet pane) → `cafleet member exec <member-id> "<cmd>"` (a command the member cannot run) → `cafleet member delete` (last resort, kills the pane immediately) → escalate to the user via {decision_surface} — are documented in [`roles/director.md`](../roles/director.md), [`reference/supervision.md`](supervision.md#recovery), [`reference/prompt-routing.md`](prompt-routing.md), and the § Quick Reference table below.

## User Delegation Protocol

CAFleet members never talk to the user directly — the Director relays. This is the relay-specific application of the canonical rule in [`SKILL.md`](../SKILL.md) § *Soliciting user reactions* (the question-shape taxonomy is in your overlay). When a member sends a `cafleet message send` asking for user input:

1. **Classify the question shape** per the question-shape taxonomy in your overlay (choice among labeled options, approve / yes-no, continue-or-abort, or open-ended / draft selection), and present it through {decision_surface}, mirroring the shape into options where the surface supports them. Follow your overlay for how the surface handles free-form text.
2. **Ask the user.** No preamble sentence above the question — the conversation context plus the question text carry it. One prompt per decision: batch multiple members' questions only when they are genuinely the same decision.
3. **Relay the answer back** via `cafleet message send` to the originating member. Pass through the user's selection verbatim; do not substitute your own judgment. If the user provided free-form text instead of a listed option, send that text.

A decision-prompt frame seen only in a capture is `awaiting_user`: leave it to the member, without inferring or answering the prompt. An explicit question received through `cafleet message send` follows the relay above. The backend's decision surface remains defined by its overlay; the broker reply procedure is [`roles/director.md`](../roles/director.md#answering-a-members-relayed-question).

### Free-form replies — judging intent

When the user supplies free-form text instead of a listed option, use LLM reasoning to determine intent — not keyword matching. Interpret the user's text to distinguish between:

- **Abort intent** (the user wants to stop or cancel the process)
- **Non-abort intent** (the user is providing verbal feedback or asking a question)

On **abort intent**, run the Abort Flow: tear down the team per § *Cleanup Protocol*, ending in `cafleet fleet delete <fleet-id>`, which soft-deletes the fleet and sweeps the root Director in one transaction.

On **non-abort intent**, explain that feedback belongs in `COMMENT(` markers at the workflow's own feedback target, then re-prompt with the same option pattern. Each workflow names that target in its Director role file.

## Cleanup Protocol

Read and follow [Shutdown](#shutdown) immediately before cleanup: delete the monitor first, then remaining authorized members, verify the root-only registry, delete the fleet and confirm closure.

A workflow that carries extra teardown — a precondition on when shutdown may begin, a roster-specific delete order, or non-CAFleet resources to release — runs those steps around this sequence and names them in its own Director role file.

## Quick Reference

| Action | Primitive | Notes |
|---|---|---|
| Verify Director pane env | `cafleet doctor` | Pre-spawn precondition; gating. Aborts the spawn protocol on any rendered issue — a multiplexer failure, a database-schema issue, a stale/invalid coding-agent state, or a setting that blocks a member's broker commands (the not-installed state never counts). Replaces raw `tmux display-message` and `TMUX` env-var expansion. |
| Bootstrap fleet + monitor member | `cafleet fleet create --name <n> --coding-agent <backend> --monitor-file <abs path to ${BASE}/.prompts/monitor-<UTC-compact>.md> --monitor-model {monitor_model} [--monitor-effort {monitor_effort}] --json` | Include effort for Claude or Codex. One command creates the fleet, Director, and monitor, and starts the monitor loop. Wait for the monitor's `ready` before the first ordinary `member create`. |
| Re-spawn a dead monitor member | `cafleet member create --fleet-id <s> --role monitor --model {monitor_model} [--effort {monitor_effort}] --name monitor --description <d> --file <abs path to ${BASE}/.prompts/monitor-<UTC-compact>.md>` | Include effort for Claude or Codex. Mid-run recovery inherits the backend. Wait for its `ready`. |
| Fleet-wide pane snapshot | `cafleet monitor scan <s>` | The health-check read for every member at once (§ Idle Semantics). |
| Spawn member | `cafleet member create --fleet-id <s> --name <n> --description <d> --file <abs path to ${BASE}/.prompts/<role>-<UTC-compact>.md>` | Pre-spawn file IS the audit artifact (see [`roles/director.md`](../roles/director.md) § *Member Create — Scratch and audit files*). Verify with `cafleet member list`. An inline positional `"<prompt>"` is still permitted for trivial one-line spawns. |
| Message member | `cafleet message send --from-member-id <director> --to-member-id <member> "..."` | Send at once; the broker keystrokes the inline preview when the member's pane is at rest (§ *Broker-held delivery*) |
| ACK reply | `cafleet message ack <message>` | Unacknowledged messages accumulate; ACK every reply you act on |
| Inspect stalled member | `cafleet member capture <member>` | Targeted deeper investigation of a single pane; replaces raw `tmux capture-pane` |
| Manual inbox-poll | `cafleet member ping <member>` | Pre-approved; re-pokes a quiet member. The broker skips a busy pane with exit 0 and a reason |
| Run a command for a member | `cafleet member exec <member> "<cmd>"` | Per [`reference/prompt-routing.md`](prompt-routing.md); the broker reports completion and resumes the member |
| Direct user turn in a member's pane | `cafleet member prompt <member> "<text>"` | Slash commands and other text a message body cannot trigger |
| Answer a member's relayed question | {decision_surface} → `cafleet message send` | Ask the user via {decision_surface} first, then relay the answer back to the member as a message; never decide silently |
| Relay user input | {decision_surface} → `cafleet message send` | Pass-through; never substitute judgment |
| Shut down team | [Shutdown](#shutdown) | Delete the monitor first → delete remaining authorized members → verify root-only registry → delete fleet → confirm closure. |

## Recovery

Read this section immediately before recovery. The Director owns recovery decisions within the task's authorized fleet scope.

### 2-stage health check

Before assuming a member is stalled, run the cheap check first — poll your own inbox, then capture the member's pane — per [`supervision.md`](supervision.md) § Stall Response. Recovery-specific detail: bump `cafleet member capture --lines` to show a member's full decision-prompt frame (the line count needed is a backend delta, see your overlay).

### Recovery entry conditions

[Idle Semantics](#idle-semantics) owns the Director's reading of a capture,
including quiet confirmation, and [Escalation](#escalation) owns the next
step. `member list` supplies registration and idle context; idle duration
and unread counts do not establish a stall. A message that has not been
consumed is either still held by the broker or waiting in the member's inbox:
`cafleet member ping` re-pokes a quiet member, and the broker skips the ping
for a busy pane.

A captured decision prompt is `awaiting_user`, not an instruction to answer
it. Relay only a question explicitly sent by the member, per
[`Answering a member's relayed question`](../roles/director.md#answering-a-members-relayed-question).
For a command a member routed to you, run `cafleet member exec` per
[`prompt-routing.md`](prompt-routing.md).

A member that never sent `ready` produces a broker notice after 180 s. Capture
its pane and run `cafleet doctor`: its broker commands may be denied by a
setting the spawn-time allow rules cannot override.

If the monitor loop is not running — the admin WebUI shows it stopped, or a
command reports that the loop did not start — read the log file the error
names and run `cafleet doctor`. The next `message send`, `message broadcast`,
or `member exec` that leaves work owed starts the loop again; held messages
are delivered once it runs.

Once inspection confirms the coding agent exited or its pane disappeared,
use `cafleet member delete` to cleanly deregister it, then `cafleet member create`
to re-spawn. The new registration has a new `member_id`. For an unresponsive
but existing agent, use supervision's escalation procedure before deciding
to re-spawn; elapsed ticks alone do not authorize deleting it.

### Recovering from a tmux disconnect

If `cafleet member capture` exits with a multiplexer subprocess error (the multiplexer server is unreachable):

1. Run `cafleet doctor` to confirm your own pane's multiplexer state. If it fails to resolve a backend, you are no longer attached to a supported multiplexer session and recovery is impossible from this shell — re-attach (on tmux, `tmux attach -t <session>`) and re-run.
2. A successful `cafleet doctor` with a failed capture does not prove the target pane is gone. Use `cafleet member list <fleet-id> --json` or `cafleet member show <member-id> --json` to identify the registered backend and pane, then investigate the capture/connection error. The registry alone does not establish physical pane presence or absence: retain `unknown` and do not ping or delete the member on that evidence alone. Ask the user for missing facts about the target pane only when that uncertainty actually blocks the work, not after every failed capture.
3. Never invoke raw tmux directly — cafleet's primitives encapsulate the fleet-isolation boundary that raw tmux bypasses.


## Shutdown

Read this section immediately before teardown within the task's authorized fleet scope, retaining any workflow-specific preconditions.

The teardown runs in this exact order. **Use cafleet primitives only** — every tmux interaction (write, inspect, metadata) is encapsulated by a cafleet command (`cafleet doctor` for pane metadata at startup); never invoke raw tmux from the Director.

1. **Delete the monitor member FIRST** (`cafleet member delete <monitor-member-id>`, first-out), so no health check runs against a fleet that is being torn down. The monitor loop is a separate detached process: it keeps running until `cafleet fleet delete` (step 4), stops on its next tick after that, and its `monitor_runtime` row is removed by the same command (see [`monitoring.md`](runtime/concepts/monitoring.md)).
2. **Delete every remaining member** via `cafleet member delete`. This call kills the pane immediately. Do this per member, not via `fleet delete` alone — `fleet delete` deregisters members in the DB but does NOT kill their panes.
3. **Verify every member is gone via cafleet.** Run `cafleet member list`. Only the root Director's own row (`kind` `director`) should remain. Any other member still present means step 2 failed — re-run `cafleet member delete` on that member, capture if needed, and report to the user if it still refuses to leave.
4. **Run `cafleet fleet delete <fleet-id>`.** This deregisters the root Director, sweeps any member rows that survived step 2, and deletes every `member_placements` row. Deleting the root Director via `member delete` is rejected — always use `fleet delete` for the final teardown step.
5. **Confirm the fleet is closed.** Run `cafleet fleet list`; the current fleet should not appear (soft-deleted fleets are hidden). If it still appears with `active` members, repeat steps 2–4 for that fleet. Other conversations' fleets are diagnostic information. Clean up only fleets covered by the task's user authorization; obtain that authorization before acting on an unrelated fleet.
