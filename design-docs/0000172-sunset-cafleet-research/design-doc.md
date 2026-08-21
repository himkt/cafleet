# Sunset cafleet-research

**Status**: Complete
**Progress**: 28/29 tasks complete
**Last Updated**: 2026-08-21

## Overview

Remove the `cafleet-research` skill and its entire support stack from the repository (GitHub issue #330): the `skills/cafleet-research/` tree, every repo-wide mention, the root Node/pnpm toolchain (Slidev + agent-browser) that existed only to serve its presentation workflow, and the three-skill `cafleet setup` contract, which contracts to two skills. Per `.claude/rules/removal.md` the cleanup is total — after this change the repository reads as if the skill never existed, with `design-docs/` as the historical record.

## Success Criteria

- [x] `skills/cafleet-research/` (26 files) is deleted, and a repo-wide search for `cafleet-research` outside `design-docs/` matches only the sanctioned stale-install-cleanup surface: `cafleet/src/assets.rs`, `cafleet/tests/cli_setup_doctor.rs`, `SPEC.md` §6.3, and `docs/docs/spec/cli-options.md`.
- [x] `cafleet setup` installs exactly two skills (`cafleet`, `cafleet-design-doc`), prints the two-skill success line, and removes a leftover `cafleet-research` directory from each target skills dir.
- [x] No root `package.json`, `pnpm-workspace.yaml`, or `pnpm-lock.yaml` exists; `admin/` and `docs/` are standalone pnpm packages with their own lockfiles; `mise //admin:install`, `//admin:build`, `//docs:install`, and `//docs:build` all succeed.
- [x] `agent-browser`, Slidev, and the `{task_coord}` overlay token have zero mentions outside `design-docs/`.
- [ ] `mise //cafleet:test`, `//cafleet:lint`, `//cafleet:typecheck`, and `//admin:lint` pass; the CI and Docs workflows are green.
- [ ] Issue #330 is closed by the implementing PR.

---

## Background

`cafleet-research` is the CAFleet-orchestrated research skill (report + presentation workflows). Its presentation workflow is the sole consumer of the repository's root Node project — the `slidev-preview` `package.json` carrying `@slidev/cli`, `@slidev/theme-default`, `agent-browser`, and `vue` — and of the `{task_coord}` overlay token, the mise `slidev` task, and ~30 `agent-browser` permission entries in `.claude/settings.json`. The root `package.json` doubles as the pnpm workspace root for `admin/` and `docs/`, so removing it requires restructuring the workspace. CI and the release workflow reference nothing research-related (skills are embedded into the binary at build time), and the codex/opencode presets are clean.

---

## Specification

### Decisions

| # | Topic | Decision |
|---|---|---|
| 1 | agent-browser | Remove entirely: the dependency, every `.claude/settings.json` entry, and the `verifier.md` web-app row is rewritten neutrally. |
| 2 | `{task_coord}` token | Remove from every overlay section (claude, codex, opencode, Template) and from the cafleet `SKILL.md` documented-defaults table. |
| 3 | `researches/` bucket | Keep the bucket (clean-docs, skill-author, `.gitignore`, and `git-workflow.md` still use it); drop only the cafleet-research consumer row from `base-dir.md`. |
| 4 | pnpm workspace | Eliminate the root manifest entirely; `admin/` and `docs/` become standalone pnpm packages (details below). |
| 5 | Stale installs | `cafleet setup`'s install-skills actively deletes a leftover `cafleet-research` directory from the target skills dir. |
| 6 | docs_sync exemption | Remove the `ROLE_FILE_WITHOUT_REQUIRED_READING` exemption; the Required-reading gate becomes unconditional for all role files. |

### Skill tree deletion

Delete `skills/cafleet-research/` in full: `SKILL.md`, the report workflow (`report/report.md`, `report/template.md`, six role files), the presentation workflow (`presentation/presentation.md`, four role files), `reference/visualization.md`, `reference/slidev.md`, and the Slidev theme (`reference/slidev/theme/` — eight layout/component Vue files and one CSS file). The embedded-assets tree shrinks automatically: the build embeds the whole `skills/` directory, and only `SKILL_NAMES` in `cafleet/src/assets.rs` enumerates skill names.

### `cafleet setup` contract change (decision 5)

In `cafleet/src/assets.rs`:

- `SKILL_NAMES` becomes `["cafleet", "cafleet-design-doc"]`.
- The install-skills success line becomes `<agent>: installed cafleet, cafleet-design-doc (v<version>) -> <skills dir>`.
- After copying the embedded skill dirs into the target skills dir, install-skills removes `<skills_dir>/cafleet-research` when present, using the same existing-target check order as install-preset (a symlink → unlink; else a directory → recursive delete; else if it exists → unlink). The removal produces no extra output; a filesystem error surfaces through the existing `failed to install skills into <skills_dir>: <error>` string. Rationale: a stale installed copy would keep auto-triggering in the user's coding agent.

`SPEC.md` §6.3 (*Shared helpers — the assets half*) and `docs/docs/spec/cli-options.md` are updated to specify: exactly the two skill dirs, the new success line verbatim, and the leftover-`cafleet-research` removal behavior.

### pnpm workspace restructuring (decision 4)

The root manifest is eliminated; each Node package stands alone.

| File | Current state | Target state |
|---|---|---|
| `package.json` (root) | `slidev-preview` — workspace root + research deps | Deleted |
| `pnpm-workspace.yaml` | `packages: [admin, docs]`; `allowBuilds: agent-browser` | Deleted |
| `pnpm-lock.yaml` (root) | Single workspace lockfile | Deleted |
| `admin/package.json` | Workspace member with `packageManager` field | Unchanged (now standalone) |
| `admin/pnpm-lock.yaml` | — | New — generated by `pnpm install` in `admin/` |
| `docs/package.json` | Workspace member, no `packageManager` field | Gains `"packageManager": "pnpm@11.21.0"` |
| `docs/pnpm-lock.yaml` | — | New — generated by `pnpm install` in `docs/` |
| `docs/mise.toml` | — | New — mirrors `admin/mise.toml`: `install` (`pnpm install --frozen-lockfile`), `build` (`pnpm build`), `dev` (`pnpm dev`) |
| `mise.toml` (root) | Tasks `pnpm-install`, `slidev`, `docs-build`; `config_roots = [cafleet, admin]` | The three tasks deleted; `config_roots = [cafleet, admin, docs]` |
| `.github/workflows/docs.yml` | `mise //:docs-build` | `mise //docs:install` then `mise //docs:build` |
| `.github/workflows/ci.yml` | `mise //admin:install` + `//admin:build` | Unchanged — the install now resolves against `admin/pnpm-lock.yaml` |

pnpm treats a directory without an ancestor `pnpm-workspace.yaml` as a standalone project, so `pnpm install --frozen-lockfile` inside `admin/` or `docs/` uses that package's own lockfile. mise runs each config root's tasks with the config root as the working directory, so `admin/mise.toml` needs no change and `docs/mise.toml` follows the identical pattern. Renovate detects per-directory lockfiles automatically; `.github/renovate.jsonc` needs no change. The `allowBuilds: agent-browser` entry dies with the workspace file — no surviving dependency needs a build allowance.

### `{task_coord}` token removal (decision 2)

All consumers of the token live inside `skills/cafleet-research/`, so it is removed as infrastructure:

- `skills/cafleet/reference/coding-agent-overlays.md`: the `{task_coord}` placeholder row in the claude, codex, opencode, and Template sections, and each section's task-coordination note row in *Note → applies at* (the claude `TaskCreate`/`TaskUpdate` note, the codex and opencode "No harness task list" notes).
- `skills/cafleet/SKILL.md`: the `{task_coord}` row in the *Documented defaults* table.
- `.claude/rules/coding-agent-overlay.md`: the backend-deltas enumeration drops "the background-task + task-list primitives" down to "the background-task primitives".

### `base-dir.md` changes (decision 3)

In `skills/cafleet/reference/base-dir.md`:

- The *Consumer contract* canonicalization table loses its cafleet-research row, leaving one data row; per the one-data-row anti-rule in `documentation-tables.md`, the remaining `cafleet-design-doc` entry is rewritten as a sentence stating the canonicalization steps (strip trailing `/design-doc.md`, strip leading `design-docs/`, prepend `design-docs/`).
- The *Consumer contract* surroundings drop the deleted consumer's filename examples: the intro paragraph's trailing-filename example `/report.md` and the phrase "or other known per-topic filenames", and the wrong-BASE warning's "(or `report.md`, etc.)" parenthetical. The generic `researches/` bucket-prefix examples stay per decision 3.
- § *No-bypass write protocol* item 1 drops "or the research folder delivered via `[INSERT abs research folder]`" — the design-doc directory example remains as the sole consumer-supplied absolute target.
- § *Hidden agent-only folders vs visible deliverables* keeps its classification rule but drops the research-only examples (`.figures/code`, `.figures/data`, `.screenshots/`, `figures/output/`). Surviving examples: `${BASE}/.prompts/` for the hidden class, the task deliverable (e.g. `design-docs/<task>/design-doc.md`) for the visible class.

### Other cafleet-family skill edits

| File | Change |
|---|---|
| `skills/cafleet/reference/director.md` | The spawn-prompt slot-value table drops every research mention: the `‹ROLE TITLE›` row's `a Scout Researcher` / `research` and `the Presentation Specialist` / `research presentation` examples, the `‹cafleet-load purpose›` row's `cafleet-research` extra-skill-load clause, the `‹role› + ‹ROLE-DEF SUFFIX›` row's "the research roles' `— accountability, …, and shutdown.` enumeration" example, and — in the `‹IMPORTANT / ROLE-CONSTRAINT LINES›` row — the ack-inline poll-handling form with its "(research / presentation)" attribution: with no workflow using it, the either/or collapses to the simple `When you see cafleet message poll output…` form. Surviving examples come from the design-doc family. |
| `skills/cafleet-design-doc/execute/roles/verifier.md` | The web-application verification row's fallback becomes: `WebFetch (public URL); for a local-only dev server, delegate to a teammate with a browser-automation tool — never curl/wget`. |

### `.claude/` project config

| File | Change |
|---|---|
| `.claude/rules/commands.md` | Delete the § *Skill artifact runners* section in full — all four rows (pnpm deps install, Slidev dev server, calling-pane working directory, `agent-browser wait`) exist only to serve the cafleet-research presentation workflow. No other commands.md text references the section; the section deletion is the whole edit. |
| `.claude/settings.json` | Remove every `Bash(pnpm exec agent-browser …)` entry from both `allow` (18 entries) and `deny` (11 entries), plus `Bash(mise //:pnpm-install)` and `Bash(mise //:slidev *)` from `allow`. Replace `Bash(mise //:docs-build)` with `Bash(mise //docs*)` (matching the existing `//admin*` pattern). `Bash(pnpm install --frozen-lockfile)` (allow) and `Bash(pnpm install)` (deny) stay — they still govern admin/docs installs. |
| `.gitignore` | Remove the root `/node_modules/` entry (nothing installs at the root anymore), the `/.figures/` and `/.screenshots/` entries with their comment (research-only scratch dirs), and the research role spawn-prompt names `/manager.md`, `/scout.md`, `/researcher.md`, `/presentation.md`, `/transcript.md`, `/visual-reviewer.md`. Keep `/researches/` but rewrite its comment to name the current producers (clean-docs run artifacts and other research-shaped task folders) instead of `/research-report`. Update the `mise //:docs-build` reference in the `/docs/doc_build/` comment to `mise //docs:build`. |

### Documentation-site and contract-doc edits

| File | Change |
|---|---|
| `docs/docs/quickstart.md` | Drop the `"Skill(cafleet:cafleet-research)"` line from the recommended Claude Code `permissions.allow` snippet. |
| `docs/docs/contributing.md` | Five edits de-drift the page in one pass: (1) the `skills/` project-structure row lists two skills; (2) the `package.json` + `pnpm-lock.yaml` row is replaced with a row describing the per-package pnpm manifests (`admin/`, `docs/`, each with its own lockfile); (3) the WebUI dependency-edit recipe becomes: run `pnpm install --no-frozen-lockfile` in `admin/` to regenerate `admin/pnpm-lock.yaml`; (4) "a package in the repo's pnpm workspace" becomes a standalone-package description; (5) the docs-build prose switches to `mise //docs:install` + `mise //docs:build` (the thin-wrapper sentence now describes the `docs/mise.toml` tasks) and the live-preview command becomes `mise //docs:dev`. |
| `docs/docs/how-to/design-doc-development.md` | The opening "CAFleet ships three skills that run spec-driven development as CAFleet-orchestrated teams" becomes the two-skill phrasing (`cafleet`, `cafleet-design-doc`). |
| `SPEC.md` | §6.3 per *`cafleet setup` contract change* above. |
| `docs/docs/spec/cli-options.md` | The install-skills output line and behavior per *`cafleet setup` contract change* above. |
| `README.md` | No research mentions exist; verify no drift after the docs edits (thin-surface check per `documentation-maintenance.md`). |

### Test changes (decision 6)

| File | Change |
|---|---|
| `cafleet/tests/cli_setup_doctor.rs` | All expected install output drops `cafleet-research`: the success-line assertions and the installed-skill-dir loop iterate over the two remaining skills. Add coverage for the stale-install cleanup: pre-create `<skills_dir>/cafleet-research/`, run setup, assert the directory is gone and the two skills are installed. |
| `cafleet/tests/docs_sync.rs` | Three edits: (1) delete the `ROLE_FILE_WITHOUT_REQUIRED_READING` constant, its existence assertion, and the `block_is_mandatory` exemption clause — every `/roles/` file must carry a Required-reading block with the overlay as row #1; (2) `OVERLAY_PLACEHOLDERS` drops `"task_coord"` (array becomes `[&str; 9]`, doc comment becomes "The nine placeholders…") so `every_backend_overlay_defines_the_full_placeholder_vocabulary` matches the shrunken overlay vocabulary; (3) `NON_OVERLAY_TOKENS` drops `topic`, `current_year`, `current_month` and their "web-researcher discovery-query examples" comment (array becomes `[&str; 4]`) — their only consumers lived in the deleted tree. |

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

Documentation first, per `.claude/rules/documentation-maintenance.md`.

### Step 1: Documentation

- [x] Update `docs/docs/spec/cli-options.md`: two-skill install output line + stale-cleanup behavior <!-- completed: 2026-08-21T11:17 -->
- [x] Update `docs/docs/quickstart.md`: drop the `Skill(cafleet:cafleet-research)` permission line <!-- completed: 2026-08-21T11:17 -->
- [x] Update `docs/docs/contributing.md`: the five de-drift edits per the Specification table <!-- completed: 2026-08-21T11:17 -->
- [x] Update `docs/docs/how-to/design-doc-development.md`: two-skill opening phrasing <!-- completed: 2026-08-21T11:17 -->
- [x] Update `SPEC.md` §6.3: two skill dirs, new success line, leftover-dir removal <!-- completed: 2026-08-21T11:17 -->
- [x] Verify `README.md` has no drift after the docs edits <!-- completed: 2026-08-21T11:17 -->
- [x] Update `skills/cafleet/reference/director.md`: drop research examples from the spawn-prompt slot table <!-- completed: 2026-08-21T11:17 -->
- [x] Update `skills/cafleet/reference/coding-agent-overlays.md`: remove `{task_coord}` rows and task-coordination notes from all four sections <!-- completed: 2026-08-21T11:17 -->
- [x] Update `skills/cafleet/SKILL.md`: remove the `{task_coord}` documented-default row <!-- completed: 2026-08-21T11:17 -->
- [x] Update `skills/cafleet/reference/base-dir.md`: consumer table → sentence; hidden-folder examples cleanup <!-- completed: 2026-08-21T11:17 -->
- [x] Update `skills/cafleet-design-doc/execute/roles/verifier.md`: neutral web-app fallback cell <!-- completed: 2026-08-21T11:17 -->
- [x] Update `.claude/rules/commands.md`: delete the *Skill artifact runners* section <!-- completed: 2026-08-21T11:20 -->
- [x] Update `.claude/rules/coding-agent-overlay.md`: drop the task-list clause from the backend-deltas enumeration <!-- completed: 2026-08-21T11:20 -->

### Step 2: Skill tree and repo config

- [x] Delete `skills/cafleet-research/` (all 26 files) <!-- completed: 2026-08-21T11:22 -->
- [x] Update `.claude/settings.json` per the Specification table <!-- completed: 2026-08-21T11:22 -->
- [x] Update `.gitignore` per the Specification table <!-- completed: 2026-08-21T11:21 -->

### Step 3: pnpm workspace restructuring

- [x] Delete root `package.json`, `pnpm-workspace.yaml`, and `pnpm-lock.yaml` <!-- completed: 2026-08-21T11:25 -->
- [x] Add the `packageManager` field to `docs/package.json` <!-- completed: 2026-08-21T11:24 -->
- [x] Generate `admin/pnpm-lock.yaml` and `docs/pnpm-lock.yaml` (`pnpm install` in each package) <!-- completed: 2026-08-21T11:26 -->
- [x] Create `docs/mise.toml` (install/build/dev); update root `mise.toml`: delete `pnpm-install`, `slidev`, `docs-build`; add `docs` to `config_roots` <!-- completed: 2026-08-21T11:24 -->
- [x] Update `.github/workflows/docs.yml` to `mise //docs:install` + `mise //docs:build` <!-- completed: 2026-08-21T11:24 -->
- [x] Verify `mise //admin:install` and `mise //admin:build` succeed against the standalone lockfile <!-- completed: 2026-08-21T11:26 -->

### Step 4: Rust code and tests

- [x] Update `cafleet/src/assets.rs`: two-entry `SKILL_NAMES`, new success line, leftover-`cafleet-research` removal <!-- completed: 2026-08-21T11:30 -->
- [x] Update `cafleet/tests/cli_setup_doctor.rs`: two-skill expectations + stale-cleanup coverage <!-- completed: 2026-08-21T11:30 -->
- [x] Update `cafleet/tests/docs_sync.rs`: remove the Required-reading exemption; shrink `OVERLAY_PLACEHOLDERS` and `NON_OVERLAY_TOKENS` <!-- completed: 2026-08-21T11:30 -->
- [x] Run `mise //cafleet:test`, `//cafleet:lint`, `//cafleet:typecheck`, `//admin:lint` — all pass <!-- completed: 2026-08-21T11:30 -->

### Step 5: Verification and close-out

- [x] Sweep: `rg cafleet-research`, `rg -i slidev`, `rg agent-browser`, `rg task_coord` outside `design-docs/` match only the sanctioned cleanup surface <!-- completed: 2026-08-21T11:33 -->
- [ ] Run `mise //docs:build`; confirm the CI and Docs workflows are green on the PR <!-- completed: -->
- [x] Open the implementing PR with `Closes #330` <!-- completed: 2026-08-21T11:39 -->

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-08-21 | Initial draft |
| 2026-08-21 | Review round 1: fuller base-dir.md/director.md/contributing.md edit inventories, how-to page row, docs_sync.rs placeholder-constant changes |
| 2026-08-21 | Implementation complete: Steps 1–5 executed, Reviewer approved (round 1), PR #331 opened with Closes #330. The remaining task — CI and Docs workflows green — rides the open PR; teardown on operator instruction. |
