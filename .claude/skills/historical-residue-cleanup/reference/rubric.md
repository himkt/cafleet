# Classification rubric (canonical)

Every swept hit lands in **exactly one** class. Both `roles/scanner.md` and
`roles/reviewer.md` cite this page as the single source of truth. The scanner
classifies; the reviewer re-checks the classification before any edit lands.

## The three classes

| Class | Definition | Action |
|---|---|---|
| **(a) Sentinel test** | A test whose *only* job is to assert that a removed / never-added flag or key produces an error (e.g. a Click "No such option" rejection, a dropped wrapper key asserted absent). | **Delete outright.** If a single test *mixes* a sentinel with live coverage, **keep the live assertion and drop only the sentinel framing** — never lose coverage. |
| **(b) Narration / citation / trajectory** | Prose that narrates the past: "previously X, now Y", "no longer / formerly / deprecated", "this replaces X", "renamed from Y", "backfills each pre-existing session", a design-number-as-reason citation (`design 0000NNN`), a trajectory note ("inverted by 0000092", "after design 0000124"), or a version qualifier ("in v1", "first cut"). | **Reword to pure present-tense current behavior** (describe only what the code does now) **or drop the citation clause**. Keep the behavior description and all coverage. |
| **(c) Keep** | A current-behavior statement, an illustrative example path / fixture slug (not a citation-as-reason), or a forward-looking rationale / editing guard ("do not refactor X into Y", "runtime config has no historical value; drop it"). | **Keep unchanged.** |

## The known-benign class (a KEEP sub-case worth naming)

Present-tense English that trips a broad sweep pattern but is **not** historical
narration — reword nothing. The scanner confirms each remaining sweep hit falls
in this benign / KEEP / exempt class before declaring the sweep clean.

- `no longer` used as *current* behavior — "a canceled message no longer appears",
  "the loop no longer owns the slot". Present-tense description of what happens
  now, not a "formerly X" narration.
- `used to` meaning *utilized to* (not *formerly*).
- `preserved for` / `retained for` describing *current* behavior (not "preserved
  for history / forensic visibility" as a removed-value justification).
- `\bstale\b` / `STALE` naming a **live** feature — monitor-liveness staleness,
  skill-install staleness. The word names current behavior, not a removed guard.
- Issue-provenance citations (`(issue #174 bullet 3)`). Issue provenance is in
  scope only for *design-number* citations, not issue numbers — keep issue refs.

## Decision procedure for one hit

1. Read the hit **plus its surrounding context** (the whole sentence, docstring,
   or test body) — never classify from the matched substring alone.
2. If the hit is a test that exists *only* to assert a removed/never-added
   flag/key errors → **(a)**. If the test mixes a sentinel with a live assertion,
   the *test* is KEEP but the sentinel framing inside it is **(a)** (drop the
   framing, keep the live assertion).
3. Else, if the hit narrates the past or cites a design number as the reason for a
   behavior → **(b)**. Reword to present tense / drop the citation clause; never
   delete the behavior description or a test.
4. Else, if the hit is present-tense current behavior, an illustrative example
   path / fixture slug, a forward-looking rationale, or a known-benign match →
   **(c) Keep**.
5. When (b) vs (c) is genuinely ambiguous, default to **Keep** and record the hit
   in the inventory's KEEP list with the reason — over-deletion is the failure
   mode the reviewer guards hardest against.

## Invariants this rubric enforces

- **No runtime behavior removed.** No class-(a) or class-(b) action ever deletes a
  flag, table, column, code path, or live assertion — only sentinel framing and
  narration prose.
- **No live coverage lost.** A class-(a) deletion of a mixed test keeps every live
  assertion; a class-(b) reword keeps the behavior description and the test.
- **No new narration introduced (R1).** Every reworded string reads as a clean
  present-tense statement of current behavior — no "previously / now / no longer
  (as past) / formerly", no "this replaces X", no "renamed from Y".
