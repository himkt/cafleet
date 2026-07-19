# Finding classes — affirmative workflow (canonical)

Every proposal a scanner writes in an affirmative run lands in exactly one
class below. The row format, KEEP guardrails, decision procedure, and reviewer
verdict flow are canonical in the shared
[`review-format.md`](../../reference/review-format.md); this page defines only
the classes. The two rules this workflow enforces are
`~/.claude/rules/affirmative-writing.md` and `.claude/rules/code-quality.md` —
read both before classifying.

## The three classes

| Class | Definition | Action |
|---|---|---|
| **P1 prohibition-pile** | A section that is mostly DO-NOT/NEVER bullets with no statement of the desired behavior the don'ts protect. | Rewrite affirmatively: state what to do and what correct looks like; keep every genuine hard constraint, paired with its affirmative counterpart. |
| **P2 unpaired prohibition** | A "never X" with no "instead do Y" — or any negatively-phrased instruction rewritable in affirmative voice carrying the same constraint. | Add the affirmative pairing, or rephrase as a pure affirmative instruction carrying the same constraint. |
| **P4 meaningless fallback / swallowed error** | Code: `dict.get` with a default where the key is guaranteed, `value or fallback` masking an invariant, a `try/except` returning a placeholder that hides a condition the caller needs. | Replace with direct access or a raised error. **BEHAVIOR-AFFECTING**: the row must name the guaranteed invariant and the covering test(s), or state "uncovered" explicitly — protocol in [`affirmative.md`](../affirmative.md). |

A candidate whose fix only removes words or duplication with unchanged voice
belongs to the simplification workflow (P3/P5); past-tense narration belongs to
the residue workflow. Record such candidates in the Observations section, never
as findings — the boundary tie-break is canonical in the umbrella
[`SKILL.md`](../../SKILL.md) § *Class-to-workflow split*.

## Legitimacy carve-outs (compliant, never findings)

- A strong "never X" **paired** with its affirmative counterpart is compliant,
  not a P1/P2 finding.
- A default that is the **documented correct behavior** for an expected, valid
  absence is compliant, not a P4 finding.

The test for P4: a fallback is legitimate when absence or variation is an
expected, well-specified case; it is a finding when absence signals a bug or
corrupt state and the fallback merely hides it.
