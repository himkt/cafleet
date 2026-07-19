# 0000144 — Consolidate Doc-Cleanup Skills into `clean-docs` (Three Workflows)

**Status**: Complete
**Progress**: 16/16 tasks complete
**Last Updated**: 2026-07-19

## Overview

Consolidate the two project-local cleanup skills `.claude/skills/clean-docs` and `.claude/skills/historical-residue-cleanup` into ONE skill — `clean-docs` — with THREE workflows (`residue`, `affirmative`, `simplification`), following the repo's established multi-workflow skill pattern (`skills/cafleet-design-doc` with create/execute/interview). The shared CAFleet orchestration spine (Director + monitor first-in + N disjoint-slice scanners + reviewer gate, exempt set, invariants, per-run process, spawn skeleton) is stated exactly once in the umbrella `SKILL.md`; each workflow body carries only its distinct mechanics and guarantees. Per `~/.claude/rules/removal.md`, the `historical-residue-cleanup` name and directory vanish entirely — after this change the repository reads as if the merged skill always existed.

## Success Criteria

- [x] The merged skill exists at `.claude/skills/clean-docs/` with an umbrella `SKILL.md` and three workflow directories (`residue/`, `affirmative/`, `simplification/`), each with `<workflow>/<workflow>.md` and per-workflow `roles/` (scanner, reviewer) — matching the `skills/cafleet-design-doc` dispatcher + `<workflow>/<workflow>.md` + `<workflow>/roles/` shape — and extending that shape with per-workflow `reference/` pages.
- [x] The shared orchestration spine (team shape, per-run process, spawn-prompt skeleton, exempt set, invariants, coordination extensions, `${BASE}` convention, backend-neutrality) is stated **once** in the umbrella `SKILL.md`; no workflow body or role file restates it.
- [x] Exactly one workflow runs per invocation, dispatched by user phrasing; each workflow's trigger phrasing is distinct (§2) and the frontmatter `description` triggers all three families.
- [x] The class-to-workflow split is explicit (§3): residue owns classes (a)/(b)/(c); affirmative owns P1/P2/P4; simplification owns P3/P5 — with the stated boundary tie-break for candidates matching more than one workflow.
- [x] The three invariants (no contract/behavior lost, no live test coverage lost, no new narration) apply to every workflow; the P4 fallback→fail-fast behavior-change carve-out exists **only** in the affirmative workflow — residue and simplification runs are strictly zero-behavior-change.
- [x] The residue workflow preserves its guarantees verbatim in meaning: multi-pass pattern-catalog sweep with hand-inspection, `inventory.md` audit record (file→action tables + KEEP list + known-benign subsection), and provable completeness (post-apply re-sweep yields zero unaccounted matches).
- [x] The judgment workflows (affirmative, simplification) preserve theirs: full-file judgment read, apply-ready rows per the shared row format, merged `findings.md` review gate, and the observations escalation channel.
- [x] Removal is total: `git grep -i "historical-residue"` over the tracked tree minus `design-docs/` returns zero hits; `.claude/skills/historical-residue-cleanup/` and the old single-skill layout files (`roles/scanner.md`, `roles/reviewer.md`, `reference/rubric.md` at the `clean-docs` root) are deleted in the same change, and the final `.claude/skills/clean-docs/` tree matches the §6 layout exactly.
- [x] Backend-neutrality is preserved: `SKILL.md`, workflow bodies, and `roles/*.md` use only `{…}` overlay tokens for backend-varying behavior; every spawn prompt carries the `CODING AGENT: {coding_agent}` line; relative overlay/base-dir/coordination links are correct at the new directory depths.
- [x] The skill loads via the Skill tool with no `{token}` leaks in any user-facing string. No dogfood run is required.

---

## Background

Both existing skills run the identical CAFleet team shape and share the same exempt set, coordination extensions, per-run process, and spawn skeleton — that boilerplate exists twice today and drifts independently. They differ only in mechanics and guarantees: `historical-residue-cleanup` is a pattern-catalog grep sweep with provable completeness targeting historical narration and removal-sentinel tests; `clean-docs` is a full-file judgment review producing apply-ready rewrites per a P1–P5 rubric. The user additionally wants the judgment skill's dual mandate split, so the consolidation produces **three** workflows: residue, affirmative (affirmative-writing enforcement: prohibition-piles, unpaired prohibitions, code fail-fast), and simplification (de-duplication and tightening). The `update-readme` skill stays separate and is out of scope.

---

## Specification

### 1. Skill identity

| Property | Value |
|---|---|
| Name (frontmatter + skill-loader name) | `clean-docs` |
| Location | `.claude/skills/clean-docs/` (project-local skills dir) |
| Workflows | `residue`, `affirmative`, `simplification` — exactly one per run |
| Orchestration | CAFleet team — Director + monitor (first-in) + N scanners + 1 reviewer, identical for all three workflows |
| Backend-neutrality | `SKILL.md`, workflow bodies, and `roles/*.md` are backend-neutral; deltas resolve via `skills/cafleet/reference/coding-agent/<name>-overlay.md` |

Members of every workflow load the skill by its name `clean-docs` via their backend's skill-loader. The frontmatter `description` covers all three trigger families and carries the members-load line.

### 2. Dispatch

The umbrella `SKILL.md` is a dispatcher, like `skills/cafleet-design-doc/SKILL.md`: it routes the user's request into one workflow body by phrasing, then that body's orchestration runs in full over the shared spine.

| Workflow | Distinct trigger phrasing |
|---|---|
| **residue** | "clean up historical narration / historical residue", "remove deprecation notes", remove "this replaces X" / "renamed from Y" / "previously … now …" prose, "drop design-number provenance citations", "delete removal-sentinel tests" |
| **affirmative** | "run an affirmative-writing sweep", "fix prohibition-only rule sections", "pair prohibitions with affirmatives", "remove meaningless fallbacks", "make code fail fast" |
| **simplification** | "simplify the docs / comments", "tighten verbose prose", "de-duplicate documentation", "remove redundant comments" |

When a request names families from more than one workflow, the Director asks the user which single workflow to run via `{decision_surface}` — one workflow per run, never a combined pass.

### 3. Class-to-workflow split

Every candidate a scanner may act on belongs to exactly one workflow. The class identifiers stay stable (residue's (a)/(b)/(c); the judgment classes P1–P5):

| Workflow | Owns | Fix character |
|---|---|---|
| **residue** | (a) sentinel test, (b) narration/citation/trajectory, (c) keep + known-benign | Delete sentinel framing; reword narration to present tense. Strictly zero behavior change. |
| **affirmative** | **P1** prohibition-pile, **P2** unpaired prohibition (including negatively-phrased instructions rewritable in affirmative voice), **P4** meaningless fallback / swallowed error | Fixes that change prohibition structure, voice, or code behavior. P4 rows are BEHAVIOR-AFFECTING: each names the guaranteed invariant and its covering tests (or is explicitly marked "uncovered", landing only with the reviewer's explicit acceptance), and is the **only** permitted behavior change in any workflow. |
| **simplification** | **P3** redundant prose, **P5** verbose phrasing (narrowed: verbosity only — the 30%+ word-reduction baseline; voice rewrites belong to P2) | Fixes that only delete redundancy or reduce words, with voice and behavior unchanged. Strictly zero behavior change. |

**Boundary tie-break.** A candidate whose fix would change prohibition structure, voice, or code behavior belongs to **affirmative**, even when it is also verbose — the affirmative rewrite subsumes the tightening. A candidate whose fix only removes words or duplication with unchanged voice belongs to **simplification**. Past-tense narration always belongs to **residue**. A scanner never proposes another workflow's classes: it records cross-workflow candidates in the Observations channel (§4, spine element 7) for the Director to report, so the user can schedule the other workflow.

### 4. Shared orchestration spine (stated once, in the umbrella `SKILL.md`)

The spine is workflow-parameterized; every element below appears exactly once in `SKILL.md` and is cited — never restated — by workflow bodies and role files.

1. **Three invariants (every workflow, every run).**
   1. **No contract or behavior lost.** Every CLI flag, error string, schema, command example, IMPORTANT line, and cross-reference survives — in fewer words, never in weaker form. Sole exception: an approved affirmative-workflow P4 row, which changes behavior deliberately (fallback → fail-fast); each P4 row names its covering tests or is explicitly marked "uncovered", and lands only with the reviewer's individual acceptance — an "uncovered" row only with that acceptance made explicit.
   2. **No live test coverage lost.** Test logic, assertions, fixtures, and parametrizations are untouchable; only comments, docstrings, and sentinel framing are in scope. A mixed sentinel test keeps its live assertion.
   3. **No new narration (R1).** Every edit reads as a clean present-tense statement of current behavior.
2. **Scope and exempt set.** In scope: the whole tracked tree minus the exempt set. Exempt (never modified): `design-docs/`, `researches/`, `cafleet/src/cafleet/db/alembic/versions/**`, `cafleet/src/cafleet/webui/dist/**`, lock files. The per-surface in-scope/untouchable table (docs/rules prose vs contract detail; source comments/docstrings vs runtime surfaces; test comments vs test logic) lives here once.
3. **Team shape.** Director (main session — resolves `${BASE}`, bootstraps the fleet, partitions **disjoint file-ownership** slices, merges partials, holds the apply behind review, verifies, tears down monitor-first) + monitor (first-in, `--role monitor --model {monitor_model}`, gates on `ready: monitor live`) + N scanners + 1 reviewer (`--model {reviewer_model}`). Disjoint file-ownership is the concurrency contract: one scanner owns every edit to a given file, so parallel apply is safe without worktrees.
4. **Coordination extensions.** The `cafleet` verb + pointer schema (`skills/cafleet-design-doc/reference/coordination.md`) plus: a `scanner` role for scanner-tagged markers, and a per-workflow whole-run pointer — `inventory` for residue, `findings` for affirmative and simplification. The reviewer's gate is `approved (inventory)` / `approved (findings)`; per-row routing uses `<file>:<line>` pointers.
5. **Per-run process (8 steps, parameterized).** Resolve `${BASE}` → bootstrap (`cafleet doctor`, `fleet create`, monitor first) → spawn workers → **scan** (workflow mechanics) → **merge + review gate** (no repository edit before the reviewer's approval) → apply (scanners apply their own slices; harness-denied writes staged under `${BASE}/.apply/` and routed to the Director) → verify (`mise //cafleet:test`, `mise //cafleet:lint`, `mise //cafleet:typecheck` green + the workflow's own check) → report + monitor-first teardown, commit only when the user asks.
6. **Workflow parameter table.** The spine names its three parameters per workflow:

   | Parameter | residue | affirmative | simplification |
   |---|---|---|---|
   | Scan mechanics | multi-pass pattern-catalog sweep + hand-inspection of every hit | full-file judgment read of every file in the slice | full-file judgment read of every file in the slice |
   | Artifact / pointer | `inventory.md` / `inventory` | `findings.md` / `findings` | `findings.md` / `findings` |
   | Workflow verification | post-apply re-sweep → zero unaccounted matches | reviewer confirms the diff is confined to approved rows and each P4 row's named coverage is validated (an "uncovered" row requires the reviewer's explicit acceptance) | reviewer confirms the diff is confined to approved rows |

7. **Observations escalation channel (every workflow).** While scanning, a scanner records — and never fixes — content drift (two surfaces disagreeing, a broken cross-reference, a doc contradicting the code) and cross-workflow candidates (§3) in its artifact's Observations section. The Director escalates them to the user in the run report.
8. **`${BASE}` convention.** Task-scoped per the `cafleet` skill's `reference/base-dir.md`: `researches/clean-docs-<workflow>-<UTC-compact>/` (gitignored, one folder per run). Agent-only scratch is dot-prefixed (`.prompts/`, `.apply/`).
9. **Spawn-prompt skeleton.** One skeleton, with the workflow named in the role line ("You are the <role> in a clean-docs <workflow> team (CAFleet-native)") and the role-definition path pointing into `<workflow>/roles/`. It keeps the existing structure: `[INSERT …]` markers rendered by the Director, the four `{fleet_id}` / `{member_id}` / `{director_member_id}` / `{coding_agent}` identity placeholders left for the CLI's `str.format`, all other literal braces doubled, the four IMPORTANT lines (bash rules, blocked→message, no edit before the relayed approval, no git writes), the ready-signal first Bash call, and the role-specific assignment + start cue. Skill-load lines name `clean-docs` and `cafleet`. The monitor's prompt comes from the `cafleet` skill's `roles/monitor.md`, not from this skeleton.
10. **Backend-neutrality.** All `{monitor_model}` / `{reviewer_model}` / `{skill_loader}` / `{decision_surface}` / `{permission_flags}` tokens resolve from `skills/cafleet/reference/coding-agent/<name>-overlay.md`; role files are referenced by absolute path in spawn prompts; spawns use `--text-file`.

### 5. The three workflow bodies (deltas only)

Each `<workflow>/<workflow>.md` carries a Required-reading block (overlay row #1, base-dir, coordination, its own reference pages), its trigger scenario, its mechanics, its artifact spec, and its guarantees — nothing from the spine.

**`residue/residue.md`** — the pattern-catalog sweep. Mechanics: every pass of `residue/reference/patterns.md` (`git grep -nIiP` with the exempt-set exclusion) over the slice; hand-inspection of every hit with surrounding context; classification per `residue/reference/rubric.md` ((a)/(b)/(c) + known-benign). Artifact: partial inventories merged into `inventory.md` — grouped file→action tables, an explicit KEEP list, a known-benign subsection, and an Observations section. Guarantee: **provable completeness** — after apply, re-running every pass yields zero unaccounted matches (every remaining hit KEEP-listed, known-benign, or exempt). Both reference pages move from the old skill with their content preserved in meaning (the rubric and catalog are proven by the 0000131 dogfood), with two systematic label changes: the moved `patterns.md` keeps its concrete `:(exclude)` pathspec list as the **mechanical embodiment** of the exempt set, citing the umbrella `SKILL.md` list (spine element 2) as canonical — one authority, no second list to drift; and the standalone-era R2/R3/R4 identifiers are replaced by citations to the umbrella's exempt set and scope (only the R1 label survives, as invariant 3).

**`affirmative/affirmative.md`** — affirmative-writing enforcement. Mechanics: full-file judgment read; classification per `affirmative/reference/rubric.md` (P1/P2/P4, with P2 absorbing negative-voice rephrasing per §3); apply-ready rows per the shared row format. P4 rows carry the BEHAVIOR-AFFECTING tag, the invariant justification, and the covering tests; the reviewer individually validates each against the affirmative-writing legitimacy carve-outs (a paired prohibition and a correct default are compliant, not findings). Enforced rules: `~/.claude/rules/affirmative-writing.md` and `.claude/rules/code-quality.md`.

**`simplification/simplification.md`** — de-duplication and tightening. Mechanics: full-file judgment read; classification per `simplification/reference/rubric.md` (P3/P5 with the 30%+ baseline and the style-only-churn drop rule); apply-ready rows per the shared row format. Strictly zero behavior change: source-code scope is comments and docstrings only.

The two judgment workflows share `reference/review-format.md` at the skill root (per the `cafleet-design-doc` precedent of root-level shared reference pages): the apply-ready row format (`location | quoted text | class | exact replacement | risk note`), the KEEP guardrails (contract detail, behavioral-contract lines, backend neutrality, legitimate prohibitions, correct defaults, runtime surfaces, R1), the decision procedure (full-context read → guardrails → classify → re-check the replacement → drop marginal findings), and the reviewer's per-row verdict flow (APPROVE / REJECT / REVISE in the merged `findings.md`). Residue stays self-contained — its rubric and patterns pages already carry its guardrails and procedure.

### 6. File layout

```
.claude/skills/clean-docs/
  SKILL.md                        # umbrella: frontmatter + dispatch (§2) + shared spine (§4)
  reference/
    review-format.md              # shared by affirmative + simplification (§5)
  residue/
    residue.md                    # workflow body — sweep mechanics + completeness guarantee
    roles/scanner.md              # sweep → hand-inspect → classify → apply own slice
    roles/reviewer.md             # over-deletion / lost-coverage / R1 guard
    reference/rubric.md           # (a)/(b)/(c) + known-benign — canonical
    reference/patterns.md         # pattern catalog + exempt-set exclusion — canonical
  affirmative/
    affirmative.md                # workflow body — P1/P2/P4 judgment review
    roles/scanner.md              # full read → propose → apply own slice after the gate
    roles/reviewer.md             # lost-meaning / lost-contract / behavior-change guard
    reference/rubric.md           # P1, P2, P4 classes — canonical
  simplification/
    simplification.md             # workflow body — P3/P5 judgment review
    roles/scanner.md
    roles/reviewer.md
    reference/rubric.md           # P3, P5 classes — canonical
```

Role files adapt from the existing skills with two systematic changes: spine citations point to the umbrella `SKILL.md` instead of restating, and relative links (overlay, base-dir, coordination) gain one directory level (`../../../../../skills/cafleet/…` from `<workflow>/roles/`). The monitoring member reuses the `cafleet` skill's canonical `roles/monitor.md` — no monitor role file lives here.

### 7. Removal-rule cleanup

`.claude/skills/historical-residue-cleanup/` (all five files) is deleted in the same change that lands the merged skill. The single cross-directory mention of the removed name — the sibling pointer in the old `clean-docs` `SKILL.md` — is eliminated by the Step-1 rewrite; with the directory deletion, the removal is total. Verification is `git grep -i "historical-residue"` over the tracked tree minus `design-docs/` → zero hits. The rewritten umbrella `SKILL.md` mentions no predecessor skill — the repository reads as if `clean-docs` with three workflows always existed. Design docs 0000131 and this one remain the historical record.

### Out of scope

- The `update-readme` skill — stays separate and unchanged.
- `README.md`, `SPEC.md`, `docs/` — these project-local `.claude/skills/` are not a product contract surface; no product documentation changes.
- Any runtime behavior change to the `cafleet` package — this change touches only skill files.
- Dogfood runs of the workflows (D1): the rubrics and mechanics are carried over from skills already proven in use; verification is load/dispatch + the zero-mentions check.

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

### Step 1: Umbrella `SKILL.md`

- [x] Rewrite `.claude/skills/clean-docs/SKILL.md`: frontmatter (`name: clean-docs`, a `description` triggering all three §2 families + the members-load line) and the dispatch table with the one-workflow-per-run rule and the `{decision_surface}` ambiguity rule. <!-- completed: 2026-07-19T02:36 -->
- [x] State the shared spine once in `SKILL.md` (§4): the three invariants with the affirmative-only P4 carve-out, scope/exempt set, team shape, coordination extensions, the parameterized 8-step per-run process with the workflow parameter table, the observations channel, the `${BASE}` convention, the spawn-prompt skeleton, and backend-neutrality with the required-reading block. <!-- completed: 2026-07-19T02:36 -->

### Step 2: Shared judgment reference

- [x] Create `reference/review-format.md` — apply-ready row format, KEEP guardrails, decision procedure, and reviewer verdict flow, shared by the affirmative and simplification workflows (§5). <!-- completed: 2026-07-19T02:38 -->

### Step 3: `residue/` workflow

- [x] Create `residue/residue.md` — trigger scenario, sweep mechanics, `inventory.md` artifact spec (tables + KEEP list + known-benign + Observations), and the provable-completeness guarantee; Required-reading block with corrected link depths. <!-- completed: 2026-07-19T02:44 -->
- [x] Create `residue/roles/scanner.md` and `residue/roles/reviewer.md`, adapted from the old skill's roles: spine content replaced by citations to the umbrella `SKILL.md`, links re-depthed, skill-load lines naming `clean-docs`. <!-- completed: 2026-07-19T02:44 -->
- [x] Move `reference/rubric.md` and `reference/patterns.md` into `residue/reference/`, content preserved in meaning, internal links updated; replace the standalone-era R2/R3/R4 labels with citations to the umbrella's exempt set and scope (R1 survives as invariant 3), and mark patterns.md's `:(exclude)` pathspec as the mechanical embodiment of the umbrella's canonical exempt-set list (§5). <!-- completed: 2026-07-19T02:44 -->

### Step 4: `affirmative/` workflow

- [x] Create `affirmative/affirmative.md` — trigger scenario, full-read judgment mechanics, `findings.md` artifact spec, the P4 BEHAVIOR-AFFECTING protocol, and the legitimacy carve-outs. <!-- completed: 2026-07-19T02:50 -->
- [x] Create `affirmative/roles/scanner.md` and `affirmative/roles/reviewer.md`, adapted from the old `clean-docs` roles and scoped to P1/P2/P4. <!-- completed: 2026-07-19T02:50 -->
- [x] Create `affirmative/reference/rubric.md` — the P1/P2/P4 classes with P2 absorbing negative-voice rephrasing (§3), citing `reference/review-format.md` for the shared format and guardrails. <!-- completed: 2026-07-19T02:50 -->

### Step 5: `simplification/` workflow

- [x] Create `simplification/simplification.md` — trigger scenario, full-read judgment mechanics, `findings.md` artifact spec, strict zero-behavior-change statement. <!-- completed: 2026-07-19T02:53 -->
- [x] Create `simplification/roles/scanner.md` and `simplification/roles/reviewer.md`, adapted and scoped to P3/P5. <!-- completed: 2026-07-19T02:53 -->
- [x] Create `simplification/reference/rubric.md` — the P3/P5 classes with the narrowed P5 (verbosity only, 30%+ baseline, style-only-churn drop rule), citing `reference/review-format.md`. <!-- completed: 2026-07-19T02:53 -->

### Step 6: Removal cleanup

- [x] Delete `.claude/skills/historical-residue-cleanup/` (SKILL.md, both roles, both reference pages) in the same change. <!-- completed: 2026-07-19T02:55 -->
- [x] Delete the old single-skill layout files the §6 tree drops: `.claude/skills/clean-docs/roles/scanner.md`, `.claude/skills/clean-docs/roles/reviewer.md`, and `.claude/skills/clean-docs/reference/rubric.md` (replaced by the per-workflow roles and rubrics). <!-- completed: 2026-07-19T02:55 -->
- [x] Verify zero mentions and layout: `git grep -i "historical-residue"` over the tracked tree minus `design-docs/` returns nothing, and `git ls-files .claude/skills/clean-docs` matches the §6 layout exactly. <!-- completed: 2026-07-19T02:56 -->

### Step 7: Load + dispatch verification

- [x] Confirm the skill loads via the Skill tool; its `description` triggers on each workflow's §2 phrasings and dispatch routes each family to the right workflow body; no `{token}` leaks in any user-facing string. <!-- completed: 2026-07-19T02:57 -->

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-07-19 | Initial draft — consolidates the two cleanup skills into `clean-docs` with three workflows (residue / affirmative / simplification) per user clarification; spine stated once in the umbrella `SKILL.md`; total removal of the `historical-residue-cleanup` name |
| 2026-07-19 | Reviewer pass: precedent-shape claim corrected (per-workflow `reference/` is an extension, not a match); P4 coverage rule reconciled across §3/spine/parameter table (named coverage; "uncovered" needs explicit reviewer acceptance); patterns.md pathspec pinned as embodiment of the canonical umbrella exempt set with R2/R3/R4 labels replaced; removal reasoning corrected (sibling pointer eliminated by the Step-1 rewrite); added deletion of the old-layout `clean-docs` files + exact-tree verification (15→16 tasks) |
| 2026-07-19 | Implementation complete: all 16 tasks and 10 success criteria done via the execute workflow (Programmer-only team); fresh Reviewer approved in 1 round; PR #214 opened; Status → Complete |
