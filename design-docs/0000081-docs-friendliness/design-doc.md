# Documentation Friendliness (README.md + docs/ + zensical.toml)

**Status**: Approved
**Progress**: 20/20 tasks complete
**Last Updated**: 2026-06-11

## Overview

Design doc 0000079 made the docs lean (SSOT map, necessity test, symbol strip); this design makes them friendly. It adds the missing reader-facing layers — worked examples with expected output, a task-oriented how-to section, a core-terms table, a troubleshooting page, API-reference orientation, a scannable CLI summary table, and a "who is this for" framing on README and the docs landing page — and fixes the quickstart/cli-options shell-variable contradiction. Documentation-only: changes are confined to `README.md`, `CONTRIBUTING.md` (verification only), `docs/`, and `zensical.toml`.

## Success Criteria

- [x] No doc instructs the reader to store ids in shell variables: `grep -rE "export (FLEET|DIRECTOR)_ID" docs/ README.md` returns nothing, and every walkthrough pastes literal integer ids, consistent with the `spec/cli-options.md` Option Source Matrix rationale.
- [x] Every CLI walkthrough in README, quickstart, and the how-to pages shows the command **and** an expected-output block, using the standard sample-id cast (fleet `1`, root Director `2`, Administrator `3`, members `4`+).
- [x] A top-level **How-to guides** nav section exists with four task pages (mixed-backend team; monitor + recover members; WebUI; design-doc-driven development) plus a section index. Each page sequences existing facts and links to canonical homes — it introduces no new normative facts.
- [x] `docs/get-started/troubleshooting.md` exists under Get Started, maps ≥ 12 symptoms to fixes, opens with `cafleet doctor`, and links to (never duplicates) the Error Messages table in `spec/cli-options.md`.
- [x] `spec/cli-options.md` opens with a subcommand summary table (one row per subcommand: name, one-line purpose, needs `--fleet-id`, identity flag, link), and every row's link resolves to a section on the page — including new compact `cafleet agent` / `cafleet message` sections.
- [x] `concepts/overview.md` opens with a **Core terms** table defining fleet, root Director, member, placement, Administrator, broker, task/message, inline preview, poll/ack, and coding-agent backend, each with a link to its canonical page. Other pages link to it instead of re-defining terms.
- [x] The API Reference section has an `api/index.md` landing page stating who needs the Python API, and each of the four mkdocstrings stubs carries an orienting paragraph.
- [x] README.md and `docs/index.md` state who CAFleet is for, and README shows a compelling end-to-end example with expected output near the top.
- [x] A **Documentation style** section is published in `docs/get-started/contributing.md` (tone/voice, sample-id cast, example format, SSOT linking rule).
- [x] 0000079's SSOT map and necessity test are preserved: no canonical fact is restated in the new pages; `mise //:docs-build` passes with all nav entries and cross-references resolving.

---

## Background

0000079 (completed 2026-06-09) reduced `docs/` + README to ~2,344 lines, established a single-source-of-truth (SSOT) map and a strict necessity test, and stripped code symbols. This design **builds on top of it and must not contradict it**: every fact still has exactly one canonical home; new pages link to those homes rather than restating them. New friendliness content (examples, how-tos, glossary, troubleshooting) passes the necessity test — users need it to use cafleet.

Verified gaps in the current tree (each confirmed against the files on 2026-06-10):

1. **Shell-var contradiction**: `docs/get-started/quickstart.md` tells readers to `export FLEET_ID` / `export DIRECTOR_ID`, while `docs/spec/cli-options.md` § Option Source Matrix explicitly forbids shell variables (permission-pattern matching). The quickstart also passes `--full` to `fleet create` — a hidden flag (`hidden=True`, absent from `--help`) that does work but selects the verbose 7-line output; the default non-JSON output is the compact `<fleet_id> director=<id> admin=<id>` line, and `spec/cli-options.md` § `cafleet fleet create` is stale in presenting the 7-line block as the default shape.
2. **No task-oriented layer** between the one-screen quickstart and the exhaustive spec pages.
3. **`spec/cli-options.md` (669 lines)** has no scannable subcommand summary up front; moreover the `agent` and `message` subcommand groups have **no reference sections at all** on the page that `concepts/overview.md` calls "the canonical CLI surface".
4. **No glossary**: concepts pages assume fleet/member/Director/placement/broker/inline-preview terminology.
5. **API Reference** is four bare 3-line mkdocstrings stubs with no orienting prose and no landing page.
6. **No troubleshooting page**: errors are catalogued in `spec/cli-options.md` § Error Messages, but nothing maps symptoms to fixes; `cafleet doctor` is barely surfaced.
7. **README and `docs/index.md`** never state who CAFleet is for and show no end-to-end example early.

User decisions (relayed 2026-06-10): humans are the primary docs audience (skills/ stay agent-centric); the how-to section is top-level with the four proposed pages; the glossary is a Core terms **table on `concepts/overview.md`** (no new page); troubleshooting goes **under Get Started**; `cli-options.md` stays **one page** plus a summary table; the quickstart is rewritten to **literal ids**; worked examples use invented literal ids; the API Reference gets orienting prose plus a landing page and stays in nav; README gets who-for + end-to-end example with no length constraint; tone/voice guidelines are published in `contributing.md`.

---

## Specification

### Guiding principles

1. **Preserve 0000079.** The SSOT map and necessity test stand. New pages are *sequencing and orientation* layers: they order existing canonical facts into tasks and link to the canonical home for every flag, behavior, and error string. When a how-to page is tempted to explain a behavior, it links instead.
2. **Humans first in `docs/`.** Tone targets a developer reading the published site; `skills/` remain the agent-facing surface and are untouched by this design.
3. **Show, then tell.** Every walkthrough step is a runnable command followed by an expected-output block. Output blocks mirror the output shapes documented in `spec/cli-options.md` — if a shown output diverges from the spec page, the spec page wins and the example is wrong.
4. **No shell variables anywhere.** All examples paste literal integer ids and say once, near the first id, "your ids will differ — substitute the integers your commands print."
5. **One new fact rule.** The only *new normative content* this design introduces is: the symptom→fix guidance on the troubleshooting page, the Core terms definitions (one-line condensations with links), the Documentation style section, and the API-reference orientation prose. Everything else is reorganization, examples, and links.

### Sample-id cast (used by every example)

Aligned with the existing `fleet create --json` example in `spec/cli-options.md`:

| Id | Role |
|---|---|
| `1` | fleet (`--label "demo"`) |
| `2` | root Director |
| `3` | built-in Administrator |
| `4`+ | members, ids assigned in spawn order and named per example (e.g. `demo-member`, `reviewer`; the mixed-backend how-to uses `4`/`5`/`6` for its three backend members) |
| `10`+ | task ids |

### SSOT map extension (new rows; 0000079 rows unchanged)

| Topic | Canonical home (the ONE full statement) | Everywhere else |
|---|---|---|
| Task-oriented walkthroughs | `how-to/*.md` (sequencing only) | normative flags/behavior stay canonical in `spec/` and `concepts/`; how-to pages link |
| Symptom→fix mapping + `cafleet doctor` surfacing | `get-started/troubleshooting.md` | error strings stay canonical in `spec/cli-options.md` § Error Messages; troubleshooting links to them |
| Core terminology definitions | `concepts/overview.md` § Core terms | other pages link on first use instead of re-defining |
| Documentation style (tone, sample-id cast, example format) | `get-started/contributing.md` § Documentation style | applied silently everywhere |
| Python API orientation (who needs it) | `api/index.md` | the four stubs carry one orienting paragraph each and link back |

### Documentation style (the text to publish in `contributing.md`)

A short `## Documentation style` section at the end of `docs/get-started/contributing.md`, containing:

- **Audience split**: `docs/` is written for human developers and operators; `skills/` is written for coding agents. Do not mix the registers.
- **Voice**: second person ("you"), active voice, present tense. Lead each page with what the reader accomplishes, not with architecture.
- **Terms**: link a term's first use on a page to the Core terms table in `concepts/overview.md`; do not re-define it.
- **Examples**: every CLI example is a runnable command using the standard sample-id cast (fleet `1`, Director `2`, Administrator `3`, members `4`+) followed by an expected-output block matching the output shapes in `spec/cli-options.md`. Never use shell variables to hold ids.
- **SSOT**: one fact, one home (per design 0000079). When another page needs the fact, link; when a fact serves no install/configure/use/understand purpose, delete.

### Core terms table (top of `concepts/overview.md`)

Insert a `## Core terms` section immediately after the opening paragraph, before the architecture diagram. One row per term, one-sentence definition, link to the canonical page:

| Term | Definition (one line) | Links to |
|---|---|---|
| fleet | isolated namespace partitioning agents; identified by a non-secret integer `fleet_id` | `concepts/fleet-isolation.md` |
| root Director | the agent created by `fleet create`; the only agent that may own members | `concepts/member-lifecycle.md` |
| member | an agent spawned by the Director via `member create`, bound to a tmux pane | `concepts/member-lifecycle.md` |
| placement | the row linking an agent to its tmux session/window/pane and backend | `spec/data-model.md` |
| Administrator | the built-in write-only agent that the WebUI sends as | `spec/data-model.md` |
| broker | the data-access layer all CLI commands and the WebUI share; writes SQLite directly | `concepts/overview.md` (this page) |
| task / message | one delivered message; lifecycle `input_required → completed/canceled` | `spec/message-envelope.md` |
| inline preview | the 2-line message preview the broker keystrokes into the recipient's pane | `concepts/tmux-push.md` |
| poll / ack | how a recipient fetches and then confirms consumption of a message | `spec/cli-options.md` |
| coding-agent backend | the binary in a member pane: `claude`, `codex`, or `opencode` | `concepts/coding-agents.md` |

Definitions are one-line condensations, not new facts; the linked page remains canonical.

### New pages

#### `docs/how-to/index.md` (nav section index)

3–5 lines: this section answers "how do I …" questions with copy-paste walkthroughs; each guide links to the spec pages for the full flag surface. Bullet list of the four guides.

#### `docs/how-to/mixed-backend-team.md` — Run a mixed-backend team

1. Prerequisites: backend binaries on `PATH`, [Configure](../get-started/configure.md) done.
2. `cafleet fleet create --label "demo" --coding-agent claude` + output block (ids `1`/`2`; note the operator declares the backend of their own pane — link `concepts/coding-agents.md`).
3. Spawn one member per backend: three `member create` invocations (`--coding-agent claude|codex|opencode`) with literal ids + output.
4. Find the panes: `member list`; pane-title asymmetry (only `claude` panes show the member name) — link, not restate, `concepts/coding-agents.md` § Known asymmetries.
5. Message round-trip across backends: `message send` from Director `2` to members, expected envelope output.
6. Teardown: `member delete` per member, then `fleet delete 1`.

#### `docs/how-to/monitor-and-recover.md` — Monitor members and recover a stalled one

1. Watch the team: `cafleet --fleet-id 1 member list --activity` + the sample output block already in `spec/cli-options.md`; how to read `last_sent`/`last_recv`/`last_ack`/`idle`.
2. Inspect a quiet member: `member capture --member-id 4` (default 30 lines; `--lines N` for more).
3. The recovery ladder, mildest first (link `concepts/tmux-push.md` for the fallback-chain rationale):
   1. `member ping` — re-poke a member that missed an inline preview.
   2. `member send-input` — answer a pending prompt (`--choice 1..3` or `--freetext`).
   3. `member exec` — dispatch a shell command into the pane (link `concepts/bash-routing.md`).
   4. `member delete`, then `member delete --force` — last resort; include the 15 s `/exit` timeout output and its built-in recovery hint.
4. Every flag and exit code: link the `spec/cli-options.md` member sections.

#### `docs/how-to/use-the-webui.md` — Use the admin WebUI

1. Start it: `cafleet server` + the default URL `http://127.0.0.1:8000`; host/port overrides link to `spec/cli-options.md` § `cafleet server`. Note the dashboard is read/write for messages only — CLI commands never need the server running.
2. Pick a fleet → unified timeline (sidebar of agents, center timeline, bottom input).
3. Send as the Administrator: `@<agent> text` for unicast, `@all text` for broadcast.
4. Inspect history, including deregistered agents' inboxes (the WebUI is the only surface showing them — link `concepts/storage.md` § No physical cleanup).
5. API contracts: link `spec/webui-api.md`. Source-checkout note: `/` returns 404 until `mise //admin:build` has run (installed wheels bundle the built UI).

#### `docs/how-to/design-doc-development.md` — Drive design-doc-driven development

1. The three skills in order: `cafleet-design-doc-create` → `cafleet-design-doc-interview` → `cafleet-design-doc-execute`; one sentence each on what the team produces (reuse the descriptions in `get-started/contributing.md`, link rather than restate the numbered list).
2. Where output lands: `design-docs/NNNNNNN-<slug>/design-doc.md`; real examples in the repo.
3. Watch the team work: every inter-agent message is persisted — open the WebUI timeline (link the WebUI how-to).
4. Invocation syntax is per coding agent — same pointer sentence the quickstart uses today.

#### `docs/get-started/troubleshooting.md` — Troubleshooting

Opens with: "Start with `cafleet doctor`" — what it prints, when to reach for it (any placement/tmux confusion), and that it needs `TMUX`/`TMUX_PANE` set. Then a symptom→fix table (≥ 12 rows; quote the observable message, give the fix, link the canonical home). Initial row set:

| Symptom | Fix (one line) + link |
|---|---|
| `Error: cafleet fleet create must be run inside a tmux session` | run inside tmux; verify with `cafleet doctor` |
| `Error: cafleet member commands must be run inside a tmux session` | same as above |
| `OperationalError: no such table: agents` | run `cafleet db init` → [Install](install.md) |
| `cafleet db init` refuses an unknown schema | old UUID-era database; delete it → [Install](install.md) upgrade warning |
| `Error: --fleet-id <int> is required …` | create a fleet, pass the printed literal id → [CLI options](../spec/cli-options.md) |
| `Error: agent <id> is not a member of fleet <sid>.` | wrong `--fleet-id`/`--agent-id` pairing; recover ids via `cafleet fleet list` (DIRECTOR column) |
| permission prompts keep interrupting an agent | shell variables break `permissions.allow` matching — paste literal ids → [Configure](configure.md) |
| `Error: binary <name> not found on PATH` | install the backend → [Coding agents](../concepts/coding-agents.md) |
| member never reacts to messages | inline preview missed; `member list --activity`, then `member ping` → [monitor-and-recover](../how-to/monitor-and-recover.md) |
| `Error: pane %N did not close within 15.0s after /exit.` | follow the printed recovery hint; `--force` as last resort |
| WebUI `/` returns 404 | UI not built (source checkout): `mise //admin:build` → [use-the-webui](../how-to/use-the-webui.md) |
| `OSError: [Errno 98] Address already in use` on `cafleet server` | another process owns the port; `--port` → [CLI options](../spec/cli-options.md) |

The Error Messages table in `spec/cli-options.md` stays canonical for exact strings and exit codes; this page quotes symptoms only as table keys and links for the full surface. Close with a pointer: "symptom not listed → check the full [Error Messages](../spec/cli-options.md#error-messages) table."

#### `docs/api/index.md` — API Reference landing

One short page: the Python API matters to **contributors** changing cafleet and to **embedders** driving the broker from Python instead of the CLI; CLI users never need it. One line per module: `broker` (all agent/fleet/message operations), `config` (settings + env vars), `coding_agent` (backend abstraction), `multiplexer` (tmux abstraction). Generated from source docstrings via mkdocstrings.

### Per-file directives (existing files)

#### `docs/get-started/quickstart.md`

- **Rewrite the "Raw CLI walkthrough"** with literal ids: run `cafleet fleet create --label "demo"` (drop the hidden `--full` flag), show the default compact output line (`1 director=2 admin=3`, per the corrected shape in `spec/cli-options.md`), then paste the literal ids into every subsequent command — `--fleet-id 1` on all of them, plus `--agent-id 2` only on the commands that take an identity flag (`member create`, `message send`); `agent list` takes only `--fleet-id`. Include the one-time "your ids will differ" note.
- Show expected output for `member create`, `agent list` (member id `4` appears), and `message send` (compact envelope, task id `10`).
- Add one recovery tip: if the `fleet create` output scrolled away, `cafleet fleet list` re-prints the fleet id and the DIRECTOR column.
- End with next-step links: How-to guides and Troubleshooting (alongside the existing CLI-options link).
- Keep the two skill-driven sections (simple example, SDD) unchanged.

#### `docs/spec/cli-options.md`

- **Add a `## Subcommand summary` table** directly after the page intro (before the Option Source Matrix): one row per subcommand — `db init`, `fleet create/list/show/delete`, `doctor`, `server`, `agent register/deregister/list/show`, `message send/broadcast/poll/ack/cancel/show`, `member create/delete/list/capture/send-input/exec/ping` (24 rows) — with a one-line purpose, whether `--fleet-id` is required, the identity flag (`--agent-id` / `--member-id` / none), and a link to the subcommand's section on this page.
- **Add the two missing reference sections** so every summary row has a target: a compact `## cafleet agent` section (register / deregister / list / show: flag tables and output shapes) and a compact `## cafleet message` section (send / broadcast / poll / ack / cancel / show: flag tables; link `spec/message-envelope.md` for the envelope schema and the existing § Message Body Truncation for `--full`/truncation instead of restating either). This closes the gap where the page `concepts/overview.md` calls "the canonical CLI surface" had no agent/message sections at all.
- **Correct the § `cafleet fleet create` output documentation**: the default non-JSON output is the compact `<fleet_id> director=<id> admin=<id>` line; the 7-line block currently presented as the default is the hidden `--full` shape. Content correction only, no restructuring.
- **Document the `member create` output shape** in its existing section: the observable text (and `--json`) output as emitted by the current CLI, verified against the implementation while editing. Guiding principle 3 makes this page the arbiter of every shown output block, and both the quickstart rewrite and the mixed-backend how-to need `member create` expected-output blocks — the canonical shape must exist here first.
- No restructuring otherwise; the page stays one page and all existing anchors (`#full-semantics`, `#message-body-truncation`, `#error-messages`) are preserved.

#### `docs/concepts/overview.md`

- Insert the **Core terms** section specified above. No other change.

#### `docs/get-started/contributing.md`

- Append the **Documentation style** section specified above. No other change.

#### `docs/api/broker.md`, `config.md`, `coding-agent.md`, `multiplexer.md`

- Add one orienting paragraph above each `:::` directive: what the module is responsible for and who would read this page (one sentence each, consistent with `api/index.md`). The mkdocstrings directives are unchanged.

#### `README.md`

- After the video embed and tagline, add a short **"Who is CAFleet for"** block (3 bullets): developers running multi-agent coding teams in tmux who want every inter-agent message persisted and auditable; teams mixing `claude` / `codex` / `opencode` members in one fleet; operators who want a single-file SQLite broker with no server to run.
- Add a **"See it work"** end-to-end example before the Install section: a condensed literal-id walkthrough (fleet create → member create → message send → teardown, ~20 lines with expected-output snippets using the sample cast) so the value is visible on the first screen. The full version lives in the quickstart; the README block links to it.
- Keep all current sections (Install, Examples, Architecture, Contributing); renumber if needed. Fix the unclosed parenthesis in §1.1: the file currently reads "(for example, GitHub CLI `gh skill`, vercel `skills`, or marketplace of each coding agent." — close the parenthesis (and smooth the phrasing, e.g. "…or the marketplace of each coding agent).").
- Add Troubleshooting and How-to guide links alongside the existing published-docs links.

#### `docs/index.md`

- Add one "who it is for" sentence (condensed from the README block) to the intro.
- Extend the "Browse the docs" list with **How-to guides** and **Troubleshooting** entries.
- Re-point the Browse list's API Reference entry from `api/broker.md` to `api/index.md` (the new section landing page, matching how the Get Started entry links its section index).

#### `docs/get-started/index.md`

- Add a Troubleshooting bullet to the section list.

#### `CONTRIBUTING.md` (repo root)

- **Verification only**: the 4-line pointer to the published contributor guide remains accurate; no change expected.

#### `zensical.toml`

Nav changes (only the `nav` array is touched):

```toml
nav = [
    { "Get Started" = [
        "get-started/index.md",
        { "Install" = "get-started/install.md" },
        { "Configure" = "get-started/configure.md" },
        { "Quickstart" = "get-started/quickstart.md" },
        { "Troubleshooting" = "get-started/troubleshooting.md" },
        { "Contributing" = "get-started/contributing.md" },
    ] },
    { "How-to guides" = [
        "how-to/index.md",
        { "Run a mixed-backend team" = "how-to/mixed-backend-team.md" },
        { "Monitor and recover members" = "how-to/monitor-and-recover.md" },
        { "Use the admin WebUI" = "how-to/use-the-webui.md" },
        { "Design-doc-driven development" = "how-to/design-doc-development.md" },
    ] },
    # Concepts and Specification sections unchanged
    { "API Reference" = [
        "api/index.md",
        { "broker" = "api/broker.md" },
        { "config" = "api/config.md" },
        { "coding_agent" = "api/coding-agent.md" },
        { "multiplexer" = "api/multiplexer.md" },
    ] },
]
```

The `navigation.indexes` feature is already enabled, so `how-to/index.md` and `api/index.md` attach as section index pages the same way `get-started/index.md` does.

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.
> After all edits, run `mise //:docs-build` to confirm nav + cross-references resolve.

### Step 1: Foundations (style + terms)

- [x] `docs/get-started/contributing.md` — append the Documentation style section (audience split, voice, term linking, example format with the sample-id cast, SSOT rule). <!-- completed: 2026-06-11T08:52 -->
- [x] `docs/concepts/overview.md` — insert the Core terms table after the opening paragraph. <!-- completed: 2026-06-11T08:52 -->

### Step 2: Quickstart fix + CLI reference scannability

- [x] `docs/get-started/quickstart.md` — rewrite the raw CLI walkthrough with literal ids (no shell vars), drop the hidden `--full` on `fleet create`, add expected-output blocks, the "your ids will differ" note, the `fleet list` recovery tip, and next-step links. <!-- completed: 2026-06-11T09:05 -->
- [x] `docs/spec/cli-options.md` — add the Subcommand summary table (24 rows with section links) after the page intro. <!-- completed: 2026-06-11T09:05 -->
- [x] `docs/spec/cli-options.md` — add the compact `cafleet agent` and `cafleet message` reference sections (flag tables + output shapes; link message-envelope.md and § Message Body Truncation instead of restating), and document the `member create` output shape in its existing section (verified against the implementation). <!-- completed: 2026-06-11T09:05 -->

### Step 3: How-to guides

- [x] `docs/how-to/index.md` — section index page. <!-- completed: 2026-06-11T09:12 -->
- [x] `docs/how-to/mixed-backend-team.md` — per the outline (prereqs → fleet create → three member creates → pane discovery → cross-backend round-trip → teardown). <!-- completed: 2026-06-11T09:12 -->
- [x] `docs/how-to/monitor-and-recover.md` — per the outline (`member list --activity` → `capture` → recovery ladder ping/send-input/exec/delete[-f]). <!-- completed: 2026-06-11T09:12 -->
- [x] `docs/how-to/use-the-webui.md` — per the outline (`cafleet server` → fleet picker → timeline → @-send → history; webui-api link; 404/build note). <!-- completed: 2026-06-11T09:12 -->
- [x] `docs/how-to/design-doc-development.md` — per the outline (three skills → output location → WebUI audit trail → invocation pointer). <!-- completed: 2026-06-11T09:12 -->

### Step 4: Troubleshooting

- [x] `docs/get-started/troubleshooting.md` — `cafleet doctor` lead + the ≥ 12-row symptom→fix table + Error Messages pointer. <!-- completed: 2026-06-11T09:15 -->
- [x] `docs/get-started/index.md` — add the Troubleshooting bullet. <!-- completed: 2026-06-11T09:15 -->

### Step 5: API Reference orientation

- [x] `docs/api/index.md` — landing page (who needs the Python API; one line per module). <!-- completed: 2026-06-11T09:17 -->
- [x] `docs/api/{broker,config,coding-agent,multiplexer}.md` — one orienting paragraph above each `:::` directive. <!-- completed: 2026-06-11T09:17 -->

### Step 6: Landing surfaces

- [x] `README.md` — add "Who is CAFleet for" + the condensed "See it work" example with output; fix the §1.1 unclosed parenthesis; add How-to / Troubleshooting links; keep and renumber existing sections. <!-- completed: 2026-06-11T09:20 -->
- [x] `docs/index.md` — add the who-for sentence, extend the Browse list with How-to guides and Troubleshooting, and re-point the API Reference entry to `api/index.md`. <!-- completed: 2026-06-11T09:20 -->
- [x] `CONTRIBUTING.md` (root) — verify the pointer needs no change. <!-- completed: 2026-06-11T09:20 -->

### Step 7: Nav + verification

- [x] `zensical.toml` — apply the nav additions (Troubleshooting entry, How-to guides section, `api/index.md` section index). <!-- completed: 2026-06-11T09:23 -->
- [x] Run `mise //:docs-build`; confirm a clean build with all nav entries and cross-references resolving. <!-- completed: 2026-06-11T09:23 -->
- [x] Consistency greps: `grep -rE "export (FLEET|DIRECTOR)_ID" docs/ README.md` empty; `grep -rn '\-\-full' docs/get-started/quickstart.md` shows no `fleet create --full`; spot-check that the sample-id cast (`--fleet-id 1`, `--agent-id 2`) is used consistently across README, quickstart, and how-to pages. <!-- completed: 2026-06-11T09:23 -->

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-06-10 | Initial draft |
| 2026-06-10 | Reviewer round 1: broadened the shell-var acceptance grep to cover DIRECTOR_ID; unified the troubleshooting row threshold at ≥ 12; made the member-id cast open-ended; scoped the quickstart literal-id instruction to commands that take an identity flag; added a directive to document the `member create` output shape in cli-options; corrected the README §1.1 verbatim quote; re-pointed the docs/index.md API Reference link to `api/index.md`. |
| 2026-06-11 | Director arbitration (Step 2): `fleet create` accepts a hidden `--full` flag and its default output is the compact `<fleet_id> director=<id> admin=<id>` line — corrected Background §1, the quickstart directive (compact default output), and added a cli-options directive to fix the stale § fleet create output shape. |
