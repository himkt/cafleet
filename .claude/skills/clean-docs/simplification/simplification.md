# Simplification Workflow — De-duplication and Tightening

Review the tracked tree for prose that can be materially shorter — redundant
restatements and verbose phrasing — and apply the reviewer-approved
tightenings. After a run, every touched surface says the same things in fewer
words, with every constraint, contract detail, and live test assertion intact.

The orchestration spine — invariants, scope/exempt set, team shape,
coordination, per-run process, `${BASE}` convention, spawn skeleton — is
canonical in the umbrella [`SKILL.md`](../SKILL.md). This page carries only the
simplification-specific mechanics and artifact. A simplification run is
**strictly zero-behavior-change**: voice and behavior stay unchanged, and
source-code scope is comments and docstrings only. A candidate whose fix would
change prohibition structure, voice, or code behavior belongs to the
affirmative workflow; record it in Observations, never as a finding.

## Trigger scenario

Route here when the user asks to simplify the docs or comments, tighten
verbose prose, de-duplicate documentation, or remove redundant comments.

## Required reading

Read the umbrella [startup and shared orchestration](../SKILL.md#required-reading) once, then this complete workflow before scanning or reviewing. Read the shared [row format, KEEP guardrails and verdict flow](../reference/review-format.md). The reviewer also reads [documentation maintenance](../../../rules/documentation-maintenance.md) and the [backend-neutrality rule](../../../rules/coding-agent-overlay.md) before judging contract or backend-neutrality changes. Use supplied session equivalents for absent optional host-rule files; route an essential unknown prerequisite to the Director before dependent work.

## Judgment mechanics

This is a **judgment review, not a grep**: each scanner reads every file in its
slice in full and proposes exact replacement text — a grep-only pass misses the
redundancy findings that are the point of the run. Classify each finding P3 /
P5 per [Finding classes](#finding-classes) and write apply-ready rows
per the shared [`reference/review-format.md`](../reference/review-format.md).

## Scanner obligations

- **Read your whole slice.** Open every tracked file in your slice in full —
  this is a judgment review; a grep-only pass misses the redundancy and
  phrasing findings that are the point of the run.
- **Propose apply-ready rows.** Classify each finding P3 / P5 per the rubric
  and write the exact final replacement text per the shared row format. Write
  your findings to the `findings-<slice>.md` path named in your spawn prompt,
  under `${BASE}`.
- **Stay zero-behavior-change.** Voice and behavior stay unchanged; in source
  code you touch comments and docstrings only. A candidate whose fix would
  change prohibition structure, voice, or code behavior belongs to the
  affirmative workflow — record it in Observations, never as a finding.
- **Re-verify after apply.** Diff your slice and confirm it contains exactly the
  approved rows, then report.
- **Honor the invariants.** Every replacement preserves all constraints and
  contract detail (invariant 1), all test logic (invariant 2), and introduces
  no narration (invariant 3).

The run pointer is `findings`: report `complete (findings) — <one-line count by class>`
for written findings and `complete (findings) — <files changed, +/− lines>` after
the slice apply; a blocker marker sits at the row in your findings file.

## Reviewer checks

Your sole job is guarding an aggressive prose-tightening run against **lost
meaning, lost contract, and any behavior change**. Per finding row:

1. **Meaning preservation**: the proposed text carries every constraint,
   condition, qualifier, and cross-reference of the original. A shorter text
   that drops one is a defect.
2. **Contract preservation**: SPEC.md contract detail, CLI examples, error
   strings, IMPORTANT lines, spawn-skeleton lossless items, and backend-neutral
   base/overlay separation survive verbatim in meaning.
3. **Zero behavior change**: a simplification run never changes voice or
   behavior. A row that touches a runtime surface, alters code outside comments
   and docstrings, or rewrites voice (an affirmative-workflow P2 candidate) is
   a defect.
4. **No new narration**: no proposed text introduces past-tense framing
   (umbrella invariant 3, R1).
5. **Precision**: approve genuine tightenings (the rubric's 30%+ baseline);
   reject style-only churn that changes words without reducing them.

Write verdicts per the shared [`reference/review-format.md`](../reference/review-format.md)
verdict flow, including its post-apply check.

## Artifact: `findings.md`

Each scanner writes `${BASE}/findings-<slice>.md`: apply-ready rows plus a
separate *Observations* section (content drift and cross-workflow candidates,
per the umbrella `SKILL.md` § *Observations escalation channel*). The Director
merges partials into the run's canonical `${BASE}/findings.md` — verdict space
per row, observations consolidated — the whole-run pointer `findings` and the
run's audit record. The **applied cleanup — the git diff plus green
verification — is the real deliverable**.

## Verification

Beyond the spine's green `mise` gates, the reviewer confirms the git diff is
confined to the approved rows (umbrella `SKILL.md` § *Workflow parameter
table*): only deletions of redundancy and word reductions, voice and behavior
unchanged, no runtime surface or live assertion touched.

## Finding classes

| Class | Definition | Action |
|---|---|---|
| **P3 redundant prose** | A sentence, comment, or docstring that restates what an adjacent sentence, table, code block, linked reference, or the code itself already says. | Delete, or merge into the surviving statement. A code comment survives only if it states a constraint the code cannot show. |
| **P5 verbose phrasing** | Prose rewritable materially shorter with zero meaning loss. Aggressive baseline: a paragraph that can lose **30%+** of its words with no loss of constraint or precision qualifies. | Propose the exact tighter text. |

Drop style-only churn that neither shrinks nor clarifies. P5 changes verbosity only; voice changes belong to affirmative P2 and past-state narration to residue, recorded as Observations. Both classes preserve voice and behavior; source edits cover comments and docstrings only.
