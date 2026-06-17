# Monitoring-Member Scope & Execute-Loop Persistence

**Status**: Approved
**Progress**: 4/12 tasks complete
**Last Updated**: 2026-06-16

## Overview

Tighten three CAFleet skill behaviors using concise, affirmative wording only: (1) confine the monitoring member's on-wake routine to exactly two commands (`cafleet member capture` and `cafleet member nudge`); (2) make `/cafleet-design-doc-execute` drive the team through every design-doc task before stopping; and (3) make the Step 7 Copilot review loop persist under Administrator-only termination. This is a documentation-only change to skill `SKILL.md` and role files — no CLI, command, or code behavior changes.

## Success Criteria

- [ ] The `cafleet-agent-team-monitoring` skill states the monitoring member's on-wake routine acts through exactly two commands — `cafleet member capture` (read-only inspection) and `cafleet member nudge` (re-engage the idle Director) — in both the spawn prompt and the wake-routine description, affirmatively.
- [ ] The `cafleet-design-doc-execute` `SKILL.md` and `roles/director.md` state that once execute is invoked the fleet runs autonomously and collaboratively through every design-doc task to completion, while preserving the designed gates (Step 5 user approval, "stop means stop", new-decision escalations), affirmatively.
- [ ] The `cafleet-design-doc-execute` Step 7 (`SKILL.md`) and `roles/director.md` state that authority to end the Copilot loop rests solely with the Administrator (the user), that the loop ends on exactly two conditions (user instructs termination, OR Copilot reports no remaining concerns), and that every other state keeps the loop turning — waiting while a review is pending and autonomously re-requesting a review that failed to land, affirmatively.
- [ ] All new and edited text for the three improvements is concise and uses affirmative phrasing only (no newly introduced `NEVER` / `DO NOT` style wording).
- [ ] The edits are internally consistent: no dangling references remain (e.g. the stale `7f` references in `director.md`), and the Step 7 branch logic, loop-state table, and `director.md` milestones row all agree.

---

## Background

Three recurring failure modes motivate this change:

1. **Monitoring-member overreach.** The dedicated monitoring member has attempted commands beyond its job (e.g. suggesting a "kill" command). Its job is exactly two on-wake actions: read-only `cafleet member capture` to inspect, and `cafleet member nudge` to re-engage an idle Director.
2. **Execute stops early.** `/cafleet-design-doc-execute` sometimes terminates before all design-doc tasks are complete. Once invoked, the fleet must keep working through every task.
3. **Copilot loop stops early.** The Step 7 Copilot review loop tends to stop after a few rounds, before Copilot reports no concerns. Ending the loop is an Administrator-only decision.

Two constraints govern every edit below: keep new/edited text **concise**, and express scope and obligations **affirmatively** (positive phrasing only). The constraint governs the **new** text for these three improvements; pre-existing negative phrasing elsewhere in the skills is out of scope and is left untouched, except where an edited passage must change to stay consistent.

---

## Specification

All paths are relative to the repo root `skills/` tree. Anchor line numbers below reflect the files at draft time and are guidance for locating the edit; match on the quoted text, not the line number.

### Improvement 1 — Monitoring-member scope (affirmative)

File: `skills/cafleet-agent-team-monitoring/SKILL.md`. The fix lives in the monitoring-member spawn prompt and its wake-routine description. The scope constrains only the **on-wake routine**; the startup/teardown lifecycle (`cafleet monitor start`, `cafleet monitor status`, the `ready:` `cafleet message send` handshakes, stopping the background task) stays intact and is not constrained by the two-action scope.

**1a. Spawn-prompt scope sentence.** In the canonical spawn prompt, immediately after the paragraph that ends `member-driving routes back through the Director.`, add a new paragraph:

```text
Your on-wake routine acts through exactly two commands: cafleet member
capture for read-only inspection, and cafleet member nudge to re-engage the idle
Director. Keep every wake within these two actions.
```

**1b. Wake-routine lead-in.** In the same spawn prompt, on the line that begins `On each wake (a "[monitor] wake: ..." nudge ...):`, append a scope lead-in clause so the routine header names the two commands. Resulting line:

```text
On each wake (a "[monitor] wake: ..." nudge keystroked into this pane by the loop),
your routine uses exactly two commands — cafleet member capture (read-only
inspection) and cafleet member nudge (re-engage the idle Director):
```

(The existing numbered steps 1–2 under this header remain unchanged.)

### Improvement 2 — Execute runs to completion (affirmative)

**2a.** File: `skills/cafleet-design-doc-execute/SKILL.md`. Immediately after the `## Process` heading and before `### Step 1:`, insert a run-to-completion paragraph:

```markdown
**Run to completion.** Once `/cafleet-design-doc-execute` is invoked, the fleet operates autonomously and collaboratively through every task in the design document. The Director keeps driving the team — dispatching the next step to each idle member the moment it is ready — until all Implementation tasks and Success Criteria are complete. The designed checkpoints stay in force: the Step 5 user-approval gate, the user's "stop means stop" halt during Step 7, and escalations that require a genuinely new user decision.
```

**2b.** File: `skills/cafleet-design-doc-execute/roles/director.md`. In `## Your Accountability`, immediately after the `**Orchestrate the per-step TDD cycle.**` bullet, insert a new bullet:

```markdown
- **Drive every task to completion.** From invocation onward, keep the team working through every step of the design document — dispatch the next task to each idle member as soon as it is ready — until all Implementation tasks and Success Criteria are complete. The designed gates remain: pause at the Step 5 user-approval gate, honor the user's "stop means stop" halt, and escalate when a genuinely new user decision is required.
```

### Improvement 3 — Copilot review loop persists (affirmative)

File: `skills/cafleet-design-doc-execute/SKILL.md` (Step 7) and `skills/cafleet-design-doc-execute/roles/director.md`.

Termination signal definition used throughout: a **Copilot no-concerns signal** is a post-push (`> last_push_ts`) Copilot-authored entry that is either a `reviews` entry with `state == "APPROVED"`, or a Copilot review/comment whose body indicates no remaining concerns even when its `state == "COMMENTED"`.

**3a. Termination-authority subsection.** In `SKILL.md`, at the start of `### Step 7: Copilot Review Loop (Director)`, immediately after the existing intro paragraph (the one ending `... so this idle-nudge is the loop's turn source.`) and before `#### PR Review Loop State`, insert a new `####` subsection:

```markdown
#### Termination authority

Once the loop is active (the PR exists and Copilot has been invited), authority to end it rests solely with the Administrator (the user). The loop ends on exactly two conditions: (1) the user instructs termination (§ User Interjection During Step 7), or (2) a post-push Copilot no-concerns signal arrives — a `reviews` entry with `state == "APPROVED"`, or a Copilot review/comment whose body indicates no remaining concerns even when `state == "COMMENTED"`. In every other state the Director keeps the loop turning: it waits while a Copilot review is pending, and it autonomously re-requests the review (7e) when a prior request failed to land. The Step 6a preconditions and the initial push / PR-create failures are pre-loop fallbacks that skip Step 7 entirely — they are distinct from ending an active loop.
```

**3b. Step 7b branch table.** In `#### 7b. Per-turn procedure`, replace the entire branch table (the four rows currently keyed on `state == "APPROVED"`, `silence_ticks < 30`, `silence_ticks >= 30`, and `≥ 1 new Copilot items`) with:

```markdown
Evaluate top-down; the first matching row wins (a post-push no-concerns signal matches row 1 before the general new-items row):

| Result | Action |
|:--|:--|
| A **post-push** Copilot no-concerns signal — a `reviews` entry with `state == "APPROVED"`, OR a Copilot review/comment whose body indicates no remaining concerns (even when `state == "COMMENTED"`) | Exit loop (success) → Step 8 |
| ≥ 1 new Copilot items | Reset `silence_ticks = 0`, go to 7c |
| 0 new Copilot items AND `silence_ticks < 30` | Increment `silence_ticks`, keep waiting |
| 0 new Copilot items AND `silence_ticks >= 30` | Run the 7e autonomous re-request check, reset `silence_ticks = 0`, keep waiting |
```

**3c. Post-table notes.** In `#### 7b`, replace the two notes that currently begin `The APPROVED check MUST be qualified ...` and `**No auto-exit on silence**: ...` with:

```markdown
The no-concerns exit MUST be qualified by the post-push filter (`submittedAt > last_push_ts` for reviews, `created_at > last_push_ts` for comments): only a Copilot signal newer than the most recent fix-push clears the current HEAD. An older approval or no-concerns note reflects a previous revision and leaves the loop running.

**Silence keeps the loop turning.** A silent Copilot is a pending review, not completion. On prolonged silence the Director autonomously re-requests the review (7e) and continues; the loop ends only on the two termination conditions above — the user instructs termination, or a post-push Copilot no-concerns signal arrives.
```

(The `**Read `reviews`, not `reviewDecision`**` note that follows stays unchanged.)

**3d. Loop-state reset rule.** In `#### PR Review Loop State`, update the `silence_ticks` row's `Update rule` cell to add the 7e reset:

```markdown
| `silence_ticks` | Consecutive Director turns (driven by the monitoring member's idle nudge) with 0 new Copilot items since the last activity | Increment each turn with 0 new items; reset to 0 when new Copilot items arrive, after a fix-push from 7d, OR after the 7e autonomous re-request |
```

**3e. Replace 7e.** Replace the entire `#### 7e. Silence escalation` subsection (its intro, the four-option `AskUserQuestion` table, and the trailing paragraph) with:

```markdown
#### 7e. Silence handling — autonomous re-request

When `silence_ticks >= 30` (≈ 30 min since the last Copilot activity AND no new items this turn), the Director re-requests the review on its own — no user prompt. Authority to end the loop stays with the Administrator (§ Termination authority); silence is a pending review, so the Director keeps it turning:

1. **Detect pending vs. failed-to-land** via `gh api repos/<owner>/<repo>/pulls/<pr-number>/requested_reviewers`:
   - **Copilot present** → the request landed and the review is pending; reset `silence_ticks = 0` and keep waiting.
   - **Copilot absent AND no post-push Copilot review exists** → the request failed to land; re-request with `gh pr edit <pr-number> --add-reviewer @copilot`, confirm Copilot now appears in `requested_reviewers`, reset `silence_ticks = 0`, and keep waiting.
2. The user may terminate at any time via the "stop means stop" halt (§ User Interjection During Step 7) — that is the one path that ends the loop short of a Copilot no-concerns signal.

The 30-tick patience window keeps the Director from re-requesting every tick; Copilot's first review after a `--add-reviewer` typically lands within 3–5 minutes.
```

**3f. Execute intro paragraph.** In `SKILL.md`, in the intro paragraph under `# Design Doc Execute (CAFleet Edition)`, replace the clause `that routes inline comments to the still-live Programmer / Tester and exits when Copilot approves or the user resolves a silence escalation` with:

```text
that routes inline comments to the still-live Programmer / Tester and ends only when the user instructs termination or Copilot reports no remaining concerns
```

**3g. Director accountability bullet.** In `roles/director.md`, in `## Your Accountability`, replace the sentence in the `**Run the PR & Copilot Review loop after Approve.**` bullet that currently reads `**The loop never auto-exits on Copilot silence** — it exits only on a post-push `state == "APPROVED"` or a user choice (the 7e round-limit / 7f silence escalations leave it running unless the user picks Finalize/abort).` with:

```markdown
**The loop ends only by Administrator authority** — on the user's instruction to terminate, or on a post-push Copilot no-concerns signal (`state == "APPROVED"`, or a post-push review/comment body indicating no remaining concerns). In every other state the Director keeps it turning: it waits while Copilot's review is pending and autonomously re-requests the review (7e) when a prior request failed to land.
```

**3h. Director milestones row.** In `roles/director.md`, in the `### Skill-specific milestones` table, replace the `Director action` cell of the `PR Review (Step 7)` row with:

```markdown
Increment `silence_ticks`. Evaluate the SKILL Step 7b branch table: exit on a post-push Copilot no-concerns signal (`state == "APPROVED"`, or a review/comment body indicating no remaining concerns); at `silence_ticks >= 30` run the 7e autonomous re-request (check `requested_reviewers`; re-request when Copilot is absent and no post-push review exists), reset `silence_ticks`, and keep waiting. On ≥ 1 new entry reset `silence_ticks = 0`, classify each new inline comment by file path per Step 7c, write `COMMENT(copilot): <body>` at the source `<file>:<line>` for source/test routes (or `COMMENT(director): <body>` at the affected paragraph for design-doc-anchored items, no cafleet route), and dispatch via `cafleet message send --fleet-id <fleet-id> --agent-id <director-agent-id> --to <member-agent-id> --text "ready (<file>:<line>)"`. The loop ends only by Administrator authority.
```

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

This is a documentation-only change (skill markdown). No tests apply; verification is by reading the edited passages against the Success Criteria. Apply edits with the `Edit` tool, matching on the quoted anchor text.

### Step 1: Monitoring-member scope (Improvement 1)

- [x] 1a. Add the spawn-prompt scope sentence after the `member-driving routes back through the Director.` paragraph in `skills/cafleet-agent-team-monitoring/SKILL.md` (Spec 1a). <!-- completed: 2026-06-17T11:50 -->
- [x] 1b. Add the two-command lead-in to the `On each wake (...)` routine header in the same file (Spec 1b). <!-- completed: 2026-06-17T11:50 -->

### Step 2: Execute runs to completion (Improvement 2)

- [x] 2a. Insert the `**Run to completion.**` paragraph after the `## Process` heading in `skills/cafleet-design-doc-execute/SKILL.md` (Spec 2a). <!-- completed: 2026-06-17T11:53 -->
- [x] 2b. Insert the `**Drive every task to completion.**` accountability bullet after the TDD-cycle bullet in `skills/cafleet-design-doc-execute/roles/director.md` (Spec 2b). <!-- completed: 2026-06-17T11:53 -->

### Step 3: Copilot review loop persists (Improvement 3)

- [ ] 3a. Insert the `#### Termination authority` subsection at the start of Step 7 in `SKILL.md` (Spec 3a). <!-- completed: -->
- [ ] 3b. Replace the Step 7b branch table (with the top-down first-match lead-in) in `SKILL.md` (Spec 3b). <!-- completed: -->
- [ ] 3c. Replace the two post-table notes in Step 7b with the qualify-no-concerns note and the `**Silence keeps the loop turning.**` note in `SKILL.md` (Spec 3c). <!-- completed: -->
- [ ] 3d. Update the `silence_ticks` reset rule in the PR Review Loop State table in `SKILL.md` (Spec 3d). <!-- completed: -->
- [ ] 3e. Replace `#### 7e. Silence escalation` with `#### 7e. Silence handling — autonomous re-request` in `SKILL.md` (Spec 3e). <!-- completed: -->
- [ ] 3f. Update the execute intro-paragraph termination clause in `SKILL.md` (Spec 3f). <!-- completed: -->
- [ ] 3g. Replace the `**The loop never auto-exits ...**` sentence in the Director accountability bullet in `roles/director.md` (Spec 3g). <!-- completed: -->
- [ ] 3h. Replace the `PR Review (Step 7)` milestones-row Director action cell in `roles/director.md` (Spec 3h). <!-- completed: -->

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-06-16 | Initial draft |
