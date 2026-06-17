---
name: cafleet-agent-team-monitoring
description: "Active monitoring mechanism for CAFleet Directors. Documents the external cafleet monitor heartbeat process that watches the root Director (180 s) and every ordinary member (720 s) on their own per-agent intervals and wakes the dedicated monitoring member whenever a watched agent is due (backend-agnostic), and the team-facilitation instructions (poll, ACK, dispatch queued work, health-check, escalate) the Director runs when the monitoring member re-engages it on demand. Load whenever you are about to spawn or manage CAFleet team members. Foundation layer — load before the cafleet-agent-team-supervision skill."
---

# CAFleet Agent Team Monitoring

Foundation layer for CAFleet Directors. This skill documents the `cafleet monitor` heartbeat that wakes a Director periodically and the team-facilitation instructions it executes on each tick. Load this skill before the `cafleet-agent-team-supervision` skill — supervision builds on the mechanism documented here.

## Placeholder convention

Angle-bracket tokens (`<fleet-id>`, `<director-agent-id>`, `<member-agent-id>`) are placeholders, **not** shell variables — substitute the literal integer ids from `cafleet fleet create` (which returns both the fleet id and the root Director's `agent_id`, so no separate `cafleet agent register`). The rule and flag placement are canonical in the `cafleet` skill § Placeholder convention.

## The monitor heartbeat

CAFleet members do not act autonomously. The Director drives the team — and the Director needs a way to wake itself up periodically to check inboxes, dispatch queued work, and detect stalls. That heartbeat is supplied by **`cafleet monitor`**, a per-fleet `scan → wake → sleep` loop that the fleet's dedicated **monitoring member** runs as a **background task** in its own pane. Because it is just a backgrounded command, the heartbeat is **backend-agnostic** — a root Director on `claude`, `codex`, or `opencode` gets the identical tick. There is no per-backend scheduling asymmetry: the monitor is the one mechanism for every backend.

Each tick the loop scans the **watched set** — the root Director (default **180 s**) and every ordinary member (default **720 s**), each on its own per-agent interval — and, when **≥ 1 watched agent is due**, wakes the monitoring member **once**. The loop's only keystroke is into the monitoring member's own pane; it **never** keystrokes a watched pane (the Director or an ordinary member). The wake nudge does **not** lead with `Esc`: the monitoring member's pane runs a read-only routine under `dontAsk` and is never parked on a permission-approval prompt, so a leading `Esc` would merely self-interrupt an in-progress routine. (The `Esc` safeguard instead lives where a target may be on a prompt — the broker's message-delivery inline preview and `cafleet member ping`.) The dedicated monitoring member is the **watcher**, not a watched agent — it carries no interval and is located by its `agent_card_json.cafleet.kind == "monitoring-member"` marker, not by a `monitor_config` row. The keystroke is a single-line *wake nudge* instructing the monitoring member to run its capture-classify-reengage routine over the freshly-due agents (see [The monitoring member](#the-monitoring-member)).

See the `cafleet` skill and the [Monitoring concepts page](https://himkt.github.io/cafleet/concepts/monitoring/) for the full command surface and policy. **The monitoring member — not the Director — runs `cafleet monitor start`** (see § Monitor Lifecycle).

**The Director is never woken by the loop.** It is re-engaged only on demand: by the monitoring member's idle nudge (`cafleet member nudge`, which persists an ACKable broker task **and** fires the hardened, `Esc`-safeguarded inline preview, when the routine classifies the Director as idle), and by the broker's inline-preview keystroke on every inbound `cafleet message send`. When woken by one of those, the Director runs the entire 5-step facilitation loop (poll → ACK → dispatch → health-check → escalate), not just an inbox read. The monitor decides only *when* to wake the monitoring member; this skill defines *what* the Director does once re-engaged.

### How ordinary members are woken

Ordinary members are **watched** (each enrolled with its own 720 s interval), but the loop **never keystrokes a member pane**. When a member comes due, the loop wakes the *monitoring member*, which captures the member read-only and surfaces a stall to the Director. Member re-engagement is always Director-mediated, via two paths:

1. **Primary** — the broker's inline-preview keystroke fired on every `cafleet message send` (`tmux.send_inline_preview`), landing the instant the Director or a teammate sends work.
2. **Manual recovery** — the Director's `Esc`-safeguarded `cafleet member ping` (it reuses the `send_poll_trigger` helper, so it inherits the same `Esc` safeguard), for a member that missed its inline preview or looks stalled.

A member that has gone quiet is surfaced to the Director by the monitoring member's assessment; the Director then re-pings via `cafleet member ping` or re-sends the instruction. The monitoring member never keystrokes task instructions into a member's pane.

## The monitoring member

The monitoring member is a single, dedicated coding-agent member — spawned **first** in the fleet with `cafleet member create --role monitor --model sonnet` — that owns the heartbeat and applies LLM judgment to the watched agents' state (the Director **and** each freshly-due member). `--role monitor` sets `agent_card_json.cafleet.kind == "monitoring-member"`; the monitoring member is **not** enrolled in `monitor_config` — it is the watcher, located by that kind marker (`find_monitoring_member`), and carries no interval of its own. Only one is allowed per fleet (a second `--role monitor` spawn is rejected). It is the **one** process that runs `cafleet monitor start` — the Director no longer runs the monitor itself.

### Canonical monitoring-member spawn prompt

Render this template to a `--prompt-file` (the `{fleet_id}` / `{agent_id}` / `{director_agent_id}` placeholders are substituted by `cafleet member create`) and spawn with `--role monitor --model sonnet`:

```text
You are the Monitoring Member of CAFleet fleet {fleet_id}. Your agent id is
{agent_id}; the Director's agent id is {director_agent_id}. You have exactly one
job: keep the Director's supervision heartbeat alive and re-engage the Director
whenever the team stalls. You never drive ordinary members directly — all
member-driving routes back through the Director.

Startup (in order, as your first actions):
1. Send the ready signal to the Director:
   cafleet message send --fleet-id {fleet_id} --agent-id {agent_id} --to {director_agent_id} --text "ready: monitoring member"
2. Launch the heartbeat as a background task in THIS pane (the loop blocks, so
   background it): cafleet monitor start --fleet-id {fleet_id}
3. Confirm it is live: cafleet monitor status --fleet-id {fleet_id}
4. Only after status shows running, report the gate signal:
   cafleet message send --fleet-id {fleet_id} --agent-id {agent_id} --to {director_agent_id} --text "ready: monitor live"
   This message gates the Director's first ordinary member create.

On each wake (a "[monitor] wake: ..." nudge keystroked into this pane by the loop):
1. Re-query the watched schedule to find which agents the monitor just flagged:
   cafleet monitor status --fleet-id {fleet_id}
   The freshly-due agents are the ones with the smallest last_ping age (e.g.
   "0s ago"); they are who you inspect this wake.
2. Capture the Director's pane (always — the Director is your only actuation
   target):
   cafleet member capture --fleet-id {fleet_id} --member-id {director_agent_id} --lines 120
   Classify the Director ACTIVE vs IDLE with your own judgment (mid-turn, running
   a tool, or typing = ACTIVE; sitting at an empty prompt with un-acked inbox or
   visibly stalled members = IDLE).
3. For each freshly-due ORDINARY member, capture its pane (read-only) and judge
   whether it is progressing or stalled:
   cafleet member capture --fleet-id {fleet_id} --member-id <member-id> --lines 120
4. Re-engage the DIRECTOR via cafleet member nudge when the Director is IDLE with
   un-acked inbox / stalled members, OR when any inspected member looks stalled —
   naming what needs attention (idle Director, stalled member <id>). member nudge
   persists an ACKable task AND fires the hardened, Esc-safeguarded inline preview,
   so a Director sitting on a permission prompt has it dismissed before the
   preview's Enter lands:
   cafleet member nudge --fleet-id {fleet_id} --agent-id {agent_id} --member-id {director_agent_id} --text "<summary>"
   If the Director is ACTIVE and no inspected member looks stalled, do nothing; end
   your turn. Never keystroke task instructions into an ordinary member's pane —
   all member-driving routes back through the Director.

Teardown: when the Director messages you to wrap up, stop your `monitor start`
background task (this delivers SIGTERM/SIGINT, so the loop clears its runtime
row), confirm to the Director, and return to the prompt. The Director then runs
member delete on you.
```

The loop's wake nudge that drives the "On each wake" routine is the single line:

```text
[monitor] wake: run your monitoring routine now — capture the Director pane,
judge it active vs idle, and if idle assess the inbox and members and re-engage
the Director with an Esc-safeguarded nudge.
```

## Team-facilitation instructions

On every supervision tick — whether fired by the monitoring member's on-demand idle nudge, by inbound work arriving via the broker's inline-preview keystroke, or executed inline within an active turn — the Director runs these five steps in order. The goal is to **facilitate the team in completing tasks**, not merely to detect stalls.

1. **Poll inbox.** `cafleet message poll --fleet-id <fleet-id> --agent-id <director-agent-id>` returns only the un-acked (`input_required`) deliveries; ACKing each one (step 2) consumes it, so the next tick's poll surfaces only what has arrived since.
2. **ACK every message** that requires no further action: `cafleet message ack --fleet-id <fleet-id> --agent-id <director-agent-id> --task-id <task-id>`. Unacknowledged tasks accumulate in the Director's inbox and obscure new arrivals.
3. **Dispatch queued work.** If a member is idle and inputs are available (review comments to route, the next implementation step in a design doc, reviewer feedback waiting at the Drafter, a teammate reply waiting to be acted on), send the instruction immediately via `cafleet message send`. **Do not wait for a fresh "go" from the user** — the user's original authorization persists across ticks; see the `cafleet-agent-team-supervision` skill § Authorization-Scope Guard.
4. **Run the health-check sequence** for any member that has not reported recent progress — cheapest, least-intrusive check first: (a) `cafleet member list` (enumerate members + pane status); (b) `cafleet message poll` (progress reports / help requests); (c) for a member silent since the last check, `cafleet member capture` to inspect it (an `AskUserQuestion`-style 4-option frame → see Stall Response for the `member send-input` escape hatch); (d) `cafleet message send` a specific instruction to any stalled/idle member; (e) once all members report completion, tell the user "All deliverables are ready for review."
5. **Escalate** to the user via `AskUserQuestion` after two nudges produce no progress, or whenever a queued action requires a *new* user decision (option choice, risky/remote-visible operation, ambiguous teammate question). Do **not** emit passive-hold messages like `Skipping. Holding for go.` — the tick is a health check, not a permission renewal.

## Monitor Lifecycle

| Phase | Action |
|---|---|
| Spawn the monitoring member (first-in) | The **first** `cafleet member create` in the fleet IS the monitoring member: `cafleet member create --fleet-id <fleet-id> --agent-id <director-agent-id> --name monitor --description <…> --role monitor --model sonnet --prompt-file <rendered monitor prompt>`. It boots, launches `cafleet monitor start` as a background task in its own pane, confirms `monitor status`, and sends `ready: monitor live` to the Director. |
| Gate ordinary members | Wait for the monitoring member's `ready: monitor live` message before the first ordinary `cafleet member create`. The Director MAY run `cafleet monitor status --fleet-id <fleet-id>` itself as optional corroboration, but it waits on the handshake message rather than block-polling status (consistent with the async wait rule). |
| Run work | The monitor wakes the monitoring member whenever a watched agent is due on its own interval (the root Director at 180 s, ordinary members at 720 s); do not intervene unless an escalation arrives. Each on-demand idle nudge from the monitoring member (or inbound work via inline preview) is the Director's cue to run the 5-step facilitation loop above. |
| User review | Keep the monitoring member and its `monitor start` task running during the review cycle — revisions and re-reviews still count as in-progress work. |
| Teardown (first-out) | Stop the monitor's background task FIRST, then delete the monitoring member before ordinary members. The authoritative full ordering is the `cafleet` skill § *Shutdown Protocol*. |

**Lifecycle rule (non-negotiable):** The monitoring member MUST stay running (with its `monitor start` task live) from the first `member create` through every phase; at teardown the monitor is stopped FIRST (first-out) — a monitor still ticking after the monitoring member is deleted races the delete path. The full stop mechanism + ordering is canonical in the `cafleet` skill § *Shutdown Protocol*.

## Stall Response

When you receive any signal that a member may be stalled (the monitoring member's idle nudge, idle notification, user nudge), evaluate using this 2-stage protocol:

> **Bash request blocking case**: When `cafleet message poll` returns a member message asking for a shell command, dispatch via `cafleet member exec "<cmd>"` per the `cafleet` skill § Routing Bash via the Director. Member blocks until the keystroke lands; process requests one at a time, don't skip ahead to other inbox items.

### Stage 1 — Message-based check (`cafleet message poll`)

```bash
cafleet message poll --fleet-id <fleet-id> --agent-id <director-agent-id>
```

`cafleet message poll` returns only the un-acked (`input_required`) deliveries addressed to the Director, newest first. ACKing a delivery consumes it, so a later poll surfaces only what has arrived since the last ACK — there is no last-tick timestamp to track. If the member has sent a progress report or help request via `cafleet message send`, you can act on it immediately without interrupting the member's work. This is non-intrusive and preferred.

### Stage 2 — Terminal capture fallback (`cafleet member capture`)

```bash
cafleet member capture --fleet-id <fleet-id> \
  --member-id <member-agent-id> --lines 120
```

`--lines 120` is the recommended fallback when classifying a 4-option AskUserQuestion frame (matches the recommendation in `skills/cafleet/reference/director.md` § Answer a member's AskUserQuestion prompt; the CLI flag default is `--lines 30`). Re-run with `--lines 200` as a fallback only if the first capture is truncated above the choice-prompt frame (the `1. …`, `2. …`, `3. …`, `4. Type something` rows are not all visible).

If `cafleet message poll` shows no recent messages from the member, fall back to capturing the terminal buffer. This is non-intrusive (read-only inspection that works even when the member is mid-task) and replaces raw `tmux capture-pane`.

If the terminal buffer shows the member paused on a 4-option choice prompt (a list of "1. …", "2. …", "3. …", "4. Type something" rows — the shape that `cafleet member send-input` is validated for), the correct unblock is `cafleet member send-input` — never raw `tmux send-keys` — and the Director MUST delegate the decision to the user BEFORE invoking the wrapper. The Director never picks the `--choice` digit or drafts the `--freetext` body on its own judgment. The full three-beat workflow and the pane-shapes table live in the cafleet skill's "Answer a member's AskUserQuestion prompt" section — that is canonical; do not duplicate them here.

> **`AskUserQuestion` is a Claude Code idiom.** The "delegate to the user" beat assumes the Director runs in Claude Code; Directors on another coding agent substitute their own decision-elicitation surface (or a plain operator message). The 4-option pane frame that `cafleet member send-input` targets is likewise Claude-Code-specific — on a codex/opencode member the read-then-respond cadence applies, but the `--choice` / `--freetext` keystrokes apply only when the captured buffer matches the validated 4-option layout.

### Escalation

If a member is still unresponsive after 2 nudges via `cafleet message send` AND `cafleet member capture` shows no forward progress in the terminal buffer, escalate to the user.

The unblock primitives and their ordering — non-intrusive `cafleet message poll` → read-only `cafleet member capture` → authoritative `cafleet message send` → `cafleet member ping` (missed auto-fire / required post-`exec` follow-up) → `cafleet member send-input` (4-option prompt, user-delegated first) → `cafleet member exec "<cmd>"` (shell dispatch) → `cafleet member delete --force` (last resort, never raw `tmux kill-pane`) → escalate to the user — are documented in the `cafleet` skill (`reference/director.md`, `reference/recovery.md`, § Routing Bash via the Director) and the supervision Quick Reference table; do not duplicate them here.
