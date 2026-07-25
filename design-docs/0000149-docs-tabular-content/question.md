## Questions

1. [Section: Overview] Should this 20-page conversion execute as one unit, or staged? | Options: A) Single branch/PR for the entire design doc B) One PR per Implementation step (per page group) C) Split into several design docs per page group
2. [Section: Overview] The Overview claims conversion "across all 20 pages" while Background exempts fleet-isolation.md, the docs/api stubs, and index.md; which framing should the doc state? | Options: A) Reword to "every page with qualifying content" and list exemptions in Success Criteria B) Keep "all 20 pages" — exempt pages satisfy the rule vacuously C) Move the exemption list into the Overview
3. [Section: Overview] The "five places where a table is misused" are never enumerated as a set; should the doc name the five misuse fixes and map each to its task? | Options: A) Yes, add a five-row list in Background B) No, the audit files carry the mapping C) Name them inline in the affected tasks
4. [Section: Overview] The change is docs/-only with no behavior change; confirm README.md and SPEC.md need no sync this cycle. | Options: A) Correct — no README/SPEC edits needed B) Run /update-readme at the end as a drift check C) SPEC.md should record the new section anchors
5. [Section: Success Criteria] How is criterion 1 (every qualifying enumeration is a table) verified — by completing the tasks or by an independent re-audit? | Options: A) Task completion is the proof B) A fresh re-audit pass over all pages at Step 9 C) Spot-check only the three recurring patterns from Background
6. [Section: Success Criteria] Does "No table on the site has exactly one data row" bind pre-existing tables untouched by this work? | Options: A) Yes — sitewide; fix any found during Step 9 B) Only tables added or edited by this work C) Sitewide, but untouched violations become follow-up findings
7. [Section: Success Criteria] How does the two-sentence cell cap treat verbatim multi-sentence contract strings (error messages)? | Options: A) A quoted verbatim string counts as one unit B) The cap applies literally; restructure such rows C) Exempt cells whose entire content is a contract string
8. [Section: Success Criteria] What concretely satisfies "the docs site builds and every anchor resolves"? | Options: A) mkdocs build --strict (or the project's equivalent) passes B) Build plus a dedicated link/anchor checker C) Manual verification of only the changed anchors D) Add anchor-check tooling as part of this work
9. [Section: Success Criteria] Steps 2, 4, and 6 delete prose, but no criterion guards against facts being lost that the new tables do not carry. Add a fact-parity criterion? | Options: A) Yes — add a criterion plus a per-deletion parity check in Step 9 B) No — the task wording "now carried by the table" is the guard C) Add it as a Step 9 task rather than a criterion
10. [Section: Background] Are the four findings-*.md audit files committed with the design doc and normative during implementation (columns, row keys, surviving prose)? | Options: A) Committed and normative — tasks defer to them for detail B) Committed but informative; the design doc tasks are the only contract C) Left uncommitted as working notes
11. [Section: Background] Should each Implementation task cite its finding ID(s) for traceability to the 59-finding audit? | Options: A) Yes — annotate tasks with finding references B) No — page-level grouping is traceable enough C) Only for tasks that deviate from the finding's proposal
12. [Section: Background] "Two pages need no tabulation work" then names fleet-isolation.md plus four api stubs (five files); fix the count or the framing? | Options: A) Reword to name the surfaces without a count B) Say "five pages" C) Leave as is — the api stubs count as one surface
13. [Section: Background] The three dropped findings: is the user expected to ratify the drops here, or are they final? | Options: A) Ratify in this interview and record as confirmed B) Final as written on auditor/author judgment C) Reconsider the multiplexer design-principles drop
14. [Section: Background] docs/how-to/mixed-backend-team.md appears only as a dropped finding; did the audit find no other qualifying content there, or is coverage missing? | Options: A) Confirmed — no other findings on that page B) Re-check the page during Step 9 C) Add its remaining findings to Implementation now
15. [Section: Specification] Should the Specification fix a rendering convention for the new tables (code spans for literals, em-dash for empty cells, alignment)? | Options: A) Yes — add a short conventions paragraph B) No — follow each page's existing style C) Only the em-dash empty-cell convention needs stating
16. [Section: The tabulate rule] Two-subject comparisons (tmux vs herdr) pass the "three or more parallel items" bar only if the behaviors count as the items; should the rule state that items may sit on either axis? | Options: A) Yes — clarify orientation in the rule B) Treat the backend matrix as a documented exception C) Leave it implicit
17. [Section: The tabulate rule] Step 4 converts the member-targeting "three numbered resolution rules" and the stale-assets numbered list to tables, while Step 6 keeps backend-selection's "three numbered precedence items"; what distinguishes convertible rule lists from keepable ones? | Options: A) Lookup/decision rules become tables; genuinely ordered precedence stays a list — state this in the rule B) Convert the backend-selection precedence items too C) Keep the member-targeting rules as a list instead
18. [Section: Ownership of duplicated enumerations] What form does "every other mention becomes a link" take? | Options: A) An inline link within a sentence naming the owning page B) A bare "See X" pointer sentence C) A link plus a one-clause summary, never restating attributes D) Implementer's judgment per site
19. [Section: Ownership of duplicated enumerations] The observed drift ("the unenrolled watcher" vs "the monitoring member"): which wording becomes canonical in the owning table? | Options: A) The Core terms table's term, per the user-facing-docs rule B) "the monitoring member" C) "the unenrolled watcher" D) Decide during implementation and record the choice in the design doc
20. [Section: Ownership of duplicated enumerations] After de-duplication, may non-owning prose carry any quantitative echo of the owned facts (e.g., "minutes-scale intervals")? | Options: A) Zero numbers — a pure link B) Magnitude words allowed, exact values forbidden C) Implementer's judgment per sentence
21. [Section: Ownership of duplicated enumerations] Is there a direction constraint on cross-page links (the plan links spec→concepts for enrollment and concepts→spec for effort)? | Options: A) No constraint; link wherever the owner is B) Prefer concepts→spec; spec pages stay self-contained C) Follow the existing site convention and record it in Step 1
22. [Section: Sourcing constraint] How are the two deliberately empty cells rendered? | Options: A) Em-dash, matching the output-shape table convention B) Literally empty C) "(not documented)" D) Restructure those tables to avoid the empty cell
23. [Section: Sourcing constraint] Copying scope text from cli-options into overview.md creates a second home for those facts; verbatim copy, paraphrase, or copy-plus-link? | Options: A) Short verbatim copy is fine for one-clause scope text B) Paraphrase to the concepts altitude C) Copy plus a link to the source section
24. [Section: Sourcing constraint] Should the two facts no page states (writable_roots "why", an MCP bypass example) be filed as follow-up documentation gaps? | Options: A) Yes — record them as follow-ups in the design doc B) No — the empty cells are the record C) Fill them in this change by consulting the implementation
25. [Section: Constraints on every edit] How are all inbound links to a retargeted section found? | Options: A) An rg sweep at edit time plus the Step 9 build check B) Rely on the strict build alone C) Keep an anchor-change ledger in the design doc and sweep once at the end
26. [Section: Constraints on every edit] Do the new `##` sections require mkdocs nav/TOC configuration changes, or are they page-internal only? | Options: A) Page-internal; nav lists pages, not sections B) Check mkdocs.yml during implementation and update if needed C) Audit nav for section anchors before starting
27. [Section: Step 1: Record the convention] Should the tabulate convention also land in .claude/rules so future agents follow it, or in contributing.md only? | Options: A) contributing.md only — it is the contributor surface B) Both contributing.md and a project rule C) A project rule only
28. [Section: Step 1: Record the convention] Is Step 1 a gate that must land before Steps 2–8 begin? | Options: A) Yes — first commit, consistent with docs-first ordering B) Any order within the branch C) Last, once the wording is proven by the completed work
29. [Section: Step 2: `docs/spec/coding-agent-backends.md`] Three additions anchor near ## Spawn argv; confirm the final section order. | Options: A) Spawn argv (argv table, then capability matrix) → Model selection → Reasoning effort B) Model selection and Reasoning effort before the capability matrix C) Implementer decides from page flow
30. [Section: Step 2: `docs/spec/coding-agent-backends.md`] Task 6 keeps each backend section's "posture, rationale, and subsections" while criterion 2 bans per-backend prose restating model/effort facts; may rationale prose mention a model or level at all? | Options: A) Yes, as rationale only — never as the enumerating statement of the contract B) No model/effort values anywhere outside the two tables C) Values allowed only inside sentences that link to the tables
31. [Section: Step 2: `docs/spec/coding-agent-backends.md`] With writable_roots' "Why" cell empty, is a three-row Why column still the right shape? | Options: A) Keep the column with one empty cell B) Drop the column; keep the whys as prose beneath C) Use a two-column table plus a rationale paragraph
32. [Section: Step 2: `docs/spec/coding-agent-backends.md`] Are the rejection strings and validation messages in the new tables re-verified against the current CLI before landing, or copied from existing docs text? | Options: A) Copy from existing docs verbatim — the docs are the contract record B) Re-verify against the CLI/tests and flag drift as a separate finding C) Re-verify and correct silently in the same edit
33. [Section: Step 3: `docs/spec/webui-api.md`] Should each per-endpoint error table link the shared fleet-scoping table, or is the single introduction enough? | Options: A) One introduction; per-endpoint tables stay lean B) Each endpoint's Errors block links the shared table C) Repeat the two shared rows per endpoint
34. [Section: Step 3: `docs/spec/webui-api.md`] If the endpoint count at implementation time differs from nine, what governs? | Options: A) The live page — one row per endpoint whatever the count, correcting the doc B) Nine is asserted; a mismatch halts for investigation C) Drop the count from the task now
35. [Section: Step 3: `docs/spec/webui-api.md`] The GET /api/monitor table deliberately adds no running-case column; where does the running case live afterward? | Options: A) Adjacent prose kept next to the table B) It is already covered by the endpoint's main field documentation C) Reconsider and add a third column
36. [Section: Step 3: `docs/spec/webui-api.md`] Does the message-endpoint comparison table become the single home for ordering/row-cap facts with per-endpoint prose trimmed, or is it a summary layered on top? | Options: A) Single home; trim per-endpoint restatements B) Summary; per-endpoint sections keep their own statements C) Per-fact judgment recorded in the audit files
37. [Section: Step 4: `docs/spec/cli-options.md`] The --effort pointer's target table duplicates the enumeration owned by coding-agent-backends.md § Reasoning effort (with different columns); which resolution? | Options: A) Point at the owning table; keep only the --model format constraint locally B) Swap ownership — cli-options carries the CLI-contract table and coding-agent-backends links to it C) Both pages carry tables (accept the duplication) D) Merge the extra columns into the owning table and point there
38. [Section: Step 4: `docs/spec/cli-options.md`] Error Messages gains Command, Exit, and Notes columns across two tasks; one table redesign or two sequential edits? | Options: A) One redesign, both checkboxes ticked together B) Sequential edits as written C) Merge the two tasks into one
39. [Section: Step 4: `docs/spec/cli-options.md`] The output-shape table covers "all 19 subcommands" while the summary table has 22 rows; what explains the difference, and is it verified at implementation time? | Options: A) Verify against the live page and record the reason for the delta in the doc B) The counts should match; reconcile them now C) Trust the audit's counts as written
40. [Section: Step 4: `docs/spec/cli-options.md`] How are sentences in the trailing output paragraphs that the new output-shape table does NOT carry handled when those paragraphs are deleted? | Options: A) Keep them; delete only the carried sentences B) Whole-paragraph deletion per the audit's survival notes C) Case-by-case with a parity check in Step 9
41. [Section: Step 4: `docs/spec/cli-options.md`] Adding missing --quiet rows to the message send/ack flag tables: is --quiet confirmed to exist on those subcommands today (a docs gap, not new behavior)? | Options: A) Yes — the flags exist; the tables are incomplete B) Verify against the CLI first; halt on mismatch C) The audit already verified it; proceed
42. [Section: Step 4: `docs/spec/cli-options.md`] What fills the env-var table's "Overridden by" column? | Options: A) The overriding CLI flag/setting where one exists, em-dash otherwise B) Only rows with an overrider keep a value; drop the column if sparse C) Rename it "Precedence" and state the full chain per row
43. [Section: Step 5: `docs/spec/data-model.md` and `docs/spec/message-envelope.md`] The summary matrix's PK/FK facts: sourced from data-model.md's existing prose or from the SPEC.md DDL? | Options: A) From the page prose; SPEC drift is out of scope B) Cross-check against SPEC.md DDL and flag mismatches C) From SPEC.md directly as the source of truth
44. [Section: Step 5: `docs/spec/data-model.md` and `docs/spec/message-envelope.md`] Clarify "treat the --full label list as closed". | Options: A) Use only labels the page already documents; em-dash any field without one B) Omit rows lacking a --full label C) The label set may be completed from CLI output if a gap appears
45. [Section: Step 6: `docs/spec/multiplexer-backends.md`] Are the eight behavioral-delta rows already enumerated in the audit file, or derived from the five bullet pairs during implementation? | Options: A) Enumerated in findings-spec-rest.md; follow it B) Derived while editing; the count is approximate C) They should be enumerated in this design doc before implementation starts
46. [Section: Step 6: `docs/spec/multiplexer-backends.md`] The selection truth table covers six of the eight combinations of three variables; should it state why the other two are absent? | Options: A) Yes — a one-line note explaining the omitted combinations B) No — six rows plus the numbered precedence items suffice C) Cover all eight, marking degenerate rows
47. [Section: Step 6: `docs/spec/multiplexer-backends.md`] After the matrix absorbs the five bullet pairs, do thin per-section remainders stay as sections or merge? | Options: A) Sections stay; each keeps its rationale and exceptions B) Merge any section left with under two sentences C) The audit file decides per section
48. [Section: Step 7: `docs/concepts/`] The rubric's `working` row currently disagrees between table and prose; what is the truth source for the new fourth column? | Options: A) Verify against the monitor role definition/implementation before writing B) The prose is correct; the table is amended to match C) The table is correct; the prose is fixed
49. [Section: Step 7: `docs/concepts/`] Where does the stall row's "(0 disables)" move when it leaves the "Set by" cell? | Options: A) The wake-reason table's "Silenced by" column B) A Notes column on the knob table C) Prose beneath the knob table
50. [Section: Step 7: `docs/concepts/`] What is the row-key source for the five member classes in the enrolled-member table? | Options: A) The audit file's proposed rows B) monitoring.md's existing prose enumerations C) Re-derive from the enrollment behavior and reconcile any drift
51. [Section: Step 7: `docs/concepts/`] The new coding-agents.md backend table's "Auto-approval posture" column overlaps the coding-agent-backends.md capability matrix's "Shell-command posture" — is this an eighth duplicated enumeration needing an owner? | Options: A) Yes — pick one owner and link from the other B) No — different altitude; keep both with differentiated wording C) Drop the posture column from the concepts table
52. [Section: Step 7: `docs/concepts/`] member-lifecycle § Commands offers "a slim lifecycle-role table or a link"; which, given the subcommand enumeration is owned by cli-options? | Options: A) Link only — a slim table risks re-duplicating the owned enumeration B) A slim table keyed by lifecycle stage (not restating purpose/flags) plus the link C) Decide during implementation
53. [Section: Step 8: Entry points and how-to] In the quickstart Configure conversion, where do the two kept code snippets sit relative to the new table? | Options: A) Below the table, each introduced by the backend it belongs to B) Above the table, as today C) Referenced from a table cell and placed in per-backend subsections
54. [Section: Step 8: Entry points and how-to] Is contributing.md's "seven pick-one tasks" count verified against the live mise task set at implementation time? | Options: A) Yes — one row per task whatever the count then B) Seven is asserted; a mismatch halts for investigation C) Reference mise.toml as the source and keep the table minimal
55. [Section: Step 9: Verify] Should the one-row-table and two-sentence-cell rules gain an automated check to prevent regression after this change? | Options: A) Yes — a small lint script in CI or pre-commit B) No — the contributing.md convention plus review suffices C) Manual check now; tooling as a follow-up design doc
56. [Section: Step 9: Verify] Who executes Step 9 — the implementer or an independent reviewer? | Options: A) The execute workflow's Reviewer/Verifier role B) The implementer, recording results in the design doc C) Both — implementer first, reviewer confirms
57. [Section: Implementation] The header says "0/57 tasks" but the checklist appears to contain 68 checkboxes; which is corrected? | Options: A) Recount and set the Progress line and Overview's task count to the checkbox total B) 57 is intended — identify which checkboxes are not countable tasks C) Reconcile in the first implementation commit
58. [Section: Implementation] Link-replacement tasks depend on owning tables existing first; should the doc state an owners-before-linkers execution order? | Options: A) Yes — one ordering note atop Implementation B) Reorder the steps so owners always precede linkers C) No constraint — everything lands on one branch, so transient dangling links are acceptable
59. [Section: Implementation] What commit granularity applies across the 9 steps? | Options: A) One commit per step B) One commit per page C) One commit per task D) Implementer's judgment within the project's git rules
60. [Section: Changelog] What gets recorded in the Changelog during implementation? | Options: A) One row per completed step B) Only substantive spec revisions arising from the interview or implementation C) A single completion row at the end
Total: 60 questions

## Answers

### Round 1 (Questions 1-4)

1. A) Single branch/PR for the entire design doc
2. A) Reword to "every page with qualifying content" and list exemptions in Success Criteria — DISCREPANCY: Overview wording must change
3. A) Add a five-row misuse list in Background — DISCREPANCY: Background must gain the list
4. A) Correct — no README/SPEC edits needed this cycle

### Round 2 (Questions 5-8)

5. A) Task completion is the proof for criterion 1
6. A) Sitewide — fix any one-row table found during Step 9 — DISCREPANCY: criterion must state sitewide scope; Step 9 needs the fix task
7. A) A quoted verbatim contract string counts as one unit — DISCREPANCY: the two-sentence cap needs this clause
8. A) The project's strict docs build passing is the check — DISCREPANCY: criterion 8 should name the concrete command

### Round 3 (Questions 9-12)

9. A) Add a fact-parity criterion plus a per-deletion parity check in Step 9 — DISCREPANCY: new criterion + new Step 9 task
10. A) The four findings-*.md files are committed and normative — DISCREPANCY: the doc must state this status
11. A) Annotate every Implementation task with its finding reference(s) — DISCREPANCY: 68 tasks need finding IDs
12. A) Reword to name the exempt surfaces without a count — DISCREPANCY: "Two pages" wording is wrong

### Round 4 (Questions 13-16)

13. A) Ratify the three dropped findings in this interview and record them as user-confirmed — DISCREPANCY: doc must record ratification
14. A) Confirmed — the audit found nothing else qualifying on mixed-backend-team.md
15. A) Add a short table-rendering conventions paragraph to Specification — DISCREPANCY: new Specification content
16. A) Clarify in the tabulate rule that counted items may sit on either axis — DISCREPANCY: rule wording must change

### Round 5 (Questions 17-20)

17. A) Lookup/decision rules become tables; genuinely ordered precedence stays a list — DISCREPANCY: state this in the tabulate rule
18. C) A link plus a one-clause summary, never restating the owned attributes — DISCREPANCY: Ownership section must specify the link form
19. A) Use the Core terms table's term, per the user-facing-docs rule — DISCREPANCY: doc must name the tie-break source
20. B) Magnitude words allowed, exact values forbidden — DISCREPANCY: Ownership section must state the echo rule

### Round 6 (Questions 21-24)

21. A) No direction constraint — link wherever the owning table lives
22. A) Em-dash, matching the output-shape table convention — DISCREPANCY: Sourcing constraint must state the rendering
23. B) Paraphrase to the concepts altitude rather than copying verbatim — DISCREPANCY: Step 8 overview.md task wording must change
24. A) Record both missing facts as follow-up documentation gaps — DISCREPANCY: doc needs a follow-ups record

### Round 7 (Questions 25-28)

25. A) An rg sweep at edit time plus the Step 9 build check — DISCREPANCY: constraint must state the sweep
26. B) Check the site config during implementation and update if needed — DISCREPANCY: add the config check
27. B) Both contributing.md and a project rule — DISCREPANCY: Step 1 gains a project-rule task
28. A) Yes — Step 1 is the first commit and gates Steps 2-8 — DISCREPANCY: state the gate in Implementation

### Round 8 (Questions 29-32)

29. A) Spawn argv (argv table, then capability matrix) -> Model selection -> Reasoning effort
30. A) Values may appear as rationale only, never as the enumerating statement of the contract — DISCREPANCY: reconcile task 6 with criterion 2
31. A) Keep the Why column with one em-dash cell
32. B) Re-verify strings against the CLI/tests and flag drift as a separate finding — DISCREPANCY: add the verification instruction

### Round 9 (Questions 33-36)

33. A) One introduction; per-endpoint tables stay lean
34. A) The live page governs — one row per endpoint whatever the count, correcting the doc — DISCREPANCY: drop the hard-coded nine
35. A) Keep the running case as adjacent prose next to the table
36. A) Single home; trim per-endpoint restatements — DISCREPANCY: task must say the prose is trimmed

### Round 10 (Questions 37-40)

37. D) Merge the extra columns into the owning table and point cli-options there — DISCREPANCY: resolves an ownership conflict between two tasks
38. A) One table redesign, both checkboxes ticked together — DISCREPANCY: merge the two Error Messages tasks
39. A) Verify against the live page and record the reason for the 19-vs-22 delta — DISCREPANCY: unexplained count mismatch
40. A) Keep the sentences the table does not carry; delete only carried prose — DISCREPANCY: deletion tasks need this qualifier

### Round 11 (Questions 41-44)

41. B) Verify --quiet against the CLI first; halt on mismatch — DISCREPANCY: task must not assert an unverified flag
42. A) The overriding flag/setting where one exists, em-dash otherwise
43. B) Cross-check against the SPEC.md DDL and flag mismatches — DISCREPANCY: task must name the verification source
44. A) Use only labels the page documents; em-dash any field without one — DISCREPANCY: "closed" needs this definition

### Round 12 (Questions 45-48)

45. A) The eight rows are enumerated in findings-spec-rest.md; follow it
46. A) Add a one-line note explaining the two omitted combinations — DISCREPANCY: task must require the note
47. A) Sections stay; each keeps its rationale and exceptions
48. A) Verify the working state against the monitor role definition/implementation before writing — DISCREPANCY: task must name the truth source

### Round 13 (Questions 49-52)

49. A) Into the wake-reason table's "Silenced by" column
50. C) Re-derive the five member classes from enrollment behavior and reconcile any drift — DISCREPANCY: task must name the source
51. A) Yes — an eighth duplicated enumeration; pick one owner and link from the other — DISCREPANCY: Ownership table must gain a row
52. B) A slim table keyed by lifecycle stage plus the link — DISCREPANCY: task must resolve the either/or

### Round 14 (Questions 53-56)

53. A) Below the table, each snippet introduced by the backend it belongs to
54. A) One row per live mise task, whatever the count is then — DISCREPANCY: drop the hard-coded seven
55. C) Manual check now; automated tooling as a follow-up design doc — DISCREPANCY: record the follow-up
56. C) Both — implementer runs and records, reviewer confirms — DISCREPANCY: Step 9 must state ownership

### Round 15 (Questions 57-60)

57. A) Recount to 68 — verified: 76 checkboxes total minus 8 Success Criteria = 68 implementation tasks — DISCREPANCY: Progress line says 0/57
58. A) One owners-before-linkers ordering note atop Implementation — DISCREPANCY: dependency order is unstated
59. A) One commit per Implementation step — DISCREPANCY: granularity is unstated
60. Free text: "no need to have changelog" — remove the Changelog section entirely. The standard template marks Changelog optional, so its removal is compliant — DISCREPANCY: section must be deleted
