# Port Research and Slidev Skills into cafleet, with `cafleet-playground/` Toolchain Consolidation

**Status**: Approved
**Progress**: 22/28 tasks complete
**Last Updated**: 2026-05-10

## Overview

Port the four global skills `research-report`, `research-presentation`, `my-slidev`, and `create-figure` from `~/.claude/skills/` into this repository under `skills/`, exposing them as `/cafleet:research-report`, `/cafleet:research-presentation`, `/cafleet:my-slidev`, and `/cafleet:create-figure`. Alongside the port, consolidate the Bun (Slidev / agent-browser) and uv (matplotlib) toolchains those skills depend on into a new top-level `cafleet-playground/` directory at the cafleet repo root, replacing the current `CLAUDE_HOME = ~/.claude` assumption with a self-contained playground.

## Success Criteria

- [ ] `skills/research-report/`, `skills/research-presentation/`, `skills/my-slidev/`, and `skills/create-figure/` exist and are loaded as `cafleet:research-report`, `cafleet:research-presentation`, `cafleet:my-slidev`, and `cafleet:create-figure` (visible in the system-reminder skill list when the cafleet plugin is active).
- [ ] `cafleet-playground/` exists at the repo root and contains `package.json`, `bun.lock`, `pyproject.toml`, and `uv.lock` migrated from `~/.claude/`.
- [ ] `/cafleet:create-figure` renders a trivial chart end-to-end using the `cafleet-playground/` uv environment.
- [ ] `/cafleet:my-slidev` compiles a 2-slide example using the `cafleet-playground/` Bun environment.
- [ ] `README.md`, `ARCHITECTURE.md`, `CLAUDE.md`, and `.claude/CLAUDE.md` list the four new skills and document `cafleet-playground/` as a top-level directory.
- [ ] Internal slash-command cross-references inside the four ported `SKILL.md` files (covering both body prose AND YAML front-matter `description:` strings) are rewritten to the cafleet namespace (`/research-presentation` → `/cafleet:research-presentation`, `/research-report` → `/cafleet:research-report`, `/my-slidev` → `/cafleet:my-slidev`, `/create-figure` → `/cafleet:create-figure`).
- [ ] `research-presentation/SKILL.md` no longer hard-codes `~/.claude/skills/research-presentation/roles/...` paths; it uses repo-relative `skills/research-presentation/roles/...` instead.
- [ ] `grep -rE '~/\.claude|CLAUDE_HOME' skills/research-report/ skills/research-presentation/ skills/my-slidev/ skills/create-figure/` returns no matches after Step 4 completes.

---

## Background

### Why this port

Two of the four skills already invoke cafleet primitives. `research-report/SKILL.md` and `research-presentation/SKILL.md` both load `Skill(cafleet)` and `Skill(cafleet:agent-team-monitoring)` and orchestrate Director / Manager / Researcher / Presentation / Transcript / Visual-Reviewer members via `cafleet member create`. Their natural home is inside the cafleet plugin, not the global `~/.claude/skills/` tree, because:

- The skills produce admin-WebUI message-timeline traffic that only makes sense alongside the cafleet broker.
- Internal references (role-file paths, slash-command cross-references) currently leak out of the skill directory into `~/.claude/skills/...`. Repo-relative paths inside the cafleet plugin keep the port self-contained.
- `my-slidev` and `create-figure` are already named in the spawn prompts of `research-presentation` (`Skill(my-slidev)`, `Skill(create-figure)`). Porting all four together preserves the dependency graph.

### Why `cafleet-playground/`

`create-figure/SKILL.md` runs matplotlib through `uv run --frozen --project <CLAUDE_HOME>` against `~/.claude/pyproject.toml` + `~/.claude/uv.lock`. `research-presentation/SKILL.md` runs Slidev through `bun install --frozen-lockfile` plus `mise run slidev` against `~/.claude/package.json` + `~/.claude/bun.lock`. Both toolchains live in the user's global Claude config directory, so the ported skills currently depend on artifacts outside the cafleet repository.

A new `cafleet-playground/` directory at the cafleet repo root collects these manifests in one place, gives the ported skills a self-contained working directory, and matches the user-controllable boundary that the cafleet plugin already owns. After the port, `~/.claude/{package.json,bun.lock,pyproject.toml,uv.lock}` are migrated into `cafleet-playground/`; the four `SKILL.md` files rebind their `CLAUDE_HOME`-style references to `cafleet-playground/`.

### Relationship to design doc 0000045

`design-docs/0000045-cafleet-design-doc-interview/design-doc.md` includes a classification table that placed the four skills in scope here as "general-purpose utility skills with no cafleet coupling; staying global is correct." That classification has drifted from reality: `research-report` and `research-presentation` already invoke `Skill(cafleet)` and `Skill(cafleet:agent-team-monitoring)` in their spawn prompts, and the present design doc decides to port all four anyway.

This design doc supersedes that verdict, scoped strictly:

(a) **Superseded rows.** Four rows in the `0000045` "Other global-only skills considered" table — `create-figure`, `my-slidev`, `research-report`, and `research-presentation` — all originally classified as "general-purpose utility skills with no cafleet coupling; staying global is correct."

(b) **Replacement classification.** Ported into the cafleet plugin under the `cafleet:<name>` namespace; the canonical source is `cafleet/skills/<name>/` inside this repository.

(c) **Coexist semantics, restated.** The supersession is about the *classification verdict*, not about file location. The pre-existing `~/.claude/skills/<skill>/` directories remain in place per the user's coexist directive (Q1 in the clarification round) — the user removes them at their own pace.

`0000045` itself is left untouched. The historical classification stays in `0000045` as the record of what was true at the time it shipped; the current state is described in this document.

### Source-of-truth direction post-port

`cafleet/skills/*` is canonical going forward. The pre-existing `~/.claude/skills/{research-report,research-presentation,my-slidev,create-figure}/` directories become frozen snapshots — the user removes them at their own pace, but no future edits land there. Any future change to the four skills lands inside this repo only.

### Coexistence semantics

Before this design lands, only `~/.claude/skills/<skill>/` exists; users invoke `/research-report`, `/research-presentation`, `/my-slidev`, `/create-figure` and Claude Code resolves them globally. After this design lands, both copies exist and are listed in the system-reminder skill list in parallel:

| User-typed slash command | Resolves to |
|---|---|
| `/research-report` | `~/.claude/skills/research-report/SKILL.md` (global, unchanged) |
| `/cafleet:research-report` | `cafleet/skills/research-report/SKILL.md` (this port) |
| `/research-presentation` | `~/.claude/skills/research-presentation/SKILL.md` (global; non-functional after manifest move — see *Consequence for the `~/.claude/skills/` snapshots* below) |
| `/cafleet:research-presentation` | `cafleet/skills/research-presentation/SKILL.md` (this port) |
| `/my-slidev` | `~/.claude/skills/my-slidev/SKILL.md` (global, unchanged) |
| `/cafleet:my-slidev` | `cafleet/skills/my-slidev/SKILL.md` (this port) |
| `/create-figure` | `~/.claude/skills/create-figure/SKILL.md` (global; non-functional after manifest move) |
| `/cafleet:create-figure` | `cafleet/skills/create-figure/SKILL.md` (this port) |

The two copies coexist; neither overrides the other. Claude Code's skill resolver dispatches by exact slash name. Listing both in the system-reminder skill list is the **expected** post-port state — it is not a regression. The user removes the global copies at their own pace; until then the only consequence is duplicated entries in the skill list (informational only).

---

## Specification

### Scope

Four skills are ported. For each, the port copies the entire skill directory tree (`SKILL.md` + any `roles/` / `theme/` / `template.md` subtree) verbatim from `~/.claude/skills/<skill>/` into `skills/<skill>/`, then applies the textual edits described under *Internal reference rewrites* below.

| Source | Target | Notes |
|---|---|---|
| `~/.claude/skills/research-report/` | `skills/research-report/` | Includes `SKILL.md`, `roles/` (director, manager, scout, researcher), `template.md` |
| `~/.claude/skills/research-presentation/` | `skills/research-presentation/` | Includes `SKILL.md`, `roles/` (director, presentation, transcript, visual-reviewer) |
| `~/.claude/skills/my-slidev/` | `skills/my-slidev/` | Includes `SKILL.md` and the entire `theme/` subtree (CSS, layouts, Vue components) — copied verbatim |
| `~/.claude/skills/create-figure/` | `skills/create-figure/` | Just `SKILL.md`; no subdirectories |

### Internal reference rewrites

The user's answers limit rewrites to slash-command cross-references plus the two hard-coded role paths. All other `Skill(...)` invocations stay as-is (they resolve via the global skill resolver regardless of namespace).

#### Slash-command rewrites

Inside each ported `SKILL.md` body text, rewrite cross-references between the ported skills to use the cafleet namespace:

| Original token | Rewritten token | Affected files |
|---|---|---|
| `/research-report` | `/cafleet:research-report` | `research-presentation/SKILL.md` (description, Step 0 error message) |
| `/research-presentation` | `/cafleet:research-presentation` | `research-report/SKILL.md` (description, Step 7) |
| `/my-slidev` (in body prose) | `/cafleet:my-slidev` | `research-presentation/SKILL.md` (Presentation role section) |
| `/create-figure` (in body prose) | `/cafleet:create-figure` | `research-presentation/SKILL.md` (Presentation role section) |

`Skill(...)` calls (e.g., `Skill(cafleet)`, `Skill(cafleet:agent-team-monitoring)`, `Skill(base-dir)`, `Skill(my-slidev)`, `Skill(create-figure)`) are **not** rewritten. They resolve via the existing skill loader and the user has explicitly chosen to keep them unchanged.

References to other plugins' slash commands — for example `/slidev` and `/slidev:slidev` in `my-slidev/SKILL.md` — are also left as-is. Only cross-references between the four ported skills are namespaced.

#### Hard-coded role paths in `research-presentation/SKILL.md`

Two embedded spawn prompts hard-code absolute paths under `~/.claude`:

- `~/.claude/skills/research-presentation/roles/transcript.md` → `skills/research-presentation/roles/transcript.md` (resolved from project root)
- `~/.claude/skills/research-presentation/roles/visual-reviewer.md` → `skills/research-presentation/roles/visual-reviewer.md` (resolved from project root)

After the rewrite, the ported `SKILL.md` reads its own role files. No reference to `~/.claude` remains.

### `cafleet-playground/` layout

A new top-level directory:

```
cafleet-playground/
├── package.json     # Bun manifest (Slidev, agent-browser, etc.)
├── bun.lock         # Bun lockfile
├── pyproject.toml   # uv manifest (matplotlib, Pillow, etc.)
└── uv.lock          # uv lockfile
```

The four files are migrated from the user's `~/.claude/` directory:

| Source path | Target path |
|---|---|
| `~/.claude/package.json` | `cafleet-playground/package.json` |
| `~/.claude/bun.lock` | `cafleet-playground/bun.lock` |
| `~/.claude/pyproject.toml` | `cafleet-playground/pyproject.toml` |
| `~/.claude/uv.lock` | `cafleet-playground/uv.lock` |

The migration is a move, not a copy (per the user's Q6 directive): after the design lands the files are no longer at `~/.claude/`. The ported skills under `cafleet/skills/<name>/` become the only consumers of these manifests, so `cafleet-playground/` is the natural owner.

#### Consequence for the `~/.claude/skills/` snapshots

The frozen `~/.claude/skills/<skill>/` snapshots described under *Source-of-truth direction post-port* coexist as reference material, but moving the manifests has a known and intentional functional impact:

| Snapshot | Functional after manifest move? | Reason |
|---|---|---|
| `~/.claude/skills/research-report/` | Yes | No Bun or uv toolchain dependency. |
| `~/.claude/skills/my-slidev/` | Yes | `SKILL.md` is documentation-only; no toolchain invocation. |
| `~/.claude/skills/research-presentation/` | No (Bun ops fail) | Step 3 expects `~/.claude/package.json` + `~/.claude/bun.lock` to be present. After the move those files no longer exist. |
| `~/.claude/skills/create-figure/` | No (uv ops fail) | The `uv run --frozen --project $CLAUDE_HOME ...` invocation requires `~/.claude/pyproject.toml` + `~/.claude/uv.lock`. After the move those files no longer exist. |

This is the intended outcome of the port. The canonical implementations live under `cafleet/skills/<name>/` with `cafleet-playground/` as their toolchain home; the `.claude` snapshots remain readable as historical reference until the user removes them. Users who still rely on the toolchain-dependent skills should switch to the `/cafleet:*` invocation before this design lands. This is **not** the partial-removal pattern proscribed by `.claude/rules/removal.md`: the snapshots are not edited, no deprecation notices are added inline, and the historical record (the `0000045` table entry plus this design doc) lives entirely outside the snapshot files.

`cafleet-playground/node_modules/` and `cafleet-playground/.venv/` are derived artifacts and stay out of git via repository `.gitignore` entries (added in the implementation phase if not already covered).

### `CLAUDE_HOME` rebinding inside the ported skills

`create-figure/SKILL.md` currently references the user's Claude config directory through a `CLAUDE_HOME` placeholder:

```text
uv run --frozen --project $CLAUDE_HOME ${SRC_DIR}/script_name.py
```

After the port:

```text
uv run --frozen --project ${CAFLEET_REPO_ROOT}/cafleet-playground ${SRC_DIR}/script_name.py
```

**Path resolution.** Every occurrence of `CLAUDE_HOME` and the surrounding "substitute with the absolute path of the user's Claude config directory" guidance in `create-figure/SKILL.md` is rewritten so the playground is resolved as an absolute path. The skill computes `${CAFLEET_REPO_ROOT}/cafleet-playground/` at invocation time, where `${CAFLEET_REPO_ROOT}` is discovered via `Skill(base-dir)` (the same mechanism the skill already uses to resolve `${BASE}`). This eliminates any CWD precondition: callers may invoke `/cafleet:create-figure` from any working directory because the skill resolves the absolute playground path itself. If `Skill(base-dir)` cannot resolve a cafleet repo root (e.g. the skill is invoked from a directory that is not inside the cafleet checkout), the skill errors out with a clear message rather than falling back to a CWD-relative path.

#### Bun invocation rebinding (`research-presentation/SKILL.md`)

`my-slidev/SKILL.md` does not currently invoke Bun directly — Slidev and agent-browser invocations live in `research-presentation/SKILL.md`. After the port, every Bun invocation in `research-presentation/SKILL.md` is rewritten to use `bun --cwd cafleet-playground ...`, where `cafleet-playground` is resolved through the same `${CAFLEET_REPO_ROOT}/cafleet-playground` mechanism described above:

| Original | Rewritten |
|---|---|
| `bun install --frozen-lockfile` | `bun --cwd ${CAFLEET_REPO_ROOT}/cafleet-playground install --frozen-lockfile` |
| `mise run slidev <folder>/slide.md` | `bun --cwd ${CAFLEET_REPO_ROOT}/cafleet-playground run slidev <folder>/slide.md` |
| `bun run agent-browser <args>` | `bun --cwd ${CAFLEET_REPO_ROOT}/cafleet-playground run agent-browser <args>` |
| `bun run agent-browser close --all` | `bun --cwd ${CAFLEET_REPO_ROOT}/cafleet-playground run agent-browser close --all` |

This keeps the rebinding self-contained to the `SKILL.md` edit — **no new `mise.toml` task is added.** The `--cwd` flag tells Bun to resolve `package.json` + `bun.lock` from the playground directory while the calling pane's working directory remains at the cafleet repo root.

The pre-existing working-directory invariant in `research-presentation/SKILL.md` Step 3 ("`Working directory: project root (the directory containing `node_modules/` and `skills/`)`") is also rewritten as part of this same edit. Post-port, `node_modules/` lives under `cafleet-playground/node_modules/`, so the invariant becomes "`Calling-pane working directory: cafleet repo root. Bun working directory (where `node_modules/` and `package.json` resolve): ${CAFLEET_REPO_ROOT}/cafleet-playground/.`"

### Documentation surfaces to update

Per the project's `Implementation Order` rule, documentation precedes code. The documentation pass covers:

| File | Update |
|---|---|
| `README.md` | Add a Project Skills bullet for each new `/cafleet:*` skill. Add `cafleet-playground/` to the project-structure section. |
| `ARCHITECTURE.md` | (`ARCHITECTURE.md` exists at the repo root — confirmed during drafting.) Append the four skills to the skill catalog and describe `cafleet-playground/` as the Bun + uv toolchain home for the ported skills. |
| `CLAUDE.md` | Add a Project Skills entry per ported skill in the same shape as the existing `cafleet:design-doc-*` entries. |
| `.claude/CLAUDE.md` | Mirror the `CLAUDE.md` additions. |
| `skills/research-report/SKILL.md` | Slash-ref rewrites per *Internal reference rewrites*. |
| `skills/research-presentation/SKILL.md` | Slash-ref rewrites + hard-coded role-path rewrites + Bun working-directory rebinding to `cafleet-playground/`. |
| `skills/my-slidev/SKILL.md` | Headmatter guidance ("`<absolute-path-to-this-skill's-theme-directory>`") points at the cafleet location for the theme path. |
| `skills/create-figure/SKILL.md` | `CLAUDE_HOME` rebinding to `cafleet-playground/`. |

`/update-readme` is unaffected; it remains the helper for re-syncing `README.md` from `ARCHITECTURE.md` + `docs/` after future doc-source changes.

### Verification: smoke test from `cafleet-playground/`

This is a documentation-only port plus a manifest move. There is no automated pytest suite — that matches the precedent in `0000045`, where the structural pytest suite was dropped post-review because it asserted markdown substrings rather than user-visible behavior.

The verification sequence is:

1. **Skill resolution** — start a fresh Claude Code session inside this repo. Confirm the system-reminder skill list includes:
   - `cafleet:research-report`
   - `cafleet:research-presentation`
   - `cafleet:my-slidev`
   - `cafleet:create-figure`
   each with the description from its ported `SKILL.md` front-matter.
2. **`cafleet:create-figure` end-to-end** — invoke `/cafleet:create-figure` with a trivial inline dataset (e.g., four bar values). Confirm:
   - the script is written under a `figures/src/` directory the skill resolves,
   - `uv run --frozen --project ${CAFLEET_REPO_ROOT}/cafleet-playground <script>` succeeds against `cafleet-playground/pyproject.toml` + `cafleet-playground/uv.lock`,
   - a PNG lands in `figures/output/`.
3. **`cafleet:my-slidev` 2-slide compile** — author a 2-slide deck (cover + bullets) referencing the ported theme path. Run `bun --cwd cafleet-playground install --frozen-lockfile` from the cafleet repo root, then `bun --cwd cafleet-playground run slidev <deck>.md`. Confirm Slidev resolves the theme and produces a non-empty preview at `http://localhost:3030`.

Smoke-testing `/cafleet:research-report` and `/cafleet:research-presentation` end-to-end is out of scope — both spawn cafleet sessions with multiple long-lived members and would consume substantial wall-clock time. Their resolution is covered by step 1; their internal-reference correctness is covered by review of the rewritten `SKILL.md` files.

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

- [x] Update `README.md` Project Skills section with one bullet per ported skill (`/cafleet:research-report`, `/cafleet:research-presentation`, `/cafleet:my-slidev`, `/cafleet:create-figure`) plus a `cafleet-playground/` entry under the project-structure section. <!-- completed: 2026-05-10T06:50 -->
- [x] Update `ARCHITECTURE.md` (which exists at the repo root) to add the four ported skills to the skill catalog and to describe `cafleet-playground/` as the Bun + uv toolchain home for the ported skills. <!-- completed: 2026-05-10T06:51 -->
- [x] Update root `CLAUDE.md` Project Skills section with one entry per ported skill, matching the format of the existing `cafleet:design-doc-*` entries. <!-- completed: 2026-05-10T06:52 -->
- [x] Mirror the `CLAUDE.md` additions in `.claude/CLAUDE.md`. <!-- completed: 2026-05-10T07:00 -->


### Step 2: Create `cafleet-playground/` and migrate manifests

- [x] Create the `cafleet-playground/` directory at the cafleet repo root. <!-- completed: 2026-05-10T07:04 -->
- [x] Move `~/.claude/package.json` → `cafleet-playground/package.json`. <!-- completed: 2026-05-10T07:25 -->
- [x] Move `~/.claude/bun.lock` → `cafleet-playground/bun.lock`. <!-- completed: 2026-05-10T07:25 -->
- [x] Move `~/.claude/pyproject.toml` → `cafleet-playground/pyproject.toml`. <!-- completed: 2026-05-10T07:25 -->
- [x] Move `~/.claude/uv.lock` → `cafleet-playground/uv.lock`. <!-- completed: 2026-05-10T07:25 -->
- [x] Add `cafleet-playground/node_modules/` and `cafleet-playground/.venv/` to `.gitignore` if they are not already covered by an existing pattern. <!-- completed: 2026-05-10T07:05 -->

### Step 3: Port skill directories

- [x] Copy `~/.claude/skills/research-report/` → `skills/research-report/` verbatim (`SKILL.md`, `roles/`, `template.md`). <!-- completed: 2026-05-10T07:42 -->
- [x] Copy `~/.claude/skills/research-presentation/` → `skills/research-presentation/` verbatim (`SKILL.md`, `roles/`). <!-- completed: 2026-05-10T07:42 -->
- [x] Copy `~/.claude/skills/my-slidev/` → `skills/my-slidev/` verbatim, including the entire `theme/` subtree (CSS, layouts, Vue components). <!-- completed: 2026-05-10T07:42 -->
- [x] Copy `~/.claude/skills/create-figure/` → `skills/create-figure/` verbatim (`SKILL.md` only). <!-- completed: 2026-05-10T07:42 -->

### Step 4: Apply internal-reference rewrites

- [x] In `skills/research-report/SKILL.md`, rewrite `/research-presentation` references in body prose to `/cafleet:research-presentation`. Leave `Skill(...)` invocations untouched. <!-- completed: 2026-05-10T07:46 -->
- [x] In `skills/research-presentation/SKILL.md`, rewrite `/research-report`, `/my-slidev`, and `/create-figure` references in body prose to their `/cafleet:*` forms. <!-- completed: 2026-05-10T07:47 -->
- [x] In `skills/research-presentation/SKILL.md`, rewrite the two hard-coded role-file paths from `~/.claude/skills/research-presentation/roles/<file>.md` to `skills/research-presentation/roles/<file>.md` (resolved from project root). <!-- completed: 2026-05-10T07:48 -->
- [x] In `skills/research-presentation/SKILL.md`, rewrite every Bun invocation (`bun install --frozen-lockfile`, `mise run slidev`, `bun run agent-browser ...`, `bun run agent-browser close --all`) to the `bun --cwd ${CAFLEET_REPO_ROOT}/cafleet-playground ...` form per the rebinding table in *Bun invocation rebinding*. No new `mise.toml` task is added. <!-- completed: 2026-05-10T07:50 -->
- [x] In `skills/research-presentation/SKILL.md` Step 3, rewrite the working-directory invariant prose paragraph ("Working directory: project root (the directory containing `node_modules/` and `skills/`)…") so it reflects the post-rebinding layout: calling-pane working directory is the cafleet repo root, Bun working directory (where `node_modules/` resolves) is `${CAFLEET_REPO_ROOT}/cafleet-playground/`. <!-- completed: 2026-05-10T07:50 -->
- [x] Rewrite YAML front-matter `description:` strings in `skills/research-presentation/SKILL.md` and `skills/create-figure/SKILL.md` so any cross-references to the four ported skills use the `/cafleet:*` form (e.g., `Do NOT use for research — use /cafleet:research-report for that.`). The token list is the same as the body-prose rewrite. <!-- completed: 2026-05-10T07:51 -->
- [x] In `skills/my-slidev/SKILL.md`, update the headmatter `theme:` guidance to point at `skills/my-slidev/theme/` resolved from the cafleet repo root. <!-- completed: 2026-05-10T07:52 -->
- [x] In `skills/create-figure/SKILL.md`, rewrite every `CLAUDE_HOME` reference and the surrounding "substitute with the absolute path of the user's Claude config directory" guidance to resolve the playground via `Skill(base-dir)` as `${CAFLEET_REPO_ROOT}/cafleet-playground/`. The uv command becomes `uv run --frozen --project ${CAFLEET_REPO_ROOT}/cafleet-playground <script>`. <!-- completed: 2026-05-10T07:54 -->

### Step 5: Smoke test

- [ ] Start a fresh Claude Code session inside this repo and confirm the system-reminder skill list includes `cafleet:research-report`, `cafleet:research-presentation`, `cafleet:my-slidev`, and `cafleet:create-figure` with descriptions matching the ported `SKILL.md` front-matter. Record the run in the Changelog. <!-- completed: -->
- [ ] Run `grep -rE '~/\.claude|CLAUDE_HOME' skills/research-report/ skills/research-presentation/ skills/my-slidev/ skills/create-figure/` and confirm zero matches. This is the canonical post-rewrite check that Step 4 caught every stale reference. <!-- completed: -->
- [ ] Run `bun --cwd cafleet-playground install --frozen-lockfile` to populate `cafleet-playground/node_modules/`. Confirm exit 0. <!-- completed: -->
- [ ] Invoke `/cafleet:create-figure` with a trivial inline dataset (four bar values). Confirm a PNG lands under the resolved `figures/output/` directory and that the uv command used `--project ${CAFLEET_REPO_ROOT}/cafleet-playground` (i.e. the absolute path resolved through `Skill(base-dir)`). <!-- completed: -->
- [ ] Author a 2-slide deck (cover + bullets) referencing `skills/my-slidev/theme/` and compile it via the rebound Slidev command. Confirm the preview renders. <!-- completed: -->

### Step 6: Commit

- [ ] Stage `design-docs/0000052-port-research-slidev-skills/design-doc.md`, `cafleet-playground/`, `skills/{research-report,research-presentation,my-slidev,create-figure}/`, the documentation updates from Step 1, and the `.gitignore` change from Step 2. Commit with `feat: port research and slidev skills into cafleet plugin`. <!-- completed: -->

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-05-10 | Initial draft |
