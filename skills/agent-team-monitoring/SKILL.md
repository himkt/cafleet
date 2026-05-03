---
name: agent-team-monitoring
description: Active monitoring mechanism for CAFleet Directors. Documents the cron-like loop primitive per backend (Claude Code: CronCreate + /loop; codex: no in-session scheduling, fallback options listed) and the team-facilitation instructions (poll, ACK, dispatch queued work, health-check, escalate). Foundation layer — load before agent-team-supervision.
---

# CAFleet Agent Team Monitoring

Foundation layer for CAFleet Directors. This skill documents the cron-like mechanism a Director uses to wake itself up periodically and the team-facilitation instructions it executes on each tick. Load this skill before `Skill(agent-team-supervision)` — supervision builds on the mechanism documented here.

## Placeholder convention

Every command below uses angle-bracket tokens (`<session-id>`, `<director-agent-id>`, `<member-agent-id>`) as **placeholders, not shell variables**. Substitute the literal UUID strings printed by `cafleet session create` (which returns both the session UUID and the root Director's `agent_id` — see `Skill(cafleet)` § Typical Workflow for the exact output shape) directly into the command. Do **not** introduce shell variables for agent or session IDs — `permissions.allow` matches command strings literally, and shell expansion breaks that matching.

**Flag placement**: `--session-id` is a global flag (placed **before** the subcommand). `--agent-id` is a per-subcommand option (placed **after** the subcommand name). For example: `cafleet --session-id <session-id> message poll --agent-id <director-agent-id>`.

- `<session-id>` — the session UUID printed on line 1 of `cafleet session create` text output (or the `session_id` field in `--json` output)
- `<director-agent-id>` — the root Director's UUID printed on line 2 of `cafleet session create` text output (or `director.agent_id` in `--json` output). `cafleet session create` inside a tmux session auto-bootstraps the root Director with its placement row — no separate `cafleet agent register` call is needed to obtain the Director's `agent_id`.
- `<member-agent-id>` — a target member's agent UUID (from `member create` / `member list`)

## Mechanism by backend

CAFleet members do not act autonomously. The Director drives the team — and the Director needs a way to wake itself up periodically to check inboxes, dispatch queued work, and detect stalls. The mechanism for this differs between coding-agent backends.

### Claude Code Director (default)

Claude Code's harness exposes two in-session scheduling tools the Director can call:

| Tool | Use |
|---|---|
| `CronCreate` | Schedule a recurring prompt to fire at a cron interval. Used to set up the active `/loop` monitor (typically 1-minute cadence). |
| `ScheduleWakeup` | Self-pacing dynamic-mode wake-up — used by `/loop` when the Director picks its own next-tick delay. |

The `/loop` Prompt Template (§ `/loop` Prompt Template below) is the canonical setup — it uses `CronCreate` under the hood. **The loop is mandatory before any `cafleet member create` call** when the Director runs under Claude Code.

### Codex Director

Codex CLI exposes **no equivalent in-session scheduling primitive** (confirmed via survey of <https://developers.openai.com/codex/cli/features> as of 2026-05). There is no `CronCreate`, no `ScheduleWakeup`, no harness-level tool the model can invoke from inside a running session to schedule a future tick. Codex Automations exist but are app-only and cannot be triggered from the local CLI.

This means: **a codex root Director cannot run the active `/loop` monitor.** The mechanism is unavailable.

#### Recommendation

When active supervision of a CAFleet member team is required, **use `cafleet session create --coding-agent claude`** for the root Director. Codex members are fully supported via `cafleet member create --coding-agent codex` — only the Director needs the cron mechanism. A mixed-backend team (claude Director + codex members) is the canonical configuration for active-supervision workloads.

#### Fallback options when codex must be the Director

If a codex root Director is required (e.g. operator preference, codex-specific workflow, no claude binary available), one of the following fallbacks must be in place. Active supervision **without** one of these fallbacks is not supported.

| Fallback | Mechanism | Operational cost |
|---|---|---|
| **Out-of-band cron driver** | An OS-level scheduler (`cron(8)`, systemd timer, `watch -n 60 …`) keystrokes the supervision-tick prompt into the codex Director's tmux pane via `tmux send-keys`. | Operator must set up + tear down the timer; not visible inside the session. |
| **MCP scheduling server** | Codex CLI supports MCP servers. A custom MCP server can expose a scheduling tool the codex Director invokes inline. | Requires writing or installing an MCP server; configuration lives in `~/.codex/config.toml`. |
| **User-driven nudges** | The user types a tick prompt at intervals (e.g. "tick" every minute). | Manual, error-prone, doesn't scale beyond short sessions. |
| **No active monitor — synchronous in-turn facilitation only** | The Director performs all health checks + dispatch within each of its own active turns; no scheduled wake-up. The team only progresses while the Director has an active turn. | Acceptable only for short, fully-Director-driven workflows (no long-running parallel members). The Director's idle window is the team's idle window. |

The fallback in use must be documented in the session's launch instructions. The supervision skill's Authorization-Scope Guard applies regardless of which fallback is in use.

## Team-facilitation instructions

On every supervision tick — whether fired by `/loop` (Claude Code) or by a fallback (codex), or executed inline within an active turn — the Director runs these five steps in order. The goal is to **facilitate the team in completing tasks**, not merely to detect stalls.

1. **Poll inbox.** `cafleet --session-id <session-id> message poll --agent-id <director-agent-id>` (optionally with `--since <iso8601>` to filter to messages received since the last tick).
2. **ACK every message** that requires no further action: `cafleet --session-id <session-id> message ack --agent-id <director-agent-id> --task-id <task-id>`. Unacknowledged tasks accumulate in the Director's inbox and obscure new arrivals.
3. **Dispatch queued work.** If a member is idle and inputs are available (review comments to route, the next implementation step in a design doc, reviewer feedback waiting at the Drafter, a teammate reply waiting to be acted on), send the instruction immediately via `cafleet message send`. **Do not wait for a fresh "go" from the user** — the user's original authorization persists across ticks; see `Skill(agent-team-supervision)` § Authorization-Scope Guard.
4. **Run the health-check sequence** below for any member that has not reported recent progress.
5. **Escalate** to the user via `AskUserQuestion` after two nudges produce no progress, or whenever a queued action requires a *new* user decision (option choice, risky/remote-visible operation, ambiguous teammate question). Do **not** emit passive-hold messages like `Skipping. Holding for go.` — the tick is a health check, not a permission renewal.

## Health-Check Sequence

Run this sequence once per supervision tick. Order matters — cheapest non-intrusive check first, most invasive last.

| Step | Command | Purpose |
|---|---|---|
| 1 | `cafleet --session-id <session-id> member list --agent-id <director-agent-id>` | Enumerate all live members and their pane status |
| 2 | `cafleet --session-id <session-id> message poll --agent-id <director-agent-id>` | Check inbox for progress reports or help requests from members |
| 3 | For each member with no recent message: `cafleet --session-id <session-id> member capture --agent-id <director-agent-id> --member-id <member-agent-id>` | Terminal capture fallback — inspect what the member is doing when it has not reported in. If the capture shows an `AskUserQuestion`-style prompt, see Stall Response below for the `member send-input` escape hatch. |
| 4 | Based on findings, `cafleet --session-id <session-id> message send --agent-id <director-agent-id> --to <member-agent-id> --text "..."` to any stalled or idle member with a specific instruction | Drive the team forward |
| 5 | When all members have reported completion (via messages or visible in terminal output), report to the user: "All deliverables are ready for review." | Signal completion to user |

## `/loop` Prompt Template (Claude Code only)

> **Claude Code-specific.** This template depends on `CronCreate` / `ScheduleWakeup`, which are Claude Code harness tools. Codex Directors cannot use this template — see § Mechanism by backend → Codex Director for fallback options.

Substitute the literal UUIDs into every `<session-id>`, `<director-agent-id>`, and `<member-agent-id>` placeholder before passing the prompt to `/loop`. The prompt must contain literal UUIDs, **not** shell variables — the `permissions.allow` matcher only allows literal command strings. Remember: `--session-id` goes before the subcommand, `--agent-id` goes after.

```
Monitor team health (interval: 1 minute). For each member spawned via `cafleet member create`:

1. Run `cafleet --session-id <session-id> --json member list --agent-id <director-agent-id>` to get all members.
2. Run `cafleet --session-id <session-id> --json message poll --agent-id <director-agent-id> --since "<ISO 8601 timestamp of last check, with +00:00 suffix — not Z>"` to check for incoming messages. ACK any progress reports.
3. For each member that has NOT sent a message since last check, run `cafleet --session-id <session-id> member capture --agent-id <director-agent-id> --member-id <member-agent-id> --lines 200` to inspect their terminal.
4. If a member's terminal shows no forward progress since last check, send them a specific instruction via `cafleet --session-id <session-id> message send --agent-id <director-agent-id> --to <member-agent-id> --text "Report your progress now. If blocked, state what is blocking you."` (the broker auto-fires a `cafleet message poll` keystroke into the member's pane to pick up the message).
5. If a member appears stalled despite a recent `message send` (the broker auto-fire was missed or arrived while the pane was busy), or after a long idle window with a queued message still unread, fire `cafleet --session-id <session-id> member ping --agent-id <director-agent-id> --member-id <member-agent-id>` to manually inject the same poll keystroke. This is the dedicated re-poke primitive — pre-approved in `permissions.allow`, no positional argument. Do NOT use `cafleet member exec` for this purpose; that is for shell dispatch only.
6. If all members have reported completion (via messages or visible in terminal output), report to the user: "All deliverables are ready for review."
7. If a member has been nudged 2 times with no progress, escalate to the user.
```

## Loop Lifecycle

| Phase | Action |
|---|---|
| Spawn members | Start the `/loop` (Claude Code) or fallback driver (codex) BEFORE the first `cafleet member create` call, so the first tick fires while spawning completes. |
| Run work | Tick at the configured cadence (1 minute is the `CronCreate` floor); do not intervene unless a tick escalates. |
| User review | Keep the loop alive during the review cycle — revisions and re-reviews still count as in-progress work. |
| User approves final artifact | The loop terminates itself after teardown begins (see Cleanup below). |

**Lifecycle rule:** The loop MUST stay active from the first `member create` through every phase (research, compilation, review, revision, user approval). At teardown, **stop the loop BEFORE deleting members** — this is step 1 of the Shutdown Protocol in `Skill(cafleet)` and is non-negotiable. A loop that keeps firing after members are deleted spams `member list` / `message poll` against a tearing-down session, can race with the member-delete path, and (most visibly) leaks cron output into the operator's terminal after the team is ostensibly gone. Full teardown order: stop every `/loop` cron (or fallback driver) → `cafleet member delete` each member → `cafleet member list` to verify the roster is empty → `cafleet session delete <session-id>` → `cafleet session list` sanity check. See `Skill(cafleet)` → "Shutdown Protocol" for the authoritative procedure.

## Stall Response — 2-Stage Health Check

When you receive any signal that a member may be stalled (loop check, idle notification, user nudge), evaluate using this 2-stage protocol:

> **Bash request blocking case**: When `cafleet message poll` returns a member message asking for a shell command, dispatch via `cafleet member exec "<cmd>"` per `Skill(cafleet)` § Routing Bash via the Director. Member blocks until the keystroke lands; process requests one at a time, don't skip ahead to other inbox items.

### Stage 1 — Message-based check (`cafleet message poll`)

```bash
cafleet --session-id <session-id> message poll --agent-id <director-agent-id> \
  --since "2026-04-12T10:00:00+00:00"
```

The `--since` flag accepts an ISO 8601 timestamp. The broker stores `status_timestamp` via `datetime.now(UTC).isoformat()`, which renders as `YYYY-MM-DDTHH:MM:SS.ffffff+00:00` (microsecond precision, `+00:00` suffix — **not** `Z`), and the `--since` filter is applied as a raw SQLite TEXT comparison, so pass timestamps in the same `+00:00` form to get correct ordering. If the member has sent a progress report or help request via `cafleet message send`, you can act on it immediately without interrupting the member's work. This is non-intrusive and preferred.

### Stage 2 — Terminal capture fallback (`cafleet member capture`)

```bash
cafleet --session-id <session-id> member capture --agent-id <director-agent-id> \
  --member-id <member-agent-id> --lines 120
```

`--lines 120` is the recommended default (matches the cafleet skill). Re-run with `--lines 200` as a fallback only if the first capture is truncated above the choice-prompt frame (the `1. …`, `2. …`, `3. …`, `4. Type something` rows are not all visible).

If `cafleet message poll` shows no recent messages from the member, fall back to capturing the terminal buffer. This is non-intrusive (read-only inspection that works even when the member is mid-task) and replaces raw `tmux capture-pane`.

If the terminal buffer shows the member paused on a 4-option choice prompt (a list of "1. …", "2. …", "3. …", "4. Type something" rows — the shape that `cafleet member send-input` is validated for), the correct unblock is `cafleet member send-input` — never raw `tmux send-keys` — and the Director MUST delegate the decision to the user BEFORE invoking the wrapper. The Director never picks the `--choice` digit or drafts the `--freetext` body on its own judgment. The full three-beat workflow (capture → user-facing decision prompt with shape-matched options → direct Bash invocation of the resolved `cafleet member send-input`, gated by the coding agent's native per-call permission prompt) and the pane-shapes table live in the cafleet skill's "Answer a member's AskUserQuestion prompt" section — that is canonical; do not duplicate the table here.

> **Note that `AskUserQuestion` should be used in Claude Code.** The "delegate to the user" beat in the workflow above assumes the Director itself runs in Claude Code, where `AskUserQuestion` is the dedicated tool for putting a structured choice in front of the operator. Directors running another coding agent must substitute their own equivalent decision-elicitation surface (or fall back to a plain message to the operator). The 4-option-frame shape that `cafleet member send-input` itself targets is a Claude Code idiom — codex members do not render the same frame, so on a codex member the read-then-respond cadence applies but the `--choice` / `--freetext` keystrokes apply only when the captured buffer matches the validated 4-option layout.

### Escalation

If a member is still unresponsive after 2 nudges via `cafleet message send` AND `cafleet member capture` shows no forward progress in the terminal buffer, escalate to the user.

| Channel | Type | When to use |
|---|---|---|
| `cafleet ... message poll --agent-id <director-agent-id>` | Non-intrusive, message-based | First — check if the member has reported in |
| `cafleet ... member capture --agent-id <director-agent-id>` | Non-intrusive, terminal snapshot | Second — when no messages, inspect what the member is doing |
| `cafleet ... message send --agent-id <director-agent-id> --to <member-agent-id> --text "..."` | Interactive, authoritative | Third — send a specific instruction to unstick the member (broker auto-fires a `cafleet message poll` keystroke into the member's pane after persisting the message) |
| `cafleet ... member ping --agent-id <director-agent-id> --member-id <member-agent-id>` | Interactive, fixed-action keystroke | Director's pre-approved manual inbox-poll nudge — fires the same `cafleet message poll` keystroke the broker auto-fires, but as a manual entry-point. Two use cases: **(a)** a member appears stalled despite a recent `cafleet message send` (the broker auto-fire was missed or the pane was busy when it arrived), or after a long idle window with a queued message still unread; **(b)** post-`member exec` chain — the Director MUST follow every successful `cafleet member exec` with this ping so the member's next turn fires immediately (see `Skill(cafleet)` § Member Exec for the chain definition; do not duplicate the wording). No positional argument, pre-approved in `permissions.allow`. Same authorization boundary as `capture` / `send-input` / `exec`. Failures surface as exit 1 (the auto-fire path swallows them silently). |
| `cafleet ... member send-input --agent-id <director-agent-id> --member-id <member-agent-id> (--choice N \| --freetext "<text>")` | Interactive, restricted keystroke | `--choice` / `--freetext` answer an `AskUserQuestion`-shaped prompt — delegate the decision to the user via the Director's own `AskUserQuestion` tool call FIRST, then invoke the resolved command via the Director's Bash tool (the coding agent's native per-call permission prompt is the consent surface; never print a fenced `bash` block for the user to paste). See the cafleet skill's "Answer a member's AskUserQuestion prompt" section for the canonical three-beat workflow + pane-shapes table. Same authorization boundary as `capture`. |
| `cafleet ... member exec --agent-id <director-agent-id> --member-id <member-agent-id> "<cmd>"` | Interactive, keystroke dispatch | Director-only shell-dispatch primitive — keystrokes `! <cmd>` + Enter into the member's pane via the coding agent's `!` shortcut (honored by both `claude` and `codex`). Shell-dispatch only — for inbox-poll-only nudges use `member ping`. See ping row for the required follow-up after every successful exec. Same authorization boundary as `capture` / `send-input`. See `Skill(cafleet)` § Routing Bash via the Director. |
| `cafleet ... member delete --agent-id <director-agent-id> --member-id <member-agent-id> --force` | Interactive, destructive | When `member delete` has already exited 2 and `capture` + `send-input` have failed to unblock the pane — forces an atomic `kill_pane` + deregister + layout rebalance. Never fall back to raw `tmux kill-pane`. |
| Process pending shell-command request from member | Blocking on member side | Dispatch via `cafleet member exec "<cmd>"` per `Skill(cafleet)` § Routing Bash via the Director. Don't skip past a member's request — the member sits idle until the keystroke lands. |
| Escalate to user | Last resort | After 2 nudges + no progress in terminal |
