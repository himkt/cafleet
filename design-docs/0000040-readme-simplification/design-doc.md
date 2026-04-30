# README simplification — anchor on the Claude Code plugin journey

**Status**: Approved
**Progress**: 12/23 tasks complete
**Last Updated**: 2026-04-30

## Overview

Replace the bloated 333-line README.md with a short, audience-focused README aimed at developers who have decided to try CAFleet and need install + first run. The new spine is the Claude Code plugin journey: (1) plugin install command, (2) an example prompt that exercises the plugin, (3) a compact CLI cheatsheet. Advanced detail moves out — ARCHITECTURE.md and `docs/spec/` remain the canonical deep-dive home.

## Success Criteria

- [ ] `README.md` is restructured around three top-level sections in this order: **Install (plugin)** → **Try it (example prompt)** → **CLI cheatsheet** (one consolidated short table). Other sections (architecture link, development, license) are present but compact.
- [ ] Total `README.md` length is ≤ 180 lines (current: 333 lines). Soft target ~150 lines; hard cap 180.
- [ ] The Features list (currently 20 changelog-style bullets, README L11–L28) is replaced by **4–6 high-level bullets** OR removed entirely if the example prompt carries the value. Decision is recorded in the rewritten README; either form satisfies this criterion.
- [ ] The two large CLI tables (Server Administration, Agent Commands — README L207–L238) are replaced by **one consolidated table with one row per command group** (`session`, `agent`, `message`, `member`, `db`, `server`, `doctor`) plus a single link to `docs/spec/cli-options.md` for the full per-subcommand surface.
- [ ] Every paragraph documenting *internal mechanics* — Administrator protection, soft-delete cascade, root-Director protection, push-notification mechanics, env-var precedence, Alembic state matrix, `agent_placements` semantics, `--permission-mode dontAsk`, the Bash-via-Director fallback, Claude Code `permissions.allow` rationale — is removed from `README.md`. None of that detail is paraphrased back; readers who need it follow a single link to `ARCHITECTURE.md`.
- [ ] The plugin install section uses the canonical Claude Code plugin install command shape verified against `.claude-plugin/plugin.json` (manifest `name = cafleet`, `repository = https://github.com/himkt/cafleet`, declares 5 skills under `./skills/*`). The command shape is `/plugin marketplace add himkt/cafleet` followed by `/plugin install cafleet@himkt/cafleet` (Claude Code marketplace + install pattern).
- [ ] The example-prompt section shows **one** end-to-end prompt that a user types into Claude Code after the plugin is installed, demonstrating one of the plugin's high-value skills (the recommended hero is `/cafleet:design-doc-create <slug>` because it exercises every primitive — session bootstrap, member spawn, message broker, monitoring loop, teardown — through a single user prompt). Showing the full prompt the user types is sufficient — no transcript of the agent's response.
- [ ] Light audit pass over `ARCHITECTURE.md` and `docs/spec/` ensures every fact `README.md` is dropping is already documented (or moved into) one of those two surfaces. No content is lost in the cut. Audit findings + any patches are recorded in the implementation Step 1 below.
- [ ] Proactive sweep of every `skills/*/SKILL.md` (5 skills: `cafleet`, `cafleet-monitoring`, `design-doc`, `design-doc-create`, `design-doc-execute`) for links or anchors pointing into README sections that are deleted. Any matches are repointed to `ARCHITECTURE.md` or `docs/spec/cli-options.md`. Sweep is recorded as a checked task in Step 4 with the file:line list of matches (or "no matches found").
- [ ] All four mise tasks pass after the change: `mise //cafleet:lint`, `mise //cafleet:format`, `mise //cafleet:typecheck`, `mise //cafleet:test`. (No source code changes are expected, but the typecheck / test runs guard against an accidental skill-loader regression if a skill's `SKILL.md` frontmatter is touched.)

---

## Background

The current `README.md` was grown additively across ~30 design docs. It now duplicates `ARCHITECTURE.md` and `docs/spec/cli-options.md` for an audience that does not need those details on first read.

| Symptom | Lines (current README) | Fix |
|---|---|---|
| 20-bullet Features list reads like a changelog | L11–L28 | Compress to 4–6 bullets or delete |
| Architecture diagram + design-decisions prose | L30–L49 | Replace with one paragraph + `ARCHITECTURE.md` link |
| Quick-start block walks every CLI step (session create → register → send → poll → ack → server) | L51–L182 | Replace with plugin-install + example-prompt + cheatsheet |
| Two CLI tables enumerate every subcommand | L207–L238 | Collapse to one row-per-group table + `docs/spec/cli-options.md` link |
| Tech-stack + Project-structure + Development blocks duplicate `ARCHITECTURE.md` | L255–L328 | Keep Development (mise commands), drop the rest |

The reframing — plugin install + example prompt + cheatsheet — is also a coverage fix: the current README documents `pip install cafleet` for the broker CLI but does **not** document how to install the cafleet **plugin** into Claude Code (the actual entry point for most users, since the plugin manifest at `.claude-plugin/plugin.json` ships 5 skills consumed via `/cafleet:*` slash commands).

---

## Specification

### 1. Target outline of the rewritten `README.md`

```
# CAFleet                                                # H1 + one-line tagline
> Local-only callout                                     # ≤ 2 lines

## Install                                               # PRIMARY entry point
  - "Install the plugin in Claude Code" subsection       # /plugin marketplace add + /plugin install
  - "Install the broker CLI (optional)" subsection       # uv tool install / pip install + cafleet db init

## Try it                                                # the example-prompt hero
  - One example prompt the user pastes into Claude Code
  - One sentence on what the plugin does in response
  - Link to skills/cafleet/SKILL.md and skills/design-doc-create/SKILL.md for more skills

## CLI cheatsheet                                        # consolidated reference
  - Single table: command-group | one-line purpose
  - Link to docs/spec/cli-options.md for the full surface

## Architecture                                          # ≤ 5 lines
  - One paragraph
  - Link to ARCHITECTURE.md

## Development                                           # contributor-only
  - mise //cafleet:sync / lint / format / typecheck / test
  - mise //admin:build / dev for the WebUI

## License
```

Total target: ≤ 180 lines including code blocks and blank lines.

### 2. Plugin install section — exact wording requirement

```markdown
## Install

### Install the plugin in Claude Code

```
/plugin marketplace add himkt/cafleet
/plugin install cafleet@himkt/cafleet
```

This adds 5 skills under the `cafleet` namespace: `cafleet`, `cafleet-monitoring`, `design-doc`, `design-doc-create`, `design-doc-execute`. Run `/help` in Claude Code to see them.

### Install the broker CLI (required for the plugin to function)

```bash
uv tool install cafleet     # or: pip install cafleet
cafleet db init              # one-time SQLite schema setup
```
```

The plugin install commands are sourced from `.claude-plugin/plugin.json` (manifest `name = cafleet`, `repository = https://github.com/himkt/cafleet`). The marketplace add + plugin install pair is the standard Claude Code plugin install pattern.

**Verification step (mandatory, before committing the README wording)**: the Programmer MUST `WebFetch` `https://docs.claude.com/en/docs/claude-code/plugins` and confirm the canonical syntax of (a) the marketplace-add command and (b) the plugin-install command — specifically whether the marketplace identifier is `himkt/cafleet` (a GitHub `owner/repo` shorthand) or a full URL, and whether the install identifier is `cafleet@himkt/cafleet` or another form. If the canonical form has shifted, adjust the wording (not the structure) and record the verified shape in the commit message. If `https://docs.claude.com/en/docs/claude-code/plugins` is unreachable or has moved, fall back to `https://docs.claude.com/en/docs/claude-code/` and follow the plugins entry in the table of contents.

### 3. Example-prompt section — what to show

Pick ONE hero example. Recommended: `/cafleet:design-doc-create my-feature`. Reasoning:

- Single user prompt produces a complete observable outcome (a design doc on disk).
- Exercises every primitive the plugin is built on: session bootstrap, member spawn, message broker, monitoring loop, teardown.
- Reuses material the user is likely to want anyway.

The section shows:

1. The exact prompt the user types into Claude Code (one fenced block).
2. One sentence describing what happens (Director spawns Drafter + Reviewer in tmux panes, they coordinate via the broker, polished design doc lands at `design-docs/my-feature/design-doc.md`).
3. A "Want more?" line linking to `skills/cafleet/SKILL.md` (raw broker primitives) and `skills/design-doc-create/SKILL.md` (the orchestration this hero uses).

No transcript. No screenshots. The user already knows what Claude Code looks like; the README's job is to show what to type, not to re-render Claude Code in markdown.

**Verbatim README content for this section** — the Programmer should reproduce this almost word-for-word (only the slug `my-feature` is illustrative and can be left as-is):

````markdown
## Try it

In any tmux session, paste this into Claude Code:

```
/cafleet:design-doc-create my-feature
```

Claude (the Director) bootstraps a CAFleet session, spawns a Drafter and a Reviewer in adjacent tmux panes, drives the clarification → draft → review loop through the message broker, and lands a polished design doc at `design-docs/my-feature/design-doc.md`.

Want more? See [`skills/cafleet/SKILL.md`](skills/cafleet/SKILL.md) for the raw broker primitives and [`skills/design-doc-create/SKILL.md`](skills/design-doc-create/SKILL.md) for the orchestration this example uses.
````

The fenced inner block is exactly one line: the slash command. No leading prose like "Type this into Claude Code" inside the fenced block — the surrounding sentence already names that.

### 4. CLI cheatsheet — single consolidated table

Replaces the current two tables (Server Administration L207–L213, Agent Commands L220–L238) with one row per command group:

| Command group | One-line purpose |
|---|---|
| `cafleet db init` | Apply schema migrations (one-time) |
| `cafleet session *` | Create / list / show / delete sessions |
| `cafleet agent *` | Register / deregister / list / show agents |
| `cafleet message *` | Send / broadcast / poll / ack / cancel / show messages |
| `cafleet member *` | Spawn / delete / list / capture / send-input / exec / ping member panes (Director only) |
| `cafleet server` | Start the admin WebUI on `127.0.0.1:8000` |
| `cafleet doctor` | Print the calling pane's tmux identifiers |

> Full per-subcommand reference: [docs/spec/cli-options.md](docs/spec/cli-options.md).

### 5. What is removed from `README.md` (and where the content already lives)

| Removed from README | Lines | Already covered in |
|---|---|---|
| 20-bullet Features list | L11–L28 | `ARCHITECTURE.md` § Component Layout, § Member Lifecycle, § Bash Routing via Director, § WebUI |
| Architecture diagram + design-decisions prose | L30–L49 | `ARCHITECTURE.md` § Architecture Diagram, § Key Design Decisions |
| Verbose `cafleet session create` text + JSON output blocks | L72–L138 | `skills/cafleet/SKILL.md` § Typical Workflow + `docs/spec/cli-options.md` |
| `cafleet session delete` semantics block | L127–L138 | `ARCHITECTURE.md` § Session Isolation (soft-delete paragraph) |
| `cafleet server` flag + env-var precedence block | L169–L182 | `ARCHITECTURE.md` § Key Design Decisions § CLI Option Sources |
| Two CLI tables (Server Administration, Agent Commands) | L207–L238 | `docs/spec/cli-options.md` |
| API Overview + Message Lifecycle table | L240–L253 | `ARCHITECTURE.md` § Task Lifecycle Mapping + `docs/spec/data-model.md` |
| Tech Stack block | L255–L260 | `ARCHITECTURE.md` § Component Layout |
| Project Structure tree | L262–L286 | `ARCHITECTURE.md` § Component Layout, § Package Structure |
| Build the WebUI block | L310–L328 | `ARCHITECTURE.md` § WebUI |

### 6. ARCHITECTURE.md and docs/ light audit

Per Q7 the user asked for a *light* audit — not a rewrite — to confirm nothing the README drops is lost. The audit produces, at most, a small set of additive sentences:

- For each row in the table above, open the "Already covered in" target and confirm the fact is present.
- If a fact is in `README.md` only (not yet in `ARCHITECTURE.md` or `docs/spec/`), add a single sentence to the appropriate target. Do not restructure.
- The audit must NOT introduce duplication elsewhere — the Removal rule (`~/.claude/rules/removal.md`) applies. After the README cut + audit lands, the removed paragraphs exist exactly once in the repo (in their canonical home), not twice.

Expected scope: 0–3 small additions to `ARCHITECTURE.md`, 0–1 small additions to `docs/spec/cli-options.md`. If the audit finds the cut would lose content not covered anywhere, that fact is recorded as a checked task and the additive patch is included in the same PR.

#### Audit findings (Step 1, completed 2026-04-30)

For each row of §5, the "Already covered in" target was opened and the fact was confirmed present. Findings (one sub-bullet per row):

- **Row 1 (Features list, L11–L28)** — covered. ARCHITECTURE.md § Component Layout (cli.py row enumerates `db init`, `session`, `agent`, `message`, `member` subgroups + `server` / `doctor` exceptions), § Session Isolation (root Director bootstrap transactional invariant, soft-delete cascade, built-in Administrator paragraph at L48), § Member Lifecycle (atomic create flow + delete ordering + write-path authorization + member exec/ping primitives), § Bash Routing via Director ("Members spawn with `--permission-mode dontAsk`... The bash-via-Director protocol is the **fallback** for the harness deny-list"), § tmux Push Notifications, § WebUI, § Storage Layer (SQLite single file, PRAGMA busy_timeout=5000), and § Design Document Orchestration Skills together cover every Features bullet. No patch needed.
- **Row 2 (Architecture diagram + design-decisions prose, L30–L49)** — covered. ARCHITECTURE.md § Architecture Diagram (equivalent ASCII diagram with sessions/agents/tasks/agent_placements/alembic_version table list) + § Key Design Decisions (contextId Convention, Task Lifecycle Mapping, CLI Option Sources subsections). No patch needed.
- **Row 3 (Verbose `cafleet session create` text + JSON output blocks, L72–L138)** — covered. docs/spec/cli-options.md § `session create` (lines 78–136) reproduces the full non-JSON two-line + label/created_at/director_name/pane/administrator block AND the nested `--json` shape with `administrator_agent_id` at top level alongside `director`. No patch needed.
- **Row 4 (`cafleet session delete` semantics block, L127–L138)** — covered. ARCHITECTURE.md § Session Isolation L42: "`cafleet session delete <id>` runs a single transaction: (1) `UPDATE sessions SET deleted_at=now WHERE session_id=X AND deleted_at IS NULL`, (2) `UPDATE agents SET status='deregistered'…`, (3) `DELETE FROM agent_placements WHERE agent_id IN (SELECT agent_id FROM agents WHERE session_id=X)`. Tasks are never touched… It is **not** transactional with tmux: surviving member panes are orphaned intentionally. Directors that want a clean shutdown run `cafleet member delete` per member first (which does send `/exit`), then `session delete`." Also docs/spec/cli-options.md § `session delete` (lines 166–188) and docs/spec/data-model.md § Session Lifecycle. No patch needed.
- **Row 5 (`cafleet server` flag + env-var precedence block, L169–L182)** — covered. ARCHITECTURE.md § Key Design Decisions § CLI Option Sources L248: "The `cafleet server` bind address and port are configured via `--host` / `--port` flags (defaults sourced from `settings.broker_host` = `127.0.0.1` and `settings.broker_port` = `8000`) or via the `CAFLEET_BROKER_HOST` / `CAFLEET_BROKER_PORT` environment variables… CLI flags win over env vars when both are supplied. The `127.0.0.1` default matches CAFleet's local-only stance; users who need external binding pass `--host 0.0.0.0` or set `CAFLEET_BROKER_HOST=0.0.0.0`." Also docs/spec/cli-options.md § `cafleet server` (lines 234–280) covers the precedence matrix verbatim. No patch needed.
- **Row 6 (Two CLI tables — Server Administration L207–L213, Agent Commands L220–L238)** — covered. docs/spec/cli-options.md is the authoritative per-subcommand surface for every group: `db init` (L262–L280 ref), `session create/list/show/delete` (L74–L188), `agent register/deregister/list/show` (per the table at L51–L73), `message send/broadcast/poll/ack/cancel/show` (per L57–L62 + Error Messages at L589–L615), `member create/delete/list/capture/send-input/exec/ping` (L281–L587), `server` (L234–L280), `doctor` (L190–L232). No patch needed.
- **Row 7 (API Overview + Message Lifecycle table, L240–L253)** — covered. ARCHITECTURE.md § Key Design Decisions § Task Lifecycle Mapping L226–L233 has the four-state table (`TASK_STATE_INPUT_REQUIRED` → unread, `TASK_STATE_COMPLETED` → acknowledged, `TASK_STATE_CANCELED` → retracted, `TASK_STATE_FAILED` → routing error). docs/spec/data-model.md § tasks (`status_state` column at L103) and § Known design debt — ACK timestamp inference (L220–L224) cover the persistence side. No patch needed.
- **Row 8 (Tech Stack block, L255–L260)** — covered. ARCHITECTURE.md § Component Layout (cli.py "click", server.py "Minimal FastAPI app", db/models.py "SQLAlchemy declarative models", config.py "Settings via pydantic-settings", admin/ "Vite + React + TypeScript + Tailwind CSS") + § Package Structure L264 ("`cafleet`: FastAPI + SQLAlchemy + Alembic + click") and L265 ("WebUI SPA: Vite + React + TypeScript + Tailwind CSS"). The "Python 3.12+ managed with uv" fact lives canonically in `cafleet/pyproject.toml` (`requires-python = ">=3.12"`) and is conveyed in the new README's Install section by the `uv tool install cafleet` command shape itself; no ARCHITECTURE.md patch needed.
- **Row 9 (Project Structure tree, L262–L286)** — covered. ARCHITECTURE.md § Component Layout has a per-file table mapping every component to its location (`broker.py`, `server.py`, `cli.py`, `config.py`, `db/models.py`, `db/engine.py`, `alembic.ini`, `alembic/env.py`, `alembic/versions/`, `webui_api.py`, `output.py`, `coding_agent.py`, `tmux.py`, `admin/`). § Package Structure L262–L267 documents the `cafleet/` + `admin/` top-level split. The `docs/spec/{data-model,webui-api,cli-options}.md` enumeration is canonical at the docs/spec/ directory itself. No patch needed.
- **Row 10 (Build the WebUI block, L310–L328)** — covered. ARCHITECTURE.md § WebUI L258: "**Static serving**: `StaticFiles` mount at `/ui` serves the SPA bundled inside the package at `cafleet/src/cafleet/webui/` (production build). `mise //admin:build` must be run before `cafleet server` / `mise //cafleet:dev` for `/ui/` to be populated; without it, `create_app()` emits a one-line `warning: admin WebUI is not built. /ui/ will return 404. Run 'mise //admin:build'.` to stderr at startup, the server starts cleanly, and `/ui/` 404s until the SPA is built." The README-only "Release maintainers: run `mise //admin:build` before any `uv build`" caveat (and the `unzip -l dist/cafleet-*.whl | grep webui/index.html` verification one-liner) is operational guidance for release maintainers — outside the install/first-run audience the new README targets — and is preserved in this design doc record + git history per the Removal rule. No ARCHITECTURE.md patch needed.

**Gap list**: 0 hard gaps. Two soft observations are preserved at their canonical sources rather than copied into ARCHITECTURE.md or docs/spec/:

1. *Python 3.12+ / uv tooling requirement*: canonical home is `cafleet/pyproject.toml` (`requires-python = ">=3.12"`); the new README's Install section conveys uv via `uv tool install cafleet` directly.
2. *Release-maintainer wheel-build caveat* (`mise //admin:build` before `uv build`; `unzip -l` wheel verification): canonical home is this design doc + git history. ARCHITECTURE.md is about runtime architecture, not release operations; CONTRIBUTING.md / RELEASE.md would be the right home if/when one is created — adding to ARCHITECTURE.md would dilute its scope.

**Proposed patch list**: 0 patches. Step 2 is a no-op (per the design doc's allowance: "If Step 1 found zero gaps, this step is a no-op and is checked off without changes.").

### 7. SKILL.md sweep procedure

For each of the 5 skills declared in `.claude-plugin/plugin.json` (`./skills/cafleet`, `./skills/cafleet-monitoring`, `./skills/design-doc`, `./skills/design-doc-create`, `./skills/design-doc-execute`):

1. Open the `SKILL.md` (and any `roles/*.md` siblings that ship with the skill).
2. Search for the literal strings `README.md`, `README#`, `../README`, and `/README` (case-sensitive — uppercase only).
3. For each match: if the link points to a section that the new README still has, leave it. If it points to a section that is being deleted, repoint to the new canonical home (typically `ARCHITECTURE.md` or `docs/spec/cli-options.md`).
4. Record the file:line of every match (and its disposition: kept / repointed / removed) as a checked task in Step 4.

Expected scope: based on the SKILL.md files already inspected (`cafleet`, `cafleet-monitoring`, `design-doc-create`), no current SKILL.md links into specific README anchors. The sweep is likely a no-op, but the design must not assume that — it must record the actual finding.

### 8. Out of scope

- No source code changes (`cafleet/src/cafleet/`).
- No CLI surface changes.
- No `ARCHITECTURE.md` rewrite — only additive patches if the audit finds gaps.
- No `docs/spec/` rewrite — same caveat.
- No new tests. The existing `mise //cafleet:test` / `lint` / `format` / `typecheck` runs are sanity checks, not validation of new behavior.
- No changes to project rules (`.claude/rules/*.md`) or settings (`.claude/settings.json`).

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

> **Implementation order rationale.** Per `.claude/rules/design-doc-numbering.md` (Implementation Order section), documentation work proceeds ARCHITECTURE.md → docs/ → README.md → SKILL.md. Steps below honor that order: Step 1 audits and **drafts** ARCH/docs patches without touching files; Step 2 **applies** those patches (ARCH/docs file edits); Step 3 rewrites README.md; Step 4 sweeps SKILL.md. Step 1 records proposed patch text only — it does not touch any file. Step 2 is the sole place ARCH/docs files are edited.

### Step 1: Audit ARCHITECTURE.md and docs/spec/ — DRAFT ONLY

- [x] For each row of the "What is removed from README" table (§5), open the "Already covered in" target and confirm the fact is present. Record findings (verbatim quote of the covering sentence, OR the literal string "covered — no patch needed") in this design doc under §6 as a checked sub-bullet per row. Do NOT edit any target file in this step. <!-- completed: 2026-04-30T12:00 -->
- [x] For every fact found to live ONLY in `README.md`, write the proposed 1–3 sentence additive patch text into this design doc under §6, alongside the target file path and the location-anchor (existing heading or paragraph) it should be inserted near. Do NOT apply the patch in this step. <!-- completed: 2026-04-30T12:00 -->
- [x] Confirm the proposed patches collectively cover every gap and do not introduce new duplication with content already present elsewhere. Mark this design-doc audit as Step 1 complete only when both the gap list and the proposed patch list are recorded. <!-- completed: 2026-04-30T12:00 -->

### Step 2: Apply ARCHITECTURE.md / docs/spec/ patches

- [x] Apply each Step 1 proposed patch (if any) to its target file using the location-anchor recorded in Step 1. If Step 1 found zero gaps, this step is a no-op and is checked off without changes. <!-- completed: 2026-04-30T12:05 (no-op: Step 1 found 0 gaps, 0 patches to apply) -->
- [x] After applying patches, re-grep the touched files plus `README.md` for the patched sentences and confirm each appears in exactly one place. The Removal rule forbids the cut content from appearing in two places after Step 3 lands. <!-- completed: 2026-04-30T12:05 (no-op: 0 patched sentences to verify; vacuous when patch list is empty) -->

### Step 3: Rewrite README.md

- [x] Delete the current Features list (L11–L28) and replace with ≤ 6 high-level bullets, OR drop the section entirely if the example prompt in §3 carries the value. Record the choice in the commit message. <!-- completed: 2026-04-30T12:30 (chose path B: removed entirely; example prompt + cheatsheet carry the value) -->
- [x] Replace the Architecture block (L30–L49) with one paragraph + a single `ARCHITECTURE.md` link. <!-- completed: 2026-04-30T12:30 -->
- [x] Replace the Quick Start block (L51–L182) with the new Install section (§2 of this doc) followed by the Try it section (§3 of this doc). Run the §2 mandatory `WebFetch` verification against `https://docs.claude.com/en/docs/claude-code/plugins` BEFORE committing the wording, and record the verified command shape in the commit message. <!-- completed: 2026-04-30T12:30 (WebFetched 2x: docs.claude.com → 301 → code.claude.com/docs/en/plugins, then code.claude.com/docs/en/discover-plugins; verified: marketplace add takes `owner/repo` GitHub shorthand, install uses `<plugin>@<marketplace-name>` where marketplace name auto-derives from `owner-repo` (hyphenated). Corrected design-doc's `cafleet@himkt/cafleet` to canonical `cafleet@himkt-cafleet`.) -->
- [x] Replace the two CLI tables (L207–L238) with the single consolidated table (§4 of this doc) + the `docs/spec/cli-options.md` link. <!-- completed: 2026-04-30T12:30 -->
- [x] Delete the API Overview / Message Lifecycle / Tech Stack / Project Structure blocks (L240–L286). <!-- completed: 2026-04-30T12:30 -->
- [x] Trim the Development block (L288–L328): keep the mise commands, drop the per-section explanation paragraphs, drop the Build the WebUI prose. <!-- completed: 2026-04-30T12:30 -->
- [x] Confirm final line count is ≤ 180 (`wc -l README.md`). <!-- completed: 2026-04-30T12:30 (final: 80 lines) -->

### Step 4: SKILL.md sweep

- [ ] Search `skills/cafleet/SKILL.md` (+ `roles/director.md`, `roles/member.md`) for `README.md`, `README#`, `../README`, `/README`. Record matches. <!-- completed: -->
- [ ] Search `skills/cafleet-monitoring/SKILL.md` for the same strings. Record matches. <!-- completed: -->
- [ ] Search `skills/design-doc/SKILL.md` (+ `template.md`, `guidelines.md`) for the same strings. Record matches. <!-- completed: -->
- [ ] Search `skills/design-doc-create/SKILL.md` (+ every `roles/*.md`) for the same strings. Record matches. <!-- completed: -->
- [ ] Search `skills/design-doc-execute/SKILL.md` (+ every `roles/*.md`) for the same strings. Record matches. <!-- completed: -->
- [ ] For each match found in the steps above, decide kept / repointed / removed and record the disposition in the design doc. Apply each repoint or removal. <!-- completed: -->

### Step 5: Verification

- [ ] `mise //cafleet:lint` passes. <!-- completed: -->
- [ ] `mise //cafleet:format` produces no diff. <!-- completed: -->
- [ ] `mise //cafleet:typecheck` passes. <!-- completed: -->
- [ ] `mise //cafleet:test` passes. <!-- completed: -->
- [ ] Manually re-read the new `README.md` from a first-time-developer perspective and confirm: (a) Install is clear in under 30 seconds, (b) the example prompt is copy-pastable, (c) the cheatsheet answers "what commands exist" without forcing a click into `docs/spec/`. <!-- completed: -->

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-04-30 | Initial draft |
| 2026-04-30 | Reviewer round 1: fix Progress count to 0/23, split Step 1 (draft) and Step 2 (apply) to remove duplication, reorder steps to honor design-doc-numbering rule (ARCH/docs apply before README rewrite), pin verbatim README content for §3 example block, name the canonical Claude Code plugins documentation URL for §2 plugin install verification. |
| 2026-04-30 | Approved by user. Status set to Approved; ready for /design-doc-execute. |
