# Shared review format (affirmative + simplification)

Canonical for the two judgment workflows: the apply-ready row format every
scanner writes, the KEEP guardrails every proposal must clear, the
per-candidate decision procedure, and the reviewer's per-row verdict flow. Each
judgment workflow's `reference/rubric.md` defines its classes and cites this
page. The residue workflow is self-contained — its rubric and pattern catalog
carry their own guardrails and procedure.

## Apply-ready row format

One row (or block) per finding, carrying all five fields:

```
location (file:line) | quoted current text (first+last words for long spans)
  | class | exact proposed replacement text (or "delete") | risk note
```

"Apply-ready" means the replacement field is the final wording — an editor can
apply the row without further judgment. The class field names one of the
workflow's rubric classes. In the affirmative workflow, P4 rows additionally
carry the BEHAVIOR-AFFECTING tag, the invariant justification, and the covering
tests (or the explicit "uncovered" mark). Precision beats volume: every row
must survive an adversarial reviewer checking for lost meaning.

## KEEP guardrails (never propose)

- **Contract detail**: SPEC.md's exact CLI options, error strings, schemas, key
  order, and text layouts; `docs/spec/` contract pages; command examples in any
  skill or doc. The detail IS the contract — de-duplicate around it, never thin it.
- **Behavioral-contract lines**: IMPORTANT lines, spawn-skeleton lossless-rule
  items, hard role constraints, start cues. These survive verbatim in meaning.
- **Backend neutrality**: base skill files stay free of backend specifics
  (per `.claude/rules/coding-agent-overlay.md`).
- **Legitimate prohibitions**: a strong "never X" paired with its affirmative
  counterpart is compliant, not a finding.
- **Correct defaults**: a default that is the documented correct behavior for an
  expected absence is compliant, not a finding.
- **Runtime surfaces**: flags, options, columns, code paths, log/user-facing/
  error strings; test logic, assertions, fixtures, parametrizations, test names.
- **No new narration**: no proposed text introduces "previously / now /
  formerly / renamed from / this replaces" (invariant 3 in the umbrella
  [`SKILL.md`](../SKILL.md)).

## Decision procedure for one candidate

1. Read the candidate with its full surrounding context (the whole section,
   docstring, or test body) — never judge from a fragment.
2. Check the KEEP guardrails first; a guarded surface produces no finding.
3. Classify per your workflow's rubric and draft the exact replacement text.
4. Re-read the replacement against the original: every constraint, condition,
   qualifier, and cross-reference must survive. If any is lost, revise or drop
   the finding.
5. When the value is genuinely marginal (words change but neither shrink nor
   clarify), drop the finding — style-only churn fails review.
6. Content disagreement between surfaces, or a candidate owned by another
   workflow, goes to the artifact's Observations section per the umbrella
   `SKILL.md`, never into a finding row.

## Reviewer verdict flow

The reviewer writes a per-row verdict — **APPROVE**, **REJECT** (with reason),
or **REVISE** (with the corrected replacement text) — as `COMMENT(reviewer)`
annotations in the merged `findings.md`, then sends the Director:

- `approved (findings)` when every surviving row is APPROVE or REVISE;
- `blocked (findings)` when the set needs Director arbitration.

**After apply**, on the Director's request: check the git diff against the
approved rows, confirm nothing beyond them changed and no assertion or contract
surface was lost, then send `approved (findings)` again (or flag the regression
with a `COMMENT(reviewer)` marker and `blocked (<file>:<line>)`).
