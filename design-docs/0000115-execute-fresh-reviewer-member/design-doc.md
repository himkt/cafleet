# Fresh Reviewer Member Replaces the Copilot Review in the Execute Workflow

**Status**: Approved
**Progress**: 26/37 tasks complete
**Last Updated**: 2026-07-04

## Overview

Replace the execute workflow's GitHub Copilot review (Step 6 `@copilot` request + Step 7 Copilot loop) with a fresh, in-fleet **Reviewer** member: spawned only after every Implementation task and Success Criterion is complete, running the most intelligent model of its coding-agent backend, and driving a review-and-revise loop with the implementing members until it approves. Reviewer approval then gates the existing admin (user) approval; push + PR creation survive unchanged minus the Copilot request, and all Copilot machinery is removed from the `cafleet-design-doc` skill family.

## Success Criteria

- [ ] `skills/cafleet-design-doc/execute/execute.md` contains a Reviewer review-loop step that (a) is gated on all Implementation tasks checked AND all Success Criteria verified, (b) spawns the Reviewer at that point and no earlier, with `--model {reviewer_model}`, and (c) precedes the user-approval step.
- [ ] `skills/cafleet-design-doc/execute/roles/reviewer.md` exists and specifies the fresh-context constraint, the read-execute review scope, the `COMMENT(reviewer): [TAG]` marker protocol, and the `approved (doc)` signal.
- [ ] `grep -ri copilot` over the spawn-guard edit surface (`skills/`, `docs/`, `SPEC.md`, `README.md`, `CLAUDE.md`, project-local `.claude/rules/` + `.claude/skills/skill-author/`) returns **zero** hits for the review feature — the only permitted residual is the unrelated statistics example in `skills/cafleet-research/report/roles/researcher.md` ("among Copilot-enabled users").
- [ ] `grep -rni unconditional skills/` returns no hits — the execute-only monitoring-member delta is gone and the execute monitor spawns from the canonical `cafleet` skill `roles/monitor.md` prompt. (`.claude/skills/skill-author/SKILL.md` keeps its generic "Unconditional idle-nudge (extended)" pattern bullet — only its Copilot reference sentence is removed — so it is deliberately outside this grep.)
- [ ] `{reviewer_model}` is defined in all three overlays, `_template.md`, and the documented-defaults table; `mise //cafleet:lint-overlay` passes reporting 9 canonical tokens.
- [ ] The per-backend model tables in `skills/cafleet/reference/director.md` § *Available models per backend* carry an intelligence level per model, and each overlay's `{reviewer_model}` value equals the entry its backend's table marks highest — for codex and opencode directly (`gpt-5.5`, `opencode/gpt-5.5-pro`); for claude via the `best` alias, which the table defines as resolving to the highest available model (Fable 5 if the org has access, else the latest Opus).
- [ ] `mise //cafleet:test` and `mise //cafleet:lint-spawn-guard` pass.

---

## Background

Today's post-implementation review is outsourced to GitHub Copilot: after admin approval, Step 6 pushes the branch, opens a PR, and requests an `@copilot` review; Step 7 polls the PR on the monitoring member's idle-nudges, routes Copilot's inline comments to the still-live Programmer / Tester, and loops until Copilot reports no concerns. This has three structural costs: the review depends on an external service and `gh` auth; the review happens **after** the admin already approved, so the admin reviews unpolished work; and the monitoring member needs an execute-only "unconditional idle-nudge" delta purely to grant the Director turns while waiting on an external reviewer that never pushes into the pane.

The replacement is an in-fleet Reviewer member with **fresh context** — spawned only when implementation is finished, so it has no memory of the implementation's compromises — reviewing before the admin sees the change. Broker messages from an in-fleet Reviewer push into the Director's pane, so the monitor delta's reason disappears with Copilot.

---

## Specification

### New execute-workflow step sequence

| Step | Name | Change |
|:--|:--|:--|
| 1–3 | Resolve path / Validate & branch / Register & spawn | Unchanged, except Step 3b loses the execute-only monitor delta (canonical conditional-nudge monitor) and Step 3c's composition matrix notes the Reviewer is **never** spawned here |
| 4 | Per-step TDD cycle + Phase D verification | Unchanged |
| **5 (new)** | **Reviewer Review Loop** | Success Criteria verification (moved from old Step 5) → spawn fresh Reviewer → review-and-revise loop until `approved (doc)` |
| 6 | User (Admin) Approval | Old Step 5 minus the Success Criteria subsection; gains the re-review invariant (post-feedback revisions return to the Reviewer before re-presenting) |
| 7 | Push & Create PR | Old Step 6 minus the `@copilot` request, its verification, and `last_push_ts` |
| 8 | Finalize & Clean Up | Teardown roster gains the Reviewer; old Step 7 (Copilot loop) is deleted with no successor |

The **run-to-completion** principle and the "stop means stop" user-interjection rule apply to the Step 5 loop exactly as they applied to the old Step 7 loop: the loop is uncapped and ends only on Reviewer approval or explicit user halt/abort.

### Step 5: Reviewer Review Loop (new) — procedure

1. **Gate**: all Implementation tasks are checked (`- [x]`) and Phase D (if run) passed. The Director then verifies the design doc's `## Success Criteria` section (the procedure moved verbatim from old Step 5: inspect, check off, route shortfalls to Programmer/Tester before proceeding). Only when every criterion is checked does the Director proceed — this is the "all design-doc tasks finished" trigger.
2. **Spawn the Reviewer** (first and only time it exists in the fleet):
   - Render the spawn prompt to `${BASE}/prompts/reviewer-<UTC-compact>.md` per the existing 3e two-step audit-file pattern.
   - ```bash
     cafleet --json member create --fleet-id <fleet-id> --agent-id <director-agent-id> \
       --name "Reviewer" \
       --description "Fresh post-implementation review" \
       --model {reviewer_model} \
       --text-file ${BASE}/prompts/reviewer-<UTC-compact>.md
     ```
   - Parse `agent_id` → `<reviewer-agent-id>`. Verify `status: active` via `cafleet member list` before assigning.
3. **Assign**: `cafleet message send --fleet-id <fleet-id> --agent-id <director-agent-id> --to <reviewer-agent-id> --text "ready (doc)"`.
4. **Review pass** (Reviewer): reads the design doc, reads the full branch diff (`git diff <base-branch>...HEAD` — the base branch name is a spawn-prompt context line), and may **run** `mise //cafleet:test`, `mise //cafleet:lint`, and the other read-only mise tasks to verify claims (read-execute scope). Findings land as `COMMENT(reviewer): [TAG] <body>` markers — source-anchored findings in the source/test file at `<file>:<line>`, spec-level findings at the affected design-doc paragraph. The Reviewer then sends `complete (doc) — N issues`, or `approved (doc)` when no substantive issues remain.
5. **Route** (Director) — by marker location, same path-pattern routing the Copilot loop used:

   | Marker location | Owner | Route |
   |:--|:--|:--|
   | Test file (`**/test_*.py`, `**/*_test.py`, `**/tests/**`) | Tester — or the Programmer when no Tester was spawned (Programmer-only composition) | `ready (<file>:<line>)` to `<tester-agent-id>` (fallback: `<programmer-agent-id>`) |
   | Any other source file | Programmer | `ready (<file>:<line>)` to `<programmer-agent-id>` |
   | Design doc | Director | Resolves directly: apply the spec change, remove the marker; no cafleet route (escalate to the user via {decision_surface} only when a product decision is needed) |

   The routed member fixes the target, removes the `COMMENT(reviewer)` marker as part of the fix, re-runs tests, and replies `addressed (<file>:<line>)`.
6. **Commit** (Director, after all routed fixes report `addressed`; separate `git add` / `git commit` calls): `fix: address Reviewer feedback - <short summary>` (Programmer scope), `fix: address Reviewer test feedback - <short summary>` (Tester scope), `docs: address Reviewer feedback - <short summary>` (design-doc scope).
7. **Re-review**: send `ready (doc)` to the Reviewer again. Loop 4→7 with **no round cap** until `approved (doc)`.
8. **Dispute arbitration**: when a routed member disputes a finding, it counter-escalates with `escalating (<file>:<line>)` + a `COMMENT(programmer)`/`COMMENT(tester)` rationale at the same pointer; the Director arbitrates via `COMMENT(director): <decision> — <rationale>` exactly like the existing test-defect protocol, with the same 3-round limit before escalating to the user via {decision_surface}.

### Reviewer spawn prompt (skeleton + delta)

Built from the canonical spawn-prompt skeleton in the `cafleet` skill's `reference/director.md`, like the other three roles:

| Slot | Reviewer |
|---|---|
| ROLE TITLE | `the Reviewer` |
| role-file | `roles/reviewer.md` (new, under `execute/roles/`) |
| skill loads | the `cafleet` skill — for communication with the Director; the `cafleet-design-doc` skill — for the coordination protocol and the design-doc format (same pair as the other execute roles per §3e) |
| CONTEXT LINES | `DESIGN DOCUMENT: [INSERT DESIGN DOC PATH]` / `BASE BRANCH: [INSERT default branch name from Step 2]` |
| poll-handling line (verbatim) | `When you see cafleet message poll output with a message from the Director, act on those instructions.` |
| IMPORTANT (verbatim) | `IMPORTANT: You are a fresh reviewer with no implementation context — judge only what you can verify from the design document, the diff, and the checks you run.` / `IMPORTANT: Do NOT write or modify implementation or test code. Your only edits are COMMENT(reviewer) markers.` / `IMPORTANT: Do NOT commit. The Director handles all git operations.` / `IMPORTANT: If blocked, send a message to the Director immediately instead of assuming.` / `IMPORTANT: Read and follow .claude/rules/bash-tool.md (CAFleet-member Bash protocol) and ~/.claude/rules/bash-command.md (general Bash hygiene) for all Bash commands.` |
| start cue | `Start by reading the design document and the branch diff. Then act on the Director's ready (doc) assignment.` |

### New role file: `skills/cafleet-design-doc/execute/roles/reviewer.md`

Modeled on the create workflow's `create/roles/reviewer.md` (Required-reading table rows 1–3 identical in shape; Communication Protocol; Shutdown section) with these execute-specific sections:

- **Accountability**: fresh-context independence; review the full branch diff against the design document for (a) design-doc compliance, (b) code quality (including the project's `.claude/rules/code-quality.md` conventions), (c) test adequacy; may execute `mise` test/lint tasks to verify claims; findings as markers, never as code edits; approve only when no substantive issues remain (minor style preferences alone do not block).
- **Tag taxonomy**: reuse the existing 5 tags with code-review meanings — `[COMPLIANCE]` violates the design-doc spec, `[GAP]` missing implementation or test coverage, `[UNCLEAR]` code or doc ambiguity, `[INCORRECT]` a bug or factual error, `[IMPROVEMENT]` quality betterment.
- **Signals**: `complete (doc) — N issues` per pass; `approved (doc)` to end the loop; `blocked (doc)` with a doc-top `COMMENT(reviewer)` when review cannot proceed.
- **Front-loading**: read the entire diff before writing feedback (same guidance as the create Reviewer).

### Step 6 (User Approval) — re-review invariant

The approval interaction is unchanged (Approve / Scan for COMMENT markers / free text), minus the Success Criteria subsection (moved into Step 5) and with the Approve description updated to "push, PR creation, then finalize". One addition: after any post-feedback revision round (user `COMMENT(...)` markers or verbal feedback routed to members), the Director routes the revised change **back through the Reviewer** (`ready (doc)`; loop per Step 5.4–5.7) and re-presents to the admin only after a fresh `approved (doc)`. The Reviewer approves before the admin sees it — always.

### Step 7 (Push & Create PR) — Copilot excised

Keeps: the three 6a preconditions (gh auth / non-default branch / commits beyond base; first failure → Step 8 local-finalize), resolve owner-repo, `git push -u origin <branch-name>`, existing-PR reuse, `gh pr create --fill`, literal PR-number capture, and the error rows for push/PR-create failures (→ Step 8 local-finalize, never force-push). Removes: the `@copilot` reviewer request, the `requested_reviewers` verification, `last_push_ts` capture, and every loop-state artifact (`silence_ticks`, 7a–7e, the Copilot error rows, the Step-7 interjection section — its "stop means stop" content is already stated once for the Step 5 loop). The old Step 7 is deleted; nothing waits on the PR after creation.

### Step 8 (Finalize) — deltas

Teardown roster: monitoring member first, then Programmer, Tester, Verifier, **Reviewer**. The final user report replaces the "Copilot loop exit reason" item with the Reviewer outcome (rounds to approval) and keeps the PR URL + skipped-step reasons.

### New overlay token: `{reviewer_model}`

| Backend | `{reviewer_model}` value | Rationale (ties to the annotated model table) |
|:--|:--|:--|
| claude | `best` | Resolves to the highest-intelligence model available to the org (Fable 5 if accessible, else the latest Opus) — self-adapting "most intelligent" |
| codex | `gpt-5.5` | Newest frontier — the highest-intelligence entry in the codex table |
| opencode | `opencode/gpt-5.5-pro` | The highest-intelligence entry in the Zen catalog |
| *(documented default — overlay silent / backend unknown)* | the spawning Director's own model (inherit the parent) | Same safe floor as `{monitor_model}`, possibly intelligence-suboptimal |

**Invariant**: each overlay's `{reviewer_model}` MUST equal the model its backend's table in `reference/director.md` marks as highest intelligence (for claude, `best` is the highest-availability alias for that model). Stated in each overlay's *Note → applies at* table? **No** — a note row would force the note-anchor set check to change across all three overlays; instead the invariant is stated once, next to the annotated tables in `reference/director.md` (prose, not a note anchor). The note-anchor token set (`{decision_surface}`, `{task_coord}`) is unchanged.

### Intelligence annotation of the model tables

In `skills/cafleet/reference/director.md` § *Available models per backend*:

- **Claude table**: add an `Intelligence` column — `fable` highest, `opus` high, `sonnet` medium, `haiku` low; `best` = "resolves to the highest available (Fable 5 if the org has access, else the latest Opus)"; `default` / `opusplan` / `[1m]` variants annotated relative to their base model.
- **Codex table**: add an `Intelligence` column — `gpt-5.5` highest, `gpt-5.4` high, `gpt-5.4-mini` medium, `gpt-5.3-codex-spark` low.
- **OpenCode section**: add one sentence ranking the catalog's intelligence tiers with `gpt-5.5-pro` highest (then `gpt-5.5` / `claude-opus-4-8`, then the mid-tier `gpt-5.4` / `claude-sonnet-4-6`, then the fast tier), and the `{reviewer_model}`-equals-highest invariant sentence covering all three backends.

The human-facing operator docs (`docs/reference/coding-agents/*.md`) list models only as pass-through examples, not ranked tables — no change there (the two homes stay independent per `.claude/rules/coding-agent-overlay.md`).

### Coverage-guard changes (code)

| File | Edit |
|:--|:--|
| `cafleet/src/cafleet/coding_agent/overlay_coverage.py` | Add `"{reviewer_model}"` to `CANONICAL_TOKENS`; update the "The 8 resolvable tokens" comment and the `main()` success message ("8 canonical tokens" → "9 canonical tokens") |
| `cafleet/tests/coding_agent/test_overlay_coverage.py` | Add `"{reviewer_model}"` to `_EXPECTED_CANONICAL`; update the "canonical 8-token set" docstrings/comments; rename the count-carrying helpers/tests to count-free names (`_all_eight` → `_all_canonical`, `_overlays_all_defining_eight` → `_overlays_all_defining_canonical`, `test_canonical_token_set_is_the_eight_resolvable_tokens` → `test_canonical_token_set_matches_expected`) |

No other code changes: `cafleet member create --model` already passes any backend model through verbatim, and `spawn_prompt_guard.py` has no Copilot patterns.

### Copilot removal map (total, per the repo removal rule)

| File | Removal |
|:--|:--|
| `skills/cafleet-design-doc/execute/execute.md` | Header paragraph rewrite; Prerequisites `gh` note now cites Step 7 push/PR only; §3b monitor-delta paragraph deleted (canonical monitor prompt; the "keeps an active heartbeat" sentence drops its Copilot-turn-source rationale — supervision alone justifies the heartbeat); old Steps 6–7 Copilot content per the Step 7 spec above; Coordination-Protocol roles note → `director`, `programmer`, `tester`, `verifier`, `reviewer`, `claude` (never `drafter`; `copilot` gone); Required-reading row 3 ("the facilitation loop that drives the Step 7 Copilot wait" → drop the Copilot clause) and row 4 ("and Copilot routing — … Copilot comments get mis-routed" → "— you coordinate in free-form bodies and findings get mis-routed"); the Coordination-Protocol intro's canonical-contents list drops "Copilot routing"; Step 8 item 4's "The Director does NOT re-request Copilot review on this final docs commit" sentence deleted |
| `skills/cafleet-design-doc/execute/roles/director.md` | Accountability: monitor bullet drops the extended-routine sentence; the "Run the PR & Copilot Review loop" bullet becomes "Run the Reviewer review loop after all tasks finish" + "Push & PR after admin approval"; Commit Protocol table: the three Copilot rows → `fix: address Reviewer feedback - <short summary>` / `fix: address Reviewer test feedback - <short summary>` / `docs: address Reviewer feedback - <short summary>`; Milestones table: the PR Review row → a Reviewer Review row (expected event: Reviewer reports `complete (doc) — N issues` or `approved (doc)`; stall: Reviewer idle without a report → re-send `ready (doc)`); Required-reading row 3 drops "(and Copilot routing)"; the Communication-Protocol paragraph's "§ *Copilot Routing*" citation is removed from its section list; the COMMENT-classification section's cross-reference to § *Copilot Routing* is dropped (marker-location rules alone suffice); the per-file-detail-recovery paragraph's "Phase 7d (Copilot fix commits)" → "Step 5 (Reviewer fix commits)" |
| `skills/cafleet-design-doc/reference/coordination.md` | Delete § *Copilot Routing* and the `copilot` role row; intro + Core Principle: "source-anchored Copilot inline review" → "source-anchored review findings" (the source-file `COMMENT` marker mechanics survive for `COMMENT(reviewer)`); `<file>:<line>` pointer row and pointer-marker pairing rule reworded likewise; `reviewer` role row covers both workflows ("design-doc review findings in the create workflow; code-review findings at `<file>:<line>` in the execute workflow — both tagged with the 5-tag taxonomy"); `approved` verb row: "create and execute workflows"; Finalize-Time Cleanup item 3: `COMMENT(copilot)` → `COMMENT(reviewer)` source markers; Director Per-File Detail Recovery: "Copilot-fix" → "Reviewer-fix" commits; `addressed` verb row: "a Copilot inline comment" dropped from the marker enumeration; the complete-vs-addressed paragraph: "any `COMMENT(role)`, any Copilot line, any Director arbitration" → "any `COMMENT(role)` marker, any Director arbitration"; § *COMMENT(role) Marker* intro: "(or, for source-anchored Copilot inline review, in the source file at `file:line`)" → "(or, for source-anchored review findings, in the source file at `file:line`)"; `director` role row: "design-doc-anchored Copilot review (see *Copilot Routing* below)" removed from its enumeration |
| `skills/cafleet-design-doc/create/create.md` | § Coordination Protocol: drop "Copilot routing" from the canonical-contents list, drop `copilot` from roles-in-play, delete the "Copilot review in this skill is design-doc-anchored only…" sentence |
| `.claude/skills/skill-author/SKILL.md` | The "Unconditional idle-nudge (extended)" bullet: delete the final reference sentence ("The `cafleet-design-doc` skill's execute workflow Copilot-review loop is the reference."); the generic pattern description stays for future external-service skills |
| `docs/how-to/design-doc-development.md` | "a Director / Programmer / Tester team implements the document" → "a Director / Programmer / Tester team implements the document and a fresh Reviewer member reviews it before your approval" |
| `docs/get-started/contributing.md` | "(Director / Programmer / Tester / optional Verifier)" → "(Director / Programmer / Tester / optional Verifier, plus a fresh Reviewer at review time)" |
| `cafleet/tests/cli/test_member.py` | The `test_member_nudge__tmux_unavailable_exits_one` docstring's "(Copilot round-2)" provenance parenthetical is deleted — the removal rule forbids where-it-came-from notes in source, and it is outside Success Criterion #3's grep surface only by accident of path |

**Explicitly out of scope**: `skills/cafleet-research/report/roles/researcher.md` line 59 mentions "Copilot-enabled users" as a statistics-qualifier example unrelated to the review feature — it stays. `README.md` and `SPEC.md` carry no execute-workflow or Copilot content and no CLI/schema/API surface changes here, so they need no edits.

### Rule-file update

`.claude/rules/coding-agent-overlay.md` enumerates "the six deltas: the decision surface, the monitor model, …". Reword the second item to "the per-role model pins (monitor + reviewer)" so the count stays six and `{reviewer_model}` is covered.

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

### Step 1: Overlay token and model-intelligence reference

- [x] Add the `{reviewer_model}` row to the value tables of `skills/cafleet/reference/coding-agent/claude.md` (`best`), `codex.md` (`gpt-5.5`), and `opencode.md` (`opencode/gpt-5.5-pro`) <!-- completed: 2026-07-04T00:09 -->
- [x] Add the `{reviewer_model}` row to `skills/cafleet/reference/coding-agent/_template.md` (placeholder guidance: "most intelligent model available on this backend, per the annotated table in `reference/director.md`") <!-- completed: 2026-07-04T00:09 -->
- [x] Add the `{reviewer_model}` row to the documented-defaults table in `skills/cafleet/SKILL.md` § *Resolve your overlay* (default: the spawning Director's own model) <!-- completed: 2026-07-04T00:09 -->
- [x] Annotate the model tables in `skills/cafleet/reference/director.md` § *Available models per backend* with intelligence levels (Claude + Codex: new `Intelligence` column; OpenCode: ranking sentence) and add the `{reviewer_model}`-equals-highest invariant sentence <!-- completed: 2026-07-04T00:09 -->
- [x] Reword the delta list in `.claude/rules/coding-agent-overlay.md` to "the per-role model pins (monitor + reviewer)" <!-- completed: 2026-07-04T00:16 -->

### Step 2: Execute workflow body (`skills/cafleet-design-doc/execute/execute.md`)

- [x] Rewrite the header paragraph to the new flow (TDD → Phase D → fresh Reviewer loop → admin approval → push + PR → finalize) <!-- completed: 2026-07-04T00:26 -->
- [x] Update Prerequisites: `gh` auth needed only for the Step 7 push/PR; failure falls back to Step 8 local-finalize <!-- completed: 2026-07-04T00:26 -->
- [x] Delete the §3b "Spawn-prompt delta (execute only)" paragraph and drop the Copilot turn-source rationale from the heartbeat sentence (canonical monitor prompt) <!-- completed: 2026-07-04T00:26 -->
- [x] Note in §3c that the Reviewer is never part of the initial team composition (spawned at Step 5 only) <!-- completed: 2026-07-04T00:26 -->
- [x] Insert the new Step 5 (Reviewer Review Loop) with the gate, spawn command (`--model {reviewer_model}`, audit-file pattern, `BASE BRANCH` context line), the route table, commit conventions, re-review loop, and dispute arbitration <!-- completed: 2026-07-04T00:26 -->
- [x] Renumber old Step 5 → Step 6 (User Approval): move the Success Criteria subsection into Step 5's gate, update the Approve option description, add the re-review invariant to the Revision Loop <!-- completed: 2026-07-04T00:26 -->
- [x] Renumber old Step 6 → Step 7 (Push & Create PR) with the `@copilot` request, `requested_reviewers` verification, and `last_push_ts` removed; trim the error-handling table accordingly <!-- completed: 2026-07-04T00:26 -->
- [x] Delete old Step 7 (Copilot Review Loop) including 7a–7e, loop state, and the interjection section; fold "stop means stop" into Step 5's loop intro <!-- completed: 2026-07-04T00:26 -->
- [x] Update Step 8: teardown roster includes the Reviewer; final report replaces the Copilot exit reason with the Reviewer outcome; delete item 4's "does NOT re-request Copilot review" sentence <!-- completed: 2026-07-04T00:26 -->
- [x] Sweep every step-number cross-reference in execute.md to the new numbering: the run-to-completion paragraph ("the Step 5 user-approval gate" / "'stop means stop' halt during Step 7"), Phase D's "Proceed directly to Step 5 (User Approval)", Step 8's intro ("Runs after Step 7 exits, or directly after Step 5 when Step 6 was skipped"), the Abort Flow, and every Error-Handling row naming Steps 5/6/7 <!-- completed: 2026-07-04T00:26 -->
- [x] Strip execute.md's residual Copilot mentions per the removal map: Required-reading rows 3–4 and the Coordination-Protocol intro's canonical-contents list <!-- completed: 2026-07-04T00:26 -->
- [x] Update the § Coordination Protocol roles-in-play note (add `reviewer`, drop `copilot`) and the role table at the top (add the Reviewer row: spawned at Step 5, reviews diff + runs checks, writes markers; does NOT write code/tests, commit, or talk to the user) <!-- completed: 2026-07-04T00:26 -->

### Step 3: New Reviewer role file

- [x] Create `skills/cafleet-design-doc/execute/roles/reviewer.md` per the Specification (Required-reading block, fresh-context accountability, read-execute scope, 5-tag code-review taxonomy, signals, communication protocol, shutdown) <!-- completed: 2026-07-04T00:30 -->
- [x] Add the Reviewer spawn-prompt delta table (skeleton slots) to execute.md §3e-style listing inside the new Step 5 <!-- completed: 2026-07-04T00:30 -->

### Step 4: Execute Director role file (`skills/cafleet-design-doc/execute/roles/director.md`)

- [x] Update the monitor accountability bullet (drop the extended-routine sentence) <!-- completed: 2026-07-04T00:32 -->
- [x] Replace the PR & Copilot loop accountability bullet with the Reviewer-loop + push/PR bullets <!-- completed: 2026-07-04T00:32 -->
- [x] Replace the three Copilot commit-message rows with the three Reviewer rows <!-- completed: 2026-07-04T00:32 -->
- [x] Replace the PR Review milestone row with the Reviewer Review row <!-- completed: 2026-07-04T00:32 -->
- [x] Strip director.md's residual Copilot mentions per the removal map: Required-reading row 3's "(and Copilot routing)", the Communication-Protocol paragraph's § *Copilot Routing* citation, the COMMENT-classification section's cross-reference to § *Copilot Routing*, and "Phase 7d (Copilot fix commits)" → "Step 5 (Reviewer fix commits)" in the per-file-detail-recovery paragraph <!-- completed: 2026-07-04T00:32 -->

### Step 5: Coordination protocol and create workflow

- [x] Apply the coordination.md removal map (delete § Copilot Routing + `copilot` role row; reword intro/Core Principle/pointer rows; extend `reviewer` + `approved` rows; fix Finalize-Time Cleanup item 3 and Per-File Detail Recovery) <!-- completed: 2026-07-04T00:35 -->
- [x] Apply the create.md removal map (roles-in-play, canonical-contents list, design-doc-anchored sentence) <!-- completed: 2026-07-04T00:35 -->

### Step 6: Peripheral docs

- [ ] Remove the Copilot reference sentence from the "Unconditional idle-nudge (extended)" bullet in `.claude/skills/skill-author/SKILL.md` <!-- completed: -->
- [ ] Update the execute-workflow team description in `docs/how-to/design-doc-development.md` <!-- completed: -->
- [ ] Update the execute-workflow team parenthetical in `docs/get-started/contributing.md` <!-- completed: -->

### Step 7: Coverage-guard code

- [x] Add `"{reviewer_model}"` to `CANONICAL_TOKENS` in `cafleet/src/cafleet/coding_agent/overlay_coverage.py`; update the count comment and the `main()` success message to 9 <!-- completed: 2026-07-04T00:09 -->
- [ ] Update `cafleet/tests/coding_agent/test_overlay_coverage.py`: add the token to `_EXPECTED_CANONICAL`, update docstrings/comments, rename the count-carrying helpers and test to count-free names <!-- completed: -->
- [ ] Delete the "(Copilot round-2)" provenance parenthetical from the `test_member_nudge__tmux_unavailable_exits_one` docstring in `cafleet/tests/cli/test_member.py` <!-- completed: -->

### Step 8: Verification sweep

- [ ] `grep -ri copilot` over the repo excluding `design-docs/`, `researches/`, `node_modules/`, `.git/` — zero feature hits (the researcher.md statistics example is the only permitted residual) <!-- completed: -->
- [ ] `grep -rni unconditional skills/` — zero hits <!-- completed: -->
- [ ] `mise //cafleet:lint-overlay` passes reporting 9 canonical tokens <!-- completed: -->
- [ ] `mise //cafleet:test` passes <!-- completed: -->
- [ ] `mise //cafleet:lint-spawn-guard` passes <!-- completed: -->

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-07-03 | Initial draft |
