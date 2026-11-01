---
name: clean-docs
description: >
  Clean up the repository's docs, comments, rules, and tests as a CAFleet team
  of disjoint-slice scanner members gated by a reviewer — exactly one workflow
  per run. Use the residue workflow when the user asks to clean up historical
  narration or historical residue, remove deprecation notes / "this replaces X"
  / "renamed from Y" / "previously … now …" prose, drop design-number
  provenance citations, or remove historical sentinel framing while preserving
  current absence tests. Use the affirmative workflow when the user asks to run
  an affirmative-writing sweep, fix prohibition-only rule sections, pair
  prohibitions with affirmatives, remove meaningless fallbacks, or make code
  fail fast. Use the simplification workflow when the user asks to simplify the
  docs or comments, tighten verbose prose, de-duplicate documentation, or remove
  redundant comments. Apply changes after the reviewer approves the merged run
  artifact. Members load clean-docs through their backend's skill-loader.
---

# Clean-Docs Skill (CAFleet-orchestrated)

Run one workflow from [Dispatch](#dispatch) using the shared orchestration
below. Each workflow section defines its scan mechanics, classes and checks;
[Review format](#review-format) owns the shared judgment-review rules. Member
roles are defined in [roles.md](roles.md).

Resolve runtime tools from the executing agent's backend. For spawns, read the
selected member backend's Model catalog and Role defaults under the CAFleet
Director's model-selection policy; validate its effort and launch capabilities
there.

## Required reading

Identify the executing backend (members use `CODING AGENT`), resolve its
[runtime bindings](../../../skills/cafleet/reference/coding-agents.md), and load
`cafleet` for BASE, member startup or Director orchestration. Read
[coordination](../../../skills/cafleet-design-doc/reference/coordination.md) for
verbs, pointers and marker pairing, then this shared spine once. Read the
selected workflow section and its required references before scanning or
reviewing. Resolve model tokens from the selected spawn backend's Role defaults
and runtime tokens from the executing backend; use documented defaults only in
their allowed cases. Apply supplied session equivalents for absent optional
host-rule files; route essential unknown prerequisites to the Director before
dependent work.

## Dispatch

Route the user's phrasing to exactly one workflow section and run its full
process through the spine below. When a request names families from more than
one workflow, the Director asks the user which single workflow to run via
`{decision_surface}`.

| When the user wants to… | Run |
|---|---|
| Clean up historical narration / historical residue; remove deprecation notes, "this replaces X" / "renamed from Y" / "previously … now …" prose; drop design-number provenance citations; remove historical sentinel framing while preserving current absence tests | [Residue](#residue) |
| Run an affirmative-writing sweep; fix prohibition-only rule sections; pair prohibitions with affirmatives; remove meaningless fallbacks; make code fail fast | [Affirmative](#affirmative) |
| Simplify the docs / comments; tighten verbose prose; de-duplicate documentation; remove redundant comments | [Simplification](#simplification) |

### Class-to-workflow split

Each candidate belongs to exactly one workflow, selected by its fix:

| Workflow | Owns | Fix character |
|---|---|---|
| **residue** | (a) sentinel framing, (b) narration/citation/trajectory, (c) keep + known-benign | Remove historical framing; preserve current absence tests and reword narration to present tense. Strictly zero behavior change. |
| **affirmative** | **P1** prohibition-pile, **P2** unpaired prohibition (including negatively-phrased instructions rewritable in affirmative voice), **P4** meaningless fallback / swallowed error | Changes prohibition structure, voice, or code behavior. Approved P4 rows are the sole permitted behavior change (invariant 1). |
| **simplification** | **P3** redundant prose, **P5** verbose phrasing | Deletes redundancy or reduces words with voice and behavior unchanged; P5 uses the 30%+ word-reduction baseline. |

A fix that changes prohibition structure, voice or code behavior belongs to
affirmative even when also verbose; the affirmative rewrite subsumes the
tightening. Word-only reductions with unchanged voice belong to simplification.
Past-tense narration belongs to residue. Record another workflow's candidates
in Observations for the Director to report, so the user can schedule that
workflow.

## Three invariants (every workflow, every run)

1. **Preserve contract and behavior.** Every CLI flag, error string, schema,
   command example, IMPORTANT line and cross-reference survives in full
   strength. The sole exception is an approved affirmative P4 row, whose
   deliberate fallback → fail-fast behavior change follows the
   [P4 protocol](#the-p4-behavior-affecting-protocol). Residue and simplification
   remain strictly zero-behavior-change.
2. **Preserve live test coverage.** Keep all test logic, assertions, fixtures,
   parametrizations and names; edit only comments, docstrings and sentinel
   framing. Current-behavior absence tests, including native parser rejection,
   retain every assertion.
3. **Write current behavior (R1).** Every edit reads as a clean present-tense
   statement. Proposed text introduces no "previously / now / formerly", "this
   replaces X" or "renamed from Y" narration.

## Scope and exempt set

Scan the whole tracked tree except `design-docs/`, `researches/` and lock files;
these exempt paths remain untouched. The design/research folders are the
historical record.

| Surface | In scope | Untouchable |
|---|---|---|
| `docs/`, `README.md`, `SPEC.md`, skills, rules | prose structure and wording | contract detail (exact flags, error strings, schemas, layouts), command examples, IMPORTANT / lossless-rule lines, backend-neutral base/overlay separation |
| source code | comments, docstrings, affirmative-workflow P4 fallback sites | flags, options, columns, code paths, log/user-facing/error strings |
| tests | comments, docstrings, sentinel framing | all logic, assertions, fixtures, parametrizations, names |

## Team shape

```
User → /clean-docs (one workflow)
 └─ Director (resolves BASE, registers the current session, partitions slices,
              merges partials, retains native reception, holds apply behind review,
              verifies and closes owned members)
     ├─ scanner-1  (owns a disjoint file slice: scan → propose → apply after gate)
     ├─ scanner-N  (…)
     └─ reviewer   (the workflow's guard, --model {reviewer_model})
```

| Role | Responsibility |
|---|---|
| **Director** | Resolve task-scoped `${BASE}`; register the current Director atomically and arm native reception; assign disjoint whole-file slices; merge partials and hold application behind reviewer approval. Relay approval, apply denied writes through full-file staging, verify, report observations and close owned members before deregistering the fleet. |
| **scanner** | Scan its slice, propose actions, record observations and write its partial artifact under `${BASE}`. After relayed approval, apply its slice's approved rows exactly as written, verify the diff and route denied writes. |
| **reviewer** | Validate the merged artifact before repository edits using the selected workflow's checks. After application, perform its workflow verification. |

**Disjoint file ownership is the concurrency contract:** one scanner owns every
edit to a file, regardless of class. Parallel application therefore needs no
worktrees. A useful default partition is docs+root files / skills+`.claude` /
source / tests; merge or split slices to fit the tree while retaining whole-file
ownership.

## Coordination

Use the `cafleet` verb + pointer schema
(`skills/cafleet-design-doc/reference/coordination.md`) with these skill-local
extensions:

- Add the `scanner` role for `COMMENT(scanner)` markers.
- The whole-run pointer denotes the canonical artifact: `inventory` for
  residue, `findings` for affirmative and simplification. Scanners report
  `complete (<run pointer>)`; the review gate is `approved (<run pointer>)` or
  `blocked (<run pointer>)`. Route individual rows with `<file>:<line>` pointers.

## Per-run process

1. **Resolve `${BASE}`** per [BASE convention](#base-convention).
2. **Bootstrap** — run `cafleet doctor`, then
   `cafleet fleet create --name <name> --coding-agent <backend> --json`; retain
   the returned literal IDs, then follow
   [Director wake](../../../skills/cafleet/roles/director.md#director-wake).
3. **Spawn workers** — scanners for disjoint slices using the other-member
   defaults, plus a reviewer using `--model {reviewer_model}` and supported
   reviewer effort. Render prompts per [Spawn deltas](#spawn-deltas) at
   `${BASE}/.prompts/<role>-<UTC-compact>.md`.
4. **Scan** — each scanner follows its workflow's scan mechanics over its slice
   and writes a partial artifact with a separate Observations section.
5. **Merge + review** — the Director merges partials into the canonical
   artifact and sends the reviewer `ready (<run pointer>)`. Repository edits
   remain gated on the reviewer's `approved (<run pointer>)`.
6. **Apply** — the Director relays approval; scanners apply only their slice's
   approved rows exactly as written, including reviewer REVISE wording. Use
   full-file staging for a denied write.
7. **Verify** — pass the repository's Rust validation tasks
   (`.claude/rules/commands.md`) and the selected workflow verification below.
8. **Report + teardown** — report the applied set and escalated observations,
   commit only when the user asks, close owned ordinary members, verify the
   root-only roster and deregister the fleet.

### Full-file staging

Every scanner reads this protocol before application. For a harness-denied
target write, stage the **complete replacement file** under `${BASE}/.apply/`,
retaining its relative target path. Compare it with the current repository
file: the diff must contain exactly the Director-relayed approved rows,
including reviewer REVISE text. Send the Director the target and staged paths;
continue independent approved work in the rest of your slice.

The Director reads the staged file and diff, checks both against approved rows
**and current user authorization**, then applies that exact file. A mismatch or
changed authorization returns to review/arbitration before writing. Staging
preserves the gate; it is not approval. Scanners and reviewer perform normal
post-apply checks after the Director's write.

### Workflow parameter table

| Parameter | residue | affirmative | simplification |
|---|---|---|---|
| Scan mechanics | Every pattern pass plus hand-inspection of every hit | Full-file judgment read of every file in the slice | Full-file judgment read of every file in the slice |
| Artifact / pointer | `inventory.md` / `inventory` | `findings.md` / `findings` | `findings.md` / `findings` |
| Workflow verification | Re-sweep: zero unaccounted matches; preserve live coverage and behavior, with current-state wording | Diff confined to approved rows; re-validate each P4 row's named coverage and explicit acceptance for an "uncovered" row | Diff confined to approved redundancy deletions and word reductions; voice, behavior, runtime surfaces and live assertions preserved |

## Observations escalation channel

Record content drift (surface disagreements, broken cross-references, docs
contradicting code) and cross-workflow candidates in the artifact's Observations
section, without proposed fixes. The Director escalates these in the run report;
scanners apply only the approved cleanup. The git diff plus green verification
is the deliverable; the run artifact is its audit record.

## BASE convention

Resolve task-scoped BASE through the shared
[task-scope resolver](../../../skills/cafleet-design-doc/reference/guidelines.md#task-scope-resolution):
`researches/clean-docs-<workflow>-<UTC-compact>/` (gitignored, one folder per run).
Use dot-prefixed folders for agent-only scratch: `.prompts/` for spawn renders
and `.apply/` for staged harness-denied writes.

## Spawn deltas

Render each member prompt from the
[canonical spawn skeleton](../../../skills/cafleet/roles/director.md#canonical-spawn-prompt-skeleton)
with the role file referenced by absolute path and this delta. Render each
IMPORTANT item as a separate prompt line, verbatim with `<run pointer>`
substituted.

| Slot | Scanner | Reviewer |
|---|---|---|
| ROLE TITLE / TEAM | `a scanner` / `clean-docs <workflow>` | `the reviewer` / `clean-docs <workflow>` |
| role-file | `roles.md` | `roles.md` |
| ROLE-DEF SUFFIX | `Follow the Scanner section.` | `Follow the Reviewer section.` |
| cafleet-load purpose | `for the broker primitives and command routing through the Director` | `for the broker primitives and command routing through the Director` |
| EXTRA SKILL LOADS | `clean-docs` (shared spine and your workflow's mechanics) | `clean-docs` (shared spine and your workflow's mechanics) |
| CONTEXT LINES | `WORKFLOW: [INSERT workflow]`, `YOUR SLICE: [INSERT the slice's path list]`, `OUTPUT FILE: [INSERT partial-artifact path]` | `WORKFLOW: [INSERT workflow]`, `MERGED ARTIFACT: [INSERT merged-artifact path]` |
| IMPORTANT / ROLE-CONSTRAINT LINES (verbatim) | `IMPORTANT: Follow the CAFleet member Bash protocol (.claude/rules/commands.md § Bash tool) and the host's Bash hygiene rule for all Bash commands.`, `IMPORTANT: If blocked, send a message to the Director immediately instead of assuming.`, `IMPORTANT: Do NOT edit any repository file until the Director relays the reviewer's approved (<run pointer>). The scan phase is read + propose only.`, `IMPORTANT: Do NOT commit code or run git write operations — the Director handles all git.`, `When you see cafleet message poll output with a message from the Director, act on those instructions.` | `IMPORTANT: Follow the CAFleet member Bash protocol (.claude/rules/commands.md § Bash tool) and the host's Bash hygiene rule for all Bash commands.`, `IMPORTANT: If blocked, send a message to the Director immediately instead of assuming.`, `IMPORTANT: Do NOT edit any repository file; your writes are verdict annotations in the run artifact.`, `IMPORTANT: Do NOT commit code or run git write operations — the Director handles all git.`, `When you see cafleet message poll output with a message from the Director, act on those instructions.` |
| start cue (verbatim) | `Begin the scan of your slice.` | `Wait for ready (<run pointer>) from the Director.` |

## Residue

Remove historical narration and sentinel framing while retaining current
behavior and absence tests. This workflow changes prose only and follows the
host's Removal and Affirmative Writing rules. Read those rules and the complete
classification and pattern catalog below before scanning or reviewing.

### Sweep mechanics and Scanner obligations

Run every pattern pass over the assigned tracked slice with the exempt-set
exclusions. Hand-inspect each hit's whole sentence, docstring or test body;
classify from that context. The catalog is a floor: classify relevant hits even
when no listed pattern matched them.

Record **every** hit, including KEEP and known-benign, in the assigned partial
inventory under `${BASE}`. Use a partial table or `COMMENT(scanner)` markers at
the hit's `<file>:<line>` in the inventory, as directed. Each row carries location,
quoted text anchor, rubric class and action. After approved application, re-run
every pass over the slice and report `complete (inventory)` or per-file
`complete (<file>:<line>)`; the partial inventory also reports
`complete (inventory)`.

### Classification

Classify each hit once; the scanner proposes and the reviewer checks its full
context before application.

| Class | Definition | Action |
|---|---|---|
| **(a) Sentinel framing** | Historical framing that advertises a removed or never-added feature; a test asserting current flag/key absence, including native parser rejection, is current coverage. | Remove only the framing; retain every test's logic, assertions, fixtures, parametrizations and names, including absence tests. |
| **(b) Narration / citation / trajectory** | Past-state prose ("previously X, now Y", "no longer / formerly / deprecated", "this replaces X", "renamed from Y", "backfills each pre-existing session"), a design-number-as-reason citation (`design 0000NNN`), trajectory ("inverted by 0000092", "after design 0000124"), or version qualifier ("in v1", "first cut"). | Reword to pure present-tense current behavior or drop the citation clause; retain the behavior description and all coverage. |
| **(c) Keep** | Current behavior, an illustrative example path / fixture slug, or a forward-looking rationale / editing guard ("do not refactor X into Y", "runtime config has no historical value; drop it"). | Keep unchanged. |

#### Known-benign KEEP matches

Keep present-tense false positives unchanged:

- `no longer` describing current behavior: "a deleted member no longer appears"
  or "the loop no longer owns the slot".
- `used to` meaning *utilized to*.
- `preserved for` / `retained for` describing current behavior, rather than
  "preserved for history / forensic visibility" justifying a removed value.
- `\bstale\b` / `STALE` naming live asset-version or skill-install staleness.
- Issue-provenance citations such as `(issue #174 bullet 3)`; the citation
  cleanup covers design numbers, so issue references stay.

#### Decision procedure for one hit

1. Read the hit and its whole surrounding sentence, docstring or test body.
2. For tests, retain all current-behavior coverage, including absence/rejection
   assertions. Classify historical framing as (a); test logic stays KEEP.
3. Otherwise, classify past narration or a design-number-as-reason citation as
   (b), preserving the behavior description and any test.
4. Classify current behavior, illustrative paths/slugs, forward-looking
   rationale and known-benign matches as (c).
5. When (b) versus (c) remains ambiguous, choose KEEP and record the reason in
   the inventory's KEEP list; the reviewer guards against over-deletion.

### Sweep pattern catalog

Sweep tracked files only; the tracked tree selects membership and includes
newly tracked surfaces automatically. Exclude the [exempt set](#scope-and-exempt-set)
from every pass with this pathspec. Extend the lockfile exclusions for every
other lockfile in the slice; the research exclusion covers tracked research
files too:

```
git grep -nIiP -e '<pattern>' -- \
  ':(exclude)design-docs/**' ':(exclude)researches/**' \
  ':(exclude)*.lock' ':(exclude)**/*.lock' \
  ':(exclude)pnpm-lock.yaml' ':(exclude)**/pnpm-lock.yaml' \
  ':(exclude)package-lock.json' ':(exclude)**/package-lock.json' \
  ':(exclude)npm-shrinkwrap.json' ':(exclude)**/npm-shrinkwrap.json' \
  ':(exclude)bun.lockb' ':(exclude)**/bun.lockb'
```

Use every pass with `-i` for case-insensitive matching (`Design 0000NNN`,
`Legacy`) and `-P` for Perl regex, required for the design-number `\b` boundary
on macOS/BSD. `-n` prints line numbers; `-I` skips binary files.

| Pattern (regex) | Catches |
|---|---|
| `deprecat` | deprecation notices |
| `no longer\|formerly\|previously\|used to` | past-state narration; hand-filter known-benign present-tense uses |
| `\blegacy\b` | legacy framing; hand-filter fixture names such as `legacy_squat` |
| `sentinel` | removal-sentinel framing |
| `historical` | historical rows / narration |
| `forensic\|preserved for\|for history` | forensic-visibility / history pointers; hand-filter current `preserved for` / `retained for` |
| `restoration` | restoration-plan pointers |
| `design[ -]?0[0-9]{6}` **and** `\b0[0-9]{6}\b` | design-number-as-reason citations across the full seven-digit ID space; boundaries exclude all-zero UUID constants (`00000000-0000-…`); illustrative slugs stay KEEP |
| `in v1\|first cut` | version qualifiers |
| `backfill\|pre-existing` | migration-backfill narration in prose; keep migration filenames |
| `this replaces\|renamed from\|inverted by\|after design` | replacement / rename / trajectory narration |

### Artifact: `inventory.md`

Merge partial inventories into `${BASE}/inventory.md`, the `inventory` pointer,
with these parts:

- File→action tables grouped by surface / rubric class, with location, quoted
  text anchor, class and action for every hit.
- An explicit KEEP list.
- A **Known-benign sweep matches** subsection for present-tense false positives.
- Observations for content drift and cross-workflow candidates.

The complete inventory is the ephemeral audit record under gitignored
`researches/`; the applied diff and green verification are the deliverable.

### Reviewer checks and completeness guarantee

Before application, re-judge every non-KEEP row against the classification and
guard against over-deletion: retain KEEP and known-benign matches, every runtime
flag/table/column/code path and live assertion. A mixed sentinel test retains
its live assertion, and absence/rejection regressions remain intact. Reworded
prose states current behavior, without "previously / now / no longer (as past) /
formerly", "this replaces X" or "renamed from Y" narration. Account for every
catalog hit as action, KEEP or exempt.

Mark a failed check at its inventory `<file>:<line>` and reply
`blocked (inventory)`, or route the fix with `ready (<file>:<line>)`. After
application, re-run every pass or review the scanners' re-sweep results and
inspect the diff: **zero unaccounted matches** remain; every hit is KEEP-listed,
known-benign or exempt. Confirm that edits remove only historical framing and
narration, preserve behavior and live coverage, and introduce no new narration.

## Affirmative

State desired behavior directly by enforcing the host's Affirmative Writing
rule and `.claude/rules/code-quality.md`, including their legitimacy carve-outs.
Read those prerequisites and [Review format](#review-format) before scanning or
reviewing. Use its shared full-file judgment process, artifact and verdict flow;
this workflow alone permits behavior changes through P4.

### Affirmative finding classes

| Class | Definition | Action |
|---|---|---|
| **P1 prohibition-pile** | A section mostly containing DO-NOT/NEVER bullets without the desired behavior they protect. | State what to do and what correct looks like; retain every genuine hard constraint paired with its affirmative counterpart. |
| **P2 unpaired prohibition** | A "never X" without "instead do Y", or a negative instruction rewritable affirmatively with the same constraint. | Add the pairing or state the same constraint as a pure affirmative instruction. |
| **P4 meaningless fallback / swallowed error** | A defaulting lookup where the key is guaranteed, `value or fallback` masking an invariant, or `try/except` returning a placeholder that hides a condition the caller needs. | Use direct access or raise an error through the P4 protocol below. |

### The P4 BEHAVIOR-AFFECTING protocol

Every P4 row deliberately changes fallback → fail-fast behavior and carries:

- The **BEHAVIOR-AFFECTING** tag.
- The **guaranteed invariant**, explaining why the key/value must exist and the
  fallback masks a bug or corrupt state.
- Its **covering tests**, or the explicit **"uncovered"** mark.
- The reviewer's **individual acceptance**, explicitly recorded for an
  "uncovered" row before application.

The reviewer checks that the invariant is genuinely guaranteed, the fail-fast
replacement is correct, and the named tests exist and cover the site. After
application, re-validate each row's named coverage. Expected, well-specified
absence retains its correct default; proposing it as P4 is a defect.

### Affirmative Reviewer checks

Apply the shared meaning, contract and narration checks in Review format plus
the P4 protocol. Paired prohibitions and documented correct defaults are
compliant, so reject findings that rewrite them as violations. Approve genuine
affirmative rewrites; reject style-only word changes that fail to improve
affirmativeness. Route word-only reductions and past-state narration to
Observations under the class-to-workflow split.

## Simplification

Retain meaning in fewer words using the [Review format](#review-format) full-file
judgment process, artifact and verdict flow. Preserve voice and behavior;
source-code edits cover comments and docstrings only. Candidates changing
prohibition structure, voice or behavior belong to affirmative; past-state
narration belongs to residue. Record them as Observations.

### Simplification finding classes

| Class | Definition | Action |
|---|---|---|
| **P3 redundant prose** | A sentence, comment or docstring restating adjacent prose, a table, a code block, a linked reference or the code itself. | Delete or merge into the surviving statement; retain a code comment only for a constraint the code cannot show. |
| **P5 verbose phrasing** | Prose materially shorter with zero meaning loss; the aggressive baseline is a paragraph that can lose **30%+** of its words with every constraint and precision intact. | Propose the exact tighter text, preserving voice and behavior. |

### Simplification Reviewer checks

Apply the shared meaning, contract and narration checks in Review format.
Require word reductions or redundancy removal with voice and behavior unchanged;
a runtime-surface edit, code change beyond comments/docstrings or voice rewrite
is a defect. Approve genuine tightenings using the 30%+ baseline and reject
style-only churn that changes words without reducing them.

## Review format

This section owns the shared process for affirmative and simplification.
Residue uses its own classification, pattern catalog and inventory.

### Judgment reading and Scanner obligations

Read every tracked file in the assigned slice in full, rather than relying on
grep: structural and redundancy findings require judgment. Use the selected
workflow's classes and propose exact final text. The reviewer also reads
[documentation maintenance](../../rules/documentation-maintenance.md), including
its Coding-agent overlay section, before judging contract or backend-neutrality
changes.

Each scanner writes `${BASE}/findings-<slice>.md` at its assigned output path,
with apply-ready rows and a separate Observations section. The Director merges
these into `${BASE}/findings.md`, consolidating observations and preserving
verdict space per row. The canonical pointer is `findings`.

Report written findings as
`complete (findings) — <one-line count by class>`; after approved application,
diff the slice against its approved rows and report
`complete (findings) — <files changed, +/− lines>`. A blocker marker sits at the
offending row in the findings artifact. Every replacement follows the shared
invariants, with affirmative P4 as the sole approved behavior exception.

### Apply-ready row format

One row or block per finding carries all five fields:

```
location (file:line) | quoted current text (first+last words for long spans)
  | class | exact proposed replacement text (or "delete") | risk note
```

The replacement is final wording, applicable without further judgment. The
class comes from the selected workflow's rubric; affirmative P4 rows also carry
the metadata and acceptance required by its protocol. Favor precise rows that
survive adversarial checks for lost meaning.

### KEEP guardrails

Preserve these surfaces while editing surrounding prose:

- **Contract detail:** `SPEC.md`'s exact CLI options, error strings, schemas,
  key order and text layouts; command facts in
  `skills/cafleet/reference/cli.md`; command examples in skills and docs.
- **Behavioral-contract lines:** IMPORTANT lines, spawn-skeleton lossless-rule
  items, hard role constraints and start cues survive verbatim in meaning.
- **Backend neutrality:** base skills retain neutral instructions under
  `.claude/rules/documentation-maintenance.md` § Coding-agent overlay.
- **Legitimate prohibitions:** a strong prohibition paired with its affirmative
  counterpart is compliant.
- **Correct defaults:** documented correct behavior for expected, valid absence
  is compliant.
- **Runtime surfaces:** flags, options, columns, code paths,
  log/user-facing/error strings, and all test logic, assertions, fixtures,
  parametrizations and names. The P4 protocol supplies invariant 1's sole
  behavior-change exception.
- **Current-state wording:** proposed text follows invariant 3 (R1).

### Decision procedure for one candidate

1. Read its full section, docstring or test-body context.
2. Check KEEP guardrails; a guarded surface produces no finding.
3. Classify by the selected workflow's rubric and draft the exact replacement.
4. Re-read against the original; retain every constraint, condition, qualifier
   and cross-reference. Revise or drop a finding that loses any of them.
5. Drop marginal word changes that neither shrink nor clarify; style-only churn
   fails review.
6. Record content disagreements and other workflows' candidates as Observations,
   outside the findings set.

### Shared Reviewer checks and verdict flow

Check each row against actual file context. Preserve every constraint,
condition, qualifier and cross-reference; exact contract detail, CLI examples,
error strings, IMPORTANT lines, spawn-skeleton lossless items and backend-neutral
base/overlay separation survive verbatim in meaning. Require current-state
wording under invariant 3 and the selected workflow's additional checks.

Annotate every row in the merged `findings.md` with `COMMENT(reviewer)` and
**APPROVE**, **REJECT** (with reason), or **REVISE** (with corrected replacement
text). Send `approved (findings)` when every surviving row is APPROVE or REVISE;
send `blocked (findings)` when Director arbitration is needed.

After application, on the Director's request, compare the diff with approved
rows, confirm only those rows changed and every assertion and contract surface
is preserved, then perform the workflow-specific verification. Send
`approved (findings)` again, or annotate a regression with `COMMENT(reviewer)`
and `blocked (<file>:<line>)`.
