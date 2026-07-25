# Tabular content across the documentation site

**Status**: Draft
**Progress**: 0/57 tasks complete
**Last Updated**: 2026-07-25

COMMENT(user-relay): The Progress counter reads 0/57 but the Implementation checklist holds 68 checkboxes (76 in the file minus the 8 Success Criteria). Verified by count during the interview — correct it to 0/68 tasks complete.

COMMENT(user-relay): Reword "across all 20 pages" to "every page with qualifying content". The current claim contradicts Background, which exempts fleet-isolation.md, the four docs/api stubs, and index.md; the exempt list moves into Success Criteria.

## Overview

The docs site states parallel, multi-attribute content as prose, bullets, and repeated per-item sections, forcing readers to reconstruct comparison matrices in their heads. This document converts that content to tables across all 20 pages, fixes five places where a table is misused, and assigns a single owning page to every enumeration that is currently duplicated.

COMMENT(user-relay): Add the exempt-page list here (fleet-isolation.md, the four docs/api stubs, index.md), so criterion 1's scope is stated where the criterion lives rather than only in Background.

COMMENT(user-relay): The one-row-table rule is sitewide, binding pre-existing tables this work never touches. State that scope in the criterion, and add a Step 9 task to fix any one-row table found.

COMMENT(user-relay): Add a clause to the two-sentence cell cap: a verbatim quoted contract string (an error message, an output line) counts as one unit regardless of its internal sentence count. Without it the cap conflicts with the verbatim-preservation constraint.

COMMENT(user-relay): Name the concrete check for the build criterion — the project's strict docs build passing. "Every anchor resolves" is otherwise unverifiable as written.

COMMENT(user-relay): Add a fact-parity criterion. Steps 2, 4, and 6 delete prose, and no current criterion guards against losing a fact the replacing table does not carry. Pair it with the per-deletion parity check added to Step 9.

## Success Criteria

- [ ] Every enumeration of three or more parallel items carrying two or more shared attributes renders as a table, on all 20 docs pages.
- [ ] `docs/spec/coding-agent-backends.md` answers "which models and effort levels does backend X accept, and what does a wrong value produce" from one row per backend, with no per-backend prose section restating it.
- [ ] `docs/spec/webui-api.md` answers "what does status N mean on endpoint E" from one row, with the two fleet-scoping errors stated once rather than three times.
- [ ] The five-state rubric table in `docs/concepts/monitoring.md` gives the same answer as the prose beneath it for all five states, including `working`.
- [ ] Each of the seven duplicated enumerations in the Ownership table below appears as a table on exactly one page; every other mention is a link.
- [ ] No table on the site has exactly one data row, and no table cell holds more than two sentences.
- [ ] No repository file path is introduced into any page outside the surfaces exempted by the user-facing-docs rule.
- [ ] The docs site builds and every intra-site anchor added or retargeted by this work resolves.

---

COMMENT(user-relay): Add a five-row list naming the five table-misuse fixes and mapping each to its Implementation task. The Overview counts them but the set is never enumerated anywhere in this document.

COMMENT(user-relay): State that the four findings-*.md audit files are committed alongside this document and are normative during implementation — tasks defer to them for proposed columns, row keys, and which prose survives.

COMMENT(user-relay): "Two pages need no tabulation work" then names fleet-isolation.md plus four docs/api stubs — five files, not two. Reword to name the exempt surfaces without a numeral.

COMMENT(user-relay): Record that the three dropped findings were reviewed and ratified by the user during the interview, so the drops read as a confirmed decision rather than unreviewed author judgment.

## Background

Four auditors read all 20 pages in full and produced 59 findings: 54 tabulate-me candidates and 5 table-misuse candidates. The audit records are `findings-cli-options.md`, `findings-spec-rest.md`, `findings-concepts.md`, and `findings-entry-howto-api.md` in this directory; each finding carries its exact location, proposed columns, row keys, and the prose that must survive the change.

Three patterns recur across the site:

| Pattern | Where it dominates | Shape |
|---|---|---|
| Repeated per-item paragraph | `coding-agent-backends.md`, `multiplexer-backends.md`, `coding-agents.md`, `design-doc-development.md` | N parallel subjects each get a prose section restating the same attribute in a different sentence shape |
| Attribute present on every row but never a column | `cli-options.md` Error Messages, `monitoring.md` rubric and knob tables, `message-envelope.md` field table | The table exists; the attribute sits in a parenthetical inside another cell, or in the paragraph below |
| Enumeration compressed into one cell or one clause | `cli-options.md` `--effort` row, `model-selection.md` fail-closed paragraph, `multiplexer-backends.md` Esc safeguard | A comparison set is written as a semicolon list inside a table cell or mid-sentence |

Two pages need no tabulation work: `docs/concepts/fleet-isolation.md` and the four `docs/api/*` mkdocstrings stubs have nothing enumerable. `docs/index.md` is a landing page whose value is narrative.

Three audited findings are deliberately **not** implemented, because tabulating them would not help a reader:

| Dropped finding | Reason |
|---|---|
| `multiplexer-backends.md` § Design principles as a table | Four bold-lead bullets already scan on the same axis; the auditor recommended against it |
| `model-selection.md` § Replacement bounds as a table | Three one-line rules; a bulleted list reads better, and the section is already clear once the two other tables on that page land |
| `mixed-backend-team.md` appendix walkthrough as a table | Trimming a copy-pasteable walkthrough costs the reader more than the redundancy does |

`cli-options.md`'s proposal to merge the `--json` availability table into the 22-row subcommand summary is also reduced: a seven-column summary table is harder to scan than the six-column one it replaces, so only the minimum fix applies (re-key the `--json` table one row per subcommand).

---

COMMENT(user-relay): Add a short table-rendering conventions paragraph covering code spans for literals, the em-dash for empty cells, and column alignment. Roughly forty new tables land across twenty pages, and without a stated convention they will not come out consistent.

## Specification

COMMENT(user-relay): Clarify that the counted "three or more parallel items" may sit on either axis — as rows or as columns. A two-subject comparison such as tmux vs herdr only clears the bar if the behaviors are the counted items, which the current wording leaves ambiguous.

COMMENT(user-relay): State what separates a convertible rule list from one that stays a list: lookup and decision rules become tables, genuinely ordered precedence stays a numbered list. Step 4 tabulates the member-targeting resolution rules while Step 6 keeps backend-selection's precedence items, and nothing currently explains the difference.

### The tabulate rule

Content becomes a table when **three or more parallel items each carry two or more of the same attributes**. Content stays prose when it is a single item, an ordered procedure, or rationale for one decision.

| Content | Form |
|---|---|
| Three or more parallel items, two or more shared attributes | Table |
| Ordered steps a reader performs in sequence | Numbered list |
| Constraints or requirements with no shared attribute axis | Bulleted list |
| One item, however many attributes | Prose |
| Rationale, caveats, and "why" for a decision | Prose, adjacent to the table it qualifies |

Two anti-rules follow from the same principle, and both are enforced by this document's success criteria: a table with exactly one data row costs a header row and buys nothing over a sentence; a cell holding more than two sentences defeats the vertical scan the table shape promises. When a finding moves an enumeration into a table, the surrounding rationale prose stays — a table replaces the enumeration, never the contract prose around it.

COMMENT(user-relay): Specify the form "every other mention becomes a link" takes: a link plus a one-clause summary, never restating the owned attributes. "Becomes a link" alone leaves each of the non-owning sites to invent its own shape.

COMMENT(user-relay): Resolve the observed "unenrolled watcher" vs "monitoring member" drift by deferring to the term in the concepts overview Core terms table, per the user-facing-docs rule. Name that tie-break here so the owning table and every link agree.

COMMENT(user-relay): State the echo rule for non-owning prose: qualitative magnitude words are allowed, exact values are not. This is what lets the watched-set prose keep describing cadence without re-homing the 180s/720s defaults.

COMMENT(user-relay): Add an eighth row to the ownership table. The coding-agents.md "Auto-approval posture" column planned in Step 7 overlaps the coding-agent-backends.md capability matrix's "Shell-command posture" from Step 2 — pick one owner and link from the other.

### Ownership of duplicated enumerations

Seven enumerations currently appear on more than one page. Each gets exactly one owning page carrying the table; every other mention becomes a link. The auditors already observed drift between copies — the enrolled-member set is described as "the unenrolled watcher" on one page and "the monitoring member" on another for the same class.

| Enumeration | Owner | Other pages |
|---|---|---|
| Per-backend `--effort` accepted levels, forwarding form, rejection string | `docs/spec/coding-agent-backends.md` § Reasoning effort | `docs/concepts/coding-agents.md` § Reasoning effort links; its asymmetry matrix carries only supported / not supported |
| Per-backend `--model` format, examples, create-time validation | `docs/spec/coding-agent-backends.md` § Model selection | `docs/concepts/model-selection.md` links |
| The seven `member` subcommands with purpose and identity flag | `docs/spec/cli-options.md` § Subcommand summary | `docs/concepts/overview.md` keeps its names-only group table; `docs/concepts/member-lifecycle.md` § Commands links |
| The four spawn-prompt identity placeholders | `docs/spec/cli-options.md` § member create | `docs/concepts/coding-agents.md` and `docs/concepts/member-lifecycle.md` link |
| Which members are enrolled in the watched set | `docs/concepts/monitoring.md` | Three `webui-api.md` sites and one `data-model.md` site link |
| Monitor default intervals (root Director, ordinary member) | `docs/concepts/monitoring.md` § Cadence and tick precision knob table | The § The watched set prose describes enrollment without restating the numbers |
| The three design-doc workflows with prompt, team, and output | `docs/how-to/design-doc-development.md` § Prompts | `docs/contributing.md` § Contributing changes links and keeps only the contributor-specific delta |

Two overlaps resolve inside a single page. In `docs/concepts/monitoring.md`, the wake-reason table owns the "silenced by" fact and the knob table cross-references it rather than carrying a second disable column. In `docs/spec/coding-agent-backends.md`, the model table and the effort table stay separate — they share only the backend key, and merging them would produce a seven-column table of unrelated contracts.

COMMENT(user-relay): State that the two deliberately empty cells render as an em-dash, matching the convention the output-shape table already uses. "Leave the cell empty" does not say what the reader sees.

COMMENT(user-relay): Record the two facts no page states — the writable_roots rationale and an MCP bypass example — as follow-up documentation gaps, so they are tracked rather than left implicit in two em-dashes.

### Sourcing constraint

Four proposed tables have cells the current pages do not supply. These are filled from the named page, never inferred:

| Table | Missing cells | Source |
|---|---|---|
| `concepts/overview.md` CLI entry points | Scope text for `setup`, `doctor`, `server` | `docs/spec/cli-options.md` § Subcommand summary |
| Identity placeholder table | "Resolves to" for `{fleet_id}`, `{director_member_id}`, `{coding_agent}`; label lines for the latter two | `docs/spec/cli-options.md` § member create |
| Codex prerequisites checklist | The "why" for `writable_roots` | Leave the cell empty — no page states it |
| Safety-floor bypass classes | An example for the MCP class | Leave the cell empty — no page gives one |

COMMENT(user-relay): State how inbound links to a retargeted section are found — a search sweep for references at the time each anchor changes, with the Step 9 strict build as the backstop. "Updates every link to it" does not say how they are located.

COMMENT(user-relay): Add a constraint to check the docs site configuration for section-level nav entries during implementation and update it if any exist. The work adds many new sections and currently assumes nav lists only pages.

### Constraints on every edit

- `docs/` is user-facing: tables state behavior, flags, values, and error strings — not source paths. `docs/contributing.md`, `docs/api/*`, path-as-contract mentions in `docs/spec/*`, and `~/…` user-machine paths remain exempt.
- Error strings, JSON key names, and CLI output shapes are contracts. Moving one into a cell preserves it verbatim.
- Adding a section heading adds an anchor; retargeting a cross-reference to a new section updates every link to it.

---

COMMENT(user-relay): Record the execution decision — the whole document implements on a single branch with a single PR, not one PR per step and not split into several design docs.

COMMENT(user-relay): Add an ordering note at the top of Implementation: tasks that create an owning table land before the tasks that replace other mentions with links to it. Several link-replacement tasks currently precede their owners.

COMMENT(user-relay): State the commit granularity — one commit per Implementation step, nine in total.

COMMENT(user-relay): Annotate every task with the finding ID(s) it implements, so all 68 tasks trace back to the 59-finding audit.

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

COMMENT(user-relay): Add a second task recording the tabulate convention as a project rule under .claude/rules, alongside the contributing.md bullet. Agents author most of these pages and read the rules, not the contributor guide.

COMMENT(user-relay): State that Step 1 is the first commit and gates Steps 2-8, consistent with this project's documentation-first implementation ordering.

### Step 1: Record the convention

- [ ] Add a tables bullet to `docs/contributing.md` § Documentation style stating the tabulate rule, the one-row and paragraph-cell anti-rules, and the single-owner-plus-link rule <!-- completed: -->

COMMENT(user-relay): Reconcile the last task with success criterion 2. A model or effort value may appear in rationale prose, but never as the enumerating statement of the contract — the criterion currently reads as a total ban on per-backend prose mentioning them.

COMMENT(user-relay): State that the rejection strings and validation messages moved into the two new tables are re-verified against the current CLI before landing, and that any drift is reported as a separate finding rather than corrected silently in the same edit.

### Step 2: `docs/spec/coding-agent-backends.md`

- [ ] Add a `## Model selection` section after `## Spawn argv` with columns `Backend | Accepted value format | Example values | Create-time validation`; rows `claude`, `codex`, `opencode`. Reduce the `--model` shared-contract bullet's trailing parenthetical to a pointer <!-- completed: -->
- [ ] Add a `## Reasoning effort` section after it with columns `Backend | Accepted levels | Forwarded as | Rejected with (exit 2)`; rows `claude`, `codex`, `opencode`. Keep the "before any registration or multiplexer side effect" guarantee in the shared-contract bullet <!-- completed: -->
- [ ] Add a per-backend capability matrix under `## Spawn argv` after the existing argv table, with columns `Backend | OS-level sandbox | Sets the pane title | Shell-command posture | Preset / config prerequisite`. Absorb the pane-title bullet; keep the first sentence of the Bash-posture bullet as the shared claim <!-- completed: -->
- [ ] Convert the Codex `~/.codex` prerequisites to a `Setting | Required value | Why` table with rows `network_access`, `writable_roots`, `trust_level`; keep both Quickstart links and the TOML block <!-- completed: -->
- [ ] Convert `### Safety-floor caveats` bullet 2 to a `Bypass class | Example | Why the allowlist misses it` table with three rows; keep the MCP directive, the verification caveat, and the codex-for-isolation escalation as prose <!-- completed: -->
- [ ] Delete the per-backend model and effort prose sentences now carried by the two new tables, leaving each `##` backend section its posture, rationale, and subsections <!-- completed: -->

COMMENT(user-relay): Drop the hard-coded "nine endpoints" from the endpoint-index task. The live page governs — one row per endpoint whatever the count is at implementation time, correcting this document if it differs.

COMMENT(user-relay): State in the message-endpoint comparison task that the new table becomes the single home for the ordering and row-cap facts, and that the per-endpoint restatements are trimmed. As written the table could be read as a summary layered on top of prose that stays.

### Step 3: `docs/spec/webui-api.md`

- [ ] Replace the one-row `## Request Headers` table with prose carrying its three contracts, and add a shared fleet-scoping error table (`Status | detail | Trigger`; rows 400, 404) introduced as applying to every fleet-scoped endpoint <!-- completed: -->
- [ ] Convert the three per-endpoint `**Errors**` blocks to `Status | detail | Trigger` tables carrying only endpoint-specific rows; keep the POST section's cross-reference to Request Headers <!-- completed: -->
- [ ] Add an endpoint index table under `## Endpoints` with columns `Method | Path | Returns | X-Fleet-Id required` and the nine endpoints as rows <!-- completed: -->
- [ ] Convert the `GET /api/monitor` null-behavior paragraph to a `Field | No runtime row has ever existed | Stale or cleared heartbeat row` table with six rows; add no column for the running case <!-- completed: -->
- [ ] Add a message-endpoint comparison table (`Endpoint | Rows returned | Excluded | Ordering | Row cap`) covering inbox, sent, and timeline; keep the shared-formatter sentence, broadcast grouping, and ACK timestamp blocks <!-- completed: -->
- [ ] Replace the enrolled-member prose at the three `webui-api.md` sites with a link to the owning table in `docs/concepts/monitoring.md` <!-- completed: -->
- [ ] Confirm `## Error Format` still explains why 422 rows carry no quoted `detail` string, and that every 422 row points at it <!-- completed: -->

COMMENT(user-relay): The --effort task builds a second table for an enumeration the Ownership section assigns to coding-agent-backends.md § Reasoning effort. Resolve it by merging this task's extra CLI-contract columns into that owning table and pointing at it from here, keeping only the --model format constraint local.

COMMENT(user-relay): Merge the two Error Messages tasks into one redesign adding Command, Exit, and Notes together. As written the same table is restructured twice in sequence.

COMMENT(user-relay): The output-shape task says "all 19 subcommands" while the summary table has 22 rows. Verify both counts against the live page and record why they legitimately differ, or reconcile them.

COMMENT(user-relay): Qualify the paragraph-deletion task: delete only the sentences the new output-shape table carries, and keep any sentence it does not. Whole-paragraph deletion would drop uncarried facts.

COMMENT(user-relay): Verify that --quiet actually exists on message send and message ack before documenting those rows, and halt if it does not. The task assumes a docs gap; if the flag is absent, adding the rows would document behavior that does not exist.

### Step 4: `docs/spec/cli-options.md`

- [ ] Replace the `--effort` cell's semicolon list in the `member create` flag table with a pointer to a per-backend table carrying `Backend | Accepted --effort levels | Rejection error (exit 2) | --model format constraint` <!-- completed: -->
- [ ] Add `Command` and `Exit` columns to `## Error Messages`, moving the exit code out of every message cell; keep each message string verbatim <!-- completed: -->
- [ ] Add a `Notes` column to `## Error Messages` and move the trailing explanatory clauses out of the message column into it <!-- completed: -->
- [ ] Convert the two inline exit-code sentences (member targeting, `monitor start`) to `Exit | Meaning` tables, one row per trigger rather than one row per code <!-- completed: -->
- [ ] Replace the three numbered resolution rules in `### Member targeting and key delivery` with a `Target state | capture / prompt / ping | show | delete` matrix over four target states <!-- completed: -->
- [ ] Add a cross-subcommand output-shape table (`Subcommand | Default text output | --full text output | JSON payload`) covering all 19 subcommands, with an em-dash where `--full` does not apply <!-- completed: -->
- [ ] Fold the `### --full semantics` table into the output-shape table, including the missing `fleet create` row, and remove the standalone table <!-- completed: -->
- [ ] Delete the per-subcommand trailing output paragraphs now carried by the output-shape table <!-- completed: -->
- [ ] Convert the two `member list` output paragraphs to a `Field | Text column | Text rendering when absent | JSON key | JSON type` table over ten fields <!-- completed: -->
- [ ] Add a consolidated environment-variable table (`Environment variable | Settings field | Default | Controls | Overridden by`) with five rows, and remove the one-row `CAFLEET_MAX_TEXT_LEN` table it absorbs <!-- completed: -->
- [ ] Add a `--quiet` semantics table (`Subcommand | Quiet output`) and add the missing `--quiet` rows to the `message send` and `message ack` flag tables <!-- completed: -->
- [ ] Convert the `cafleet setup` db half to a `Prior DB state | Outcome | Output / refusal message` table with five rows, keeping every string verbatim <!-- completed: -->
- [ ] Convert the `cafleet setup` failure paragraphs to a `Half | Trigger | Message | Effect` table <!-- completed: -->
- [ ] Convert `#### Spawn-prompt substitution` to a `Placeholder | Substituted value | How the spawned member sees it` table over the four placeholders — this table is the site-wide owner <!-- completed: -->
- [ ] Replace the `## Stale-assets guard` numbered list with a `Recorded install state | Result | Exit` table plus an `Exempt surface | Why exempt | Behavior under a stale/missing install` table, supplying the missing reason for `server` <!-- completed: -->
- [ ] Re-key the `## JSON output (--json)` table one row per subcommand so it can be scanned by name; leave the 22-row subcommand summary at its current width <!-- completed: -->
- [ ] Replace the one-row `## Global Options` table with a sentence covering `--version` <!-- completed: -->
- [ ] Trim the `### member prompt` form-comparison table cells to their keystroke sequences and move both rationale sentences to prose beneath it <!-- completed: -->

COMMENT(user-relay): State that the summary matrix's primary-key and foreign-key facts are cross-checked against the SPEC.md DDL, with any mismatch reported rather than silently resolved in favour of one source.

COMMENT(user-relay): Define "treat the --full label list as closed" — use only labels the page already documents, and em-dash any field that has none.

### Step 5: `docs/spec/data-model.md` and `docs/spec/message-envelope.md`

- [ ] Add a summary matrix under `## Tables` with columns `Table | Primary key | Parent | FK ON DELETE | Row removal` over the seven tables; shrink the key-style paragraph above it to the two facts the table cannot carry <!-- completed: -->
- [ ] Replace the enrolled-member prose in `### monitor_config and monitor_runtime` with a link to the owning table in `docs/concepts/monitoring.md` <!-- completed: -->
- [ ] Widen the `### Compact rendered envelope (default)` field table to `Field | Compact text mode | Compact JSON key | --full text label | --full JSON key` over the existing ten rows in order; treat the `--full` label list as closed <!-- completed: -->
- [ ] Convert `## Flag cross-reference` to a `Control | Default | Effect on the envelope` table over `--json`, `--full`, and `CAFLEET_MAX_TEXT_LEN`, keeping each control's link <!-- completed: -->

COMMENT(user-relay): The selection truth table covers six of the eight combinations of three variables. Require a one-line note explaining why the other two are absent, so the gap reads as deliberate rather than as an oversight.

### Step 6: `docs/spec/multiplexer-backends.md`

- [ ] Add a `## Backend matrix` section after the intro with columns `Behavior | tmux | herdr` and the eight behavioral deltas as rows <!-- completed: -->
- [ ] Reduce the five per-section tmux/herdr bullet pairs to their rationale and exception prose, leaving the matrix as the comparison surface <!-- completed: -->
- [ ] Convert `### The Esc safeguard` to a `Keystroke path | Leads with Esc? | Payload | Why` table over the five paths; keep the shared press-settle-type mechanism as prose above it <!-- completed: -->
- [ ] Add a `CAFLEET_MULTIPLEXER | HERDR_ENV | TMUX | Result` truth table to `## Backend selection` covering six combinations; keep the three numbered precedence items <!-- completed: -->
- [ ] Convert the second paragraph of `## Native agent-state (herdr only)` to a `Wake trigger | tmux | herdr` table with four rows; add no rows for undocumented states, and keep the `blocked` rule as prose <!-- completed: -->

COMMENT(user-relay): The rubric's working row currently disagrees between table and prose, so neither is a safe source for the new fourth column. Verify the behavior against the monitor role definition and implementation before writing, rather than copying either existing text.

COMMENT(user-relay): State that the five member classes in the enrolled-member table are re-derived from actual enrollment behavior and any drift against the page's existing prose is reconciled — this page becomes the site-wide owner, so a wrong row key propagates to every linking page.

COMMENT(user-relay): Resolve the member-lifecycle § Commands either/or: a slim table keyed by lifecycle stage, not restating purpose or flags, plus the link to the owning summary in cli-options.

### Step 7: `docs/concepts/`

- [ ] `monitoring.md`: add a wake-reason table (`Reason | Fires when | Advances last_ping_at? | Availability | Silenced by`) over `interval`, `unacked`, `stall-check`, `status:done` <!-- completed: -->
- [ ] `monitoring.md`: add a fourth column to the five-state rubric table giving each state its behavior when the member is tagged `unacked`, so the table and the prose agree for `working` <!-- completed: -->
- [ ] `monitoring.md`: cross-reference the wake-reason table's "silenced by" column from the knob table instead of adding a second disable column; move the stall row's `(0 disables)` out of its "Set by" cell <!-- completed: -->
- [ ] `monitoring.md`: remove the duplicated `180s` / `720s` defaults from the § The watched set prose, leaving the knob table as their single home <!-- completed: -->
- [ ] `monitoring.md`: add the enrolled-member table (`Member class | Enrolled | monitor field on GET /api/members`) with five rows — this page is the site-wide owner <!-- completed: -->
- [ ] `coding-agents.md`: replace § Reasoning effort's running sentence with a link to the owning table in `docs/spec/coding-agent-backends.md`, keeping the validation and default prose <!-- completed: -->
- [ ] `coding-agents.md`: convert § Known asymmetries to a `Dimension | claude | codex | opencode` matrix over reasoning effort, pane title, and sandbox isolation, carrying supported / not-supported rather than restating the level sets; absorb the second opencode-unsupported mention <!-- completed: -->
- [ ] `coding-agents.md`: add a `Backend | Product | Auto-approval posture | How the pane loads the cafleet skill` table collecting the per-backend facts currently spread across the intro and § cafleet usage from a member pane <!-- completed: -->
- [ ] `coding-agents.md`: replace the inline placeholder enumeration with a link to the owning table in `docs/spec/cli-options.md` <!-- completed: -->
- [ ] `model-selection.md`: convert § The model list's column description to a `Column | What it holds | How to read it` table over five rows, moving the judgment and estimate caveats into the rows they qualify <!-- completed: -->
- [ ] `model-selection.md`: convert § Cost efficiency mode's role policies to a `Role | Model chosen | Needs the cost efficiency mode trigger?` table over ordinary member, monitor, and reviewer <!-- completed: -->
- [ ] `model-selection.md`: convert the closing override paragraph to a `Situation | What the Director does` table over the five conditions, keeping the fail-closed framing sentence <!-- completed: -->
- [ ] `member-lifecycle.md`: convert § Delete ordering to a `Member state | What member delete does | Multiplexer effect` table over the three states <!-- completed: -->
- [ ] `member-lifecycle.md`: replace § Commands' prose enumeration with a slim lifecycle-role table or a link to the owning summary in `docs/spec/cli-options.md`; keep the identity-flag rule and the `member prompt` paragraph <!-- completed: -->
- [ ] `member-lifecycle.md`: replace the inline placeholder enumeration with a link to the owning table <!-- completed: -->
- [ ] `storage.md`: convert § Schema management to a `Database state | What cafleet setup does` table over four states, keeping the no-schema failure outside the table <!-- completed: -->
- [ ] `overview.md`: extend the CLI group table to all seven entry points, sourcing the scope text for `setup`, `doctor`, and `server` from the CLI options spec <!-- completed: -->
- [ ] `overview.md`: trim the `monitor` glossary cell to a one-clause definition matching its neighbours, leaving the mechanism to the linked Monitoring page <!-- completed: -->

COMMENT(user-relay): The overview.md CLI entry-point task sources scope text from cli-options, creating a second home for it. Paraphrase to the concepts altitude instead of copying, so the two pages differ in wording and only the spec page reads as the contract.

COMMENT(user-relay): Drop the hard-coded "seven pick-one tasks" from the contributing.md Development task. Verify against the live mise task set and write one row per task, whatever the count is then.

COMMENT(user-relay): In the quickstart Configure conversion, place the two kept code snippets below the new table, each introduced by the backend it belongs to.

### Step 8: Entry points and how-to

- [ ] `quickstart.md`: convert § Configure's three per-backend sub-headings to a `Backend | Config file | Manual configuration | Installed by cafleet setup | Reference` table; keep both code snippets and both rationale paragraphs <!-- completed: -->
- [ ] `quickstart.md`: convert the prerequisites bullets to a `Requirement | Accepted | Notes` table over three rows <!-- completed: -->
- [ ] `contributing.md`: convert § Tech stack's bold-key bullets to a `Concern | Technology | Notes` table over six rows, keeping every external link <!-- completed: -->
- [ ] `contributing.md`: replace § Contributing changes' three-item list with a link to the owning workflows table in `docs/how-to/design-doc-development.md`, keeping the ordering sentence and the contributor-specific delta <!-- completed: -->
- [ ] `contributing.md`: split § Development's code block into a first-time setup sequence that stays a code block and a `Task | Runs | When you need it` table over the seven pick-one tasks <!-- completed: -->
- [ ] `design-doc-development.md`: convert § Prompts to a `Stage | Prompt | Workflow | Team` table over the three stages, keeping each prompt in a code span and the stage numbering <!-- completed: -->

COMMENT(user-relay): Add a task fixing any one-row table found anywhere on the site, not only in tables this work touched — the one-row rule is sitewide.

COMMENT(user-relay): Add a per-deletion fact-parity check: for every paragraph Steps 2, 4, and 6 delete, confirm each fact it carried survives in the replacing table or in kept prose.

COMMENT(user-relay): State that Step 9 is executed twice — the implementer runs the checks and records results, then the reviewer independently confirms them.

COMMENT(user-relay): Record a follow-up design doc for automated enforcement of the one-row-table and two-sentence-cell rules. This change verifies them manually; tooling is deliberately out of scope here.

### Step 9: Verify

- [ ] Confirm each of the seven owned enumerations appears as a table on exactly one page and every other mention is a link <!-- completed: -->
- [ ] Confirm no table has exactly one data row, no cell exceeds two sentences, and no repository path was introduced outside the exempt surfaces <!-- completed: -->
- [ ] Build the docs site and confirm every anchor added or retargeted by this work resolves <!-- completed: -->

---

COMMENT(user-relay): Remove this Changelog section entirely, including its heading, table, and the preceding horizontal rule. The user does not want a changelog on this document; the standard design-doc template marks the section optional, so its absence is compliant.

## Changelog

| Date | Changes |
|------|---------|
| 2026-07-25 | Initial draft from a four-auditor pass over all 20 docs pages |
