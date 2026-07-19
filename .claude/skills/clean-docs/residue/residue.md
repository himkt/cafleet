# Residue Workflow — Historical Residue Cleanup

Sweep the tracked tree for **historical narration** (past-state prose, "this
replaces X", "renamed from Y", design-number provenance citations, trajectory
notes, version qualifiers) and **historical-guard residue** (removal-sentinel
tests) and remove it per `~/.claude/rules/removal.md` and
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
"previously … now …" prose, drop design-number provenance citations, or delete
removal-sentinel tests.

## Required reading

Identify your coding agent first — a member's spawn prompt names it on the
`CODING AGENT:` line; the Director (main session) uses its own identity — then
Read your overlay and **resolve** it before your first action.

**Load-bearing — Read in order before acting:**

| # | Read | What you lose if you skip it |
|---|------|------------------------------|
| 1 | your overlay [`../../../../skills/cafleet/reference/coding-agent/<name>-overlay.md`](../../../../skills/cafleet/reference/coding-agent/) — read **and resolve** it (see *Resolve your overlay* in the cafleet `SKILL.md`) | you emit a literal `{skill_loader}` / `{decision_surface}`, guess a wrong value, or ignore a backend note |
| 2 | the `cafleet` skill's [`reference/base-dir.md`](../../../../skills/cafleet/reference/base-dir.md) | the task-scope BASE resolution, the no-bypass write protocol, and the `<unset>` contract — you mis-root run artifacts or fall back to `/tmp` |
| 3 | the `cafleet-design-doc` skill's [`reference/coordination.md`](../../../../skills/cafleet-design-doc/reference/coordination.md) | the verb + pointer + `COMMENT(role)` schema and the clean-docs extensions (`scanner` role, `inventory` pointer) — your status hops mis-route |
| 4 | this workflow's [`reference/rubric.md`](reference/rubric.md) | the fixed classification rubric ((a)/(b)/(c) + known-benign) — you mis-classify hits or over-delete |
| 5 | this workflow's [`reference/patterns.md`](reference/patterns.md) | the multi-pass pattern catalog and the exempt-set exclusion — your sweep is shallow, breaking the multi-pass discipline |

## Sweep mechanics

Each scanner runs **every** pass of [`reference/patterns.md`](reference/patterns.md)
over its slice — `git grep -nIiP` with the exempt-set exclusion — then
hand-inspects every hit with its surrounding context (the whole sentence,
docstring, or test body) and classifies it per
[`reference/rubric.md`](reference/rubric.md): (a) sentinel test, (b)
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
