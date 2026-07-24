# Remove Implementation File Path Mentions from User-Facing Docs

**Status**: Approved
**Progress**: 10/10 tasks complete
**Last Updated**: 2026-07-25

## Overview

The `docs/` site mentions concrete repository file paths (source files, module dotted paths, skill-internal reference files) that are noise for users of the tool. This design rewrites each in-scope mention into a self-contained behavioral explanation, and adds a project rule (`.claude/rules/user-facing-docs.md`) prescribing that user-facing docs describe behavior without referencing implementation file paths.

## Success Criteria

- [ ] Every in-scope path mention in the inventory below is rewritten per its specification — no mere deletions; each replacement is a self-contained explanation.
- [ ] All exempt-class mentions (contributor instructions, `docs/api/*`, path-as-contract spec mentions, user-machine paths, user-repo deliverable locations) are untouched.
- [ ] `.claude/rules/user-facing-docs.md` exists with the content specified below, and `.claude/rules/documentation-maintenance.md` carries a one-line cross-reference to it.
- [ ] A final sweep of `docs/` finds no repo-internal implementation path outside the exempt classes.
- [ ] `mise //:docs-build` succeeds after the edits.

---

## Background

A full scan of `docs/` (index, quickstart, contributing, concepts/, how-to/, api/, spec/) found repo-path mentions concentrated in four places: `contributing.md` (14, nearly all genuine contributor instructions), `api/broker.md` and the other `api/*` pages (implementation index with mkdocstrings `:::` directives), `spec/*` (mostly path-as-contract: release preset paths, install tables), and a scattering of skill-internal pointers and file-name diagram labels in `concepts/` and `how-to/`. Ten pages — including `index.md` and `quickstart.md` — already carry zero repo-path mentions.

The user's decisions (relayed via the Director) fix the scope:

| Question | Decision |
|---|---|
| `contributing.md` | In scope with a documented exemption: paths stay where the path IS the contributor instruction; incidental implementation pointers are cleaned up. |
| `docs/api/*` | Exempt entirely — implementation-facing API reference; left as is. |
| `docs/spec/*` | Path-as-contract mentions stay; only incidental pointers are rewritten. |
| Skill-internal pointers (`reference/supervision.md` etc.) | Remove and rewrite — explain the mechanism inline; agents load skills by name. |
| `design-docs/NNNNNNN-<slug>/design-doc.md` in how-to | Keep — a deliverable location in the user's own repo. |
| Mermaid file-name node labels in `concepts/overview.md` | Relabel with component names. |
| Enforcement | Prose rule only — no automated guard. |
| Rule placement | Standalone `.claude/rules/user-facing-docs.md` + one-line cross-reference from `documentation-maintenance.md`. |

---

## Specification

### Mention taxonomy

| Class | Disposition | Rationale |
|---|---|---|
| Implementation pointer in user-facing prose (source file, module dotted path, skill-internal reference file) | **Rewrite** | Users need the behavior, not the code location. |
| Contributor instruction in `contributing.md` where the path IS the instruction | **Keep** | The page's job is pointing contributors at files. |
| `docs/api/*` content (mkdocstrings `:::` directives, submodule maps, module paths) | **Keep** | The API reference's subject matter is the implementation. |
| Path-as-contract in `docs/spec/*` (release preset paths + install targets, `SPEC.md` as DDL SSOT) | **Keep** | The path is part of the specified contract. |
| User-machine runtime/install paths (`~/...`, DB locations) | **Keep** | User-facing facts about the reader's own machine. |
| Deliverable locations in the user's own repo (`design-docs/NNNNNNN-<slug>/design-doc.md`) | **Keep** | Output layout the user interacts with. |
| Docs-page-to-docs-page navigation links (including `#cafleet.broker.*` anchors into the exempt `api/` pages) | **Keep** | Site navigation, not implementation pointers. |

### Full inventory and dispositions

Scan result: 11 in-scope mentions across 6 files — 3 in `overview.md`, 2 in `model-selection.md`, 3 in `monitoring.md`, and 1 each in `mixed-backend-team.md`, `multiplexer-backends.md`, and `cli-options.md` (9 rewrite rows below; the `monitoring.md` row bundles its 3 occurrences). Everything else keeps its current text.

| Location | Path mentioned | Disposition |
|---|---|---|
| `concepts/overview.md:34` | `broker/` (mermaid node label) | Rewrite (relabel) |
| `concepts/overview.md:35` | `webui/app.py` (mermaid node label) | Rewrite (relabel) |
| `concepts/overview.md:36` | `webui/api.py` (mermaid node label) | Rewrite (relabel) |
| `concepts/model-selection.md:15` | `skills/cafleet/reference/model-list.md` | Rewrite |
| `concepts/model-selection.md:69` | `reference/director.md` | Rewrite |
| `concepts/monitoring.md:21-22`, table row `:27`, `:44` | `reference/supervision.md` (×3) | Rewrite |
| `how-to/mixed-backend-team.md:31-32` | `reference/supervision.md` | Rewrite |
| `spec/multiplexer-backends.md:8-9` | `cafleet.multiplexer.base` | Rewrite (incidental pointer; the Protocol's API home is already linked elsewhere on the page) |
| `spec/cli-options.md:640-641` | `reference/prompt-routing.md` | Rewrite (incidental pointer) |
| `contributing.md` (all 14 mentions) | project-structure table, `admin/package.json` + `admin/bun.lock` recipe, `design-docs/NNNNNNN-<slug>/` layout, `docs/`-vs-`skills/` audience split | Keep — every mention is a genuine contributor instruction; the scan found no incidental implementation pointers on this page |
| `api/broker.md`, `api/coding-agent.md`, `api/config.md`, `api/multiplexer.md` | all | Keep (exempt page class) |
| `spec/coding-agent-backends.md:88`, `:133`; `spec/cli-options.md:262-263` | `presets/codex/cafleet.rules`, `presets/opencode/cafleet.md` | Keep (path-as-contract: release-archive install sources) |
| `spec/data-model.md:13` | `SPEC.md` | Keep — a cross-reference to the authoritative DDL specification document, not an implementation path |
| `how-to/design-doc-development.md:21`, `:28`, `:40` | `design-docs/NNNNNNN-<slug>[/design-doc.md]` | Keep (deliverable location in the user's repo) |

### Rewrite specifications

Each rewrite replaces the path with a self-contained statement of the behavior or an attribution to the shipping artifact (the skill users install), never a bare deletion.

#### R1 — `concepts/overview.md` mermaid architecture diagram

Relabel the three file-path nodes with component names. `broker` is already defined in the page's Core terms table as "the data-access layer all CLI commands and the WebUI share", so the label reuses the term:

| Current node | New node |
|---|---|
| `Broker["broker/<br/>(sync SQLAlchemy)"]` | `Broker["broker<br/>(sync SQLAlchemy)"]` |
| `Server["webui/app.py<br/>(FastAPI)"]` | `Server["FastAPI server"]` |
| `WebUIAPI["webui/api.py"]` | `WebUIAPI["WebUI API layer"]` |

Edge definitions keep the same node ids (`Broker`, `Server`, `WebUIAPI`); only the bracketed labels change.

#### R2 — `concepts/model-selection.md` § The model list

Current (line 15-16):

> The model list lives at `skills/cafleet/reference/model-list.md` in every deployed cafleet skill replica. It is a catalog-style reference page: …

New:

> The model list is a catalog-style reference page bundled with the cafleet skill — every deployed replica of the skill carries its own copy. It has one table per backend (`claude`, `codex`, and `opencode`), …

(The second sentence of the current paragraph merges into the rewritten opening; the rest of the paragraph is unchanged. The read-before-spawn fact stays with the page's opening paragraph, which already owns it.)

#### R3 — `concepts/model-selection.md` § Replacement

Current (lines 68-69):

> The procedure is specified in the cafleet skill's `reference/director.md`.

New:

> The full step-by-step procedure is part of the cafleet skill's Director instructions, which the Director loads with the skill.

#### R4 — `concepts/monitoring.md` § Heartbeat vs facilitation (three occurrences)

| Current | New |
|---|---|
| "Everything requiring agent judgment stays the Director's job, defined in the `/cafleet` skill's `reference/supervision.md`:" (lines 20-22) | "Everything requiring agent judgment stays the Director's job, defined by the cafleet skill's supervision protocol:" |
| Table row: "the Director, per `/cafleet` `reference/supervision.md`" (line 27) | "the Director, per the cafleet skill's supervision protocol" |
| "The full gate lives in the `/cafleet` skill's `reference/supervision.md`." (line 44) | "The full gate is part of the cafleet skill's supervision protocol, which the Director follows whenever it re-engages a member." |

#### R5 — `how-to/mixed-backend-team.md` § Prompt

Current (lines 31-32):

> Your agent loads the `cafleet` skill and reads its Director-only `reference/supervision.md` before spawning members.

New:

> Your agent loads the `cafleet` skill and follows its Director-only supervision protocol before spawning members.

#### R6 — `spec/multiplexer-backends.md` opening paragraph

Current (lines 8-9):

> The multiplexer is abstracted behind the `Multiplexer` Protocol (`cafleet.multiplexer.base`), so the spawn, keystroke-delivery, capture, and teardown paths are backend-neutral.

New:

> The multiplexer is abstracted behind the `Multiplexer` Protocol, so the spawn, keystroke-delivery, capture, and teardown paths are backend-neutral.

Dropping the parenthetical is not a bare deletion in the rule's sense: the sentence stays self-contained (the Protocol is named), and navigation is already served by the identical `[API reference](../api/multiplexer.md)` link later on the same page (§ Backend selection).

#### R7 — `spec/cli-options.md` § `member prompt`

Current (lines 639-641):

> … it is the dispatch half of the bash-via-Director fallback protocol, canonical in the cafleet skill's `reference/prompt-routing.md`.

New:

> … it is the dispatch half of the cafleet skill's bash-via-Director fallback protocol.

### New rule: `.claude/rules/user-facing-docs.md`

Full file content:

```markdown
# User-Facing Docs

`docs/` is the user- and operator-facing documentation site. Its pages explain
what the tool does and how to use it — in terms of behavior, concepts, and
user-visible artifacts, never in terms of where the implementation lives.

## Describe behavior, not code locations

- Name components by concept ("the broker", "the FastAPI server", "the
  supervision protocol"), anchored to the Core terms table in the concepts
  overview — not by source file or module path.
- When a page needs to say where a protocol or catalog is defined, attribute
  it to the shipping artifact the reader has ("part of the cafleet skill",
  "bundled with every deployed skill replica") without the repo-internal file
  path.
- When removing a path mention, replace it with a self-contained explanation
  of the behavior or a link to the docs page that owns the fact — never a bare
  deletion that leaves the sentence emptier.

## Exemptions — where a concrete path is the content

| Surface | Why paths are legitimate there |
|---|---|
| `docs/contributing.md` | Contributor instructions: paths stay where the path IS the instruction (project-structure table, dependency-edit recipes, design-doc layout). Incidental implementation pointers are still rewritten. |
| `docs/api/*` | Implementation-facing API reference (mkdocstrings) — module and file paths are its subject matter. |
| Path-as-contract mentions in `docs/spec/*` | Paths that are part of a specified contract (release-archive preset paths and their install targets, the `SPEC.md` DDL source of truth). Incidental pointers are still rewritten. |
| User-machine paths | `~/...` install and runtime locations describe the reader's own machine. |
| Deliverable locations in the user's repo | e.g. `design-docs/NNNNNNN-<slug>/design-doc.md` as a workflow output layout. |
```

### Cross-reference in `.claude/rules/documentation-maintenance.md`

Append one line to the § *First-class documentation targets* section:

> How `docs/` pages reference — or avoid referencing — repository file paths is governed by `user-facing-docs.md`.

### Out-of-scope surfaces

`README.md`, `SPEC.md`, and `skills/*/SKILL.md` need no edits: the rewrites change no CLI, configuration, or contract surface, and skills are agent-facing (outside the new rule's scope by its own audience definition).

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

### Step 1: Add the project rule (documentation first)

- [x] Create `.claude/rules/user-facing-docs.md` with the content in § *New rule* <!-- completed: 2026-07-25T00:42 -->
- [x] Append the one-line cross-reference to `.claude/rules/documentation-maintenance.md` § *First-class documentation targets* <!-- completed: 2026-07-25T00:42 -->

### Step 2: Rewrite concepts/ pages

- [x] Apply R1 to `docs/concepts/overview.md` (three mermaid node labels) <!-- completed: 2026-07-25T00:44 -->
- [x] Apply R2 and R3 to `docs/concepts/model-selection.md` <!-- completed: 2026-07-25T00:44 -->
- [x] Apply R4 to `docs/concepts/monitoring.md` (three occurrences) <!-- completed: 2026-07-25T00:44 -->

### Step 3: Rewrite how-to/ and spec/ pages

- [x] Apply R5 to `docs/how-to/mixed-backend-team.md` <!-- completed: 2026-07-25T00:45 -->
- [x] Apply R6 to `docs/spec/multiplexer-backends.md` <!-- completed: 2026-07-25T00:45 -->
- [x] Apply R7 to `docs/spec/cli-options.md` <!-- completed: 2026-07-25T00:45 -->

### Step 4: Verify

- [x] Sweep `docs/` for repo-internal path patterns (`.py`, `.md` outside docs-nav links, `src/`, `skills/`, `reference/`, module dotted paths) and confirm every remaining hit belongs to an exempt class in the taxonomy table <!-- completed: 2026-07-25T00:48 -->
- [x] Run `mise //:docs-build` and confirm the site builds cleanly <!-- completed: 2026-07-25T00:48 -->

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-07-24 | Initial draft |
| 2026-07-24 | Reviewer round 1: corrected in-scope mention count to 11; trimmed R2's duplicated read-before-spawn clause; R6 drops the redundant API-reference link instead of duplicating it |
