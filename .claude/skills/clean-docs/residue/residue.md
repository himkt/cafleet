# Residue Workflow — Historical Residue Cleanup

Sweep the tracked tree for **historical narration** (past-state prose, "this
replaces X", "renamed from Y", design-number provenance citations, trajectory
notes, version qualifiers) and historical sentinel framing, preserving current-behavior absence tests, per `~/.claude/rules/removal.md` and
`~/.claude/rules/affirmative-writing.md`. After a run, every artifact reads as
a clean present-tense statement of current behavior — and no behavior, no flag,
no column, no code path, and no live test assertion is lost.

The orchestration spine — invariants, scope/exempt set, team shape,
coordination, per-run process, `${BASE}` convention, spawn skeleton — is
canonical in the umbrella [`SKILL.md`](../SKILL.md). This page carries only the
residue-specific mechanics, artifact, and guarantee. A residue run is
**strictly zero-behavior-change**: the affirmative workflow's P4 carve-out does
not apply here.

## Trigger scenario

Route here when the user asks to clean up historical narration or historical
residue, remove deprecation notes / "this replaces X" / "renamed from Y" /
"previously … now …" prose, drop design-number provenance citations, or remove historical sentinel framing.

## Required reading

Read the umbrella [startup and shared orchestration](../SKILL.md#required-reading) once, then this complete workflow before scanning or reviewing. Read [every pattern pass and exclusion](reference/patterns.md), plus the host removal and affirmative-writing rules. Use supplied session equivalents for absent optional host-rule files; route an essential unknown prerequisite to the Director before dependent work.

## Sweep mechanics

Each scanner runs **every** pass of [`reference/patterns.md`](reference/patterns.md)
over its slice — `git grep -nIiP` with the exempt-set exclusion — then
hand-inspects every hit with its surrounding context (the whole sentence,
docstring, or test body) and classifies it per
[Classification](#classification): (a) sentinel framing, (b)
narration/citation/trajectory, (c) keep, plus the known-benign KEEP sub-case.
The catalog is a **floor, not a ceiling**: a scanner may classify a hit no
pattern named. A single grep is never sufficient, and no hit is ever classified
from the matched substring alone.

## Artifact: `inventory.md`

Partial inventories merge into the run's canonical `${BASE}/inventory.md`, the
whole-run pointer `inventory`. It has four parts:

- Grouped **file→action inventory** tables (by surface / by rubric class), each
  row = location + quoted text anchor + rubric class + action.
- An explicit **KEEP list** to prevent over-deletion.
- A **"Known-benign sweep matches"** subsection listing the present-tense false
  positives (see the rubric's known-benign class).
- An **Observations** section for content drift and cross-workflow candidates,
  per the umbrella `SKILL.md` § *Observations escalation channel*.

The inventory records **every** hit — including KEEP and known-benign — so the
merged inventory can prove the sweep is complete. It is the run's audit record
(ephemeral, under gitignored `researches/`); the **applied cleanup — the git
diff plus green verification — is the real deliverable**.

## Guarantee: provable completeness

After apply, re-running every pass over the swept tree yields **zero
unaccounted matches**: every remaining hit is KEEP-listed, known-benign, or
exempt. This re-sweep is the residue workflow's verification (umbrella
`SKILL.md` § *Workflow parameter table*); the reviewer confirms it, and
confirms no live coverage or runtime behavior was lost and no new narration was
introduced (invariant 3, R1).

## Classification

Classify every hit once; the scanner proposes and the reviewer checks the full context before application.

| Class | Definition | Action |
|---|---|---|
| **(a) Sentinel framing** | Historical framing that advertises a removed or never-added feature. A test asserting the current absence of a flag/key (including native parser rejection) is current-behavior coverage. | Remove the historical framing while retaining all test logic, assertions, fixtures, parametrizations and names; keep current-behavior absence tests. |
| **(b) Narration / citation / trajectory** | Prose that narrates the past: "previously X, now Y", "no longer / formerly / deprecated", "this replaces X", "renamed from Y", "backfills each pre-existing session", a design-number-as-reason citation (`design 0000NNN`), a trajectory note ("inverted by 0000092", "after design 0000124"), or a version qualifier ("in v1", "first cut"). | **Reword to pure present-tense current behavior** (describe only what the code does now) **or drop the citation clause**. Keep the behavior description and all coverage. |
| **(c) Keep** | A current-behavior statement, an illustrative example path / fixture slug (not a citation-as-reason), or a forward-looking rationale / editing guard ("do not refactor X into Y", "runtime config has no historical value; drop it"). | **Keep unchanged.** |

### The known-benign class (a KEEP sub-case worth naming)

Present-tense English that trips a broad sweep pattern but is **not** historical
narration — reword nothing. The scanner confirms each remaining sweep hit falls
in this benign / KEEP / exempt class before declaring the sweep clean.

- `no longer` used as *current* behavior — "a deleted member no longer appears",
  "the loop no longer owns the slot". Present-tense description of what happens
  now, not a "formerly X" narration.
- `used to` meaning *utilized to* (not *formerly*).
- `preserved for` / `retained for` describing *current* behavior (not "preserved
  for history / forensic visibility" as a removed-value justification).
- `\bstale\b` / `STALE` naming a **live** feature — monitor-liveness staleness,
  skill-install staleness. The word names current behavior, not a removed guard.
- Issue-provenance citations (`(issue #174 bullet 3)`). Issue provenance is in
  scope only for *design-number* citations, not issue numbers — keep issue refs.

### Decision procedure for one hit

1. Read the hit **plus its surrounding context** (the whole sentence, docstring,
   or test body) — never classify from the matched substring alone.
2. For a test, preserve all current-behavior coverage, including absence/rejection assertions. Classify historical sentinel framing as **(a)** and remove only that framing; the test logic stays KEEP.
3. Else, if the hit narrates the past or cites a design number as the reason for a
   behavior → **(b)**. Reword to present tense / drop the citation clause; never
   delete the behavior description or a test.
4. Else, if the hit is present-tense current behavior, an illustrative example
   path / fixture slug, a forward-looking rationale, or a known-benign match →
   **(c) Keep**.
5. When (b) vs (c) is genuinely ambiguous, default to **Keep** and record the hit
   in the inventory's KEEP list with the reason — over-deletion is the failure
   mode the reviewer guards hardest against.

### Invariants this rubric enforces

The umbrella [`SKILL.md`](../SKILL.md)'s three invariants, realized per class:

- **No runtime behavior removed (invariant 1).** No class-(a) or class-(b) action
  ever deletes a flag, table, column, code path, or live assertion — only sentinel
  framing and narration prose. A residue run is strictly zero-behavior-change.
- **No live coverage lost (invariant 2).** A class-(a) framing edit
  keeps every live assertion; a class-(b) reword keeps the behavior description
  and the test.
- **No new narration introduced (invariant 3, R1).** Every reworded string reads
  as a clean present-tense statement of current behavior — no "previously / now /
  no longer (as past) / formerly", no "this replaces X", no "renamed from Y".
