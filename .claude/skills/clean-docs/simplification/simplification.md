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

Identify your coding agent first — a member's spawn prompt names it on the
`CODING AGENT:` line; the Director (main session) uses its own identity — then
Read your overlay and **resolve** it before your first action.

**Load-bearing — Read in order before acting:**

| # | Read | What you lose if you skip it |
|---|------|------------------------------|
| 1 | your overlay [`../../../../skills/cafleet/reference/coding-agent/<name>-overlay.md`](../../../../skills/cafleet/reference/coding-agent/) — read **and resolve** it (see *Resolve your overlay* in the cafleet `SKILL.md`) | you emit a literal `{skill_loader}` / `{decision_surface}`, guess a wrong value, or ignore a backend note |
| 2 | the `cafleet` skill's [`reference/base-dir.md`](../../../../skills/cafleet/reference/base-dir.md) | the task-scope BASE resolution, the no-bypass write protocol, and the `<unset>` contract — you mis-root run artifacts or fall back to `/tmp` |
| 3 | the `cafleet-design-doc` skill's [`reference/coordination.md`](../../../../skills/cafleet-design-doc/reference/coordination.md) | the verb + pointer + `COMMENT(role)` schema and the clean-docs extensions (`scanner` role, `findings` pointer) — your status hops mis-route |
| 4 | this workflow's [`reference/rubric.md`](reference/rubric.md) | the P3/P5 classes, the 30%+ baseline, and the style-only-churn drop rule — you propose churn or mis-scope voice rewrites |
| 5 | the shared [`reference/review-format.md`](../reference/review-format.md) | the apply-ready row format, KEEP guardrails, decision procedure, and verdict flow — your proposals are unreviewable or unsafe |

## Judgment mechanics

This is a **judgment review, not a grep**: each scanner reads every file in its
slice in full and proposes exact replacement text — a grep-only pass misses the
redundancy findings that are the point of the run. Classify each finding P3 /
P5 per [`reference/rubric.md`](reference/rubric.md) and write apply-ready rows
per the shared [`reference/review-format.md`](../reference/review-format.md).

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
