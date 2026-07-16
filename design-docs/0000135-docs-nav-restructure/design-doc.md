# Docs-Site Navigation Restructure: Flatten Get Started

**Status**: Approved
**Progress**: 6/18 tasks complete
**Last Updated**: 2026-07-16

## Overview

Flatten the docs-site navigation: promote Quickstart to a simple top-level page, remove the Get Started section entirely (deleting its Overview page), and move Contributing to the tail of the nav, placed before API Reference. Every cross-link into the old `get-started/` paths — in `docs/`, `README.md`, `CONTRIBUTING.md`, and the `update-readme` project skill — is updated in the same change.

## Success Criteria

- [ ] `zensical.toml` nav renders exactly six top-level entries in order: Quickstart, How-to guides, Concepts, Specification, Contributing, API Reference — with Quickstart and Contributing as simple pages (no sub-section).
- [ ] `docs/get-started/` no longer exists; `docs/quickstart.md` and `docs/contributing.md` are the pages' new homes; the old Overview page is deleted with nothing salvaged.
- [ ] A repo-wide sweep finds zero `get-started` references outside `design-docs/` and git history.
- [ ] `mise //:docs-build` completes cleanly with no broken-link warnings.
- [ ] `README.md`, `CONTRIBUTING.md`, `docs/index.md`, and `.claude/skills/update-readme/SKILL.md` reflect the new structure (README's Documentation bullets and Full-guide URL, CONTRIBUTING.md's published-guide URL and source path, the home page's button and Browse list, the skill's section list and quickstart path).

---

## Background

The nav currently opens with a three-page "Get Started" section (Overview, Quickstart, Contributing). The Overview page (`docs/get-started/index.md`) is a two-sentence intro plus links to its two siblings — pure navigation overhead — and Contributing is contributor-facing material that does not belong at the front of a user-facing nav. The restructure gives first-time users a one-click Quickstart and moves Contributing next to the other tail-of-nav reference material.

Decisions confirmed with the user:

| Decision | Resolution |
|---|---|
| File placement | Move files top-level: `docs/get-started/quickstart.md` → `docs/quickstart.md`, `docs/get-started/contributing.md` → `docs/contributing.md`; remove `docs/get-started/` entirely. |
| Old published URLs | Hard break, no redirect stubs. `/get-started/`, `/get-started/quickstart/`, `/get-started/contributing/` will 404. |
| Home-page button | Retarget to `quickstart/` and relabel to "Quickstart". |
| Home-page Browse list | Mirror the new six-entry nav order with separate Quickstart and Contributing bullets. |
| README + update-readme skill | Both in scope: README gets Quickstart (first) and Contributing (before API Reference) bullets plus the fixed Full-guide URL; the skill's hard-coded section list and quickstart path are updated so it cannot regenerate the stale structure. |
| Overview page content | Deleted outright; nothing salvaged. |
| Verification | `mise //:docs-build` clean with no broken-link warnings + manual sweep for zero remaining `get-started` references outside design docs and git history. |

---

## Specification

### File moves

| Current path | New path |
|---|---|
| `docs/get-started/quickstart.md` | `docs/quickstart.md` |
| `docs/get-started/contributing.md` | `docs/contributing.md` |
| `docs/get-started/index.md` | deleted |

Both moved pages keep their front-matter icons (`lucide/zap`, `lucide/heart-handshake`) and all content; only relative links change (see below).

### New nav (`zensical.toml`)

The `nav` array becomes:

```toml
nav = [
    { "Quickstart" = "quickstart.md" },
    { "How-to guides" = [
        { "Overview" = "how-to/index.md" },
        { "Run a mixed-backend team" = "how-to/mixed-backend-team.md" },
        { "Monitor and recover members" = "how-to/monitor-and-recover.md" },
        { "Use the admin WebUI" = "how-to/use-the-webui.md" },
        { "Design-doc-driven development" = "how-to/design-doc-development.md" },
    ] },
    { "Concepts" = [
        { "Overview" = "concepts/overview.md" },
        { "Fleet isolation" = "concepts/fleet-isolation.md" },
        { "Storage" = "concepts/storage.md" },
        { "Member lifecycle" = "concepts/member-lifecycle.md" },
        { "Coding agents" = "concepts/coding-agents.md" },
        { "Monitoring" = "concepts/monitoring.md" },
    ] },
    { "Specification" = [
        { "Data model" = "spec/data-model.md" },
        { "Message envelope" = "spec/message-envelope.md" },
        { "CLI options" = "spec/cli-options.md" },
        { "Multiplexer backends" = "spec/multiplexer-backends.md" },
        { "WebUI API" = "spec/webui-api.md" },
        { "Coding-agent backends" = [
            { "Overview" = "reference/coding-agents/index.md" },
            { "Claude members" = "reference/coding-agents/claude.md" },
            { "Codex members" = "reference/coding-agents/codex.md" },
            { "Opencode members" = "reference/coding-agents/opencode.md" },
        ] },
    ] },
    { "Contributing" = "contributing.md" },
    { "API Reference" = [
        { "broker" = "api/broker.md" },
        { "config" = "api/config.md" },
        { "coding_agent" = "api/coding-agent.md" },
        { "multiplexer" = "api/multiplexer.md" },
    ] },
]
```

The How-to guides, Concepts, Specification, and API Reference sections are unchanged; only the leading Get Started section is replaced by the top-level Quickstart page and the top-level Contributing page is inserted before API Reference.

### URL surface change

| Old published URL | New published URL |
|---|---|
| `https://himkt.github.io/cafleet/get-started/quickstart/` | `https://himkt.github.io/cafleet/quickstart/` |
| `https://himkt.github.io/cafleet/get-started/contributing/` | `https://himkt.github.io/cafleet/contributing/` |
| `https://himkt.github.io/cafleet/get-started/` | none (404; hard break, no redirects) |

### Relative links inside the moved pages

Both pages move up one directory level, so every `../<dir>/` link drops its `../` prefix. Line numbers are as of this writing.

`docs/quickstart.md` (formerly `docs/get-started/quickstart.md`):

| Line | Old link target | New link target |
|---|---|---|
| 18 | `../spec/multiplexer-backends.md` | `spec/multiplexer-backends.md` |
| 127 | `../reference/coding-agents/opencode.md` | `reference/coding-agents/opencode.md` |
| 286 | `../spec/cli-options.md` | `spec/cli-options.md` |
| 287 | `../how-to/index.md` | `how-to/index.md` |

`docs/contributing.md` (formerly `docs/get-started/contributing.md`):

| Line | Old link target | New link target |
|---|---|---|
| 115 | `../concepts/overview.md#core-terms` | `concepts/overview.md#core-terms` |
| 120 | `../spec/cli-options.md` | `spec/cli-options.md` |

The absolute GitHub URL on line 103 of `contributing.md` and all in-page anchors are unaffected.

### Inbound cross-link updates in `docs/`

| File | Line | Old target | New target |
|---|---|---|---|
| `docs/spec/cli-options.md` | 189 | `../get-started/quickstart.md#install` | `../quickstart.md#install` |
| `docs/how-to/mixed-backend-team.md` | 17 | `../get-started/quickstart.md#install` | `../quickstart.md#install` |
| `docs/how-to/mixed-backend-team.md` | 18 | `../get-started/quickstart.md#configure` | `../quickstart.md#configure` |
| `docs/concepts/overview.md` | 91 | `../get-started/quickstart.md` | `../quickstart.md` |
| `docs/reference/coding-agents/codex.md` | 46 | `../../get-started/quickstart.md#trust-the-working-directory` | `../../quickstart.md#trust-the-working-directory` |
| `docs/how-to/design-doc-development.md` | 36 | `../get-started/contributing.md` | `../contributing.md` |

### `docs/index.md`

Two edits:

1. The primary button (line 24) becomes:

   ```markdown
   [Quickstart :material-arrow-right:](quickstart/){ .md-button .md-button--primary }
   ```

2. The "Browse the docs" list (lines 28–32) mirrors the new nav order:

   ```markdown
   - [Quickstart](quickstart.md) — install, configure, and run your first fleet.
   - [How-to guides](how-to/) — prompt-first task guides for common workflows.
   - [Concepts](concepts/overview.md) — architecture and the ideas behind it.
   - [Specification](spec/data-model.md) — data model, message envelope, CLI, multiplexer backends, WebUI API, and coding-agent backends.
   - [Contributing](contributing.md) — project layout, local development loop, and the design-doc-driven contribution flow.
   - [API Reference](api/broker.md) — Python API generated from source.
   ```

### `README.md`

Two edits (the docs-site section links are README's thin surface, so this is a required sync per `documentation-maintenance.md`):

1. The Install section's Full-guide link (line 14) becomes:

   ```markdown
   Full guide: <https://himkt.github.io/cafleet/quickstart/>
   ```

2. The Documentation section replaces the "Get Started" bullet with Quickstart (first) and Contributing (before API Reference), matching nav order:

   ```markdown
   - [Quickstart](https://himkt.github.io/cafleet/quickstart/) — install, configure, and run your first fleet.
   - [How-to guides](https://himkt.github.io/cafleet/how-to/) — prompt-first task guides.
   - [Concepts](https://himkt.github.io/cafleet/concepts/overview/) — architecture and the ideas behind it.
   - [Specification](https://himkt.github.io/cafleet/spec/data-model/) — data model, message envelope, CLI, multiplexer backends, WebUI API, coding-agent backends.
   - [Contributing](https://himkt.github.io/cafleet/contributing/) — project layout, local development loop, and the contribution flow.
   - [API Reference](https://himkt.github.io/cafleet/api/broker/) — Python API generated from source.
   ```

### `.claude/skills/update-readme/SKILL.md`

The project skill hard-codes the old structure and would regenerate it on its next run. Two edits:

| Line | Old text | New text |
|---|---|---|
| 34 | `... the Install block with the Install section of docs/get-started/quickstart.md ...` | `... the Install block with the Install section of docs/quickstart.md ...` |
| 44 | `Links to the docs-site sections (Get Started, How-to guides, Concepts, Specification, API Reference)` | `Links to the docs-site sections (Quickstart, How-to guides, Concepts, Specification, Contributing, API Reference)` |

### `CONTRIBUTING.md` (repo root)

The root stub points contributors at the published guide and its source page; both lines retarget:

| Line | Old text | New text |
|---|---|---|
| 3 | `https://himkt.github.io/cafleet/get-started/contributing/` | `https://himkt.github.io/cafleet/contributing/` |
| 4 | `docs/get-started/contributing.md` | `docs/contributing.md` |

### Out of scope

- `SPEC.md`, `skills/` (the shipped agent skills), `admin/`, and `.github/` contain no `get-started` references (verified by sweep); no edits there.
- No redirect mechanism for the old published URLs (decision above).
- No content changes to any page beyond the link and label edits specified here.

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

### Step 1: Move and delete pages

- [x] `git mv docs/get-started/quickstart.md docs/quickstart.md` <!-- completed: 2026-07-16T13:58 -->
- [x] `git mv docs/get-started/contributing.md docs/contributing.md` <!-- completed: 2026-07-16T13:58 -->
- [x] `git rm docs/get-started/index.md` (removes the now-empty `docs/get-started/`) <!-- completed: 2026-07-16T13:58 -->

### Step 2: Fix relative links inside the moved pages

- [x] `docs/quickstart.md`: drop the `../` prefix on the 4 links listed in the Specification <!-- completed: 2026-07-16T14:02 -->
- [x] `docs/contributing.md`: drop the `../` prefix on the 2 links listed in the Specification <!-- completed: 2026-07-16T14:02 -->

### Step 3: Update the nav

- [x] Replace the `nav` array in `zensical.toml` with the block in the Specification <!-- completed: 2026-07-16T14:11 -->

### Step 4: Update `docs/index.md`

- [ ] Retarget and relabel the primary button to `[Quickstart :material-arrow-right:](quickstart/)` <!-- completed: -->
- [ ] Replace the "Browse the docs" list with the six-entry list in the Specification <!-- completed: -->

### Step 5: Update inbound cross-links in `docs/`

- [ ] `docs/spec/cli-options.md` — Quickstart install link <!-- completed: -->
- [ ] `docs/how-to/mixed-backend-team.md` — Quickstart install + configure links <!-- completed: -->
- [ ] `docs/concepts/overview.md` — Quickstart link <!-- completed: -->
- [ ] `docs/reference/coding-agents/codex.md` — Quickstart trust-the-working-directory link <!-- completed: -->
- [ ] `docs/how-to/design-doc-development.md` — Contributing link <!-- completed: -->

### Step 6: Update `README.md`, `CONTRIBUTING.md`, and the update-readme skill

- [ ] `README.md` — Full-guide URL and the six-bullet Documentation section per the Specification <!-- completed: -->
- [ ] `CONTRIBUTING.md` — retarget the published-guide URL (line 3) and source path (line 4) <!-- completed: -->
- [ ] `.claude/skills/update-readme/SKILL.md` — quickstart path (line 34) and section list (line 44) <!-- completed: -->

### Step 7: Verification

- [ ] `mise //:docs-build` completes cleanly with no broken-link warnings <!-- completed: -->
- [ ] Repo-wide sweep confirms zero `get-started` references outside `design-docs/` and git history <!-- completed: -->

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-07-16 | Initial draft |
