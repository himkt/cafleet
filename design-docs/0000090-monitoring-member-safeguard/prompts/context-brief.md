# Context Brief — Design 0000090: Safe Director-only monitoring with a dedicated monitoring member

This is the user's request and the LOCKED design decisions for the design doc you (the Drafter) must write. Read this FIRST, then read the affected source files for grounding before drafting.

## Problem (verified in code)

`cafleet monitor`'s ping keystroke is, in `cafleet/src/cafleet/multiplexer/tmux.py` (`_send_literal_then_enter`):
`tmux send-keys -t <pane> -l <text>` → `sleep 0.12s` → `tmux send-keys -t <pane> Enter`.

There is **NO `Esc`** before the keystroke. If the target pane is a coding agent sitting on a permission-approval prompt, the agent ignores the literal text but the trailing bare **`Enter` CONFIRMS the pending permission prompt** → the permission guard is bypassed. This is a dangerous safety hole.

Separately, the monitor loop (`cafleet/src/cafleet/monitor/loop.py`, default tick 5s / per-agent ping interval 60s) unconditionally pings **every** enrolled agent: the Director via `send_poll_trigger` (bare `cafleet … message poll` keystroke) and **all members** via `send_resume_trigger` (a resume-nudge text keystroke).

## New design — LOCKED decisions (confirmed with the user; do NOT re-litigate these)

### 1. Add a dedicated monitoring member

A coding-agent member, spawned via `cafleet member create`, with a monitoring-only role/prompt. It runs `cafleet monitor start` as a background task in its own pane. On each wake it:

1. Captures the Director's pane via `cafleet member capture` and uses its **own LLM judgment** to classify the Director as **active** vs **idle ("doing nothing")**.
2. If the Director is **ACTIVE** → do nothing (the Director is managing the team).
3. If the Director is **IDLE** → assess the full picture: check the inbox, the Director's own current task, and members' progress (capture member panes, read-only). Because **THE DIRECTOR IS RESPONSIBLE FOR MANAGING THE ENTIRE TASK**, the monitoring member's corrective action is to **RE-ENGAGE THE DIRECTOR** with an `Esc`-safeguarded nudge summarizing what needs attention (un-ACKed inbox items, stalled members) so the Director resumes managing. The monitoring member does **NOT** issue task instructions to members directly — it routes all member-driving back through the Director.

### 2. Simplify `cafleet monitor` (the mechanical loop)

- **DROP** the unconditional pinging of ordinary members. The `send_resume_trigger` branch for ordinary members is removed **entirely** — apply the project removal rule (`~/.claude/.../removal.md`): delete every mention, no deprecation notices, the repo must read as if blind member-pinging never existed.
- Only the **Director** and the **monitoring member** are enrolled / pinged by the loop:
  - **Director** → `Esc`-safeguarded `cafleet … message poll` keystroke (its facilitation heartbeat).
  - **Monitoring member** → `Esc`-safeguarded wake nudge so it runs its capture-and-assess routine each tick.
  - **Ordinary members** → NOT enrolled, never pinged by the loop.
- **ESC SAFEGUARD**: before every keystroke ping (`send-keys -l <text>` → `Enter`), send an `Esc` keystroke **FIRST** to dismiss any pending permission prompt, so the trailing `Enter` can never confirm a guarded action. Apply to ALL monitor keystroke pings (Director poll + monitoring-member wake).

### Topology (locked)

Monitor loop → Director (`Esc` + poll) AND → monitoring member (`Esc` + wake). Monitoring member → re-engages the Director on demand when it judges the Director idle / the team stalled. The monitoring member never keystrokes ordinary members with task instructions.

## Affected surfaces (documentation-first per `.claude/rules/design-doc-numbering.md`; ALL updated in the same cycle)

- **Code**: `cafleet/src/cafleet/monitor/loop.py` (drop ordinary-member ping; restrict targets to Director + monitoring member), `cafleet/src/cafleet/multiplexer/tmux.py` (`_send_literal_then_enter` → `Esc`-first; the `send_poll_trigger` / `send_resume_trigger` / new wake-trigger helpers), `cafleet/src/cafleet/broker/monitor.py` (`list_monitor_targets` enrollment + a way to identify the monitoring member, e.g. a role flag), `cafleet/src/cafleet/cli/member.py` & `cafleet/src/cafleet/cli/monitor.py` (spawn the monitoring member / its role).
- **Tests**: `cafleet/tests/monitor/test_loop.py`, `cafleet/tests/monitor/test_should_ping.py`, `cafleet/tests/cli/test_monitor.py`, `cafleet/tests/broker/test_monitor.py`, `cafleet/tests/cli/test_member_ping.py`.
- **Docs**: `docs/concepts/monitoring.md`, `README.md`.
- **Skills**: `skills/cafleet-agent-team-monitoring/SKILL.md`, `skills/cafleet-agent-team-supervision/SKILL.md`, `skills/cafleet/SKILL.md` (member-ping / monitor references), plus `.claude/rules/bash-tool.md` which currently documents the `member ping` "send text + Enter" mechanism.

## Open implementation questions for the doc to RESOLVE (these are details, NOT a re-opening of the locked decisions above)

1. **How the loop identifies the monitoring member** — a role/kind column on the agent/placement, vs. enrollment-only-of-{Director, monitoring member} in `monitor_config`. Pick one and justify; cover the migration if a schema/enrollment change is needed.
2. **Bootstrap** — the monitoring member is spawned as the FIRST member (before the others); how it learns the Director's `agent_id` and pane (e.g. from `cafleet member list` / fleet info); who launches `cafleet monitor start` now (it moves from the Director's own background task into the monitoring member's pane).
3. **Exact `Esc` keystroke ordering and count** — how many `Esc` presses, and the precise `send-keys` sequence (e.g. `send-keys Escape` → small delay → `send-keys -l <text>` → delay → `send-keys Enter`). Account for coding agents that need a brief settle after `Esc`.
4. **Lifecycle migration** — how `cafleet-agent-team-monitoring` and `cafleet-agent-team-supervision` skill lifecycles change now that `monitor start` runs inside the monitoring member rather than as the Director's own background task; teardown ordering (the monitoring member is a member, so it is deleted during shutdown — make sure the monitor stops first).

## Notes

- This is design number **0000090**; the highest existing is 0000089.
- Follow the project's documentation-first implementation ordering: docs/concepts + README + SKILLs first in the implementation steps, then code, then tests.
- The implementation section MUST be broken into actionable, ordered steps with explicit verification (the project uses a Phase A/B/C/D style — consult the cafleet-design-doc template/guidelines you loaded).
