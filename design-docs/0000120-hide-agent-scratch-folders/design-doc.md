# Hide coding-agent-only scratch/audit folders behind a dot-prefix

**Status**: Approved
**Progress**: 4/16 tasks complete
**Last Updated**: 2026-07-05

## Overview

Rename the coding-agent-only scratch/audit folders documented across the cafleet skills to hidden (dot-prefixed) folders so they are not exposed to users, and codify a general, reusable convention that every future skill follows: assets created only for coding agents go in dot-prefixed hidden folders, while user-facing deliverables stay visible.

## Success Criteria

- [ ] The general hidden-vs-visible folder convention is codified in `skills/cafleet/reference/base-dir.md` as a new subsection (single home, no duplication elsewhere).
- [ ] `prompts/` is renamed to `.prompts/` in every skill file that documents it, with no aliases and no deprecation notices.
- [ ] `figures/src` becomes `.figures/code` and `figures/data` becomes `.figures/data` (both hidden), while `figures/output` stays visible as `figures/output`; the `SRC_DIR` placeholder is renamed to `CODE_DIR`.
- [ ] `screenshots/` is renamed to `.screenshots/` in every presentation-workflow file, including the `.keep` write and the per-batch review-log markdown.
- [ ] `.gitignore` is updated: `/prompts/` becomes `/.prompts/` with a refreshed comment, and hidden-scratch stray-catch entries are added.
- [ ] No `prompts/`, `figures/src`, `figures/data`, or `screenshots/` path reference remains anywhere under `skills/` or in `.gitignore` (the three content-category false positives are explicitly left untouched); `figures/output` remains present and visible.

---

## Background

Several cafleet-family skills instruct coding agents to write scratch, audit, and intermediate artifacts into folders under the task's resolved base directory (`${BASE}`):

- **`prompts/`** — the pre-spawn render of each member's spawn prompt (`${BASE}/prompts/<role>-<UTC-compact>.md`), which is both the CLI `--text-file` input and the permanent audit artifact.
- **`figures/src`, `figures/data`, `figures/output`** — matplotlib source scripts, input data, and rendered chart PNGs for the research/presentation workflows.
- **`screenshots/`** — the Visual Reviewer's captured slide PNGs and its per-batch review-log markdown.

Of these, only the rendered charts under `figures/output` are user-facing deliverables — they are embedded into slides and reports. Everything else is coding-agent-only scratch or audit output that clutters the user's task folder. Hiding the agent-only folders behind a dot-prefix keeps them out of the user's way while leaving deliverables visible.

**This is a documentation-only change to the skills.** These folders are entirely skill-instruction-driven: there is **no** cafleet Python source that creates them (verified — no `mkdir` for `prompts`/`figures`/`screenshots` exists under `cafleet/src`; the sole textual hit is unrelated prose). `docs/`, `SPEC.md`, and `README.md` contain **zero** references. All path references live under `skills/`, plus one operational surface (`.gitignore`).

The rename is applied as a **hard rename** per the project's `removal.md` rule: every mention is updated in the same change, with no deprecation notices and no aliases. The git history and this design doc are the historical record.

---

## Specification

### The general convention (codified in `base-dir.md`)

A new subsection is added to `skills/cafleet/reference/base-dir.md` — the canonical no-bypass write authority that every file-writing member already reads. It is the single home for the rule; it is **not** duplicated in `.claude/rules/coding-agent-overlay.md` (which governs backend-specific deltas, not write-location visibility) or anywhere else.

The subsection states the affirmative rule:

> **Hidden agent-only folders vs visible deliverables.** Assets a coding agent creates only for its own workflow — scratch, audit trails, and intermediate build inputs — live in **dot-prefixed hidden folders** under `${BASE}` (e.g. `${BASE}/.prompts/`, `${BASE}/.figures/code`, `${BASE}/.figures/data`, `${BASE}/.screenshots/`). Assets that are **user-facing deliverables** — the artifacts the user opens, embeds, or ships — live in **visible, unprefixed folders** (e.g. `${BASE}/figures/output/` for the rendered charts embedded into slides and reports).
>
> When a skill adds a new output folder under `${BASE}`, classify it first: coding-agent-only → dot-prefix it; user-facing deliverable → leave it visible/unprefixed.

### The rename mapping

| Before | After | Visibility | Rationale |
|--------|-------|------------|-----------|
| `prompts/` | `.prompts/` | hidden | Spawn-prompt audit renders — agent-only. |
| `figures/src` | `.figures/code` | hidden | Matplotlib source scripts — agent-only. Also renamed `src` → `code`. |
| `figures/data` | `.figures/data` | hidden | Figure input data — agent-only. |
| `figures/output` | `figures/output` | **visible (unchanged)** | Rendered charts embedded into slides/reports — user-facing deliverable. |
| `screenshots/` | `.screenshots/` | hidden | Visual Reviewer PNG captures + per-batch review-log markdown — agent-only. |

The `figures` rename produces **two sibling top-level directories** under `${BASE}`: a visible `figures/` containing only `output/`, and a new hidden `.figures/` containing `code/` and `data/`. Slide/report embeds keep their existing visible path `./figures/output/<name>.png`.

The visualization placeholder `SRC_DIR` is renamed to `CODE_DIR` (resolving to `${BASE}/.figures/code`); `OUTPUT_DIR` (`${BASE}/figures/output`) and `DATA_DIR` (`${BASE}/.figures/data`) keep their names.

### Out of scope / explicitly unchanged

- **Content-category prose (three false positives).** Three tokens that look like folder paths are actually prose and MUST NOT be touched: `skills/cafleet-research/reference/slidev.md` (the `blank for diagrams/figures/code` layout hint) and `skills/cafleet-research/presentation/roles/presentation.md` (the `blank for tables/figures/diagrams` hint) mean "figures/code" and "figures/diagrams" as *content categories*, not folder paths; and `skills/cafleet-research/presentation/presentation.md:24` (`Capture screenshots/snapshots of assigned slides`) means "screenshots or snapshots" (two capture types), not the folder path `screenshots/` — renaming it would produce the nonsense `Capture .screenshots/snapshots`.
- **The `figures/output` embed reference** in `skills/cafleet-research/presentation/roles/presentation.md` (`![...](./figures/output/filename.png)`) stays exactly as-is — `figures/output` remains visible.
- **`docs/`, `SPEC.md`, `README.md`** — no references exist; no edits.
- **cafleet Python source** — creates none of these folders; no edits.

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.
> Line numbers below are indicative (captured pre-edit) — the executor should match on the folder-path token, not the line number, since earlier edits in the same file shift later lines.

### Step 1: Codify the general convention and rename `prompts/` in `base-dir.md`

- [x] Add the new "Hidden agent-only folders vs visible deliverables" subsection (text in Specification above) to `skills/cafleet/reference/base-dir.md`, in the § *No-bypass write protocol* area. <!-- completed: 2026-07-05T02:41 -->
- [x] In `skills/cafleet/reference/base-dir.md`, rename `${BASE}/prompts/<role>-<UTC-compact>.md` → `${BASE}/.prompts/<role>-<UTC-compact>.md` at both occurrences (the Step-0 audit-file example and the no-bypass protocol item 1). <!-- completed: 2026-07-05T02:41 -->

### Step 2: Rename `prompts/` → `.prompts/` across the `cafleet` skill

- [x] `skills/cafleet/reference/director.md` — rename `prompts/` → `.prompts/` in the three `--text-file /abs/path/to/<BASE>/prompts/...` examples and in § *Member Create — Scratch and audit files* (the `<BASE>/prompts/<role>-<UTC-compact>.md` canonical-path text, the "Create `<BASE>/prompts/` on first write" instruction, and the `${BASE} == <unset>` fallback path). <!-- completed: 2026-07-05T02:43 -->
- [x] `skills/cafleet/reference/supervision.md` — rename `prompts/` → `.prompts/` in the two `${BASE}/prompts/<role>-<UTC-compact>.md` references (the Spawn-the-member step and the Spawn-member table row). <!-- completed: 2026-07-05T02:43 -->

### Step 3: Rename `prompts/` → `.prompts/` across the `cafleet-design-doc` workflows

- [ ] `skills/cafleet-design-doc/create/create.md` — rename `prompts/` → `.prompts/` at all occurrences (the task-scoped audit-path note, the two-step audit-file callout, and the Drafter/Reviewer `--text-file ${BASE}/prompts/...` render+spawn blocks), including the `<task-folder>/prompts/` prose. <!-- completed: -->
- [ ] `skills/cafleet-design-doc/execute/execute.md` — rename `prompts/` → `.prompts/` at all occurrences (the two-step audit-file callout and the Programmer/Tester/Verifier/Reviewer render+spawn blocks). <!-- completed: -->
- [ ] `skills/cafleet-design-doc/interview/interview.md` — rename `prompts/` → `.prompts/` at all occurrences (the audit-file callout and the Analyzer render+spawn block). <!-- completed: -->

### Step 4: Rename `prompts/` → `.prompts/` across the `cafleet-research` skill

- [ ] `skills/cafleet-research/report/report.md` — rename `prompts/` → `.prompts/` at all occurrences (the Step-0 resolution note, the task-scoped `<topic-folder>/prompts/` prose, the two-step audit-file callout, and the Manager/Scout/Researcher render+spawn blocks). <!-- completed: -->
- [ ] `skills/cafleet-research/presentation/presentation.md` — rename `prompts/` → `.prompts/` at all `prompts/` occurrences (the task-scoped audit-path note, the two-step audit-file callout, and the Presentation/Transcript/VR-batch render+spawn blocks). <!-- completed: -->

### Step 5: Split `figures/` in `visualization.md` (hidden `.figures/code` + `.figures/data`, visible `figures/output`)

- [ ] `skills/cafleet-research/reference/visualization.md` — apply the figures split:
  - Intro sentence and Step-0 prose: scripts and data go under `.figures/` (hidden); rendered outputs stay under `figures/output` (visible).
  - **Compound brace-list token (Step-0, ~line 17):** the single token `${BASE}/figures/{src,output,data}` straddles the visible/hidden boundary and must NOT be dot-prefixed as a whole. Split it across both destinations: `src` → hidden `.figures/code`, `data` → hidden `.figures/data`, `output` → visible `figures/output`. Rewrite the prose so it reads as, e.g., "scripts and data land under `${BASE}/.figures/{code,data}` while rendered charts land under `${BASE}/figures/output`".
  - `${SRC_DIR} = ${BASE}/figures/src` → `${CODE_DIR} = ${BASE}/.figures/code` (rename the placeholder to `CODE_DIR` and update every downstream use of `${SRC_DIR}`).
  - `${DATA_DIR} = ${BASE}/figures/data` → `${DATA_DIR} = ${BASE}/.figures/data`.
  - `${OUTPUT_DIR} = ${BASE}/figures/output` — unchanged.
  - Step-1 "Create the script" — write the `.py` into `${CODE_DIR}`.
  - The example comment path (`/tmp/claude-code/researches/foo/figures/output`) — unchanged (it is an `output` path).
  - The "Figure artifacts always live under `${BASE}/figures/`" invariant sentence — reword to reflect the visible-`figures/output` / hidden-`.figures/{code,data}` split. <!-- completed: -->
- [ ] Verify `skills/cafleet-research/presentation/roles/presentation.md` line embedding `./figures/output/filename.png` is left unchanged, and its `blank for tables/figures/diagrams` content-category hint is NOT touched. <!-- completed: -->

### Step 6: Rename `screenshots/` → `.screenshots/` across the presentation workflow

- [ ] `skills/cafleet-research/presentation/presentation.md` — rename `screenshots/` → `.screenshots/` at its two real path occurrences: the `<folder>/screenshots/.keep` persistent-directory creation step and the `<folder>/screenshots/vr<start>-r<round>.md` start-cue. Do NOT touch the Visual Reviewer role-table prose `Capture screenshots/snapshots of assigned slides` (see § Out of scope — a false positive). <!-- completed: -->
- [ ] `skills/cafleet-research/presentation/roles/director.md` — rename `screenshots/` → `.screenshots/` in the `<folder>/screenshots/vr<start>-r<round>-p<N>.png` review-read instruction. <!-- completed: -->
- [ ] `skills/cafleet-research/presentation/roles/visual-reviewer.md` — rename `screenshots/` → `.screenshots/` at its four real path occurrences: the per-slide capture-path prose (:23), the review-log persist prose (:26), the `agent-browser ... screenshot` command (:102), and the Write-tool review-log persist step (:140). The parent-directory prose (:144, "the parent directory exists") holds no `screenshots/` path token and needs no edit. <!-- completed: -->

### Step 7: Update `.gitignore`

- [ ] `.gitignore` — rename `/prompts/` → `/.prompts/` (line ~38) and refresh the surrounding comment (lines ~34–38) to name the hidden per-task folders (`researches/<slug>/.prompts/`, `design-docs/<NNNNNNN>-<slug>/.prompts/`). <!-- completed: -->
- [ ] `.gitignore` — add repo-root stray-catch entries `/.figures/` and `/.screenshots/`, mirroring the `/.prompts/` rationale (hidden coding-agent scratch dirs that can land at the repo root when `${BASE}` resolves to the repo root). Do NOT add `figures/` — its `output/` is a visible deliverable. <!-- completed: -->

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-07-05 | Initial draft |
