# Claude Config and README Cleanup

**Status**: Approved
**Progress**: 5/12 tasks complete
**Last Updated**: 2026-05-17

## Overview

Trim the root `CLAUDE.md`, the project-local `.claude/rules/skill-discovery.md`, and `README.md` down to the content that is not already provided elsewhere (the system-reminder skill list, the user's global rules, `docs/spec/*`, and `ARCHITECTURE.md`). Move contributor-only material from `README.md` into a new top-level `CONTRIBUTING.md`.

## Success Criteria

- [ ] Root `CLAUDE.md` no longer enumerates per-skill descriptions or specific design documents, and no longer contains a `## Skill Discovery & Authorization Scope` section.
- [ ] `.claude/rules/skill-discovery.md` no longer exists in the repository.
- [ ] `README.md` no longer contains the `### Notable flags` table, the `### Message body truncation` prose subsection, the `## Project structure` section, or the `## Development` section.
- [ ] `README.md` links to a new top-level `CONTRIBUTING.md`, and the inline `docs/spec/*` pointer below the CLI cheatsheet covers message body truncation explicitly.
- [ ] `CONTRIBUTING.md` exists at the repository root and contains the moved `Project structure` + `Development` content plus a `Contributing changes` subsection pointing at the `/cafleet:design-doc-*` skills.
- [ ] The pre-drafting investigation already verified that every removed README flag and the truncation rules survive in `docs/spec/cli-options.md` / `docs/spec/message-envelope.md` (see Background); no additional execution-time verification is required.

---

## Background

Three duplication / drift problems motivate the cleanup:

1. **Root `CLAUDE.md` re-states content the harness already injects.** The `## Skills` section enumerates seven CAFleet skills with their trigger descriptions, but those descriptions also arrive in the session-start system-reminder. The enumeration is pure duplication and rots whenever a skill description is reworded. Likewise, the design-document enumeration (five docs by number) is an arbitrary slice of the larger list in `design-docs/` and has no mechanism to stay current.
2. **`.claude/rules/skill-discovery.md` overlaps with the user's global `~/.claude/rules/skill-discovery.md` and with `skills/agent-team-supervision/SKILL.md`.** The CAFleet-specific Authorization-Scope Guard already lives in the skill that ships with the plugin (self-contained for downstream installs). The project-local copy is mostly a pointer to that skill, plus duplicates of the general "skill-first for GitHub" / "stop means stop" guidance that the global rules file covers.
3. **`README.md` carries deep CLI prose and contributor-only content.** The `### Notable flags` table, the `### Message body truncation` prose, the `## Project structure` section, and the `## Development` section are all out of place in an end-user-facing README. The flag and truncation content is already fully covered by `docs/spec/cli-options.md` and `docs/spec/message-envelope.md`; the structure / development content belongs in `CONTRIBUTING.md`.

A pre-drafting investigation (read by the Director and re-verified by the Drafter) confirms:

- Every flag in README's `### Notable flags` (`--pretty`, `--quiet`, `--full`, `--activity`, `--lines` / `--tail` / `--ansi` / `--no-ansi`, `CAFLEET_MAX_TEXT_LEN`) is fully documented in `docs/spec/cli-options.md`.
- The body-truncation rules in README's `### Message body truncation` are fully documented in `docs/spec/cli-options.md` § Message Body Truncation and `docs/spec/message-envelope.md`.
- `CONTRIBUTING.md` does not currently exist at the repository root.

No spec-doc gap-fill is required before the README prose is removed.

---

## Specification

### Out of scope

- No CLI behavior changes.
- No `skills/*/SKILL.md` content changes (the broker CLI surface is untouched, so the skills are not affected).
- No project rules under `.claude/rules/` other than `skill-discovery.md` (deleted entirely) and a single one-line pointer update in `commands.md` to fix the dangling reference left by the deletion.
- No `ARCHITECTURE.md` or `docs/spec/*` content changes (the existing spec coverage is sufficient — verified during investigation).
- No commit-convention, PR-checklist, or code-of-conduct content in `CONTRIBUTING.md` (only the moved sections plus the authorized design-doc-flow subsection).

### Root `CLAUDE.md` — final structure

After the cleanup, the file contains exactly four sections (header + three `##`s):

```markdown
# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Skills

The system-reminder lists every available skill with its trigger description. Read that list at the start of every session and invoke matching skills via the Skill tool BEFORE taking any other action.

## Project: CAFleet

A message broker and agent registry for coding agents.

- **Single package**:
  - `cafleet/` — `cafleet` (FastAPI + SQLAlchemy + Alembic + click)
- **Unified CLI command**: `cafleet` (with `db init` for schema management, `session` for session CRUD, and all agent/messaging commands)

## Tech Stack

- Python 3.12+ managed with uv
- Server: FastAPI + SQLAlchemy + Alembic
- CLI: click (direct SQLite via broker module)

## Commands

See `.claude/rules/commands.md` for the full command reference.
```

Concretely:

| Old content | Action |
|---|---|
| `## Skills` section enumerating seven `/cafleet`, `/agent-team-monitoring`, `/agent-team-supervision`, `/design-doc`, `/design-doc-create`, `/design-doc-interview`, `/design-doc-execute` entries (current lines 5–15) | Replaced verbatim with the one-sentence system-reminder pointer above. |
| Five `**Design document**:` bullets under `## Project: CAFleet` (current lines 21–25) | Removed entirely. The `design-docs/` directory is the canonical list. |
| `## Skill Discovery & Authorization Scope` section (current lines 40–44) | Removed entirely. |

The replacement Skills sentence — used verbatim, per the user's confirmation:

> The system-reminder lists every available skill with its trigger description. Read that list at the start of every session and invoke matching skills via the Skill tool BEFORE taking any other action.

### `.claude/rules/skill-discovery.md` — full removal

The project-local file is deleted. Coverage is preserved through three existing surfaces:

| Concern | Surviving home |
|---|---|
| Skill-first for GitHub operations (`@copilot` reviewer slug, `gh api repos/.../comments`) | User's global `~/.claude/rules/skill-discovery.md`. |
| "Stop means stop" halt-signal handling | User's global `~/.claude/rules/criticism-response.md` and global `~/.claude/rules/skill-discovery.md`. |
| CAFleet-specific Authorization-Scope Guard | `skills/agent-team-supervision/SKILL.md` (ships with the plugin and stays self-contained for downstream installs). |

No back-reference, deprecation note, or "see git history" comment is added to any other file. Per `~/.claude/rules/removal.md`, the deletion is total; the design document itself is the historical record.

### `.claude/rules/commands.md` — pointer redirect

A single live reference to the deleted file exists at `.claude/rules/commands.md:34` (the right-column cell of the "NEVER bypass mise" table):

```
| `uv run cafleet ...` for verification/smoke | delegate to a teammate that already has permission, or ask the user | see `.claude/rules/skill-discovery.md` (Authorization scope section) |
```

Per `~/.claude/rules/removal.md`, every live reference to a removed file must be cleaned up in the same change. The cell is redirected to the surviving canonical home named in `.claude/rules/skill-discovery.md — full removal` above:

```
| `uv run cafleet ...` for verification/smoke | delegate to a teammate that already has permission, or ask the user | see `skills/agent-team-supervision/SKILL.md` § *Authorization-Scope Guard* |
```

Only the right-column cell is touched; the other two columns of the row and every other table row remain unchanged. Historical references inside older `design-docs/*/design-doc.md` files are git-history artifacts and intentionally stay (they record the state at the time those docs were written).

### `README.md` — content removed and added

#### Removed

| Section | Lines (current) | Reason |
|---|---|---|
| `### Notable flags` table | 173–182 | Every flag duplicates `docs/spec/cli-options.md`. |
| `### Message body truncation` prose paragraph | 184–186 | Duplicates `docs/spec/cli-options.md` § Message Body Truncation and `docs/spec/message-envelope.md`. |
| `## Project structure` section | 192–201 | Moves to `CONTRIBUTING.md`. |
| `## Development` section | 203–222 | Moves to `CONTRIBUTING.md`. |

#### Retained as-is (explicitly out of scope for this design)

- The two verbose install-verification callouts: the `~/.claude/settings.json` example (current lines 24–60) and the `~/.codex/config.toml` example (current lines 75–86). They are end-user install-verification artifacts, not contributor-only content, and stay in `README.md` under `### Install CAFleet skills`.

#### Modified

The existing one-line `docs/spec/*` pointer immediately below the CLI cheatsheet (current line 171) is extended to explicitly name message body truncation, so the spec stays discoverable after the inline prose is removed. The exact replacement text:

```markdown
> CLI reference (per-command sections for `session`, `member`, `doctor`, `server`; `agent` / `message` / `db init` covered via the option-source table and `cafleet <cmd> --help`): [docs/spec/cli-options.md](docs/spec/cli-options.md). Message envelope shape (compact rendered + `--full` typed-column) and message body truncation rules (`CAFLEET_MAX_TEXT_LEN`, `--full`, `--quiet`): [docs/spec/cli-options.md](docs/spec/cli-options.md) § Message Body Truncation and [docs/spec/message-envelope.md](docs/spec/message-envelope.md).
```

The dedicated `§ Message Body Truncation` heading lives in `cli-options.md`; `message-envelope.md` covers the rendered shape after truncation. Both are cited so the discoverability the current README inline prose provides is preserved one-for-one.

#### Added

A new top-level `## Contributing` section is added near the end of `README.md` (between `## Architecture` and `## License`), pointing at the new `CONTRIBUTING.md`:

```markdown
## Contributing

Build, test, and project-structure instructions, plus the design-doc-driven contribution flow, live in [CONTRIBUTING.md](CONTRIBUTING.md).
```

### New `CONTRIBUTING.md` — full content

Created at the repository root with the following structure (and only this content — no commit-convention or PR-checklist additions). The outer fence in this design doc uses **four** backticks because the spec block contains a triple-backtick ` ```bash ` inner fence; the file written to disk uses the standard three-backtick fence for that inner block.

````markdown
# Contributing to CAFleet

CAFleet is developed using its own CAFleet-orchestrated skills — the repository dogfoods the spec-driven-development flow it ships. This document covers the project layout, the local development loop, and the contribution path.

## Project structure

| Top-level entry | Purpose |
|---|---|
| `cafleet/` | The `cafleet` Python package (FastAPI + SQLAlchemy + Alembic + click). |
| `admin/` | Admin WebUI SPA (Vite + React + TypeScript + Tailwind CSS). |
| `skills/` | Plugin skills shared by the Claude Code and Codex manifests. |
| `package.json` + `bun.lock` (repo root) | Bun toolchain manifests for the Slidev + agent-browser tools used in the repo. Driven via `mise //:bun-install` / `mise //:slidev <deck>`; `node_modules/` is gitignored. |
| `design-docs/` | Numbered design documents (`NNNNNNN-<slug>/design-doc.md`). |
| `docs/` | CLI reference, message envelope, and other operator-facing docs. |

## Development

Clone the repo and use mise for all common tasks:

```bash
git clone https://github.com/himkt/cafleet.git
cd cafleet

mise //:uv-sync
mise //cafleet:install    # editable uv tool install of the cafleet CLI
cafleet db init           # apply schema migrations (idempotent; rerun after upgrades)

mise //cafleet:lint       # ruff check + ruff format --check
mise //cafleet:format     # ruff format
mise //cafleet:typecheck  # ty
mise //cafleet:test       # pytest

mise //admin:build        # build the WebUI (required before /ui/ is served)
mise //admin:dev          # WebUI dev server (Vite)
```

## Contributing changes

CAFleet uses its own design-doc-driven development skills to evolve the codebase. New contributors should follow the same flow:

1. `/cafleet:design-doc-create <one-line description>` — orchestrates a Director / Drafter / Reviewer team to produce a design doc under `design-docs/NNNNNNN-<slug>/`.
2. `/cafleet:design-doc-interview design-docs/NNNNNNN-<slug>` — fine-grained Q&A pass that annotates the doc with `COMMENT(claude)` markers for `/cafleet:design-doc-create` resume mode to absorb.
3. `/cafleet:design-doc-execute design-docs/NNNNNNN-<slug>` — TDD-cycle implementation pass (Director / Programmer / Tester / optional Verifier).

The same flow works from Codex with `$cafleet:design-doc-*` prefixes. Existing design documents under [`design-docs/`](design-docs/) are real examples produced by this loop.
````

### Implementation order rationale

This design has no code changes. Per `.claude/rules/design-doc-numbering.md`, documentation updates are the first implementation step in every design doc — here they are the only implementation. The order below groups edits by file so each step ends with a coherent, committable state:

1. **`CLAUDE.md` first** — establishes the new Skills-section convention before any other doc references it.
2. **`.claude/rules/skill-discovery.md` deletion plus `commands.md` pointer redirect second** — the removed CLAUDE.md pointer at the bottom of step 1 referenced this file; deleting it and updating the single live `commands.md` reference in the same step keeps each intermediate commit free of dangling links.
3. **`CONTRIBUTING.md` third** — creates the destination before README starts linking to it.
4. **`README.md` fourth** — removes the duplicated content and adds the `CONTRIBUTING.md` link last, when the destination already exists.

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

### Step 1: Trim root `CLAUDE.md`

- [x] Replace the entire `## Skills` heading-and-body block (current lines 5–15, i.e. the `## Skills` line plus the 7-bullet enumeration) with the heading + prose block shown in the code-fenced final-file view in Specification § Root `CLAUDE.md` — final structure. **The code-fenced final-file view is the authoritative target**; the blockquote that follows it repeats only the body sentence for emphasis on the verbatim wording, not as an alternative form to substitute. <!-- completed: 2026-05-17T01:45 -->
- [x] Remove the five `**Design document**:` bullets under `## Project: CAFleet` (current lines 21–25). <!-- completed: 2026-05-17T01:45 -->
- [x] Remove the entire `## Skill Discovery & Authorization Scope` section (current lines 40–44). <!-- completed: 2026-05-17T01:45 -->

### Step 2: Remove project-local skill-discovery rules file and fix the live pointer

- [x] Delete `.claude/rules/skill-discovery.md` from the repository. <!-- completed: 2026-05-17T01:50 -->
- [x] Update the right-column cell of `.claude/rules/commands.md:34` to redirect from ``see `.claude/rules/skill-discovery.md` (Authorization scope section)`` to ``see `skills/agent-team-supervision/SKILL.md` § *Authorization-Scope Guard*`` (verbatim text in Specification § `.claude/rules/commands.md` — pointer redirect). No other file gets a back-reference, deprecation note, or "see git history" pointer. <!-- completed: 2026-05-17T01:50 -->

### Step 3: Create `CONTRIBUTING.md`

- [ ] Create `CONTRIBUTING.md` at the repository root with the four-section content specified in Specification § New `CONTRIBUTING.md` (intro + `Project structure` + `Development` + `Contributing changes`). <!-- completed: -->

### Step 4: Clean up `README.md`

- [ ] Remove the `### Notable flags` table (current lines 173–182). <!-- completed: -->
- [ ] Remove the `### Message body truncation` subsection heading and its single prose paragraph (current lines 184–186). <!-- completed: -->
- [ ] Remove the `## Project structure` section (current lines 192–201). <!-- completed: -->
- [ ] Remove the `## Development` section (current lines 203–222). <!-- completed: -->
- [ ] Replace the existing `docs/spec/*` pointer line below the CLI cheatsheet (current line 171) with the extended version that names message body truncation (verbatim text in Specification § `README.md` Modified). <!-- completed: -->
- [ ] Add a new `## Contributing` section between `## Architecture` and `## License`, linking to `CONTRIBUTING.md` (verbatim text in Specification § `README.md` Added). <!-- completed: -->

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-05-17 | Initial draft |
