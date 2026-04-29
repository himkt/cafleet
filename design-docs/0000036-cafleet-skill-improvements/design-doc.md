# 0000036 — CAFleet Skill Improvements

**Status**: Approved
**Progress**: 7/22 tasks complete
**Last Updated**: 2026-04-29

## Overview

The project's `skills/design-doc-create/` and `skills/design-doc-execute/` skills lag behind the user's global skills (`~/.claude/skills/design-doc-create`, `design-doc-execute`, `design-doc`, `agent-team-supervision`, `agent-team-monitoring`) on five concrete points: step numbering bugs, missing idle-semantics guidance in director roles, missing shutdown sections in member roles, missing stall-response ladder in director roles, and cleanup steps that don't reference the canonical 5-step Shutdown Protocol in `skills/cafleet/SKILL.md`. This design lifts the CAFleet-native skills to the global quality bar while keeping every CAFleet-specific primitive (broker `cafleet message send`, `cafleet member create` / `delete`, `cafleet session delete`, push notifications) intact.

## Success Criteria

- [ ] Every step header in `skills/design-doc-create/SKILL.md` Step 1 is contiguous (no missing `1b`).
- [ ] Every step header in `skills/design-doc-execute/SKILL.md` Step 3 is contiguous (no missing `3b`).
- [ ] Both director role files (`skills/design-doc-create/roles/director.md`, `skills/design-doc-execute/roles/director.md`) contain an **Idle Semantics** subsection that mirrors the global `agent-team-supervision` rule, adapted for CAFleet's tmux-pane idle model.
- [ ] Both director role files contain a **Stall Response Ladder** subsection (1 — specific instruction, 2 — second specific nudge, 3 — escalate to user via `AskUserQuestion`).
- [ ] All five member role files (`design-doc-create/roles/{drafter,reviewer}.md`, `design-doc-execute/roles/{programmer,tester,verifier}.md`) contain a **Shutdown** subsection explaining that termination is via `cafleet member delete` → `/exit` (no `shutdown_request` JSON message).
- [ ] `skills/design-doc-create/SKILL.md` Step 6 and `skills/design-doc-execute/SKILL.md` Step 8 explicitly defer to `Skill(cafleet)` § *Shutdown Protocol* for the full 5-step teardown order (cron-cancel → `member delete` per member → `member list` verification → `session delete` → `session list` sanity check) instead of inlining a partial subset.
- [ ] `.claude/settings.json` `permissions.allow` lists every `cafleet:` plugin skill (`Skill(cafleet:cafleet)`, `Skill(cafleet:cafleet-monitoring)`, `Skill(cafleet:design-doc)`, `Skill(cafleet:design-doc-create)`, `Skill(cafleet:design-doc-execute)`) so loading any of them no longer triggers a permission prompt.
- [ ] No regressions: every existing example command, prompt body, and table elsewhere in the touched files reads identically apart from the targeted edits.

---

## Background

The user maintains canonical "global" skills under `~/.claude/skills/` that act as the quality bar for in-process Agent Teams. The cafleet project mirrors a subset of those skills as **CAFleet-native** variants (`skills/design-doc-create`, `skills/design-doc-execute`) that swap the Agent Teams primitives (`TeamCreate`, `Agent(team_name=…)`, `SendMessage`, `TeamDelete`) for cafleet broker primitives (`cafleet session create`, `cafleet member create`, `cafleet message send`, `cafleet session delete`).

These CAFleet variants were written incrementally and have drifted from the global versions on five small-but-consequential points (enumerated in **Specification** below). The drift causes real harm:

- A spawned Drafter / Reviewer / Programmer / Tester / Verifier never reads what to do when `/exit` arrives, so the role file leaves a Skill-level question unanswered.
- A Director loading the CAFleet variant misses the "idle is normal" rule and may nudge members on every idle notification, flooding the inbox.
- The cleanup section duplicates a partial subset of the canonical Shutdown Protocol, so when `Skill(cafleet)` updates its protocol, the design-doc-create / design-doc-execute SKILL.md files silently fall behind.
- Numbering bugs (`1a → 1c`, `3a → 3c`) signal incompleteness even though the content is intact.

Fixing these is documentation-only — no code, schema, CLI, or test changes.

---

## Specification

### S1. Step numbering bugs

`skills/design-doc-create/SKILL.md` Step 1 currently uses sub-headers `1a → 1c → 1d → 1e → 1f → 1g` (no `1b`).
`skills/design-doc-execute/SKILL.md` Step 3 currently uses sub-headers `3a → 3c → 3d → 3e → 3f → 3g` (no `3b`).

**Fix**: shift each post-gap header up one letter so the sequences become `1a..1f` and `3a..3f` contiguous, preserving every body and the order of operations. The "Start the monitoring `/loop`" subsection in design-doc-create stays second because it is a CAFleet-native step that has no global counterpart and must run before any spawn.

| File | Old → New |
|:--|:--|
| `skills/design-doc-create/SKILL.md` | `1c → 1b` (Start the monitoring `/loop`) |
| `skills/design-doc-create/SKILL.md` | `1d → 1c` (Read role definitions) |
| `skills/design-doc-create/SKILL.md` | `1e → 1d` (Spawn the Drafter) |
| `skills/design-doc-create/SKILL.md` | `1f → 1e` (Spawn the Reviewer) |
| `skills/design-doc-create/SKILL.md` | `1g → 1f` (Verify members are live) |
| `skills/design-doc-execute/SKILL.md` | `3c → 3b` (Start the monitoring `/loop`) |
| `skills/design-doc-execute/SKILL.md` | `3d → 3c` (Analyze implementation tasks) |
| `skills/design-doc-execute/SKILL.md` | `3e → 3d` (Read role files) |
| `skills/design-doc-execute/SKILL.md` | `3f → 3e` (Spawn each member) |
| `skills/design-doc-execute/SKILL.md` | `3g → 3f` (Verify members are live) |

The two `/loop`-cancel callouts in `design-doc-execute/SKILL.md` that reference `Step 3c` (the team-health cron ID origin) — at Step 7a, Step 8 step 5, and inside `roles/director.md` — must be updated to `Step 3b` to remain accurate.

### S2. Idle Semantics in CAFleet director roles

Insert into both `skills/design-doc-create/roles/director.md` and `skills/design-doc-execute/roles/director.md`, immediately before the existing **Communication Protocol** section (so Idle Semantics frames the protocol that follows):

```markdown
## Idle Semantics

**Members go idle after every turn. A member's tmux pane sitting at the prompt between turns is the expected state, NOT a stall.** A member sending you a `cafleet message send` and then returning to the prompt is the normal flow — they sent their output and are waiting for the next push notification or the next assignment.

- Idle members receive messages normally; the broker's push notification (`tmux send-keys` of `cafleet message poll`) wakes them.
- `/loop` notifications about idle panes are informational. Do not react unless you are ready to assign new work, OR the member's idleness is **blocking your next step** (a downstream phase cannot start, an expected deliverable file is missing past its milestone, you sent a message and received no reply after a reasonable window).
- Do NOT comment on idleness or nudge a member just because they went idle. Only nudge per the Stall Response Ladder below.
```

This mirrors the global `agent-team-supervision` Idle Semantics block, adapted to CAFleet's tmux-pane idle model.

### S3. Stall Response Ladder in CAFleet director roles

Insert into both director role files, immediately after the **Idle Semantics** section above:

```markdown
## Stall Response Ladder

A member is stalled when they **block your next step** — not merely because they are idle. Signals:

- The deliverable file you expect at this milestone does not exist.
- `cafleet message poll --agent-id <director-agent-id>` shows no progress message from the member since the last assignment AND `cafleet member capture` shows no forward progress in the pane buffer.
- You sent a `cafleet message send` and the member has not replied past one full `/loop` tick.

**Response ladder (in order — do NOT skip rungs):**

1. Send a specific instruction via `cafleet message send` — never a generic "are you OK?". State the deliverable you expect and the blocker you are trying to unblock.
2. If still no reply after a second nudge across one more `/loop` tick, run `cafleet member capture --member-id <member-agent-id> --lines 200` and inspect the pane state. If the pane is on an `AskUserQuestion` frame, follow the canonical three-beat workflow in `Skill(cafleet)` § *Answer a member's AskUserQuestion prompt*.
3. After 2 nudges without progress, escalate to the user via `AskUserQuestion` with concrete options (re-spawn / redistribute / drop scope / Other). Do NOT silently `cafleet member delete` and re-spawn — the user might know something you don't (intentional pause, network glitch).
```

This is a direct adaptation of `agent-team-supervision` § *Stall Response* into CAFleet primitives.

### S4. Shutdown subsection in every CAFleet member role file

Append to each of the five member role files a **Shutdown** subsection. The wording is the same across all five; only the role name changes.

Files affected:

- `skills/design-doc-create/roles/drafter.md`
- `skills/design-doc-create/roles/reviewer.md`
- `skills/design-doc-execute/roles/programmer.md`
- `skills/design-doc-execute/roles/tester.md`
- `skills/design-doc-execute/roles/verifier.md`

Body to append (substitute `<role>` per file):

```markdown
## Shutdown

You are terminated by the Director via `cafleet --session-id <session-id> member delete --agent-id <director-agent-id> --member-id <my-agent-id>`. The CLI sends `/exit` to your pane and waits up to 15 s for it to disappear.

You do NOT need to handle any `shutdown_request` JSON message — that is the in-process Agent Teams primitive. The CAFleet equivalent is `/exit`, dispatched by the Director through the tmux push primitive. When you receive `/exit`, your `claude` process terminates immediately; nothing is required of you.

If your Director sends `cafleet message send` instructing you to wrap up (e.g. "report final status, then I will run member delete"), do that one final report via `cafleet message send` and return to the prompt. The Director will then run `cafleet member delete` from its own pane.
```

The text is identical for each role; substituting nothing is necessary because the role file's surrounding sections already establish role identity.

### S5. Cleanup steps reference the canonical Shutdown Protocol

`skills/design-doc-create/SKILL.md` Step 6 currently inlines a 4-step cleanup (finalize → CronDelete → member delete each → session delete). It is missing the `member list` verification rung and the `session list` sanity-check rung from the canonical Shutdown Protocol in `skills/cafleet/SKILL.md`.

`skills/design-doc-execute/SKILL.md` Step 8 has the same shape — it inlines a partial subset.

**Fix**: replace the inlined teardown body in each SKILL.md with a single sentence pointing at the canonical protocol, while keeping the skill-specific bits (final Drafter finalize, Status:Complete commit, conditional `git push`, PR URL summary) inline.

#### `skills/design-doc-create/SKILL.md` Step 6 — new body

```markdown
### Step 6: Finalize & Clean Up (Director)

1. Instruct the Drafter to finalize:
   ```bash
   cafleet --session-id <session-id> message send --agent-id <director-agent-id> \
     --to <drafter-agent-id> --text "User approved. Please finalize: set Status to Approved, refresh Last Updated, bump the Progress header field if present in the template, verify implementation steps are actionable, then report done."
   ```
   Wait for the Drafter's confirmation.

2. Run the canonical teardown per `Skill(cafleet)` § *Shutdown Protocol*:
   1. `CronDelete` the `/loop` monitor (cron ID recorded at Step 1b).
   2. `cafleet member delete` for each member (Drafter, then Reviewer). Each call blocks until the pane is gone (15 s timeout); on exit 2 follow the `member capture` + `send-input` recovery in the canonical protocol, or rerun with `--force`.
   3. `cafleet member list` — the team's roster MUST be empty before continuing.
   4. `cafleet session delete <session-id>` (positional, no `--session-id` flag).
   5. `cafleet session list` — the session MUST not appear (soft-deleted sessions are hidden).

The session row is soft-deleted and `tasks` are preserved so the message trail remains inspectable in the admin WebUI.
```

#### `skills/design-doc-execute/SKILL.md` Step 8 — new body

```markdown
### Step 8: Finalize & Clean Up (Director)

Runs after Step 7 exits, or directly after Step 5 when Step 6 was skipped (gh not authenticated / default branch / no commits / approve-local intent).

1. Update design document Status to "Complete" and add a final Changelog entry.
2. `git add <design-doc>` (separate Bash call).
3. `git commit -m "docs: mark design doc as complete"` (separate Bash call).
4. **Push decision** (separate Bash call): run `git rev-parse --abbrev-ref <branch-name>@{upstream}`.
   - Exit code 0 (branch is tracked on origin): `git push`. Covers both the "Step 6 fully succeeded" path and the "Step 6 partial-fail (push OK, PR create failed)" path.
   - Non-zero exit: skip the push. The docs commit stays local.
   - The Director does NOT re-request Copilot review on this final docs commit.
5. Run the canonical teardown per `Skill(cafleet)` § *Shutdown Protocol*:
   1. `CronDelete` the currently active `/loop` monitor — team-health (cron ID from Step 3b) if Step 6 was skipped, augmented (cron ID from Step 7a) otherwise.
   2. `cafleet member delete` for each spawned member (Programmer, Tester if spawned, Verifier if spawned). Each call blocks until the pane is gone; on exit 2 follow the `member capture` + `send-input` recovery, or rerun with `--force`.
   3. `cafleet member list` — the team's roster MUST be empty before continuing.
   4. `cafleet session delete <session-id>`.
   5. `cafleet session list` — the session MUST not appear.
6. **Report to the user**: include the PR URL (if Step 6 created one), the review-round summary (rounds used, exit reason: approved / quiescent / round-limit / skipped), and any skipped-step reasons.
```

The Director role files (`design-doc-create/roles/director.md` § *Shutdown Protocol* and `design-doc-execute/roles/director.md` § *Shutdown Protocol*) get the same treatment — replace inlined steps with a one-line pointer to `Skill(cafleet)` § *Shutdown Protocol* plus the skill-specific cron-ID nuance (which loop is active when).

### S6. Allow `cafleet:` plugin skills in `.claude/settings.json`

`.claude/settings.json` currently allows the global skill names (`Skill(design-doc-create)`, `Skill(design-doc-execute)`, `Skill(design-doc)`, `Skill(update-readme)`) but not the project-local **CAFleet plugin** variants exposed under the `cafleet:` namespace by `enabledPlugins.cafleet@cafleet`. As a result, every invocation of `cafleet:design-doc-create`, `cafleet:design-doc-execute`, `cafleet:cafleet`, or `cafleet:cafleet-monitoring` triggers a permission prompt that interrupts the orchestration loop the skills themselves are trying to run.

**Fix**: extend `permissions.allow` with one entry per cafleet plugin skill. Listing each skill explicitly (instead of a `Skill(cafleet:*)` glob) keeps the permission surface auditable and matches the existing entry-per-skill style elsewhere in the file.

Insert these five entries into the `permissions.allow` array (placement is not load-bearing — alphabetical near the existing `Skill(...)` entries is fine):

```json
"Skill(cafleet:cafleet)",
"Skill(cafleet:cafleet-monitoring)",
"Skill(cafleet:design-doc)",
"Skill(cafleet:design-doc-create)",
"Skill(cafleet:design-doc-execute)"
```

Constraints:

- Do NOT remove the existing non-prefixed `Skill(design-doc-*)` / `Skill(update-readme)` allow entries — they cover the global user-level skills, which remain reachable and useful (e.g. when working in this repo on a non-cafleet design doc).
- Do NOT touch `permissions.deny` or `permissions.ask`.
- Keep the JSON file valid: comma-terminate every preceding array element when inserting; the final entry in the array carries no trailing comma.
- Do NOT modify `.claude/settings.local.json` — this is a per-user override file and the project-wide allow list belongs in `settings.json`.

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

### Step 1: Numbering fixes (S1)

- [x] In `skills/design-doc-create/SKILL.md`, renumber Step 1 sub-headers `1c → 1b`, `1d → 1c`, `1e → 1d`, `1f → 1e`, `1g → 1f`. <!-- completed: 2026-04-29T13:00 -->
- [x] In `skills/design-doc-execute/SKILL.md`, renumber Step 3 sub-headers `3c → 3b`, `3d → 3c`, `3e → 3d`, `3f → 3e`, `3g → 3f`. <!-- completed: 2026-04-29T13:00 -->
- [x] In `skills/design-doc-execute/SKILL.md` and `skills/design-doc-execute/roles/director.md`, update every reference to "Step 3c" (the team-health cron ID origin) to "Step 3b" — Grep for the literal `Step 3c` and `from Step 3c` to find every occurrence. <!-- completed: 2026-04-29T13:00 -->

### Step 2: Idle Semantics + Stall Response Ladder (S2, S3)

- [x] Add the **Idle Semantics** subsection (verbatim per S2 in this design doc) to `skills/design-doc-create/roles/director.md`, immediately before the existing `## Communication Protocol` heading. <!-- completed: 2026-04-29T13:05 -->
- [x] Add the **Stall Response Ladder** subsection (verbatim per S3) to `skills/design-doc-create/roles/director.md`, immediately after the new `## Idle Semantics` heading. <!-- completed: 2026-04-29T13:05 -->
- [x] Add the **Idle Semantics** subsection to `skills/design-doc-execute/roles/director.md`, immediately before the existing `## Communication Protocol` heading. <!-- completed: 2026-04-29T13:05 -->
- [x] Add the **Stall Response Ladder** subsection to `skills/design-doc-execute/roles/director.md`, immediately after the new `## Idle Semantics` heading. <!-- completed: 2026-04-29T13:05 -->

### Step 3: Member Shutdown subsections (S4)

- [ ] Append the `## Shutdown` subsection (verbatim per S4) to `skills/design-doc-create/roles/drafter.md`. <!-- completed: -->
- [ ] Append the `## Shutdown` subsection to `skills/design-doc-create/roles/reviewer.md`. <!-- completed: -->
- [ ] Append the `## Shutdown` subsection to `skills/design-doc-execute/roles/programmer.md`. <!-- completed: -->
- [ ] Append the `## Shutdown` subsection to `skills/design-doc-execute/roles/tester.md`. <!-- completed: -->
- [ ] Append the `## Shutdown` subsection to `skills/design-doc-execute/roles/verifier.md`. <!-- completed: -->

### Step 4: Cleanup steps reference canonical Shutdown Protocol (S5)

- [ ] Replace `skills/design-doc-create/SKILL.md` Step 6 body with the version specified in S5 (Drafter finalize + 5-rung canonical teardown pointer). Verify the cron ID origin reference reads `Step 1b` after S1 renumbering. <!-- completed: -->
- [ ] Replace `skills/design-doc-execute/SKILL.md` Step 8 body with the version specified in S5 (status-Complete commit + push decision + 5-rung canonical teardown pointer). Verify the cron ID origin references read `Step 3b` and `Step 7a` after S1 renumbering. <!-- completed: -->
- [ ] Update `skills/design-doc-create/roles/director.md` § *Shutdown Protocol* to point at `Skill(cafleet)` § *Shutdown Protocol* with a one-sentence pointer plus the skill-specific cron-ID nuance. <!-- completed: -->
- [ ] Update `skills/design-doc-execute/roles/director.md` § *Shutdown Protocol* the same way (cron ID nuance: team-health vs augmented). <!-- completed: -->

### Step 5: Allow `cafleet:` plugin skills (S6)

- [ ] Add `Skill(cafleet:cafleet)` to `.claude/settings.json` `permissions.allow`. <!-- completed: -->
- [ ] Add `Skill(cafleet:cafleet-monitoring)` to `.claude/settings.json` `permissions.allow`. <!-- completed: -->
- [ ] Add `Skill(cafleet:design-doc)` to `.claude/settings.json` `permissions.allow`. <!-- completed: -->
- [ ] Add `Skill(cafleet:design-doc-create)` to `.claude/settings.json` `permissions.allow`. <!-- completed: -->
- [ ] Add `Skill(cafleet:design-doc-execute)` to `.claude/settings.json` `permissions.allow`. <!-- completed: -->
- [ ] Verify `.claude/settings.json` parses as valid JSON after the edits (e.g. `python -m json.tool < .claude/settings.json > /dev/null`) and that the existing global-skill entries (`Skill(design-doc-create)`, `Skill(design-doc-execute)`, `Skill(design-doc)`, `Skill(update-readme)`) and `Bash(...)` entries are unchanged. <!-- completed: -->

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-04-29 | Initial draft |
| 2026-04-29 | Added S6 / Step 5 — extend `.claude/settings.json` `permissions.allow` with the five `cafleet:` plugin skill entries (resolves `COMMENT(himkt)` from initial review). |
| 2026-04-29 | Status flipped to **Approved** by user. |
