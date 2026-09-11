# Affirmative Workflow — Affirmative-Writing Enforcement

Review the tracked tree for prose and code that violates the affirmative-writing
rules — prohibition piles lacking a positive spec, unpaired "never X" rules,
negatively-phrased instructions, silent-fallback code — per
`~/.claude/rules/affirmative-writing.md` and `.claude/rules/code-quality.md`,
then apply the reviewer-approved rewrites. After a run, every touched surface
states the desired behavior directly, with every constraint, contract detail,
and live test assertion intact.

The orchestration spine — invariants, scope/exempt set, team shape,
coordination, per-run process, `${BASE}` convention, spawn skeleton — is
canonical in the umbrella [`SKILL.md`](../SKILL.md). This page carries only the
affirmative-specific mechanics, artifact, and protocol. This is the **only**
workflow permitted to change code behavior, and only through the P4 protocol
below.

## Trigger scenario

Route here when the user asks to run an affirmative-writing sweep, fix
prohibition-only rule sections, pair prohibitions with affirmatives, remove
meaningless fallbacks, or make code fail fast.

## Required reading

Read the umbrella [startup and shared orchestration](../SKILL.md#required-reading) once, then this complete workflow before scanning or reviewing. Read the shared [row format, KEEP guardrails and verdict flow](../reference/review-format.md). Read `~/.claude/rules/affirmative-writing.md` and `.claude/rules/code-quality.md`, including their legitimacy carve-outs. Use supplied session equivalents for absent optional host-rule files; route an essential unknown prerequisite to the Director before dependent work.

## Judgment mechanics

This is a **judgment review, not a grep**: each scanner reads every file in its
slice in full and proposes exact replacement text — a grep-only pass misses the
structural findings that are the point of the run. Classify each finding P1 /
P2 / P4 per [Finding classes](#finding-classes) and write apply-ready
rows per the shared [`reference/review-format.md`](../reference/review-format.md).

## Artifact: `findings.md`

Each scanner writes `${BASE}/findings-<slice>.md`: apply-ready rows plus a
separate *Observations* section (content drift and cross-workflow candidates,
per the umbrella `SKILL.md` § *Observations escalation channel*). The Director
merges partials into the run's canonical `${BASE}/findings.md` — verdict space
per row, observations consolidated — the whole-run pointer `findings` and the
run's audit record. The **applied cleanup — the git diff plus green
verification — is the real deliverable**.

## The P4 BEHAVIOR-AFFECTING protocol

A P4 fix (fallback → fail-fast) deliberately changes code behavior — the sole
exception to invariant 1, existing only in this workflow. Every P4 row:

- carries the **BEHAVIOR-AFFECTING** tag;
- names the **guaranteed invariant** that makes the fallback meaningless (why
  the key/value is guaranteed to exist);
- names its **covering tests**, or is explicitly marked **"uncovered"**;
- lands only with the reviewer's **individual acceptance** — an "uncovered" row
  only with that acceptance made explicit.

The reviewer validates each P4 row against the code and the named coverage
before approving it; the post-apply check re-validates that the diff is
confined to approved rows and each P4 row's named coverage holds.

## Legitimacy carve-outs (compliant, never findings)

Per `affirmative-writing.md` § *What's legitimate* and `code-quality.md`:

- A **paired prohibition** — a strong "never X" standing next to its
  affirmative counterpart ("always use Y") — is compliant, not a P1/P2 finding.
- A **correct default** — a default that is the documented correct behavior for
  an expected, valid absence — is compliant, not a P4 finding.

A row that "fixes" either is itself a defect the reviewer rejects.

## Finding classes

| Class | Definition | Action |
|---|---|---|
| **P1 prohibition-pile** | A section that is mostly DO-NOT/NEVER bullets with no statement of the desired behavior the don'ts protect. | Rewrite affirmatively: state what to do and what correct looks like; keep every genuine hard constraint, paired with its affirmative counterpart. |
| **P2 unpaired prohibition** | A "never X" with no "instead do Y" — or any negatively-phrased instruction rewritable in affirmative voice carrying the same constraint. | Add the affirmative pairing, or rephrase as a pure affirmative instruction carrying the same constraint. |
| **P4 meaningless fallback / swallowed error** | Code: `dict.get` with a default where the key is guaranteed, `value or fallback` masking an invariant, a `try/except` returning a placeholder that hides a condition the caller needs. | Replace with direct access or a raised error. **BEHAVIOR-AFFECTING**: the row must name the guaranteed invariant and the covering test(s), or state "uncovered" explicitly — protocol in [the P4 protocol](#the-p4-behavior-affecting-protocol). |

The umbrella class-to-workflow split routes word-only changes to simplification and past-state narration to residue; record these as Observations. A fallback qualifies as P4 when absence signals a bug or corrupt state and the fallback hides it. Expected, well-specified absence retains its correct default.
