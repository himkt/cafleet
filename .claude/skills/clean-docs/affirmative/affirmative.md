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

Identify your coding agent first — a member's spawn prompt names it on the
`CODING AGENT:` line; the Director (main session) uses its own identity — then
Read your overlay and **resolve** it before your first action.

**Load-bearing — Read in order before acting:**

| # | Read | What you lose if you skip it |
|---|------|------------------------------|
| 1 | your overlay [`../../../../skills/cafleet/reference/coding-agent/<name>-overlay.md`](../../../../skills/cafleet/reference/coding-agent/) — read **and resolve** it (see *Resolve your overlay* in the cafleet `SKILL.md`) | you emit a literal `{monitor_model}` / `{reviewer_model}` / `{skill_loader}`, guess a wrong value, or ignore a backend note |
| 2 | the `cafleet` skill's [`reference/base-dir.md`](../../../../skills/cafleet/reference/base-dir.md) | the task-scope BASE resolution, the no-bypass write protocol, and the `<unset>` contract — you mis-root run artifacts or fall back to `/tmp` |
| 3 | the `cafleet-design-doc` skill's [`reference/coordination.md`](../../../../skills/cafleet-design-doc/reference/coordination.md) | the verb + pointer + `COMMENT(role)` schema and the clean-docs extensions (`scanner` role, `findings` pointer) — your status hops mis-route |
| 4 | this workflow's [`reference/rubric.md`](reference/rubric.md) | the P1/P2/P4 classes — you mis-classify or miss the P2 voice absorption |
| 5 | the shared [`reference/review-format.md`](../reference/review-format.md) | the apply-ready row format, KEEP guardrails, decision procedure, and verdict flow — your proposals are unreviewable or unsafe |
| 6 | `~/.claude/rules/affirmative-writing.md` and `.claude/rules/code-quality.md` | the two rules this workflow enforces, including their legitimacy carve-outs — you reject compliant text or approve violations |

## Judgment mechanics

This is a **judgment review, not a grep**: each scanner reads every file in its
slice in full and proposes exact replacement text — a grep-only pass misses the
structural findings that are the point of the run. Classify each finding P1 /
P2 / P4 per [`reference/rubric.md`](reference/rubric.md) and write apply-ready
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
