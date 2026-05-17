# Surface `cafleet doctor` as the Canonical Env-Check Primitive for CAFleet Directors

**Status**: Complete
**Progress**: 3/3 tasks complete
**Last Updated**: 2026-05-17

## Overview

`cafleet doctor` already emits the exact pane-identity output (`session_name`, `window_id`, `pane_id`, `TMUX_PANE`) that a CAFleet Director needs before spawning teammates, but the skill files never name it as the canonical "verify env" entry point. This design surfaces `cafleet doctor` in two skill files — `skills/cafleet/SKILL.md` (Typical Workflow) and `skills/agent-team-supervision/SKILL.md` (Spawn Protocol + Quick Reference) — so Directors discover it instead of reaching for raw `tmux display-message` or `TMUX` env-var expansion.

## Success Criteria

- [x] A Director reading `skills/cafleet/SKILL.md` Typical Workflow encounters `cafleet doctor` as Step 0 ("Verify pane env") before any `cafleet session create` call.
- [x] A Director reading `skills/agent-team-supervision/SKILL.md` § *Spawn Protocol* finds a sub-bullet under step 1 instructing them to run `cafleet doctor` first and abort the spawn if it exits non-zero.
- [x] The Quick Reference table in `skills/agent-team-supervision/SKILL.md` includes a "Verify Director pane env" row mapping to `cafleet doctor`.
- [x] No edits land in `~/.claude/rules/bash-command.md` or any other rule file (global or project). The skill files carry the full guidance.
- [x] No CLI behavior changes — `cafleet doctor` is documentation-only surfacing of an existing subcommand.

---

## Background

### Motivating incident

While spawning a three-teammate CAFleet team in session `e66a2f1f-1fb8-442e-997c-79aee7496a28`, the Director (main Claude) tried two raw probes to confirm its tmux pane identity:

1. `echo $TMUX` — blocked by the variable-expansion guard in `~/.claude/rules/bash-command.md`.
2. `tmux display-message -p` — a raw tmux call, against the "use cafleet primitives only" norm in `skills/agent-team-supervision/SKILL.md`.

`cafleet doctor` would have answered both probes in one call — and is the **only** sanctioned env-check primitive — but the Director did not reach for it because no skill file pointed at it from a "verify env" scanning path.

### Reframe (user-supplied, captured during clarification)

An earlier draft of this change included a new Tool Substitution row in `~/.claude/rules/bash-command.md` mapping raw tmux probes to `cafleet doctor`. The user rejected that path:

- `~/.claude/rules/bash-command.md` is the user's **project-independent global rule** and MUST NOT depend on any project-specific binary like `cafleet`.
- A project-local overlay rule was also rejected — the substitution guidance lives inside the skill files instead.
- `tmux env` / `tmux show-environment` is explicitly out of scope (it surfaces the tmux server's environment, which `cafleet doctor` does not replicate).

Net design surface: two skill files, no rule files.

---

## Specification

### File 1 — `skills/cafleet/SKILL.md`

**Insertion point**: § *Typical Workflow* (currently starts at "1. Create a session"). The existing § *Doctor* (lines 202–211) already documents the subcommand adequately and is left untouched.

**Change**: prepend a new **Step 0 — Verify pane env** as a list item with marker `0.`; the existing items keep their literal `1.`–`7.` markers and CommonMark's start-number rule renders them as 1–7.

Literal text to insert directly above the existing "1. **Create a session**" line:

````markdown
0. **Verify pane env** (Director / spawn-aware operator):
   ```bash
   cafleet doctor
   # tmux:
   #   session_name:  <name>
   #   window_id:     @<n>
   #   pane_id:       %<n>
   #   TMUX_PANE:     %<n>
   ```

   Confirms the calling shell has `TMUX` and `TMUX_PANE` set. Reach for this BEFORE `cafleet session create` and BEFORE any `cafleet member create` call — it is the canonical pane-identity probe, replacing raw `tmux display-message` and `TMUX` / `TMUX_PANE` env-var expansion. See § *Doctor* for the full output shape and exit semantics.
````

### File 2 — `skills/agent-team-supervision/SKILL.md`

#### Change 2a: Spawn Protocol sub-bullet

**Insertion point**: § *Spawn Protocol* step 1 (line 64 in the current file). The existing step reads:

> 1. **Ensure the supervision mechanism is already running** — for Claude Code Directors, the `/loop` monitor must be active; for codex Directors, one of the fallbacks listed in `Skill(agent-team-monitoring)` § Mechanism by backend …

**Change**: add a sub-bullet at the head of step 1, before the "Ensure" text. The sub-bullet is **gating** — if `cafleet doctor` exits non-zero (missing `TMUX` / `TMUX_PANE`), the Director aborts the spawn protocol and surfaces the error to the user; member spawning cannot proceed outside tmux because `cafleet member create` itself requires tmux.

Literal restructure of step 1 (the existing prose becomes the second sub-bullet):

```markdown
1. **Verify env, then ensure supervision is running**:
   - **Pre-spawn env-check (gating)**: run `cafleet doctor`. If it exits non-zero or reports missing `TMUX` / `TMUX_PANE`, ABORT the spawn protocol and surface the error to the user — `cafleet member create` requires the Director to be inside a tmux pane, and silently proceeding would fail later with a less-actionable error. This is the canonical pane-identity probe; do NOT reach for raw `tmux display-message` or `TMUX` env-var expansion.
   - **Ensure the supervision mechanism is already running** — for Claude Code Directors, the `/loop` monitor must be active; for codex Directors, one of the fallbacks listed in `Skill(agent-team-monitoring)` § Mechanism by backend (out-of-band cron driver, MCP scheduling server, user-driven nudges, or no-active-monitor synchronous mode) must be in place. See `Skill(agent-team-monitoring)` § `/loop` Prompt Template for the canonical Claude Code setup.
```

Steps 2 and 3 (Spawn the member / Verify the member is active) are unchanged in **content**.

> **Note on baseline drift (resolved during execution).** At execution time the actual file already had a brief 4-step shape: step 1 was a one-line `cafleet doctor` stub, step 2 was the supervision-ensure prose (the "existing step" quoted at line 74 above), and steps 3 + 4 were Spawn / Verify. The arbitrated execution consolidated the step-1 stub and the step-2 supervision-ensure prose into the new combined two-sub-bullet step 1 (the gating language is a strict superset of the stub's content), then renumbered the prior steps 3 → 2 and 4 → 3. The end-state matches the literal restructure above; only the step quoted at line 74 differs from what was actually in the file when execution started.

#### Change 2b: Quick Reference row

**Insertion point**: the Quick Reference table at the bottom of the file (line 100+). The table is currently anchored at "Start the supervision tick". The env-check row is the natural new first row — the env-check precedes starting the supervision tick.

Literal row to insert as the **first** data row of the table (above "Start the supervision tick"):

```markdown
| Verify Director pane env | `cafleet doctor` | Pre-spawn precondition; gating. Aborts the spawn protocol when `TMUX` / `TMUX_PANE` are missing. Replaces raw `tmux display-message` and `TMUX` env-var expansion. |
```

### Out of scope

The following files were considered and explicitly excluded:

| File | Why excluded |
|------|--------------|
| `~/.claude/rules/bash-command.md` | User-owned global rule; must not depend on the project-specific `cafleet` binary. |
| Any new project-local rule overlay (e.g. a new `.claude/rules/*.md`) | User explicitly declined this path during clarification — guidance belongs in the skills. |
| `skills/agent-team-monitoring/SKILL.md` | Mentions raw `tmux send-keys` only in the codex out-of-band fallback row, which is correct (the operator runs raw tmux from outside the codex session). No conflict with the env-check guidance. |
| `skills/cafleet/reference/director.md`, `skills/cafleet/reference/recovery.md` | Surveyed for contradictory raw-tmux instructions; none found in the env-check axis. |

### No code changes

`cafleet doctor` already exists, is fully functional, and emits both text and `--json` shapes. This design touches only documentation.

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

### Step 1: Add Step 0 "Verify pane env" to `skills/cafleet/SKILL.md` Typical Workflow

- [x] Insert the new Step 0 block (literal text in § *Specification → File 1*) directly above the existing "1. **Create a session**" line in § *Typical Workflow*. Leave the existing § *Doctor* section unchanged. <!-- completed: 2026-05-17T00:48 -->

### Step 2: Replace Spawn Protocol step 1 in `skills/agent-team-supervision/SKILL.md` with env-check sub-bullet + existing-text sub-bullet

- [x] Replace the current step-1 stub and step-2 supervision-ensure paragraph with the two-sub-bullet step 1 documented in § *Specification → File 2 → Change 2a*. Renumber the remaining steps so "Spawn the member" becomes step 2 and "Verify the member is active" becomes step 3. <!-- completed: 2026-05-17T00:52 -->

### Step 3: Prepend env-check row to the Quick Reference table in `skills/agent-team-supervision/SKILL.md`

- [x] Insert the literal "Verify Director pane env" row (§ *Specification → File 2 → Change 2b*) as the new first data row of the table, immediately under the header separator and above the "Start the supervision tick" row. <!-- completed: 2026-05-17T00:54 -->

---

## Changelog

- **2026-05-17** — Implemented all three steps via `/cafleet:design-doc-execute` (Director + Programmer team). Phase A skipped per documentation-only team composition. Step 2 required arbitration to consolidate a pre-existing brief `cafleet doctor` stub at file step 1 with the supervision-ensure prose at file step 2 into the new two-sub-bullet form (richer gating language strictly supersedes the stub); steps 3 → 2 and 4 → 3 renumbered. PR #79 opened with `@copilot` review; first pass surfaced four inline comments (two source, two design-doc) which were addressed in one fix-push (commit `fe78f06`) and one docs commit (`6b6cd16`). Copilot's second pass returned `state: COMMENTED` with "generated no new comments" — accepted as de-facto approval per user. Status: Approved → Complete.
