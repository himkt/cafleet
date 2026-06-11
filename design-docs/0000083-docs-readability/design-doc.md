# Documentation Readability (docs/ + README.md + zensical.toml)

**Status**: Approved
**Progress**: 24/24 tasks complete
**Last Updated**: 2026-06-11

## Overview

Designs 0000079 (SSOT map, necessity test, symbol strip) and 0000081 (how-to layer, troubleshooting, core terms, worked examples) made the docs lean and friendly; this design makes them **correct, consistent, and elegant**. It fixes six verified doc/source divergences, removes the agent-addressed register that still leaks into two human-facing pages, applies an aggressive readability trim to `spec/cli-options.md` (rationale, internals, and negative-space assertions out; every observable contract preserved), and runs a site-wide consistency pass over page titles, nav labels, icons, placeholders, and the `zensical.toml` boilerplate. Documentation-only: changes are confined to `README.md`, `docs/`, and `zensical.toml` — `skills/` and source code are untouched.

## Success Criteria

- [x] All six stale facts are fixed and verified by greps: `grep -rn "TASK_STATE" docs/` empty; `grep -rni "upsert" docs/` empty; `grep -rn "artifact" docs/spec/webui-api.md` empty; `docs/spec/message-envelope.md` lists exactly three `status_state` values (`input_required`, `completed`, `canceled`); `grep -rnE '\$FLEET|\$DIRECTOR' docs/` empty; the phrase "poll trigger" no longer appears in `docs/reference/coding-agents/codex.md`'s verification recipe.
- [x] No agent-addressed register remains in `docs/`: `grep -rn "You read this file directly" docs/` empty and `grep -rn "skills/cafleet/SKILL.md" docs/` returns exactly one hit — the intentional pointer in `spec/cli-options.md` § `member send-input` (the canonical three-beat workflow reference); the "Skill-file split" row in `concepts/token-reduction.md` is rewritten without literal skill paths (see Adjacent trims). The codex/opencode pages read as operator documentation throughout. File paths are unchanged. (The `skills/cafleet/reference/exec-routing.md` pointer in `concepts/bash-routing.md`, kept by 0000079, does not match this grep and stays.)
- [x] `spec/cli-options.md`: the `--fleet-id` / identity-flag requirements are stated **only** in the Subcommand summary table (the two "Subcommands that (do NOT) require --fleet-id" lists and the three per-command lists under "Agent ID" are gone); no Click exception class names, no `json.dumps` signature, no regex, no SQL expressions, no index names, and no benchmark targets remain; every flag table, output shape, error string, and exit code survives, including the full Error Messages table.
- [x] Every page in `docs/` carries an `icon:` frontmatter, and every page that has its own nav label carries an H1 matching that label (sentence case) — including the two extra retitles (`get-started/contributing.md`, `concepts/bash-routing.md`); `docs/index.md` has no nav label and is exempt from the title rule; no two nav labels are identical (the Specification sub-section is renamed so "Coding agents" appears once, on the Concepts page).
- [x] `zensical.toml` contains no starter-template tutorial comments, no "Read more:" link comments, and no commented-out template options; the effective configuration is unchanged except the one nav-label rename.
- [x] Leftover placeholders are standardized: `grep -rn '<s>\|<sid>\|<r>' docs/` empty; no `$`-prefixed prompt lines remain in command examples.
- [x] The WebUI theming paragraph (light/dark themes, `localStorage`, brand violet accent) is removed from `concepts/overview.md`.
- [x] Sample output blocks are cosmetically clean (aligned columns) even where the real CLI output mis-aligns — per the user's "docs-only, hide ugly output" decision. Field names and values still match the shapes in `spec/cli-options.md`.
- [x] `mise //:docs-build` passes with every nav entry and cross-reference resolving.
- [x] Per `removal.md`: the repository reads as if the removed content never existed — no deprecation notices, no "previously…" phrasing, no design-doc back-pointers in user docs.

---

## Background

0000079 (completed 2026-06-09) established the SSOT map, the necessity test, and the dotted-symbol strip. 0000081 (completed 2026-06-11) added the how-to layer, troubleshooting, Core terms, worked examples with the sample-id cast (fleet `1`, Director `2`, Administrator `3`, members `4`+, tasks `10`+), and the Documentation style section in `contributing.md`. This design builds on both and must not contradict them.

Remaining gaps, each verified on 2026-06-11:

1. **Doc/source divergences** (verified against `cafleet/src/cafleet/`): `data-model.md` names a `TASK_STATE_INPUT_REQUIRED` enum value that exists nowhere in source (real `status_state` values: `input_required`, `completed`, `canceled` — `broker.py:831/954/1087`); `message-envelope.md` lists a fourth `failed` state that does not exist; `webui-api.md` says the `body` field is "extracted from the task's first artifact's first text part" while the code maps `body` straight from the `text` column (`webui_api.py:52`); both spec pages say `created_at` is "preserved across UPSERT" while tasks are insert-only (`broker.py:775-789`, no `ON CONFLICT`); `codex.md`'s verification recipe expects the pane to receive "the poll trigger" while the auto-fire path keystrokes a 2-line inline preview (`multiplexer/tmux.py:176`); and both backend verification recipes use `$FLEET` / `$DIRECTOR` shell variables, contradicting the no-shell-vars rule 0000081 published in `contributing.md` § Documentation style.
2. **Register leak**: `docs/reference/coding-agents/{codex,opencode}.md` address the coding agent directly ("**You read this file directly** instead — the spawn prompt tells you to", "Substitute the literal ids handed to you in your spawn prompt") and point to `skills/cafleet/SKILL.md` as "the full broker CLI reference" — violating the audience split (docs/ for humans, skills/ for agents).
3. **Density in `spec/cli-options.md`** (886 lines): design rationale, negative-space assertions, internals that survived the 0000079 dotted-symbol strip (a `json.dumps` call signature, an ANSI-strip regex, SQL `MAX(...)` expressions, index names, a `< 100 ms` benchmark target, Click exception class names), the fleet-id/agent-id requirements stated three to four times, and giant flag-table cells (`--prompt-file`, `--model`, `--name`).
4. **Site inconsistency**: spec/api/reference pages have no icon frontmatter and page titles diverge from nav labels ("CLI Option Specification" vs "CLI options"); the nav label "Coding agents" appears twice; `zensical.toml` is roughly 250 lines of starter-template tutorial comments around ~160 lines of real config; leftover placeholders `<s>` / `<sid>` / `<r>` from the session-id era survive in three pages.

User decisions (relayed 2026-06-11): docs-only scope with **cosmetic output cleanup allowed** (sample outputs may be prettier than real CLI output); the agent-addressed sections are **rewritten for human operators** with file paths unchanged; full title/nav/icon consistency pass with URL moves permitted; `zensical.toml` boilerplate stripped; the `cli-options.md` trim is **aggressive** (option a); all six stale facts fixed; placeholder standardization and the WebUI-theming cut approved; README heading-number removal and a blanket plain-language sentence-rewriting pass **vetoed** (not in scope); acceptance is qualitative criteria + clean build + greps, no line-count targets.

**Move decision**: although URL moves were permitted, `docs/reference/coding-agents/{codex,opencode}.md` stay at their current paths. Five `skills/*` files reference the literal path (`skills/cafleet/SKILL.md:80`, `skills/cafleet/reference/director.md:34,47`, `skills/cafleet-agent-team-monitoring/SKILL.md:52`, `skills/cafleet-my-slidev/SKILL.md:251`, `skills/cafleet-research-report/SKILL.md:508`) and skills are out of scope; the same path-stability constraint underlies the user's A2 answer ("file paths unchanged so spawn prompts still resolve"). The duplicate nav label is resolved by renaming the nav label only.

---

## Specification

### Guiding principles

1. **Preserve 0000079 and 0000081.** The SSOT map, necessity test, sample-id cast, and Documentation style section stand. Where this design deletes a duplicate, the canonical home keeps the full statement.
2. **Observable contract is sacred.** Flags, output shapes, error strings, exit codes, env vars, file paths, and column names all survive. What goes: design rationale, implementation mechanics (function signatures, regexes, SQL, index names, benchmark targets, Click class names), and negative-space assertions that explain design intent. Negatives that prevent a plausible wrong action stay (e.g. "there is no `--force` flag" on `fleet delete`, "no env-var fallback").
3. **Cuts, not rewrites.** The plain-language sentence pass was vetoed: surviving sentences are not reworded except where a deletion forces a splice. README's numbered headings stay.
4. **Human register only.** Every sentence in `docs/` addresses a human developer or operator. Statements about what members/agents do are third person.
5. **Cosmetic output cleanup.** Sample output blocks may be aligned and tidied even where the real CLI output is uglier; the fields and values shown must still match the output shapes documented in `spec/cli-options.md`.

### Stale-fact fixes (verified against source)

| # | File(s) | Current text | Replacement |
|---|---|---|---|
| 1 | `spec/data-model.md` (`tasks.status_state` row) | "TaskState enum value (e.g., `TASK_STATE_INPUT_REQUIRED`)." | "`input_required`, `completed`, or `canceled`." |
| 2 | `spec/message-envelope.md` (`status_state` row) | "`input_required` (queued), `completed` (acked), `canceled` (retracted), `failed` (routing error)." | Drop `failed` — three values only: "`input_required` (queued), `completed` (acked), `canceled` (retracted)." |
| 3 | `spec/webui-api.md` (inbox section) | "The `body` field is extracted from the task's first artifact's first text part. If no text part exists, `body` is `""`." | "The `body` field is the task's `text` column." |
| 4 | `spec/data-model.md` (`tasks.created_at` row) and `spec/message-envelope.md` (`created_at` row) | "first-write only, preserved across UPSERT" / "First-write timestamp; preserved across UPSERT." | "Set at insert time, never updated." (tasks are insert-only) |
| 5 | `reference/coding-agents/codex.md` (verification recipe) | "Expect: codex pane receives the poll trigger and the member ack-loops correctly." | "Expect: the codex pane receives the 2-line inline preview and the member ack-loops correctly." |
| 6 | `reference/coding-agents/codex.md` + `opencode.md` (verification recipes) | `$FLEET` / `$DIRECTOR` shell variables, `# Capture: FLEET=<id>, DIRECTOR=<id>` comments | Literal sample-id cast (fleet `1`, Director `2`, members `4`/`5`) with the standard one-line "your ids will differ" note, per `contributing.md` § Documentation style. |

### Register rewrite (codex.md + opencode.md)

Apply identically to both pages, paths unchanged:

- Rewrite the "cafleet usage from inside a codex/opencode pane" section in operator register. Replace "Codex does not load Claude Code's `Skill()` tool. **You read this file directly** instead — the spawn prompt tells you to." with: "Codex members cannot load Claude Code skills, so their spawn prompt points them at this page instead." (opencode equivalent for `opencode`). Keep the three-command example block.
- Replace "Substitute the literal ids handed to you in your spawn prompt. There is no env-var fallback." with "Members substitute the literal ids from their spawn prompt; there is no env-var fallback."
- Replace the closing pointer "For the full broker CLI reference …, see `skills/cafleet/SKILL.md`." with a link to [CLI options](../../spec/cli-options.md) — the human-facing canonical CLI reference.

### Information-architecture consistency pass

**Titles** (H1 = nav label, sentence case; URLs unchanged):

| File | Current H1 | New H1 |
|---|---|---|
| `spec/cli-options.md` | CLI Option Specification | CLI options |
| `spec/data-model.md` | SQLite Data Model Specification | Data model |
| `spec/message-envelope.md` | Message Envelope Specification | Message envelope |
| `spec/webui-api.md` | WebUI API Specification | WebUI API |
| `reference/coding-agents/codex.md` | Codex Members | Codex members |
| `reference/coding-agents/opencode.md` | Opencode Members | Opencode members |
| `get-started/contributing.md` | Contributing to CAFleet | Contributing |
| `concepts/bash-routing.md` | Bash routing via Director | Bash routing |

**Icons** (add `icon:` frontmatter to the pages that lack it; all names are valid lucide icons):

| File | Icon |
|---|---|
| `spec/data-model.md` | `lucide/table` |
| `spec/message-envelope.md` | `lucide/mail` |
| `spec/cli-options.md` | `lucide/square-terminal` |
| `spec/webui-api.md` | `lucide/globe` |
| `reference/coding-agents/codex.md` | `lucide/bot` |
| `reference/coding-agents/opencode.md` | `lucide/bot` |
| `api/index.md`, `api/broker.md`, `api/config.md`, `api/coding-agent.md`, `api/multiplexer.md` | `lucide/code` |

**Nav** (`zensical.toml`): rename the Specification sub-section label `"Coding agents"` → `"Coding-agent backends"` so the label "Coding agents" appears exactly once (the Concepts page). No path changes.

**README**: add language hints to the five unlabeled fenced blocks, matching `install.md` / `quickstart.md` style: `bash` for the § 3.1(a) gh-skill block and the § 3.1(c) codex marketplace block (shell commands), `text` for the § 3.1(b) Claude Code marketplace slash-commands and the two prompt blocks in § 4.1 / § 4.2. Heading numbers stay (vetoed).

### `zensical.toml` boilerplate strip

Configuration values are unchanged except the one nav-label rename above. Delete:

- The banner comment block at the top and every "Read more: https://zensical.org/..." tutorial comment.
- Every commented-out template option: `#extra_css`, `#extra_javascript`, `#variant`, `#custom_dir`, `#favicon`, `#logo = "assets/logo.png"` (and the two tutorial comment banners around it), the `[project.theme.icon]` block, the `[project.theme.font]` block, the `[[project.extra.social]]` block, and the commented-out entries inside `features` (`#"content.action.edit"`, `#"content.action.view"`, `# "header.autohide"`, `# "navigation.expand"`, `#"navigation.instant.progress"`, `#"navigation.prune"`, `#"navigation.tabs"`, `#"navigation.tabs.sticky"`, `# "toc.follow"`, `#"toc.integrate"`).
- The multi-line per-feature explanations inside `features` — the feature strings themselves all stay.

Keep, as single-line comments at most: the section separators that aid scanning, the mermaid custom-fence stanza as-is, and one line above the mkdocstrings block ("resolves `:::` directives against `cafleet/src`").

### `spec/cli-options.md` — aggressive trim (per-section directives)

Everything not listed below is kept verbatim. The Error Messages table, every flag table, every output shape (text + JSON), and every exit-code table survive (with the in-cell cuts noted).

**Deduplicate the requirement statements** — the Subcommand summary table is the single home for "needs `--fleet-id`" and the identity flag:

- Delete the sections "Subcommands that require `--fleet-id`" and "Subcommands that do NOT require `--fleet-id`" (including the `--version` eager-option paragraph — the observable fact already lives in the Global Options `--version` row). Keep the "Create a fleet first" snippet by folding it to the end of the Option Source Matrix section.
- Under "Agent ID (`--agent-id`)": delete the three lists ("Commands that require `--agent-id`", "Commands that do NOT…", and the `--member-id` list). Keep the opening paragraph (per-subcommand option, `int` typing, ids pasted in full) and add one sentence: "Which subcommand takes which identity flag is in the [Subcommand summary](#subcommand-summary)."
- Global Options `--fleet-id` row: replace the subcommand enumeration with "see the [Subcommand summary](#subcommand-summary)"; keep the int typing, the error string, and the silently-accepted-and-ignored fact.

**Strip internals** (observable behavior stays, mechanics go):

- `--json` row: drop the `json.dumps(..., separators=(",",":"), ensure_ascii=False)` signature → "compact single-line JSON; non-ASCII (like the `…` suffix) is emitted as UTF-8, not escaped."
- `--version` row: drop "Sourced from the installed package metadata via `importlib.metadata`."
- `member capture` `--ansi` row: drop the `re.sub(r'\x1b\[[0-?]*[ -/]*[@-~]', '', text)` regex → "ANSI escape sequences are stripped and carriage-return redraw fragments cleaned up."
- `member list --activity`: in the flag cell, drop "The aggregation filters `Task.type != 'broadcast_summary'` for the `last_ack` proxy (mirrors `poll_tasks`)" and "Primary inputs to the Director's `/loop` monitoring tick" → "broadcast summary rows are excluded from `last_ack`." In the output subsection, replace the `MAX(tasks.status_timestamp)` / index-name / "benchmark target is < 100 ms at 1k messages" paragraph with plain definitions: `last_sent` = the member's most recent outgoing message; `last_recv` = its most recent delivery; `last_ack` = the most recent delivery it acknowledged (broadcast summaries excluded); `idle` = wall-time since the latest of `last_sent` / `last_recv`.
- `server` section: in the env-var table drop the "Wired via `Field(validation_alias=…)`" cells (keep variable ↔ settings-field ↔ default); in Behavior replace the `uvicorn.run("cafleet.server:app", …)` bullet with "Runs uvicorn with its defaults — no reload, no custom workers, no custom log level"; delete the bullet "The `cafleet server` handler does not perform any disk check itself; the dist-directory warning is entirely owned by the app factory" (keep the warning text and that all three entry points emit it).
- Error Messages table and per-subcommand validation tables: drop every `click.UsageError` / `click.ClickException` / "Click built-in" / `IntRange(1, 3)` class mention; keep the error string and the exit code. (E.g. "Click `IntRange(1, 3)` built-in (exit 2)" → "values outside 1–3 are rejected (exit 2)".)
- Both `--coding-agent` cells (fleet create, member create): drop the "Help text: `…` (Click appends `[default: claude]` automatically.)" sentences.

**Cut rationale and negative-space assertions**:

- `--full` semantics intro: keep the first sentence; drop "deliberately one flag rather than `--full-envelope` / `--full-recipients` / `--full-card` / `--full-body` variants".
- The "never adds per-recipient envelopes or a `recipient_ids` list" fact is stated three times (twice in the `--full` table, once in Message Body Truncation): keep it once, in the `--full` semantics `message broadcast` row.
- `fleet create` hidden `--full` output: drop "(preserves backward-compatible scripts that parse only the first line)".
- `fleet create`: the operator-declares-the-backend explanation appears in the `--coding-agent` cell AND in the paragraph after the JSON block — keep the post-JSON paragraph version once, shrink the cell to one line + link to [Coding agents](../concepts/coding-agents.md).
- `doctor`: drop "Intended as the home for future health checks (DB connectivity, orphan-placement scans, etc.); today it covers tmux metadata only."
- `member capture`: drop "(down from 80)" and the calibration paragraph ("The default is calibrated against per-tick cost … the default is bumped to 50.").
- `member exec` / `member ping` JSON-output notes: drop "No `action` field — the subcommand name IS the action." (both) and "No `polled` field — failures surface via exit 1, not via a `polled: false` field." → keep "failures surface via exit 1". Drop "There is no operator-controlled keystroke body to validate."
- `--quiet` row in Message Body Truncation: keep "Mutually exclusive with `--full`"; drop "; the two are not expected to be combined".
- Message Body Truncation: drop "Follows the `CAFLEET_`-prefixed convention already used by …" (rationale); shorten the broadcast paragraph to two sentences + a link to the `--full` semantics row; in the closing paragraph drop "in this release" (keep the WebUI-renders-full-bodies fact as one sentence).
- "Member targeting and key delivery" § Literal key delivery: keep the observable guarantee (each key sequence is delivered literally — shell meta, key names, and multi-byte characters arrive as plain characters; no shell ever evaluates the text); drop the per-invocation `-l` flag mechanics sentence and the two per-subcommand "see … for the per-invocation `-l` rationale" cross-references.

**Giant cells → prose** (`member create` flag table — every cell becomes one line; details live in the named subsection or canonical page):

- `--name`: "Display name of the new member." + link to [Known asymmetries](../concepts/coding-agents.md#known-asymmetries-intentional-non-goals) (pane-title canonical home; the current cell re-explains it — an SSOT violation).
- `--coding-agent`: backend choice + the `binary not found` error string + link to [Opencode members](../reference/coding-agents/opencode.md) for the preset note.
- `--model`: "Model forwarded to the backend binary's `--model` flag; omitted by default." + link to [Model selection](../concepts/coding-agents.md#model-selection) (canonical validation rules; the error string stays in Error Messages). Delete the cell's duplicated validation prose.
- `--prompt-file`: "Absolute path to a UTF-8 file used as the spawn prompt; mutually exclusive with the positional prompt." Move the unique facts (verbatim read, same `str.format()` substitution, error strings pointer) into the existing "Spawn-prompt input modes" subsection.
- In "Spawn-prompt input modes": keep the four-row mode table; replace the audit-artifact paragraph ("The `--prompt-file` path is BOTH the spawn input AND the permanent audit artifact. CAFleet-native team skills render … `<BASE>/prompts/…`") with the operational fact only: inline prompts beyond a few KB exceed tmux's argv ceiling (`tmux command failed: command too long` rolls back the registration) — use `--prompt-file` for long prompts.
- After the "Spawn command per backend" table (which stays — it is the only place documenting exact argv incl. `--model` placement): shrink the following paragraph (dontAsk / auto-resolve / bash-via-Director re-explanation) to one sentence + links to [Bash routing](../concepts/bash-routing.md) and the two backend pages.
- `member send-input` § Director-side usage pattern: shrink to two sentences — the three-beat workflow (capture → AskUserQuestion → send-input) is canonical in `skills/cafleet/SKILL.md`; this page documents only the CLI surface. (After the token-reduction rewrite below, this is the only `skills/cafleet/SKILL.md` mention left in docs/; the `skills/cafleet/reference/exec-routing.md` pointer in `concepts/bash-routing.md`, kept by 0000079, is unaffected.)

### Adjacent trims (same approved categories, itemized)

- `spec/data-model.md`: state the "ids are never reused (`sqlite_sequence` high-water mark)" fact once in the SQL Schema intro; drop the per-row repeats in the `fleets.fleet_id`, `agents.agent_id`, and `tasks.task_id` cells (and the "(hypothetically)" aside). Delete the paragraph after the `tasks` index table ("`status_state` and `status_timestamp` are promoted to columns so … directly from the index.") — it restates the index table and the row notes.
- `spec/message-envelope.md`: in the JSON-output table, drop the `json.dumps(...)` signature (same replacement wording as cli-options).
- `concepts/overview.md`: delete the WebUI theming paragraph ("The UI ships light and dark themes … focus rings.") in full (approved D1).
- `concepts/token-reduction.md`: delete the closing "In short, …" paragraph (duplicates the table). In the table rows, drop the remaining internals: "(the `last_ack` proxy filters out broadcast-summary rows). Existing task indexes cover the join." → "broadcast summary rows are excluded from `last_ack`"; "with no Director-name placeholder" (slim-spawn-prompt row); "core is ≤ 350 lines" → "compact core". Rewrite the "Skill-file split" row without literal skill paths: the core cafleet skill stays compact (identity + poll/send/ack); director-only, broadcast, exec-routing, recovery, and output-flag content loads from on-demand reference files. The techniques themselves all stay.
- `spec/webui-api.md`: fix the cross-reference "See `docs/spec/data-model.md` for the accompanying design-debt note." → "See [Data model](data-model.md) § ACK timestamp inference."

### Placeholder + sample-output cleanup

- `spec/cli-options.md`: in the `member list --activity` example, `cafleet --fleet-id <s> member list --activity` → `cafleet --fleet-id 1 member list --activity` (sample cast), and drop the `$ ` prompt prefix (the only example using one); in the `member ping` intro prose and key-sequence table, the generic command shapes keep placeholders but standardized — `<s>` / `<sid>` → `<fleet-id>` and `<m>` / `<member_id>` → `<member-id>` (matching the page's own `member ping` snippet); `fleet <sid>` in the Error Messages table → `fleet <fleet-id>`.
- `spec/message-envelope.md`: replace `<r>` with `<my-agent-id>` in the two poll example headers (`cafleet --json message poll --agent-id <my-agent-id>`), consistent with the truncation examples in `cli-options.md`.
- `concepts/tmux-push.md`: replace `<r>` with `<recipient-id>` in the prose and the sequence diagram (`--to <recipient-id>`, `member ping --member-id <recipient-id>`).
- `spec/cli-options.md` `fleet list`: replace the bare 40-char-wide header line with a compact header + one sample row using the sample cast (cosmetic cleanup; the real CLI pads wider).
- `how-to/mixed-backend-team.md`: align the `backend` column in the `member list` sample block so the `opencode` row lines up (cosmetic cleanup; the real CLI mis-aligns at 7 chars).

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.
> After all edits, run `mise //:docs-build` to confirm nav + cross-references resolve.

### Step 1: Stale facts + register rewrite

- [x] `docs/spec/data-model.md` — fix the `status_state` enum example (fact 1) and the `created_at` UPSERT wording (fact 4). <!-- completed: 2026-06-11T11:33 -->
- [x] `docs/spec/message-envelope.md` — drop the nonexistent `failed` state (fact 2) and the UPSERT wording (fact 4). <!-- completed: 2026-06-11T11:33 -->
- [x] `docs/spec/webui-api.md` — replace the artifact/text-part sentence with the `text`-column fact (fact 3); fix the design-debt cross-reference. <!-- completed: 2026-06-11T11:33 -->
- [x] `docs/reference/coding-agents/codex.md` — fix the poll-trigger expectation (fact 5); rewrite the verification recipe with literal sample ids (fact 6); rewrite the "cafleet usage from inside a codex pane" section in operator register and re-point the SKILL.md reference to CLI options. <!-- completed: 2026-06-11T11:33 -->
- [x] `docs/reference/coding-agents/opencode.md` — rewrite the verification recipe with literal sample ids (fact 6); rewrite the "cafleet usage from inside an opencode pane" section in operator register and re-point the SKILL.md reference to CLI options. <!-- completed: 2026-06-11T11:33 -->

### Step 2: `spec/cli-options.md` aggressive trim

- [x] Deduplicate the requirement statements: delete the two `--fleet-id` list sections and the three "Agent ID" lists; re-point the Global Options `--fleet-id` row at the Subcommand summary; fold the "Create a fleet first" snippet into the Option Source Matrix section. <!-- completed: 2026-06-11T11:45 -->
- [x] Strip internals: `json.dumps` signature, `importlib.metadata`, ANSI regex, `--activity` SQL/index/benchmark paragraph and cell, server `validation_alias` cells and `uvicorn.run` bullet, every Click class-name mention (error strings and exit codes preserved), both "Help text: …" sentences. <!-- completed: 2026-06-11T11:45 -->
- [x] Cut rationale / negative-space assertions per the Specification list (the `--full` variants aside, the triple-stated broadcast negative kept once, hidden-`--full` backward-compat note, doctor future-health-checks, capture calibration, exec/ping "No X field" notes, truncation-section rationale, literal-key-delivery `-l` mechanics). <!-- completed: 2026-06-11T11:45 -->
- [x] Restructure the `member create` flag table to one-line cells (`--name`, `--coding-agent`, `--model`, `--prompt-file`); move unique `--prompt-file` facts into Spawn-prompt input modes; replace the audit-artifact paragraph with the argv-ceiling fact; shrink the post-spawn-table paragraph and the Director-side usage pattern to the specified lengths. <!-- completed: 2026-06-11T11:45 -->
- [x] Placeholder + output cleanup on this page: the `--activity` example to the sample cast (`<s>` → `1`, drop the `$ ` prefix), the `member ping` prose and key-sequence table to `<fleet-id>` / `<member-id>`, the Error Messages `fleet <sid>` → `fleet <fleet-id>`, compact `fleet list` header + sample row. <!-- completed: 2026-06-11T11:45 -->

### Step 3: Adjacent trims

- [x] `docs/spec/data-model.md` — dedupe the never-reused/`sqlite_sequence` fact to the intro; delete the post-index restatement paragraph. <!-- completed: 2026-06-11T11:48 -->
- [x] `docs/spec/message-envelope.md` — drop the `json.dumps` signature from the JSON-output table; replace `<r>` with `<my-agent-id>` in the two poll example headers. <!-- completed: 2026-06-11T11:48 -->
- [x] `docs/concepts/overview.md` — delete the WebUI theming paragraph. <!-- completed: 2026-06-11T11:48 -->
- [x] `docs/concepts/token-reduction.md` — delete the "In short" paragraph; strip the three remaining in-row internals; rewrite the "Skill-file split" row without literal skill paths. <!-- completed: 2026-06-11T11:48 -->
- [x] `docs/concepts/tmux-push.md` — standardize `<r>` → `<recipient-id>` (prose + diagram). <!-- completed: 2026-06-11T11:48 -->
- [x] `docs/how-to/mixed-backend-team.md` — align the `backend` column in the `member list` sample block. <!-- completed: 2026-06-11T11:48 -->

### Step 4: Titles, icons, nav, README

- [x] Retitle the eight H1s per the titles table (four spec pages, two backend pages, contributing, bash-routing). <!-- completed: 2026-06-11T11:54 -->
- [x] Add `icon:` frontmatter to the 11 pages per the icons table. <!-- completed: 2026-06-11T11:54 -->
- [x] `zensical.toml` — rename the Specification sub-section label to "Coding-agent backends"; strip the starter-template boilerplate per the Specification (config values otherwise unchanged). <!-- completed: 2026-06-11T11:54 -->
- [x] `README.md` — add language hints to the five unlabeled fenced blocks (`bash` for gh-skill and codex marketplace, `text` for the Claude Code marketplace and the two prompt blocks). <!-- completed: 2026-06-11T11:54 -->

### Step 5: Verification

- [x] Run `mise //:docs-build`; confirm a clean build with every nav entry and cross-reference resolving. <!-- completed: 2026-06-11T11:56 -->
- [x] Stale-fact greps: `grep -rn "TASK_STATE" docs/`, `grep -rni "upsert" docs/`, `grep -rn "artifact" docs/spec/webui-api.md`, `grep -rnE '\$FLEET|\$DIRECTOR' docs/` all empty; `message-envelope.md` lists exactly three states; no "poll trigger" in codex.md's recipe. <!-- completed: 2026-06-11T11:56 -->
- [x] Register + placeholder greps: `grep -rn "You read this file directly" docs/` empty; `grep -rn "skills/cafleet/SKILL.md" docs/` shows only the cli-options send-input pointer; `grep -rn '<s>\|<sid>\|<r>' docs/` empty. <!-- completed: 2026-06-11T11:56 -->
- [x] Spot-check cli-options.md: requirements stated only in the Subcommand summary; no `click.`, `Click built-in`, `IntRange`, `json.dumps`, `re.sub`, `MAX(`, `idx_tasks`, or benchmark-target strings remain (`grep -nE 'click\.|Click built-in|IntRange|json\.dumps|re\.sub|MAX\(|idx_tasks|100 ms' docs/spec/cli-options.md` empty); Error Messages table and all flag tables intact. <!-- completed: 2026-06-11T11:56 -->

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-06-11 | Initial draft |
| 2026-06-11 | Reviewer round 1: corrected the task count (24); resolved the SKILL.md grep by rewriting the token-reduction "Skill-file split" row; extended the titles table with contributing.md and bash-routing.md and exempted `docs/index.md`; covered all five unlabeled README blocks with `bash`/`text` hints; added `#logo` to the zensical.toml delete list; scoped the "one skills/ pointer" claim to `skills/cafleet/SKILL.md`; added the message-envelope `<r>` and cli-options `member ping` placeholder directives; extended the spot-check grep with `Click built-in|IntRange`. |
