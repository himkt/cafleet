# Port Research and Slidev Skills into cafleet, with Repo-Root Toolchain Consolidation

**Status**: Complete
**Progress**: 35/36 tasks complete (Step 5.1 skill-resolution check deferred to post-release plugin reinstall)
**Last Updated**: 2026-05-10

## Overview

Port the four global skills `research-report`, `research-presentation`, `my-slidev`, and `create-figure` from `~/.claude/skills/` into this repository under `skills/`, exposing them as `/research-report`, `/research-presentation`, `/my-slidev`, and `/create-figure`. Alongside the port, consolidate the Bun (Slidev / agent-browser) and uv (matplotlib) toolchains those skills depend on into the cafleet repo root: `package.json` + `bun.lock` move to the repo root, and matplotlib lands in the existing repo-root `pyproject.toml` as a `[dependency-groups.research]` group. This replaces the previous `CLAUDE_HOME = ~/.claude` assumption with a self-contained, repo-rooted toolchain that needs no `--cwd` plumbing.

## Success Criteria

- [ ] `.claude/skills/research-report/`, `.claude/skills/research-presentation/`, `.claude/skills/my-slidev/`, and `.claude/skills/create-figure/` exist and are loaded as `research-report`, `research-presentation`, `my-slidev`, and `create-figure` (visible in the system-reminder skill list when the cafleet plugin is active). <!-- deferred: working-tree files exist with correct front-matter; cafleet:-namespace exposure requires plugin reinstall (cache lives at ~/.claude/plugins/cache/cafleet/cafleet/0.6.1/) -->
- [x] The cafleet repo root holds `package.json` + `bun.lock` (migrated verbatim from `~/.claude/`) and the existing `pyproject.toml` exposes matplotlib via a `[dependency-groups.research]` group, with `uv.lock` re-resolved to include the new dep.
- [x] `/create-figure` renders a trivial chart end-to-end via the repo-root uv `research` dependency group (`mise //:figure <script>` / `uv run --frozen --group research <script>`).
- [x] The repo-root Bun environment (`mise //:bun-install` / `bun install --frozen-lockfile`) installs the Slidev toolchain at the repo root and the slidev binary is reachable via `bun run slidev` (verified `bun run slidev --version` → 52.14.1). End-to-end deck rendering uses `mise //:slidev <deck>` (long-running dev server).
- [x] `README.md`, `ARCHITECTURE.md`, `CLAUDE.md`, and `.claude/CLAUDE.md` list the four new skills and document the repo-root toolchain (Bun manifests at root, matplotlib in the `research` uv group).
- [x] Internal slash-command cross-references inside the four ported `SKILL.md` files (covering both body prose AND YAML front-matter `description:` strings) are rewritten to the cafleet namespace (`/research-presentation` → `/research-presentation`, `/research-report` → `/research-report`, `/my-slidev` → `/my-slidev`, `/create-figure` → `/create-figure`).
- [x] `research-presentation/SKILL.md` no longer hard-codes `~/.claude/skills/research-presentation/roles/...` paths; it uses repo-relative `.claude/skills/research-presentation/roles/...` instead.
- [x] `grep -E '~/\.claude|CLAUDE_HOME' .claude/skills/research-report .claude/skills/research-presentation .claude/skills/my-slidev .claude/skills/create-figure` returns no matches after Step 4 completes. Scope covers SKILL.md and the `roles/` subtree — every reference to `~/.claude/agents/web-researcher.md` is rewritten to `.claude/agents/web-researcher.md` (resolved from the cafleet repo root), and the misleading `~/.claude/tasks/...` task-store paths are replaced with the canonical `TaskCreate` / `TaskUpdate` / `TaskList` tool reference.

---

## Background

### Why this port

Two of the four skills already invoke cafleet primitives. `research-report/SKILL.md` and `research-presentation/SKILL.md` both load `Skill(cafleet)` and `Skill(cafleet:agent-team-monitoring)` and orchestrate Director / Manager / Researcher / Presentation / Transcript / Visual-Reviewer members via `cafleet member create`. Their natural home is inside the cafleet plugin, not the global `~/.claude/skills/` tree, because:

- The skills produce admin-WebUI message-timeline traffic that only makes sense alongside the cafleet broker.
- Internal references (role-file paths, slash-command cross-references) currently leak out of the skill directory into `~/.claude/skills/...`. Repo-relative paths inside the cafleet plugin keep the port self-contained.
- `my-slidev` and `create-figure` are already named in the spawn prompts of `research-presentation` (`Skill(my-slidev)`, `Skill(create-figure)`). Porting all four together preserves the dependency graph.

### Why repo-root toolchain

`create-figure/SKILL.md` runs matplotlib through `uv run --frozen --project <CLAUDE_HOME>` against `~/.claude/pyproject.toml` + `~/.claude/uv.lock`. `research-presentation/SKILL.md` runs Slidev through `bun install --frozen-lockfile` plus `mise run slidev` against `~/.claude/package.json` + `~/.claude/bun.lock`. Both toolchains live in the user's global Claude config directory, so the ported skills currently depend on artifacts outside the cafleet repository.

The cafleet repo already owns a `pyproject.toml` + `uv.lock` workspace (root member: `cafleet/`); adding matplotlib as a `[dependency-groups.research]` group avoids creating a parallel uv tree. Bun has no existing manifest at the repo root, so `package.json` + `bun.lock` migrate there directly. This flat layout has two ergonomic wins: (1) Bun and uv resolve their manifests with no `--cwd` / `--project` plumbing, and (2) the previous `bun --cwd <dir> install` form (which Bun parses as a script lookup) is avoided entirely. After the port, `~/.claude/{package.json,bun.lock,pyproject.toml,uv.lock}` are migrated into the repo root: `package.json` + `bun.lock` move verbatim, and matplotlib (the only meaningful runtime dep from `~/.claude/pyproject.toml`) lands in the new `[dependency-groups.research]` group of the existing `pyproject.toml`. The four `SKILL.md` files rebind their `CLAUDE_HOME`-style references to repo-root paths driven by `mise` task wrappers (`mise //:bun-install`, `mise //:slidev`, `mise //:figure`); `agent-browser` invocations stay verbatim as `bun run agent-browser <args>` (no mise wrapper).

### Relationship to design doc 0000045

`design-docs/0000045-cafleet-design-doc-interview/design-doc.md` includes a classification table that placed the four skills in scope here as "general-purpose utility skills with no cafleet coupling; staying global is correct." That classification has drifted from reality: `research-report` and `research-presentation` already invoke `Skill(cafleet)` and `Skill(cafleet:agent-team-monitoring)` in their spawn prompts, and the present design doc decides to port all four anyway.

This design doc supersedes that verdict, scoped strictly:

(a) **Superseded rows.** Four rows in the `0000045` "Other global-only skills considered" table — `create-figure`, `my-slidev`, `research-report`, and `research-presentation` — all originally classified as "general-purpose utility skills with no cafleet coupling; staying global is correct."

(b) **Replacement classification.** Ported into the cafleet plugin under the `cafleet:<name>` namespace; the canonical source is `cafleet/skills/<name>/` inside this repository.

(c) **Coexist semantics, restated.** The supersession is about the *classification verdict*, not about file location. The pre-existing `~/.claude/skills/<skill>/` directories remain in place per the user's coexist directive (Q1 in the clarification round) — the user removes them at their own pace.

`0000045` itself is left untouched. The historical classification stays in `0000045` as the record of what was true at the time it shipped; the current state is described in this document.

### Source-of-truth direction post-port

`.claude/skills/<name>/` inside this repo is the sole source of truth post-port. The pre-existing `~/.claude/skills/{research-report,research-presentation,my-slidev,create-figure}/` directories are deleted as part of this migration; only the project-local (unprefixed) versions exist after the design lands. The Slidev-creator subagent (`~/.claude/agents/slide-creator.md`) and the web-researcher subagent (`~/.claude/agents/web-researcher.md`) are also moved into `.claude/agents/` inside this repo, and slide-creator's body is rewritten to invoke `my-slidev` (no namespace).

### Post-migration skill resolution

After this design lands, the four skills resolve as project-local (unprefixed) Claude Code skills under the cafleet repo's `.claude/skills/` directory:

| User-typed slash command | Resolves to | Available |
|---|---|---|
| `/research-report` | `.claude/skills/research-report/SKILL.md` | Inside the cafleet repo only |
| `/research-presentation` | `.claude/skills/research-presentation/SKILL.md` | Inside the cafleet repo only |
| `/my-slidev` | `.claude/skills/my-slidev/SKILL.md` | Inside the cafleet repo only |
| `/create-figure` | `.claude/skills/create-figure/SKILL.md` | Inside the cafleet repo only |

The `~/.claude/skills/<name>/` global snapshots are removed, and the four skills are NOT part of the cafleet plugin's `./skills/` tree (which holds only the seven plugin-packaged `cafleet:*`-namespaced skills). This matches the user directive that these skills be project-local rather than globally namespaced.

---

## Specification

### Scope

Four skills are ported. For each, the port copies the entire skill directory tree (`SKILL.md` + any `roles/` / `theme/` / `template.md` subtree) verbatim from `~/.claude/skills/<skill>/` into `.claude/skills/<skill>/`, then applies the textual edits described under *Internal reference rewrites* below.

| Source | Target | Notes |
|---|---|---|
| `~/.claude/skills/research-report/` | `.claude/skills/research-report/` | Includes `SKILL.md`, `roles/` (director, manager, scout, researcher), `template.md` |
| `~/.claude/skills/research-presentation/` | `.claude/skills/research-presentation/` | Includes `SKILL.md`, `roles/` (director, presentation, transcript, visual-reviewer) |
| `~/.claude/skills/my-slidev/` | `.claude/skills/my-slidev/` | Includes `SKILL.md` and the entire `theme/` subtree (CSS, layouts, Vue components) — copied verbatim |
| `~/.claude/skills/create-figure/` | `.claude/skills/create-figure/` | Just `SKILL.md`; no subdirectories |

### Internal reference rewrites

The user's answers limit rewrites to slash-command cross-references plus the two hard-coded role paths. All other `Skill(...)` invocations stay as-is (they resolve via the global skill resolver regardless of namespace).

#### Slash-command rewrites

The cross-references between the four ported skills stay unprefixed because the skills are project-local (`.claude/skills/<name>/`) rather than plugin-packaged. The port preserves the original unprefixed slash commands; no namespacing rewrite is applied for cross-skill references:

| Original token | Final token | Affected files |
|---|---|---|
| `/research-report` | `/research-report` (unchanged) | `research-presentation/SKILL.md` (description, Step 0 error message) |
| `/research-presentation` | `/research-presentation` (unchanged) | `research-report/SKILL.md` (description, Step 7) |
| `/my-slidev` (in body prose) | `/my-slidev` (unchanged) | `research-presentation/SKILL.md` (Presentation role section) |
| `/create-figure` (in body prose) | `/create-figure` (unchanged) | `research-presentation/SKILL.md` (Presentation role section) |

`Skill(...)` calls (e.g., `Skill(cafleet)`, `Skill(cafleet:agent-team-monitoring)`, `Skill(base-dir)`, `Skill(my-slidev)`, `Skill(create-figure)`) likewise stay unprefixed for the four ported skills and namespaced for the cafleet plugin skills they call into. They resolve via the existing skill loader.

References to other plugins' slash commands — for example `/slidev` and `/slidev:slidev` in `my-slidev/SKILL.md` — are also left as-is.

#### Hard-coded role paths in `research-presentation/SKILL.md`

Two embedded spawn prompts hard-code absolute paths under `~/.claude`:

- `~/.claude/skills/research-presentation/roles/transcript.md` → `.claude/skills/research-presentation/roles/transcript.md` (resolved from project root)
- `~/.claude/skills/research-presentation/roles/visual-reviewer.md` → `.claude/skills/research-presentation/roles/visual-reviewer.md` (resolved from project root)

After the rewrite, the ported `SKILL.md` reads its own role files. No reference to `~/.claude` remains.

### Repo-root toolchain layout

After the port, the cafleet repo root holds the migrated Bun manifests directly, and matplotlib joins the existing repo-root uv workspace as a `[dependency-groups.research]` group:

```
<repo root>/
├── package.json          # Bun manifest (Slidev, agent-browser, vue) — migrated from ~/.claude/
├── bun.lock              # Bun lockfile — migrated from ~/.claude/
├── pyproject.toml        # existing uv workspace root, with a new [dependency-groups.research] group
└── uv.lock               # existing uv lockfile, re-resolved to include matplotlib in the research group
```

Migration table:

| Source | Target |
|---|---|
| `~/.claude/package.json` | `<repo root>/package.json` (verbatim move) |
| `~/.claude/bun.lock` | `<repo root>/bun.lock` (verbatim move) |
| `~/.claude/pyproject.toml` (matplotlib dep) | `<repo root>/pyproject.toml` `[dependency-groups.research]` (folded in) |
| `~/.claude/uv.lock` | `<repo root>/uv.lock` (re-resolved to include the research group) |

The migration is a move, not a copy (per the user's Q6 directive): after the design lands the four files are no longer at `~/.claude/`. The ported skills under `skills/<name>/` become the only consumers of these manifests, so the repo root is the natural owner.

The repo-root layout has two ergonomic wins over a sidecar `cafleet-playground/` subdir: (1) Bun resolves `node_modules/` at the calling pane's CWD (the repo root) — no `--cwd` flag, no `--cwd <dir> install` script-vs-subcommand parsing quirk; (2) the existing uv workspace already lives at the repo root, so matplotlib slots in alongside the cafleet python deps without forking a parallel uv tree.

`<repo root>/node_modules/` and `<repo root>/.venv/` are derived artifacts and stay out of git via repository `.gitignore` entries.

#### Removal of the `~/.claude/skills/` snapshots

The four `~/.claude/skills/<name>/` directories are deleted as part of this migration; the ported `.claude/skills/<name>/` directories inside this repo are the only remaining copies. The `~/.claude/agents/slide-creator.md` and `~/.claude/agents/web-researcher.md` subagents are also moved into the cafleet repo at `.claude/agents/` and slide-creator's body is rewritten to invoke `my-slidev` (no namespace). Users who previously typed `/research-report`, `/research-presentation`, `/my-slidev`, or `/create-figure` get the same slash commands resolved to the project-local `.claude/skills/<name>/SKILL.md` files when working inside the cafleet repo.

### Mise task wrappers (repo-root)

The repo-root `mise.toml` adds the following tasks so callers do not have to remember the canonical `bun ...` / `uv run ...` invocation invariants (`--frozen-lockfile`, `--frozen --group research`, etc.):

| Task | Wraps |
|---|---|
| `mise //:bun-install` | `bun install --frozen-lockfile` |
| `mise //:slidev <slide>` | `script -qfc 'bun run slidev --open false ${usage_slide}'` (long-running dev server inside a PTY so Slidev does not detect a non-TTY stdout and exit early) |
| `mise //:figure <script>` | `uv run --frozen --group research <script>` |

`agent-browser` is intentionally NOT wrapped in a mise task — its existing `bun run agent-browser <args>` invocation form is kept verbatim across all ported `SKILL.md` files, with the corresponding allow / deny patterns migrated into the cafleet repo's `.claude/settings.json`.

### `CLAUDE_HOME` rebinding inside the ported skills

`create-figure/SKILL.md` currently references the user's Claude config directory through a `CLAUDE_HOME` placeholder:

```text
uv run --frozen --project $CLAUDE_HOME ${SRC_DIR}/script_name.py
```

After the port:

```text
mise //:figure ${SRC_DIR}/script_name.py
```

(equivalently `uv run --frozen --group research ${SRC_DIR}/script_name.py` — the mise wrapper exists to capture the `--frozen --group research` invariants).

Every occurrence of `CLAUDE_HOME` and the surrounding "substitute with the absolute path of the user's Claude config directory" guidance in `create-figure/SKILL.md` is rewritten to point at the repo-root uv `research` dependency group. The skill no longer needs to compute a per-invocation absolute path: `mise //:figure` (and the underlying `uv run --frozen --group research`) resolve everything from the calling pane's CWD when that CWD is anywhere inside the cafleet checkout.

#### Bun invocation rebinding (`research-presentation/SKILL.md`)

`my-slidev/SKILL.md` does not currently invoke Bun directly — Slidev and agent-browser invocations live in `research-presentation/SKILL.md`. After the port, every Bun invocation in `research-presentation/SKILL.md` is rewritten to the corresponding `mise <task>` form (which wraps the canonical `bun ...` invocation):

| Original | Rewritten |
|---|---|
| `bun install --frozen-lockfile` | `mise //:bun-install` |
| `mise run slidev <folder>/slide.md` | `mise //:slidev <folder>/slide.md` |
| `bun run agent-browser <args>` | `bun run agent-browser <args>` (kept verbatim — no mise wrapper for agent-browser) |
| `bun run agent-browser close --all` | `bun run agent-browser close --all` (kept verbatim) |

The mise wrappers run from the calling pane's CWD (the cafleet repo root), and Bun resolves `node_modules/` + `package.json` from the same root — no `--cwd` flag is needed.

The pre-existing working-directory invariant in `research-presentation/SKILL.md` Step 3 ("`Working directory: project root (the directory containing `node_modules/` and `skills/`)`") is also rewritten as part of this same edit. Post-port, `node_modules/` lives at the cafleet repo root next to `skills/`, so the invariant becomes "`Calling-pane working directory: cafleet repo root.`" — Bun resolves all manifests from the same root without any `--cwd` plumbing.

### Documentation surfaces to update

Per the project's `Implementation Order` rule, documentation precedes code. The documentation pass covers:

| File | Update |
|---|---|
| `README.md` | Add a Project Skills bullet for each new `/cafleet:*` skill. Add the repo-root toolchain entries (Bun manifests at root + `[dependency-groups.research]`) to the project-structure section. |
| `ARCHITECTURE.md` | (`ARCHITECTURE.md` exists at the repo root — confirmed during drafting.) Append the four skills to the skill catalog and describe the repo-root toolchain (Bun + uv `research` group) home for the ported skills. |
| `CLAUDE.md` | Add a Project Skills entry per ported skill in the same shape as the existing `cafleet:design-doc-*` entries. |
| `.claude/CLAUDE.md` | Mirror the `CLAUDE.md` additions. |
| `mise.toml` (repo root) | Add the `bun-install`, `slidev`, `figure` task wrappers per *Mise task wrappers*. (No mise wrapper for `agent-browser` — `bun run agent-browser <args>` stays verbatim.) |
| `.claude/skills/research-report/SKILL.md` | Slash-ref rewrites per *Internal reference rewrites*. |
| `.claude/skills/research-presentation/SKILL.md` | Slash-ref rewrites + hard-coded role-path rewrites + Bun invocation rebinding to the repo-root mise wrappers per *Bun invocation rebinding*. |
| `.claude/skills/my-slidev/SKILL.md` | Headmatter guidance ("`<absolute-path-to-this-skill's-theme-directory>`") points at the cafleet location for the theme path. |
| `.claude/skills/create-figure/SKILL.md` | `CLAUDE_HOME` rebinding to the repo-root uv `research` dependency group (`mise //:figure` / `uv run --frozen --group research`). |

`/update-readme` is unaffected; it remains the helper for re-syncing `README.md` from `ARCHITECTURE.md` + `docs/` after future doc-source changes.

### Verification: repo-root smoke test

This is a documentation-only port plus a manifest move. There is no automated pytest suite — that matches the precedent in `0000045`, where the structural pytest suite was dropped post-review because it asserted markdown substrings rather than user-visible behavior.

The verification sequence is:

1. **Skill resolution** — start a fresh Claude Code session inside this repo. Confirm the system-reminder skill list includes:
   - `research-report`
   - `research-presentation`
   - `my-slidev`
   - `create-figure`
   each with the description from its ported `SKILL.md` front-matter. (Note: this requires the cafleet plugin cache to include the new skills — typically a `/plugin install` refresh after the design lands. Pre-release working-tree-only verification is partial.)
2. **`create-figure` end-to-end** — invoke `/create-figure` with a trivial inline dataset (e.g., four bar values). Confirm:
   - the script is written under a `figures/src/` directory the skill resolves,
   - `mise //:figure <script>` (equivalently `uv run --frozen --group research <script>`) succeeds against the repo-root `pyproject.toml` `[dependency-groups.research]` and `uv.lock`,
   - a PNG lands in `figures/output/`.
3. **`my-slidev` toolchain reachability** — author a 2-slide deck (cover + bullets) referencing the ported theme path. Run `mise //:bun-install` from the cafleet repo root to populate `node_modules/`, then verify the Slidev binary is reachable from the repo-root Bun environment (`bun run slidev --version`). End-to-end deck rendering uses `mise //:slidev <deck>` (long-running dev server inside `script -qfc`).

Smoke-testing `/research-report` and `/research-presentation` end-to-end is out of scope — both spawn cafleet sessions with multiple long-lived members and would consume substantial wall-clock time. Their resolution is covered by step 1; their internal-reference correctness is covered by review of the rewritten `SKILL.md` files.

### Out of scope

- Removing or rewriting the `~/.claude/skills/<skill>/` directories. Those snapshots stay in place; the user removes them at their own pace. (Coexistence behavior is specified under *Coexistence semantics* in the Background.)
- Editing `0000045`'s design doc.
- Forking `Skill(base-dir)` into the cafleet plugin (an open question carried over from `0000045`; still deferred).
- Shipping the ported skills inside the cafleet wheel via `pyproject.toml` packaging changes. Filesystem-only delivery matches the existing `cafleet/skills/` entries.
- Renaming any of the four skills.

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-05-10T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

### Step 1: Documentation prerequisites

Documentation lands before any file move or skill copy.

- [x] Update `README.md` Project Skills section with one bullet per ported skill (`/research-report`, `/research-presentation`, `/my-slidev`, `/create-figure`) plus repo-root toolchain entries (Bun manifests at root + `[dependency-groups.research]`) under the project-structure section. <!-- completed: 2026-05-10T06:50 -->
- [x] Update `ARCHITECTURE.md` (which exists at the repo root) to add the four ported skills to the skill catalog and to describe the repo-root Bun + uv `research`-group toolchain for the ported skills. <!-- completed: 2026-05-10T06:51 -->
- [x] Update root `CLAUDE.md` Project Skills section with one entry per ported skill, matching the format of the existing `cafleet:design-doc-*` entries. <!-- completed: 2026-05-10T06:52 -->
- [x] Mirror the `CLAUDE.md` additions in `.claude/CLAUDE.md`. <!-- completed: 2026-05-10T07:00 -->
- [x] Add the repo-root `mise.toml` task wrappers (`bun-install`, `slidev`, `figure`) per *Mise task wrappers*. <!-- completed: 2026-05-10T19:00 -->


### Step 2: Migrate Bun + uv manifests into the cafleet repo root

- [x] Move `~/.claude/package.json` → `<repo root>/package.json` (verbatim). <!-- completed: 2026-05-10T07:25 -->
- [x] Move `~/.claude/bun.lock` → `<repo root>/bun.lock` (verbatim). <!-- completed: 2026-05-10T07:25 -->
- [x] Add a `[dependency-groups.research]` group to the existing repo-root `pyproject.toml` containing `matplotlib>=3.10.8` (the only non-trivial dep from `~/.claude/pyproject.toml`); delete `~/.claude/pyproject.toml`. <!-- completed: 2026-05-10T16:30 -->
- [x] Re-resolve the existing repo-root `uv.lock` to include the new `research` group; delete `~/.claude/uv.lock`. <!-- completed: 2026-05-10T16:30 -->
- [x] Ensure `node_modules/` and `.venv/` at the repo root are covered by `.gitignore`. <!-- completed: 2026-05-10T16:30 -->

### Step 3: Port skill directories

- [x] Copy `~/.claude/skills/research-report/` → `.claude/skills/research-report/` verbatim (`SKILL.md`, `roles/`, `template.md`); delete the source after copy. <!-- completed: 2026-05-10T07:42 (copy), 2026-05-10T22:00 (source deleted) -->
- [x] Copy `~/.claude/skills/research-presentation/` → `.claude/skills/research-presentation/` verbatim (`SKILL.md`, `roles/`); delete the source after copy. <!-- completed: 2026-05-10T07:42 (copy), 2026-05-10T22:00 (source deleted) -->
- [x] Copy `~/.claude/skills/my-slidev/` → `.claude/skills/my-slidev/` verbatim, including the entire `theme/` subtree (CSS, layouts, Vue components); delete the source after copy. <!-- completed: 2026-05-10T07:42 (copy), 2026-05-10T22:00 (source deleted) -->
- [x] Copy `~/.claude/skills/create-figure/` → `.claude/skills/create-figure/` verbatim (`SKILL.md` only); delete the source after copy. <!-- completed: 2026-05-10T07:42 (copy), 2026-05-10T22:00 (source deleted) -->
- [x] Move `~/.claude/agents/slide-creator.md` → `.claude/agents/slide-creator.md` and rewrite its `my-slidev` references to use the unprefixed project-local skill name. Move `~/.claude/agents/web-researcher.md` → `.claude/agents/web-researcher.md`. Delete `~/.claude/mise.toml` (the `[tasks.slidev]` content is now in the repo-root `mise.toml`). Delete stale `~/.claude/node_modules/` and `~/.claude/.venv/` artifacts. <!-- completed: 2026-05-10T22:00 -->

### Step 4: Apply internal-reference rewrites

- [x] In `.claude/skills/research-report/SKILL.md`, rewrite `/research-presentation` references in body prose to `/research-presentation`. Leave `Skill(...)` invocations untouched. <!-- completed: 2026-05-10T07:46 -->
- [x] In `.claude/skills/research-presentation/SKILL.md`, rewrite `/research-report`, `/my-slidev`, and `/create-figure` references in body prose to their `/cafleet:*` forms. <!-- completed: 2026-05-10T07:47 -->
- [x] In `.claude/skills/research-presentation/SKILL.md`, rewrite the two hard-coded role-file paths from `~/.claude/skills/research-presentation/roles/<file>.md` to `.claude/skills/research-presentation/roles/<file>.md` (resolved from project root). <!-- completed: 2026-05-10T07:48 -->
- [x] In `.claude/skills/research-presentation/SKILL.md`, rewrite every Bun invocation (`bun install --frozen-lockfile`, `mise run slidev`, `bun run agent-browser ...`, `bun run agent-browser close --all`) to the `mise <task>` wrapper form per the rebinding table in *Bun invocation rebinding* (mise tasks live in the repo-root `mise.toml`). <!-- completed: 2026-05-10T07:50 -->
- [x] In `.claude/skills/research-presentation/SKILL.md` Step 3, rewrite the working-directory invariant prose paragraph ("Working directory: project root (the directory containing `node_modules/` and `skills/`)…") so it reflects the post-rebinding layout: calling-pane working directory is the cafleet repo root, and Bun resolves `node_modules/` + `package.json` from the same root with no `--cwd` flag. <!-- completed: 2026-05-10T07:50 -->
- [x] Rewrite YAML front-matter `description:` strings in `.claude/skills/research-presentation/SKILL.md` and `.claude/skills/create-figure/SKILL.md` so any cross-references to the four ported skills use the `/cafleet:*` form (e.g., `Do NOT use for research — use /research-report for that.`). The token list is the same as the body-prose rewrite. <!-- completed: 2026-05-10T07:51 -->
- [x] In `.claude/skills/my-slidev/SKILL.md`, update the headmatter `theme:` guidance to point at `.claude/skills/my-slidev/theme/` resolved from the cafleet repo root. <!-- completed: 2026-05-10T07:52 -->
- [x] In `.claude/skills/create-figure/SKILL.md`, rewrite every `CLAUDE_HOME` reference and the surrounding "substitute with the absolute path of the user's Claude config directory" guidance to point at the repo-root uv `research` dependency group. The uv command becomes `mise //:figure <script>` (equivalently `uv run --frozen --group research <script>`). <!-- completed: 2026-05-10T07:54 -->

### Step 5: Smoke test

- [ ] Start a fresh Claude Code session inside this repo and confirm the system-reminder skill list includes `research-report`, `research-presentation`, `my-slidev`, and `create-figure` with descriptions matching the ported `SKILL.md` front-matter. Record the run in the Changelog. <!-- deferred: pre-release working tree cannot self-verify; the cafleet plugin loads from the version-pinned cache directory (`~/.claude/plugins/cache/cafleet/cafleet/0.6.1/skills/`), and new skills become visible under the `cafleet:` namespace only after the plugin is bumped and reinstalled. -->
- [x] Run `grep -E '~/\.claude|CLAUDE_HOME' .claude/skills/research-report/SKILL.md .claude/skills/research-presentation/SKILL.md .claude/skills/my-slidev/SKILL.md .claude/skills/create-figure/SKILL.md` and confirm zero matches. This is the canonical post-rewrite check that Step 4 caught every stale reference in the four top-level `SKILL.md` files; per Success Criterion 8, role files under `roles/` are out of scope. <!-- completed: 2026-05-10T19:00 -->
- [x] Run `mise //:bun-install` (equivalently `bun install --frozen-lockfile` from the repo root) to populate `<repo root>/node_modules/`. Confirm exit 0. <!-- completed: 2026-05-10T19:00 (bun install via mise //:bun-install installed 622 packages) -->
- [x] Invoke `/create-figure` with a trivial inline dataset (four bar values). Confirm a PNG lands under the resolved `figures/output/` directory and that the uv command used `--frozen --group research` (`mise //:figure <script>`). <!-- completed: 2026-05-10T19:00 (mise //:figure rendered /tmp/claude-code/verifier/figures/output/test_bars.png via uv run --frozen --group research) -->
- [x] Author a 2-slide deck (cover + bullets) referencing `.claude/skills/my-slidev/theme/` and verify the Slidev binary is reachable from the repo-root Bun environment via `bun run slidev --version`. End-to-end deck rendering uses `mise //:slidev <deck>` (long-running dev server inside `script -qfc 'bun run slidev --open false ${usage_slide}'` so Slidev does not exit on non-TTY stdout). <!-- completed: 2026-05-10T19:00 (bun run slidev --version → 52.14.1; deck authored at /tmp/claude-code/verifier/slides/test-deck.md) -->

### Step 6: Commit

- [x] Stage `design-docs/0000052-port-research-slidev-skills/design-doc.md`, the repo-root `package.json` + `bun.lock` + `pyproject.toml` + `uv.lock`, the repo-root `mise.toml` task additions, `skills/{research-report,research-presentation,my-slidev,create-figure}/`, the documentation updates from Step 1, the `.claude/settings.json` permissions migration, and the `.gitignore` change from Step 2. <!-- completed: 2026-05-10T19:00 (work landed across commits f8a200c, b1de32f, 16996ac, be14c8e, 722c1f7 on feat/port-research-slidev-skills; the pivot consolidated to flat repo-root toolchain in 722c1f7) -->

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-05-10 | Initial draft |
| 2026-05-10 | Mid-implementation pivot from a `cafleet-playground/` subdirectory layout to a flat repo-root toolchain. The original layout placed all four manifests under a sidecar `cafleet-playground/`, requiring `bun --cwd <dir> ...` and `uv run --project <dir> ...` plumbing on every invocation. Bun parses `bun --cwd <dir> install` as a script lookup (`Script not found "install"`), which surfaced during the first smoke-test attempt; the corrected form `bun install --cwd <dir>` works but `bun --cwd <dir> run <name>` quirks remain. The flat layout (Bun manifests at repo root + matplotlib in the existing `pyproject.toml` `[dependency-groups.research]` group) sidesteps both quirks and avoids forking a parallel uv tree. Step 2 task list, Step 4 Bun rebinding table, Step 5 commands, Background § *Why repo-root toolchain*, Specification § *Repo-root toolchain layout*, Documentation surfaces (added `mise.toml` row), and Verification section all rewritten in this pass. Step 1–4 implementation work was already shipped against the subdirectory layout; the same commits are now reinterpreted as having migrated to the flat layout, with follow-up edits removing the `cafleet-playground/` directory entirely and folding matplotlib into the repo-root `pyproject.toml`. |
