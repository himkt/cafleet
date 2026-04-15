# `cafleet-design-doc-execute` PR & Copilot Review Loop

**Status**: Approved
**Progress**: 0/22 tasks complete
**Last Updated**: 2026-04-15

## Overview

Extend `/cafleet-design-doc-execute` so that, after user approval, the Director automatically pushes the feature branch, opens a PR via `gh pr create --fill`, requests a Copilot review, and then runs a cron-driven review loop: on each tick it polls the PR for new Copilot feedback, routes actionable comments to the still-live Programmer / Tester via `cafleet send`, pushes the fix, re-requests review, and exits when Copilot has been quiescent for N ticks or has approved the PR. Only after the review loop exits does the Director mark the design doc Complete, push the final docs commit, and shut the team down.

## Success Criteria

- [ ] After the user selects "Approve" in Step 5, the Director performs `git push -u origin <branch>` (if unpushed) and `gh pr create --fill` without further prompting, and records the resulting PR number literally for all subsequent `gh` calls (no shell variables).
- [ ] Immediately after `gh pr create` succeeds, the Director runs `gh pr edit <pr-number> --add-reviewer @copilot` and verifies the request via `gh api repos/<owner>/<repo>/pulls/<pr-number>/requested_reviewers`.
- [ ] The Director replaces the team-health `/loop` with an augmented one that additionally polls PR review state (`gh pr view <pr-number> --json reviews` + `gh api repos/<owner>/<repo>/pulls/<pr-number>/comments`) each tick, and the swap happens create-before-delete on entry to Step 7 (start augmented `/loop` first, then `CronDelete` the old team-health cron ID) to avoid a monitoring gap.
- [ ] When a new Copilot review arrives (`submittedAt` from `gh pr view --json reviews` or `created_at` from `gh api .../comments` > `last_push_ts`, AND the corresponding login field — `author.login` for `gh pr view` reviews, `user.login` for `gh api` inline comments — matches `^copilot` case-insensitive), the Director classifies each inline comment by file path (source → Programmer, test → Tester, design-doc → Director-direct) and dispatches via `cafleet --session-id <session-id> send --agent-id <director-agent-id> --to <member-agent-id>`; no user prompt is required for routine routing.
- [ ] After each fix the Director commits, runs `git push`, and re-requests Copilot review via `gh pr edit <pr-number> --add-reviewer @copilot`, incrementing `round` and updating `last_push_ts` to the post-push timestamp.
- [ ] The loop exits when any of these hold: (a) the latest entry in `gh pr view --json reviews` whose `author.login` matches `^copilot` has `state == "APPROVED"`, (b) 5 consecutive loop ticks have elapsed since `last_push_ts` with no new Copilot review or inline comment (Copilot's first pass can take several minutes — 5 ticks gives comfortable slack), or (c) `round >= 5` in which case the Director escalates to the user via `AskUserQuestion`. `reviewDecision` at PR level is NOT used as the exit signal because Copilot is often not a required reviewer, so that field does not reliably flip to APPROVED on Copilot-only approvals.
- [ ] Step 8 (renamed from old Step 6: Finalize & Clean Up) runs only after Step 7 exits, writes `docs: mark design doc as complete`, pushes the commit whenever `git rev-parse --abbrev-ref <branch-name>@{upstream}` succeeds (i.e. the branch is already tracked on origin), then cancels the `/loop`, deletes members, and deregisters — in that order.
- [ ] When `gh auth status` fails, the current branch equals the default branch, or `gh pr create` fails, the Director aborts Steps 6 and 7 with a user-visible error message and proceeds directly to Step 8 (finalize locally) without creating a PR.
- [ ] `.claude/skills/cafleet-design-doc-execute/SKILL.md` and `roles/director.md` document the new Steps 6, 7, 8 verbatim, including the commit-message table, the loop prompt template, the classification rules, and the exit conditions.
- [ ] The existing Agent Teams `/design-doc-execute` skill is NOT modified (scope: CAFleet variant only).

---

## Background

Today the execute flow ends with a local commit (`docs: mark design doc as complete`) and a team shutdown. The user pushes, opens the PR, and requests Copilot review manually. Every manual handoff is a place where a run stalls — the team is already deleted by the time the first Copilot comment arrives, so addressing feedback means starting a new session and re-uploading context.

Copilot PR review is the effective final quality gate for this repo (see `.claude/rules/skill-discovery.md`: `@copilot` is the project-standard reviewer slug). Integrating the review loop into the execute skill means the TDD team stays alive long enough to resolve first-pass review comments while design-doc context is still loaded in each member's terminal.

The `cafleet-monitoring` `/loop` already runs every minute; extending its prompt to also poll PR review state reuses the existing cron firing rather than adding a second scheduler.

---

## Specification

### Workflow Shape

Current (before this change):

```
Step 5 Approval  →  Step 6 Finalize (mark Complete, commit, shutdown)
```

New:

```
Step 5 Approval
  ↓
Step 6 Push & Create PR         (git push -u, gh pr create --fill, add @copilot)
  ↓
Step 7 Copilot Review Loop      (cron-driven: poll → route → fix → push → re-request)
  ↓
Step 8 Finalize & Clean Up      (mark Complete, commit, push, cancel loop, shutdown)
```

Steps 1–5 are unchanged in logic. Step 5's "Approve" option now carries an extended semantic: selecting Approve authorises Steps 6 + 7 + 8 in one sweep. If the user wants to approve without pushing, they use the "Other" option and state so explicitly; the Director's existing LLM intent judgment handles this by skipping Steps 6 + 7 and proceeding directly to Step 8 (local finalize).

### Step 5 Approval Semantic Extension

Update the approval option description in the SKILL.md table:

| Option | Label | Description (new) | Behavior |
|:--|:--|:--|:--|
| 1 | **Approve** | Proceed with push, PR creation, Copilot review loop, then finalize | Steps 6 → 7 → 8 |
| 2 | **Scan for COMMENT markers** | Add `COMMENT(name): feedback` markers to the changed source files, then select this option | Revision loop (unchanged) |
| 3 | *(Other — built-in)* | *(Free text input, e.g. "approve but skip PR")* | Intent judgment: "approve-local" skips to Step 8 local finalize, "abort" triggers Abort Flow |

No new option buttons — all behavior is reachable through existing Approve / Other.

### Step 6: Push & Create PR

Preconditions (checked in order; first failure aborts to Step 8 local-finalize with a user-visible message):

| Check | Command | Failure action |
|:--|:--|:--|
| `gh` authenticated | `gh auth status` | Report "gh not authenticated; skipping PR creation" → Step 8 |
| Not on default branch | `git branch --show-current` vs `gh repo view --json defaultBranchRef --jq '.defaultBranchRef.name'` | Report "on default branch; cannot open PR" → Step 8 |
| Branch has commits beyond base | `git log <base>..HEAD --oneline` | Report "no commits to push" → Step 8 |

Procedure (each command is a separate Bash call; do NOT chain with `&&`):

1. **Resolve owner/repo**: `gh repo view --json nameWithOwner --jq '.nameWithOwner'` — capture the literal `<owner>/<repo>` string (e.g. `himkt/cafleet`) and substitute it into every `gh api repos/<owner>/<repo>/...` call below. Like the PR number, this is a literal string — no shell variables.
2. `git push -u origin <branch-name>` — initial push. If this fails (non-fast-forward, branch protection, etc.), report the exact stderr to the user and proceed to Step 8. Do NOT force-push.
3. Check for an existing PR on this branch: `gh pr list --head <branch-name> --json number --jq '.[0].number // empty'`. If the result is non-empty, reuse that PR number. Otherwise, run `gh pr create --fill` and parse the printed URL's trailing number.
4. Store the PR number as a literal string (e.g. `42`) and substitute it into `<pr-number>` in every subsequent command. Do NOT use a shell variable — `permissions.allow` matches literal command strings.
5. `gh pr edit <pr-number> --add-reviewer @copilot` — request Copilot review.
6. Verify: `gh api repos/<owner>/<repo>/pulls/<pr-number>/requested_reviewers` should list Copilot. If Copilot is absent from the response AND no Copilot review already exists (`gh pr view <pr-number> --json reviews`), report "Copilot reviewer unavailable" and proceed to Step 8 local-finalize.
7. Record `last_push_ts` as the ISO 8601 timestamp of the push completion (use the Director's wall-clock time captured immediately after step 2 returned, or run `date -u +%Y-%m-%dT%H:%M:%SZ`).

### Step 7: Copilot Review Loop

#### 7a. Replace the monitoring `/loop`

On entry to Step 7:

1. Start a new `/loop` with the augmented prompt (template in the Loop Prompt section below). Record the new cron ID.
2. `CronDelete` the existing team-health loop (cron ID recorded in Step 3c).
3. The new loop keeps the team-health checks AND adds PR review polling.

**Order matters**: create-before-delete eliminates any window where no monitor is running. A one-tick overlap (both loops firing for one minute) is harmless — the Director just receives two nudge prompts and reconciles them trivially.

On exit from Step 7 (any exit condition), keep the augmented loop running — Step 8's shutdown is responsible for the final `CronDelete`.

#### 7b. Per-tick procedure

On each wake-up (1-minute interval), the Director runs — in order:

1. **Team health** (unchanged from `cafleet-monitoring`): `member list` → `poll` → `member capture` fallback → nudge stalled members.
2. **Fetch new PR reviews**: `gh pr view <pr-number> --json reviews` (GraphQL-shaped; fields are `author.login`, `state`, `submittedAt`, `body`) and `gh api repos/<owner>/<repo>/pulls/<pr-number>/comments` (REST-shaped; fields are `user.login`, `body`, `path`, `line`, `created_at`).
3. **Filter Copilot-authored entries**: keep items where the login field (`author.login` for `gh pr view` reviews, `user.login` for `gh api` inline comments) matches the regex `^copilot` (case-insensitive). Copilot reviews currently post under a login that begins with `copilot` — the exact slug varies by account plan, so prefix match is the safe filter.
4. **New-since-push check**: keep items whose timestamp (`submittedAt` for reviews, `created_at` for inline comments) is strictly later than `last_push_ts`.
5. **Branch by result**:

| Result | Action |
|:--|:--|
| The most recent Copilot-authored entry in `reviews` has `state == "APPROVED"` | Exit loop (success) → Step 8 |
| 0 new Copilot items AND `ticks_since_last_new_review >= 5` | Exit loop (quiescent) → Step 8 |
| 0 new Copilot items AND `ticks_since_last_new_review < 5` | Increment counter, continue |
| ≥ 1 new Copilot items | Go to 7c |

Why 5 ticks (not 3): Copilot's first review after a push can take 3–5 minutes. 3 ticks risks declaring quiescence while Copilot is still composing its response. 5 ticks (~5 minutes) gives the model comfortable headroom without dragging the session out indefinitely.

Why not `reviewDecision`: the PR-level `reviewDecision` only reflects required reviewers (typically CODEOWNERS). Copilot is usually not a CODEOWNER, so an approve from Copilot alone leaves `reviewDecision` null/REVIEW_REQUIRED. Reading the Copilot-specific entry in the `reviews` array is the reliable signal.

#### 7c. Classify and route

For each new inline comment:

| Path pattern | Owner | Route |
|:--|:--|:--|
| Design doc (`design-docs/**/design-doc.md`) | Director | Director applies directly, no route |
| Test file (matches project test globs, e.g. `**/test_*.py`, `**/*_test.py`, `**/tests/**`) | Tester | `cafleet --session-id <session-id> send --agent-id <director-agent-id> --to <tester-agent-id> --text "Copilot review: <file>:<line> — <comment body>. Please address."` |
| Any other source file | Programmer | `cafleet --session-id <session-id> send --agent-id <director-agent-id> --to <programmer-agent-id> --text "Copilot review: <file>:<line> — <comment body>. Please address."` |

For review-level comments (body text not attached to a line), route by Director judgment: if it's spec-level → Director resolves; if it's implementation-level → Programmer; if it's test-level → Tester.

#### 7d. Fix, commit, push, re-request

1. Wait for each routed member to report completion via `cafleet poll`. Members do not commit; the Director commits after each report.
2. Commit fixes per scope (separate `git add` and `git commit` calls; each its own Bash call, no `&&`):
   - Programmer fixes: `git commit -m "fix: address Copilot review - <short summary>"`
   - Tester fixes: `git commit -m "fix: address Copilot test review - <short summary>"`
   - Director doc fixes: `git commit -m "docs: address Copilot review - <short summary>"`
3. `git push` (no flags — branch already tracks origin from Step 6).
4. Update `last_push_ts` to the post-push wall-clock timestamp. Reset `ticks_since_last_new_review = 0`. Increment `round`.
5. `gh pr edit <pr-number> --add-reviewer @copilot` — re-request review. (Re-adding the same reviewer triggers a fresh Copilot pass.)
6. Continue the loop.

#### 7e. Round limit

When `round >= 5`, break the auto-loop and escalate to the user via `AskUserQuestion`:

| Option | Behavior |
|:--|:--|
| 1. Continue | Reset `round = 0`, resume Step 7 |
| 2. Finalize now | Exit loop → Step 8 (accept remaining Copilot comments as-is) |
| 3. *(Other)* | Intent judgment; abort-intent → Abort Flow |

### Step 8: Finalize & Clean Up (renamed from old Step 6)

1. Update design doc Status → "Complete", add Changelog entry.
2. `git add <design-doc>` (separate Bash call)
3. `git commit -m "docs: mark design doc as complete"` (separate Bash call)
4. Push decision (separate Bash call):
   - Run `git rev-parse --abbrev-ref <branch-name>@{upstream}`. Exit code 0 means the branch is tracked on origin.
   - If tracked: `git push`. This covers both the "Step 6 fully succeeded" path and the "Step 6 partial-fail (push OK, PR create failed)" path, so the final docs commit is never orphaned locally when the branch is already on origin.
   - If not tracked (Step 6 aborted before the `git push -u`): skip the push. The docs commit stays local.
5. `CronDelete` the currently active `/loop` monitor — whichever one is running (team-health from Step 3c if Step 6 was skipped, augmented from Step 7 otherwise).
6. `cafleet --session-id <session-id> member delete --agent-id <director-agent-id> --member-id <programmer-agent-id>` (+ tester / verifier if spawned).
7. `cafleet --session-id <session-id> deregister --agent-id <director-agent-id>`.
8. Report to the user: PR URL (if created), review-round summary (rounds used, exit reason), and any skipped-step reasons.

The final docs commit lands on the PR branch but the Director does NOT re-request Copilot review on it — docs status changes are not worth another review round.

### Loop Prompt Template (augmented)

Replaces the template in `cafleet-monitoring` SKILL.md only for the duration of Step 7. The Director substitutes literal UUIDs and the literal PR number.

```
Monitor team health AND PR review state (interval: 1 minute).

TEAM HEALTH:
1. Run `cafleet --session-id <session-id> --json member list --agent-id <director-agent-id>`.
2. Run `cafleet --session-id <session-id> --json poll --agent-id <director-agent-id> --since "<ISO 8601 timestamp of last check>"`. ACK progress reports.
3. For each member that has not sent a message since last check, run `cafleet --session-id <session-id> member capture --agent-id <director-agent-id> --member-id <member-agent-id> --lines 200`.
4. Nudge stalled members via `cafleet --session-id <session-id> send --agent-id <director-agent-id> --to <member-agent-id> --text "Report your progress now. If blocked, state what is blocking you."`.

PR REVIEW:
5. Run `gh pr view <pr-number> --json reviews` (GraphQL shape: `author.login`, `state`, `submittedAt`, `body`).
6. Run `gh api repos/<owner>/<repo>/pulls/<pr-number>/comments` (REST shape: `user.login`, `body`, `path`, `line`, `created_at`).
7. Filter to entries where the appropriate login field (`author.login` for GraphQL reviews, `user.login` for REST inline comments) starts with `copilot` (case-insensitive) and the appropriate timestamp (`submittedAt` / `created_at`) > `<last-push-timestamp>`.
8. If the most recent Copilot-authored entry in `reviews` has `state == "APPROVED"`: signal Step 7 exit (success).
9. If filter returned 0 entries for 5 consecutive ticks: signal Step 7 exit (quiescent).
10. If filter returned ≥ 1 entries: classify by file path and dispatch via `cafleet --session-id <session-id> send --agent-id <director-agent-id> --to <member-agent-id> --text "Copilot review: <file>:<line> — <body>. Please address."`.

ESCALATION:
11. If any member has been nudged 2 times with no progress, escalate to the user.
12. If `round >= 5`, escalate to the user with the Continue / Finalize-now / Other prompt.
```

The Director holds `last_push_ts`, `ticks_since_last_new_review`, and `round` as in-context variables across loop firings — these are not persisted to disk.

### Commit Message Additions

Add the following rows to the Commit Protocol Summary in `roles/director.md`:

| Event | Commit Message Format |
|:--|:--|
| Fix routed to Programmer (Copilot review) | `fix: address Copilot review - <short summary>` |
| Fix routed to Tester (Copilot review) | `fix: address Copilot test review - <short summary>` |
| Design-doc fix by Director (Copilot review) | `docs: address Copilot review - <short summary>` |

### Error Handling

| Case | Detection | Behavior |
|:--|:--|:--|
| `gh auth status` fails | Step 6 precondition check | Skip Step 6 + 7, go directly to Step 8 local-finalize |
| On default branch | Step 6 precondition check | Skip Step 6 + 7, go directly to Step 8 local-finalize |
| No commits beyond base | Step 6 precondition check | Skip Step 6 + 7, go directly to Step 8 local-finalize |
| `git push` rejected | stderr of `git push` | Report exact stderr to user, skip Step 7, go to Step 8 local-finalize. NEVER force-push. |
| `gh pr create` fails | stderr of `gh pr create` | Report, skip Step 7, go to Step 8 local-finalize |
| `@copilot` reviewer unavailable | `gh api .../requested_reviewers` shows no Copilot AND no prior Copilot review | Report "Copilot reviewer unavailable for this PR"; skip Step 7; go to Step 8 |
| Fix-push fails mid-loop (round > 0) | stderr of `git push` | Escalate to user (AskUserQuestion: retry / finalize now / abort) |
| Round limit reached (`round >= 5`) | Counter check in loop | AskUserQuestion — see Round Limit section |
| User selects "Other" in Step 5 with abort-intent text | Existing LLM intent judgment | Abort Flow (unchanged — no push) |
| User selects "Other" in Step 5 with approve-local intent | Existing LLM intent judgment, extended | Skip Steps 6 + 7; go to Step 8 local-finalize |

### User Interjection During Step 7

`/loop` firings keep arriving while the user is speaking to the Director. The Director obeys the project's "Stop means stop" rule (`.claude/rules/skill-discovery.md`): when the user signals halt (explicit "stop", "wait", profanity / frustration, repeated rejection of tool calls), the Director:

1. Stops dispatching new `cafleet send` / `git commit` / `git push` / `gh` actions immediately.
2. Acknowledges the user briefly and waits for explicit instructions.
3. Treats subsequent `/loop` firings as notification-only — runs the PR review poll for situational awareness but does NOT route comments, commit, or push until the user re-engages with a specific instruction.
4. Does NOT silently tear the team down — the state stays paused so the user can resume or explicitly abort.

If the user explicitly aborts, follow the existing Abort Flow (update doc Status → "Aborted", commit, run Shutdown Protocol). Step 7's cleanup is identical to Step 8's cleanup in that case — `CronDelete` the augmented loop, delete members, deregister.

### Scope: CAFleet skill only

This change modifies ONLY:

- `.claude/skills/cafleet-design-doc-execute/SKILL.md`
- `.claude/skills/cafleet-design-doc-execute/roles/director.md`

It does NOT touch:

- The global Agent Teams `/design-doc-execute` skill under `~/.claude/skills/`.
- `cafleet-monitoring` SKILL.md (the augmented loop template lives inside `cafleet-design-doc-execute`; `cafleet-monitoring`'s own template stays team-health-only and remains the template used by create / other skills).
- `roles/programmer.md`, `roles/tester.md`, `roles/verifier.md` — they already accept arbitrary `cafleet send` directives and need no structural change; Copilot review feedback is just another directive.
- Any Python code in `cafleet/`, `admin/`, `ARCHITECTURE.md`, `README.md`, or `docs/`. This is a pure skill-documentation change.

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-04-15T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

### Step 1: SKILL.md — Step 5 approval semantic extension

- [ ] Update the Step 5 option table in `.claude/skills/cafleet-design-doc-execute/SKILL.md` so Option 1 (Approve) says "Proceed with push, PR creation, Copilot review loop, then finalize" and references Steps 6 → 7 → 8 <!-- completed: -->
- [ ] Add an explicit "approve-local" branch to the Step 5 "Other" intent-judgment description (skip Steps 6 + 7, go to Step 8) <!-- completed: -->

### Step 2: SKILL.md — Step 6 Push & Create PR

- [ ] Insert a new "Step 6: Push & Create PR" section after Step 5 and before the old Step 6 <!-- completed: -->
- [ ] Include the preconditions table (gh auth / default branch / commits-beyond-base) with explicit fallthrough to Step 8 local-finalize <!-- completed: -->
- [ ] Document the 6-step procedure (push → check/create PR → record number literal → add @copilot → verify → capture `last_push_ts`), including the "no shell variables, literal PR number" rule <!-- completed: -->

### Step 3: SKILL.md — Step 7 Copilot Review Loop

- [ ] Insert a new "Step 7: Copilot Review Loop" section <!-- completed: -->
- [ ] Document 7a (loop replacement via create-before-delete: start augmented `/loop` first, then `CronDelete` the team-health cron ID), 7b (per-tick procedure including the filter table and the 5-tick quiescence rule), 7c (classification & routing table including review-level comment judgment), 7d (fix/commit/push/re-request), 7e (round limit escalation with AskUserQuestion) <!-- completed: -->
- [ ] Embed the augmented loop prompt template, the "Error Handling" table, and the "User Interjection During Step 7" subsection verbatim inside Step 7 of the SKILL.md <!-- completed: -->

### Step 4: SKILL.md — Step 8 Finalize & Clean Up

- [ ] Rename the old "Step 6: Finalize & Clean Up" section to "Step 8: Finalize & Clean Up" and update the numbering references throughout the file <!-- completed: -->
- [ ] Add the `git push` sub-step between the commit and `CronDelete` steps, with the conditional "only if Step 6 succeeded" note <!-- completed: -->
- [ ] Update the user-facing end-of-run report to include the PR URL and review-round summary when Step 6 ran <!-- completed: -->

### Step 5: SKILL.md — cross-cutting updates

- [ ] Update the opening summary paragraph and the "## Process" ToC-style numbering to reflect Steps 1 – 8 <!-- completed: -->
- [ ] Update the Prerequisites section if applicable (no new prerequisites beyond `gh auth status`; keep the tmux requirement) <!-- completed: -->
- [ ] Add a "PR Review Loop State" sub-section near Step 7 documenting the three in-context variables (`last_push_ts`, `ticks_since_last_new_review`, `round`) <!-- completed: -->

### Step 6: roles/director.md

- [ ] Add a new "PR & Copilot Review" bullet to "Your Accountability" covering: open PR, request @copilot, run augmented monitoring, classify + route, fix + push + re-request, exit on quiescence/approval/round-limit, then finalize <!-- completed: -->
- [ ] Extend the "Commit Protocol Summary" table with the three new rows (`fix: address Copilot review - …`, `fix: address Copilot test review - …`, `docs: address Copilot review - …`) <!-- completed: -->
- [ ] Extend the "Progress Monitoring" milestones table with a new "PR Review" phase row (expected event: Copilot posts review; stall indicator: no new Copilot activity in 3 ticks; Director action: evaluate exit conditions vs route new comments) <!-- completed: -->
- [ ] Update the "Shutdown Protocol" to note that the `CronDelete` target is whichever loop is active (team-health in Step 3–6, augmented in Step 7), and that shutdown runs only after Step 8's final push <!-- completed: -->

### Step 7: Verification

- [ ] Re-read the edited SKILL.md end-to-end and confirm every `--session-id <session-id>`, `--agent-id <…-agent-id>`, and `<pr-number>` reference uses the literal-placeholder convention (no `$VAR` or `${VAR}` forms outside of quoted prose examples) <!-- completed: -->
- [ ] Grep for stale "Step 6" / "Step 7" references elsewhere in the skill tree (`.claude/skills/cafleet-design-doc-execute/`) and update or explain each hit <!-- completed: -->
- [ ] Cross-check `roles/director.md` commit table matches the SKILL.md commit-message formats verbatim <!-- completed: -->
- [ ] Verify that `cafleet-monitoring` SKILL.md's original loop prompt is NOT modified (the augmented version lives only in `cafleet-design-doc-execute` SKILL.md) <!-- completed: -->

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-04-15 | Initial draft |
