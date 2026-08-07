# Unified Coding-Agent Overlay File

**Status**: Approved
**Progress**: 3/17 tasks complete
**Last Updated**: 2026-08-07

## Overview

Merge the four files under `skills/cafleet/reference/coding-agent/` (`claude-overlay.md`, `codex-overlay.md`, `opencode-overlay.md`, `_template.md`) into one file, `skills/cafleet/reference/coding-agent-overlays.md`, with one self-contained top-level section per backend plus the template skeleton. This deliberately reverses the CF-F1 "Out" disposition of design doc 0000163 — the user has clarified that their actual intent is to merge, including the template — and accepts the context-isolation trade-off recorded there.

## Success Criteria

- [ ] `skills/cafleet/reference/coding-agent-overlays.md` exists with four top-level sections (`## claude`, `## codex`, `## opencode`, `## Template`), each self-contained; the four old files and the `coding-agent/` directory are deleted.
- [ ] No mention of the deleted artifacts remains outside `design-docs/` and git history: `claude-overlay.md` / `codex-overlay.md` / `opencode-overlay.md`, the `<name>-overlay.md` placeholder pattern, `_template.md`, and the `reference/coding-agent/` directory path (per the removal rule, the repo reads as if the per-file layout never existed; the rule file `.claude/rules/coding-agent-overlay.md` keeps its name and matches none of these patterns).
- [ ] Every skill entry point's Required-reading row #1 links the merged file with a per-backend anchor (`coding-agent-overlays.md#<name>`).
- [ ] `.claude/rules/coding-agent-overlay.md` states the single-file layout as policy, including the reader contract and the accepted trade-off.
- [ ] The two `docs_sync.rs` guards assert per-backend-section (section slicing), preserving current guard strength; `mise //cafleet:test` and `mise //cafleet:lint` pass.
- [ ] The `cafleet` binary is reinstalled and the deployed skill replicas under `~/.claude/skills/` are refreshed from it.

---

## Background

Design doc 0000163 (skill-boilerplate-dedup) recorded finding CF-F1 — "Overlay consolidation into one `overlays.md`" — as **Out**, with the rationale: "the per-backend file split is the isolation mechanism that keeps wrong-backend values out of every member's context, and two docs_sync tests pin the four-file layout." The user has since clarified that their actual intent was to merge the overlays into a single file, template included.

**Accepted trade-off.** With one file, every reader's context contains all three backends' values (a Read pulls the whole file). The substitute for per-file isolation is structural: strict, self-contained per-backend sections plus a resolve-only-your-section rule, which keeps overlay resolution deterministic even though wrong-backend values are now visible. The Director remains the sole documented cross-section reader (it applies the *target member's* backend section for pane-state cues — semantics unchanged from today, where it reads the target's overlay file).

Current constraints this change must address:

| Constraint | Where |
|---|---|
| Two test guards iterate the per-backend files and pin the four-file layout | `cafleet/tests/docs_sync.rs` — `every_backend_overlay_defines_the_capture_cues` (~line 188), `every_backend_overlay_defines_the_full_placeholder_vocabulary` (~line 552) |
| Required-reading row #1 links and prose pointers name `coding-agent/<name>-overlay.md`; the exhaustive per-file enumeration is Implementation Steps 3–5 | `skills/cafleet/**`, `skills/cafleet-design-doc/**`, `skills/cafleet-research/**`, `.claude/skills/clean-docs/**`, `.claude/skills/skill-author/SKILL.md`, `.claude/skills/cafleet-model-list-refresh/SKILL.md` |
| The one-file-per-backend layout is stated as project policy | `.claude/rules/coding-agent-overlay.md` |
| Skills are embedded into the binary at build time and deployed to `~/.claude/skills/` by `cafleet setup` | `cafleet/build.rs` (`SKILLS` table); replica refresh requires a binary reinstall first |

`SPEC.md`, the Rust CLI source, and the operator docs under `docs/docs/spec/` carry no per-backend overlay file-path contract, so the change is confined to skills, tests, project rules, and one docs wording check.

---

## Specification

### Merged file layout

Path: `skills/cafleet/reference/coding-agent-overlays.md`. The `coding-agent/` directory is deleted (a directory holding one file is residue). Structure:

```markdown
# Coding-Agent Overlays

[Intro: one section per backend; every section is self-contained. Reader
contract pointer: identify your backend from the spawn prompt's
`CODING AGENT:` line, then resolve ONLY your backend's section per the
cafleet SKILL.md § Resolve your overlay. Values in other sections are
never applicable to you. The Director is the sole cross-section reader:
it applies the TARGET member's section for pane-state cues.]

## claude
[placeholder table]
### Note → applies at
### Pane-state capture cues
### Worked resolution

## codex
[same interior structure]

## opencode
[same interior structure]

## Template
[the _template.md skeleton near-verbatim: the `<backend name>` /
angle-bracket placeholders and all authoring guidance preserved;
adding a backend = copy this section as a new `## <name>` section]
```

Content rules:

- Per-backend section content migrates **verbatim** from the current files; only heading levels change (`# Overlay: claude` → `## claude`; interior `##` headings demote to `###`).
- Each backend section stays fully self-contained: its own placeholder table, *Note → applies at* table, pane-state capture cues (with the tie-break pointer sentence), and worked resolution. Cross-section references ("same as claude") are forbidden — self-containment is what keeps single-section reading sufficient and resolution deterministic.
- The `## Template` section preserves `_template.md` near-verbatim (interior headings demoted), and its lead-in states the new procedure: adding a backend means copying the Template section into a new `## <name>` section of this same file.
- Top-level section anchors are the link contract: `#claude`, `#codex`, `#opencode`. Duplicate interior heading names across sections are acceptable (nothing links them by anchor).

### Reader contract

The canonical resolve procedure stays owned by `skills/cafleet/SKILL.md` § *Resolve your overlay*; this change updates its wording, not its ownership:

- "Read your overlay" becomes "Read `reference/coding-agent-overlays.md` and resolve **only your backend's section**" (backend identified from the `CODING AGENT:` line as today; standalone agents use their own identity).
- The materialize step's resolution order is unchanged; the source of a token's value is your backend's section of the merged file (then the documented defaults, unchanged, when your section omits the token or the backend is unknown).
- New explicit rule: a value taken from another backend's section is a resolution defect, same class as emitting a literal `{token}`.
- Director exception (documented in the same place as today's cross-overlay behavior, `reference/supervision.md` / the monitoring taxonomy): the Director applies the cues of the **target member's** backend section.

### Link form

Every Required-reading row #1 (and equivalent prose pointer) changes from:

```markdown
your overlay [`<relpath>/coding-agent/<name>-overlay.md`](<relpath>/coding-agent/)
```

to:

```markdown
your overlay section [`<relpath>/coding-agent-overlays.md#<name>`](<relpath>/coding-agent-overlays.md)
```

The visible text carries the per-backend anchor (with `<name>` as the reader-substituted placeholder, as today); the href targets the file itself, since a literal `<name>` inside an href does not resolve in renderers. Pointers that today link concrete backends by name (e.g. `reference/cli.md`'s "Per-backend deltas: claude / codex / opencode") link the concrete anchors instead: `coding-agent-overlays.md#claude` etc.

### Rule rewrite — `.claude/rules/coding-agent-overlay.md`

The rule is rewritten to state the single-file layout as the current, intended policy (no reversal narration in the rule itself; this design doc is the historical record):

- **Where backend specifics live**: the single file `skills/cafleet/reference/coding-agent-overlays.md`, one top-level section per backend, with the `## Template` section as the skeleton for new backends. The existing content inventory (decision surface, permission flags, background/task primitives, pane title, effort levels, skill-loading recipe) and the `{reviewer_model}` / model-list relationship carry over unchanged.
- **Base-neutrality scoping**: the clause "every `reference/*.md` page (outside `skills/cafleet/reference/coding-agent/`)" becomes "every `reference/*.md` page (other than the per-backend sections of `skills/cafleet/reference/coding-agent-overlays.md`)".
- **How the base and overlay connect**: row #1 remains the merged file with the reader's per-backend anchor; the resolve steps and documented-defaults paragraphs update file-path wording only.
- **New: reader contract** — sections are self-contained; resolve only your backend's section; the Director is the sole cross-section reader; the context-isolation trade-off is accepted with sectioning as the deterministic-resolution substitute.
- **Two independent homes**: unchanged in substance; the agent-facing home is now the single file.

### Test guard rewrite — `cafleet/tests/docs_sync.rs`

Add a section-slicing helper and rewrite the two guards to assert per-section, preserving current per-file guard strength (a whole-file assertion would let one backend's section silently lose a required term):

```rust
/// Slice the merged overlay file at top-level `## ` boundaries.
/// Panics (test failure) when the named section is missing.
fn overlay_section<'a>(text: &'a str, name: &str) -> &'a str
```

| Guard | Rewritten assertion |
|---|---|
| `every_backend_overlay_defines_the_capture_cues` | For each of `claude` / `codex` / `opencode`: the backend's **section** contains `working`, `stall_candidate`, `quiet`, `ambiguous`, `pre-ping capture gate`. The `Template` **section** contains `working`, `stall_candidate`, `Note → applies at`. The absent set (`pre-nudge` + `REMOVED_VOCABULARY`) is asserted once against the whole file. |
| `every_backend_overlay_defines_the_full_placeholder_vocabulary` | For each of the four sections (`claude`, `codex`, `opencode`, `Template`): every `OVERLAY_PLACEHOLDERS` entry appears as `{placeholder}` **within that section**. |

The unknown-token walker elsewhere in `docs_sync.rs` scans skill files by directory walk and picks up the merged file automatically; no change needed there beyond the four files disappearing.

### Docs surface

`docs/docs/concepts/monitoring.md` says the Director applies "the cues of the target's backend overlay" — concept-level wording with no repo path, which remains accurate when "overlay" denotes the backend's section. Verify during implementation and adjust only if a sentence stops reading correctly; per `user-facing-docs.md`, no repository path may be introduced.

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

### Step 1: Author the merged overlay file

- [x] Create `skills/cafleet/reference/coding-agent-overlays.md` per § *Merged file layout*: intro with the reader-contract pointer, `## claude` / `## codex` / `## opencode` migrated verbatim (headings demoted), and `## Template` preserving the `_template.md` skeleton near-verbatim with the copy-this-section authoring lead-in. <!-- completed: 2026-08-07T13:04 -->
- [x] Delete `skills/cafleet/reference/coding-agent/claude-overlay.md`, `codex-overlay.md`, `opencode-overlay.md`, `_template.md`, and the now-empty `coding-agent/` directory. <!-- completed: 2026-08-07T13:05 -->

### Step 2: Rewrite the project rule

- [x] Rewrite `.claude/rules/coding-agent-overlay.md` per § *Rule rewrite* (single-file layout, per-backend sections + Template section, anchored row #1, reader contract with the Director exception and the accepted trade-off, updated neutrality scoping clause). <!-- completed: 2026-08-07T13:10 -->

### Step 3: Update the cafleet skill

- [ ] `skills/cafleet/SKILL.md`: Required-reading row #1 link form, § *Resolve your overlay* wording (resolve only your backend's section; other-section values are a resolution defect), and the Required-reading intro sentence. <!-- completed: -->
- [ ] `skills/cafleet/roles/member.md` and `roles/director.md`: row #1 link form. <!-- completed: -->
- [ ] `skills/cafleet/reference/supervision.md` (required-reading note and § decision-surface delta), `reference/recovery.md` (decision-prompt row), `reference/cli.md` (per-backend delta links → concrete anchors), `reference/director.md` (backend-delta links, spawn-rendering paragraph's overlay-selector sentence, relayed-question paragraph). <!-- completed: -->

### Step 4: Update the consumer skill families

- [ ] `skills/cafleet-design-doc/`: `SKILL.md`, `create/create.md`, `interview/interview.md`, `execute/execute.md`, and the 9 role files (`create/roles/{director,drafter,reviewer}.md`, `interview/roles/analyzer.md`, `execute/roles/{director,programmer,tester,reviewer,verifier}.md`) — row #1 link form. <!-- completed: -->
- [ ] `skills/cafleet-research/`: `SKILL.md`, `report/report.md`, `presentation/presentation.md`, and the 8 role files (`report/roles/{director,manager,researcher,scout}.md`, `presentation/roles/{director,presentation,transcript,visual-reviewer}.md`) — row #1 link form. <!-- completed: -->

### Step 5: Update the repo-local .claude/skills consumers

- [ ] `.claude/skills/clean-docs/`: `SKILL.md` (row #1 and the spawn-prompt overlay pointer), the 3 workflow bodies (`residue/residue.md`, `affirmative/affirmative.md`, `simplification/simplification.md`), and the 6 role files (`{residue,affirmative,simplification}/roles/{scanner,reviewer}.md`) — row #1 link form. <!-- completed: -->
- [ ] `.claude/skills/skill-author/SKILL.md` (overlay-home bullet, pointer instruction, rule reference — describe the single-file + Template-section procedure) and `.claude/skills/cafleet-model-list-refresh/SKILL.md` (the `{reviewer_model}` row pointer). <!-- completed: -->

### Step 6: Docs wording check

- [ ] Verify `docs/docs/concepts/monitoring.md`'s "backend overlay" wording still reads correctly against the merged layout; adjust only if needed, introducing no repository path. <!-- completed: -->

### Step 7: Rewrite the docs_sync guards

- [x] Add the `overlay_section` slicing helper to `cafleet/tests/docs_sync.rs` (top-level `## ` boundaries; missing section fails loudly). <!-- completed: 2026-08-07T13:05 -->
- [x] Rewrite `every_backend_overlay_defines_the_capture_cues` per § *Test guard rewrite* (per-section term asserts for the three backends + Template; whole-file absent asserts). <!-- completed: 2026-08-07T13:05 -->
- [x] Rewrite `every_backend_overlay_defines_the_full_placeholder_vocabulary` per § *Test guard rewrite* (per-section `{placeholder}` vocabulary across the four sections). <!-- completed: 2026-08-07T13:05 -->

### Step 8: Verify and deploy

- [ ] Run `mise //cafleet:test` and `mise //cafleet:lint`; both pass. <!-- completed: -->
- [ ] Repo-wide sweep scoped to the deleted artifacts: no match for `(claude|codex|opencode)-overlay\.md`, the literal `<name>-overlay.md`, `_template\.md`, or `reference/coding-agent/` remains outside `design-docs/` (`rg` over the repo, `design-docs/` excluded). The surviving rule path `.claude/rules/coding-agent-overlay.md` and the merged filename `coding-agent-overlays.md` match none of these patterns. <!-- completed: -->
- [ ] Reinstall the binary (`mise //cafleet:install`), then run `cafleet setup` to refresh the deployed skill replicas under `~/.claude/skills/` from the embedded assets. <!-- completed: -->

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-08-07 | Initial draft |
