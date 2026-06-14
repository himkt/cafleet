---
name: cafleet-agent-team-monitoring
description: "Active monitoring mechanism for CAFleet Directors. Documents the external cafleet monitor heartbeat process that wakes only the dedicated monitoring member on its interval (backend-agnostic), and the team-facilitation instructions (poll, ACK, dispatch queued work, health-check, escalate) the Director runs when the monitoring member re-engages it on demand. Load whenever you are about to spawn or manage CAFleet team members. Foundation layer — load before the cafleet-agent-team-supervision skill."
---

# CAFleet Agent Team Monitoring

Foundation layer for CAFleet Directors. This skill documents the `cafleet monitor` heartbeat that wakes a Director periodically and the team-facilitation instructions it executes on each tick. Load this skill before the `cafleet-agent-team-supervision` skill — supervision builds on the mechanism documented here.

## Placeholder convention

Every command below uses angle-bracket tokens (`<fleet-id>`, `<director-agent-id>`, `<member-agent-id>`) as **placeholders, not shell variables**. Substitute the literal integer ids printed by `cafleet fleet create` (which returns both the fleet id and the root Director's `agent_id` — see the `cafleet` skill § Typical Workflow for the exact output shape) directly into the command. Do **not** introduce shell variables for agent or fleet IDs — `permissions.allow` matches command strings literally, and shell expansion breaks that matching.

**Flag placement**: `--fleet-id` and `--agent-id` are both per-subcommand options (placed **after** the subcommand name). For example: `cafleet message poll --fleet-id <fleet-id> --agent-id <director-agent-id>`.

- `<fleet-id>` — the fleet id printed on line 1 of `cafleet fleet create` text output (or the `fleet_id` field in `--json` output)
- `<director-agent-id>` — the root Director's id printed on line 2 of `cafleet fleet create` text output (or `director.agent_id` in `--json` output). `cafleet fleet create` inside a tmux session auto-bootstraps the root Director with its placement row — no separate `cafleet agent register` call is needed to obtain the Director's `agent_id`.
- `<member-agent-id>` — a target member's agent id (from `member create` / `member list`)

## The monitor heartbeat

CAFleet members do not act autonomously. The Director drives the team — and the Director needs a way to wake itself up periodically to check inboxes, dispatch queued work, and detect stalls. That heartbeat is supplied by **`cafleet monitor`**, a per-fleet `scan → ping → sleep` loop that the fleet's dedicated **monitoring member** runs as a **background task** in its own pane. Because it is just a backgrounded command, the heartbeat is **backend-agnostic** — a root Director on `claude`, `codex`, or `opencode` gets the identical tick. There is no per-backend scheduling asymmetry: the monitor is the one mechanism for every backend.

The loop wakes **only the monitoring member** — never the root Director and never ordinary members. Every ping is **`Esc`-safeguarded**: the loop presses `Escape`, lets the pane settle ~0.1 s, then types the literal text and `Enter`, so a pane sitting on a pending permission-approval prompt dismisses the prompt instead of having the trailing `Enter` confirm it. The monitoring member is pinged **unconditionally** on its interval (default 60 s) once due; `pending_count` is shown in `monitor status` but does not gate the ping. The keystroke is a single-line *wake nudge* instructing the monitoring member to run its capture-classify-reengage routine (see [The monitoring member](#the-monitoring-member)).

See the `cafleet` skill and the [Monitoring concepts page](https://himkt.github.io/cafleet/concepts/monitoring/) for the full command surface and policy. **The monitoring member — not the Director — runs `cafleet monitor start`** (see § Monitor Lifecycle).

**The Director is never woken by the loop.** It is re-engaged only on demand: by the monitoring member's idle nudge (an `Esc`-safeguarded `cafleet message send` when the routine classifies the Director as idle), and by the broker's inline-preview keystroke on every inbound `cafleet message send`. When woken by one of those, the Director runs the entire 5-step facilitation loop (poll → ACK → dispatch → health-check → escalate), not just an inbox read. The monitor decides only *when* to wake the monitoring member; this skill defines *what* the Director does once re-engaged.

### How ordinary members are woken

With ordinary members no longer enrolled in the monitor, the loop never nudges them. Member re-engagement is always Director-mediated, via two paths:

1. **Primary** — the broker's inline-preview keystroke fired on every `cafleet message send` (`tmux.send_inline_preview`), landing the instant the Director or a teammate sends work.
2. **Manual recovery** — the Director's `Esc`-safeguarded `cafleet member ping` (it reuses the `send_poll_trigger` helper, so it inherits the same `Esc` safeguard), for a member that missed its inline preview or looks stalled.

A member that has gone quiet is surfaced to the Director by the monitoring member's idle assessment; the Director then re-pings via `cafleet member ping` or re-sends the instruction. There is no automatic, unconditional member nudge.

## The monitoring member

The monitoring member is a single, dedicated coding-agent member — spawned **first** in the fleet with `cafleet member create --role monitor --model sonnet` — that owns the heartbeat and applies LLM judgment to the Director's state. `--role monitor` sets `agent_card_json.cafleet.kind == "monitoring-member"` and enrolls it in `monitor_config`; only one is allowed per fleet (a second `--role monitor` spawn is rejected). It is the **one** process that runs `cafleet monitor start` — the Director no longer runs the monitor itself.

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
1. Capture the Director's pane:
   cafleet member capture --fleet-id {fleet_id} --member-id {director_agent_id} --lines 120
2. Classify the Director ACTIVE vs IDLE with your own judgment (mid-turn, running
   a tool, or typing = ACTIVE; sitting at an empty prompt with un-acked inbox or
   visibly stalled members = IDLE).
   - ACTIVE -> do nothing; end your turn.
   - IDLE -> assess the full picture: the Director's inbox, its current task, and
     any ordinary members that look stalled (read-only
     cafleet member capture --fleet-id {fleet_id} --member-id <member-id>). Then
     re-engage the DIRECTOR with a concise nudge naming what needs attention
     (un-acked inbox items, stalled members):
     cafleet message send --fleet-id {fleet_id} --agent-id {agent_id} --to {director_agent_id} --text "<summary>"
   Never keystroke task instructions into an ordinary member's pane.

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
4. **Run the health-check sequence** below for any member that has not reported recent progress.
5. **Escalate** to the user via `AskUserQuestion` after two nudges produce no progress, or whenever a queued action requires a *new* user decision (option choice, risky/remote-visible operation, ambiguous teammate question). Do **not** emit passive-hold messages like `Skipping. Holding for go.` — the tick is a health check, not a permission renewal.

## Health-Check Sequence

Run this sequence once per supervision tick. Order matters — cheapest non-intrusive check first, most invasive last.

| Step | Command | Purpose |
|---|---|---|
| 1 | `cafleet member list --fleet-id <fleet-id>` | Enumerate all live members and their pane status |
| 2 | `cafleet message poll --fleet-id <fleet-id> --agent-id <director-agent-id>` | Check inbox for progress reports or help requests from members |
| 3 | For each member with no recent message: `cafleet member capture --fleet-id <fleet-id> --member-id <member-agent-id>` | Terminal capture fallback — inspect what the member is doing when it has not reported in. If the capture shows an `AskUserQuestion`-style prompt, see Stall Response below for the `member send-input` escape hatch. |
| 4 | Based on findings, `cafleet message send --fleet-id <fleet-id> --agent-id <director-agent-id> --to <member-agent-id> --text "..."` to any stalled or idle member with a specific instruction | Drive the team forward |
| 5 | When all members have reported completion (via messages or visible in terminal output), report to the user: "All deliverables are ready for review." | Signal completion to user |

## Monitor Lifecycle

| Phase | Action |
|---|---|
| Spawn the monitoring member (first-in) | The **first** `cafleet member create` in the fleet IS the monitoring member: `cafleet member create --fleet-id <fleet-id> --agent-id <director-agent-id> --name monitor --description <…> --role monitor --model sonnet --prompt-file <rendered monitor prompt>`. It boots, launches `cafleet monitor start` as a background task in its own pane, confirms `monitor status`, and sends `ready: monitor live` to the Director. |
| Gate ordinary members | Wait for the monitoring member's `ready: monitor live` message before the first ordinary `cafleet member create`. The Director MAY run `cafleet monitor status --fleet-id <fleet-id>` itself as optional corroboration, but it waits on the handshake message rather than block-polling status (consistent with the async wait rule). |
| Run work | The monitor ticks at its configured cadence (default ping interval 60 s), waking the monitoring member; do not intervene unless an escalation arrives. Each on-demand idle nudge from the monitoring member (or inbound work via inline preview) is the Director's cue to run the 5-step facilitation loop above. |
| User review | Keep the monitoring member and its `monitor start` task running during the review cycle — revisions and re-reviews still count as in-progress work. |
| Teardown (first-out) | Stop the monitor's background task, then delete the monitoring member FIRST, before ordinary members (see Cleanup below). |

**Lifecycle rule:** The monitoring member MUST stay running (with its `monitor start` background task live) from the first `member create` through every phase (research, compilation, review, revision, user approval). At teardown, **stop the monitor's background task BEFORE the monitoring member's pane is killed** — this is step 1 of the canonical teardown in the `cafleet` skill § *Shutdown Protocol* (the authoritative full ordering) and is non-negotiable: a monitor that keeps ticking after the monitoring member is deleted keystrokes ping commands into tearing-down panes and races with the delete path. There is no `cafleet monitor stop` — the clean stop is the Director messaging the monitoring member to stop its `monitor start` background task (the task-stop delivers SIGTERM/SIGINT, so the loop runs `finally` and clears the runtime row); the monitoring member confirms; only then does the Director `cafleet member delete` it. Because the loop no longer pings ordinary members, this "stop the monitor first" race now applies only to the monitoring member itself.

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

> **Note that `AskUserQuestion` should be used in Claude Code.** The "delegate to the user" beat in the workflow above assumes the Director itself runs in Claude Code, where `AskUserQuestion` is the dedicated tool for putting a structured choice in front of the operator. Directors running another coding agent must substitute their own equivalent decision-elicitation surface (or fall back to a plain message to the operator). The 4-option-frame shape that `cafleet member send-input` itself targets is a Claude Code idiom — neither codex nor opencode members render the same frame, so on a codex or opencode member the read-then-respond cadence applies but the `--choice` / `--freetext` keystrokes apply only when the captured buffer matches the validated 4-option layout.

### Escalation

If a member is still unresponsive after 2 nudges via `cafleet message send` AND `cafleet member capture` shows no forward progress in the terminal buffer, escalate to the user.

| Channel | Type | When to use |
|---|---|---|
| `cafleet ... message poll --agent-id <director-agent-id>` | Non-intrusive, message-based | First — check if the member has reported in |
| `cafleet ... member capture --member-id <member-agent-id>` | Non-intrusive, terminal snapshot | Second — when no messages, inspect what the member is doing |
| `cafleet ... message send --agent-id <director-agent-id> --to <member-agent-id> --text "..."` | Interactive, authoritative | Third — send a specific instruction to unstick the member (broker keystrokes a 2-line inline preview of the message into the member's pane via `tmux.send_inline_preview` after persisting; no `cafleet message poll` invocation is in the auto-fire path) |
| `cafleet ... member ping --member-id <member-agent-id>` | Interactive, fixed-action keystroke | Director's pre-approved manual inbox-poll nudge — keystrokes `Esc` + `cafleet message poll` into the member's pane (the leading `Esc`, inherited from the `send_poll_trigger` helper, dismisses any pending permission prompt so the trailing `Enter` cannot confirm it) as a manual re-poke for the case where the broker's inline preview was missed. Two use cases: **(a)** a member appears stalled despite a recent `cafleet message send` (the inline preview was missed or the pane was busy when it arrived), or after a long idle window with a queued message still unread; **(b)** post-`member exec` chain — the Director MUST follow every successful `cafleet member exec` with this ping so the member's next turn fires immediately (see the `cafleet` skill § Member Exec for the chain definition; do not duplicate the wording). No positional argument, pre-approved in `permissions.allow`. The only boundary is fleet isolation (cross-fleet `--member-id` → not found; no caller check), shared with `capture` / `send-input` / `exec`. Failures surface as exit 1 (the auto-fire path swallows them silently). |
| `cafleet ... member send-input --member-id <member-agent-id> (--choice N \| --freetext "<text>")` | Interactive, restricted keystroke | `--choice` / `--freetext` answer an `AskUserQuestion`-shaped prompt — delegate the decision to the user via the Director's own `AskUserQuestion` tool call FIRST, then invoke the resolved command via the Director's Bash tool (the coding agent's native per-call permission prompt is the consent surface; never print a fenced `bash` block for the user to paste). See the cafleet skill's "Answer a member's AskUserQuestion prompt" section for the canonical three-beat workflow + pane-shapes table. The only boundary is fleet isolation, shared with `capture`. |
| `cafleet ... member exec --member-id <member-agent-id> "<cmd>"` | Interactive, keystroke dispatch | Director-only shell-dispatch primitive — keystrokes `! <cmd>` + Enter into the member's pane via the coding agent's `!` shortcut (honored by both `claude` and `codex`). Shell-dispatch only — for inbox-poll-only nudges use `member ping`. See ping row for the required follow-up after every successful exec. The only boundary is fleet isolation, shared with `capture` / `send-input`. See the `cafleet` skill § Routing Bash via the Director. |
| `cafleet ... member delete --member-id <member-agent-id> --force` | Interactive, destructive | When `member delete` has already exited 2 and `capture` + `send-input` have failed to unblock the pane — forces an atomic `kill_pane` + deregister + layout rebalance. Never fall back to raw `tmux kill-pane`. |
| Process pending shell-command request from member | Blocking on member side | Dispatch via `cafleet member exec "<cmd>"` per the `cafleet` skill § Routing Bash via the Director. Don't skip past a member's request — the member sits idle until the keystroke lands. |
| Escalate to user | Last resort | After 2 nudges + no progress in terminal |
