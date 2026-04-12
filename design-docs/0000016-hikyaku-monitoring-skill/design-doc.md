# Hikyaku Monitoring Skill

**Status**: Approved
**Progress**: 8/8 tasks complete
**Last Updated**: 2026-04-12

## Overview

Create a hikyaku-native agent team supervision skill (`.claude/skills/hikyaku-monitoring/SKILL.md`) that replaces the generic `agent-team-supervision` skill for projects using Hikyaku. The new skill defines the Director's monitoring obligations using hikyaku-native commands (`hikyaku poll`, `hikyaku member list`, `hikyaku member capture`) and provides a ready-to-use `/loop` prompt template with a 2-stage health check protocol.

## Success Criteria

- [ ] `.claude/skills/hikyaku-monitoring/SKILL.md` exists and contains a complete behavioral protocol (Core Principle, Monitoring Mandate, Spawn Protocol, Stall Response) plus a `/loop` prompt template
- [ ] The 2-stage health check is documented: `hikyaku poll` (message-based) → `hikyaku member capture` (terminal capture fallback)
- [ ] `.claude/skills/hikyaku/SKILL.md` Monitoring mandate section is updated to reference `Skill(hikyaku-monitoring)` instead of `Skill(agent-team-supervision)`
- [ ] The hikyaku SKILL.md "Monitoring mandate" section references the new skill without duplicating its content
- [ ] `CLAUDE.md` (both root and `.claude/CLAUDE.md`) list the new skill in their skills sections
- [ ] `ARCHITECTURE.md` Member Lifecycle section documents the supervision skill
- [ ] `README.md` mentions the monitoring skill in the relevant features or documentation section
- [ ] No raw `tmux` commands appear in the new skill — all tmux interaction goes through `hikyaku member` subcommands

---

## Background

### Current state

The Director's monitoring obligations are defined by the generic `agent-team-supervision` skill at `~/.claude/skills/agent-team-supervision/SKILL.md`. This skill was written before Hikyaku's `member` subcommands existed, so it instructs the Director to use raw `tmux capture-pane` for pane inspection and `SendMessage` as the sole inter-agent communication channel. The hikyaku SKILL.md Monitoring mandate section currently says:

```
the Director MUST load `Skill(agent-team-supervision)` and start a `/loop` monitor as that skill instructs
```

### Problems

1. **Command mismatch.** The generic skill documents `tmux capture-pane -p -t <pane> -S -<lines>` and `tmux ls` for stall inspection. Projects using Hikyaku should use `hikyaku member capture` instead, which enforces cross-Director boundaries and looks up pane IDs from the placement table.

2. **Missing message-based health check.** The generic skill only has two channels: `SendMessage` (interactive) and `tmux capture-pane` (read-only fallback). Hikyaku adds a third channel — `hikyaku poll` — that lets the Director check its own inbox for progress reports from members without requiring interactive back-and-forth. The generic skill has no concept of this.

3. **No `/loop` prompt template.** The generic skill says "set up a `/loop` monitor" and lists what the loop should do, but does not provide a concrete loop prompt using hikyaku commands. Every Director must reinvent the same loop body.

4. **Indirection.** Directors in hikyaku projects must load two skills (`hikyaku` + `agent-team-supervision`) and mentally merge their instructions. A single hikyaku-native skill eliminates this cognitive overhead.

---

## Specification

### New file: `.claude/skills/hikyaku-monitoring/SKILL.md`

The skill file follows the same frontmatter + markdown structure as existing skills. It is organized into five sections.

#### Frontmatter

```yaml
---
name: hikyaku-monitoring
description: Mandatory supervision protocol for a Director managing member agents via Hikyaku. Defines monitoring loop, spawn protocol, and stall response using hikyaku-native commands.
---
```

#### Section 1: Core Principle

Identical in spirit to the generic skill but explicitly references Hikyaku members:

> **You are the instruction giver. If you stop giving instructions, the entire team stops.**
>
> Members spawned via `hikyaku member create` do not act autonomously. They respond to your messages. If you are not actively monitoring and instructing, work halts silently.

#### Section 2: Monitoring Mandate

Instructs the Director to start a `/loop` monitor with a **3-minute interval** before spawning any member. The loop uses hikyaku-native commands exclusively.

**`$DIRECTOR_ID` convention:** Throughout this skill, `$DIRECTOR_ID` is a placeholder for the agent ID the Director received from `hikyaku register`. The Director must substitute its actual agent ID. This is not a pre-set environment variable -- it is the value captured from the registration output.

| Step | Command | Purpose |
|---|---|---|
| 1 | `hikyaku member list --agent-id $DIRECTOR_ID` | Enumerate all live members and their pane status |
| 2 | `hikyaku poll --agent-id $DIRECTOR_ID` | Check inbox for progress reports or help requests from members |
| 3 | For each member with no recent message: `hikyaku member capture --agent-id $DIRECTOR_ID --member-id $MID` | Terminal capture fallback -- inspect what the member is doing when it has not reported in |
| 4 | Based on findings, `SendMessage` to any stalled or idle member with a specific instruction | Drive the team forward |
| 5 | When all members have reported completion (via messages or visible in terminal output), report to the user: "All deliverables are ready for review." | Signal completion to user |

**Lifecycle rule:** The loop MUST stay active from the first `member create` until the final shutdown (`CronDelete` only in the cleanup step). It must run through all phases: research, compilation, review, revision, user approval.

#### Section 3: Spawn Protocol

Every time the Director spawns a member:

1. Ensure the `/loop` monitor is already running (set it up if not)
2. Spawn the member via `hikyaku member create --agent-id $DIRECTOR_ID --name <name> --description <desc> -- "<prompt>"`
3. Verify the member is active by checking `hikyaku member list` output shows the new member with a non-null `pane_id`

Never spawn members without an active monitor. Never cancel the monitor until all work is fully complete and the team is being shut down.

#### Section 4: Stall Response — 2-Stage Health Check

When the Director receives any signal that a member may be stalled (loop check, idle notification, user nudge), it evaluates using a 2-stage protocol. This is the key enhancement over the generic skill:

**Stage 1 — Message-based check (`hikyaku poll`):**

```bash
hikyaku poll --agent-id $DIRECTOR_ID --since "2026-04-12T10:00:00Z"
```

The `--since` flag accepts an ISO 8601 timestamp. If the member has sent a progress report or help request via `hikyaku send`, the Director can act on it immediately without interrupting the member's work. This is non-intrusive and preferred.

**Stage 2 — Terminal capture fallback (`hikyaku member capture`):**

```bash
hikyaku member capture --agent-id $DIRECTOR_ID --member-id $MEMBER_ID --lines 200
```

If `hikyaku poll` shows no recent messages from the member, fall back to capturing the terminal buffer. This is non-intrusive (read-only inspection that works even when the member is mid-task) and replaces the generic skill's raw `tmux capture-pane`.

**Escalation:** If a member is still unresponsive after 2 nudges via `SendMessage` AND `hikyaku member capture` shows no forward progress in the terminal buffer, escalate to the user.

| Channel | Type | When to use |
|---|---|---|
| `hikyaku poll` | Non-intrusive, message-based | First — check if the member has reported in |
| `hikyaku member capture` | Non-intrusive, terminal snapshot | Second — when no messages, inspect what the member is doing |
| `SendMessage` | Interactive, authoritative | Third — send a specific instruction to unstick the member |
| Escalate to user | Last resort | After 2 nudges + no progress in terminal |

#### Section 5: `/loop` Prompt Template

A ready-to-use loop prompt that Directors can pass to `Skill(loop)` or `/loop`. The Director must replace `<director-agent-id>` with the actual agent ID obtained from `hikyaku register` output. The template uses concrete hikyaku commands:

```
Monitor team health (interval: 3 minutes). For each member spawned via hikyaku member create:

1. Run `hikyaku --json member list --agent-id <director-agent-id>` to get all members.
2. Run `hikyaku --json poll --agent-id <director-agent-id> --since "<ISO 8601 timestamp of last check>"` to check for incoming messages. ACK any progress reports.
3. For each member that has NOT sent a message since last check, run `hikyaku member capture --agent-id <director-agent-id> --member-id <member_id> --lines 200` to inspect their terminal.
4. If a member's terminal shows no forward progress since last check, send them a specific instruction via SendMessage: "Report your progress now. If blocked, state what is blocking you."
5. If all members have reported completion (via messages or visible in terminal output), report to the user: "All deliverables are ready for review."
6. If a member has been nudged 2 times with no progress, escalate to the user.
```

### Changes to existing files

#### `.claude/skills/hikyaku/SKILL.md`

**Change 1** — Update the Monitoring mandate section:

Before:
```markdown
Before spawning **any** member, the Director MUST load `Skill(agent-team-supervision)` and start a `/loop` monitor as that skill instructs.
```

After:
```markdown
Before spawning **any** member, the Director MUST load `Skill(hikyaku-monitoring)` and start a `/loop` monitor as that skill instructs.
```

**Change 2** — Update the paragraph following the monitoring mandate to reference the new skill's 2-stage health check instead of just `hikyaku member capture`:

Before:
```markdown
To inspect a stalled member, use `hikyaku member capture`:
```

After:
```markdown
To inspect a stalled member, follow the 2-stage health check in `Skill(hikyaku-monitoring)`: first check `hikyaku poll` for messages, then fall back to `hikyaku member capture`:
```

**Change 3** — Remove or update the "Note on external `agent-team-supervision` skill" at the end of the Member Capture section:

Before:
```markdown
**Note on external `agent-team-supervision` skill**: The external `agent-team-supervision` skill (user-level, outside this repo) still documents raw `tmux capture-pane`. For projects using Hikyaku, prefer `hikyaku member capture` as it enforces the cross-Director boundary. The external skill alignment is tracked as follow-up work.
```

After:
```markdown
**Note**: Projects using Hikyaku use `Skill(hikyaku-monitoring)` instead of the generic `agent-team-supervision` skill. The hikyaku-monitoring skill uses `hikyaku member capture` exclusively (no raw `tmux capture-pane`), enforcing the cross-Director boundary.
```

#### `CLAUDE.md` (root)

Add to the Skills section:

```markdown
- `/hikyaku-monitoring` — Mandatory supervision protocol for a Director managing member agents via Hikyaku. Defines monitoring loop, spawn protocol, and stall response.
```

#### `.claude/CLAUDE.md`

Add to the Project Skills section:

```markdown
- `/hikyaku-monitoring` — Mandatory supervision protocol for a Director managing member agents via Hikyaku. Defines monitoring loop, spawn protocol, and stall response.
```

#### `ARCHITECTURE.md`

Add a paragraph in the Member Lifecycle section (after the existing "Commands" paragraph):

> **Supervision skill**: The Director's monitoring obligations are defined in `.claude/skills/hikyaku-monitoring/SKILL.md`. This skill must be loaded (`Skill(hikyaku-monitoring)`) before spawning any members. It provides a 2-stage health check protocol (message poll then terminal capture) and a ready-to-use `/loop` prompt template.

The Component Layout table is not updated because it lists source code files only; SKILL.md files are documentation artifacts, not runtime components.

#### `README.md`

Add a bullet to the Features section:

```markdown
- **Director Monitoring Skill** -- `.claude/skills/hikyaku-monitoring/SKILL.md` defines mandatory supervision protocol for Directors: 2-stage health check (poll inbox → capture terminal), spawn protocol, stall response, and a `/loop` prompt template
```

### Coexistence with `agent-team-supervision`

The generic `agent-team-supervision` skill (`~/.claude/skills/agent-team-supervision/SKILL.md`) remains unchanged -- it is a user-level file outside this repo and out of scope.

| Scenario | Behavior |
|---|---|
| Director loads only `Skill(hikyaku-monitoring)` | Correct. All supervision uses hikyaku-native commands. |
| Director loads only `Skill(agent-team-supervision)` | Works but suboptimal. Raw `tmux capture-pane` bypasses cross-Director boundary enforcement. |
| Director loads both | Harmless but redundant. `hikyaku-monitoring` supersedes every instruction in `agent-team-supervision`. The hikyaku SKILL.md will explicitly state "load `Skill(hikyaku-monitoring)` instead of `Skill(agent-team-supervision)`" to prevent double-loading. |

The updated hikyaku SKILL.md Monitoring mandate section makes the replacement explicit: "the Director MUST load `Skill(hikyaku-monitoring)`". This wording replaces the previous `Skill(agent-team-supervision)` reference, so Directors following the hikyaku skill will never load the generic one.

### What is NOT changed

- The generic `agent-team-supervision` skill (`~/.claude/skills/agent-team-supervision/SKILL.md`) -- out of scope, user-level file outside this repo. Non-hikyaku projects continue to use it as-is.
- The hikyaku SKILL.md command reference -- already complete and correct per user instruction
- The hikyaku CLI implementation (`cli.py`, `tmux.py`, `api.py`) -- no code changes needed
- The registry server -- no backend changes

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

### Step 1: Documentation updates

- [x] Update `ARCHITECTURE.md` — add supervision skill paragraph to Member Lifecycle section <!-- completed: 2026-04-12T08:45 -->
- [x] Update `README.md` — add Director Monitoring Skill to Features section <!-- completed: 2026-04-12T08:45 -->
- [x] Update `CLAUDE.md` (root) — add `/hikyaku-monitoring` to Skills section <!-- completed: 2026-04-12T08:45 -->
- [x] Update `.claude/CLAUDE.md` — add `/hikyaku-monitoring` to Project Skills section <!-- completed: 2026-04-12T08:45 -->

### Step 2: Create the monitoring skill

- [x] Create `.claude/skills/hikyaku-monitoring/SKILL.md` with full content (frontmatter, Core Principle, Monitoring Mandate, Spawn Protocol, Stall Response with 2-stage health check, `/loop` prompt template) <!-- completed: 2026-04-12T08:48 -->

### Step 3: Update hikyaku skill references

- [x] Update `.claude/skills/hikyaku/SKILL.md` — change `Skill(agent-team-supervision)` to `Skill(hikyaku-monitoring)` in Monitoring mandate section <!-- completed: 2026-04-12T08:50 -->
- [x] Update `.claude/skills/hikyaku/SKILL.md` — update stall inspection paragraph to reference 2-stage health check <!-- completed: 2026-04-12T08:50 -->
- [x] Update `.claude/skills/hikyaku/SKILL.md` — replace the "Note on external `agent-team-supervision` skill" with updated note about `hikyaku-monitoring` <!-- completed: 2026-04-12T08:50 -->

---

## Changelog (spec revisions only, not implementation progress)

| Date | Changes |
|------|---------|
| 2026-04-12 | Initial draft |
| 2026-04-12 | Rev 1: Add 3-min interval, clarify $DIRECTOR_ID placeholder, fix team-lead ambiguity, add coexistence section, remove line-number refs |
