# Finding classes — simplification workflow (canonical)

Every proposal a scanner writes in a simplification run lands in exactly one
class below. The row format, KEEP guardrails, decision procedure, and reviewer
verdict flow are canonical in the shared
[`review-format.md`](../../reference/review-format.md); this page defines only
the classes.

## The two classes

| Class | Definition | Action |
|---|---|---|
| **P3 redundant prose** | A sentence, comment, or docstring that restates what an adjacent sentence, table, code block, linked reference, or the code itself already says. | Delete, or merge into the surviving statement. A code comment survives only if it states a constraint the code cannot show. |
| **P5 verbose phrasing** | Prose rewritable materially shorter with zero meaning loss. Aggressive baseline: a paragraph that can lose **30%+** of its words with no loss of constraint or precision qualifies. | Propose the exact tighter text. |

**Style-only-churn drop rule.** When the value is genuinely marginal — words
change but neither shrink nor clarify — drop the finding; style-only churn
fails review.

P5 is **verbosity only**: a rewrite that changes voice (negative → affirmative)
belongs to the affirmative workflow's P2, and past-tense narration belongs to
the residue workflow. Record such candidates in the Observations section, never
as findings — the boundary tie-break is canonical in the umbrella
[`SKILL.md`](../../SKILL.md) § *Class-to-workflow split*.

Both classes are strictly zero-behavior-change: voice and behavior stay
unchanged, and source-code scope is comments and docstrings only.
