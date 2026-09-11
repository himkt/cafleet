# Unify the coding-agent reference

**Status**: Complete
**Progress**: 9/9 tasks complete
**Last Updated**: 2026-09-11

## Overview

Merge the CAFleet model catalog and coding-agent overlays into `skills/cafleet/reference/coding-agents.md`, with a self-contained section for each backend. Store backend role defaults once and make readers select the appropriate backend for executing instructions, spawning a member, or observing a member. Keep model-selection policy in `skills/cafleet/roles/director.md` and preserve the existing catalog and operational behavior.

## Success Criteria

- [x] The unified reference contains complete `claude`, `codex`, `opencode`, and `Template` sections, with each backend's runtime bindings, role defaults, model catalog, notes, capture cues, and worked resolution.
- [x] Each concrete reviewer and monitor default has one authoritative definition per backend, and every consumer resolves it for the intended spawn backend.
- [x] Existing model rows, aliases, classes, ordering, prices, context notes, source URLs, runtime details, and the `2026-09-05` catalog freshness date are preserved.
- [x] Reader instructions distinguish executing-agent runtime resolution, spawned-member selection, and observed-member capture interpretation; mixed-backend examples resolve consistently.
- [x] Current repository surfaces reference the unified file; both input files are deleted, with no redirects or historical notices in current documentation.
- [x] The refresh skill and authoring guidance describe the unified ownership and structure, and existing documentation checks pass after their reference paths are updated.

---

## Background

`reference/model-list.md` owns the model tables and a cross-backend defaults table. `reference/coding-agent-overlays.md` repeats those defaults in each backend's placeholder table, requiring the refresh skill to synchronize two files. The authoring rule explicitly describes this duplication.

The runtime reader contract currently says to resolve only the reader's own backend, while the Director's selection policy permits choosing another backend for a member. A single reference needs an explicit purpose for each lookup so a Director can choose another backend's model while continuing to use its own execution tools and decision surface. Capture interpretation already uses the observed member's backend and should remain a separate lookup.

Repository consumers include core and design-document skills, hidden `.claude` skills and rules, a quickstart spawn example, and `cafleet/tests/docs_sync.rs`. That test suite reads the overlay file directly and checks backend sections and required-reading contracts.

---

## Specification

### Scope and ownership

This document plans a repository documentation migration. Creating and approving it changes only this design document; implementation is a subsequent workflow. Implementation updates Markdown and the existing documentation tests needed to validate it. CLI behavior, backend inference, spawn flags, model rankings, model prices, and runtime lifecycle behavior retain their current contracts.

The repository is the source of the migrated skill assets. Installed skill replicas receive the unified file through the existing release and `cafleet setup` process. This migration does not perform a release, deployment, provider refresh, or edit installed replicas. Keep design documents and research artifacts outside commits.

| Content | Authoritative home after implementation |
|---|---|
| Backend runtime bindings, notes, capture cues, and worked resolutions | The relevant backend section in `reference/coding-agents.md` |
| Backend model catalog and concrete monitor/reviewer defaults | The same backend section in `reference/coding-agents.md` |
| Model provenance, freshness, units, and maintenance attribution | Unified reference introduction and backend-local source links |
| Selection policy, overrides, staleness decisions, and cost efficiency mode | `roles/director.md` § Model selection |
| Placeholder resolution procedure and neutral documented defaults | `skills/cafleet/SKILL.md` § Resolve your overlay |
| Spawn mechanics and model-to-backend inference | Existing `reference/director.md` and runtime specifications |
| Model maintenance procedure | `.claude/skills/cafleet-model-list-refresh/SKILL.md` |
| Backend authoring convention | `.claude/rules/coding-agent-overlay.md` and `.claude/skills/skill-author/SKILL.md` § 8 |

The term “overlay” continues to describe runtime bindings within the unified reference. Keep the refresh skill's callable name and the authoring-rule filename. Their purposes remain useful after the two reference filenames disappear.

### Unified file structure

Use the title `# Coding Agents`. Introduce the three lookup purposes described below, then retain the catalog contract: maintainer-owned refresh, a 30-day cadence, last refreshed `2026-09-05`, standard provider USD per MTok as planning estimates, and capability ordering as reviewed judgment. The freshness date applies to model data; reorganizing prose does not advance it. Link selection decisions to `../roles/director.md#model-selection` and resolution mechanics to `../SKILL.md#resolve-your-overlay`.

Use these top-level headings in this order: `## claude`, `## codex`, `## opencode`, `## Template`. Preserve the backend anchors `#claude`, `#codex`, and `#opencode`. Each backend section contains these subsections in order:

| Subsection | Required contents |
|---|---|
| `### Runtime bindings` | The existing runtime placeholder rows: decision surface, permission flags, background run/stop, pane title, skill loader, and effort levels. Keep concrete values and constraints unchanged. |
| `### Role defaults` | A two-column `Placeholder` / `Value` table defining monitor and reviewer model placeholders exactly once for this backend. |
| `### Model catalog` | The complete existing backend table, with its existing column names and row order, backend-specific catalog prose, and relevant official source links. |
| `### Note → applies at` | All existing bound caveats, with updated references to their consuming instructions. |
| `### Pane-state capture cues` | Existing four states, their concrete cues, and the existing supervision tie-break pointer. |
| `### Worked resolution` | The existing fully resolved monitor-loop launch for this backend. |

Source links belong with the backend catalog: Anthropic pricing and Claude Code configuration for `claude`; OpenAI pricing and Codex availability for `codex`; OpenCode Zen for `opencode`. Retain all five original URLs. The shared introduction describes common units, freshness, and selection ownership; backend sections provide their concrete data without references to another backend's section.

Move the existing monitor/reviewer values into each backend's Role defaults table verbatim. Remove their rows from Runtime bindings and remove the separate cross-backend summary table. Consumers link to the backend section rather than maintaining a second set of literal defaults. Model names naturally remain present in catalog rows as well as their role assignments; this is catalog membership, not a second definition of the role default.

Expand `## Template` to the same six-subsection shape. Include placeholders for catalog columns appropriate to the backend, official provenance links, exact spawn tokens, reviewed capability order, role defaults drawn from that catalog, and every current runtime note/cue/worked-resolution requirement. A new backend author supplies its own complete section. Shared policy and supervision rules remain linked to their authoritative homes.

### Resolve by the subject of the action

Make the following dispatch explicit in the unified reference introduction, the core resolution procedure, and authoring guidance. Required-reading row one continues to gate runtime resolution for the executing agent; Directors read the selected backend again when a spawn decision requires it.

| Purpose | Backend selector | Values and behavior |
|---|---|---|
| Execute the agent's current instructions | Its `CODING AGENT:` identity, or its own identity for a standalone agent | Resolve runtime bindings such as decision surface, skill loader, and managed execution from its own backend and apply the corresponding notes. |
| Select/configure a spawned member | Backend selected under `roles/director.md` policy | Read that backend's catalog and Role defaults; validate any target-specific effort or launch capability against that backend. Model placeholders resolve here, separately from the executing agent's runtime bindings. |
| Interpret a captured member pane | The observed member's recorded backend | Apply that backend's capture cues. Keep the observer's execution tools and decision surface unchanged. |

Keep the existing nine placeholder names. Resolve the seven runtime bindings from their Runtime bindings table for the relevant operation and the two model placeholders from the selected spawn backend's Role defaults table. Effort describes backend capability: when validating a member spawn, consult the target backend's effort row. A Director's local long-lived work still uses the Director's own execution primitive.

Update absolute statements that all other backend values are inapplicable into this positive, purpose-specific contract. Apply bound notes at their named instructions and resolve concrete command values before emission. Preserve the core skill's documented neutral defaults for their existing, explicitly allowed missing/unknown-backend cases; they remain generic resolution policy rather than another concrete backend-default catalog. A missing required backend section, malformed table, or broken reference in an installed supported-backend reference is a documentation defect to report, not a reason to borrow another backend's data. Preserve existing spawn validation and user-relay behavior.

For monitor bootstrap, the target backend equals the Director's backend by construction. Resolve its monitor default for `--monitor-model`; monitor recovery uses the same backend default as `--model` and keeps backend inheritance. For a reviewer, resolve the reviewer default from the selected member backend, including when it differs from the Director's. Explicit user choices continue to win under the existing policy.

### Selection and maintenance

Rewrite `roles/director.md` links and “your overlay mirroring the model list” wording to refer to the selected backend's canonical Role defaults. Preserve the existing rules: monitor selection applies to every fleet, reviewer selection uses the chosen backend's most capable listed model, ordinary members use cost-based selection only for the exact user trigger, user overrides prevail, and staleness restricts cost efficiency mode while other spawns proceed normally. Preserve plan confirmation for the Claude context-window opt-in and mismatch/no-fit relay behavior. `reference/director.md` continues to point at this policy and reads the unified reference from the exact loaded skill root.

Revise `cafleet-model-list-refresh` to maintain only model catalogs, their provenance/context notes, their canonical Role defaults tables, and the shared freshness metadata in the unified file. Its existing source allowlist, availability filtering, Zen curation, reviewed capability ordering, approval-before-application, atomic successful refresh, and failure-without-edit behavior continue to apply. Replace the mirror/synchronization step with updating each backend's Role defaults directly. Define the new common catalog preamble as the maintained contract in place of the instruction to preserve the old file's preamble verbatim.

Runtime bindings, bound runtime notes, pane cues, and worked resolutions belong to runtime documentation maintenance and remain outside a model refresh's edit scope. A future model refresh advances the catalog date only after its prescribed approval and successful application. The structural migration preserves the existing date and data without fetching providers.

Update release/deployment prose to describe the unified reference as part of the shipped `skills/` tree. Keep the existing release-coupled deployment process and its ownership boundaries; unrelated release-instruction modernization is outside this migration.

### Consumer migration and removal

Update relative Markdown links, prose paths, absolute-path spawn examples, required-reading labels, and same-file fragments together. Prefer stable backend anchors for cross-file links to avoid duplicate subsection heading fragments. Keep agent reference and human backend-spec documentation as independent homes under the existing authoring convention.

| Surface | Required change |
|---|---|
| `skills/cafleet/SKILL.md` and `roles/{director,member,monitor}.md` | Unified required-reading paths, action-subject resolution, and single-home default wording. |
| `skills/cafleet/reference/{director,supervision,cli}.md` | Unified links, target backend model/capture lookups, bootstrap and recovery default wording. |
| `skills/cafleet-design-doc/**` | All entry-point and role required-reading links; reviewer/monitor instructions that currently say “your overlay's value.” |
| `.claude/skills/clean-docs/**` | Equivalent workflow and role links plus selected-backend resolution for spawns. |
| `.claude/skills/cafleet-model-list-refresh/SKILL.md` | Unified ownership, refresh targets, freshness contract, and deployment path. |
| `.claude/rules/coding-agent-overlay.md` and `.claude/skills/skill-author/SKILL.md` | Six-subsection backend structure, three lookup purposes, and model-data ownership. |
| `docs/docs/quickstart.md` | Installed-reference path in the runnable spawn example. |
| `cafleet/tests/docs_sync.rs` | Unified file path and documentation-contract expectations, while preserving runtime-contract coverage. |

Treat this as a starting inventory and search all current repository surfaces, including hidden files, for both input filenames and duplicated-default wording. Delete `skills/cafleet/reference/model-list.md` and `skills/cafleet/reference/coding-agent-overlays.md` in the same implementation change. Current artifacts describe the unified reference directly; historical design documents, research artifacts, and git history remain unchanged.

### Verification contract

Use the existing `docs_sync` suite for the changed reference path, required-reading entry points, full placeholder vocabulary, per-backend capture cues, and monitor launch contracts. Adapt assertions that depend on table placement to the split Runtime bindings/Role defaults structure. Preserve meaningful runtime assertions rather than weakening them to make reorganized prose pass.

Compare each migrated catalog against its pre-change source for exact row cells and ordering, and compare runtime tables/notes/cues/worked resolutions for preserved operational meaning. Check each role default resolves to an entry or alias in its backend catalog and has one authoritative assignment. Inspect updated links and concrete anchors, including required-reading links in hidden skills; existing path tests alone do not prove anchor validity. Placeholder anchors such as `<name>` are examples that must resolve to the corresponding concrete backend anchor when used.

Run a repository stale-reference search including hidden files and excluding historical design/research artifacts and `.git`; expect zero current references to either removed filename. Search separately for wording about mirroring defaults and selecting all values exclusively from the executing backend. Run `cargo test --manifest-path cafleet/Cargo.toml --test docs_sync`, `cargo fmt --manifest-path cafleet/Cargo.toml --check` if test code changes, and `mise //docs:build` for the quickstart/site change. Use these existing checks; this migration requires no new test framework or live fleet/provider exercise.

Manually trace these scenarios against the resulting instructions:

| Scenario | Expected resolution |
|---|---|
| Codex Director selects an OpenCode reviewer | OpenCode reviewer default/model catalog and effort capability; Codex decision surface and local execution tools. |
| Claude monitor observes a Codex member | Codex pane cues for classification; Claude background execution for its own monitor loop. |
| Codex Director bootstraps or recovers its monitor | Codex monitor default, bootstrap/recovery flag shape unchanged. |
| Ordinary member loads its skill | Its own runtime section is sufficient, with no model-selection obligation added. |
| User pins a backend/model or catalog is stale | Existing Director override, validation, and staleness policy still determines the next action. |
| Maintainer proposes a model refresh | Model data/defaults/date update together after approval; runtime sections remain intact. |

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

### Step 1: Build the unified reference

- [x] Create `skills/cafleet/reference/coding-agents.md` with the specified introduction, three complete backend sections, and Template; migrate all catalog and runtime content while preserving data and freshness. <!-- completed: 2026-09-11T14:11 -->
- [x] Place concrete role defaults solely in each backend's Role defaults table and express the three lookup purposes, keeping policy links to the Director and core skill. <!-- completed: 2026-09-11T14:11 -->

### Step 2: Migrate readers and maintenance guidance

- [x] Update core CAFleet resolution, Director selection, supervision, monitor recovery, and reference links so runtime, spawn, and observed-member subjects resolve independently. <!-- completed: 2026-09-11T14:15 -->
- [x] Update all family workflow/role required-reading links, hidden clean-docs consumers, and the quickstart spawn path, including reviewer/monitor model wording. <!-- completed: 2026-09-11T14:15 -->
- [x] Revise the refresh skill and both authoring guidance surfaces to the unified structure and ownership, preserving callable names, refresh policy, and release process. <!-- completed: 2026-09-11T14:16 -->

### Step 3: Complete removal and validation

- [x] Update `cafleet/tests/docs_sync.rs` to read the unified reference and validate its current structure while retaining existing operational coverage. <!-- completed: 2026-09-11T14:22 -->
- [x] Delete both input reference files and sweep current repository surfaces, including hidden files, for stale filenames, broken links/anchors, and default-mirroring wording. <!-- completed: 2026-09-11T14:22 -->
- [x] Verify catalog/runtime preservation and walk the resolution/refresh scenarios against the final prose; resolve every discrepancy before completion. <!-- completed: 2026-09-11T14:22 -->
- [x] Run the specified existing documentation tests, formatting check where applicable, and docs build; review the final diff and keep design/research artifacts outside commits. <!-- completed: 2026-09-11T14:23 -->

## Changelog

- 2026-09-11: Completed all nine implementation tasks and six success criteria. Fresh Reviewer approved in one round; documentation tests, formatting, docs build, preservation comparison, and link audits passed. User approved publication; opened PR https://github.com/himkt/cafleet/pull/374. This design document remains outside version control.
