---
name: cafleet-agent-team-monitoring
description: "Active monitoring mechanism for CAFleet Directors. Documents the external cafleet monitor heartbeat process that wakes the Director on its interval (backend-agnostic), and the team-facilitation instructions (poll, ACK, dispatch queued work, health-check, escalate) the Director runs on each monitor tick. Load whenever you are about to spawn or manage CAFleet team members. Foundation layer — load before the cafleet-agent-team-supervision skill."
---

# CAFleet Agent Team Monitoring

Foundation layer for CAFleet Directors. This skill documents the `cafleet monitor` heartbeat that wakes a Director periodically and the team-facilitation instructions it executes on each tick. Load this skill before the `cafleet-agent-team-supervision` skill — supervision builds on the mechanism documented here.

## Placeholder convention

Every command below uses angle-bracket tokens (`<fleet-id>`, `<director-agent-id>`, `<member-agent-id>`) as **placeholders, not shell variables**. Substitute the literal UUID strings printed by `cafleet fleet create` (which returns both the fleet UUID and the root Director's `agent_id` — see the `cafleet` skill § Typical Workflow for the exact output shape) directly into the command. Do **not** introduce shell variables for agent or fleet IDs — `permissions.allow` matches command strings literally, and shell expansion breaks that matching.

**Flag placement**: `--fleet-id` is a global flag (placed **before** the subcommand). `--agent-id` is a per-subcommand option (placed **after** the subcommand name). For example: `cafleet --fleet-id <fleet-id> message poll --agent-id <director-agent-id>`.

- `<fleet-id>` — the fleet UUID printed on line 1 of `cafleet fleet create` text output (or the `fleet_id` field in `--json` output)
- `<director-agent-id>` — the root Director's UUID printed on line 2 of `cafleet fleet create` text output (or `director.agent_id` in `--json` output). `cafleet fleet create` inside a tmux session auto-bootstraps the root Director with its placement row — no separate `cafleet agent register` call is needed to obtain the Director's `agent_id`.
- `<member-agent-id>` — a target member's agent UUID (from `member create` / `member list`)

## The monitor heartbeat

CAFleet members do not act autonomously. The Director drives the team — and the Director needs a way to wake itself up periodically to check inboxes, dispatch queued work, and detect stalls. That heartbeat is supplied by **`cafleet monitor`**, a detached, per-fleet background process external to the coding agent. Because it lives outside the coding agent, the heartbeat is **backend-agnostic** — a root Director on `claude`, `codex`, or `opencode` gets the identical tick. There is no per-backend scheduling asymmetry: the monitor is the one mechanism for every backend.

Start the monitor once, from the Director's pane, before the first `cafleet member create` call:

```bash
cafleet --fleet-id <fleet-id> monitor start
```

`monitor start` detaches and returns control immediately; confirm it with `cafleet --fleet-id <fleet-id> monitor status`. The monitor pings the root Director **unconditionally** on its interval (default 60 s) and pings a member **only** when that member has pending un-acked inbox items. See the `cafleet` skill and the [Monitoring concepts page](https://himkt.github.io/cafleet/concepts/monitoring/) for the full command surface and policy.

**A monitor wake is a bare poll, so the cue to facilitate must come from the skill.** Pinging the Director keystrokes *exactly* `cafleet … message poll` into its pane — that bare poll, on its own, performs only **step 1** below. **Treat every monitor poll-trigger wake as the cue to run the entire 5-step facilitation loop** (poll → ACK → dispatch → health-check → escalate), not to read the inbox and stop. The monitor decides only *when*; this skill defines *what* the Director does on each wake.

## Team-facilitation instructions

On every supervision tick — whether fired by a `cafleet monitor` wake or executed inline within an active turn — the Director runs these five steps in order. The goal is to **facilitate the team in completing tasks**, not merely to detect stalls.

1. **Poll inbox.** `cafleet --fleet-id <fleet-id> message poll --agent-id <director-agent-id>` returns only the un-acked (`input_required`) deliveries; ACKing each one (step 2) consumes it, so the next tick's poll surfaces only what has arrived since.
2. **ACK every message** that requires no further action: `cafleet --fleet-id <fleet-id> message ack --agent-id <director-agent-id> --task-id <task-id>`. Unacknowledged tasks accumulate in the Director's inbox and obscure new arrivals.
3. **Dispatch queued work.** If a member is idle and inputs are available (review comments to route, the next implementation step in a design doc, reviewer feedback waiting at the Drafter, a teammate reply waiting to be acted on), send the instruction immediately via `cafleet message send`. **Do not wait for a fresh "go" from the user** — the user's original authorization persists across ticks; see the `cafleet-agent-team-supervision` skill § Authorization-Scope Guard.
4. **Run the health-check sequence** below for any member that has not reported recent progress.
5. **Escalate** to the user via `AskUserQuestion` after two nudges produce no progress, or whenever a queued action requires a *new* user decision (option choice, risky/remote-visible operation, ambiguous teammate question). Do **not** emit passive-hold messages like `Skipping. Holding for go.` — the tick is a health check, not a permission renewal.

## Health-Check Sequence

Run this sequence once per supervision tick. Order matters — cheapest non-intrusive check first, most invasive last.

| Step | Command | Purpose |
|---|---|---|
| 1 | `cafleet --fleet-id <fleet-id> member list` | Enumerate all live members and their pane status |
| 2 | `cafleet --fleet-id <fleet-id> message poll --agent-id <director-agent-id>` | Check inbox for progress reports or help requests from members |
| 3 | For each member with no recent message: `cafleet --fleet-id <fleet-id> member capture --member-id <member-agent-id>` | Terminal capture fallback — inspect what the member is doing when it has not reported in. If the capture shows an `AskUserQuestion`-style prompt, see Stall Response below for the `member send-input` escape hatch. |
| 4 | Based on findings, `cafleet --fleet-id <fleet-id> message send --agent-id <director-agent-id> --to <member-agent-id> --text "..."` to any stalled or idle member with a specific instruction | Drive the team forward |
| 5 | When all members have reported completion (via messages or visible in terminal output), report to the user: "All deliverables are ready for review." | Signal completion to user |

## Monitor Lifecycle

| Phase | Action |
|---|---|
| Spawn members | Start `cafleet --fleet-id <fleet-id> monitor start` BEFORE the first `cafleet member create` call, so the first tick fires while spawning completes. Confirm with `monitor status`. |
| Run work | The monitor ticks at its configured cadence (default ping interval 60 s); do not intervene unless a wake escalates. Each Director wake is the cue to run the 5-step facilitation loop above. |
| User review | Keep the monitor running during the review cycle — revisions and re-reviews still count as in-progress work. |
| User approves final artifact | Stop the monitor once teardown begins (see Cleanup below). |

**Lifecycle rule:** The monitor MUST stay running from the first `member create` through every phase (research, compilation, review, revision, user approval). At teardown, **stop the monitor BEFORE deleting members** — this is step 1 of the Shutdown Protocol in the `cafleet` skill and is non-negotiable. A monitor that keeps ticking after members are deleted keystrokes polls into tearing-down panes and can race with the member-delete path. Full teardown order: `cafleet --fleet-id <fleet-id> monitor stop` → `cafleet member delete` each member → `cafleet member list` to verify the roster is empty → `cafleet fleet delete <fleet-id>` → `cafleet fleet list` sanity check (`fleet delete` also stops the monitor, so the explicit `monitor stop` is belt-and-suspenders). See the `cafleet` skill → "Shutdown Protocol" for the authoritative procedure.

## Stall Response

When you receive any signal that a member may be stalled (monitor wake, idle notification, user nudge), evaluate using this 2-stage protocol:

> **Bash request blocking case**: When `cafleet message poll` returns a member message asking for a shell command, dispatch via `cafleet member exec "<cmd>"` per the `cafleet` skill § Routing Bash via the Director. Member blocks until the keystroke lands; process requests one at a time, don't skip ahead to other inbox items.

### Stage 1 — Message-based check (`cafleet message poll`)

```bash
cafleet --fleet-id <fleet-id> message poll --agent-id <director-agent-id>
```

`cafleet message poll` returns only the un-acked (`input_required`) deliveries addressed to the Director, newest first. ACKing a delivery consumes it, so a later poll surfaces only what has arrived since the last ACK — there is no last-tick timestamp to track. If the member has sent a progress report or help request via `cafleet message send`, you can act on it immediately without interrupting the member's work. This is non-intrusive and preferred.

### Stage 2 — Terminal capture fallback (`cafleet member capture`)

```bash
cafleet --fleet-id <fleet-id> member capture \
  --member-id <member-agent-id> --lines 120
```

`--lines 120` is the recommended fallback when classifying a 4-option AskUserQuestion frame (matches the recommendation in `skills/cafleet/reference/director.md` § Answer a member's AskUserQuestion prompt; the CLI flag default is `--lines 30`). Re-run with `--lines 200` as a fallback only if the first capture is truncated above the choice-prompt frame (the `1. …`, `2. …`, `3. …`, `4. Type something` rows are not all visible).

If `cafleet message poll` shows no recent messages from the member, fall back to capturing the terminal buffer. This is non-intrusive (read-only inspection that works even when the member is mid-task) and replaces raw `tmux capture-pane`.

If the terminal buffer shows the member paused on a 4-option choice prompt (a list of "1. …", "2. …", "3. …", "4. Type something" rows — the shape that `cafleet member send-input` is validated for), the correct unblock is `cafleet member send-input` — never raw `tmux send-keys` — and the Director MUST delegate the decision to the user BEFORE invoking the wrapper. The Director never picks the `--choice` digit or drafts the `--freetext` body on its own judgment. The full three-beat workflow (capture → user-facing decision prompt with shape-matched options → direct Bash invocation of the resolved `cafleet member send-input`, gated by the coding agent's native per-call permission prompt) and the pane-shapes table live in the cafleet skill's "Answer a member's AskUserQuestion prompt" section — that is canonical; do not duplicate the table here.

> **Note that `AskUserQuestion` should be used in Claude Code.** The "delegate to the user" beat in the workflow above assumes the Director itself runs in Claude Code, where `AskUserQuestion` is the dedicated tool for putting a structured choice in front of the operator. Directors running another coding agent must substitute their own equivalent decision-elicitation surface (or fall back to a plain message to the operator). The 4-option-frame shape that `cafleet member send-input` itself targets is a Claude Code idiom — neither codex nor opencode members render the same frame, so on a codex or opencode member the read-then-respond cadence applies but the `--choice` / `--freetext` keystrokes apply only when the captured buffer matches the validated 4-option layout.

### Escalation

If a member is still unresponsive after 2 nudges via `cafleet message send` AND `cafleet member capture` shows no forward progress in the terminal buffer, escalate to the user.

| Channel | Type | When to use |
|---|---|---|
| `cafleet ... message poll --agent-id <director-agent-id>` | Non-intrusive, message-based | First — check if the member has reported in |
| `cafleet ... member capture --member-id <member-agent-id>` | Non-intrusive, terminal snapshot | Second — when no messages, inspect what the member is doing |
| `cafleet ... message send --agent-id <director-agent-id> --to <member-agent-id> --text "..."` | Interactive, authoritative | Third — send a specific instruction to unstick the member (broker keystrokes a 2-line inline preview of the message into the member's pane via `tmux.send_inline_preview` after persisting; no `cafleet message poll` invocation is in the auto-fire path) |
| `cafleet ... member ping --member-id <member-agent-id>` | Interactive, fixed-action keystroke | Director's pre-approved manual inbox-poll nudge — keystrokes `cafleet message poll` into the member's pane as a manual re-poke for the case where the broker's inline preview was missed. Two use cases: **(a)** a member appears stalled despite a recent `cafleet message send` (the inline preview was missed or the pane was busy when it arrived), or after a long idle window with a queued message still unread; **(b)** post-`member exec` chain — the Director MUST follow every successful `cafleet member exec` with this ping so the member's next turn fires immediately (see the `cafleet` skill § Member Exec for the chain definition; do not duplicate the wording). No positional argument, pre-approved in `permissions.allow`. The only boundary is fleet isolation (cross-fleet `--member-id` → not found; no caller check), shared with `capture` / `send-input` / `exec`. Failures surface as exit 1 (the auto-fire path swallows them silently). |
| `cafleet ... member send-input --member-id <member-agent-id> (--choice N \| --freetext "<text>")` | Interactive, restricted keystroke | `--choice` / `--freetext` answer an `AskUserQuestion`-shaped prompt — delegate the decision to the user via the Director's own `AskUserQuestion` tool call FIRST, then invoke the resolved command via the Director's Bash tool (the coding agent's native per-call permission prompt is the consent surface; never print a fenced `bash` block for the user to paste). See the cafleet skill's "Answer a member's AskUserQuestion prompt" section for the canonical three-beat workflow + pane-shapes table. The only boundary is fleet isolation, shared with `capture`. |
| `cafleet ... member exec --member-id <member-agent-id> "<cmd>"` | Interactive, keystroke dispatch | Director-only shell-dispatch primitive — keystrokes `! <cmd>` + Enter into the member's pane via the coding agent's `!` shortcut (honored by both `claude` and `codex`). Shell-dispatch only — for inbox-poll-only nudges use `member ping`. See ping row for the required follow-up after every successful exec. The only boundary is fleet isolation, shared with `capture` / `send-input`. See the `cafleet` skill § Routing Bash via the Director. |
| `cafleet ... member delete --member-id <member-agent-id> --force` | Interactive, destructive | When `member delete` has already exited 2 and `capture` + `send-input` have failed to unblock the pane — forces an atomic `kill_pane` + deregister + layout rebalance. Never fall back to raw `tmux kill-pane`. |
| Process pending shell-command request from member | Blocking on member side | Dispatch via `cafleet member exec "<cmd>"` per the `cafleet` skill § Routing Bash via the Director. Don't skip past a member's request — the member sits idle until the keystroke lands. |
| Escalate to user | Last resort | After 2 nudges + no progress in terminal |
