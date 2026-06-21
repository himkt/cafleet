# Collapse the design-doc and research skill constellations into two umbrella skills

**Status**: Approved
**Progress**: 0/22 tasks complete
**Last Updated**: 2026-06-21

## Overview

Collapse seven CAFleet skills into two umbrella skills using the per-workflow-folder pattern: the three design-doc workflow skills (`cafleet-design-doc-create` / `-execute` / `-interview`) fold into the existing `cafleet-design-doc` umbrella, and the four research/media skills (`cafleet-research-report` / `-presentation` / `cafleet-my-slidev` / `cafleet-create-figure`) merge into a new `cafleet-research` umbrella. Each umbrella's `SKILL.md` becomes a dispatcher whose description carries every sub-trigger keyword; orchestration workflows live in per-workflow folders (`<workflow>/<workflow>.md` + `<workflow>/roles/`), and the format spec and the two media utilities become load-on-demand `reference/` pages. This is a docs/skills-only change — **no source code** — that directly extends the precedent set by design 0000103.

## Success Criteria

- [ ] `grep -rn "cafleet-design-doc-create\|cafleet-design-doc-execute\|cafleet-design-doc-interview\|cafleet-research-report\|cafleet-research-presentation\|cafleet-my-slidev\|cafleet-create-figure" skills/ docs/ README.md .claude/ .claude-plugin/` returns nothing (the `design-docs/` historical record is excluded by the inclusion list; `site/` and repo-root `prompts/` are excluded as generated/historical artifacts).
- [ ] The surviving umbrella slugs `cafleet-design-doc` and `cafleet-research` are present and registered; `cafleet` is untouched.
- [ ] The seven skill directories `skills/cafleet-design-doc-create/`, `-execute/`, `-interview/`, `skills/cafleet-research-report/`, `-presentation/`, `skills/cafleet-my-slidev/`, `skills/cafleet-create-figure/` are deleted (staged via `git rm -r`).
- [ ] `.claude-plugin/plugin.json`'s `"skills"` array no longer registers any of the seven deleted dirs and DOES register `./skills/cafleet-research` (array shrinks 9→3: `cafleet`, `cafleet-design-doc`, `cafleet-research`).
- [ ] `docs/get-started/configure.md`'s allowlist lists exactly three `Skill(cafleet:…)` entries — `cafleet`, `cafleet-design-doc`, `cafleet-research` — with no residue comment.
- [ ] Both umbrella `SKILL.md` files carry the union of their sub-workflows' `allowed-tools` and a description carrying ALL sub-trigger keywords (create / implement / validate design doc; research report; presentation / slides; chart / plot / graph / visualize).
- [ ] The five workflow bodies (`create/create.md`, `execute/execute.md`, `interview/interview.md`, `report/report.md`, `presentation/presentation.md`) and the two utility reference pages (`reference/slidev.md`, `reference/visualization.md`) carry NO frontmatter; only the two umbrella `SKILL.md` files do.
- [ ] The Slidev theme assets (`reference/slidev/theme/`) are preserved byte-for-byte, and the headmatter `theme:` install-path string resolves to `skills/cafleet-research/reference/slidev/theme`.
- [ ] Every relative link in every moved/created file resolves (depth rewrites applied; see R-PATHS); no `../cafleet-design-doc/…` link and no dangling `cafleet-my-slidev` / `cafleet-create-figure` skill-load remains.
- [ ] The execute Director's three functional role-file paths resolve to `skills/cafleet-design-doc/execute/roles/{programmer,tester,verifier}.md` (member spawn unbroken; R10).
- [ ] The two 0000103-era files (`skills/cafleet/reference/base-dir.md`, `skills/cafleet/reference/director.md`) reference the new umbrella slugs and every 0000103-repointed `reference/*` pointer is preserved (depth-adjusted only where it is a true relative link).
- [ ] `mise //cafleet:lint` passes (no source code touched; lint confirms nothing regressed).

---

## Background

This design extends 0000103 (collapse-skill-constellation-into-cafleet, on this same branch), which collapsed the four-skill `cafleet` CORE cluster into one `/cafleet` umbrella using a `reference/` + `roles/` pattern. It reuses that doc's vocabulary, its two-independent-homes rule, its zero-residue grep discipline, its relative-path depth-rewrite table, and its numbered blast-radius style.

### Core vocabulary (inherited from 0000103)

- **`roles/X.md`** — a first-person spawn-prompt anchor for a cafleet member spawned with `--role X` ("You are the Drafter…"). Consumed by one spawned agent.
- **`reference/Y.md`** — a third-person load-on-demand knowledge page (format spec, technique guide, utility catalog).
- **`create` / `execute` / `interview` / `report` / `presentation` are NOT roles.** Each is a **workflow** that itself spawns a team of roles. A workflow body is an orchestration procedure, and each workflow's member roles need a collision-free home — three different `director.md` files would collide in a single flat `roles/`. The per-workflow-folder layout gives each workflow its own `roles/` namespace.

### The seven skills being merged

**Group A — design-doc family (→ existing `cafleet-design-doc` umbrella):**

| Skill | Role today | Lines | Structure |
|---|---|---|---|
| `cafleet-design-doc` | FORMAT spec (umbrella core) | SKILL 18 + template 49 + guidelines 55 + coordination 146 | becomes the umbrella core/dispatcher |
| `cafleet-design-doc-create` | Create workflow | SKILL 259 | + `roles/` director, drafter, reviewer |
| `cafleet-design-doc-execute` | Execute/implement workflow | SKILL 514 | + `roles/` director, programmer, tester, verifier |
| `cafleet-design-doc-interview` | Validate workflow | SKILL 249 | + `roles/analyzer.md` only — its Director is **inlined** in SKILL.md (no `roles/director.md`) |

**Group B — research/media family (→ NEW `cafleet-research` umbrella):**

| Skill | Role today | Lines | Structure |
|---|---|---|---|
| `cafleet-research-report` | Report orchestration | SKILL 278 + template 73 | + `roles/` director, manager, researcher, scout, web-researcher |
| `cafleet-research-presentation` | Presentation orchestration | SKILL 299 | + `roles/` director, presentation, transcript, visual-reviewer |
| `cafleet-my-slidev` | Slidev theme utility | SKILL 135 | + `techniques/` (3 files) + `theme/` (Vue components + layouts + CSS assets); no team/roles |
| `cafleet-create-figure` | matplotlib utility | SKILL 236 | no team/roles |

### Why collapse

The proliferation costs 0000103 identified apply here too: each satellite skill is a separate permission surface and a separate description-trigger surface, and the design-doc trio fragments one document-lifecycle concern across three skills that share one format spec. Two media utilities (`my-slidev`, `create-figure`) are consumed almost entirely *by name* from the presentation workflow's role startup blocks, not by independent auto-trigger. Folding them into umbrellas removes the fragmentation while the dispatcher descriptions preserve auto-discovery.

### Two settled inputs (fixed; not re-litigated)

1. All four research/media skills go under one `cafleet-research` umbrella (accepting that two general-purpose media utilities are filed under a "research" umbrella; mitigated by putting chart/plot/graph AND slide/presentation trigger keywords into the `cafleet-research` description).
2. Per-workflow folder layout (`<workflow>/<workflow>.md` + `<workflow>/roles/`).

### Discoverability tradeoff (the accepted regression)

Collapsing seven distinct per-phase / per-utility auto-trigger descriptions into two umbrella descriptions means an ad-hoc request that previously matched (e.g.) `cafleet-create-figure`'s description now relies on the `cafleet-research` umbrella description carrying the `chart / plot / graph / visualize` keywords. The mitigation is the same as 0000103's R1/R2: each umbrella description is written to carry ALL sub-trigger keywords, and each `SKILL.md` body is a clear dispatcher that routes to the right workflow body or reference page. The accepted residual regression: a user who depended on the *exact* phrasing of a deleted description may need slightly more general phrasing to hit the umbrella. This is stated explicitly as accepted (see R1).

---

## Specification

### Target layout — Group A (`skills/cafleet-design-doc/`)

```
cafleet-design-doc/
  SKILL.md                      (dispatcher; frontmatter: union allowed-tools + multi-trigger description)
  reference/template.md         (moved from cafleet-design-doc/template.md)
  reference/guidelines.md       (moved from cafleet-design-doc/guidelines.md)
  reference/coordination.md     (moved from cafleet-design-doc/coordination.md)
  create/create.md              (was cafleet-design-doc-create/SKILL.md body; frontmatter dropped)
  create/roles/{director,drafter,reviewer}.md
  execute/execute.md            (was cafleet-design-doc-execute/SKILL.md body; frontmatter dropped)
  execute/roles/{director,programmer,tester,verifier}.md
  interview/interview.md        (was cafleet-design-doc-interview/SKILL.md body; frontmatter dropped; Director stays inline)
  interview/roles/analyzer.md
```

### Target layout — Group B (`skills/cafleet-research/`)

```
cafleet-research/
  SKILL.md                      (dispatcher; frontmatter: union allowed-tools + multi-trigger description)
  reference/visualization.md    (was cafleet-create-figure/SKILL.md body; frontmatter dropped)
  reference/slidev.md           (was cafleet-my-slidev/SKILL.md body; frontmatter dropped)
  reference/slidev/techniques/{formatting,math-formulas,two-column-layouts}.md
  reference/slidev/theme/        (Vue components + layouts + CSS assets, preserved byte-for-byte)
  report/report.md              (was cafleet-research-report/SKILL.md body; frontmatter dropped)
  report/template.md            (moved from cafleet-research-report/template.md)
  report/roles/{director,manager,researcher,scout,web-researcher}.md
  presentation/presentation.md  (was cafleet-research-presentation/SKILL.md body; frontmatter dropped)
  presentation/roles/{director,presentation,transcript,visual-reviewer}.md
```

The rule: **utilities → `reference/` pages; orchestration workflows → per-workflow folders with `roles/`.** The slidev utility groups its body (`reference/slidev.md`), its techniques, and its theme assets under a single `reference/slidev/` subtree so the three move as one unit.

### Dispatcher `SKILL.md` frontmatter

Both umbrellas carry exactly one `SKILL.md` with frontmatter. All other moved bodies (the five workflow bodies, the two utility reference pages, the format-spec pages) carry **no** frontmatter — they become load-on-demand procedure/knowledge files the dispatcher routes to.

**`cafleet-design-doc/SKILL.md`** — the existing format skill is rewritten into a dispatcher:

- `allowed-tools`: the **union** of the three workflows' grants. `create` has `Read, Write, Edit, Glob, Grep, Bash, WebSearch, WebFetch`; `execute` adds `Task` (TDD orchestration) and `interview` adds `AskUserQuestion`. Union: `Read, Write, Edit, Glob, Grep, Bash, Task, WebSearch, WebFetch, AskUserQuestion`. The slightly broader grant for a consumer that only wants the format spec is an accepted cost (R3).
- `description`: carries the format-consult trigger PLUS create/implement/validate triggers. Draft wording:
  > Design document format spec plus CAFleet-native orchestration to create, validate, and implement design docs. Consult the template/guidelines when editing an existing design doc; or create a new design doc / specification / technical spec (Director/Drafter/Reviewer), validate/interview an existing one through multi-round Q&A, or implement/execute one with a TDD team. Do NOT write or implement design docs freeform — route through this skill's workflows.
- body: a dispatcher index — a "when to use" table routing to `reference/template.md`+`guidelines.md`+`coordination.md` (format spec) and to `create/create.md` / `interview/interview.md` / `execute/execute.md` (the three workflows). Retains the coding-agent overlay note (`../cafleet/reference/coding-agent/<name>.md`, unchanged depth) and the "teammates load this skill by name `cafleet-design-doc`" instruction.

**`cafleet-research/SKILL.md`** — NEW dispatcher:

- `allowed-tools`: the union across report + presentation + the two utilities. Report/presentation orchestration need `Read, Write, Edit, Glob, Grep, Bash, Task, WebSearch, WebFetch`; the utilities add nothing beyond `Read, Write, Edit, Bash`. Union: `Read, Write, Edit, Glob, Grep, Bash, Task, WebSearch, WebFetch`. (Confirm each source skill's frontmatter at authoring time and take the literal union.)
- `description`: carries research-report + presentation/slides + chart/figure/visualize triggers so a "make a bar chart" or "build a slide deck" request still auto-fires. Draft wording:
  > Comprehensive multi-source research reports, Slidev presentations, charts, and data visualizations. Use to create a research report (multi-agent Director/Manager/Researcher team writing to researches/<topic>/), build a Slidev presentation / slide deck / reading transcript from a report, create a chart / plot / graph / figure or visualize data with matplotlib, or author slides with the custom Slidev theme. Do NOT do a quick web search and summarize — route through this skill's workflows.
- body: a dispatcher index routing to `report/report.md` (research) → which chains to `presentation/presentation.md` (slides/transcript), and to `reference/visualization.md` (matplotlib) + `reference/slidev.md` (Slidev theme) for the standalone utility paths. Retains the coding-agent overlay note (`../cafleet/reference/coding-agent/<name>.md`).

### Format-spec relocation (Group A)

The three format files move from `cafleet-design-doc/{template,guidelines,coordination}.md` (depth 2) to `cafleet-design-doc/reference/{…}.md` (depth 3). Every true relative link into them is rewritten for the new target path AND the linking file's new depth (see R-PATHS). The dispatcher `SKILL.md` links them as `reference/template.md` etc.

### Slidev theme path (functional — must resolve at render)

`cafleet-my-slidev/SKILL.md:14` hard-codes the Slidev theme install path in headmatter:

```
theme: <cafleet-plugin-install-dir>/skills/cafleet-my-slidev/theme
```

After the move the assets live at `skills/cafleet-research/reference/slidev/theme/`. The headmatter string is rewritten to:

```
theme: <cafleet-plugin-install-dir>/skills/cafleet-research/reference/slidev/theme
```

The Vue components (`Admonition.vue`, `Highlight.vue`), the eight layout `.vue` files, and `styles/index.css` move **byte-for-byte** (git mv) so Slidev still resolves the theme when a deck copies/points at it at render time. The discovery-hint comment block (`SKILL.md:16-19`) is preserved with the path updated.

### Relative-path depth rewrites (R-PATHS)

Two link classes need rewriting; one class is preserved verbatim.

**(1) Format-spec links (Group A) — rewrite target path + depth.** Old links point at `../cafleet-design-doc/<file>.md` (from a depth-2 SKILL) or `../../cafleet-design-doc/<file>.md` (from a depth-3 role file). New target is `reference/<file>.md` inside the same umbrella:

| Linking file (new location) | Old link | New link |
|---|---|---|
| `create/create.md`, `execute/execute.md`, `interview/interview.md` (depth 3 body) | `../cafleet-design-doc/X.md` | `../reference/X.md` |
| `create/roles/*.md`, `execute/roles/*.md` (depth 4 role) | `../../cafleet-design-doc/X.md` | `../../reference/X.md` |
| `SKILL.md` (dispatcher, depth 2) | `template.md` (sibling) | `reference/template.md` |

**(2) Coding-agent overlay link — rewrite depth only.** The overlay link target (`…/cafleet/reference/coding-agent/<name>.md`) is a true relative link whose depth changes for the moved bodies/roles:

| Linking file (new location) | Overlay link |
|---|---|
| `SKILL.md` (dispatcher, depth 2, both umbrellas) | `../cafleet/reference/coding-agent/<name>.md` (UNCHANGED) |
| `<workflow>/<workflow>.md` (depth 3) | `../../cafleet/reference/coding-agent/<name>.md` |
| `<workflow>/roles/*.md` (depth 4) | `../../../cafleet/reference/coding-agent/<name>.md` |
| `reference/slidev.md`, `reference/visualization.md` (depth 3) | `../../cafleet/reference/coding-agent/<name>.md` |

**(3) 0000103 cafleet-relative shorthand pointers — PRESERVE verbatim (no rewrite).** Pointers like "Load the `cafleet` skill and Read its `reference/supervision.md`" and "Read `skills/cafleet/reference/base-dir.md`" are prose shorthand resolved *after loading the cafleet skill* / *repo-relative*, NOT `../`-style markdown links. They do not change when a role/body file moves. **They must be carried through the move byte-for-byte** — this is the preserve-not-regress requirement (R-SEQ).

**Slidev-internal links (Group B):** from `reference/slidev.md`, the techniques links (`techniques/<file>.md`) become `slidev/techniques/<file>.md`, and the create-figure palette reference becomes `visualization.md` (sibling under `reference/`).

Grep-assert every relative link in every moved/created file resolves before completing (R-PATHS verification).

### Cross-reference blast radius

Every site below is verified live against the working tree (the 0000103 end-state baseline; see R-SEQ) via an exhaustive `grep -rn` for each of the seven deleted slugs across the inclusion scope. Grouped by category; line numbers are current pre-move locations. **43 sites enumerated.**

**Reconciliation with the zero-residue grep (this table is a ceiling, not a floor).** The seven deleted slugs produce **92 line-occurrences** across `skills/ docs/ README.md .claude/ .claude-plugin/` today (a line naming K slugs counts K times — e.g. `guidelines.md:32` names all three design-doc slugs, `base-dir.md:28` likewise). They resolve as:

- **10 frontmatter `name:`/`description:` lines VANISH** when the workflow bodies / utility pages lose their frontmatter (Q4; Steps 1 & 4) — no in-place edit: `create/execute/my-slidev/create-figure` `SKILL.md:2`, `interview/research-report/research-presentation` `SKILL.md:2-3`.
- **The 7 `plugin.json` + 7 `configure.md` entries** collapse into the two single-block edits #25 / #33.
- **The surviving umbrella `cafleet-design-doc/SKILL.md:3` description** (names `cafleet-design-doc-create` ×2) is rewritten wholesale in Step 2 — not a blast-radius row.
- **Every remaining body-prose / link / functional-path line** maps to a numbered row below.

After Steps 1-11 the zero-residue grep returns **0** matches in scope.

**B.0 — Functional role-file paths (Director spawn resolution; repo-relative path → rewrite, NOT prose):**

| # | Site | Edit |
|---|---|---|
| 37 | `execute/execute.md` (was execute SKILL) L176/L177/L178 — the "Read role files" spawn block | `skills/cafleet-design-doc-execute/roles/{programmer,tester,verifier}.md` → `skills/cafleet-design-doc/execute/roles/{programmer,tester,verifier}.md`. **Functional**: the execute Director Reads these to spawn its team; a stale path breaks spawn. (Verified the other four SKILLs use install-agnostic absolute-path-by-reference (`<abs path to this skill>/roles/…`), which moves with the file — no rewrite needed.) |

**B.1 — Group A intra-umbrella format-spec links (true relative links; rewrite per R-PATHS):**

| # | Site | Edit |
|---|---|---|
| 1 | `create/create.md` (was create SKILL) L21/L22/L23/L27 | `../cafleet-design-doc/{template,guidelines,coordination,coordination}.md` → `../reference/…` |
| 2 | `execute/execute.md` (was execute SKILL) L22/L23/L24/L28 | → `../reference/…` |
| 3 | `interview/interview.md` (was interview SKILL) L20/L21/L26 | → `../reference/…` |
| 4 | `create/roles/director.md` L30/L43/L79 | `../../cafleet-design-doc/coordination.md` → `../../reference/coordination.md` |
| 5 | `create/roles/drafter.md` L27/L56/L62 | → `../../reference/coordination.md` |
| 6 | `create/roles/reviewer.md` L28/L32 | → `../../reference/coordination.md` |
| 7 | `execute/roles/director.md` L41/L79/L83/L113 | → `../../reference/coordination.md` |
| 8 | `execute/roles/programmer.md` L27/L90/L102 | → `../../reference/coordination.md` |
| 9 | `execute/roles/tester.md` L27/L41/L55/L65 | → `../../reference/coordination.md` |
| 10 | `execute/roles/verifier.md` L27/L58/L62/L69 | → `../../reference/coordination.md` |

(`interview/roles/analyzer.md` carries no format-spec link — verified.)

**B.2 — Group A intra-umbrella slug→workflow phrasing (now internal cross-workflow refs; rewrite slug names to "the <X> workflow"):**

| # | Site | Edit |
|---|---|---|
| 11 | `create/create.md` L31, L69 | "the `cafleet-design-doc-execute` skill" / "`-create` skill" → "the execute / create workflow" |
| 12 | `execute/execute.md` L32, L33 | "`cafleet-design-doc-create` skill" → "create workflow"; "`cafleet-design-doc-interview` skill" → "interview workflow" |
| 13 | `interview/interview.md` L22, L28, L225, L226, L227 | "`cafleet-design-doc-create` skill" / resume-mode / "`-execute` skill" → create / execute workflow phrasing (interview→create resume-mode handoff is now internal) |
| 14 | `reference/coordination.md` L5, L22, L48, L90, L91, L138, L146 | create/execute/interview skill names → workflow phrasing |
| 15 | `reference/guidelines.md` L32 | "The `cafleet-design-doc-create`, `-interview`, `-execute` skills" → "the cafleet-design-doc skill's create/interview/execute workflows"; preserve the `cafleet` `reference/base-dir.md` prose pointer (0000103) |
| 38 | `execute/execute.md` L54 | self-invocation prose "Once `/cafleet-design-doc-execute` is invoked" → "Once the execute workflow is invoked" |
| 39 | `execute/execute.md` L228 | Verifier Phase-1 exemption prose "the Analyzer's question list in the `cafleet-design-doc-interview` skill" → "…in the interview workflow" (distinct from #12's L32/L33) |
| 40 | `execute/roles/verifier.md` L27 | the **prose** slug "Analyzer's question list in the `cafleet-design-doc-interview` skill" → "…in the interview workflow" (separate from #10, which rewrites only this file's coordination LINK) |

**B.3 — Group B intra-umbrella references (slug-load → reference page / internal workflow):**

| # | Site | Edit |
|---|---|---|
| 16 | `reference/slidev.md` (was my-slidev SKILL) L14 | theme install path → `…/skills/cafleet-research/reference/slidev/theme` (functional) |
| 17 | `reference/slidev.md` L85, L121-123 | `techniques/<file>.md` links → `slidev/techniques/<file>.md` |
| 18 | `reference/slidev.md` L91 | "must match the `cafleet-create-figure` skill's palette" → "must match `visualization.md`'s palette" |
| 19 | `presentation/presentation.md` (was research-presentation SKILL) L16, L24, L33, L134 | "the `cafleet-my-slidev` skill" / "`cafleet-create-figure` skill" → "the `reference/slidev.md` page" / "`reference/visualization.md`" (Read `../reference/…`). (L3 is frontmatter `description:` — dropped per Q4, not rewritten.) |
| 20 | `presentation/presentation.md` L76 | "Invoke the `cafleet-research-report` skill first" → "run the report workflow first (`../report/report.md`)" |
| 21 | `presentation/roles/presentation.md` L9, L10, L25, L29, L33 | "Load the `cafleet-my-slidev` skill" / "`cafleet-create-figure` skill" → "Read `../../reference/slidev.md`" / "`../../reference/visualization.md`" |
| 22 | `presentation/roles/director.md` L91, L93 | "re-runs the `cafleet-research-report` skill" → "re-runs the report workflow" |
| 23 | `report/report.md` (was research-report SKILL) L9, L253 | "chain into the `cafleet-research-presentation` skill" → "chain into the presentation workflow (`../presentation/presentation.md`)". (L3 is frontmatter `description:` — dropped per Q4, not rewritten.) |
| 41 | `presentation/presentation.md` L65 | usage-error string "invoke the `cafleet-research-presentation` skill with `<folder-name>`" → "run the presentation workflow with `<folder-name>`" |
| 42 | `presentation/presentation.md` L203 | "one-time operation per `cafleet-research-presentation` skill invocation" → "per presentation-workflow run" |
| 43 | `reference/visualization.md` (was create-figure SKILL) L24 | calling-context example "the `cafleet-research-presentation` skill passes its research folder as the figure base" → "the presentation workflow passes its research folder as the figure base" |

**B.4 — cross-umbrella reference (Group B file → Group A umbrella):**

| # | Site | Edit |
|---|---|---|
| 24 | `report/roles/web-researcher.md` L7 (description prose) | "Best used in combination with the `cafleet-design-doc-create` skill" → "…with the `cafleet-design-doc` skill (create workflow)" (the two umbrellas never cross-LINK; this is prose only) |

**B.5 — `docs/` (external):**

| # | Site | Edit |
|---|---|---|
| 25 | `docs/get-started/configure.md` L23/L25/L26/L27/L28/L29/L30 | DELETE the 7 deleted-slug `Skill(cafleet:…)` lines; ADD `Skill(cafleet:cafleet-research)`; keep `Skill(cafleet:cafleet)` (L22) + `Skill(cafleet:cafleet-design-doc)` (L24); no residue comment |
| 26 | `docs/how-to/design-doc-development.md` L17, L24, L31 | "Triggers the `cafleet-design-doc-create/-interview/-execute` skill" → "Triggers the `cafleet-design-doc` skill's create/interview/execute workflow" |
| 27 | `docs/get-started/contributing.md` L55, L58, L60, L62 | the three "Invoke the `cafleet-design-doc-*` skill" steps + the resume-mode mention → `cafleet-design-doc` workflow phrasing |
| 28 | `docs/get-started/quickstart.md` L32 | "Invoke the `cafleet-design-doc-create` skill" → "Invoke the `cafleet-design-doc` skill (create workflow)" |

(`docs/spec/cli-options.md`: verified no Group-A/B deleted-slug refs — the base-dir mirror at :986 is 0000103's R5 concern, out of scope here. Other `docs/concepts/` and `docs/how-to/` pages: verified no deleted-slug refs.)

**B.6 — `README.md`:**

| # | Site | Edit |
|---|---|---|
| 29 | `README.md` L71 | "Invoke the `cafleet-design-doc-create` skill" → "Invoke the `cafleet-design-doc` skill (create workflow)" |

**B.7 — `.claude/`:**

| # | Site | Edit |
|---|---|---|
| 30 | `.claude/rules/commands.md` L50, L51, L52 | the skill-artifact-runner rows: "`cafleet-create-figure` matplotlib scripts" → "`cafleet-research` visualization (matplotlib) scripts"; "`cafleet-research-presentation` bun deps / Slidev dev server" → "`cafleet-research` presentation …" (artifact names only; runner commands unchanged) |
| 31 | `.claude/skills/skill-author/SKILL.md` L100, L302 | "the `cafleet-design-doc-execute` skill's Copilot-review loop" → "the `cafleet-design-doc` skill's execute workflow"; the `claude`-role taxonomy line naming `-interview` / `-execute` → workflow phrasing (the marker-role list itself is unchanged) |
| 32 | `.claude/rules/coding-agent-overlay.md` | VERIFY the "every cafleet-family SKILL.md, every `roles/*.md`, every `skills/cafleet/reference/*.md`" enumeration still describes the new per-workflow `roles/` + workflow-body layout, and the two-independent-homes rule still holds; light wording touch only if the enumeration now under-describes the umbrellas (no deleted-slug text present today) |

**B.8 — Plugin manifest:**

| # | Site | Edit |
|---|---|---|
| 33 | `.claude-plugin/plugin.json` L13/L15/L16/L17/L18/L19/L20 | DELETE the 7 deleted-dir entries; ADD `"./skills/cafleet-research"`; keep `"./skills/cafleet"` + `"./skills/cafleet-design-doc"` (array 9→3) |

**B.9 — 0000103-era files (preserve-not-regress; slug-text update only — see R-SEQ):**

| # | Site | Edit |
|---|---|---|
| 34 | `skills/cafleet/reference/base-dir.md` L28 | "The `cafleet-design-doc-create` / `-execute` / `-interview` skills" → "The `cafleet-design-doc` skill (create/execute/interview workflows)"; the bucket-resolution behavior text is unchanged |
| 35 | `skills/cafleet/reference/base-dir.md` L29 | "The `cafleet-research-report` / `-presentation` skills" → "The `cafleet-research` skill (report/presentation workflows)"; behavior text unchanged |
| 36 | `skills/cafleet/reference/director.md` L144 | "`cafleet-design-doc` (design-doc family); `cafleet-my-slidev` + `cafleet-create-figure` (Presentation Specialist)" → "`cafleet-design-doc` (design-doc family); `cafleet-research` (Presentation Specialist — its `reference/slidev.md` + `reference/visualization.md`)" |

### Section-to-destination mapping (nothing lost)

| From | What | To |
|---|---|---|
| `cafleet-design-doc/{template,guidelines,coordination}.md` | format spec | `cafleet-design-doc/reference/{…}.md` |
| `cafleet-design-doc/SKILL.md` | format-skill body | rewritten in place as the dispatcher |
| `cafleet-design-doc-create/SKILL.md` (frontmatter) | name/description/allowed-tools | DROP (description triggers fold into the umbrella description; allowed-tools fold into the union) |
| `cafleet-design-doc-create/SKILL.md` (body) | create workflow | `cafleet-design-doc/create/create.md` |
| `cafleet-design-doc-create/roles/*` | create roles | `cafleet-design-doc/create/roles/*` |
| `cafleet-design-doc-execute/SKILL.md` body + `roles/*` | execute workflow + roles | `cafleet-design-doc/execute/execute.md` + `execute/roles/*` |
| `cafleet-design-doc-interview/SKILL.md` body (Director inline) + `roles/analyzer.md` | interview workflow + analyzer | `cafleet-design-doc/interview/interview.md` (Director stays inline) + `interview/roles/analyzer.md` |
| `cafleet-create-figure/SKILL.md` body | matplotlib utility | `cafleet-research/reference/visualization.md` (frontmatter dropped) |
| `cafleet-my-slidev/SKILL.md` body | Slidev theme utility | `cafleet-research/reference/slidev.md` (frontmatter dropped) |
| `cafleet-my-slidev/techniques/*` | technique guides | `cafleet-research/reference/slidev/techniques/*` |
| `cafleet-my-slidev/theme/*` | Vue + CSS assets | `cafleet-research/reference/slidev/theme/*` (byte-for-byte) |
| `cafleet-research-report/SKILL.md` body + `template.md` + `roles/*` | report workflow | `cafleet-research/report/report.md` + `report/template.md` + `report/roles/*` |
| `cafleet-research-presentation/SKILL.md` body + `roles/*` | presentation workflow | `cafleet-research/presentation/presentation.md` + `presentation/roles/*` |
| `.claude-plugin/plugin.json` | 7 deleted-dir entries | REMOVED; `cafleet-research` ADDED (9→3) |

### Settled decisions

| Ref | Decision | Resolution |
|---|---|---|
| Q1 (sequencing) | Depend on 0000103 fully landing first | 0000104 is written against the 0000103 end-state baseline; preserve every 0000103 `reference/*` pointer (see R-SEQ) |
| Q2 (naming) | Slugs | Group A reuses `cafleet-design-doc`; Group B new slug `cafleet-research` (both verbatim) |
| Q3 (interview Director) | Inline vs extract | KEEP inline in `interview/interview.md`; `interview/roles/` holds only `analyzer.md`; invent no new content |
| Q4 (frontmatter) | Workflow-body/utility frontmatter | DROP `name`/`description`/`allowed-tools` from all 5 workflow bodies + the 2 utility reference pages; only the 2 umbrella `SKILL.md` files keep frontmatter, each with the union allowed-tools + multi-trigger description |
| Q5 (slidev theme) | Theme asset destination | `reference/slidev/theme/`; group `slidev.md` + `techniques` + `theme` under one `reference/slidev/` subtree; rewrite the headmatter install-path string; move assets byte-for-byte |

### Risk register

| ID | Severity | Risk | Mitigation |
|---|---|---|---|
| R-SEQ | High | 0000103 is on this same branch and NOT fully landed (its Step 16 `git rm` + Steps 17-18 remain). Starting 0000104 before it lands risks regressing 0000103's repointed `cafleet/reference/*` pointers and double-editing `plugin.json`. | **Land 0000103 fully first.** Author 0000104 against the 0000103 end-state baseline. Preserve every 0000103 cafleet-relative shorthand pointer verbatim (they are not `../` links, so the move requires no depth change); only sites #34-36 get slug-text updates. Add a `director.md`/`base-dir.md`-specific grep to verification confirming the 0000103 pointers survive. |
| R1 | High | Discoverability regression — seven distinct auto-trigger descriptions collapse into two | Both umbrella descriptions carry ALL sub-trigger keywords (create/implement/validate; report; presentation/slides; chart/plot/graph/visualize); each `SKILL.md` body is an explicit dispatcher. Accepted residual: exact-phrasing dependence on a deleted description may need more general phrasing. |
| R2 | Medium | A utility filed under a "research" umbrella ("make a bar chart" feels unrelated to research) | The `cafleet-research` description leads with research but explicitly enumerates chart/plot/graph/figure/visualize and slide-deck triggers; the dispatcher body routes standalone-utility requests straight to `reference/visualization.md` / `reference/slidev.md`. |
| R3 | Medium | `allowed-tools` union over-grants for a consumer that only wants one workflow / the format spec | Take the literal union per umbrella; document the broader grant as an accepted cost; the surviving frontmatter lives only on the two dispatcher `SKILL.md` files. |
| R4 | High | Slidev theme path is a FUNCTIONAL render-time path; a wrong rewrite silently breaks decks | Rewrite the headmatter install-path string to `skills/cafleet-research/reference/slidev/theme`; move the Vue+CSS assets byte-for-byte (git mv); verify the path resolves and the discovery-hint comment is updated. |
| R5 | Medium | Relative-path depth 2→3/4 moves break links | Apply R-PATHS; grep-assert every relative link in every moved/created file resolves; distinguish true `../` links (rewrite) from 0000103 shorthand (preserve). |
| R6 | Medium | interview→create resume-mode handoff (COMMENT(claude) markers) becomes an internal cross-workflow contract | Preserve the contract; rephrase the slug references to "create workflow" without changing the marker mechanics; the two workflows now live under one umbrella so the handoff is intra-umbrella. |
| R7 | Medium | report→presentation chain becomes an internal cross-workflow chain | Rephrase "invoke the `cafleet-research-presentation` skill" to "the presentation workflow (`../presentation/presentation.md`)"; preserve the approval-gated chaining behavior. |
| R8 | Low | `web-researcher.md` cross-umbrella reference to the create workflow | Prose-only repoint to "the `cafleet-design-doc` skill (create workflow)"; the two umbrellas never cross-link (two-independent-homes). |
| R9 | Low | Global `~/.claude/skills` mirror orphans (stale physical copies of the 7 deleted dirs auto-trigger) | Post-merge action: remove/re-sync the seven global mirror dirs; record in the Changelog (mirrors 0000103 R8). |
| R10 | High | Functional break — `execute/SKILL.md:176-178` are repo-relative role-file paths the execute Director Reads to spawn its team; a stale path silently breaks member spawn (not just grep residue) | Rewrite the three paths to `skills/cafleet-design-doc/execute/roles/{programmer,tester,verifier}.md` (#37); verify the execute Director resolves and Reads each. The other four SKILLs use install-agnostic absolute-path-by-reference, which moves with the file — no rewrite, but assert this in verification. |

### Alternatives & accepted trade-offs

| Option | Decision | Rationale |
|---|---|---|
| Per-workflow folders (`<workflow>/<workflow>.md` + `roles/`) | **CHOSEN (settled)** | Gives each workflow a collision-free `roles/` namespace (three `director.md` files cannot coexist in a flat `roles/`); mirrors the role/reference split 0000103 proved. |
| Flat `roles/` for the whole umbrella | Rejected | Three `director.md` collisions; no clean workflow boundary. |
| All four research/media skills under one `cafleet-research` umbrella | **CHOSEN (settled)** | One research-family permission/discovery surface; the two utilities are consumed by-name from the presentation workflow anyway; mitigated by multi-keyword description. |
| Separate `cafleet-media` umbrella for the two utilities | Rejected (settled out) | A third umbrella re-introduces proliferation; the chosen mitigation (multi-keyword description) preserves discovery. |
| Utilities as `reference/` pages | **CHOSEN** | Utilities have no team/roles; a load-on-demand knowledge page is the right home; keeps the dispatcher body lean. |
| Extract interview's Director to `interview/roles/director.md` | Rejected (Q3 settled) | The Director is inlined today; extraction would invent content; the per-workflow folder already prevents collision. |

**Two-independent-homes rule (inherited).** The agent-facing skill content (`skills/cafleet-research/`, `skills/cafleet-design-doc/`) and the human-facing operator docs (`docs/`) stay independent — they never cross-link. Restating the same operational fact in both is fine; linking between them is not.

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-06-21T14:30 -->`
> Documentation/skills ARE the deliverable (no source code). Steps are ordered create-new-structure → repoint-consumers → delete-old → verify, so every new link resolves at edit time. **Prerequisite: 0000103 is fully landed (R-SEQ).**

### Step 1: Build the `cafleet-design-doc` umbrella structure

- [ ] `git mv` the format spec to `reference/`: `template.md`, `guidelines.md`, `coordination.md` → `cafleet-design-doc/reference/` <!-- completed: -->
- [ ] `git mv` the three sub-skills into per-workflow folders: `cafleet-design-doc-create/SKILL.md` → `cafleet-design-doc/create/create.md` (+ `roles/`); `-execute/SKILL.md` → `execute/execute.md` (+ `roles/`); `-interview/SKILL.md` → `interview/interview.md` (+ `roles/analyzer.md`) <!-- completed: -->
- [ ] Drop `name`/`description`/`allowed-tools` frontmatter from `create/create.md`, `execute/execute.md`, `interview/interview.md` (Director stays inline in `interview.md`) <!-- completed: -->

### Step 2: Rewrite `cafleet-design-doc/SKILL.md` as the dispatcher

- [ ] Rewrite the description to carry create + implement/execute + validate/interview + format-consult triggers; set `allowed-tools` to the three-workflow union; build the dispatcher when-to-use index routing to `reference/{template,guidelines,coordination}.md` and `create/`/`interview/`/`execute/`; keep the overlay note + the "load by name `cafleet-design-doc`" instruction <!-- completed: -->

### Step 3: Repoint Group A intra-umbrella links, slug phrasing, and functional paths (#1-15, #37-40)

- [ ] Rewrite all format-spec links per R-PATHS (#1-10): bodies `../cafleet-design-doc/X.md` → `../reference/X.md`; role files `../../cafleet-design-doc/X.md` → `../../reference/X.md`; depth-adjust every overlay link in the moved bodies/roles <!-- completed: -->
- [ ] Rewrite the functional role-file paths in `execute/execute.md` L176-178 → `skills/cafleet-design-doc/execute/roles/{programmer,tester,verifier}.md` (#37, R10) <!-- completed: -->
- [ ] Rewrite slug→workflow phrasing in the moved bodies + format-spec files (#11-15, #38-40), incl. the execute self-invocation prose, the Analyzer-precedent prose in `execute.md` + `execute/roles/verifier.md`; preserve the 0000103 base-dir prose pointer in `reference/guidelines.md` <!-- completed: -->

### Step 4: Build the `cafleet-research` umbrella structure

- [ ] `git mv` `cafleet-research-report/{SKILL.md→report/report.md, template.md→report/template.md, roles→report/roles}`; `cafleet-research-presentation/{SKILL.md→presentation/presentation.md, roles→presentation/roles}` <!-- completed: -->
- [ ] `git mv` `cafleet-create-figure/SKILL.md` → `cafleet-research/reference/visualization.md`; `cafleet-my-slidev/SKILL.md` → `reference/slidev.md`; `cafleet-my-slidev/techniques/` → `reference/slidev/techniques/`; `cafleet-my-slidev/theme/` → `reference/slidev/theme/` (byte-for-byte) <!-- completed: -->
- [ ] Drop frontmatter from `report/report.md`, `presentation/presentation.md`, `reference/slidev.md`, `reference/visualization.md` <!-- completed: -->

### Step 5: Create `cafleet-research/SKILL.md` dispatcher

- [ ] New `SKILL.md`: description carries research-report + presentation/slides + chart/plot/graph/figure/visualize triggers; `allowed-tools` = union of report/presentation/utility grants; dispatcher body routes to `report/`→`presentation/` and to `reference/visualization.md` + `reference/slidev.md`; include the overlay note <!-- completed: -->

### Step 6: Repoint Group B intra-umbrella references (#16-24, #41-43)

- [ ] Rewrite the slidev theme headmatter path (#16), the slidev techniques links (#17), the create-figure palette reference (#18); the presentation slug-loads → `reference/` Read pointers + report-workflow pointer (#19-23); the presentation usage/invocation strings (#41-42); the `reference/visualization.md` calling-context example (#43); the cross-umbrella `web-researcher.md` prose pointer (#24); depth-adjust every overlay link in the moved Group B bodies/roles <!-- completed: -->

### Step 7: Repoint external docs (#25-29)

- [ ] `configure.md` allowlist (#25: delete 7, add `cafleet-research`, keep `cafleet` + `cafleet-design-doc`); `design-doc-development.md` (#26); `contributing.md` (#27); `quickstart.md` (#28); `README.md` (#29) — agent-facing and operator-facing homes stay independent (no cross-link) <!-- completed: -->

### Step 8: Repoint `.claude/` (#30-32)

- [ ] `commands.md` artifact-runner rows (#30); `skill-author/SKILL.md` prose (#31); verify `coding-agent-overlay.md` enumeration still describes the new layout (#32) <!-- completed: -->

### Step 9: Update 0000103-era files (preserve-not-regress, #34-36)

- [ ] Slug-text updates only to `cafleet/reference/base-dir.md` L28/L29 and `cafleet/reference/director.md` L144; confirm every other 0000103 `reference/*` pointer in these files is untouched (R-SEQ grep) <!-- completed: -->

### Step 10: Prune and register the plugin manifest (#33)

- [ ] Remove the 7 deleted-dir entries from `.claude-plugin/plugin.json`'s `"skills"` array; add `"./skills/cafleet-research"` (array 9→3); no residue comment <!-- completed: -->

### Step 11: Delete the seven old skill directories

- [ ] `git rm -r skills/cafleet-design-doc-create skills/cafleet-design-doc-execute skills/cafleet-design-doc-interview skills/cafleet-research-report skills/cafleet-research-presentation skills/cafleet-my-slidev skills/cafleet-create-figure` (LAST, after all repointing AND the manifest prune) <!-- completed: -->

### Step 12: Verification (zero-residue proof)

- [ ] Run the zero-residue grep for all seven deleted slugs over `skills/ docs/ README.md .claude/ .claude-plugin/`, returning **0** (down from the 92 pre-move line-occurrences; `design-docs/`, `site/`, repo-root `prompts/` excluded as historical/generated). Confirm the seven dirs are gone (staged deletions) and `plugin.json` + `configure.md` list exactly `cafleet`, `cafleet-design-doc`, `cafleet-research` <!-- completed: -->
- [ ] Confirm every relative link in every moved/created file resolves (R-PATHS): no `../cafleet-design-doc/…` link, no dangling `cafleet-my-slidev`/`cafleet-create-figure` skill-load, correct overlay depths; **the execute Director's three functional role-file paths (#37) resolve to `skills/cafleet-design-doc/execute/roles/…` and the other four workflows' absolute-path-by-reference still resolves (R10)**; the slidev theme path resolves and assets are byte-for-byte; the two utility reference pages + five workflow bodies carry no frontmatter; both dispatcher `SKILL.md` files carry the union allowed-tools + multi-trigger description; the R-SEQ grep confirms 0000103 pointers preserved <!-- completed: -->
- [ ] `mise //cafleet:lint` passes (no source touched; lint confirms nothing regressed) <!-- completed: -->
- [ ] Post-merge deployment action (R9): remove/re-sync the seven global mirror dirs `~/.claude/skills/{cafleet-design-doc-create,-execute,-interview,cafleet-research-report,-presentation,cafleet-my-slidev,cafleet-create-figure}` so stale copies stop auto-triggering; record in the Changelog <!-- completed: -->

### Step 13: Finalize

- [ ] Set Status → Complete, update Progress, verify all tasks checked with timestamps <!-- completed: -->

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-06-21 | Initial draft |
| 2026-06-21 | Reviewer round 1: ran an exhaustive per-slug `grep -rn` (92 line-occurrences) and reconciled the blast radius against it; added the functional role-file-path group B.0 (#37, `execute/SKILL.md:176-178`) + risk R10 (functional spawn break); enumerated 7 previously-missed body-prose sites (#37-43: execute self-invocation L54, Analyzer-precedent prose in `execute.md` L228 + `verifier.md` L27, presentation usage/invocation strings L65/L203, `visualization.md` calling-context example L24); dropped the frontmatter `description:` L3 lines from rows #19/#23 (they vanish with the Q4 frontmatter drop); corrected the false "`visualization.md` references no Group B slug" claim; restated the table as 43 enumerated sites that grep-prove a 92→0 reduction; added Success-Criteria + Step-3/6/12 coverage; task count 0/22. |
| 2026-06-21 | Finalized: user approved via the create-skill review loop (2 rounds — round 1 closed the 5 blast-radius gaps above; round 2 confirmed clean). Blast radius grep-verified to 43 enumerated sites / 92→0 residue. Status → Approved. |
