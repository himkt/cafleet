# Findings: docs/concepts

## Summary

Fifteen tabulate-me findings and one table-misuse finding across the seven
assigned pages: `overview.md` 1, `coding-agents.md` 3, `model-selection.md` 4,
`monitoring.md` 3, `member-lifecycle.md` 3, `fleet-isolation.md` 0,
`storage.md` 1. The dominant pattern is **the per-backend / per-state /
per-condition delta written as a repeated paragraph or bullet**: the same
attribute (which backend supports it, what the Director does, what the flag
forwards, whether it advances a timestamp) is restated once per item in prose,
so a reader who wants to compare items must reconstruct the matrix in their
head. `coding-agents.md` is the worst case — its per-backend facts are spread
across four sections with no single comparison surface, and the effort-level
enumeration the user seeded from `docs/spec` is duplicated here in the same
prose form. The second pattern, concentrated in `monitoring.md`, is the
**incomplete existing table**: the page tabulates well, but two of its three
tables are missing a column that the immediately-following prose supplies for
every row. Two pages are already in good shape — `fleet-isolation.md` has no
findings, and `storage.md` has one; both are mostly single-topic prose with
nothing enumerable to extract.

## Tabulate-me findings

### F1. overview.md — ## CLI — the three top-level commands are named in prose while the four groups get a table

- **Location**: `docs/concepts/overview.md`, `## CLI`
- **Current form**: prose sentence + an existing table that covers only part of the surface. The sentence reads "The `cafleet` CLI is organized as three top-level commands (`setup`, `doctor`, `server`) plus four command groups:", followed by the `Group | Scope | Subcommands` table which has rows only for `fleet` / `member` / `message` / `monitor`.
- **Proposed table columns**: Entry point | Scope | Subcommands (extend the existing table; `setup`, `doctor`, `server` take an em-dash in the Subcommands cell)
- **Row keys**: `setup`, `doctor`, `server`, `fleet`, `member`, `message`, `monitor`
- **Prose that must remain**: the lead-in sentence (reduced to naming the seven entry points), and the whole paragraph after the table about `member` being the single home for the member lifecycle and the `kind` column.
- **Why**: the reader currently gets four of the seven CLI entry points in a scannable grid and the other three as an inline parenthetical, so the page never shows the CLI surface in one view.
- **Note for the implementer**: this page supplies no Scope text for `setup` / `doctor` / `server`. Source those three cells from the CLI options spec page's subcommand summary rather than inventing them.
- **Priority**: Low

### F2. coding-agents.md — ## Reasoning effort — per-backend effort flag and level set written as one running sentence

- **Location**: `docs/concepts/coding-agents.md`, `## Reasoning effort`
- **Current form**: prose. "`cafleet member create --effort <level>` forwards a reasoning-effort level to the spawned backend binary: claude via `--effort <level>` (levels `low`, `medium`, `high`, `xhigh`, `max`) and codex via `--config=model_reasoning_effort=<level>` (levels `minimal`, `low`, `medium`, `high`, `xhigh`)…" — three backends, two attributes each, packed into a single sentence with parenthetical level sets.
- **Proposed table columns**: Backend | Forwarded as | Accepted levels
- **Row keys**: `claude`, `codex`, `opencode` (the `opencode` row states that effort is unsupported and the create call is rejected)
- **Prose that must remain**: the opening sentence that `--effort` forwards to the spawned binary; the sentence that, unlike `--model`, the level set is validated per backend at create time before any registration or multiplexer side effect; that omitting the flag leaves the binary's own default; and the pointer to the backends spec page for exact argv forms and error strings.
- **Why**: this is the user's seed complaint in its concepts-page form — a reader comparing claude's five levels against codex's five different levels has to parse two nested parentheticals to notice that `max` is claude-only and `minimal` is codex-only.
- **Priority**: High

### F3. coding-agents.md — ## Known asymmetries (intentional non-goals) — three per-backend deltas as three repeated bullets

- **Location**: `docs/concepts/coding-agents.md`, `## Known asymmetries (intentional non-goals)`, anchor `#known-asymmetries-intentional-non-goals`
- **Current form**: bullets, each a bolded dimension followed by the same "only X does this, the others do that" shape. "**Reasoning effort.** Only `claude` and `codex` expose a reasoning-effort control…"; "**Pane title.** Only the `claude` spawn argv carries `--name`…"; "**Sandbox isolation.** Only `codex` provides OS-level (kernel-enforced) isolation…". This is the textbook repeated per-item paragraph pattern — every bullet restates the same attribute for each of the three backends.
- **Proposed table columns**: Dimension | `claude` | `codex` | `opencode`
- **Row keys**: Reasoning effort, Pane title, Sandbox isolation
- **Prose that must remain**: the section's framing that these are intentional non-goals; the `--effort` + `opencode` rejection message; the sentence that the `pane_id` column of `cafleet member list` is ground truth for all three; and the operator guidance that anyone needing kernel-enforced isolation should use the `codex` backend. Those are per-section notes, not per-cell content — keep them as a short prose tail beneath the table.
- **Why**: three bullets that each sweep all three backends is a 3×3 matrix written out longhand; the table lets a reader pick a backend column and read its full posture down one line.
- **Priority**: High

### F4. coding-agents.md — intro + ## cafleet usage from a member pane — per-backend facts scattered across the page with no comparison surface

- **Location**: `docs/concepts/coding-agents.md`, the unheaded intro (paragraphs 1 and 4) and `## cafleet usage from a member pane`
- **Current form**: prose in two separate places. The intro opens "cafleet supports three coding-agent binaries inside member panes: `claude` (Claude Code), `codex` (OpenAI Codex CLI), and `opencode` (opencode.ai)" and later states "claude and codex members run cafleet (and any shell command) directly; opencode members run only the commands on their preset's deny-by-default bash allowlist and route everything else to the Director." The usage section adds "claude panes load the Claude Code skills directly, while codex and opencode panes read the cafleet skill files by absolute path" and "All three honor a leading-`!` shell shortcut".
- **Proposed table columns**: Backend | Product | Auto-approval posture | How the pane loads the cafleet skill
- **Row keys**: `claude`, `codex`, `opencode`
- **Prose that must remain**: that the backend is selected per member with `--coding-agent`, that mixed-backend teams are allowed with no broker-level differences, that the value is recorded in the placement's `coding_agent` column; that identity reaches a member through its spawn prompt as rendered literals; that `CAFLEET_DATABASE_URL` is the only forwarded environment variable; that all three honor the leading-`!` shortcut so `member prompt --shell` works against any pane shape.
- **Why**: a reader choosing a backend currently has to collect its properties from four separate paragraphs on one page; one matrix answers "what do I get with `opencode`?" in a single row.
- **Note for the implementer**: F2, F3 and F4 can each stand alone, or all three can collapse into **one** per-backend capability matrix near the top of the page (rows = dimensions, columns = the three backends) with the individual sections keeping only their prose caveats. Recommend deciding F2/F3/F4 together rather than piecemeal, so the page ends with one comparison surface instead of three.
- **Priority**: Medium

### F5. model-selection.md — ## The model list — the model list's own columns described as a running sentence

- **Location**: `docs/concepts/model-selection.md`, `## The model list`
- **Current form**: prose. "It has one table per backend (`claude`, `codex`, and `opencode`), ordered most → least capable, with each model's spawn token, its alias (claude models), reviewed capability class, and standard input/output USD-per-MTok prices, plus links to the approved official source pages…" — five column descriptions in one sentence, with the how-to-read caveats for two of them deferred to the next sentence ("Capability classes and the ordering are maintainer judgment, never a provider benchmark claim; prices are standard provider rates — planning estimates, not an invoice guarantee.").
- **Proposed table columns**: Column | What it holds | How to read it
- **Row keys**: spawn token, alias, capability class, input/output price, source link
- **Prose that must remain**: that the model list is a catalog-style reference page bundled with the cafleet skill and carried by every deployed replica; that it has one table per backend ordered most → least capable; the maintenance and staleness rules; and that the `opencode` section is a curated subset keeping the `opencode/` prefix in the `--model` value.
- **Why**: the page is describing a table in prose, and the two caveats that matter most (capability class is judgment, price is an estimate) are stranded a sentence away from the columns they qualify.
- **Priority**: Medium

### F6. model-selection.md — ## Cost efficiency mode — three role selection policies split between a paragraph and two bullets

- **Location**: `docs/concepts/model-selection.md`, `## Cost efficiency mode`
- **Current form**: mixed prose + bullets. The ordinary-member rule is a paragraph ("Cost-minimized selection for an **ordinary member** applies only when the user asks for it…"), while the two exceptions are bullets under "Two roles are policy exceptions on **every** team spawn, trigger or not:" — "**Monitor** — the cheapest listed model that can run the monitoring protocol reliably." and "**Reviewer** — the most capable listed model of the chosen backend."
- **Proposed table columns**: Role | Model chosen | Needs the `cost efficiency mode` trigger?
- **Row keys**: ordinary member, monitor, reviewer
- **Prose that must remain**: how the trigger is detected (the exact phrase in the originating user request, parsed once, never activated by a member message or tool output); that the Director estimates difficulty from the member's spawn prompt and reads the model list; that without the trigger existing workflow model behavior is unchanged; and that the Director picks the backend first and compares within that backend's table.
- **Why**: the three roles differ on exactly two axes, but one is written as a paragraph and the other two as bullets, so the shared structure is invisible and the reader cannot see at a glance that only the ordinary-member row is trigger-gated.
- **Priority**: Medium

### F7. model-selection.md — ## Cost efficiency mode — five override / fail-closed conditions compressed into one dense paragraph

- **Location**: `docs/concepts/model-selection.md`, `## Cost efficiency mode`, the closing paragraph
- **Current form**: prose. "Explicit flags stay overrides: a user-supplied `--coding-agent`, `--model`, or `--effort` is recorded rather than silently replaced (a mismatched backend/model pair is relayed to the user instead of spawned), and a user-pinned model is never deleted and replaced automatically. A stale model list disables cost efficiency mode, and a task no listed model fits gets an operator relay — the Director **fails closed** instead of spawning a guessed model." Five distinct condition→behavior pairs in two sentences, two of them buried in a parenthetical and an em-dash clause.
- **Proposed table columns**: Situation | What the Director does
- **Row keys**: user supplies `--coding-agent` / `--model` / `--effort`; backend and model do not match; a user-pinned model would otherwise be replaced; the model list is stale; no listed model fits the task
- **Prose that must remain**: the framing sentence that explicit flags stay overrides and that the Director fails closed rather than spawning a guessed model.
- **Why**: these are the rules an operator consults when the Director did something unexpected, and they are currently the least scannable text on the page — a lookup table turns "why did it not use my model?" into one row.
- **Priority**: High

### F8. model-selection.md — ## Replacement — replacement triggers and bounds as an inline parenthetical

- **Location**: `docs/concepts/model-selection.md`, `## Replacement`
- **Current form**: prose with an inline parenthetical list. "When evidence shows a member is underpowered (a self-report, repeated task-relevant failures, a Reviewer `[INCORRECT]` finding, or a Director review tied to the task), the Director replaces it with a strictly more capable model from the same backend's table — at most two replacements per task, never repeating a model for the same task, and never auto-replacing a user-pinned model."
- **Proposed table columns**: Bound | Rule
- **Row keys**: replacements per task, model reuse, user-pinned model
- **Prose that must remain**: the replacement action itself (a strictly more capable model from the same backend's table) and the pointer to the Director instructions loaded with the cafleet skill.
- **Why**: the three bounds are the operative constraints and they are currently the tail of a 60-word sentence.
- **Note for the implementer**: this is the weakest finding in the set. The four evidence triggers are single-attribute items, so they should become a plain bulleted list rather than table rows; only the three bounds are genuinely tabular, and a table of three one-line rules is a marginal gain over a bulleted list. Drop this one if the page reads well after F6 and F7 land.
- **Priority**: Low

### F9. monitoring.md — ## Heartbeat vs facilitation + ## The watched set — the four wake-nudge reasons named in one line, then explained across two sections

- **Location**: `docs/concepts/monitoring.md`, `## Heartbeat vs facilitation` (the paragraph beginning "The loop's only keystroke is a **wake nudge**…") and `## The watched set`
- **Current form**: an inline parenthetical enumeration followed by scattered per-item prose. The four reasons appear as a bare list — "naming each due member as `<role> <id> (<name>) [<reasons>]` (reasons: `interval`, `status:done`, `stall-check`, `unacked`)" — and each is then explained somewhere else: `interval` and `stall-check` in the first watched-set paragraph, `unacked` in its own paragraph, `status:done` in the herdr paragraph, and the disable switches for all of them down in `## Cadence and tick precision`.
- **Proposed table columns**: Reason | Fires when | Advances `last_ping_at`? | Availability | Silenced by
- **Row keys**: `interval`, `unacked`, `stall-check`, `status:done`
- **Prose that must remain**: the wake-nudge format line and the standing-inspect-target note about the Director; the enrollment sentence (root Director and every ordinary member; the monitoring member and placementless rows excluded); the three-part flag condition (enabled, pane alive, interval elapsed); the wake-storm rationale for the timestamp stamp; the re-flag/stop-flagging behavior of `unacked`; and the herdr paragraph's point that `blocked` never wakes the watcher and that this trigger augments rather than replaces the interval trigger.
- **Why**: `[<reasons>]` is what an operator actually reads in a wake nudge, and decoding one of the four tags currently means hunting through three sections; the table makes the reasons the page's index into its own mechanics.
- **Priority**: High

### F10. monitoring.md — ## The monitoring member — the five-state rubric table is missing the unacked-report column its own step 4 supplies

- **Location**: `docs/concepts/monitoring.md`, `## The monitoring member`, anchor `#the-monitoring-member` — the `State | Evidence | Monitor action` table inside step 2, and step 4 immediately below it
- **Current form**: incomplete table. The rubric table carries three columns for the five states, and then step 4 restates the same five states in prose to give each a second action: "report it to the Director whenever its pane classifies `finished`, `stalled`, or `working` — a busy pane is still reported and the Director decides. `awaiting_user` suppresses the report… and `unknown` suppresses it too". Every row of the existing table has an unacked-report behavior; none of them shows it.
- **Proposed table columns**: State | Evidence | Monitor action | Action when the member is tagged `unacked` (extend the existing table with a fourth column)
- **Row keys**: the existing five — `awaiting_user`, `unknown`, `finished`, `stalled`, `working`
- **Prose that must remain**: the "first match wins" instruction above the table; the ambiguity tie-break paragraph explaining why an unresolvable capture classifies `awaiting_user`; and the per-state rationale from step 4 (a busy pane is still reported and the Director decides; `awaiting_user` means the member waits on the user, not on a nudge; an unreadable capture cannot rule out a pending prompt). Keep those as a short tail — they are reasons, not cells.
- **Why**: `working` is the sharpest case — the rubric table says "None" while step 4 says report it when tagged `unacked`, and a reader consulting only the table gets the wrong answer.
- **Priority**: High

### F11. monitoring.md — ## Cadence and tick precision — the knob table lacks the "disabled by" column the following paragraph supplies

- **Location**: `docs/concepts/monitoring.md`, `## Cadence and tick precision`, anchor `#cadence-and-tick-precision` — the `Knob | Default | Set by` table and the paragraph beneath it
- **Current form**: incomplete table. The paragraph after the table supplies a disable path per knob: "It has no dedicated switch: `cafleet monitor config --disable` (`enabled = 0`) silences the member from **all** triggers, while `CAFLEET_MONITOR_STALL_INTERVAL=0` disables only stall-check and does not affect `unacked`." The stall-check row already smuggles its own disable into the "Set by" cell as "(`0` disables)", so the column is half-present and inconsistently placed.
- **Proposed table columns**: Knob | Default | Set by | Disabled by (extend the existing table; move the stall row's `(0 disables)` out of the "Set by" cell into the new column)
- **Row keys**: the existing five — root Director ping interval, ordinary member ping interval, stall-check interval, unacked-delivery staleness / re-fire, scan tick
- **Prose that must remain**: the tick-floor explanation and the advice to set the tick smaller than the smallest interval you care about; that per-member intervals are editable via `monitor config` or the WebUI; the stall-detection cadence rationale; and the paragraph on the unacked trigger reusing the member's interval as both threshold and re-fire period, coupling staleness tolerance to check cadence by design.
- **Why**: "how do I turn this one off?" is the second question every knob raises, and the answer is currently in a paragraph that mixes two different disable mechanisms in one sentence.
- **Note for the implementer**: the "Silenced by" column of F9 and this "Disabled by" column carry overlapping facts from different angles (per wake reason vs per knob). Land F9 first, then decide whether this column is still needed or whether a cross-reference suffices.
- **Priority**: Medium

### F12. member-lifecycle.md — ## Delete ordering — three delete cases as one prose paragraph

- **Location**: `docs/concepts/member-lifecycle.md`, `## Delete ordering`
- **Current form**: prose. "`cafleet member delete` tears down the pane (when one exists) and soft-deletes the member. Pane path: kill the pane immediately (tolerating an already-gone pane), then deregister — exit 0. A member with a pending placement (no pane yet) is a plain registry soft-delete, and so is a placementless registry row (no placement row) — `cafleet member delete` soft-deletes both without touching the multiplexer." Three member states, each with the same two attributes, run together in four sentences.
- **Proposed table columns**: Member state | What `member delete` does | Multiplexer effect
- **Row keys**: member with a live pane, member with a pending placement (no pane id yet), placementless registry row (no placement row)
- **Prose that must remain**: the one-line summary that `member delete` tears down the pane when one exists and soft-deletes the member; the tolerance note that an already-gone pane is not an error; and the exit-0 result.
- **Why**: the three cases differ only in whether a pane exists, and that is exactly what the current phrasing hides behind two subordinate clauses.
- **Priority**: Medium

### F13. member-lifecycle.md — ## Commands — seven member subcommands enumerated in a prose paragraph

- **Location**: `docs/concepts/member-lifecycle.md`, `## Commands`
- **Current form**: prose. "The lifecycle ops live in the `member` group: `member create`, `member delete`, `member show` (single-member detail — kind, skills, placement block), and `member list` (every active registry entry of the fleet, with `kind` and `idle` columns). Keystroke interaction lives in the same group: `member capture`, `member prompt`, and `member ping`." Seven items, some with a parenthetical gloss and some with none, plus a following sentence that supplies a second attribute for all of them: "`member create` takes no identity flag — the CLI resolves the Director from `fleets.director_member_id`; every other lifecycle verb targets its member by `--member-id`".
- **Proposed table columns**: Command | Category (lifecycle / keystroke interaction) | What it does
- **Row keys**: `member create`, `member delete`, `member show`, `member list`, `member capture`, `member prompt`, `member ping`
- **Prose that must remain**: the `member create` / every-other-verb identity-flag rule and the `--fleet-id` scoping sentence; the whole closing paragraph on `member prompt`'s plain form versus its `--shell` form; and the pointer to the CLI options page for every flag and the shared resolution rules.
- **Why**: three of the seven commands get a description and four get none, so the paragraph reads as an incomplete gloss rather than a surface the reader can scan.
- **Note for the implementer**: the CLI options spec page already carries a full per-subcommand table with Purpose and Identity-flag columns covering all seven. Keep the concepts table deliberately slim — command plus its role in the lifecycle — and leave the identity-flag column to the spec, or replace the enumeration entirely with a link. See *Cross-page duplication* below.
- **Priority**: High

### F14. member-lifecycle.md + coding-agents.md — the four identity placeholders enumerated inline on both pages, glossed on neither

- **Location**: `docs/concepts/member-lifecycle.md`, `## Atomic create flow` (the sentence beginning "Identity reaches the spawned pane as literals rendered into the prompt"); and `docs/concepts/coding-agents.md`, `## cafleet usage from a member pane` (the sentence beginning "Identity reaches a member through its spawn prompt")
- **Current form**: an inline four-item enumeration on each page. The lifecycle page writes "substituting `{fleet_id}`, `{member_id}` (the member's own newly-allocated id), `{director_member_id}`, and `{coding_agent}`" — one of the four glossed, three bare. The coding-agents page writes "renders the four identity placeholders — `{fleet_id}`, `{member_id}`, `{director_member_id}`, and `{coding_agent}` — to literals" with no glosses at all, then gives the example lines `FLEET ID: 1`, `YOUR MEMBER ID: 4`.
- **Proposed table columns**: Placeholder | Resolves to | Appears in the spawn prompt as
- **Row keys**: `{fleet_id}`, `{member_id}`, `{director_member_id}`, `{coding_agent}`
- **Prose that must remain**: that `member create` runs the substitution over the resolved prompt at spawn time; that the member reads its ids as plain text lines and passes them explicitly on every command; and the doubled-brace requirement (which lives in `## Spawn-prompt input modes` on the lifecycle page).
- **Why**: this is the one enumeration a spawned member's author must get exactly right, and today it is a comma-separated run of four tokens repeated on two pages with a single parenthetical gloss between them.
- **Note for the implementer**: neither page supplies the "resolves to" text for `{fleet_id}`, `{director_member_id}`, or `{coding_agent}`, and neither supplies the literal label line for the latter two. Source those cells from the CLI options spec page's `member create` section — do not infer them. Put the table on **one** page (the coding-agents usage section is the better home, since it already carries the example label lines) and have the other link to it.
- **Priority**: Medium

### F15. storage.md — ## Schema management — four setup outcomes stated as a run of clauses

- **Location**: `docs/concepts/storage.md`, `## Schema management`
- **Current form**: prose. "…it migrates the database in place to the bundled head revision, preserving existing data (message history included), so it is idempotent and safe to re-run after every upgrade. It refuses to auto-downgrade a database that is ahead of the bundled head, and refuses an unversioned database with tables it does not recognize. Without the schema, the first request fails with `OperationalError: no such table: members`." Four database states, each with an outcome, chained across three sentences.
- **Proposed table columns**: Database state | What `cafleet setup` does
- **Row keys**: behind the bundled head, already at the bundled head, ahead of the bundled head, unversioned with unrecognized tables
- **Prose that must remain**: that the schema is a chain of Alembic migrations with the current revision recorded in `alembic_version`; that operators run the schema-only invocation once before starting the server; the data-preservation and idempotency guarantee; and the no-schema failure (`OperationalError: no such table: members`), which is a consequence of never running setup rather than a setup outcome and so belongs outside the table.
- **Why**: "what happens if I re-run setup against my database?" is the operator's actual question, and the two refusal cases are currently a single sentence joined by "and".
- **Priority**: Medium

## Table-misuse findings

### M1. overview.md — ## Core terms — the `monitor` row's definition cell is a paragraph in a glossary table

- **Location**: `docs/concepts/overview.md`, `## Core terms`, the `monitor` row
- **Current form**: an over-long cell in an otherwise well-sized table. The cell reads "a fleet-scoped loop the monitoring member runs as a background task, waking the monitoring member by keystroke whenever a watched member (Director or ordinary member) is due on its own interval" — roughly three times the length of every other Definition cell in the table, with three subordinate clauses and a parenthetical, and it repeats "the monitoring member" twice.
- **Proposed fix**: trim the cell to a one-clause glossary definition in the register of its neighbours (the `message` and `broker` rows are the right length), and let the mechanism live on the Monitoring page — which this row already links to in its "Links to" column, and which covers the wake mechanics in full.
- **Why**: a glossary table is scanned vertically; one cell that wraps to several lines breaks the row rhythm and makes the table taller than the diagram beneath it, for a definition the reader must follow the link to actually use.
- **Priority**: Low

## Cross-page duplication

- **Per-backend reasoning-effort levels (F2)** — the same enumeration appears on `docs/concepts/coding-agents.md` (`## Reasoning effort` and again in `## Known asymmetries`) and in `docs/spec/coding-agent-backends.md`, where it is stated per backend under separate sub-headings rather than as a table. That spec page is the user's named seed example and belongs to another auditor's scope. Recommend the Director settle the spec-side table first and then decide whether the concepts page gets its own three-row summary (F2) or drops to a link — but the concepts page should not keep the current prose form either way. Note also that the opencode-unsupported fact is stated **twice on the concepts page itself**, once in `## Reasoning effort` and once in the `## Known asymmetries` bullet; whichever table lands should absorb both mentions.
- **Member subcommands (F13)** — `docs/spec/cli-options.md` already carries a complete subcommand summary table with Purpose and Identity-flag columns for all seven `member` verbs. `docs/concepts/overview.md` lists the same seven as bare names in its CLI group table, and `docs/concepts/member-lifecycle.md` re-enumerates them in prose with partial glosses. Three surfaces, one enumeration. Recommend the spec table stay authoritative, the overview keep its names-only grouping, and the lifecycle page carry either a slim lifecycle-role table or just a link — a full-fidelity third copy would drift.
- **The four identity placeholders (F14)** — enumerated on both `docs/concepts/coding-agents.md` and `docs/concepts/member-lifecycle.md`, with a partial gloss on one and none on the other. One table on one page; the other page links.
- **Monitor default intervals** — `180s` for the root Director and `720s` for ordinary members appear both in `## The watched set` prose and in the `## Cadence and tick precision` knob table on `docs/concepts/monitoring.md`. Intra-page duplication only, but worth resolving while F9 and F11 are open: the table should be the single home for the numbers and the watched-set prose should describe enrollment without restating the defaults.
- **The five-state pane rubric** — checked across the whole docs site; it appears **only** on `docs/concepts/monitoring.md`. No duplication to resolve, and the capture-gate paragraph in `## Heartbeat vs facilitation` correctly references the rubric rather than restating it.
