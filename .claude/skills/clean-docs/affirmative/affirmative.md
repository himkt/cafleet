# Affirmative Workflow — Affirmative-Writing Enforcement

Review the tracked tree for prose and code that violates the affirmative-writing
rules — prohibition piles lacking a positive spec, unpaired "never X" rules,
negatively-phrased instructions, silent-fallback code — per
the host's Affirmative Writing rule and `.claude/rules/code-quality.md`,
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

Read the umbrella [startup and shared orchestration](../SKILL.md#required-reading) once, then this complete workflow before scanning or reviewing. Read the shared [row format, KEEP guardrails and verdict flow](../reference/review-format.md). Read the host's Affirmative Writing rule and `.claude/rules/code-quality.md`, including their legitimacy carve-outs. The reviewer also reads [documentation maintenance](../../../rules/documentation-maintenance.md) and the [backend-neutrality rule](../../../rules/coding-agent-overlay.md) before judging contract or backend-neutrality changes. Use supplied session equivalents for absent optional host-rule files; route an essential unknown prerequisite to the Director before dependent work.

## Judgment mechanics

This is a **judgment review, not a grep**: each scanner reads every file in its
slice in full and proposes exact replacement text — a grep-only pass misses the
structural findings that are the point of the run. Classify each finding P1 /
P2 / P4 per [Finding classes](#finding-classes) and write apply-ready
rows per the shared [`reference/review-format.md`](../reference/review-format.md).

## Scanner obligations

- **Read your whole slice.** Open every tracked file in your slice in full —
  this is a judgment review; a grep-only pass misses the structural P1/P2
  findings that are the point of the run.
- **Propose apply-ready rows.** Classify each finding P1 / P2 / P4 per the
  rubric and write the exact final replacement text per the shared row format.
  Every P4 row carries the BEHAVIOR-AFFECTING tag, the invariant justification,
  and the covering tests (or the explicit "uncovered" mark). Write your
  findings to the `findings-<slice>.md` path named in your spawn prompt, under
  `${BASE}`.
- **Re-verify after apply.** Diff your slice and confirm it contains exactly the
  approved rows, then report.
- **Honor the invariants.** Every replacement preserves all constraints and
  contract detail (invariant 1 — an approved P4 row is the sole behavior-change
  exception), all test logic (invariant 2), and introduces no narration
  (invariant 3).

The run pointer is `findings`: report `complete (findings) — <one-line count by class>`
for written findings and `complete (findings) — <files changed, +/− lines>` after
the slice apply; a blocker marker sits at the row in your findings file.

## Reviewer checks

Your sole job is guarding an affirmative-writing enforcement run against **lost
meaning, lost contract, and unjustified behavior change**. Per finding row:

1. **Meaning preservation**: the proposed text carries every constraint,
   condition, qualifier, and cross-reference of the original. A rewrite that
   drops one is a defect.
2. **Contract preservation**: SPEC.md contract detail, CLI examples, error
   strings, IMPORTANT lines, spawn-skeleton lossless items, and backend-neutral
   base/overlay separation survive verbatim in meaning.
3. **Behavior (P4)**: every BEHAVIOR-AFFECTING row is individually justified —
   the invariant is genuinely guaranteed, the fail-fast replacement is correct,
   and the named test coverage exists and covers the site. Accept an
   "uncovered" row only explicitly, as your recorded individual acceptance. A
   P4 row whose absence-case is a legitimate expected state is a defect.
4. **No new narration**: no proposed text introduces past-tense framing
   (umbrella invariant 3, R1).
5. **Legitimacy carve-outs**: a row that "fixes" a paired prohibition or a
   correct default is a defect — those are compliant per the host's Affirmative
   Writing rule.
6. **Precision**: approve genuine affirmative rewrites; reject style-only churn
   that changes words without improving affirmativeness.

Write verdicts per the shared [`reference/review-format.md`](../reference/review-format.md)
verdict flow. After apply, check the git diff against the approved rows and
re-validate each P4 row's named coverage.

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

Per the host's Affirmative Writing rule (§ *What's legitimate*) and `code-quality.md`:

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
