# Findings: docs/spec/cli-options.md

## Summary

17 findings: 14 tabulate-me candidates and 3 table-misuse candidates. This page
is already the most table-dense page on the site (one flag table per
subcommand, an Option Source Matrix, a Subcommand summary, a 43-row Error
Messages table), so the dominant pattern is **not** "no tables at all" — it is
**tabular content that leaked out of the tables that should own it**. Three
sub-patterns recur: (a) a parallel enumeration compressed into a single table
*cell* as a semicolon-separated prose list (the `--effort` per-backend levels —
the exact shape of the user's seed example — and the `--quiet` sentence);
(b) an attribute supplied uniformly for every row but never promoted to a
column (the exit code, present in parentheses on all 43 Error Messages rows);
and (c) a per-subcommand fact restated as a trailing prose sentence in ~15
sections instead of one cross-cutting table (the "Text output: X; JSON: {…}"
paragraph, and the `member list` column-by-column narration). A fourth,
smaller pattern is drift between an escape-hatch table and the subcommands that
actually accept the flag (`--full` and `--quiet` both).

## Tabulate-me findings

### F1. `member create` — per-backend `--effort` levels compressed into one table cell

- **Location**: `## cafleet member — Member Lifecycle + Pane Interaction` › `### member create` (`#member-create`), the `--effort` row of the flag table
- **Current form**: prose list inside a table cell — "Reasoning-effort level forwarded to the backend binary (claude: `low`, `medium`, `high`, `xhigh`, `max`; codex: `minimal`, `low`, `medium`, `high`, `xhigh`; opencode: unsupported). Validated per backend before any side effect…". The matching rejection strings live four hundred lines away as three separate Error Messages rows ("`member create --effort` with a level unknown to the claude backend", "`member create --coding-agent codex --effort` with an unknown level", "`member create --coding-agent opencode --effort` with any value"), and the `--model` per-backend format constraint (opencode's `<provider-id>/<model-id>`) is a fourth orphan row with no counterpart in the flag table at all.
- **Proposed table columns**: `Backend` | `Accepted --effort levels` | `Rejection error (exit 2)` | `--model format constraint`
- **Row keys**: `claude`, `codex`, `opencode`
- **Why**: this is the seed example verbatim — a reader choosing an effort level for a backend currently has to parse a semicolon list inside a cell, then hunt the Error Messages table to learn what a wrong value produces; one three-row table answers both at a glance.
- **Priority**: High

### F2. Error Messages — exit code supplied for every row but never promoted to a column

- **Location**: `## Error Messages`
- **Current form**: incomplete table. Two columns (`Situation` | `Error Message`), with the exit code buried in a parenthetical inside the *message* cell on all 43 rows — "…`run 'cafleet setup' first` (exit 1; see Stale-assets guard)", "`Error: Missing option '--member-id'.` (exit 2)", "`Error: fleet 'X' not found.` (exit 1)".
- **Proposed table columns**: `Command` | `Situation` | `Error message` | `Exit`
- **Row keys**: the existing 43 rows unchanged; the parenthetical exit code moves into the new `Exit` column (values `1` and `2`), and any residual parenthetical that is *not* an exit code (e.g. "no DB writes", "the just-registered member is rolled back", "fires before any side effect") stays in the situation cell or becomes a short note. The proposed `Command` column takes the value already implied by each row's leading text — `setup`, `fleet create`, `fleet delete`, `member create`, `member delete`, `member prompt`, `member capture`/`prompt`/`ping`, `message *`, `monitor start`, `monitor config`, and `(any fleet-scoped command)` for the two stale-assets rows.
- **Why**: exit code is the single most scannable fact in an error catalog and is exactly the "status/error code explanation" shape the user named; a reader asking "which failures exit 2?" currently has to read 43 message cells.
- **Priority**: High

### F3. Exit-code semantics stated as inline semicolon prose in two places

- **Location**: (a) `## cafleet member — Member Lifecycle + Pane Interaction` › `### Member targeting and key delivery`, closing sentence; (b) `## cafleet monitor — Supervision Scheduler` › `### monitor start`, closing sentence
- **Current form**: prose. (a) "Common exit codes: `0` dispatch success; `1` multiplexer unavailable, member not found, missing placement, pending placement, or a `send-keys` failure; `2` per-subcommand argument/validation errors." (b) "Exit `0` on clean exit; `1` already running / unknown fleet / multiplexer unreachable; `2` usage errors."
- **Proposed table columns**: `Exit` | `Meaning` (one small table per site, replacing the sentence)
- **Row keys**: site (a): `0`, `1`, `2` — with the `1` cell broken into its five listed triggers as a short bulleted cell or, better, one row per trigger sharing the `1` code. Site (b): `0`, `1`, `2` — `1` splitting into "already running", "unknown fleet", "multiplexer unreachable".
- **Why**: a semicolon list of codes is the least scannable form of the most lookup-driven content on the page; a reader debugging exit 1 wants to see its triggers stacked, not comma-joined mid-sentence.
- **Priority**: High

### F4. Member targeting and key delivery — verb × placement-state matrix written as a numbered list with inline exceptions

- **Location**: `## cafleet member — Member Lifecycle + Pane Interaction` › `### Member targeting and key delivery`
- **Current form**: numbered list of three "resolution rules", each stating a behavior *and* its per-verb exceptions in prose — "1. A cross-fleet, unknown, or inactive `--member-id` resolves to 'not found'…", "2. No placement row → exit 1 … — except `member show` and `member delete`, which tolerate a missing placement.", "3. A pending placement (`pane_id` is `None`) → `capture` / `prompt` / `ping` exit 1; `delete` tolerates it."
- **Proposed table columns**: `Target state` | `capture` / `prompt` / `ping` | `show` | `delete`
- **Row keys**: `active, placed with pane_id`; `active, placement pending (pane_id is None)`; `active, no placement row`; `cross-fleet / unknown / inactive`
- **Why**: this is a two-dimensional matrix (four target states × four verbs) currently encoded as exception clauses; a reader asking "does `member delete` work on a placementless member?" must today assemble the answer from an "except" in rule 2 and a "tolerates it" in rule 3. This is not an ordered procedure — the numbering implies a sequence that does not exist.
- **Priority**: High

### F5. Output shapes — "Text output … ; JSON: {…}" restated as a trailing prose sentence in ~15 sections

- **Location**: page-wide repeated pattern, one instance per subcommand section: `### fleet create` ("Default output is one compact line carrying the two ids"), `### fleet list` ("Text output renders `FLEET_ID`, `DIRECTOR`, …"), `### fleet show` ("adding a `deleted_at:` line in text mode"), `### fleet delete` ("Prints `Deleted fleet <fleet_id>. Deregistered N members.`"), `## cafleet message — Message Broker` ("Text output is the subcommand's acknowledgement line…"), `### message broadcast` ("Default text output is `broadcast id=…`"), `### member create` ("Default output is the compact line `<member_id> <name> backend=…`"), `### member delete` ("Success output is a `Member deleted.` header plus…"), `### member show` ("Default text is the compact one-line row…"), `### member capture` ("JSON: `{member_id, pane_id, lines, content}`; text emits the content…"), `### member prompt` ("Text output: `Sent prompt '<text>' to member <name> (<pane_id>).`; … JSON: `{member_id, pane_id, text, shell}`"), `### member ping` ("Text: `Pinged member <name> (<pane_id>) — poll keystroke dispatched.`; JSON: `{member_id, pane_id}`"), `### monitor config` ("Text: `member 5: interval 720s, enabled, last_ping …`")
- **Current form**: repeated per-item paragraph — the same two attributes (default text shape, JSON key set) narrated once per subcommand, in a different sentence form each time ("Default output is…", "Text output renders…", "Prints…", "Success output is…", "Text:").
- **Proposed table columns**: `Subcommand` | `Default text output` | `--full` text output | `JSON payload`
- **Row keys**: `fleet create`, `fleet list`, `fleet show`, `fleet delete`, `message send`, `message broadcast`, `message poll`, `message ack`, `message show`, `member create`, `member delete`, `member show`, `member list`, `member capture`, `member prompt`, `member ping`, `monitor status`, `monitor config`, `doctor` — with `—` in the `--full` column for the subcommands that do not accept it
- **Why**: the strongest type-2 signal on the page — the same attribute restated in prose 15 times. A reader scripting against the CLI wants the JSON key sets side by side; today they must scroll the entire page and re-read each closing paragraph. This table also subsumes and de-duplicates the existing `--full` semantics table (see F8).
- **Priority**: High

### F6. `member list` — text columns and JSON fields narrated field-by-field in two prose paragraphs

- **Location**: `## cafleet member …` › `### member list` (`#member-list`), the two paragraphs beginning "Text output renders one row per member with `member_id`, `name`, `kind` …" and "`--json` returns one dict per row with `member_id`, `name`, `kind`, `placement` …"
- **Current form**: prose. The text paragraph enumerates six columns and then supplies per-column edge-case rendering in follow-on clauses ("A placementless row renders `-` in its placement cells (`backend`, `pane_id`); a placed row whose pane id is not yet patched renders `(pending)` in `pane_id`. `idle` is the wall-time since … humanized as `Ns` / `Nm` / `Nh`, `-` when the member has no message activity."). The JSON paragraph then re-enumerates an overlapping but different field set with its own type/null rules.
- **Proposed table columns**: `Field` | `Text column` | `Text rendering when absent` | `JSON key` | `JSON type`
- **Row keys**: `member_id`, `name`, `kind`, `backend`, `pane_id`, `idle`, `placement`, `last_sent`, `last_recv`, `last_ack`
- **Why**: ten fields with four attributes each, currently requiring the reader to hold the text list in their head while reading the JSON list to spot that `backend`/`pane_id` are text-only projections of `placement` and that `last_ack` is JSON-only. A table makes the text/JSON asymmetry visible instead of inferable.
- **Priority**: High

### F7. Environment variables scattered across three sections with no single catalog

- **Location**: `## Option Source Matrix` (rows "Database URL" → `CAFLEET_DATABASE_URL`, "Multiplexer backend" → `CAFLEET_MULTIPLEXER`); `## Message Body Truncation` (the one-row table for `CAFLEET_MAX_TEXT_LEN`); `## cafleet server — Admin WebUI Server` (`#cafleet-server`) (the `--host` / `--port` rows naming `CAFLEET_BROKER_HOST` / `CAFLEET_BROKER_PORT`, plus the sentence "Flag wins over env var; env var wins over the hardcoded default.")
- **Current form**: incomplete/split tables — five environment variables sharing the same attribute set (settings field, default, what it controls, what overrides it) documented in three different places, one of which is a single-row table.
- **Proposed table columns**: `Environment variable` | `Settings field` | `Default` | `Controls` | `Overridden by`
- **Row keys**: `CAFLEET_DATABASE_URL`, `CAFLEET_MULTIPLEXER`, `CAFLEET_MAX_TEXT_LEN`, `CAFLEET_BROKER_HOST`, `CAFLEET_BROKER_PORT`
- **Why**: "which env vars does cafleet read?" is a first-class operator question with no single answer surface today; consolidating also gives the one-row truncation table three siblings and makes the flag > env > default precedence a column rather than a sentence.
- **Priority**: Medium

### F8. `--full` semantics table is missing `fleet create`

- **Location**: `### --full semantics (cross-subcommand escape hatch)` (`#full-semantics`)
- **Current form**: incomplete table. Four rows — `message {send,poll,ack,show}`, `message broadcast`, `member show`, `member create` — but `### fleet create`'s own flag table documents a fifth: "`--full` | no | Switches the non-JSON output from the compact one-line form to a labeled block." The surrounding prose supplies the missing row's content; the escape-hatch table claims to be "a documented flag on every subcommand that accepts it".
- **Proposed table columns**: unchanged — `Subcommand` | `Default behavior` | `--full behavior`
- **Row keys**: existing four plus `fleet create` (default: the compact `<fleet_id> director=<director_member_id>` line; `--full`: the labeled block)
- **Why**: a table that advertises completeness and is not complete actively misinforms — a reader concludes `fleet create --full` does not exist.
- **Priority**: Medium

### F9. `--quiet` documented only as a prose aside, absent from the flag tables that should carry it

- **Location**: `## Message Body Truncation`, the closing sentence — "A `--quiet` flag on `cafleet message send`, `cafleet message ack`, and `cafleet member ping` suppresses the normal output and prints only the bare `message_id` (the target member id for `ping`), for shell capture."
- **Current form**: prose naming three subcommands with a per-subcommand difference in what is printed. Two of the three flag tables omit the flag entirely: `### message send`'s table lists only `--from-member-id`, `--to-member-id`, `--text`, `--text-file`, `--full`; `### message ack`'s lists only `--member-id`, `--message-id`, `--full`. Only `### member ping`'s table has a `--quiet` row.
- **Proposed table columns**: `Subcommand` | `Quiet output` (a small semantics table paralleling `--full` semantics), **plus** a `--quiet` row added to the `message send` and `message ack` flag tables so per-subcommand lookup succeeds
- **Row keys**: `message send` (bare `message_id`), `message ack` (bare `message_id`), `member ping` (bare target member id)
- **Why**: the flag is invisible to anyone reading a subcommand section directly — the canonical way this page is used — and the one asymmetry (`ping` prints a member id, not a message id) is currently a parenthetical.
- **Priority**: Medium

### F10. `cafleet setup` db half — three prior-DB states mapped to output lines via code-block comments

- **Location**: `## cafleet setup — Onboarding and Schema Management` (`#cafleet-setup`), step 1 "db half", the fenced block beginning "`Created <db_file> and applied migrations to head (<head>).   # fresh DB`", and the adjacent refusal block beginning "`DB has existing tables but no alembic_version.`"
- **Current form**: a code block whose right-hand `#` comments carry the second attribute — three (state, output) pairs — followed by a second code block holding two refusal strings whose triggering states are described only by the lead-in "It refuses two states".
- **Proposed table columns**: `Prior DB state` | `Outcome` | `Output / refusal message`
- **Row keys**: `no DB file` (fresh — created and migrated to head), `behind head` (upgraded), `at head` (no-op), `tables present but no alembic_version` (refused), `revision unknown to this CLI version` (refused)
- **Why**: the five states of the db half are the whole contract of `cafleet setup`, currently split across two code blocks with the state expressed once as a trailing comment and once as unnumbered prose; one table shows all five and makes the success/refusal split visible. (The exact strings stay verbatim in the message column — a table does not weaken them.)
- **Priority**: Medium

### F11. `cafleet setup` failure strings enumerated in a prose paragraph

- **Location**: `## cafleet setup …` (`#cafleet-setup`), the paragraph beginning "The halves fail independently (`db half failed: <msg>` / `assets half failed: <msg>`); if any half that ran failed…", plus the following paragraph "Assets-half pre-flight: the `asset_installs` table must exist, else the half fails with…", plus `### Assets half` (`#assets-half`) steps 1–2 ("a failure aborts with `failed to install skills into <skills_dir>: <error>`" / "aborts with `failed to install preset into <target>: <error>`")
- **Current form**: prose. Six or more distinct failure strings, each with a trigger and an effect on the exit status, spread across three paragraphs and two numbered steps — while the page's `## Error Messages` table (which owns exactly this content shape) carries only one `setup` row.
- **Proposed table columns**: `Half` | `Trigger` | `Message` | `Effect`
- **Row keys**: `db half` — refusal states (cross-referencing F10); `assets half` — `asset_installs` table missing (`the database schema is missing or outdated; run 'cafleet setup' first`), no release for version (`no release found for version <version>`), asset missing from release (`asset cafleet-assets-v<version>.zip not found in release <version>`), skills install failure (`failed to install skills into <skills_dir>: <error>`), preset install failure (`failed to install preset into <target>: <error>`), all agents skipped (`assets half skipped (all agents skipped)` — not-run, cannot contribute a failure)
- **Why**: `setup` is the only command whose failure catalog lives outside the Error Messages table; a reader diagnosing a failed setup must read four paragraphs of narrative rather than scan rows. Either this table lands in-section or these rows join `## Error Messages`.
- **Priority**: Medium

### F12. Spawn-prompt substitution — four placeholders enumerated in a sentence

- **Location**: `### member create` › `#### Spawn-prompt substitution`
- **Current form**: prose — "runs `str.format` over the resolved prompt body, substituting exactly four placeholders: `{fleet_id}`, `{member_id}` (the member's own newly-allocated id), `{director_member_id}`, and `{coding_agent}`." Two of the four get a parenthetical gloss; two get none. The same four names are re-listed inside an Error Messages cell ("Supported placeholders: {fleet_id}, {member_id}, {director_member_id}, {coding_agent}").
- **Proposed table columns**: `Placeholder` | `Substituted value` | `How the spawned member sees it`
- **Row keys**: `{fleet_id}`, `{member_id}`, `{director_member_id}`, `{coding_agent}`
- **Why**: this is the authoring contract for every spawn prompt; the reader needs all four glossed uniformly, and today two of them are glossed and two are left to inference from the name.
- **Priority**: Medium

### F13. Stale-assets guard — condition/outcome cases as a numbered list, exemptions as a prose clause

- **Location**: `## Stale-assets guard` (`#stale-assets-guard`)
- **Current form**: numbered list of three items — "1. Missing DB file, missing `asset_installs` table, or zero rows → exit 1 with …", "2. Any recorded `cafleet_version` differing from the runtime CLI version … → exit 1 with …", "3. Otherwise the command proceeds silently." — followed by prose "Exempt surfaces: `setup` (must remain runnable to repair), `doctor` (reports instead of blocking), and `server`."
- **Proposed table columns**: two tables. (a) `Recorded install state` | `Result` | `Exit`. (b) `Exempt surface` | `Why exempt` | `Behavior under a stale/missing install`
- **Row keys**: (a) `no DB file / no asset_installs table / zero rows`, `recorded version ≠ runtime CLI version (including a downgrade)`, `all recorded versions match`. (b) `setup`, `doctor`, `server`.
- **Why**: case analysis, not a procedure — the numbering falsely implies ordered steps. The exemption clause is three items each carrying a distinct reason, and one of the three (`server`) currently has no stated reason at all, which the column would expose.
- **Priority**: Medium

### F14. Two tables keyed by subcommand that should be one

- **Location**: `## Subcommand summary` (the 22-row `Subcommand | Purpose | --fleet-id | Identity flag | Section` table) and `## JSON output (--json)` (`#json-output`) (the `Group | Subcommands` table listing which subcommands accept `--json`)
- **Current form**: two tables with the same key. The `--json` table inverts the key (grouping subcommand names into a cell per group), so answering "does `member capture` accept `--json`?" from the summary table is impossible — the reader must jump to a second table and scan inside a cell.
- **Proposed table columns**: fold `--json` (and `--full`, per F8/F5) into the summary table: `Subcommand` | `Purpose` | `--fleet-id` | `Identity flag` | `--json` | `--full` | `Section`, and reduce the JSON-output section to its placement/semantics prose plus the "all other subcommands reject `--json`" rule.
- **Row keys**: the existing 22 summary rows
- **Why**: the summary table is the page's index; a reader checking flag availability for one subcommand should get the answer from the row they are already on. Note the tradeoff — seven columns is wide; if the merge is rejected, the minimum fix is re-keying the `--json` table one-row-per-subcommand so it can be scanned by name.
- **Priority**: Low

## Table-misuse findings

### M1. Single-row tables used where a sentence or a flag list would read better

- **Location**: `## Global Options` (a two-column table with exactly one row, `--version`); `## Message Body Truncation` (a four-column table with exactly one row, `CAFLEET_MAX_TEXT_LEN`); and the one-flag subcommand tables at `### fleet list` (`--json` only), `### fleet delete` (`--fleet-id` only), and `### member delete` (`--member-id` only)
- **Current form**: table used for a single item — e.g. `## Global Options` renders a `Flag | Required | Notes` header row to carry one flag whose `Required` cell is "no".
- **Proposed remedy**: `## Global Options` becomes a sentence ("`--version`, placed before the subcommand, prints `cafleet <version>` and exits 0, bypassing the `--fleet-id` requirement."). The `CAFLEET_MAX_TEXT_LEN` row merges into the consolidated environment-variable table (F7), removing the standalone table. The one-flag subcommand tables are a genuine tradeoff — house consistency across ~20 subcommand sections is a real argument for keeping them — so treat those three as lowest priority and change them only if the whole per-subcommand pattern changes.
- **Why**: a header row plus one data row costs more vertical space and more eye movement than the sentence it encodes, and a table implies a set the reader should scan.
- **Priority**: Low

### M2. `member prompt` — form-comparison table whose cells hold multi-sentence explanations

- **Location**: `### member prompt` (`#member-prompt`), the `Form | Keystroke sequence | Follow-up` table
- **Current form**: table whose cells no longer scan. The plain-form keystroke cell reads "`Esc` → settle → literal `TEXT` → `Enter`. The trailing `Enter` submits a real user turn; the leading `Esc` (as in `member ping` / inline previews) keeps it from blindly confirming a pending permission prompt." — the sequence (the scannable part) is one line, followed by two sentences of rationale. The `--shell` row does the same with a parenthetical cross-reference.
- **Proposed remedy**: keep the table with the sequence and follow-up reduced to their scannable form (`Esc` → settle → `TEXT` → `Enter` / `! TEXT` → `Enter` / `None` / `cafleet member ping required`), and move the two rationale sentences (why the `Esc`, why no `Esc` before `!`) to prose immediately beneath the table.
- **Why**: the comparison the table exists to support — plain vs `--shell` keystrokes — is currently buried behind explanatory prose inside the cells, so the reader reads paragraphs to extract two short sequences.
- **Priority**: Low

### M3. Error Messages — contract strings mixed with rationale prose in the same cell

- **Location**: `## Error Messages`, specifically the rows for "`message send` / `message broadcast` / … with an acting member id … that is not in `--fleet-id`" and "`member prompt` with `\n` or `\r`"
- **Current form**: table cells carrying multi-clause explanations appended to the contract string — "…`Error: member <member-id> is not in fleet <fleet-id>.` (exit 1) — the fleet-membership gate runs before any read/write operation, and also fires for an unknown id." and "…(exit 2; checked first, against the original text — a `\"\\n\"`-only input raises this error, not the empty-text one)".
- **Proposed remedy**: with the `Exit` column extracted (F2), give the table a fourth `Notes` column and move these trailing clauses there, leaving the `Error message` column holding only the verbatim string. Rows with no note leave the cell empty.
- **Why**: the message column is the one thing a reader greps this table for; interleaving prose into it defeats a column-wide visual scan and makes the copy-pasteable contract string harder to isolate.
- **Priority**: Low
