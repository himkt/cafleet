# README — Spotlight design-doc-driven development; collapse the duplicate Coding agents H3

**Status**: Approved
**Progress**: 9/17 tasks complete
**Last Updated**: 2026-05-16

## Overview

Simplify `README.md` with two coordinated changes: (1) collapse the duplicate `### Coding agents` H3 inside `## CLI cheatsheet` — its Codex sandbox `> [!IMPORTANT]` callout moves into the `#### Codex` install subsection (with a one-line cross-reference to `docs/codex-members.md` appended there) and its flag-context paragraph is dropped from the README; `## Install` gains a one-paragraph coding-agent intro stating which backends are supported. This is a collapse-and-relocate, not a literal merge of two sections — only the "two backends supported" sentence is genuinely de-duplicated, the flag-context paragraph is dropped per user clarification B3. (2) Add a new `## Design-doc-driven development` H2 that surfaces the three CAFleet design-doc skills (`/cafleet:design-doc-create`, `/cafleet:design-doc-interview`, `/cafleet:design-doc-execute`) and pitches the dogfooding angle — design documents under `design-docs/` are authored, validated, and implemented through these skills.

## Success Criteria

- [ ] `README.md` `## Install` section begins with a one-paragraph coding-agent intro stating that CAFleet works with `claude` (Claude Code) and `codex` (OpenAI Codex CLI); the existing `### Install CAFleet skills` (with `#### Claude Code` and `#### Codex`) and `### Install CAFleet CLI` subsections follow.
- [ ] The Codex sandbox `> [!IMPORTANT]` callout (`sandbox_workspace_write.writable_roots = ["/home/<you>/.local/share/cafleet"]`) lives in the `#### Codex` subsection of `### Install CAFleet skills`, adjacent to the rest of the Codex plugin install steps. The verbatim TOML block is preserved.
- [ ] The `#### Codex` install subsection ends with a one-line cross-reference: `For codex CLI version pin and operational specifics, see [docs/codex-members.md](docs/codex-members.md).` This preserves the only inbound link to `docs/codex-members.md` from the README after the `### Coding agents` removal — placed where an operator setting up Codex actually needs it.
- [ ] The current `### Coding agents` H3 subsection (under `## CLI cheatsheet`, README L122–L134 in the pre-change file) is removed entirely. Its flag-context paragraph (the `--coding-agent {claude,codex}` description and the "a single Director may mix claude+codex members" sentence) is NOT replicated elsewhere in `README.md` per user clarification B3 — operators discover that detail via `docs/codex-members.md` (now linked from the `#### Codex` install subsection) and `cafleet --help`.
- [ ] The existing `## Simple example to use CAFleet on Claude Code or Codex` section (README L76–L92 in the pre-change file) stays verbatim. No retitling, no pointer additions, no content edits.
- [ ] A new `## Design-doc-driven development` top-level H2 sits immediately after the `## Simple example …` section and immediately before `## CLI cheatsheet`. The new section contains exactly three components in this order: (i) a 2–3 sentence dogfooding pitch paragraph, (ii) a bullet list naming all three design-doc skills with one-line role descriptions, (iii) a single line linking to the in-repo `design-docs/` directory as evidence.
- [ ] The new section does NOT link to individual `skills/*/SKILL.md` files, does NOT call out specific design-doc exemplars by number, and does NOT use a `> [!NOTE]` / `> [!TIP]` callout block. Pitch paragraph + bullet list + single directory link only.
- [ ] No edits to any file other than `README.md`. `ARCHITECTURE.md`, `docs/`, `skills/*/SKILL.md`, source code, and tests are out of scope.
- [ ] `mise //cafleet:lint`, `mise //cafleet:format`, `mise //cafleet:typecheck`, and `mise //cafleet:test` all pass after the change (sanity check — none should be impacted by a README-only diff, but run them to confirm).

---

## Background

The current `README.md` (178 lines) duplicates coding-agent setup detail across two surfaces:

1. `## Install` → `### Install CAFleet skills` → `#### Claude Code` / `#### Codex` documents the per-backend plugin install commands.
2. `## CLI cheatsheet` → `### Coding agents` re-explains which backends are supported, documents the `--coding-agent` flag, and parks the Codex sandbox `> [!IMPORTANT]` callout there.

A reader walking the README top-to-bottom encounters "CAFleet supports claude and codex" twice, with the second restatement adding flag-level CLI detail that does not belong on a landing page. The CLI flag detail is already canonical in `docs/codex-members.md` and via `cafleet --help`; the Codex sandbox callout belongs adjacent to the Codex plugin install steps (where the operator is actively configuring Codex), not parked in the cheatsheet.

Separately, the README never surfaces what is arguably CAFleet's most compelling developer-facing pitch: CAFleet's design-doc workflow is itself driven by the three CAFleet design-doc skills. Design documents under `design-docs/` (62 directories at the time of writing) are authored, validated, and implemented through that loop, and the repo is a self-evident dogfooding artifact — but the README does not say so. The existing `## Simple example to use CAFleet on Claude Code or Codex` shows a `/cafleet:design-doc-create` invocation, but it reads as a generic "try this" demo rather than a deliberate developer-facing pitch. (Pre-skill design documents under low design-doc numbers were authored manually; the present-tense phrasing here matches §5's verbatim README content and avoids the unverifiable historical absolute.)

This change reframes that pitch as a first-class section while collapsing the duplicate coding-agent prose into a single landing-page paragraph at the top of `## Install`.

---

## Specification

### 1. Target section spine of `README.md` after the change

```
# CAFleet                                                # H1 + tagline + local-only callout (UNCHANGED)

## Install                                               # CHANGED — intro paragraph added
  - one-paragraph coding-agent intro (NEW)
  - ### Install CAFleet skills                           # UNCHANGED structure
    - #### Claude Code                                   # UNCHANGED
    - #### Codex                                         # CHANGED — sandbox IMPORTANT callout appended
  - ### Install CAFleet CLI (required for CAFleet to function)  # UNCHANGED

## Simple example to use CAFleet on Claude Code or Codex # UNCHANGED — kept verbatim per user instruction

## Design-doc-driven development                         # NEW H2

## CLI cheatsheet                                        # CHANGED — Coding agents H3 removed
  - ### Notable flags                                    # UNCHANGED
  - ### Message body truncation                          # UNCHANGED
  - (### Coding agents removed)

## Architecture                                          # UNCHANGED
## Project structure                                     # UNCHANGED
## Development                                           # UNCHANGED
## License                                               # UNCHANGED
```

Net structural delta: one new H2 (`## Design-doc-driven development`), one removed H3 (`### Coding agents`), one new intro paragraph in `## Install`, one relocated callout (Codex sandbox `> [!IMPORTANT]`). Every other section, sub-section, table, and code block is byte-for-byte unchanged.

### 2. New coding-agent intro paragraph at the top of `## Install`

Inserted between the `## Install` heading (currently README L7) and the existing `### Install CAFleet skills` heading (currently README L9). Exact wording:

```markdown
## Install

CAFleet works with two coding agents: `claude` (Claude Code) and `codex` (OpenAI Codex CLI). Install the plugin in whichever one you use — the broker CLI is shared.

### Install CAFleet skills
```

Rules for this paragraph:

- One sentence stating which backends are supported, one sentence stating that the plugin install is per-backend while the broker CLI is shared. No more.
- Do NOT mention the `--coding-agent {claude,codex}` flag. That detail is dropped from README per the user's instruction (B3 of the clarification round).
- Do NOT mention that a single Director may mix claude and codex members. Same rationale.
- Do NOT link to `docs/codex-members.md` from this paragraph. The existing per-backend Codex subsection (now augmented with the sandbox callout) handles all the deep links.

### 3. Codex sandbox `> [!IMPORTANT]` callout relocation

The callout currently lives under `### Coding agents` (H3, README L126–L134 in the pre-change file). It is moved to the end of the `#### Codex` subsection (H4) of `### Install CAFleet skills`. The heading level changes (H3 → H4) but the callout body, TOML code fence, and prose are byte-for-byte identical. After the relocation, the callout body is:

````markdown
> [!IMPORTANT]
> Codex members need the cafleet DB directory to be writable from inside the codex sandbox. Add it to `sandbox_workspace_write.writable_roots` in any `config.toml` codex reads (e.g. `~/.codex/config.toml`):
>
> ```toml
> [sandbox_workspace_write]
> writable_roots = ["/home/<you>/.local/share/cafleet"]
> ```
>
> Use the absolute path matching `CAFLEET_DATABASE_URL` or the default XDG location.
````

Placement detail: the callout goes AFTER the existing `> [!IMPORTANT]` skill-verification callout in `#### Codex` (the one that currently ends the Codex subsection with the `~/.codex/config.toml` `[marketplaces.cafleet]` block). The result is two adjacent `> [!IMPORTANT]` blockquotes in the `#### Codex` subsection: the existing one about skill verification, then the relocated one about sandbox writable_roots. Separate them with one blank line between the closing `>` of the first and the opening `> [!IMPORTANT]` of the second.

After both callouts, append one final line — a cross-reference back to `docs/codex-members.md`:

```markdown
For codex CLI version pin and operational specifics, see [docs/codex-members.md](docs/codex-members.md).
```

This line is the only inbound link to `docs/codex-members.md` from `README.md` after the `### Coding agents` removal. It is placed at the structural site where an operator configuring Codex actually needs it (immediately after the install commands and the two operational `> [!IMPORTANT]` callouts), not parked in the cheatsheet. Separate from the second `> [!IMPORTANT]` callout above it by one blank line; separate from the next section (`### Install CAFleet CLI`) by one blank line below.

### 4. The flag-context paragraph from the removed `### Coding agents` section

The opening paragraph of the current `### Coding agents` section is:

> cafleet supports two coding-agent binaries for member panes: `claude` (Claude Code) and `codex` (OpenAI Codex CLI). Pass `--coding-agent {claude,codex}` on `cafleet session create` (operator-declared metadata for the root Director) and `cafleet member create` (selects the spawn-command builder and records the placement). The default is `claude`, so existing invocations are unchanged. A single Director may spawn both `claude` and `codex` members in the same session. Operational details for codex members — including the codex CLI version pin and verification recipe — live in [docs/codex-members.md](docs/codex-members.md).

This paragraph is **deleted from `README.md`** per user clarification B3 (flag-switching detail is dropped from the landing page; it lives in `docs/codex-members.md` and `cafleet --help`). The two facts the deleted paragraph carried are still discoverable:

| Fact dropped from README | Where it remains canonical | Inbound link from README after this change |
|---|---|---|
| `--coding-agent {claude,codex}` flag on `session create` / `member create` | `docs/spec/cli-options.md` and `cafleet <cmd> --help` | None from README; reached via `docs/spec/` directory listing or `cafleet <cmd> --help`. |
| A single Director may mix `claude` and `codex` members in one session | `docs/codex-members.md` | Via the one-line cross-reference at the end of `#### Codex` install subsection (§3). |

The new intro paragraph in `## Install` (§2 above) names the two supported backends; deeper flag-level detail is intentionally not on the landing page. The removal-rule (`~/.claude/rules/removal.md`) compliance question is separate from this paragraph drop and is covered in §7: that rule prohibits *deprecation notices* and *historical breadcrumbs* ("formerly the X section", "moved to Y"), not ordinary cross-references — and §3's one-line `docs/codex-members.md` link is an ordinary cross-reference, not a breadcrumb.

### 5. New `## Design-doc-driven development` section — exact content

Inserted between the end of `## Simple example to use CAFleet on Claude Code or Codex` (currently ending at README L92) and the start of `## CLI cheatsheet` (currently README L94). Verbatim content:

```markdown
## Design-doc-driven development

CAFleet itself is developed end-to-end using CAFleet. Design documents under `design-docs/` are authored, validated, and implemented through the three CAFleet design-doc skills — the same skills you get when you install the plugin. The same Director-and-members orchestration that the Simple example above kicks off is how the CAFleet repo grows.

- `/cafleet:design-doc-create` — Director spawns a Drafter and a Reviewer, drives the clarification → draft → review loop, lands a polished design doc on disk.
- `/cafleet:design-doc-interview` — Director spawns a short-lived Analyzer, drives multi-round `AskUserQuestion` validation, writes `COMMENT(claude)` annotations inline.
- `/cafleet:design-doc-execute` — Director spawns a Programmer and a Tester (and optionally a Verifier), drives the TDD cycle step-by-step, commits after each phase, then runs the PR + Copilot-review loop.

See the [`design-docs/`](design-docs/) directory for every design document this repo has shipped.
```

Section structure rules:

- The pitch paragraph is **exactly 3 sentences**. The first sentence states the dogfooding claim ("CAFleet itself is developed end-to-end using CAFleet."). The second is a current-state statement about how design documents under `design-docs/` are produced — present tense ("are authored, validated, and implemented"), NOT a historical absolute ("every design doc was…"). The skills `/cafleet:design-doc-create`, `/cafleet:design-doc-interview`, and `/cafleet:design-doc-execute` were introduced at specific design-doc numbers (most recently `/cafleet:design-doc-interview` at design 0000045), so pre-skill design documents were authored manually; the present-tense phrasing is evidence-anchored and avoids the unverifiable universal. The third sentence connects back to the Simple example shown above.
- The bullet list has **exactly 3 items**, one per design-doc skill, in the order `create → interview → execute` (the natural workflow order).
- Each bullet is **one line**: a slash-command name in backticks, an em-dash, a one-sentence role description that names the key members spawned and the deliverable. The role descriptions are paraphrased from the SKILL.md `description` frontmatter of each skill — verify against the current SKILL.md at draft time so the wording matches reality (no SKILL.md edits are made; the README mirrors the SKILL.md, not the other way around).
- The closing line is **exactly one sentence** with one link, pointing at the in-repo `design-docs/` directory. No named exemplar design docs. No links to individual `skills/*/SKILL.md` files.

### 6. What is NOT changed

To bound the change surface explicitly:

| Untouched | Why |
|---|---|
| `## Install` → `### Install CAFleet skills` → `#### Claude Code` subsection | The user did not ask for changes to the Claude Code install steps. Stays byte-for-byte. |
| The existing first `> [!IMPORTANT]` callout inside `#### Codex` (skill verification, with the `[marketplaces.cafleet]` and `[plugins."cafleet@cafleet"]` TOML block) | The user only asked to preserve and relocate the sandbox callout. The skill-verification callout is independent and untouched. |
| `### Install CAFleet CLI` subsection | The user did not ask for changes. Stays byte-for-byte. |
| `## Simple example to use CAFleet on Claude Code or Codex` (L76–L92) | The user explicitly said "leave verbatim." |
| `## CLI cheatsheet`, `### Notable flags`, `### Message body truncation` | Out of scope. Only the `### Coding agents` subsection is removed from this H2. |
| `## Architecture`, `## Project structure`, `## Development`, `## License` | Out of scope. |
| `ARCHITECTURE.md`, `docs/`, `skills/*/SKILL.md`, `cafleet/src/cafleet/`, tests | Out of scope per the clarification round (E2). |

### 7. Removal-rule compliance

Per `~/.claude/rules/removal.md`, when the `### Coding agents` H3 is removed, no breadcrumb, deprecation note, or "see also" pointer is left behind. Specifically, the new intro paragraph in `## Install` (§2) and the new `## Design-doc-driven development` section (§5) MUST NOT contain phrases like:

- `(formerly the "Coding agents" section)`
- `See the previous "Coding agents" section`
- `The "Coding agents" subsection has moved to docs/codex-members.md`

The historical record of the removal lives in this design doc (Status: Complete after implementation) and in the git commit message. The README after this change reads as if the `### Coding agents` H3 had never existed.

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-05-16T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

### Step 1: Edit `## Install` — add coding-agent intro paragraph, relocate Codex sandbox callout, append cross-reference

- [x] Insert the new coding-agent intro paragraph (verbatim text from §2) immediately after the `## Install` heading and before the `### Install CAFleet skills` heading. Preserve one blank line above and below the new paragraph. <!-- completed: 2026-05-16T13:57 -->
- [x] Move the Codex sandbox `> [!IMPORTANT]` callout (verbatim text from §3) from `### Coding agents` to the end of `#### Codex` inside `### Install CAFleet skills`. Place it AFTER the existing skill-verification `> [!IMPORTANT]` callout, separated by exactly one blank line. Preserve the TOML code fence and indentation byte-for-byte. <!-- completed: 2026-05-16T13:57 -->
- [x] Append the one-line cross-reference `For codex CLI version pin and operational specifics, see [docs/codex-members.md](docs/codex-members.md).` (verbatim from §3) immediately after the second `> [!IMPORTANT]` callout in `#### Codex`, separated by one blank line above and one blank line below. This is the only inbound link to `docs/codex-members.md` from `README.md` after the change. <!-- completed: 2026-05-16T13:57 -->

### Step 2: Edit `## CLI cheatsheet` — remove `### Coding agents` H3 entirely

- [x] Delete the `### Coding agents` H3 heading and its full body (the opening flag-context paragraph plus the original location of the sandbox callout). After the delete, the section ordering inside `## CLI cheatsheet` is: cheatsheet table → `### Notable flags` → `### Message body truncation` (with the `### Coding agents` slot gone). <!-- completed: 2026-05-16T14:01 -->
- [x] Confirm by re-reading the file that no breadcrumb sentence (`See ...`, `formerly ...`, `moved to ...`) was left behind in either the cheatsheet table area or in the surrounding sections. The removal must be silent — git history is the record. <!-- completed: 2026-05-16T14:01 -->

### Step 3: Insert new `## Design-doc-driven development` section

- [x] Before inserting the new section, open `skills/design-doc-create/SKILL.md`, `skills/design-doc-interview/SKILL.md`, and `skills/design-doc-execute/SKILL.md`. Read each file's frontmatter `description` and the `Role` table at the top of the body. Confirm the one-line role descriptions in §5's bullet list still match reality (Drafter+Reviewer for create, Analyzer for interview, Programmer+Tester+optional Verifier for execute). If any role table has drifted, adjust the README bullet text to match — do NOT edit the SKILL.md. <!-- completed: 2026-05-16T14:03 -->
- [x] Before finalising the pitch paragraph, verify its wording does not include an unverifiable absolute about pre-skill design docs. The §5 verbatim text uses present tense ("Design documents under `design-docs/` are authored, validated, and implemented through the three CAFleet design-doc skills") deliberately, since the three skills were introduced at specific design-doc numbers (the latest is `/cafleet:design-doc-interview` at design 0000045) and pre-skill documents were authored manually. Reject any rewording that re-introduces a historical universal ("Every design document was authored using these skills"). <!-- completed: 2026-05-16T14:03 -->
- [x] Insert the new `## Design-doc-driven development` H2 (verbatim text from §5) immediately after the closing line of `## Simple example to use CAFleet on Claude Code or Codex` and immediately before the `## CLI cheatsheet` H2. Preserve one blank line above the new heading and one blank line below the closing `[design-docs/](design-docs/)` line. <!-- completed: 2026-05-16T14:03 -->
- [x] Confirm the bullet list has exactly 3 items, the pitch paragraph is exactly 3 sentences, and the closing line is exactly one sentence with exactly one link target (`design-docs/`). <!-- completed: 2026-05-16T14:03 -->

### Step 4: Verification

- [ ] Re-read the new `## Install` section top-to-bottom and confirm: (a) the intro paragraph names both backends in one short paragraph, (b) the Claude Code subsection is unchanged, (c) the Codex subsection ends with the two adjacent `> [!IMPORTANT]` callouts (skill verification, then sandbox writable_roots) followed by the one-line `docs/codex-members.md` cross-reference, (d) the CLI install subsection is unchanged. <!-- completed: -->
- [ ] Verify the new `## Design-doc-driven development` section against the §5 structural checklist mechanically: (a) the pitch paragraph contains the dogfooding hook phrase `CAFleet itself`, (b) the pitch paragraph is exactly 3 sentences (count periods at sentence boundaries), (c) the bullet list has exactly 3 items naming `/cafleet:design-doc-create`, `/cafleet:design-doc-interview`, and `/cafleet:design-doc-execute` in that order, (d) the closing line is exactly one sentence with one link whose target resolves to `design-docs/` (relative path, no anchor). <!-- completed: -->
- [ ] Confirm `git diff README.md` shows only the five edits described above (intro paragraph add, sandbox callout relocation, cross-reference append, `### Coding agents` removal, `## Design-doc-driven development` add) and no incidental whitespace drift. <!-- completed: -->
- [ ] Confirm `git status` shows ONLY `README.md` modified — no other files touched. <!-- completed: -->
- [ ] `mise //cafleet:lint` passes. <!-- completed: -->
- [ ] `mise //cafleet:format` produces no diff. <!-- completed: -->
- [ ] `mise //cafleet:typecheck` passes. <!-- completed: -->
- [ ] `mise //cafleet:test` passes. <!-- completed: -->

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-05-16 | Initial draft |
| 2026-05-16 | Reviewer round 1: retitle to "...collapse the duplicate Coding agents H3" to match the actual change shape; correct §3 heading-level claim (H3 → H4 crossing, not "same H4"); restore one-line `docs/codex-members.md` cross-reference at end of `#### Codex` install subsection so the README still has an inbound link to that file; re-attribute the flag-paragraph-drop justification to user clarification B3 (not the removal rule, which prohibits breadcrumbs, not ordinary cross-references); soften §5 pitch from "Every design document was authored…" (unverifiable absolute — pre-skill docs were authored manually) to present-tense "Design documents under `design-docs/` are authored, validated, and implemented…"; replace subjective Step-4 "first-time reader" check with the §5 mechanical checklist; add Step-1 task for the cross-reference append and Step-3 task for pitch-wording verification; update Progress 0/14 → 0/17. |
| 2026-05-16 | Reviewer round 2: Background paragraph rewritten to match the §5 present-tense framing (drop the leftover "every design document … was authored end-to-end" universal that §5 had already disavowed); internal consistency between Background and §5 restored. |
| 2026-05-16 | Approved by user. Status set to Approved; ready for /design-doc-execute. |
