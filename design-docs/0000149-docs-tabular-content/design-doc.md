# Tabular content across the documentation site

**Status**: Approved
**Progress**: 70/70 tasks complete
**Last Updated**: 2026-07-25

## Overview

The docs site states parallel, multi-attribute content as prose, bullets, and repeated per-item sections, forcing readers to reconstruct comparison matrices in their heads. This document converts that content to tables on every page with qualifying content, fixes five places where a table is misused, and assigns a single owning page to every enumeration that is currently duplicated.

## Success Criteria

- [x] Every enumeration of three or more parallel items carrying two or more shared attributes renders as a table, on every page with qualifying content. The exempt surfaces are `docs/concepts/fleet-isolation.md`, the four `docs/api/*` mkdocstrings stubs, and `docs/index.md`.
- [x] `docs/spec/coding-agent-backends.md` answers "which models and effort levels does backend X accept, and what does a wrong value produce" from one row per backend. A model or effort value may still appear in a backend section's rationale prose; what no per-backend section keeps is the enumerating statement of the accepted set.
- [x] `docs/spec/webui-api.md` answers "what does status N mean on endpoint E" from one row, with the two fleet-scoping errors stated once rather than three times.
- [x] The five-state rubric table in `docs/concepts/monitoring.md` gives the same answer as the prose beneath it for all five states, including `working`.
- [x] Each of the eight duplicated enumerations in the Ownership table below appears as a table on exactly one page; every other mention is a link plus a one-clause summary.
- [x] No table on the site has exactly one data row, and no table cell holds more than two sentences. Both rules bind sitewide, including tables this work never otherwise touches. A verbatim quoted contract string — an error message, an output line — counts as one unit toward the cell cap regardless of its internal sentence count.
- [x] No repository file path is introduced into any page outside the surfaces exempted by the user-facing-docs rule.
- [x] Every fact carried by a paragraph that Step 2, 4, or 6 deletes survives in the replacing table or in kept prose.
- [x] The project's strict docs build passes, and every intra-site anchor added or retargeted by this work resolves.

---

## Background

Four auditors read all 20 pages in full and produced 59 findings: 54 tabulate-me candidates and 5 table-misuse candidates. The audit records are `findings-cli-options.md`, `findings-spec-rest.md`, `findings-concepts.md`, and `findings-entry-howto-api.md` in this directory; each finding carries its exact location, proposed columns, row keys, and the prose that must survive the change. These four files are committed alongside this document and are **normative during implementation**: where a task names a finding id, the finding's proposed columns, row keys, and prose-that-must-remain govern, and this document states only the delta.

Their normativity has one bound: an audit finding records what a page *said* at audit time, not what the system *does*. Where a proposed cell conflicts with a contract source — the `SPEC.md` DDL, an error string in the CLI, a role definition — the contract source wins and the finding is treated as incomplete. A conflict **between two contract sources** is different: that is reported, never resolved in favour of one.

Finding ids are namespaced per audit file: `CLI-*` (`findings-cli-options.md`), `SPEC-*` (`findings-spec-rest.md`), `CON-*` (`findings-concepts.md`), `EHA-*` (`findings-entry-howto-api.md`).

Three patterns recur across the site:

| Pattern | Where it dominates | Shape |
|---|---|---|
| Repeated per-item paragraph | `coding-agent-backends.md`, `multiplexer-backends.md`, `coding-agents.md`, `design-doc-development.md` | N parallel subjects each get a prose section restating the same attribute in a different sentence shape |
| Attribute present on every row but never a column | `cli-options.md` Error Messages, `monitoring.md` rubric and knob tables, `message-envelope.md` field table | The table exists; the attribute sits in a parenthetical inside another cell, or in the paragraph below |
| Enumeration compressed into one cell or one clause | `cli-options.md` `--effort` row, `model-selection.md` fail-closed paragraph, `multiplexer-backends.md` Esc safeguard | A comparison set is written as a semicolon list inside a table cell or mid-sentence |

### The five table-misuse fixes

The Overview counts five; these are they, each mapped to the task that lands it.

| Finding | Misuse | Fix lands at |
|---|---|---|
| `CLI-M1` | One-row tables at `## Global Options` and `## Message Body Truncation` | Step 4 — the `## Global Options` sentence task and the consolidated environment-variable table task |
| `CLI-M2` | `member prompt` form-comparison table whose cells hold multi-sentence rationale | Step 4 — the `### member prompt` trim task |
| `CLI-M3` | `## Error Messages` cells mixing contract strings with rationale prose | Step 4 — the `Command` / `Exit` / `Notes` redesign task |
| `SPEC-M1` | `webui-api.md` `## Request Headers` one-row table with a three-sentence cell | Step 3 — the Request Headers task |
| `CON-M1` | `overview.md` `## Core terms` `monitor` row whose definition cell is a paragraph | Step 7 — the glossary-trim task |

### Surfaces with nothing to tabulate

`docs/concepts/fleet-isolation.md` and the four `docs/api/*` mkdocstrings stubs have nothing enumerable. `docs/index.md` is a landing page whose value is narrative. These surfaces are exempt from criterion 1.

### Deliberately dropped findings

Three audited findings are **not** implemented, because tabulating them would not help a reader. The user reviewed all three during the interview and ratified the drops; they are a confirmed decision, not unreviewed author judgment.

| Dropped finding | Reason |
|---|---|
| `SPEC-F18` — `multiplexer-backends.md` § Design principles as a table | Four bold-lead bullets already scan on the same axis; the auditor recommended against it |
| `CON-F8` — `model-selection.md` § Replacement bounds as a table | Three one-line rules; a bulleted list reads better, and the section is already clear once the two other tables on that page land |
| `EHA-F7` — `mixed-backend-team.md` appendix walkthrough as a table | Trimming a copy-pasteable walkthrough costs the reader more than the redundancy does |

`CLI-F14`'s proposal to merge the `--json` availability table into the 22-row subcommand summary is also reduced: a seven-column summary table is harder to scan than the six-column one it replaces, so only the minimum fix applies (re-key the `--json` table one row per subcommand).

---

## Specification

### The tabulate rule

Content becomes a table when **three or more parallel items each carry two or more of the same attributes**. Content stays prose when it is a single item, an ordered procedure, or rationale for one decision.

The counted parallel items may sit on **either axis** — as rows or as columns. A two-subject comparison such as tmux vs herdr clears the bar when the counted items are the behaviors compared (rows), with the two subjects as columns; it does not clear the bar on the strength of the two subjects alone.

| Content | Form |
|---|---|
| Three or more parallel items, two or more shared attributes | Table |
| Ordered steps a reader performs in sequence | Numbered list |
| Constraints or requirements with no shared attribute axis | Bulleted list |
| One item, however many attributes | Prose |
| Rationale, caveats, and "why" for a decision | Prose, adjacent to the table it qualifies |

**Rule lists: which convert and which stay.** A rule list becomes a table when the reader uses it for **lookup or decision** — they arrive knowing one key and want the matching behavior, and the rules' order is incidental. A rule list stays a numbered list when the ordering is **genuinely load-bearing** — an earlier rule preempts a later one, so renumbering would change the outcome. This is why Step 4 tabulates the member-targeting resolution rules (four target states looked up by key; the numbering implies a sequence that does not exist) while Step 6 keeps backend-selection's three precedence items as a numbered list (an override genuinely preempts auto-detect) and adds a truth table showing that order's *outcomes* alongside it.

Two anti-rules follow from the same principle, and both are enforced by this document's success criteria: a table with exactly one data row costs a header row and buys nothing over a sentence; a cell holding more than two sentences defeats the vertical scan the table shape promises. When a finding moves an enumeration into a table, the surrounding rationale prose stays — a table replaces the enumeration, never the contract prose around it.

### Table-rendering conventions

Roughly forty new tables land across twenty pages. These conventions keep them consistent:

| Element | Convention |
|---|---|
| Literals — flags, values, env vars, error strings, field and column names | Code span |
| A cell with no applicable value | Em-dash (`—`), never an empty cell or "N/A" |
| Column alignment | Default left alignment; no alignment colons unless an existing adjacent table already uses them |
| Header wording | Noun phrase, sentence case, no trailing punctuation |
| Boolean-ish columns | `yes` / `no`, not `✓` / `✗` |
| A literal `\|` inside a cell | A raw `<code>` element using `&#124;` (plus `&lt;` / `&gt;` where the string also carries angle brackets). A `\|` escape is **not** a fix — Python-Markdown leaves the backslash visible inside a code span |

The pipe rule is verified by inspecting rendered HTML, not by the docs build: the build renders a broken cell without error either way, so a green build is not evidence.

### Ownership of duplicated enumerations

Eight enumerations currently appear on more than one page. Each gets exactly one owning page carrying the table; every other mention becomes a **link plus a one-clause summary** — enough for the reader to know whether to follow it, never a restatement of the owned attributes.

**Term tie-break.** The auditors observed drift between copies — the enrolled-member set is described as "the unenrolled watcher" on one page and "the monitoring member" on another for the same class. Per the user-facing-docs rule, the term used in the Core terms table of the concepts overview governs; the owning table and every linking mention adopt it.

**Echo rule for non-owning prose.** A non-owning page may describe an owned enumeration in **qualitative magnitude** terms ("the Director is checked far more often than an ordinary member"); it may not restate **exact values**. This is what lets the watched-set prose keep describing cadence without re-homing the `180s` / `720s` defaults.

| Enumeration | Owner | Other pages |
|---|---|---|
| Per-backend `--effort` accepted levels, forwarding form, rejection string | `docs/spec/coding-agent-backends.md` § Reasoning effort | `docs/concepts/coding-agents.md` § Reasoning effort links; its asymmetry matrix carries only supported / not supported |
| Per-backend `--model` format, examples, create-time validation | `docs/spec/coding-agent-backends.md` § Model selection | `docs/concepts/model-selection.md` links |
| Per-backend shell-command / auto-approval posture | `docs/spec/coding-agent-backends.md` § Spawn argv capability matrix | `docs/concepts/coding-agents.md` links; its own per-backend table carries product and skill-loading only |
| The seven `member` subcommands with purpose and identity flag | `docs/spec/cli-options.md` § Subcommand summary | `docs/concepts/overview.md` keeps its names-only group table; `docs/concepts/member-lifecycle.md` § Commands links |
| The four spawn-prompt identity placeholders | `docs/spec/cli-options.md` § member create | `docs/concepts/coding-agents.md` and `docs/concepts/member-lifecycle.md` link |
| Which members are enrolled in the watched set | `docs/concepts/monitoring.md` | Three `webui-api.md` sites and one `data-model.md` site link |
| Monitor default intervals (root Director, ordinary member) | `docs/concepts/monitoring.md` § Cadence and tick precision knob table | The § The watched set prose describes enrollment without restating the numbers |
| The three design-doc workflows with prompt, team, and output | `docs/how-to/design-doc-development.md` § Prompts | `docs/contributing.md` § Contributing changes links and keeps only the contributor-specific delta |

Two overlaps resolve inside a single page. In `docs/concepts/monitoring.md`, the wake-reason table owns the "silenced by" fact and the knob table cross-references it rather than carrying a second disable column. In `docs/spec/coding-agent-backends.md`, the model table and the effort table stay separate — they share only the backend key, and merging them would produce a seven-column table of unrelated contracts.

### Sourcing constraint

Four proposed tables have cells the current pages do not supply. These are filled from the named page, never inferred. The two cells no page supplies render as an **em-dash**, matching the convention above.

| Table | Missing cells | Source |
|---|---|---|
| `concepts/overview.md` CLI entry points | Scope text for `setup`, `doctor`, `server` | `docs/spec/cli-options.md` § Subcommand summary, paraphrased to the concepts altitude |
| Identity placeholder table | "Resolves to" for `{fleet_id}`, `{director_member_id}`, `{coding_agent}`; label lines for the latter two | The canonical spawn-prompt skeleton, which renders all four literally. `docs/spec/cli-options.md` § member create names the four placeholders but carries no label lines, so it cannot source this table |
| Codex prerequisites checklist | The "why" for `writable_roots` | Em-dash — no page states it |
| Safety-floor bypass classes | An example for the MCP class | Em-dash — no page gives one |

**Follow-up documentation gaps.** The two em-dashes above are unstated facts, not stylistic choices. Both are recorded here as tracked gaps for a later change: the rationale for the `writable_roots` codex requirement, and a concrete example of the MCP bypass class. Neither is invented in this work.

### Constraints on every edit

- `docs/` is user-facing: tables state behavior, flags, values, and error strings — not source paths. `docs/contributing.md`, `docs/api/*`, path-as-contract mentions in `docs/spec/*`, and `~/…` user-machine paths remain exempt.
- Error strings, JSON key names, and CLI output shapes are contracts. Moving one into a cell preserves it verbatim.
- Adding a section heading adds an anchor. When a cross-reference is retargeted to a new section, inbound links are located by a **search sweep for references to the old anchor at the time that anchor changes**, with the Step 9 strict docs build as the backstop that catches anything the sweep missed.
- The docs site configuration is checked for **section-level nav entries** during implementation, and updated if any exist. This work adds many new sections; the current assumption is that nav lists only pages, and that assumption is verified rather than trusted.

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

**Execution shape.** The whole document implements on a **single branch with a single PR** — not one PR per step, and not split into several design docs.

**Commit granularity.** One commit per Implementation step, **nine in total**. A link-replacement task the ordering rule below defers rides in the commit of the step that supplies its owning table, not in its own step's commit — so a step may commit with that one task still unchecked, and the step carrying the owning table commits one task more than it lists.

**Step execution order.** Steps run `1, 2, 3, 7, 4, 5, 6, 8, 9`. Step 7 is pulled ahead of Steps 4–6 because it creates the `monitoring.md` owning table that Step 3's and Step 5's link-replacement tasks both point at; commit messages still name the step, so the nine commits stay one-per-step in a non-numeric order.

**Ordering.** Step 1 is the first commit and gates Steps 2–8, consistent with this project's documentation-first implementation ordering. Beyond that, a task that **creates an owning table lands before any task that replaces another mention with a link to it**, even when the linking task carries a lower step number. Concretely: Step 7's `monitoring.md` enrolled-member table and its knob table land before Step 3's and Step 5's link-replacement tasks; Step 2's model and effort tables land before Step 7's `coding-agents.md` link task; Step 4's placeholder table lands before Step 7's two placeholder link tasks.

Every task names the finding id(s) it implements, so all 70 tasks trace back to the 59-finding audit. Step 1 and Step 9 tasks implement no finding and are marked accordingly.

### Step 1: Record the convention

- [x] Add a tables bullet to `docs/contributing.md` § Documentation style stating the tabulate rule, the one-row and paragraph-cell anti-rules, and the single-owner-plus-link rule *(no finding — convention)* <!-- completed: 2026-07-25T13:25 -->
- [x] Record the same tabulate convention as a project rule under `.claude/rules`, covering the tabulate rule, both anti-rules, the single-owner-plus-link rule, and the table-rendering conventions — agents author most of these pages and read the rules, not the contributor guide *(no finding — convention)* <!-- completed: 2026-07-25T13:30 -->

### Step 2: `docs/spec/coding-agent-backends.md`

- [x] Add a `## Model selection` section after `## Spawn argv` with columns `Backend | Accepted value format | Example values | Create-time validation`; rows `claude`, `codex`, `opencode`. Reduce the `--model` shared-contract bullet's trailing parenthetical to a pointer *(SPEC-F6)* <!-- completed: 2026-07-25T13:33 -->
- [x] Add a `## Reasoning effort` section after it with columns `Backend | Accepted levels | Forwarded as | Rejected with (exit 2)`; rows `claude`, `codex`, `opencode`. Keep the "before any registration or multiplexer side effect" guarantee in the shared-contract bullet *(SPEC-F7)* <!-- completed: 2026-07-25T13:33 -->
- [x] Add a per-backend capability matrix under `## Spawn argv` after the existing argv table, with columns `Backend | OS-level sandbox | Sets the pane title | Shell-command posture | Preset / config prerequisite`. Absorb the pane-title bullet; keep the first sentence of the Bash-posture bullet as the shared claim. This matrix is the site-wide owner of the shell-command / auto-approval posture *(SPEC-F8)* <!-- completed: 2026-07-25T13:33 -->
- [x] Convert the Codex `~/.codex` prerequisites to a `Setting | Required value | Why` table with rows `network_access`, `writable_roots`, `trust_level`; em-dash the `writable_roots` why-cell; keep both Quickstart links and the TOML block *(SPEC-F9)* <!-- completed: 2026-07-25T13:33 -->
- [x] Convert `### Safety-floor caveats` bullet 2 to a `Bypass class | Example | Why the allowlist misses it` table with three rows; em-dash the MCP example cell; keep the MCP directive, the verification caveat, and the codex-for-isolation escalation as prose *(SPEC-F10)* <!-- completed: 2026-07-25T13:33 -->
- [x] Delete the per-backend model and effort prose sentences now carried by the two new tables, leaving each `##` backend section its posture, rationale, and subsections. A model or effort value may remain where it is rationale rather than the enumerating statement of the contract *(SPEC-F6, SPEC-F7)* <!-- completed: 2026-07-25T13:33 -->

**Verification constraint for this step.** The rejection strings and validation messages moved into the two new tables are re-verified against the current CLI before landing. Any drift between the page and the CLI is reported as a **separate finding**, not corrected silently in the same edit.

### Step 3: `docs/spec/webui-api.md`

- [x] Replace the one-row `## Request Headers` table with prose carrying its three contracts, and add a shared fleet-scoping error table (`Status | detail | Trigger`; rows 400, 404) introduced as applying to every fleet-scoped endpoint *(SPEC-M1, SPEC-F1)* <!-- completed: 2026-07-25T13:37 -->
- [x] Convert each per-endpoint `**Errors**` block to a `Status | detail | Trigger` table carrying only endpoint-specific rows; keep the POST section's cross-reference to Request Headers. A block left with a single endpoint-specific row after the shared rows hoist out stays prose — the one-row anti-rule outranks the conversion *(SPEC-F1)* <!-- completed: 2026-07-25T13:37 -->
- [x] Add an endpoint index table under `## Endpoints` with columns `Method | Path | Returns | X-Fleet-Id required`, one row per endpoint the live page documents — the page governs the count, and this document is corrected if it differs *(SPEC-F2)* <!-- completed: 2026-07-25T13:37 -->
- [x] Convert the `GET /api/monitor` null-behavior paragraph to a `Field | No runtime row has ever existed | Stale or cleared heartbeat row` table with six rows; add no column for the running case *(SPEC-F3)* <!-- completed: 2026-07-25T13:37 -->
- [x] Add a message-endpoint comparison table (`Endpoint | Rows returned | Excluded | Ordering | Row cap`) covering inbox, sent, and timeline. This table becomes the **single home** for the ordering and row-cap facts; the per-endpoint restatements of those two facts are trimmed. Keep the shared-formatter sentence, broadcast grouping, and ACK timestamp blocks *(SPEC-F4)* <!-- completed: 2026-07-25T13:37 -->
- [x] Replace the enrolled-member prose at the three `webui-api.md` sites with a link plus one-clause summary pointing at the owning table in `docs/concepts/monitoring.md` *(SPEC-F5)* <!-- completed: 2026-07-25T13:46 -->
- [x] Confirm `## Error Format` still explains why 422 rows carry no quoted `detail` string, and that every 422 row points at it *(SPEC-F1)* <!-- completed: 2026-07-25T13:37 -->

### Step 4: `docs/spec/cli-options.md`

- [x] Replace the `--effort` cell's semicolon list in the `member create` flag table with a pointer to the owning table in `docs/spec/coding-agent-backends.md` § Reasoning effort, merging this page's extra CLI-contract columns into that owning table; keep only the `--model` format constraint local *(CLI-F1)* <!-- completed: 2026-07-25T13:58 -->
- [x] Redesign `## Error Messages` in one pass: add `Command`, `Exit`, and `Notes` columns together, moving the exit code out of every message cell and the trailing explanatory clauses out of the message column, leaving `Error message` holding only the verbatim string *(CLI-F2, CLI-M3)* <!-- completed: 2026-07-25T13:58 -->
- [x] Convert the two inline exit-code sentences (member targeting, `monitor start`) to `Exit | Meaning` tables, one row per trigger rather than one row per code *(CLI-F3)* <!-- completed: 2026-07-25T13:58 -->
- [x] Replace the three numbered resolution rules in `### Member targeting and key delivery` with a `Target state | capture / prompt / ping | show | delete` matrix over four target states *(CLI-F4)* <!-- completed: 2026-07-25T13:58 -->
- [x] Add a cross-subcommand output-shape table (`Subcommand | Default text output | --full text output | JSON payload`), with an em-dash where `--full` does not apply. Verify the subcommand count against the live page and against the `## Subcommand summary` row count; if the two legitimately differ, record why in a one-line note, otherwise reconcile them *(CLI-F5)* <!-- completed: 2026-07-25T13:58 -->
- [x] Fold the `### --full semantics` table into the output-shape table, including the missing `fleet create` row, and remove the standalone table *(CLI-F8)* <!-- completed: 2026-07-25T13:58 -->
- [x] Delete from each per-subcommand trailing output paragraph only the sentences the new output-shape table carries; keep every sentence it does not *(CLI-F5)* <!-- completed: 2026-07-25T13:58 -->
- [x] Convert the two `member list` output paragraphs to a `Field | Text column | Text rendering when absent | JSON key | JSON type` table over ten fields *(CLI-F6)* <!-- completed: 2026-07-25T13:58 -->
- [x] Add a consolidated environment-variable table (`Environment variable | Settings field | Default | Controls | Overridden by`) with five rows, and remove the one-row `CAFLEET_MAX_TEXT_LEN` table it absorbs *(CLI-F7, CLI-M1)* <!-- completed: 2026-07-25T13:58 -->
- [x] Verify `--quiet` exists on `message send` and `message ack` against the current CLI. If it does, add a `--quiet` semantics table (`Subcommand | Quiet output`) and the missing `--quiet` rows to both flag tables. If it does not, **halt this task and report** rather than documenting behavior that does not exist *(CLI-F9)* <!-- completed: 2026-07-25T13:58 -->
- [x] Convert the `cafleet setup` db half to a `Prior DB state | Outcome | Output / refusal message` table with five rows, keeping every string verbatim *(CLI-F10)* <!-- completed: 2026-07-25T13:58 -->
- [x] Convert the `cafleet setup` failure paragraphs to a `Half | Trigger | Message | Effect` table *(CLI-F11)* <!-- completed: 2026-07-25T13:58 -->
- [x] Convert `#### Spawn-prompt substitution` to a `Placeholder | Substituted value | How the spawned member sees it` table over the four placeholders — this table is the site-wide owner *(CLI-F12, CON-F14)* <!-- completed: 2026-07-25T13:58 -->
- [x] Replace the `## Stale-assets guard` numbered list with a `Recorded install state | Result | Exit` table plus an `Exempt surface | Why exempt | Behavior under a stale/missing install` table, supplying the missing reason for `server` *(CLI-F13)* <!-- completed: 2026-07-25T13:58 -->
- [x] Re-key the `## JSON output (--json)` table one row per subcommand so it can be scanned by name; leave the 22-row subcommand summary at its current width *(CLI-F14)* <!-- completed: 2026-07-25T13:58 -->
- [x] Replace the one-row `## Global Options` table with a sentence covering `--version` *(CLI-M1)* <!-- completed: 2026-07-25T13:58 -->
- [x] Trim the `### member prompt` form-comparison table cells to their keystroke sequences and move both rationale sentences to prose beneath it *(CLI-M2)* <!-- completed: 2026-07-25T13:58 -->

### Step 5: `docs/spec/data-model.md` and `docs/spec/message-envelope.md`

- [x] Add a summary matrix under `## Tables` with columns `Table | Primary key | Parent | FK ON DELETE | Row removal` over the seven tables; shrink the key-style paragraph above it to the two facts the table cannot carry. Cross-check every primary-key and foreign-key cell against the `SPEC.md` DDL; report any mismatch rather than silently resolving it in favour of one source *(SPEC-F11)* <!-- completed: 2026-07-25T14:03 -->
- [x] Replace the enrolled-member prose in `### monitor_config and monitor_runtime` with a link plus one-clause summary pointing at the owning table in `docs/concepts/monitoring.md` *(SPEC-F5)* <!-- completed: 2026-07-25T14:03 -->
- [x] Widen the `### Compact rendered envelope (default)` field table to `Field | Compact text mode | Compact JSON key | --full text label | --full JSON key` over the existing ten rows in order. Treat the `--full` label list as **closed**: use only the labels § Text mode already documents (`id`, `state`, `from`, `to`, `type`, `text`), and em-dash any field that has none *(SPEC-F12)* <!-- completed: 2026-07-25T14:03 -->
- [x] Convert `## Flag cross-reference` to a `Control | Default | Effect on the envelope` table over `--json`, `--full`, and `CAFLEET_MAX_TEXT_LEN`, keeping each control's link *(SPEC-F13)* <!-- completed: 2026-07-25T14:03 -->

### Step 6: `docs/spec/multiplexer-backends.md`

- [x] Add a `## Backend matrix` section after the intro with columns `Behavior | tmux | herdr`, one row per behavioral delta in SPEC-F14's row-key list — nine, keeping the two `send_prompt` forms separate since their payloads differ on both backends *(SPEC-F14)* <!-- completed: 2026-07-25T14:34 -->
- [x] Reduce every per-section tmux/herdr delta — three bullet pairs and three prose sites — to its rationale and exception prose, leaving the matrix as the comparison surface *(SPEC-F14)* <!-- completed: 2026-07-25T14:34 -->
- [x] Convert `### The Esc safeguard` to a `Keystroke path | Leads with Esc? | Payload | Why` table over the five paths; keep the shared press-settle-type mechanism as prose above it *(SPEC-F15)* <!-- completed: 2026-07-25T14:34 -->
- [x] Add a `CAFLEET_MULTIPLEXER | HERDR_ENV | TMUX | Result` truth table to `## Backend selection` covering six of the eight combinations, with a one-line note stating why the other two are absent; keep the three numbered precedence items *(SPEC-F16)* <!-- completed: 2026-07-25T14:34 -->
- [x] Convert the second paragraph of `## Native agent-state (herdr only)` to a `Wake trigger | tmux | herdr` table with four rows; add no rows for undocumented states, and keep the `blocked` rule as prose *(SPEC-F17)* <!-- completed: 2026-07-25T14:34 -->

### Step 7: `docs/concepts/`

- [x] `monitoring.md`: add a wake-reason table (`Reason | Fires when | Advances last_ping_at? | Availability | Silenced by`) over `interval`, `unacked`, `stall-check`, `status:done` *(CON-F9)* <!-- completed: 2026-07-25T13:46 -->
- [x] `monitoring.md`: add a fourth column to the five-state rubric table giving each state its behavior when the member is tagged `unacked`. Verify the `working` row's behavior against the monitor role definition and the implementation rather than copying either the existing table cell or the existing prose — the two currently disagree, so neither is a safe source *(CON-F10)* <!-- completed: 2026-07-25T13:46 -->
- [x] `monitoring.md`: cross-reference the wake-reason table's "silenced by" column from the knob table instead of adding a second disable column; move the stall row's `(0 disables)` out of its "Set by" cell *(CON-F11)* <!-- completed: 2026-07-25T13:46 -->
- [x] `monitoring.md`: remove the duplicated `180s` / `720s` defaults from the § The watched set prose, leaving the knob table as their single home; the prose may keep a qualitative magnitude description per the echo rule *(CON cross-page — monitor default intervals)* <!-- completed: 2026-07-25T13:46 -->
- [x] `monitoring.md`: add the enrolled-member table (`Member class | Enrolled | monitor field on GET /api/members`) with five rows — this page is the site-wide owner. Re-derive the five member classes from actual enrollment behavior and reconcile any drift against this page's existing prose; a wrong row key here propagates to every linking page *(SPEC-F5)* <!-- completed: 2026-07-25T13:46 -->
- [x] `coding-agents.md`: replace § Reasoning effort's running sentence with a link plus one-clause summary pointing at the owning table in `docs/spec/coding-agent-backends.md`, keeping the validation and default prose *(CON-F2)* <!-- completed: 2026-07-25T13:46 -->
- [x] `coding-agents.md`: convert § Known asymmetries to a `Dimension | claude | codex | opencode` matrix over reasoning effort, pane title, and sandbox isolation, carrying supported / not-supported rather than restating the level sets; absorb the second opencode-unsupported mention *(CON-F3)* <!-- completed: 2026-07-25T13:46 -->
- [x] `coding-agents.md`: add a `Backend | Product | How the pane loads the cafleet skill` table collecting the per-backend facts currently spread across the intro and § cafleet usage from a member pane. Carry no auto-approval-posture column — that enumeration is owned by the Step 2 capability matrix; link to it instead *(CON-F4)* <!-- completed: 2026-07-25T13:46 -->
- [x] `coding-agents.md`: replace the inline placeholder enumeration with a link plus one-clause summary pointing at the owning table in `docs/spec/cli-options.md` *(CON-F14)* <!-- completed: 2026-07-25T13:58 -->
- [x] `model-selection.md`: convert § The model list's column description to a `Column | What it holds | How to read it` table over five rows, moving the judgment and estimate caveats into the rows they qualify *(CON-F5)* <!-- completed: 2026-07-25T13:46 -->
- [x] `model-selection.md`: convert § Cost efficiency mode's role policies to a `Role | Model chosen | Needs the cost efficiency mode trigger?` table over ordinary member, monitor, and reviewer *(CON-F6)* <!-- completed: 2026-07-25T13:46 -->
- [x] `model-selection.md`: convert the closing override paragraph to a `Situation | What the Director does` table over the five conditions, keeping the fail-closed framing sentence *(CON-F7)* <!-- completed: 2026-07-25T13:46 -->
- [x] `member-lifecycle.md`: convert § Delete ordering to a `Member state | What member delete does | Multiplexer effect` table over the three states *(CON-F12)* <!-- completed: 2026-07-25T13:46 -->
- [x] `member-lifecycle.md`: replace § Commands' prose enumeration with a slim table keyed by **lifecycle stage** — not restating purpose or flags — plus a link to the owning subcommand summary in `docs/spec/cli-options.md`; keep the identity-flag rule and the `member prompt` paragraph *(CON-F13)* <!-- completed: 2026-07-25T13:46 -->
- [x] `member-lifecycle.md`: replace the inline placeholder enumeration with a link plus one-clause summary pointing at the owning table *(CON-F14)* <!-- completed: 2026-07-25T13:58 -->
- [x] `storage.md`: convert § Schema management to a `Database state | What cafleet setup does` table over four states, keeping the no-schema failure outside the table *(CON-F15)* <!-- completed: 2026-07-25T13:46 -->
- [x] `overview.md`: extend the CLI group table to all seven entry points. **Paraphrase** the scope text for `setup`, `doctor`, and `server` to the concepts altitude rather than copying the spec wording, so the two pages differ in wording and only the spec page reads as the contract *(CON-F1)* <!-- completed: 2026-07-25T13:46 -->
- [x] `overview.md`: trim the `monitor` glossary cell to a one-clause definition matching its neighbours, leaving the mechanism to the linked Monitoring page *(CON-M1)* <!-- completed: 2026-07-25T13:46 -->

### Step 8: Entry points and how-to

- [x] `quickstart.md`: convert § Configure's three per-backend sub-headings to a `Backend | Config file | Manual configuration | Installed by cafleet setup | Reference` table. Place the two kept code snippets **below** the table, each introduced by the backend it belongs to; keep both rationale paragraphs *(EHA-F1)* <!-- completed: 2026-07-25T14:49 -->
- [x] `quickstart.md`: convert the prerequisites bullets to a `Requirement | Accepted | Notes` table over three rows *(EHA-F2)* <!-- completed: 2026-07-25T14:49 -->
- [x] `contributing.md`: convert § Tech stack's bold-key bullets to a `Concern | Technology | Notes` table over six rows, keeping every external link *(EHA-F3)* <!-- completed: 2026-07-25T14:49 -->
- [x] `contributing.md`: replace § Contributing changes' three-item list with a link plus one-clause summary pointing at the owning workflows table in `docs/how-to/design-doc-development.md`, keeping the ordering sentence and the contributor-specific delta *(EHA-F4)* <!-- completed: 2026-07-25T14:49 -->
- [x] `contributing.md`: split § Development's code block into a first-time setup sequence that stays a code block and a `Task | Runs | When you need it` table. Verify the pick-one task set against the live mise task set and write one row per task, whatever the count is then *(EHA-F5)* <!-- completed: 2026-07-25T14:49 -->
- [x] `design-doc-development.md`: convert § Prompts to a `Stage | Prompt | Workflow | Team` table over the three stages, keeping each prompt in a code span and the stage numbering *(EHA-F6)* <!-- completed: 2026-07-25T14:49 -->

### Step 9: Verify

Step 9 is executed **twice**: the implementer runs every check and records the results, then the reviewer independently confirms them.

- [x] Confirm each of the eight owned enumerations appears as a table on exactly one page and every other mention is a link plus a one-clause summary *(no finding — verification)* <!-- completed: 2026-07-25T14:53 -->
- [x] Confirm no table has exactly one data row, no cell exceeds two sentences, and no repository path was introduced outside the exempt surfaces *(no finding — verification)* <!-- completed: 2026-07-25T14:53 -->
- [x] Fix any one-row table found **anywhere on the site**, including tables this work never otherwise touched *(no finding — sitewide anti-rule)* <!-- completed: 2026-07-25T14:53 -->
- [x] For every paragraph Step 2, 4, or 6 deletes, confirm each fact it carried survives in the replacing table or in kept prose *(no finding — verification)* <!-- completed: 2026-07-25T14:53 -->
- [x] Run the project's strict docs build and confirm it passes and that every anchor added or retargeted by this work resolves *(no finding — verification)* <!-- completed: 2026-07-25T14:53 -->

**Out of scope, recorded as follow-up.** This change verifies the one-row-table and two-sentence-cell rules manually. Automated enforcement of both is deliberately out of scope here and is recorded as a follow-up design doc.
