# Finding classes, KEEP guardrails, and the row format (canonical)

Every proposal a scanner writes lands in exactly one class below, and every row
follows the apply-ready format. Both `roles/scanner.md` and `roles/reviewer.md`
cite this page as the single source of truth. The two rules this skill enforces
are `~/.claude/rules/affirmative-writing.md` and `.claude/rules/code-quality.md`
— read both before classifying.

## The five finding classes

| Class | Definition | Action |
|---|---|---|
| **P1 prohibition-pile** | A section that is mostly DO-NOT/NEVER bullets with no statement of the desired behavior the don'ts protect. | Rewrite affirmatively: state what to do and what correct looks like; keep every genuine hard constraint, paired with its affirmative counterpart. |
| **P2 unpaired prohibition** | A "never X" with no "instead do Y". | Add the affirmative pairing, or rephrase as a pure affirmative instruction carrying the same constraint. |
| **P3 redundant prose** | A sentence, comment, or docstring that restates what an adjacent sentence, table, code block, linked reference, or the code itself already says. | Delete, or merge into the surviving statement. A code comment survives only if it states a constraint the code cannot show. |
| **P4 meaningless fallback / swallowed error** | Code: `dict.get` with a default where the key is guaranteed, `value or fallback` masking an invariant, a `try/except` returning a placeholder that hides a condition the caller needs. | Replace with direct access or a raised error. **BEHAVIOR-AFFECTING**: the row must name the guaranteed invariant and the covering test(s) (or state "uncovered"). |
| **P5 verbose or negative phrasing** | Prose rewritable materially shorter, or in affirmative voice, with zero meaning loss. Aggressive baseline: a paragraph that can lose 30%+ of its words with no loss of constraint or precision qualifies. | Propose the exact tighter text. |

## KEEP guardrails (never propose)

- **Contract detail**: SPEC.md's exact CLI options, error strings, schemas, key
  order, and text layouts; `docs/spec/` contract pages; command examples in any
  skill or doc. The detail IS the contract — de-duplicate around it, never thin it.
- **Behavioral-contract lines**: IMPORTANT lines, spawn-skeleton lossless-rule
  items, hard role constraints, start cues. These survive verbatim in meaning.
- **Backend neutrality**: base skill files stay free of backend specifics
  (per `.claude/rules/coding-agent-overlay.md`).
- **Legitimate prohibitions**: a strong "never X" paired with its affirmative
  counterpart is compliant, not a P1/P2 finding.
- **Correct defaults**: a default that is the documented correct behavior for an
  expected absence is compliant, not a P4 finding.
- **Runtime surfaces**: flags, options, columns, code paths, log/user-facing/
  error strings; test logic, assertions, fixtures, parametrizations, test names.
- **No new narration (R1)**: no proposed text introduces "previously / now /
  formerly / renamed from / this replaces".

## Apply-ready row format

One row (or block) per finding, carrying all five fields:

```
location (file:line) | quoted current text (first+last words for long spans)
  | class | exact proposed replacement text (or "delete") | risk note
```

"Apply-ready" means the replacement field is the final wording — an editor can
apply the row without further judgment. P4 rows additionally carry the
BEHAVIOR-AFFECTING tag, the invariant justification, and the covering tests.
Precision beats volume: every row must survive an adversarial reviewer checking
for lost meaning.

## Observations (drift channel — separate from findings)

While reading, a scanner often notices **content drift**: two pages disagreeing,
a broken cross-reference, a doc contradicting the code, an internal
inconsistency. Drift is not simplification — record each such item in the
findings file's *Observations* section (location + what disagrees with what) and
propose nothing. The Director escalates observations to the user; fixing one
requires a ruling on which side is authoritative, which is the user's call.

## Decision procedure for one candidate

1. Read the candidate with its full surrounding context (the whole section,
   docstring, or test body) — never judge from a fragment.
2. Check the KEEP guardrails first; a guarded surface produces no finding.
3. Classify P1–P5 and draft the exact replacement text.
4. Re-read the replacement against the original: every constraint, condition,
   qualifier, and cross-reference must survive. If any is lost, revise or drop
   the finding.
5. When simplification value is genuinely marginal (words change but neither
   shrink nor clarify), drop the finding — style-only churn fails review.
6. Content disagreement between surfaces → the Observations section, not a
   finding.
