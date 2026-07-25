# Findings: docs entry points, how-to, api

## Summary

Ten files read in full; **7 tabulate-me findings** and **0 table-misuse findings**.
By file: `docs/quickstart.md` — 2 (F1, F2); `docs/contributing.md` — 3 (F3, F4, F5);
`docs/how-to/design-doc-development.md` — 1 (F6); `docs/how-to/mixed-backend-team.md` — 1 (F7);
`docs/how-to/use-the-webui.md` — 0; `docs/index.md` — 0; `docs/api/broker.md` — 0;
`docs/api/coding-agent.md` — 0; `docs/api/config.md` — 0; `docs/api/multiplexer.md` — 0.

The dominant pattern is **the per-backend / per-workflow repeated sub-heading**: the same
three items (`claude` / `codex` / `opencode`, or create / interview / execute) are written
out three times, each as its own heading or paragraph restating the same attribute slots —
where the config lives, what `cafleet setup` installs, whether manual configuration is
needed; or which workflow, which team, which input. The reader who wants "what do I do for
codex?" or "which workflow takes a path?" has to read all three prose blocks and diff them
mentally. A secondary pattern is the **bold-key bullet list** (`contributing.md` § Tech
stack), which is already a two-column table wearing bullet clothes.

Files that yielded zero findings, and why:

- **`docs/index.md`** — a landing page: a video embed, a two-paragraph pitch, one button.
  The three backends do appear as a parallel set in the pitch sentence, but that sentence's
  value is its narrative flow, and breaking a landing-page pitch into a grid would cost more
  than it buys. No finding.
- **`docs/api/config.md`**, **`docs/api/multiplexer.md`** — generated reference stubs: a
  front-matter block, a short orientation paragraph, and the mkdocstrings `:::` directive.
  Nothing enumerable. No finding.
- **`docs/api/coding-agent.md`** — same stub shape. It names the four `str.format`
  placeholders in one sentence, but they carry only a name (no second attribute stated
  there), and the fact is owned elsewhere. No finding.
- **`docs/api/broker.md`** — already has the right table (§ Package layout, `Submodule |
  Contents`), and it is complete: every submodule named in the surrounding prose has a row.
  The § Re-export contract prose that follows is a single rule with one exception, correctly
  left as prose. No finding.
- **`docs/how-to/use-the-webui.md`** — four short sections that are genuinely sequential
  task steps (start the server → pick a fleet → send → inspect), each one to two sentences.
  The UI regions (sidebar / timeline / bottom input) are named once each across different
  steps, never as a parallel set. No finding.

Considered and deliberately **rejected** (so the Director does not re-open them):

- `contributing.md` § Documentation style — five bold-key bullets, but each rule's body is
  one to three sentences of free-form guidance. Tabulating it would produce exactly the
  table-misuse pattern (#5): a grid whose cells are paragraphs. Leave as bullets.
- `design-doc-development.md` § Invocation syntax — three backends in one parenthetical,
  but each carries a single attribute (its invocation surface). Below the 2+ attribute bar.
- `mixed-backend-team.md` § Prerequisites — two bullets. Below the three-item bar.
- `quickstart.md` § Raw CLI walkthrough and `mixed-backend-team.md`'s tear-down sequence —
  ordered command/output narratives. Correct as they are; see F7 for the one part of the
  latter that is parallel rather than sequential.

## Tabulate-me findings

### F1. docs/quickstart.md — ## Configure — per-backend configuration deltas buried in three repeated sub-headings

- **Location**: `docs/quickstart.md`, `## Configure` (anchor `#configure`), spanning its
  three child sub-headings `### Claude Code` (`#claude-code`), `### Codex` (`#codex`), and
  `### Opencode` (`#opencode`).
- **Current form**: repeated sub-headings, one per backend, each pairing an admonition or
  paragraph with a snippet. Opens: *"CAFleet is designed to run inside a coding agent
  without per-command permission prompts — each backend has a different config file and
  permission system…"*, then `### Claude Code` → *"Typically the config entries below go in
  `~/.claude/settings.json`."*; `### Codex` → *"Typically the config entries below go in
  `~/.codex/config.toml`."*; `### Opencode` → *"No manual configuration is required.
  `cafleet setup` installs opencode's `cafleet` agent preset to…"*. The intro sentence
  explicitly promises a per-backend delta ("each backend has a different config file and
  permission system") and then makes the reader reconstruct that delta from three prose
  blocks.
- **Proposed table columns**: `Backend | Config file | Manual configuration | Installed by
  cafleet setup | Reference`
- **Row keys**: `claude` (Claude Code); `codex` (OpenAI Codex CLI); `opencode`. Cell
  content, respectively — Config file: `~/.claude/settings.json`, `~/.codex/config.toml`,
  none; Manual configuration: the `permissions.allow` / `permissions.ask` entries below,
  the `[sandbox_workspace_write]` entries below, none required; Installed by
  `cafleet setup`: the skills, the skills plus `~/.codex/rules/cafleet.rules`, the skills
  plus the `cafleet` agent preset at `~/.opencode/agents/cafleet.md`; Reference: the
  existing per-backend links to Coding-agent backends (the `#cafleet-rules-file` and
  `#opencode` anchors) — leave the claude row's reference cell as the sub-section below.
  All of these are user-machine `~/…` paths, which the user-facing-docs rule explicitly
  exempts, so no repository path enters the table.
- **Prose that must remain**: the intro sentence (rewritten to hand off to the table); both
  code snippets (the JSON permissions block and the TOML sandbox block) with their
  sub-headings, since a snippet cannot live in a cell; and both rationale paragraphs — why
  `Bash(cafleet *)` is one allow-everything pattern and why `member prompt` sits in `ask`;
  and why `network_access = true` is required (the local-socket / `Operation not permitted`
  explanation) and what `writable_roots` grants. Those are one-decision rationales, not
  enumerable attributes.
- **Why**: the reader arrives knowing which backend they run and wants only their own row;
  today they must read all three sections to learn that opencode needs nothing and codex
  needs a sandbox change.
- **Priority**: High

### F2. docs/quickstart.md — ## Install — prerequisites list without its version/alternatives columns

- **Location**: `docs/quickstart.md`, `## Install` (anchor `#install`), the
  "Prerequisites:" bullet list.
- **Current form**: bullets. Opens: *"Prerequisites:"* / *"- Python 3.12+"* / *"- A
  terminal multiplexer — tmux or herdr (see Multiplexer backends)"* / *"- At least one of:
  `claude` (Claude Code), `codex` (OpenAI Codex CLI), or `opencode`"*. Each bullet already
  carries three attributes in run-on form — what is required, which versions or
  alternatives satisfy it, and (for two of the three) where to read more — but only the
  multiplexer bullet exposes its link, and only the coding-agent bullet exposes that one of
  several is enough.
- **Proposed table columns**: `Requirement | Accepted | Notes`
- **Row keys**: `Python` (Accepted: 3.12 or newer; Notes: —); `Terminal multiplexer`
  (Accepted: tmux or herdr; Notes: auto-detected — link to Multiplexer backends);
  `Coding agent` (Accepted: `claude` (Claude Code), `codex` (OpenAI Codex CLI), or
  `opencode`; Notes: at least one; a mixed fleet may use all three — link to Coding agents).
- **Prose that must remain**: the "Prerequisites:" lead-in, and the `uv tool install` /
  `cafleet setup` code block that follows, unchanged.
- **Why**: three requirements with genuinely different satisfaction rules (an exact floor,
  a choice of two, an at-least-one-of-three) read as one uniform list today; the columns
  make the difference visible at a glance on the page a new user hits first.
- **Priority**: Medium

### F3. docs/contributing.md — ## Tech stack — bold-key bullets that are already a two-column table

- **Location**: `docs/contributing.md`, `## Tech stack` (anchor `#tech-stack`).
- **Current form**: bullets in `**Key:** value` shape — the canonical repeated
  per-item pattern. Opens: *"- **Language:** Python 3.12+, managed with uv"* / *"-
  **Server:** FastAPI (admin WebUI)"* / *"- **Database:** SQLAlchemy 2.x (sync `pysqlite`
  driver) + SQLite"* / *"- **CLI:** click"* / *"- **Admin frontend:** Vite + Bun (SPA
  served at `/`)"* / *"- **Task runner:** mise"*. Six parallel items, each carrying a
  concern, a technology (with its link), and — for four of the six — a parenthetical
  qualifier that is a third attribute hiding inside the value.
- **Proposed table columns**: `Concern | Technology | Notes`
- **Row keys**: `Language` (Python 3.12+ / managed with uv); `Server` (FastAPI / admin
  WebUI only); `Database` (SQLAlchemy 2.x + SQLite / sync `pysqlite` driver); `CLI` (click
  / —); `Admin frontend` (Vite + Bun / SPA served at `/`); `Task runner` (mise / —). Keep
  every existing external link on the technology cell.
- **Prose that must remain**: none — the section is the list.
- **Why**: the parentheticals currently hide the one thing a new contributor scans for —
  which layer a technology actually applies to — and this page already uses a table for
  § Project structure, so the two orientation sections would finally match.
- **Priority**: High

### F4. docs/contributing.md — ## Contributing changes — the three skill workflows as a numbered list with restated attributes

- **Location**: `docs/contributing.md`, `## Contributing changes` (anchor
  `#contributing-changes`), the three-item numbered list.
- **Current form**: a numbered list whose items are parallel rather than sequential, each
  restating the same attribute slots. Opens: *"1. Invoke the `cafleet-design-doc` skill
  (create workflow) with a one-line description — orchestrates a Director / Drafter /
  Reviewer team to produce a design doc under `design-docs/NNNNNNN-<slug>/`."*, then
  *"2. Invoke the `cafleet-design-doc` skill (interview workflow) with the path…"*, then
  *"3. Invoke the `cafleet-design-doc` skill (execute workflow) with the path…"*. Every
  item repeats "Invoke the `cafleet-design-doc` skill (X workflow) with <input> — <team> —
  <output>": one sentence shape, three fillings.
- **Proposed table columns**: `Workflow | What you pass | Team | What you get`
- **Row keys**: `create` (a one-line feature description / Director + Drafter + Reviewer /
  a design doc under `design-docs/NNNNNNN-<slug>/`); `interview` (the
  `design-docs/NNNNNNN-<slug>` path / Director + Analyzer / the doc annotated with
  `COMMENT(user-relay)` markers that create's resume mode absorbs); `execute` (the
  `design-docs/NNNNNNN-<slug>` path / Director + Programmer + Tester + optional Verifier,
  plus a fresh Reviewer at review time / a TDD-cycle implementation pass). Paths are the
  deliverable-location form the user-facing-docs rule exempts, and this page is exempt in
  any case.
- **Prose that must remain**: the lead-in ("CAFleet uses its own design-doc-driven
  development skills…"), one sentence stating that the workflows are normally run in this
  order (the ordering the numbers currently carry must not be lost), and the closing
  pointer to the coding-agent skill documentation and the `design-docs/` examples.
- **Why**: the reader's actual question is "which workflow do I invoke, and what do I pass
  it?" — a column answers it; three same-shaped sentences make them diff prose to find out.
- **Priority**: Medium
- **Cross-page note for the Director**: F4 and F6 describe the *same three workflows* from
  the contributor and user angles, and the two pages' attribute sets overlap almost
  completely. If both are tabulated, use the same column vocabulary in both; the page's own
  § Documentation style "SSOT: one fact, one home" rule may argue for one table plus a link
  rather than two. That is a scoping call for the Director, not something I resolved.

### F5. docs/contributing.md — ## Development — the mise task catalog carried as trailing code comments

- **Location**: `docs/contributing.md`, `## Development` (anchor `#development`), the
  `bash` block.
- **Current form**: a code block whose trailing comments are doing table work. Opens:
  *"mise //cafleet:install    # editable uv tool install of the cafleet CLI"*, and
  continues *"mise //cafleet:lint       # ruff check + ruff format --check"*,
  *"mise //cafleet:format     # ruff check --fix + ruff format"*,
  *"mise //cafleet:typecheck  # ty"*, *"mise //cafleet:test       # pytest"*,
  *"mise //admin:build        # build the WebUI (required before / is served)"*,
  *"mise //admin:dev          # WebUI dev server (Vite)"*,
  *"mise //admin:install      # reinstall WebUI deps from the committed lockfile"*. The
  block mixes two different things: a genuine first-time setup **sequence** (clone → cd →
  `uv-sync` → `install` → `setup`) and a **catalog** of seven independent tasks a
  contributor picks from by name, each with a command, an underlying tool, and a purpose.
- **Proposed table columns**: `Task | Runs | When you need it`
- **Row keys**: `mise //cafleet:lint` (ruff check + ruff format --check); `mise
  //cafleet:format` (ruff check --fix + ruff format); `mise //cafleet:typecheck` (ty);
  `mise //cafleet:test` (pytest); `mise //admin:build` (Vite build — required before `/`
  is served); `mise //admin:dev` (Vite dev server); `mise //admin:install` (reinstall
  WebUI deps from the committed lockfile, `--frozen-lockfile`).
- **Prose that must remain**: the "Clone the repo and use mise for all common tasks:"
  lead-in and a **shortened code block** keeping only the ordered first-time setup
  (`git clone` → `cd` → `mise //:uv-sync` → `mise //cafleet:install` → `cafleet setup
  --skip …`), which stays copy-pasteable and must not be tabulated; plus the paragraph
  about editing `admin/package.json` and regenerating `admin/bun.lock`, which explains why
  `mise //admin:install` cannot update the lockfile.
- **Why**: separating the run-once sequence from the pick-one-by-name catalog lets a
  contributor find the test or format command without reading a setup script, and the
  "Runs" column surfaces the underlying tool that today hides in a comment.
- **Priority**: Medium

### F6. docs/how-to/design-doc-development.md — ## Prompts — three workflows as three repeated prompt/paragraph pairs

- **Location**: `docs/how-to/design-doc-development.md`, `## Prompts` (anchor `#prompts`).
- **Current form**: three repeated code-block-plus-paragraph pairs, the paragraphs
  identically shaped. Opens: *"Create a design doc for <one-line feature description>."*
  followed by *"Triggers the `cafleet-design-doc` skill's create workflow — a Director /
  Drafter / Reviewer team drafts the design document."*; then *"Interview me about
  design-docs/NNNNNNN-<slug>."* followed by *"Triggers the `cafleet-design-doc` skill's
  interview workflow — a Director + Analyzer pair annotates the document with your
  answers."*; then *"Implement design-docs/NNNNNNN-<slug>."* followed by *"Triggers the
  `cafleet-design-doc` skill's execute workflow — a Director / Programmer / Tester team
  implements the document and a fresh Reviewer member reviews it before your approval."*
  Every paragraph is the same sentence with three slots refilled — the strongest tabulate
  signal on any of my pages.
- **Proposed table columns**: `Stage | Prompt | Workflow | Team`
- **Row keys**: `1. Draft` (`Create a design doc for <one-line feature description>.` /
  create / Director + Drafter + Reviewer); `2. Refine` (`Interview me about
  design-docs/NNNNNNN-<slug>.` / interview / Director + Analyzer, annotating the document
  with your answers); `3. Implement` (`Implement design-docs/NNNNNNN-<slug>.` / execute /
  Director + Programmer + Tester, with a fresh Reviewer before your approval). Put each
  prompt in a code span so it stays copy-pasteable; keep the `Stage` numbering so the
  page's "one prompt per stage, in order" promise survives the move into a grid.
- **Prose that must remain**: the page intro ("CAFleet ships three skills that run
  spec-driven development as CAFleet-orchestrated teams. Give your coding agent one prompt
  per stage, in order.") and the closing pointer to Contributing for the
  contributor-facing description of the loop.
- **Why**: the reader is choosing which prompt to paste; a `Prompt` column puts the three
  side by side instead of separated by two paragraphs of near-identical prose.
- **Priority**: High

### F7. docs/how-to/mixed-backend-team.md — ## Appendix: the CLI underneath — three identical member-create blocks that differ only by three values

- **Location**: `docs/how-to/mixed-backend-team.md`, `## Appendix: the CLI underneath`
  (anchor `#appendix-the-cli-underneath`), inside the "Expand the walkthrough" block —
  the "Spawn one member per backend:" run of three commands and their three outputs.
- **Current form**: three near-verbatim command/output pairs. Opens: *"Spawn one member
  per backend:"* then `cafleet member create --fleet-id 1 --name "alice" --description
  "claude member" --coding-agent claude --text "You are alice. Wait for instructions."` →
  `3 alice backend=claude pane=%7`, and the same eight-token command twice more for `bob`
  / codex / `4 bob backend=codex pane=%8` and `carol` / opencode / `5 carol
  backend=opencode pane=%9`. Unlike the surrounding tear-down and message steps, these
  three are not a sequence — they are one parameterized action applied to three parallel
  items, and only three tokens change between them.
- **Proposed table columns**: `Member | Backend | Resulting member id | Pane`
- **Row keys**: `alice` (claude / 3 / `%7`); `bob` (codex / 4 / `%8`); `carol` (opencode /
  5 / `%9`).
- **Prose that must remain**: the "Spawn one member per backend:" lead-in, and **one**
  `member create` command block shown in full (the `alice` invocation) with a sentence
  saying to repeat it substituting the name, description, and `--coding-agent` value from
  each table row. Everything else on the page stays as is — the `fleet create` step, the
  `member list` output (already a table, and it stays: it shows the real CLI output shape),
  the message-send step, and the tear-down sequence are all genuinely ordered narrative.
- **Why**: collapses three redundant twelve-line blocks into one command plus a
  three-row table, and makes the id/pane assignment the walkthrough depends on later
  ("repeat with `--to-member-id 4` and `--to-member-id 5`") scannable in one place.
- **Priority**: Low
- **Caveat for the Director**: this trims a walkthrough, and walkthroughs earn latitude.
  If the team's convention is that expandable appendices stay literal and copy-pasteable
  end to end, drop this finding — it is the weakest of the seven.

## Table-misuse findings

None.

The only existing tables on my ten pages are `docs/contributing.md` § Project structure,
`docs/api/broker.md` § Package layout, and the `cafleet member list` output samples in the
two walkthroughs. All three kinds are used correctly: parallel items, short cells, a real
key column. One borderline observation, below the reporting bar and recorded only so the
Director knows it was checked — in `contributing.md` § Project structure the
`package.json + bun.lock` row's Purpose cell runs to two sentences plus a semicolon clause
("Bun toolchain manifests for the Slidev + agent-browser tools used in the repo. Driven via
`mise //:bun-install` / `mise //:slidev <deck>`; `node_modules/` is gitignored."), noticeably
longer than its five neighbours. It still scans, and one long row out of six does not make
the table stop working. No change proposed.
