# 0000058 — Stale Config Audit

**Status**: Approved
**Progress**: 32/32 tasks complete
**Last Updated**: 2026-05-14T11:41

## Overview

Second-pass configuration cleanup picking up where design 0000057 left off. Removes the `[tasks.sync-skills]` mise task and the five home-dir skill mirrors it maintained (the cafleet plugin v0.8.0 now ships all twelve skills, so the divergence-guard is obsolete), rewrites `skills/create-figure/SKILL.md` to use the pyproject.toml-managed `[dependency-groups].research` invocation instead of `--with matplotlib`, drops a stale `!.claude/agents` override from `.gitignore`, and records audit findings for every other configuration surface enumerated in the brief so future contributors do not re-litigate them.

## Success Criteria

- [x] `mise.toml` no longer contains a `[tasks.sync-skills]` block.
- [x] The five home-dir mirrors at `~/.claude/skills/{create-figure,my-slidev,research-presentation,research-report,base-dir}/` no longer exist on the maintainer machine (operator-side action).
- [x] Every unprefixed `Skill(<name>)` reference in `skills/` body text — where `<name>` is one of `create-figure`, `my-slidev`, `research-presentation`, `research-report`, `base-dir` — has been rewritten to `Skill(cafleet:<name>)`.
- [x] `skills/create-figure/SKILL.md` no longer contains the string `--with matplotlib`; every execution example uses `uv run --frozen --group research python <script>`.
- [x] `skills/my-slidev/SKILL.md` headmatter template (lines ~14-17) and theme-path notes (lines ~207, ~225) reference the plugin install path (`~/.claude/plugins/cache/cafleet/cafleet/<version>/skills/my-slidev/theme`) as the single authoritative location; the `~/.claude/skills/my-slidev/theme` (home-dir working-copy) branch is removed. Verified by `git grep -n "~/.claude/skills/my-slidev/theme" -- skills/my-slidev/SKILL.md` returning zero matches (Step 4 verification task).
- [x] `.gitignore` no longer contains the `!.claude/agents` override line.
- [x] `git grep` for `uv run --with matplotlib` across tracked files outside `design-docs/` returns zero matches.
- [x] `git grep` for `sync-skills` across tracked files outside `design-docs/` returns zero matches.
- [x] Zero deprecation notices, removal markers, or "see design 0000058" pointers appear in any tracked file outside `design-docs/` (per `~/.claude/rules/removal.md`).
- [x] **Skill self-containment.** Every skill body under `skills/**` is environment-agnostic — no cafleet-specific run commands (`uv run --frozen --group research`, `mise //:bun-install`, `mise //:slidev`), no "cafleet repo root" / "cafleet repo's …" working-directory hardcoding, and no cross-references to cafleet's `permissions.deny` configuration. The cafleet-specific runner glue lives in `.claude/rules/commands.md`.


---

## Background

Three independent forces converge to motivate this cleanup batch.

| Force | Source |
|---|---|
| The cafleet plugin manifest at `.claude-plugin/plugin.json` v0.8.0 (and `.codex-plugin/plugin.json` via the `./skills/` auto-discovery glob) ships **all twelve** skills, including the five (`create-figure`, `my-slidev`, `research-presentation`, `research-report`, `base-dir`) that the `[tasks.sync-skills]` mise task mirrors to `~/.claude/skills/<name>/`. With the plugin path now canonical, the home-dir mirrors only enable unprefixed `Skill(<name>)` invocation — a behavior the maintainer has explicitly chosen to give up in exchange for a single source of truth. |
| Design 0000054 D2 chose `uv run --with matplotlib python <script>` as the `create-figure` execution recipe to make the skill self-bootstrap in any project. The user has now reversed that decision: the skill is repo-bound and should use the pyproject.toml-managed `[dependency-groups].research` instead, so dependencies are fixed via `uv.lock` rather than ephemerally resolved. The `--frozen --group research` permission entry in `.claude/settings.json` (line 31), the `[dependency-groups].research` block in `pyproject.toml` (lines 13-15, restored by commit `1925cb8` after 0000054 had deleted it), and the new SKILL.md text all line up under this single pivot. |
| Design 0000054 deleted the entire `.claude/agents/` directory (both agent specs embedded inline into host SKILL.md files per D4). The `!.claude/agents` override in `.gitignore` line 7 no longer un-ignores any tracked path; it is purely vestigial. |

Design 0000057 audited `.claude/settings.json` and declared all entries LIVE. That audit was incorrect on one entry: `Bash(uv run --frozen --group research *)` was flagged by the 0000054 Programmer as stale-but-harmless (the entry gated `mise //:figure` which 0000054 deleted), then re-classified as LIVE by 0000057. This design re-uses that entry — but only because the create-figure rewrite intentionally re-introduces the `--frozen --group research` invocation. The 0000054 Programmer's classification and 0000057's classification are both retired by this design's A2 pivot.

This design also expands the audit scope (per user instruction B1) to include `admin/mise.toml`, `cafleet/mise.toml`, `cafleet/pyproject.toml`, and `uv.lock`, and uses the document-everything pattern (B2) so every surface — including the LIVE ones — gets a recorded finding row.

---

## Specification

### Decisions (user-supplied, clarification round, 2026-05-14)

| # | Question | Answer |
|---|---|---|
| A1 | sync-skills task removal scope. | Option (b): REMOVE the mise task, DELETE the five home-dir mirrors at `~/.claude/skills/{create-figure,my-slidev,research-presentation,research-report,base-dir}/`, AND REWRITE every unprefixed `Skill(<one of the five>)` cross-ref inside plugin-source skill bodies to `Skill(cafleet:<name>)`. |
| A2 | `.claude/settings.json:31 Bash(uv run --frozen --group research *)`. | Pivot — keep the permission AND rewrite `skills/create-figure/SKILL.md` to stop using the `--with matplotlib` self-bootstrap pattern. Switch back to `uv run --frozen --group research python <script>` using the pyproject.toml-managed dependency group. User's exact words: *"Use pyproject.toml. Remove --with matplotlib related instructions. I want to fix the dependencies."* |
| A3 | `pyproject.toml` `[dependency-groups].research`. | Stay (consistent with A2). |
| A4 | `.gitignore` `!.claude/agents`. | Remove. The `.claude/agents/` directory was deleted by 0000054. |
| B1 | Audit scope. | Expanded — include `admin/mise.toml`, `cafleet/mise.toml`, `cafleet/pyproject.toml`, `uv.lock` in the audit section even when clean. |
| B2 | Document style. | Document-everything (like 0000057). One section per audited surface, with findings recorded for LIVE / no-edit surfaces too. |
| B3 | Removal-rule strictness. | Strict per `~/.claude/rules/removal.md`. No deprecation notices, no historical-record pointers outside this design doc. |

### 1. `mise.toml` — `[tasks.sync-skills]` removal

**Action.** Delete `mise.toml` lines 27-37 inclusive (the blank separator on line 27, the `[tasks.sync-skills]` block on lines 28-36, and the trailing blank on line 37). After this edit, `mise.toml` ends at the `[tasks.slidev]` `description = "..."` line (currently line 26) followed by a single trailing newline; lines 27-37 are all removed.

**Current state (verbatim — `mise.toml` lines 28-36):**

```toml
[tasks.sync-skills]
run = [
  "rm -rf ~/.claude/skills/create-figure && cp -r skills/create-figure ~/.claude/skills/create-figure",
  "rm -rf ~/.claude/skills/my-slidev && cp -r skills/my-slidev ~/.claude/skills/my-slidev",
  "rm -rf ~/.claude/skills/research-presentation && cp -r skills/research-presentation ~/.claude/skills/research-presentation",
  "rm -rf ~/.claude/skills/research-report && cp -r skills/research-report ~/.claude/skills/research-report",
  "rm -rf ~/.claude/skills/base-dir && cp -r skills/base-dir ~/.claude/skills/base-dir",
]
description = "Mirror the five home-dir-mirrored skills … (see file for the full description, deleted by this step)"
```

**Rationale.** The `cafleet` plugin (`.claude-plugin/plugin.json` v0.8.0) ships all twelve skills directly under the `cafleet:` namespace, including the five skills this task mirrors. The home-dir copies served only to support unprefixed `Skill(<name>)` invocation. Per A1, callers will use `Skill(cafleet:<name>)` (prefixed) going forward, eliminating the need for the home-dir branch. Removing the task is one of three coordinated edits — the others are §3 (rewrite unprefixed cross-refs to prefixed form) and §6 (operator-side mirror deletion).

**Reference removal (downstream).** No `README.md`, `ARCHITECTURE.md`, or `docs/` references to the `sync-skills` task exist (`git grep -n "sync-skills\|sync_skills" -- README.md ARCHITECTURE.md docs/ CLAUDE.md` returns zero hits). The task name does appear inside earlier design docs (`design-docs/0000022`, `design-docs/0000053`, `design-docs/0000054`); per `~/.claude/rules/removal.md`, design-doc historical record is preserved.

### 2. Self-containment principle (post-PR-71-review revision)

**Principle.** Skills must be **self-contained and environment-agnostic**. A skill describes its own domain (the matplotlib chart-generation logic, the Slidev authoring layout, the agent-browser commands). It does NOT hardcode the host project's run-glue — the choice of `uv run --frozen --group research`, `mise //:slidev`, `mise //:bun-install`, the working-directory invariant, or the host's `permissions.deny` configuration is project-specific glue that belongs in `.claude/rules/`, not in the skill body.

**Source of the principle.** The A2 pivot in this design's clarification round originally read *"Use pyproject.toml. Remove `--with matplotlib` related instructions. I want to fix the dependencies."* The first implementation pass interpreted this as "switch the skill from `--with matplotlib` to `--frozen --group research`," which made `skills/create-figure/SKILL.md` repo-bound to the cafleet checkout. The user's intent — surfaced on PR #71 — was the opposite: **the skill should stop documenting any specific invocation**; the cafleet-specific runner (`uv run --frozen --group research python <script>`) belongs in `.claude/rules/commands.md` where every other host-glue command already lives. This revision applies that principle uniformly to every skill site that violates it.

**Scope.** `git grep` audit (executed during PR #71 review) surfaced 12 sites across 5 files that hardcode cafleet-specific run-glue inside skill bodies. The revision rewrites all 12 to be invocation-agnostic and moves the runner detail to `.claude/rules/commands.md`.

| # | File:Line | Current binding | Severity |
|---|---|---|---|
| 1 | `skills/create-figure/SKILL.md:12` | "cafleet repo's pyproject.toml-managed `[dependency-groups].research` invocation" | Hard binding |
| 2 | `skills/create-figure/SKILL.md:87` | `Run via uv run --frozen --group research python <script>` | Hard binding |
| 3 | `skills/create-figure/SKILL.md:90` | `uv run --frozen --group research python ${SRC_DIR}/script_name.py` | Hard binding |
| 4 | `skills/create-figure/SKILL.md:93` | "`--frozen` pins to `uv.lock`; `--group research` pulls matplotlib via the cafleet `pyproject.toml`" | Hard binding |
| 5 | `skills/research-presentation/SKILL.md:16` | "`bun run agent-browser ...` from the cafleet repo root" | Hard binding |
| 6 | `skills/research-presentation/SKILL.md:228` | "Calling-pane working directory: cafleet repo root" | Hard binding |
| 7 | `skills/research-presentation/SKILL.md:230` | `mise //:bun-install` (cafleet-specific task) | Hard binding |
| 8 | `skills/research-presentation/SKILL.md:231` | `mise //:slidev <folder>/slide.md` (cafleet-specific task) | Hard binding |
| 9 | `skills/research-presentation/roles/director.md:123` | `mise //:slidev <folder>/slide.md` (cafleet-specific task) | Hard binding |
| 10 | `skills/research-presentation/roles/director.md:126` | "`agent-browser wait --load networkidle` is denied by repo permissions" | Cafleet `permissions.deny` cross-ref |
| 11 | `skills/research-presentation/roles/visual-reviewer.md:106` | "the project's `settings.json` `permissions.deny` blocks `Bash(bun run agent-browser ... wait ...)`" | Cafleet `permissions.deny` cross-ref |
| 12 | `skills/base-dir/SKILL.md:84` | "The cafleet repo's own `.gitignore` already excludes…" | Cafleet-installation informational cross-ref |

**Required edits — skills (invocation-agnostic rewrites):**

| Site | Replacement strategy |
|---|---|
| 1-4 (`create-figure`) | Strip every cafleet-specific runner detail. The skill describes generating a self-contained matplotlib script; the run command is delegated to the host project's rules (e.g., `.claude/rules/commands.md`). Example invocation block becomes pseudo-form `<project-python-runner> ${SRC_DIR}/script_name.py`. |
| 5-6 (`research-presentation` SKILL.md L16, L228) | Replace "cafleet repo root" / "cafleet repo's …" language with a generic invariant — the calling pane needs a directory that contains the Slidev `package.json` (typically the host project root). |
| 7-9 (`research-presentation` SKILL.md L230-L231, `roles/director.md` L123) | Replace `mise //:bun-install` and `mise //:slidev <folder>/slide.md` with a reference to the host project's `.claude/rules/` for the canonical command. The skill states the underlying invariant (`bun install --frozen-lockfile`, `bun run slidev <folder>/slide.md` PTY-wrapped). |
| 10-11 (`director.md` L126, `visual-reviewer.md` L106) | State `agent-browser wait` as **discouraged** (unreliable across renderers + slow CI) instead of as "denied by repo permissions". Note that host projects (the canonical cafleet setup included) typically block it via `permissions.deny`, but the skill recommendation does not depend on a specific project's configuration. |
| 12 (`base-dir/SKILL.md` L84) | Rewrite the cafleet-gitignore reference to a generic instruction: host projects that install cafleet should add `/.cafleet-base-dir.json` to their root `.gitignore` so the anchor file does not surface as untracked. The cafleet repo's own `.gitignore` is one example of such a project. |

**Required edits — `.claude/rules/commands.md` (project-specific glue, receiving site):**

Add the following entries so the cafleet-specific invocations the skills used to embed continue to be discoverable inside this repo. Each entry maps a skill-emitted artifact to the canonical cafleet runner:

| Skill artifact | Cafleet runner |
|---|---|
| `/create-figure` matplotlib scripts | `uv run --frozen --group research python <script>` (pyproject.toml `[dependency-groups].research = ["matplotlib"]` provides the dependency; `uv.lock` pins the version) |
| `/research-presentation` bun deps | `mise //:bun-install` (= `bun install --frozen-lockfile`) |
| `/research-presentation` Slidev dev server | `mise //:slidev <folder>/slide.md` (= PTY-wrapped `bun run slidev --open false <folder>/slide.md`; default URL `http://localhost:3030`) |
| Calling-pane working directory for bun / agent-browser / Slidev | The cafleet repo root (contains `package.json` and `node_modules/`) |
| `agent-browser wait` family | Denied by `.claude/settings.json` `permissions.deny` (blocks `Bash(bun run agent-browser ... wait ...)`). Use `sleep N` + open-retry loops instead. |

### 3. Unprefixed `Skill(<one of the five>)` cross-references in plugin-source skills

**Action.** Rewrite every unprefixed `Skill(create-figure)`, `Skill(my-slidev)`, `Skill(research-presentation)`, `Skill(research-report)`, `Skill(base-dir)` invocation inside `skills/` body text to the prefixed `Skill(cafleet:<name>)` form. Per A1, the home-dir mirrors that previously made the bare-name form resolvable are being deleted; the prefixed form is the only invocation that will resolve post-cleanup.

**Verified affected sites (from `git grep -nE "Skill\((my-slidev|create-figure|research-report|research-presentation|base-dir)\)" -- skills/`):**

| File | Line | Current reference | Rewrite to |
|---|---|---|---|
| `skills/my-slidev/SKILL.md` | 167 | `Skill(my-slidev)` (in narrative describing how the skill is reachable from Claude Code) | `Skill(cafleet:my-slidev)` |
| `skills/my-slidev/SKILL.md` | 207 | `Skill(base-dir)` (in parenthetical clarifying what base-dir resolves vs the install location) | `Skill(cafleet:base-dir)` |
| `skills/my-slidev/SKILL.md` | 225 | `Skill(base-dir)` (duplicate of the 207 parenthetical inside the embedded `slide-creator` Output Constraints) | `Skill(cafleet:base-dir)` |
| `skills/research-presentation/SKILL.md` | 14 | `Skill(my-slidev) + Skill(create-figure)` (in the Presentation-role row of the topology table) | `Skill(cafleet:my-slidev) + Skill(cafleet:create-figure)` |
| `skills/research-presentation/SKILL.md` | 31 | `Skill(my-slidev), Skill(create-figure)` (in the topology ASCII diagram) | `Skill(cafleet:my-slidev), Skill(cafleet:create-figure)` |
| `skills/research-presentation/SKILL.md` | 123 | `- Skill(my-slidev) — for Slidev authoring layouts and rules` | `- Skill(cafleet:my-slidev) — for Slidev authoring layouts and rules` |
| `skills/research-presentation/SKILL.md` | 124 | `- Skill(create-figure) — if the report includes data that would render better as a chart` | `- Skill(cafleet:create-figure) — if the report includes data that would render better as a chart` |
| `skills/research-presentation/roles/presentation.md` | 10 | `- Skill(my-slidev) — for Slidev authoring layouts and rules` | `- Skill(cafleet:my-slidev) — for Slidev authoring layouts and rules` |
| `skills/research-presentation/roles/presentation.md` | 11 | `- Skill(create-figure) — if the report includes data that renders better as a chart` | `- Skill(cafleet:create-figure) — if the report includes data that renders better as a chart` |
| `skills/research-presentation/roles/presentation.md` | 67 | `Load \`Skill(create-figure)\` and follow its Chart Type Selection and Color Rules strictly.` | `Load \`Skill(cafleet:create-figure)\` and follow its Chart Type Selection and Color Rules strictly.` |
| `skills/research-report/SKILL.md` | 344 | `Skill(research-report)` (in narrative describing how the skill is reachable from Claude Code) | `Skill(cafleet:research-report)` |

**Pre-existing prefixed sites are correct.** `git grep -lE "Skill\(cafleet:base-dir\)" skills/` returns 20 files already using the prefixed form (every member-role file under `skills/design-doc-*`, `skills/research-*`, plus `skills/cafleet/reference/director.md` and `skills/base-dir/SKILL.md` itself). Those sites need no change — only the 11 unprefixed sites in the table above.

**Line-number stability caveat.** Line numbers reflect the pre-edit snapshot. Implementation should re-grep with the anchor pattern `Skill\((my-slidev|create-figure|research-report|research-presentation|base-dir)\)` before each edit to relocate the site, rather than trust the literal line after earlier edits in the same step have shifted the file.

### 4. `skills/my-slidev/SKILL.md` headmatter and theme-path notes — collapse to plugin path

**Action.** With the home-dir working-copy branch eliminated by A1, the dual-branch theme-path text in `skills/my-slidev/SKILL.md` becomes incoherent (it documents an install path that no longer exists). Collapse every dual-branch passage to use the plugin-installed path (`~/.claude/plugins/cache/cafleet/cafleet/<version>/skills/my-slidev/theme`) as the single authoritative location.

**Verified affected sites (`skills/my-slidev/SKILL.md`):**

| Site | Pre-edit content (verbatim) | Required edit |
|---|---|---|
| Lines 14-17 (Headmatter template) | `theme: ~/.claude/skills/my-slidev/theme  # working-copy install (skill present under ~/.claude/skills/)`<br>`# For plugin-only installs (no working copy in ~/.claude/skills/), use the plugin's installed path instead:`<br>`# theme: ~/.claude/plugins/cache/cafleet/cafleet/<version>/skills/my-slidev/theme`<br>`# Replace <version> with the installed cafleet plugin version, or run \`claude plugin list\` to find it.` | Replace with: `theme: ~/.claude/plugins/cache/cafleet/cafleet/<version>/skills/my-slidev/theme`<br>`# Replace <version> with the installed cafleet plugin version, or run \`claude plugin list\` to find it.` (drop the working-copy-install default line and its companion comment header; the plugin path becomes the unconditional default). |
| Line 207 (slide-creator Step 6 — long-form passage) | `Use the literal \`theme:\` path documented in the embedding skill's headmatter template — \`~/.claude/skills/my-slidev/theme\` when this skill is installed under \`~/.claude/skills/\`, or \`~/.claude/plugins/cache/cafleet/cafleet/<version>/skills/my-slidev/theme\` when it is installed only via the cafleet plugin. The path is a fixed, documented location; do NOT try to derive it dynamically (Skill(base-dir) resolves a CWD-based working directory, not the install location of the calling skill).` | Rewrite to: `Use the literal \`theme:\` path documented in the embedding skill's headmatter template — \`~/.claude/plugins/cache/cafleet/cafleet/<version>/skills/my-slidev/theme\`. The path is a fixed, documented location; do NOT try to derive it dynamically (Skill(cafleet:base-dir) resolves a CWD-based working directory, not the install location of the calling skill).` (collapse the home-dir branch; also pick up the §3 prefix rewrite of `Skill(base-dir)` → `Skill(cafleet:base-dir)` in the parenthetical). |
| Line 225 (slide-creator Output Constraints — duplicate of 207) | Same content as line 207. | Apply the identical edit. (A single `replace_all` Edit covering both 207 and 225 is the lowest-diff implementation.) |

**Note.** Lines 8 (Theme location) and 14-17 (headmatter `theme:`) are independent passages — line 8 already reads "Theme location: \`theme/\` inside this skill's directory." which is install-location-agnostic and needs no edit. Only 14-17 (the headmatter template), 207, and 225 carry the dual-branch text.

### 5. `.gitignore` — remove `!.claude/agents` override

**Action.** Delete `.gitignore` line 7 (`!.claude/agents`). The override re-included a directory that design 0000054 deleted entirely; with no files inside `.claude/agents/` to track (and the directory itself absent from the working tree — verified `git ls-files .claude/agents/` returns empty), the override no longer protects any tracked path.

**Risk.** If `.claude/agents/` is ever re-created (e.g., a future contributor restoring a standalone agent file), the user's global `~/.config/git/ignore` excludes `.claude/` and the absence of the override would re-hide that new directory. This is the desired behavior — per design 0000054 D4, agent specs live inline inside host `SKILL.md` files, and the `.claude/agents/` path should never come back.

### 6. Operator-side: delete the five home-dir mirrors

**Action.** On the maintainer machine, delete the five home-dir skill mirrors:

```bash
rm -rf ~/.claude/skills/create-figure
rm -rf ~/.claude/skills/my-slidev
rm -rf ~/.claude/skills/research-presentation
rm -rf ~/.claude/skills/research-report
rm -rf ~/.claude/skills/base-dir
```

**Justification.** This is a one-time, operator-side action: it touches files outside the repo. Once these are gone, the only resolution for the five skills is the plugin path (`Skill(cafleet:<name>)`). The implementing harness (Programmer member spawned by `/design-doc-execute`) cannot perform these deletes itself because the home-dir paths fall outside the Programmer's Bash allow-list; expect them to route through the Director via `cafleet member exec` (per `.claude/rules/bash-tool.md`).

**Post-deletion sanity check.** After the five `rm -rf` complete, `ls ~/.claude/skills/` should list only `update-readme/` (the sole project-local skill that was never mirrored — see design 0000054). Any other entry is an artifact of a prior session that needs separate cleanup outside this design's scope.

### 7. Per-surface audit (document-everything)

Per B2, each configuration surface enumerated in the brief plus the four B1-expanded surfaces gets an explicit finding row. Surfaces with no required edit are marked **LIVE / no-edit**; surfaces with edits cross-reference the relevant §1-§6 above. This is the same pattern 0000057 used so the screening does not have to be re-run.

| Surface | Tracked path | Finding | Required edit |
|---|---|---|---|
| Root mise tasks | `mise.toml` | `[tasks.sync-skills]` is stale (per §1); `[tasks.uv-sync]`, `[tasks.bun-install]`, `[tasks.slidev]` are LIVE (matched by `.claude/settings.json` allow patterns and consumed by active workflows); `[tools]` (`uv`, `bun`) are LIVE; `experimental_monorepo_root = true` + `[monorepo].config_roots = ["cafleet", "admin"]` enable the `mise //cafleet:*` / `mise //admin:*` invocation syntax and are LIVE. | Delete `[tasks.sync-skills]` per §1. No other change. |
| Admin mise tasks | `admin/mise.toml` | `[tasks.lint]`, `[tasks.dev]`, `[tasks.install]`, `[tasks.build]` are all LIVE — each is invoked via `mise //admin:<task>` (matched by `.claude/settings.json` allow line 23 `Bash(mise //admin*)`). | None. |
| Cafleet mise tasks | `cafleet/mise.toml` | `[tasks.dev]`, `[tasks.test]`, `[tasks.lint]`, `[tasks.format]`, `[tasks.typecheck]`, `[tasks.install]`, `[tasks.build]`, `[tasks.publish]` are all LIVE — matched by `.claude/rules/commands.md` and invoked via `mise //cafleet:<task>` (allow pattern line 24 `Bash(mise //cafleet*)`). | None. |
| `.claude/settings.json` permissions.allow | `.claude/settings.json` lines 3-44 | 41 entries — 40 are LIVE; the one previously-flagged entry `Bash(uv run --frozen --group research *)` (line 31) is re-classified LIVE by this design because §2 reinstates the `--frozen --group research` invocation in the rewritten `create-figure` skill. The 13 `Skill(...)` entries match the 12 skills shipped by `.claude-plugin/plugin.json` v0.8.0 plus the project-local `Skill(update-readme)`. | None. |
| `.claude/settings.json` permissions.deny | `.claude/settings.json` lines 46-66 | 19 entries — all LIVE. They block direct underlying-tool invocation (`uv run pytest`, `uv run python -m`, `uv run --package *`, `mise run *`, `sqlite3 *`) per `.claude/rules/commands.md`, block dangerous `agent-browser` operations (`eval`, `wait --load networkidle`, generic `open *`, `set *`), and block `bun install` without `--frozen-lockfile`. | None. |
| `.claude/settings.json` permissions.ask | `.claude/settings.json` lines 67-69 | Single entry `Bash(cafleet * member exec *)` matches the bash-via-Director fallback protocol documented in `.claude/rules/bash-tool.md`. LIVE. | None. |
| `.claude/settings.json` hooks / env | `.claude/settings.json` | No `hooks` key, no `env` key. The four `CAFLEET_*` env vars (DATABASE_URL, BROKER_HOST, BROKER_PORT, MAX_TEXT_LEN) consumed by `cafleet.config.Settings` rely on safe defaults and are not declared here. | None. |
| `.claude/rules/*.md` | `bash-tool.md`, `code-quality.md`, `commands.md`, `design-doc-numbering.md`, `git-workflow.md`, `skill-discovery.md` | All six files were audited by design 0000057 §3 and confirmed correctly placed in `.claude/rules/` (project-specific) vs. `~/.claude/rules/` (general). No drift since. | None. |
| `CLAUDE.md` (root) | `CLAUDE.md` | Audited by design 0000057 §1; the duplicate `.claude/CLAUDE.md` was deleted. Root `CLAUDE.md` is unchanged in this design. Skill bullet list matches `.claude-plugin/plugin.json` user-invocable skills. | None. |
| `.claude-plugin/plugin.json` | `.claude-plugin/plugin.json` | v0.8.0 with 12 `./skills/<name>` entries matching the 12 skill directories under `skills/`. LIVE. | None. |
| `.claude-plugin/marketplace.json` | `.claude-plugin/marketplace.json` | v0.8.0, single plugin entry pointing at `./`. LIVE. | None. |
| `.codex-plugin/plugin.json` | `.codex-plugin/plugin.json` | v0.8.0 with `"skills": "./skills/"` auto-discovery glob. Picks up every directory under `skills/` automatically. LIVE. | None. |
| `.pre-commit-config.yaml` | (not present) | The brief enumerated this file but it does NOT exist in the repo. No pre-commit framework is configured; lint/format/typecheck run via `mise //cafleet:lint`, `mise //cafleet:format`, `mise //cafleet:typecheck`. | None. |
| `.gitignore` | `.gitignore` | Line 7 `!.claude/agents` is stale (per §5). Other override entries (`!design-docs`, `!CLAUDE.md`, `!.claude`, `!.claude/rules`, `!.claude/skills`, `!.claude/settings.json`) are LIVE. Per-session and audit-artifact entries (lines 13, 17, 19-25, 29, 33, 37-49, 52-55) are LIVE. | Delete line 7 per §5. |
| Root `pyproject.toml` | `pyproject.toml` | `[project]`, `[tool.uv.workspace]`, `[dependency-groups].dev` LIVE; `[dependency-groups].research = ["matplotlib"]` LIVE per A3 (kept to back §2's rewritten create-figure invocation). | None. |
| Cafleet package `pyproject.toml` | `cafleet/pyproject.toml` | All sections LIVE — `[project]` (deps: fastapi, uvicorn, sqlalchemy, alembic, click, pydantic, pydantic-settings), `[project.scripts] cafleet`, `[build-system]`, `[tool.hatch.build.targets.wheel]` (includes `alembic.ini`, `alembic/**`, `webui/**`), `[dependency-groups].dev` (pytest, ruff, ty), `[tool.ty.*]` config, `[tool.ruff.lint]` rule selection. | None. |
| `uv.lock` | `uv.lock` | Auto-generated by `uv lock` / `uv sync`. Reflects the current pyproject.toml manifests (root + cafleet package). Since this design does not alter any `pyproject.toml`, no `uv.lock` regeneration is required. | None. |

### 8. Constraints carried from the user's clarifying answers

| Constraint | Source | Enforcement |
|---|---|---|
| Audit prose lives only in this design doc. No "this was kept because…" comments in code/config/skill files. | B2 + `~/.claude/rules/removal.md`. | §1–§5 record decisions only here; the implementation steps in § Implementation do not write justification comments into any file. |
| Zero deprecation notices after the cleanup. | B3 + `~/.claude/rules/removal.md`. | §1–§5 delete-and-replace without leaving "moved to" / "deprecated" markers. The `sync-skills` task vanishes; the `!.claude/agents` override vanishes; the `--with matplotlib` text is replaced by the `--frozen --group research` text without any "previously …" callout. |
| Audit scope expanded to admin/cafleet mise + cafleet pyproject + uv.lock. | B1. | §7 includes rows for `admin/mise.toml`, `cafleet/mise.toml`, `cafleet/pyproject.toml`, `uv.lock`. |
| Document-everything pattern. | B2. | §7 records every audited surface, including LIVE ones. |
| Repo-bound `create-figure` is intentional. | A2. | §2 explicitly removes the "self-bootstraps in any project" prose and replaces it with a "repo-bound — assumes cafleet checkout" note. |
| Operator-side mirror deletion is part of "Success Criteria". | A1. | §6 documents the five `rm -rf` calls and notes the harness-routing expectation. The Success Criteria includes the home-dir-mirrors-no-longer-exist bullet so completion is verified. |

### 9. Adjacent finding — out of scope

`README.md` lines 16 and 24 say "**11** plugin-packaged skills" but `.claude-plugin/plugin.json` v0.8.0 ships **12** (the addition of `base-dir` in design 0000053 was not reflected). This is documentation drift, not a config-cleanup finding, and the user's brief did not enumerate `README.md` as an audit surface. Recorded here for visibility; a separate small design doc can correct it without coupling to this cleanup. No edit in this design.

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

Per `.claude/rules/design-doc-numbering.md`, documentation updates come first. The four `SKILL.md` files in §2-§4 are first-class documentation targets. The config edits in Step 2 follow. Operator-side mirror deletion (Step 3) and verification (Step 4) complete the work.

### Step 1: Documentation / SKILL.md updates

- [x] Edit `skills/create-figure/SKILL.md`: rewrite line 12, lines 87, 90, 93 per §2's table. After this edit, `git grep "uv run --with matplotlib" -- skills/create-figure/SKILL.md` MUST return zero matches. <!-- completed: 2026-05-14T11:27 -->
- [x] Edit `skills/my-slidev/SKILL.md`: rewrite line 167 (Skill prefix), lines 207 and 225 (Skill prefix + collapse dual-branch theme-path text), lines 14-17 (collapse headmatter template to plugin path only). Re-grep `Skill\((my-slidev|create-figure|research-report|research-presentation|base-dir)\)` against this file post-edit — MUST return zero matches. <!-- completed: 2026-05-14T11:28 -->
- [x] Edit `skills/research-presentation/SKILL.md`: rewrite lines 14, 31, 123, 124 (four Skill prefix sites). Re-grep `Skill\((my-slidev|create-figure)\)` against this file post-edit — MUST return zero matches. <!-- completed: 2026-05-14T11:28 -->
- [x] Edit `skills/research-presentation/roles/presentation.md`: rewrite lines 10, 11, 67 (three Skill prefix sites). Re-grep `Skill\((my-slidev|create-figure)\)` against this file post-edit — MUST return zero matches. <!-- completed: 2026-05-14T11:28 -->
- [x] Edit `skills/research-report/SKILL.md`: rewrite line 344 (Skill prefix). Re-grep `Skill\(research-report\)` (without the `cafleet:` prefix) against this file post-edit — MUST return zero matches. <!-- completed: 2026-05-14T11:28 -->

### Step 2: Config file edits

- [x] Edit `mise.toml`: delete lines 27-37 inclusive (blank separator on 27, `[tasks.sync-skills]` block on 28-36, trailing blank on 37) per §1. After this edit, `mise.toml` ends at the `[tasks.slidev]` `description = "..."` line followed by a single trailing newline; `git grep "sync-skills" -- mise.toml` MUST return zero matches. <!-- completed: 2026-05-14T11:32 -->
- [x] Edit `.gitignore`: delete line 7 (`!.claude/agents`). `git grep "\.claude/agents" -- .gitignore` post-edit MUST return zero matches. <!-- completed: 2026-05-14T11:32 -->

### Step 3: Operator-side — delete the five home-dir mirrors

Expect each `rm -rf` to be routed through the Director via `cafleet member exec` if the spawned Programmer's Bash allow-list does not cover `~/.claude/skills/` paths. The user's machine state, not the repo, is being mutated; the corresponding Success-Criteria bullet is verified by post-step `ls ~/.claude/skills/` listing only `update-readme/`.

- [x] Run `rm -rf ~/.claude/skills/create-figure`. Verify with `test ! -d ~/.claude/skills/create-figure`. <!-- completed: 2026-05-14T11:39 -->
- [x] Run `rm -rf ~/.claude/skills/my-slidev`. Verify with `test ! -d ~/.claude/skills/my-slidev`. <!-- completed: 2026-05-14T11:39 -->
- [x] Run `rm -rf ~/.claude/skills/research-presentation`. Verify with `test ! -d ~/.claude/skills/research-presentation`. <!-- completed: 2026-05-14T11:39 -->
- [x] Run `rm -rf ~/.claude/skills/research-report`. Verify with `test ! -d ~/.claude/skills/research-report`. <!-- completed: 2026-05-14T11:39 -->
- [x] Run `rm -rf ~/.claude/skills/base-dir`. Verify with `test ! -d ~/.claude/skills/base-dir`. <!-- completed: 2026-05-14T11:39 -->

### Step 4: Verification

All greps scope to **tracked files only** via `git ls-files` (or the equivalent `git grep` which respects tracked-file scope). Hits inside `design-docs/` are acceptable and expected — those are the historical record and are preserved per `~/.claude/rules/removal.md`.

- [x] Run `git grep -nE 'Skill\((my-slidev|create-figure|research-report|research-presentation|base-dir)\)' -- skills/`. Expected: zero matches. Any hit indicates an unprefixed `Skill(...)` cross-ref that Step 1 missed. <!-- completed: 2026-05-14T11:41 -->
- [x] Run `git grep -n "uv run --with matplotlib" -- skills/ .claude/ docs/ ARCHITECTURE.md README.md cafleet/ admin/`. Expected: zero matches outside `design-docs/`. <!-- completed: 2026-05-14T11:41 -->
- [x] Run `git grep -n "sync-skills" -- .` and verify every remaining match is inside `design-docs/*.md`. Zero matches outside `design-docs/`. (The sibling token `sync_skills` was considered and dropped — it does not appear in the codebase, so a single literal grep is enough and avoids the BRE-vs-ERE alternation question.) <!-- completed: 2026-05-14T11:41 -->
- [x] Run `git grep -n "~/.claude/skills/my-slidev/theme" -- skills/my-slidev/SKILL.md`. Expected: zero matches. Confirms the §4 theme-path collapse removed every reference to the home-dir working-copy install location. <!-- completed: 2026-05-14T11:41 -->
- [x] Run `git grep -n "!\.claude/agents" -- .gitignore`. Expected: zero matches. <!-- completed: 2026-05-14T11:41 -->
- [x] Confirm the five home-dir mirrors are gone via five `test ! -d ~/.claude/skills/<name>` checks (or a single `ls ~/.claude/skills/` showing only `update-readme/`). <!-- completed: 2026-05-14T11:41 -->

### Step 5: Self-containment revision (post-PR-71-review)

Per §2's revised principle, undo the cafleet-binding parts of Step 1 task 1 and apply the same fix to seven additional pre-existing violations. The runner-glue (mise tasks, `uv run --frozen --group research`, working-directory invariant, `permissions.deny` cross-references) moves to `.claude/rules/commands.md`; the skills become invocation-agnostic. See §2's tables for the full site list and replacement strategy.

**5a. Edit `.claude/rules/commands.md` — receive runner-specific glue:**

- [x] Append a new section to `.claude/rules/commands.md` documenting the cafleet-specific runner for each skill artifact per §2's last table (matplotlib scripts, bun deps, Slidev dev server, calling-pane working directory, `agent-browser wait` denial). The section header should mark these as the host-project glue that the skills themselves no longer embed. <!-- completed: 2026-05-14T11:54 -->

**5b. Strip cafleet bindings from `skills/create-figure/SKILL.md` (sites 1-4):**

- [x] Rewrite lines 12, 87, 90, 93 of `skills/create-figure/SKILL.md` to drop every `uv run --frozen --group research` / `cafleet repo` / `[dependency-groups].research` reference. The skill describes only the self-contained Python script and points at `.claude/rules/` for the host-project run command. After this edit, `git grep -nE "(uv run --(frozen|group|with)|cafleet repo|\[dependency-groups\])" -- skills/create-figure/SKILL.md` MUST return zero matches. <!-- completed: 2026-05-14T11:54 -->

**5c. Strip cafleet bindings from `skills/research-presentation/SKILL.md` (sites 5-8):**

- [x] Rewrite line 16 (Visual Reviewer cell): strip `from the cafleet repo root, equivalent to \`bun run agent-browser …\`` redundancy; keep the canonical agent-browser invocation only. <!-- completed: 2026-05-14T11:54 -->
- [x] Rewrite line 228 (working-directory paragraph): replace "Calling-pane working directory: cafleet repo root" with the generic invariant "the calling pane needs a directory that contains the Slidev `package.json`". <!-- completed: 2026-05-14T11:54 -->
- [x] Rewrite lines 230-231 (Server Startup steps 1-2): replace `mise //:bun-install` and `mise //:slidev <folder>/slide.md` with references to the host project's `.claude/rules/`. State the underlying invariant (`bun install --frozen-lockfile`, `bun run slidev <folder>/slide.md` PTY-wrapped) but not the cafleet-specific task names. After this edit, `git grep -n "mise //" -- skills/research-presentation/SKILL.md` MUST return zero matches. <!-- completed: 2026-05-14T11:54 -->

**5d. Strip cafleet bindings from `skills/research-presentation/roles/director.md` (sites 9-10):**

- [x] Rewrite line 123 (Start command table row): replace `mise //:slidev <folder>/slide.md` with a reference to the host project's `.claude/rules/` for the canonical Slidev launcher. State the underlying invariant (`bun run slidev --open false <slide>` PTY-wrapped via `script -qfc`) but not the mise task name. <!-- completed: 2026-05-14T11:54 -->
- [x] Rewrite line 126 (Readiness check table row): drop "`agent-browser wait --load networkidle` is denied by repo permissions" — state the recommendation (`agent-browser wait` is discouraged; use `sleep` + open-retry) without referencing cafleet's `permissions.deny`. <!-- completed: 2026-05-14T11:54 -->

**5e. Strip cafleet bindings from `skills/research-presentation/roles/visual-reviewer.md` (site 11):**

- [x] Rewrite line 106: drop "the project's `settings.json` `permissions.deny` blocks `Bash(bun run agent-browser ... wait ...)`" — state the recommendation (`agent-browser wait` is unreliable and host projects typically block it; use `sleep` + open-retry) without naming cafleet's specific configuration. <!-- completed: 2026-05-14T11:54 -->

**5f. Strip cafleet bindings from `skills/base-dir/SKILL.md` (site 12):**

- [x] Rewrite line 84 (Gitignore handling item 1): replace "The cafleet repo's own `.gitignore` already excludes …" with a generic instruction to host projects (add `/.cafleet-base-dir.json` to the project `.gitignore`). The cafleet repo's own configuration becomes one example, not the subject of the sentence. <!-- completed: 2026-05-14T11:54 -->

**5g. Verification:**

- [x] Run `git grep -nE "(uv run --frozen --group research|mise //:slidev|mise //:bun-install)" -- skills/`. Expected: zero matches. <!-- completed: 2026-05-14T11:54 -->
- [x] Run `git grep -n "cafleet repo root\|cafleet repo's\|cafleet checkout" -- skills/`. Expected: zero matches outside `design-docs/`. <!-- completed: 2026-05-14T11:54 -->
- [x] Run `git grep -nE "denied by repo permissions|project's \`settings.json\` \`permissions.deny\` blocks" -- skills/`. Expected: zero matches. <!-- completed: 2026-05-14T11:54 -->
- [x] Run `git grep -n "cafleet repo's own" -- skills/base-dir/SKILL.md`. Expected: zero matches. <!-- completed: 2026-05-14T11:54 -->
- [x] Run `git grep -n "uv run --frozen --group research" -- .claude/rules/commands.md`. Expected: exactly one match (the new commands.md entry). <!-- completed: 2026-05-14T11:54 -->

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-05-14 | Initial draft. |
| 2026-05-14 | Reviewer round 1 addressed. Fixed Progress count `0/19` → `0/18` (matched the 18 Implementation tasks after Step 4 expansion); dropped two non-edit Success Criteria bullets (`pyproject.toml [dependency-groups].research = ["matplotlib"]` is unchanged; `.claude/settings.json:31` is unchanged) — "X was not edited" is implicit in not having a §1-§6 entry; added a Step 4 verification grep for `~/.claude/skills/my-slidev/theme` against `skills/my-slidev/SKILL.md` and a companion SC qualifier so the §4 theme-path collapse is auditable; replaced §1 + Step 2 task 1's ambiguous "stranded blank line" prose with the concrete "delete lines 27-37 inclusive" instruction and an explicit post-edit shape; switched Step 4 task 3's `git grep` from BRE-`\|` alternation to a single literal `git grep -n "sync-skills" -- .` (the sibling token `sync_skills` does not appear in the codebase, so alternation is unnecessary). |
| 2026-05-14 | PR #71 user review surfaced the underlying principle — skills must be self-contained and environment-agnostic; project-specific run-glue belongs in `.claude/rules/`, not in skill bodies. The first-pass implementation of A2 had violated this principle by switching `skills/create-figure/SKILL.md` from `--with matplotlib` (self-bootstrapping) to `uv run --frozen --group research` (cafleet-bound). Broader `git grep` audit found 12 sites across 5 files with the same anti-pattern (4 newly-introduced + 8 pre-existing in `skills/research-presentation/**` and `skills/base-dir/SKILL.md`). Revised §2 to articulate the principle, added §2's site table covering all 12 sites, added Step 5 (Self-containment revision) with the 14 corrective tasks + verification greps, added a new Success Criterion enforcing the principle, bumped Progress from `18/18` to `18/32`. The cafleet-specific runner glue (`uv run --frozen --group research`, `mise //:bun-install`, `mise //:slidev`, working-directory invariant, `agent-browser wait` denial) moves to `.claude/rules/commands.md`. |
| 2026-05-14 | Status promoted from `Draft` to `Approved` upon user approval relayed by the Director. Implementation steps verified actionable (no `[TBD]`, no standing `COMMENT(role)` issue markers, every task carries the `<!-- completed: 2026-05-14T11:54 -->` timestamp slot — 18 tasks × 1 slot each = 18 slots, plus the 1 example slot in the §Implementation header). |
