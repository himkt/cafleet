# Drafter clarifying questions — design 0000118 (SPEC/docs vs impl consistency)

I have read `audit-findings.md` in full and spot-checked 4 cited locations
(`models.py:73` NOT NULL, `messaging.py:218` `to_agent_id=0`, `fleet.py:83/106`
positional `FLEET_ID`, `formatters.py:219` EM DASH) — all confirmed accurate.

I understand the default policy: align docs→code for the ~24 doc-only items, and
flag 2.1/2.2/2.4/3.1/4.1 as explicit CODE fixes. These 4 questions cover the
items the directive leaves genuinely open. No design-doc file will be created
until I have answers.

## Q1 — [DECISION NEEDED, contested] `tasks.to_agent_id` NULL-vs-0 (items 1.1 + 1.2)

Code: schema NOT NULL; broadcast_summary rows write `to_agent_id = 0`; `to:`
surfacing uses truthiness (`if to_id:`, `if task.get("to_agent_id")`). Docs/SPEC
§5.5 say nullable, NULL on broadcast_summary, "no 0 sentinel", forbid truthiness
— and SPEC §5.5 labels the NULL-no-sentinel design **"resolved"**. Verifiers
split 3 code / 2 doc. Two coherent resolutions:

- **(A) Align docs → code** — keep NOT NULL + `0` sentinel + truthiness; rewrite
  SPEC:299 / 371-379 / 662-663 / 2612 + data-model.md:80 + message-envelope.md:87
  to document the `0` sentinel. No code or migration change.
- **(B) Align code → SPEC "resolved" design** — migrate schema to nullable, write
  NULL on broadcast_summary, change both truthiness checks (queries.py:68-70,
  formatters.py:39-40) to `is None` / `IS NULL`. Needs an Alembic migration;
  `--json` output changes `0`→`null`.

Which resolution?

## Q2 — 3.5 hidden `--full` / `--quiet` / etc. (SPEC §10 "no hidden flags")

The directive names "the SPEC-section half of 3.5" as a CODE fix, which conflicts
with the audit's own preference ("keep flags hidden, align docs"). Which direction?

- **(A) Code fix** — unhide the flags so behavior honors SPEC §10 "no hidden
  flags"; cli-options.md "documented" then becomes correct as-is.
- **(B) Doc fix** — keep flags hidden; rewrite SPEC §10 + cli-options.md to state
  these flags are intentionally hidden (the audit's recommendation).

## Q3 — 2.2 vs 3.3 broker-guard exit codes (consistency)

Both are broker guards raising `click.UsageError` (exit 2). 2.2 is classified a
code fix (→ exit 1 / `ClickException`). 3.3 the audit resolves doc-side (document
exit 2). The audit flags the tension explicitly. Which?

- **(A) Normalize both** broker guards to `ClickException` (exit 1), matching the
  sibling guard at `member.py:328-331` — 3.3 also becomes a code fix.
- **(B) Keep divergent** — 2.2 code-fix to exit 1, 3.3 doc-align to exit 2.

## Q4 — [Scope] Executable code fixes, or proposals-only?

Your request says specify each edit "so it is directly executable"; the audit
says code fixes are "surfaced to the user for confirmation." For the code-side
items (2.1, 2.2, 2.4, 3.1, 4.1, plus whatever Q2/Q3 select), should the design
doc's Implementation section:

- **(A) Include them as actionable tasks** to implement now, with the
  accompanying test updates — one executable design doc covering both doc and
  code edits; or
- **(B) Document them as recommended fixes** pending your separate sign-off,
  while only the doc-alignment edits are executed in this cycle?
