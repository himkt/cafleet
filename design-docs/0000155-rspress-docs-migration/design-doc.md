# Migrate the Documentation Site from Zensical to Rspress

**Status**: Approved
**Progress**: 28/28 tasks complete
**Last Updated**: 2026-07-31

## Overview

Replace the Python-based zensical documentation toolchain with rspress v2, making `docs/` the rspress project root as a pnpm workspace package. The Python toolchain leaves the repository entirely: `pyproject.toml`, `uv.lock`, the uv tool pin, `zensical.toml`, and `.bumpversion.toml` are all removed. The published site keeps its URL (`https://himkt.github.io/cafleet/`), navigation structure, and content parity across all 19 pages — except the five mermaid diagrams, which are removed entirely by execute-time amendment (their surrounding prose stands alone).

## Success Criteria

- [x] `mise //:docs-build` builds the rspress site into `docs/doc_build` with all 19 pages, sidebar parity with the zensical nav, built-in search, and the light/dark toggle
- [x] No mermaid content remains: no fenced mermaid blocks, no `.mmd` sources, no diagram image references, and no mermaid tooling anywhere outside `design-docs/` and git history. Exempt: generic mentions of mermaid as a presentation-authoring feature in the extraction-bound cafleet-research skill — `skills/cafleet-research/reference/slidev.md:95` (Slidev's built-in slide-diagram feature) and `skills/cafleet-research/presentation/roles/presentation.md:42` (chart-type guidance)
- [x] The two `??? example` collapsibles render as `:::details` containers
- [x] The five `&#124;` table cells and the literal-brace headings in `spec/webui-api.md` render intact in the built HTML
- [x] Internal links resolve (`markdown.checkDeadLinks` fails the build on a broken route target) and fragment anchors resolve (verified by the dedicated Step 7 anchor task)
- [x] `.github/workflows/docs.yml` keeps its trigger shape (build on PR, deploy Pages on push to `main`) on the pnpm toolchain
- [x] No live reference to this repository's zensical / uv / bump-my-version toolchain remains outside `design-docs/` and git history. Exempt: generic tool-name examples that do not describe this repo's toolchain — `uv run` as a generic invocation example at `skills/cafleet-research/reference/visualization.md:72`, `uv.lock` as a generic lock-file example at `.claude/skills/clean-docs/residue/reference/patterns.md:17,31`

---

## Background

The docs site is built by zensical (a Python static-site generator) configured in `zensical.toml`, installed through the root uv workspace (`pyproject.toml` + `uv.lock`, `mise //:uv-sync`), and built by `mise //:docs-build` (`uv run zensical build --clean`) both locally and in `.github/workflows/docs.yml`, which deploys `site/` to GitHub Pages.

The root `pyproject.toml` carries two residents beyond zensical:

| Resident | Group | Current use |
|---|---|---|
| `bump-my-version` | `dev` | Release version bumps via `.bumpversion.toml` (rewrites the version in `cafleet/Cargo.toml` and `Cargo.lock`) |
| `matplotlib` | `research` | The cafleet-research visualization runner (`uv run --frozen --group research python <script>`, catalogued in `.claude/rules/commands.md`) |

Full Python removal therefore requires decisions on both; the user has made them (see *Recorded decisions*).

---

## Specification

### Recorded decisions (user-confirmed)

| # | Question | Decision |
|---|---|---|
| 1 | bump-my-version fate | Dropped entirely. `.bumpversion.toml` is deleted; releases bump the single `version` line in `cafleet/Cargo.toml` by hand and let the next cargo build refresh `Cargo.lock`. |
| 2 | matplotlib / research group fate | Removed. The cafleet-research skill is to be isolated into a separate repository (see *Scope*). |
| 3 | Workspace shape | rspress v2, with `docs/` added to `pnpm-workspace.yaml`; dependencies in `docs/package.json`; shared root `pnpm-lock.yaml`. |
| 4 | Mermaid diagrams | Removed entirely (user decision at execute time): the five diagrams are deleted from the docs — no `.mmd` sources, no rendered SVGs, no mermaid tooling anywhere in the repo. The prose surrounding each former diagram carries the content on its own. |
| 5 | Page icons | The `icon: lucide/...` frontmatter is deleted from all 19 pages. |
| 6 | Home page | Amended at execute time (user decision, final): an rspress.rs-style two-column hero — gradient `CAFleet` title, tagline, and the Quickstart/GitHub action buttons on the left; the demo video as the large, playable right-side visual; columns stack on narrow viewports. No sidebar on home (inherent to the hero layout). No description paragraph — the tagline is the pitch, and the `update-readme` skill aligns the README pitch with it. |
| 7 | CI / task | The root-level `mise //:docs-build` task name is kept, retargeted to the pnpm-driven rspress build. Same GitHub Pages URL (`base: '/cafleet/'`), same `docs.yml` trigger shape. |
| 8 | Content-root nesting | The user's original request makes `docs/` the rspress **project root**, scaffolded per the official getting-started guide — whose layout nests content in `<project-root>/docs`. The pages therefore move to `docs/docs/`, and the two repo-wide consequences (the `docs_sync.rs` test paths and every repo-internal reference to a `docs/...` page path) are updated in the same change. |

### Scope

**In scope**: the docs-site migration; total zensical removal; total Python toolchain removal (`pyproject.toml`, `uv.lock`, the uv tool pin and `uv-sync` task in `mise.toml`, the uv permission allows); removal of bump-my-version and `.bumpversion.toml`.

**Out of scope**: extracting `skills/cafleet-research/` into its own repository. That is a user-directed follow-up task. Interim consequence accepted here: the cafleet-research visualization runner row is removed from `.claude/rules/commands.md` together with the Python toolchain, so until the extraction lands the skill's visualization workflow has no in-repo Python runner (the skill itself is invocation-agnostic and needs no edit). The Director confirms this interim gap with the user at review time.

### Target layout

`docs/` becomes the rspress project root; the 19 markdown pages move one level down into the content root (`docs/docs/`, rspress's `root: 'docs'` scaffold default):

```
docs/
├── package.json              # workspace package "cafleet-docs"
├── rspress.config.ts
├── theme/index.tsx           # custom theme: places the demo video as the hero's right-side visual
├── theme/styles.css          # globalStyles: two-column hero styles + embed sizing
└── docs/                     # rspress content root
    ├── _nav.json
    ├── _meta.json
    ├── index.md              # hero home page
    ├── quickstart.md
    ├── contributing.md
    ├── how-to/  (+ _meta.json)
    ├── concepts/ (+ _meta.json)
    └── spec/     (+ _meta.json)
```

The nested content root follows decision 8. Its two repo-wide consequences are handled by dedicated tasks in Step 2 and rows in the change table: `cafleet/tests/docs_sync.rs` reads six pages by repo-root-relative `docs/...` paths and must move to `docs/docs/...`, and every repo-internal reference to a moved page path (relative links across `skills/cafleet/SKILL.md` and `skills/cafleet/reference/*.md`, plus `CONTRIBUTING.md`, `.claude/rules/documentation-maintenance.md`, `.claude/skills/update-readme/SKILL.md`, `.claude/skills/skill-author/SKILL.md`, and `.claude/skills/clean-docs/reference/review-format.md`) is updated via the path-reference sweep.

### docs/package.json

```json
{
  "name": "cafleet-docs",
  "private": true,
  "type": "module",
  "scripts": {
    "dev": "rspress dev",
    "build": "rspress build",
    "preview": "rspress preview"
  },
  "devDependencies": {
    "@rspress/core": "^2"
  }
}
```

`@rspress/core` is the rspress v2 package (provides the `rspress` binary; `defineConfig` is imported from it). It is the docs package's only dependency.

### docs/rspress.config.ts

```ts
import path from 'node:path';
import { defineConfig } from '@rspress/core';

export default defineConfig({
  root: 'docs',
  base: '/cafleet/',
  globalStyles: path.join(__dirname, 'theme/styles.css'),
  title: 'CAFleet',
  description: 'Message broker and member registry for coding agents.',
  llms: true,
  markdown: {
    checkDeadLinks: true,
  },
  themeConfig: {
    footer: {
      message: 'Released under the MIT License. © 2026 himkt.',
    },
    socialLinks: [
      { icon: 'github', mode: 'link', content: 'https://github.com/himkt/cafleet' },
    ],
  },
});
```

The config file is TypeScript and type-checked against `@rspress/core` at build time, so a drifted field name fails loudly; the implementer adjusts exact `themeConfig` field shapes to the installed v2 types if they differ. Search and the light/dark toggle are rspress theme defaults and need no configuration. `llms: true` (execute-time amendment, user decision) makes the static build additionally emit `llms.txt`, `llms-full.txt`, and a per-page `.md` beside every `.html`, so each page's markdown source is reachable by swapping the URL's `.html`/route for `.md` on any static host. `themeConfig` defines neither `nav` nor `sidebar` — rspress v2 auto-generates both only when the config omits them, taking the navbar from `_nav.json` and the sidebar from the `_meta.json` files (v2 contract, guide/basic/auto-nav-sidebar).

### Navbar (`_nav.json`) and sidebar (`_meta.json`)

The navbar comes from `docs/docs/_nav.json` (the v2 rename of the v1 navbar-level `_meta.json`):

```json
[
  { "text": "Docs", "link": "/quickstart" }
]
```

Sidebar order and labels reproduce the zensical `nav` exactly. `index.md` (`pageType: home`) stays out of the sidebar. File entries pin the zensical label explicitly so a page's h1 cannot drift the nav. The root `_meta.json` is the single global sidebar.

`docs/docs/_meta.json`:

```json
[
  { "type": "file", "name": "quickstart", "label": "Quickstart" },
  { "type": "dir", "name": "how-to", "label": "How-to guides" },
  { "type": "dir", "name": "concepts", "label": "Concepts" },
  { "type": "dir", "name": "spec", "label": "Specification" },
  { "type": "file", "name": "contributing", "label": "Contributing" }
]
```

`docs/docs/how-to/_meta.json`:

```json
[
  { "type": "file", "name": "mixed-backend-team", "label": "Mixed-backend Team" },
  { "type": "file", "name": "design-doc-development", "label": "Spec Driven Dev" },
  { "type": "file", "name": "use-the-webui", "label": "Admin WebUI" }
]
```

`docs/docs/concepts/_meta.json`:

```json
[
  { "type": "file", "name": "overview", "label": "Overview" },
  { "type": "file", "name": "fleet-isolation", "label": "Fleet isolation" },
  { "type": "file", "name": "storage", "label": "Storage" },
  { "type": "file", "name": "member-lifecycle", "label": "Member lifecycle" },
  { "type": "file", "name": "coding-agents", "label": "Coding agents" },
  { "type": "file", "name": "model-selection", "label": "Model selection" },
  { "type": "file", "name": "monitoring", "label": "Monitoring" }
]
```

`docs/docs/spec/_meta.json`:

```json
[
  { "type": "file", "name": "data-model", "label": "Data model" },
  { "type": "file", "name": "message-envelope", "label": "Message envelope" },
  { "type": "file", "name": "cli-options", "label": "CLI options" },
  { "type": "file", "name": "multiplexer-backends", "label": "Multiplexer backends" },
  { "type": "file", "name": "coding-agent-backends", "label": "Agent backends" },
  { "type": "file", "name": "webui-api", "label": "WebUI API" }
]
```

### Page conversion rules

Pages keep the `.md` extension. Conversions:

| Zensical construct | Occurrences | rspress replacement |
|---|---|---|
| `icon: lucide/...` frontmatter | all 19 pages | Delete the line; delete the whole frontmatter block where it becomes empty |
| `??? example "Expand the walkthrough"` | `quickstart.md:122`, `how-to/mixed-backend-team.md:57` | `:::details Expand the walkthrough` container: dedent the 4-space-indented body, close with `:::` |
| Fenced ` ```mermaid ` block | 5 (inventory below) | Deleted (execute-time amendment); the surrounding prose stands alone |
| `:material-arrow-right:` shortcode + `{ .md-button ... }` attr_list | `index.md:25` only | Removed by the hero home conversion (the Quickstart button becomes a hero action) |
| Directory-style link `quickstart/` | `index.md:25` only | Route link `/quickstart` in the hero action |

Relative `.md` links between pages (the dominant link form) are supported by rspress natively and stay as-is. `markdown.checkDeadLinks` validates route targets at build time; it is not relied on for `#fragment` targets — those are covered by Step 7's dedicated anchor-verification task (extract every internal fragment link from the built HTML and confirm each target id exists).

### Home page (`index.md`)

`pageType: home` with a two-column hero modeled on rspress.rs (user decision, final): at wide viewports the left column carries the gradient `CAFleet` name, the "Agent Teams reinvented" heading, the tagline "Collaborative coding across multiple coding-agent backends, with full code transparency.", and the `Quickstart` → `/quickstart` (brand) / `GitHub` → `https://github.com/himkt/cafleet` (alt) action buttons — the GitHub button's frontmatter text carries an inline GitHub-mark SVG (rspress renders action text as HTML; sized 1.1em with a 0.5em gap, and the llms markdown branch strips tags from action text); the right column is the YouTube demo `<iframe>` (`aria-label` `CAFleet demo video` — an accessible name without the native hover tooltip a `title` attribute produces), large and playable, filling roughly the right half. The columns stack — text above video — on narrow viewports. No sidebar renders on the home page (inherent to the home layout); every other page keeps the global sidebar. Desktop is the primary design target (user: most visitors arrive on PC) — sizing decisions optimize wide viewports first, with the narrow-viewport stack as a graceful fallback only.

The descriptive paragraph is dropped: the tagline is the page's pitch, and `.claude/skills/update-readme/SKILL.md` aligns the README pitch with the tagline rather than a body paragraph. Implementation freedom: realize the right-side visual with the least custom surface — the hero frontmatter plus the existing custom theme (`docs/theme/index.tsx`) placing the iframe in the hero's visual slot, or an equivalent minimal mechanism; the styling lives in `docs/theme/styles.css` (wired via the config's `globalStyles`), replacing the previous centered-hero and `cafleet-home-body` rules. The hero scales fluidly with the screen: the container is `min(90vw, 1680px)` wide with the video column at ~58%, vertically centered within a height band of `min(100vh − navbar, 860px)` so content sits in the upper region of tall screens; the video renders as a straight depth card (16px radius, layered shadow with a brand-purple glow; no tilt, no hover effect); the display type is fluid — title/subtitle `clamp(3rem, 2.6vw + 1.5rem, 5.5rem)` at weight 800 with tight leading, tagline `clamp(1.05rem, 0.45vw + 0.9rem, 1.35rem)` muted at 34ch and weight 500, action buttons at weight 600. The home footer message renders at 1.05rem/500 in `--rp-c-text-2`. Docs-page body text (paragraphs, list items, table cells) renders at 17px at the theme's default weight, with headings untouched and inline code pinned to its pre-raise 14px/400. Site-wide, the theme's `antialiased` font smoothing is overridden to `auto` (native subpixel rendering; the antialiased default renders text too thin on macOS/retina), and the navbar height is raised to 72px via the theme's `--rp-nav-height` variable (the hero's height-band calc follows it). The demo embed src carries `?rel=0` (player controls shown; pause-overlay suggestions restricted to the same channel — YouTube allows no full removal; the pre-play title overlay is YouTube-mandated).

### Mermaid diagram removal

The five fenced mermaid diagrams are deleted outright (execute-time amendment of decision 4). Each deletion site keeps its surrounding prose, which describes the same behavior in words; no image, source file, or tooling replaces the block.

| Diagram removal site (pre-move path) |
|---|
| `concepts/overview.md:31` |
| `concepts/member-lifecycle.md:25` |
| `concepts/monitoring.md:179` |
| `spec/data-model.md:17` |
| `spec/multiplexer-backends.md:216` |

### MDX-hazard inventory and decision rule

rspress compiles pages with the MDX toolchain, and the official docs do not state whether `.md` files get a plain-markdown parse (where these constructs are inert) or a full-MDX parse (where they break the build or get swallowed as expressions). The hazard inventory:

| Hazard | Where |
|---|---|
| `&#124;` entity inside `<code>` table cells | `spec/coding-agent-backends.md:163`, `spec/message-envelope.md:24,29,32`, `spec/cli-options.md:799` (5 total) |
| Literal `{member_id}`-style braces outside code spans | The four `spec/webui-api.md` headings at 159, 184, 209, 251 |
| Raw HTML (`<code>`, `<iframe>`) | spec tables; `index.md` |

Brace occurrences elsewhere (`spec/cli-options.md:578-581,815` and the `spec/webui-api.md` table cells at 33-36, 217-218) sit inside backtick code spans, which the MDX parser leaves untouched; they are not hazards and are excluded from the inventory.

Decision rule (applied in Step 7): build the site and inspect the rendered HTML at each inventory location. If everything renders literally, no page edit is needed. If a location breaks the build or renders wrong, remediate **minimally and locally**: wrap the affected literal in a code span (for heading paths like `GET /api/members/{member_id}/monitor`, code-span the path portion and re-verify every inbound anchor to that heading), or backslash-escape the character where a code span would distort the content. No blanket rewrite of the spec pages.

### Toolchain and configuration changes

| File | Change |
|---|---|
| `pyproject.toml` | Delete |
| `uv.lock` | Delete |
| `zensical.toml` | Delete |
| `.bumpversion.toml` | Delete |
| `mise.toml` | Remove the `"aqua:astral-sh/uv"` tool pin and `[tasks.uv-sync]`; retarget `[tasks."docs-build"]` to `run = "pnpm --dir docs build"` with `depends = ["pnpm-install"]` (a fresh clone works with no manual prerequisite, matching the cargo-task convention) and description "Build the rspress documentation site"; extend the `pnpm-install` description to mention the docs site |
| `pnpm-workspace.yaml` | Add `docs` to `packages` |
| `.claude/settings.json` | Remove the allow entries `Bash(mise //:uv-sync)`, `Bash(uv run --frozen --group research *)`, `Bash(uv run python -m *)`; keep `Bash(mise //:docs-build)`. Execute-time amendment (user-approved): add `Bash(pnpm exec agent-browser --session vr-batch-* click *)` and `... fill *` allow entries so the Verifier can exercise search and the dark toggle end-to-end, and narrow `pnpm exec agent-browser * --help` / `* --version` to the bare `--help` / `--version` forms |
| `presets/opencode/cafleet.md` | Remove the `"mise //:uv-sync": "allow"` row |
| `SPEC.md` | Update the embedded opencode preset block to match the preset file (smallest edit removing the drift) |
| `.claude/rules/commands.md` | Remove the `uv-sync` bullet and the cafleet-research visualization-runner row from the Skill artifact runners table |
| `docs/contributing.md` | Rewrite § *Building docs locally* for the pnpm/rspress flow (`mise //:docs-build` wraps `pnpm --dir docs build`; local preview via `pnpm --dir docs dev`); update the project-structure table's `docs/` row to describe the rspress project layout |
| `.gitignore` | Remove the `/site/` entry and its zensical comment; remove the dead Python entries `/.venv/`, `__pycache__/`, `*.pyc`; add `/docs/doc_build/` and `/docs/node_modules/` (the root-anchored `/node_modules/` does not cover nested directories — `admin/node_modules/` already has its own entry) |
| `.claude/skills/update-readme/SKILL.md` | Retarget the "Read the `zensical.toml` nav" step to the rspress sidebar source (the `docs/docs/**/_meta.json` files); its `docs/` page paths are updated by the path-reference sweep |
| `cafleet/tests/docs_sync.rs` | Update the repo-root-relative page paths (`docs/concepts/...`, `docs/spec/...`) to `docs/docs/...` |
| Repo-internal `docs/` path references | Update every reference to a moved page path to its `docs/docs/...` location (relative links from `skills/` gain one segment): `skills/cafleet/SKILL.md`, `skills/cafleet/reference/*.md`, `CONTRIBUTING.md`, `.claude/rules/documentation-maintenance.md`, `.claude/skills/skill-author/SKILL.md`, `.claude/skills/clean-docs/reference/review-format.md`, plus any further hits from the Step 2 sweep |
| `.github/workflows/docs.yml` | Build job steps become: checkout → mise-action → `mise //:docs-build` (its `pnpm-install` dependency installs first) → upload-pages-artifact with `path: docs/doc_build`. Deploy job and both triggers unchanged |

`README.md` is unaffected (its thin surface — pitch, install commands, docs-site links — does not change). The cafleet-research skill files are unaffected (the visualization reference is invocation-agnostic and points at the host project's rules, which this change edits).

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

### Step 1: Documentation-first updates

- [x] Rewrite `docs/contributing.md` § *Building docs locally* and the project-structure `docs/` row for the pnpm/rspress flow <!-- completed: 2026-07-31T07:23 -->
- [x] Update `.claude/rules/commands.md`: remove the `uv-sync` bullet and the visualization-runner row <!-- completed: 2026-07-31T07:26 -->
- [x] Update `presets/opencode/cafleet.md` (drop the `uv-sync` allow) and mirror the change in the `SPEC.md` embedded preset block <!-- completed: 2026-07-31T07:23 -->
- [x] Retarget `.claude/skills/update-readme/SKILL.md` from the `zensical.toml` nav to the `_meta.json` sidebar source per the change table <!-- completed: 2026-07-31T07:26 -->
- [x] Sweep remaining live references (`rg -n 'zensical|uv-sync|bump-my-version|uv run|uv\.lock' --glob '!design-docs/**'` plus lockfiles excluded) and update each hit that describes this repo's toolchain — generic tool-name examples are exempt per Success Criteria; hits deleted by Step 5 are skipped <!-- completed: 2026-07-31T07:26 -->

### Step 2: Scaffold the rspress workspace

- [x] Add `docs` to `pnpm-workspace.yaml` <!-- completed: 2026-07-31T07:30 -->
- [x] Create `docs/package.json` per the specification <!-- completed: 2026-07-31T07:30 -->
- [x] Create `docs/rspress.config.ts` per the specification <!-- completed: 2026-07-31T07:30 -->
- [x] Move the 19 pages into `docs/docs/` preserving the `how-to/`, `concepts/`, `spec/` subdirectories <!-- completed: 2026-07-31T07:40 -->
- [x] Create the four `_meta.json` files per the specification <!-- completed: 2026-07-31T07:30 -->
- [x] Apply the `.gitignore` edits per the change table, then run `mise //:pnpm-install` <!-- completed: 2026-07-31T07:40 -->
- [x] Update `cafleet/tests/docs_sync.rs` page paths to `docs/docs/...` and confirm `mise //cafleet:test` passes <!-- completed: 2026-07-31T07:40 -->
- [x] Sweep repo-internal references to moved page paths (`rg -n 'docs/(concepts|spec|how-to|quickstart|index|contributing)' --glob '!docs/**' --glob '!design-docs/**'`) and update each per the change table <!-- completed: 2026-07-31T07:40 -->

### Step 3: Page conversions

- [x] Delete the `icon:` frontmatter from all 19 pages (drop empty frontmatter blocks) <!-- completed: 2026-07-31T07:46 -->
- [x] Convert `index.md` to the hero home page per the specification <!-- completed: 2026-07-31T07:46 -->
- [x] Convert the two `??? example` collapsibles to `:::details` containers (dedent bodies) <!-- completed: 2026-07-31T07:46 -->
- [x] Sweep internal links: fix the directory-style `quickstart/` link; leave relative `.md` links for `checkDeadLinks` to validate <!-- completed: 2026-07-31T07:46 -->

### Step 4: Mermaid diagram removal

- [x] Remove the five diagram references from the pages so each removal site reads as continuous prose <!-- completed: 2026-07-31T07:55 -->
- [x] Delete the `docs/diagrams/` mermaid sources; confirm no mermaid content remains under `docs/` <!-- completed: 2026-07-31T07:55 -->

### Step 5: Python toolchain removal

- [x] Delete `pyproject.toml`, `uv.lock`, `zensical.toml`, `.bumpversion.toml` <!-- completed: 2026-07-31T07:57 -->
- [x] Edit `mise.toml`: drop the uv pin and `uv-sync`; retarget `docs-build`; update the `pnpm-install` description <!-- completed: 2026-07-31T07:57 -->
- [x] Edit `.claude/settings.json`: remove the three uv-related allow entries <!-- completed: 2026-07-31T07:57 -->

### Step 6: CI

- [x] Update `.github/workflows/docs.yml` per the specification (single `mise //:docs-build` build step, upload path `docs/doc_build`) <!-- completed: 2026-07-31T07:58 -->

### Step 7: Verification

- [x] `mise //:docs-build` completes cleanly with `checkDeadLinks` enabled <!-- completed: 2026-07-31T08:30 -->
- [x] Apply the MDX-hazard decision rule: inspect the rendered HTML at every inventory location; remediate minimally where broken and re-verify inbound anchors <!-- completed: 2026-07-31T08:30 -->
- [x] Verify fragment anchors: extract every internal `#fragment` link from the built HTML and confirm each target id exists <!-- completed: 2026-07-31T08:30 -->
- [x] Visual pass over the built site (`pnpm --dir docs preview`): hero home with demo embed, sidebar parity, the two details containers, the former diagram sites reading as continuous prose, search and dark toggle working <!-- completed: 2026-07-31T08:30 -->
- [x] Final reference sweep is clean: no live reference to this repo's `zensical` / `uv` / `bump-my-version` toolchain outside `design-docs/` and git history (generic tool-name examples exempt per Success Criteria) <!-- completed: 2026-07-31T08:30 -->

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-07-31 | Initial draft |
| 2026-07-31 | Reviewer round 1: recorded the nested-layout decision and its consequences (`docs_sync.rs`, path-reference sweep), sweep exemption policy, corrected MDX-hazard inventory, concrete `.gitignore` edits, `docs-build` task dependency, anchor-verification task |
| 2026-07-31 | Execute-time amendment (user decision): dropped the `@mermaid-js/mermaid-cli` devDependency and the `diagrams` script; SVG regeneration is a one-off `pnpm dlx --allow-build=puppeteer` invocation, keeping puppeteer/Chrome out of the workspace lockfile |
| 2026-07-31 | Execute-time amendment 2 (user decision): the five mermaid diagrams are removed from the docs entirely — no `.mmd` sources, no SVGs, no mermaid tooling; the surrounding prose carries the content. Step 4 rewritten as a removal step |
| 2026-07-31 | Verifier round: rspress v2 nav contract adopted (`_nav.json` navbar + root `_meta.json` global sidebar, `themeConfig.nav` dropped); execute-time amendment 3 (user decision): `docs/theme/index.tsx` HomeLayout override mounts the home markdown body via `afterHero` |
| 2026-07-31 | Reviewer round: recorded the user-approved `.claude/settings.json` agent-browser permission amendments (click/fill for verification sessions, help/version narrowing) in the change table |
| 2026-07-31 | User-review revision round: hero tightened via `theme/styles.css` (`globalStyles`); execute-time amendment 4 (user decision): `llms: true` emits `llms.txt` / `llms-full.txt` / per-page `.md` in the static build |
| 2026-07-31 | User-review rounds 2–3: demo embed capped at 760px; home body wrapped in a `cafleet-home-body` container (centered, doc-like typography) |
| 2026-07-31 | User-review final home round (decision 6 amended): rspress.rs-style two-column hero — title/tagline/buttons left, the demo video as the large right-side visual, stacking on narrow viewports; description paragraph dropped (tagline is the pitch; `update-readme` aligns the README to it) |
