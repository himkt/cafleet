---
name: clean-docs
description: >
  Clean up the repository's docs, comments, rules, and tests as a CAFleet team
  of disjoint-slice scanner members gated by a reviewer — exactly one workflow
  per run. Use the residue workflow when the user asks to clean up historical
  narration or historical residue, remove deprecation notes / "this replaces X"
  / "renamed from Y" / "previously … now …" prose, drop design-number
  provenance citations, or delete removal-sentinel tests. Use the affirmative
  workflow when the user asks to run an affirmative-writing sweep, fix
  prohibition-only rule sections, pair prohibitions with affirmatives, remove
  meaningless fallbacks, or make code fail fast. Use the simplification
  workflow when the user asks to simplify the docs or comments, tighten verbose
  prose, de-duplicate documentation, or remove redundant comments. Nothing is
  applied before the reviewer approves the merged run artifact. Members load
  this skill by its name clean-docs via their backend's skill-loader.
---

# Clean-Docs Skill (CAFleet-orchestrated)

One skill, three workflows — **residue**, **affirmative**, **simplification** —
run as the same CAFleet team shape over the same per-run process. This page is
the dispatcher and the single home of the **shared orchestration spine**; each
workflow body (`<workflow>/<workflow>.md`) carries only its own trigger
scenario, scan mechanics, artifact spec, and guarantees, citing the spine here
rather than restating it.

## Required reading

Identify your coding agent first — a member's spawn prompt names it on the
`CODING AGENT:` line; the Director (main session) uses its own identity — then
Read your overlay and **resolve** it before your first action.

**Load-bearing — Read in order before acting:**

| # | Read | What you lose if you skip it |
|---|------|------------------------------|
| 1 | your overlay [`../../../skills/cafleet/reference/coding-agent/<name>-overlay.md`](../../../skills/cafleet/reference/coding-agent/) — read **and resolve** it (see *Resolve your overlay* in the cafleet `SKILL.md`) | you emit a literal `{monitor_model}` / `{reviewer_model}` / `{skill_loader}` / `{decision_surface}`, guess a wrong value, or ignore a backend note |
| 2 | the `cafleet` skill's [`reference/base-dir.md`](../../../skills/cafleet/reference/base-dir.md) | the task-scope BASE resolution, the no-bypass write protocol, and the `<unset>` contract — you mis-root run artifacts or fall back to `/tmp` |
| 3 | the `cafleet-design-doc` skill's [`reference/coordination.md`](../../../skills/cafleet-design-doc/reference/coordination.md) | the verb + pointer + `COMMENT(role)` schema and this skill's extensions (`scanner` role, per-workflow run pointer) — your status hops mis-route |

The workflow body you route into carries its own Required-reading block for the
workflow's reference pages (rubric, pattern catalog, shared review format).

Before acting, resolve every `{token}` you will use to its overlay value (or the
documented default); a literal `{token}` in any command, message, or user-facing
string is a defect.

## Dispatch

Route the user's request into exactly **one** workflow body by phrasing, then
run that body's orchestration in full over the spine below. One workflow per
run — never a combined pass. When a request names families from more than one
workflow, the Director asks the user which single workflow to run via
`{decision_surface}`.

| When the user wants to… | Run |
|:--|:--|
| Clean up historical narration / historical residue; remove deprecation notes, "this replaces X" / "renamed from Y" / "previously … now …" prose; drop design-number provenance citations; delete removal-sentinel tests | the **residue** workflow ([residue/residue.md](residue/residue.md)) |
| Run an affirmative-writing sweep; fix prohibition-only rule sections; pair prohibitions with affirmatives; remove meaningless fallbacks; make code fail fast | the **affirmative** workflow ([affirmative/affirmative.md](affirmative/affirmative.md)) |
| Simplify the docs / comments; tighten verbose prose; de-duplicate documentation; remove redundant comments | the **simplification** workflow ([simplification/simplification.md](simplification/simplification.md)) |

### Class-to-workflow split

Every candidate a scanner may act on belongs to exactly one workflow. The class
identifiers are canonical in each workflow's rubric; ownership is:

| Workflow | Owns | Fix character |
|---|---|---|
| **residue** | (a) sentinel test, (b) narration/citation/trajectory, (c) keep + known-benign | Delete sentinel framing; reword narration to present tense. Strictly zero behavior change. |
| **affirmative** | **P1** prohibition-pile, **P2** unpaired prohibition (including negatively-phrased instructions rewritable in affirmative voice), **P4** meaningless fallback / swallowed error | Fixes that change prohibition structure, voice, or code behavior. P4 rows are BEHAVIOR-AFFECTING — the only permitted behavior change in any workflow (invariant 1). |
| **simplification** | **P3** redundant prose, **P5** verbose phrasing (verbosity only — the 30%+ word-reduction baseline; voice rewrites belong to P2) | Fixes that only delete redundancy or reduce words, with voice and behavior unchanged. Strictly zero behavior change. |

**Boundary tie-break.** A candidate whose fix would change prohibition
structure, voice, or code behavior belongs to **affirmative**, even when it is
also verbose — the affirmative rewrite subsumes the tightening. A candidate
whose fix only removes words or duplication with unchanged voice belongs to
**simplification**. Past-tense narration always belongs to **residue**. A
scanner never proposes another workflow's classes: it records cross-workflow
candidates in its artifact's Observations section (see *Observations escalation
channel* below) for the Director to report, so the user can schedule the other
workflow.

## Three invariants (every workflow, every run)

1. **No contract or behavior lost.** Every CLI flag, error string, schema,
   command example, IMPORTANT line, and cross-reference survives — in fewer
   words, never in weaker form. Sole exception: an approved
   affirmative-workflow P4 row, which changes behavior deliberately
   (fallback → fail-fast); each P4 row names its covering tests or is
   explicitly marked "uncovered", and lands only with the reviewer's individual
   acceptance — an "uncovered" row only with that acceptance made explicit.
   Residue and simplification runs are strictly zero-behavior-change.
2. **No live test coverage lost.** Test logic, assertions, fixtures, and
   parametrizations are untouchable; only comments, docstrings, and sentinel
   framing are in scope. A mixed sentinel test keeps its live assertion.
3. **No new narration (R1).** Every edit reads as a clean present-tense
   statement of current behavior — no "previously / now / formerly", no "this
   replaces X", no "renamed from Y".

## Scope and exempt set

**In scope**: the whole tracked tree minus the exempt set. **Exempt (never
modified)**: `design-docs/`, `researches/`,
`cafleet/src/cafleet/db/alembic/versions/**`, `cafleet/src/cafleet/webui/dist/**`,
and lock files. A migration legitimately references prior state; the generated
`dist/` bundle is not authored prose; the design/research folders are the
historical record.

Per surface, the scope of what may change:

| Surface | In scope | Untouchable |
|---|---|---|
| `docs/`, `README.md`, `SPEC.md`, skills, rules | prose structure and wording | contract detail (exact flags, error strings, schemas, layouts), command examples, IMPORTANT / lossless-rule lines, backend-neutral base/overlay separation |
| source code | comments, docstrings, affirmative-workflow P4 fallback sites | flags, options, columns, code paths, log/user-facing/error strings |
| tests | comments, docstrings, sentinel framing | all logic, assertions, fixtures, names |

## Team shape

```
User → /clean-docs (one workflow)
 └─ Director (main session — resolves BASE, bootstraps fleet, partitions slices,
              merges partials, holds the apply behind review, verifies, commits
              when asked, tears down monitor-first)
     ├─ monitor    (first-in, --role monitor; heartbeat; gates the first ordinary spawn)
     ├─ scanner-1  (owns a disjoint file slice: scan → propose → apply after gate)
     ├─ scanner-N  (…)
     └─ reviewer   (the workflow's guard, --model {reviewer_model})
```

| Role | Responsibility |
|---|---|
| **Director** | Resolve task-scoped `${BASE}`; `cafleet fleet create`; spawn the monitor first and gate on `ready: monitor live`; partition the in-scope tree into **disjoint file-ownership** slices (one file → one scanner, whole surfaces per scanner); merge partial artifacts into the run's canonical artifact; route it to the reviewer and **hold the apply until the reviewer's approval**; relay approval to scanners; apply any edit a scanner's harness denies (verify the staged diff first); run verification; escalate observations to the user; tear down monitor-first. |
| **monitor** | The mandatory dedicated monitoring member (canonical conditional idle-nudge). Reuses the `cafleet` skill's `roles/monitor.md`; the Director never runs `cafleet monitor start`. |
| **scanner** (×N) | For its slice: run the workflow's scan mechanics, propose actions per the workflow's rubric, record observations separately, write its partial artifact under `${BASE}`. After approval is relayed: apply its own slice's approved rows exactly as written, re-verify its diff, route harness-denied writes to the Director. |
| **reviewer** | Validate the merged artifact **before** any edit, per the workflow's guarantees and guardrails. After apply: run the workflow verification (parameter table below). |

**Disjoint file-ownership is the concurrency contract.** No two scanners edit
the same file, so parallel apply is safe without worktrees. **One scanner owns
every edit to a given file, regardless of class** — a file touched by several
classes is assigned whole to one scanner. A useful default partition:
docs+root files / skills+`.claude` / source+admin / tests — merge or split
slices to balance the scanner count against the tree size.

## Coordination

The skill adopts the `cafleet` verb + pointer schema
(`skills/cafleet-design-doc/reference/coordination.md`) with two skill-local
extensions, since a run produces a run artifact, not a design document:

- **Role taxonomy** gains a `scanner` role for `COMMENT(scanner)` markers.
- **Pointer**: a per-workflow whole-run pointer denotes the run artifact as a
  whole — `inventory` for residue, `findings` for affirmative and
  simplification. Scanners report `complete (<run pointer>)`; the reviewer's
  gate is `approved (inventory)` / `approved (findings)` (or
  `blocked (<run pointer>)`); per-row routing uses `<file>:<line>` pointers.

## Per-run process

1. **Resolve `${BASE}`** — task-scoped per the `cafleet` skill's
   `reference/base-dir.md`, convention
   `researches/clean-docs-<workflow>-<UTC-compact>/` (gitignored, one folder
   per run).
2. **Bootstrap** — `cafleet doctor` (gating), `cafleet fleet create`; spawn the
   monitor first (`--role monitor --model {monitor_model}`, members spawned
   `{permission_flags}`), gate on `ready: monitor live`.
3. **Spawn workers** — scanners (one per disjoint slice) and the reviewer
   (`--model {reviewer_model}`), each from a rendered prompt at
   `${BASE}/.prompts/<role>-<UTC-compact>.md`.
4. **Scan (fan-out)** — each scanner runs the workflow's **scan mechanics**
   (parameter table below) over its slice and writes its partial artifact under
   `${BASE}`, with a separate *Observations* section.
5. **Merge + review (gate)** — the Director merges partials into the run's
   canonical artifact and routes it to the reviewer with
   `ready (<run pointer>)`. **No repository edit happens before the reviewer's
   `approved (<run pointer>)`.**
6. **Apply** — the Director relays approval; each scanner applies its own
   slice's approved rows exactly as written. A scanner whose harness denies a
   write (commonly `.claude/`) stages the full target file under
   `${BASE}/.apply/` and routes to the Director, who diff-reviews the staged
   file and applies it.
7. **Verify** — `mise //cafleet:test`, `mise //cafleet:lint`,
   `mise //cafleet:typecheck` green, plus the **workflow verification**
   (parameter table below).
8. **Report + teardown** — the Director reports the applied set and the
   escalated observations, commits only when the user asks, and tears down
   monitor-first (monitoring member → ordinary members → verify via
   `member list` → `fleet delete`).

### Workflow parameter table

The spine is parameterized by exactly three per-workflow values:

| Parameter | residue | affirmative | simplification |
|---|---|---|---|
| Scan mechanics | multi-pass pattern-catalog sweep + hand-inspection of every hit | full-file judgment read of every file in the slice | full-file judgment read of every file in the slice |
| Artifact / pointer | `inventory.md` / `inventory` | `findings.md` / `findings` | `findings.md` / `findings` |
| Workflow verification | post-apply re-sweep → zero unaccounted matches | reviewer confirms the diff is confined to approved rows and each P4 row's named coverage is validated (an "uncovered" row requires the reviewer's explicit acceptance) | reviewer confirms the diff is confined to approved rows |

## Observations escalation channel

While scanning, a scanner records — and never fixes — content drift (two
surfaces disagreeing, a broken cross-reference, a doc contradicting the code)
and cross-workflow candidates (*Class-to-workflow split* above) in its
artifact's Observations section. The Director escalates them to the user in the
run report. The **applied cleanup — the git diff plus green verification — is
the real deliverable**; observations are escalated, never self-applied.

## `${BASE}` convention

Task-scoped per the `cafleet` skill's `reference/base-dir.md`:
`researches/clean-docs-<workflow>-<UTC-compact>/` (gitignored, one folder per
run). Agent-only scratch is dot-prefixed: `.prompts/` (spawn-prompt renders),
`.apply/` (staged harness-denied writes).

## Spawn-prompt skeleton

One skeleton for all three workflows, with the workflow named in the role line
and the role-definition path pointing into `<workflow>/roles/`. The Director
renders each `[INSERT …]` marker to a literal before writing; leaves the four
`{fleet_id}` / `{member_id}` / `{director_member_id}` / `{coding_agent}`
identity placeholders for the CLI's `str.format` at spawn; doubles every OTHER
literal brace as `{{` / `}}`:

```
You are the <role> in a clean-docs <workflow> team (CAFleet-native).

ROLE DEFINITION: Open [INSERT abs path to <workflow>/roles/<role>.md] with the Read tool BEFORE any other action.

Load these skills at startup:
- the clean-docs skill — for the shared spine and your workflow's mechanics
- the cafleet skill — for the broker primitives and bash-via-Director routing

FLEET ID: {fleet_id}
DIRECTOR MEMBER ID: {director_member_id}
YOUR MEMBER ID: {member_id}
BASE: [INSERT abs BASE path the Director resolved]
CODING AGENT: {coding_agent}

<role-specific assignment: the scanner's slice path list + partial-artifact path,
 or the reviewer's merged-artifact path>

IMPORTANT: Read and follow .claude/rules/bash-tool.md (CAFleet-member Bash protocol) and ~/.claude/rules/bash-command.md (general Bash hygiene) for all Bash commands.
IMPORTANT: If blocked, send a message to the Director immediately instead of assuming.
IMPORTANT: Do NOT edit any repository file until the Director relays the reviewer's approved (<run pointer>). The scan phase is read + propose only.
IMPORTANT: Do NOT commit code or run git write operations — the Director handles all git.

On spawn, as your first Bash call, send the ready signal: cafleet message send --fleet-id {fleet_id} --from-member-id {member_id} --to-member-id {director_member_id} --text "ready"

When you see cafleet message poll output with a message from the Director, act on those instructions.

<start cue: scanner — begin the scan of your slice; reviewer — wait for ready (<run pointer>)>
```

The monitor's spawn prompt comes from the `cafleet` skill's `roles/monitor.md`
canonical skeleton, not from this one.

## Backend-neutrality

`SKILL.md`, the workflow bodies, and every `<workflow>/roles/*.md` are
backend-neutral: they use `{monitor_model}` / `{reviewer_model}` /
`{skill_loader}` / `{decision_surface}` / `{permission_flags}` tokens resolved
from `../../../skills/cafleet/reference/coding-agent/<name>-overlay.md`, and
every member's spawn-prompt identity block carries a
`CODING AGENT: {coding_agent}` line so the member resolves its overlay. Role
files are referenced by absolute path in spawn prompts (never inlined); spawns
use `--text-file`.

## Skill file layout

```
.claude/skills/clean-docs/
  SKILL.md                        # this file — dispatch + shared spine, backend-neutral
  reference/
    review-format.md              # shared by affirmative + simplification: row format, guardrails, verdict flow
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

The monitoring member reuses the `cafleet` skill's canonical `roles/monitor.md`
(no monitor role file lives here).
