---
name: historical-residue-cleanup
description: >
  Sweep the whole repository for historical-context narration and
  historical-guard residue and remove it, per removal.md and
  affirmative-writing.md, while preserving every runtime behavior and every
  live test assertion. Use when the user asks to clean up historical narration
  or historical residue, remove deprecation notes / "this replaces X" /
  "renamed from Y" / "previously … now …" prose, drop design-number provenance
  citations from tests, delete removal-sentinel tests, or run an
  affirmative-writing sweep across the repo. Runs as a CAFleet team
  (Director + monitor + N scanners + 1 reviewer). Members load this skill by
  its name historical-residue-cleanup via their backend's skill-loader.
---

# Historical-Residue Cleanup Skill (CAFleet-orchestrated)

On demand, sweep the whole tracked tree for **historical narration** (past-state
prose, "this replaces X", "renamed from Y", design-number provenance citations,
trajectory notes, version qualifiers) and **historical-guard residue**
(removal-sentinel tests) and remove it per `~/.claude/rules/removal.md` and
`~/.claude/rules/affirmative-writing.md`. After a run, every artifact reads as a
clean present-tense statement of current behavior — and no behavior, no flag, no
column, no code path, and no live test assertion is lost.

This is a CAFleet-orchestrated team: a fan-out of **scanner** members each owning
a disjoint file slice, guarded by a **reviewer** whose sole job is preventing
over-deletion and lost coverage, coordinated by the Director through the message
broker. A single subagent cannot provide the multi-pass, hand-inspected,
adversarially-reviewed sweep this work requires.

## Two invariants (never violated by any run)

1. **No runtime behavior is removed.** Every CLI flag, table, column, code path,
   and API/CLI surface that exists before a run still exists after it. This skill
   edits prose and deletes sentinel-test framing only — never product behavior.
2. **No live test coverage is lost.** Only removal-sentinel framing and historical
   narration are removed. Every live assertion stays. When a single test *mixes* a
   sentinel with live coverage, keep the live assertion and drop only the sentinel
   framing.

Both invariants are load-bearing: a run that would violate either is a defect the
reviewer must catch before any edit lands.

## Required reading

Identify your coding agent first — a member's spawn prompt names it on the
`CODING AGENT:` line; the Director (main session) uses its own identity — then
Read your overlay and **resolve** it before your first action.

**Load-bearing — Read in order before acting:**

| # | Read | What you lose if you skip it |
|---|------|------------------------------|
| 1 | your overlay [`../../../skills/cafleet/reference/coding-agent/<name>.md`](../../../skills/cafleet/reference/coding-agent/) — read **and resolve** it (see *Resolve your overlay* in the cafleet `SKILL.md`) | you skip resolution — you emit a literal `{monitor_model}` / `{reviewer_model}` / `{skill_loader}` / `{decision_surface}`, **or** guess a wrong/default value, **or** ignore a backend note |
| 2 | the `cafleet` skill's [`reference/base-dir.md`](../../../skills/cafleet/reference/base-dir.md) | the task-scope BASE resolution, the no-bypass write protocol, and the `<unset>` contract — you mis-root run artifacts or fall back to `/tmp` |
| 3 | the `cafleet-design-doc` skill's [`reference/coordination.md`](../../../skills/cafleet-design-doc/reference/coordination.md) | the verb + pointer + `COMMENT(role)` schema and the two skill-local extensions (`scanner` role, `inventory` pointer) — your status hops mis-route |

The reference pages of THIS skill — the rubric and the pattern catalog — are the
canonical classification and sweep sources; both role files cite them:

| Read | When |
|------|------|
| [`reference/rubric.md`](reference/rubric.md) | classifying any swept hit (the fixed sentinel / narration / keep rubric + the known-benign class) |
| [`reference/patterns.md`](reference/patterns.md) | running the multi-pass sweep (the pattern catalog + the exempt-set exclusion) |

Before acting, resolve every `{token}` you will use to its overlay value (or the
documented default); a literal `{token}` in any command, message, or user-facing
string is a defect.

## The four hard requirements (R1–R4)

Each is encoded here as a `SKILL.md` instruction, in `reference/rubric.md` and/or
`roles/reviewer.md`, and as a Success Criterion in the design doc.

| # | Requirement |
|---|---|
| **R1** | **No-add + remove.** The skill removes existing historical narration AND is forbidden from introducing new narration in its own edits. Every edit it makes is a clean present-tense statement of current behavior — no "previously / now / no longer / formerly", no "this replaces X", no "renamed from Y". |
| **R2** | **Exempt set (never modified).** `design-docs/`, `researches/`, `cafleet/src/cafleet/db/alembic/versions/**`, `cafleet/src/cafleet/webui/dist/**`, and lock files. A migration legitimately references prior/renamed state; the generated `dist/` bundle is not authored prose; the design/research folders are the historical record. |
| **R3** | **In scope (swept every run).** The **whole tracked tree minus the R2 exempt set** (canonical). The major authored surfaces — `docs/`, `README.md`, `SPEC.md`, `skills/`, `.claude/`, `cafleet/src/`, `cafleet/tests/`, `admin/src/`, plus root files like `CLAUDE.md`, `pyproject.toml`, `mise.toml`, `package.json` — are illustrative, not an exhaustive allow-list: any tracked file outside R2 is in scope. |
| **R4** | **Thorough read.** A structured multi-pass sweep (`reference/patterns.md`) plus hand-inspection of every hit, fanned out across scanner members. Never a shallow single grep. |

## Team shape

```
User → /historical-residue-cleanup
 └─ Director (main Claude — resolves BASE, bootstraps fleet, partitions surfaces,
              merges inventory, holds the apply behind review, verifies, tears down)
     ├─ monitor    (first-in, --role monitor; heartbeat + idle-nudge; gates the first ordinary spawn)
     ├─ scanner-1  (owns a disjoint file slice: sweep → hand-inspect → classify → apply)
     ├─ scanner-2  (…)
     ├─ scanner-N  (…)
     └─ reviewer   (over-deletion / lost-coverage / new-narration (R1) guard)
```

| Role | Responsibility |
|---|---|
| **Director** | Resolve task-scoped `${BASE}`; `cafleet fleet create`; spawn the monitor first and gate on `ready: monitor live`; partition the R3 surfaces into **disjoint file-ownership** slices (no two scanners edit the same file); spawn scanners + reviewer; merge partial inventories into the run's canonical `inventory.md`; route it to the reviewer and **hold the apply until `approved (inventory)`**; after apply, run verification; tear down monitor-first. |
| **monitor** | The mandatory dedicated monitoring member (canonical conditional idle-nudge). Owns the heartbeat; reuses the `cafleet` skill's canonical `roles/monitor.md`. The Director never runs `cafleet monitor start`. |
| **scanner** (×N) | For its assigned slice: run the multi-pass sweep (`reference/patterns.md`), hand-inspect every hit, classify each with the rubric (`reference/rubric.md`), write a partial file→action inventory. After the reviewer approves the merged inventory, apply its slice's edits (disjoint files → no merge conflicts, no worktree isolation needed) and re-run the sweep on its slice. |
| **reviewer** | Validate the merged inventory against removal.md + affirmative-writing.md **before** any edit: catch mis-classification, over-deletion (a KEEP-item marked for removal), and any planned edit that would lose live coverage or introduce new narration (R1). Sign off with `approved (inventory)`. After apply, confirm the re-sweep is clean and no assertion or runtime behavior was lost. |

**Disjoint file-ownership is the concurrency contract.** The Director partitions by
file path so scanners never contend, which is why parallel apply is safe without
worktrees. **One scanner owns every edit to a given file, regardless of which
rubric class those edits fall under** — a file touched by several classes is
assigned whole to one scanner, never split across scanners by class.

### Coordination

The skill adopts the `cafleet` verb + pointer schema
(`skills/cafleet-design-doc/reference/coordination.md`) with two skill-local
extensions, since a cleanup run produces an `inventory.md`, not a design document:

- **Role taxonomy** gains a `scanner` role: a scanner records each classified hit
  as a scanner-tagged `COMMENT(scanner)` marker in the run's inventory at the
  hit's `<file>:<line>` pointer (or as a partial table under `${BASE}`).
- **Pointer** `inventory` denotes the run's `inventory.md` as a whole — the
  cleanup-run analog of `doc`. The reviewer's sign-off is `approved (inventory)`;
  per-file routing uses `<file>:<line>` pointers.

## Per-run process

The Director runs the five skill-author sub-systems in order (resolve BASE →
bootstrap fleet → spawn monitor first → spawn members → tear down monitor-first):

1. **Resolve `${BASE}`** — task-scoped, per the `cafleet` skill's
   `reference/base-dir.md`. Task convention: `researches/historical-residue-<UTC-compact>/`
   (analysis-shaped, one folder per run; `researches/` is gitignored, so run
   artifacts stay out of version control automatically).
2. **Bootstrap** — `cafleet fleet create`; spawn the monitor first
   (`--role monitor --model {monitor_model}`, members spawned `{permission_flags}`),
   gate on `ready: monitor live`.
3. **Partition** — split the R3 surfaces into disjoint file-ownership slices sized
   to the scanner count (a scanner per top-level surface group, or finer for a
   large tree).
4. **Sweep + classify (fan-out)** — each scanner runs the multi-pass pattern
   catalog (`reference/patterns.md`) over its slice, hand-inspects every hit,
   classifies with the rubric (`reference/rubric.md`), and writes its partial
   inventory as `COMMENT(scanner)` markers or a partial table under `${BASE}`.
5. **Merge + review (gate)** — the Director merges partials into the run's
   canonical `inventory.md` (grouped tables + KEEP list + known-benign subsection).
   The reviewer validates it and replies `approved (inventory)`. **No edit happens
   before approval.**
6. **Apply** — each scanner applies its own slice's edits; the R2 set is never
   touched; every live assertion is preserved.
7. **Verify** — `mise //cafleet:test`, `mise //cafleet:lint`, `mise //cafleet:typecheck`
   green; re-run the sweep across tracked files outside the exempt set → zero
   unaccounted matches (every remaining hit KEEP-listed or exempt). The reviewer
   confirms no coverage or runtime behavior was lost and no new narration was
   introduced (R1).
8. **Report + teardown** — the Director reports the run summary and tears down
   monitor-first (delete the monitoring member first — its `cafleet monitor start`
   loop dies with the pane — then ordinary members, then `cafleet fleet delete`).

## Per-run output artifact

Under `${BASE}`, the skill produces an `inventory.md` with three parts:

- Grouped **file→action inventory** tables (by surface / by rubric class), each row
  = location + quoted text anchor + action.
- An explicit **KEEP list** to prevent over-deletion.
- A **"Known-benign sweep matches"** subsection listing the present-tense false
  positives (a `no longer` describing current behavior, `used to` = *utilized to*,
  `preserved for` = *retained for*, `\bstale\b` naming a live feature, issue-number
  provenance).

The inventory is the run's audit record (ephemeral, under gitignored `researches/`).
The **applied cleanup — the git diff plus green verification — is the real
deliverable.**

## Skill file layout

```
.claude/skills/historical-residue-cleanup/
  SKILL.md              # this file — dispatch + the CAFleet orchestration procedure, backend-neutral
  roles/
    scanner.md          # sweep → hand-inspect → classify → apply own slice
    reviewer.md         # over-deletion / lost-coverage / new-narration (R1) guard
  reference/
    rubric.md           # the fixed classification rubric — canonical
    patterns.md         # the sweep pattern catalog — canonical
```

The monitoring member reuses the `cafleet` skill's canonical `roles/monitor.md`
(no monitor role file lives here).

### Backend-neutrality

`SKILL.md` and `roles/*.md` are backend-neutral: they use `{monitor_model}` /
`{reviewer_model}` / `{skill_loader}` / `{decision_surface}` / `{permission_flags}`
tokens resolved from `../../../skills/cafleet/reference/coding-agent/<name>.md`,
and every member's spawn-prompt identity block carries a `CODING AGENT: {coding_agent}`
line so the member resolves its overlay. Role files are referenced by absolute path
in spawn prompts (never inlined); spawns use `--text-file`; spawn-prompt audit
renders land at the dot-prefixed `${BASE}/.prompts/<role>-<UTC-compact>.md`
(agent-only scratch, per base-dir.md § *Hidden agent-only folders vs visible
deliverables*).

**Spawn-prompt skeleton** (Director renders each `[INSERT …]` marker to a literal
before writing; leaves the four `{fleet_id}` / `{member_id}` / `{director_member_id}`
/ `{coding_agent}` identity placeholders for the CLI's `str.format` at spawn;
doubles every OTHER literal brace as `{{` / `}}`):

```
You are the <role> in a historical-residue-cleanup team (CAFleet-native).

ROLE DEFINITION: Open [INSERT abs path to roles/<role>.md] with the Read tool BEFORE any other action.

Load these skills at startup:
- the historical-residue-cleanup skill — for the rubric, patterns, and per-run process
- the cafleet skill — for the broker primitives

FLEET ID: {fleet_id}
DIRECTOR MEMBER ID: {director_member_id}
YOUR MEMBER ID: {member_id}
BASE: [INSERT abs BASE path the Director resolved]
CODING AGENT: {coding_agent}

<role-specific assignment: the scanner's slice file list, or the reviewer's inventory path>
```
