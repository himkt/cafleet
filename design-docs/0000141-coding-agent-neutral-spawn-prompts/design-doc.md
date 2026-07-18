# Coding-Agent-Neutral Spawn Prompts and Skill-Wide Neutrality Sweep

**Status**: Approved
**Progress**: 20/32 tasks complete
**Last Updated**: 2026-07-18

## Overview

The execute-workflow spawn prompts order every member to read `.claude/rules/bash-tool.md` and `~/.claude/rules/bash-command.md` — Claude-Code-specific paths that led codex and opencode members to probe `~/.claude`. This design replaces that line with a backend-neutral pointer to the cafleet skill's member Bash protocol and sweeps the remaining coding-agent-specific residue out of the skill bodies, including three approved scope expansions: renaming the overlay files to a collision-free scheme, renaming the `claude` marker-role tokens, and renaming the `/tmp/claude-code` scratch directory to `/tmp/cafleet`.

## Success Criteria

All criteria are verified by manual repo-wide search (no new automated guard test). Search scope is `skills/`, `docs/`, `SPEC.md`, `README.md`, and `.claude/` unless narrower; `design-docs/` and `researches/` are always excluded.

- [ ] `bash-tool.md` and `bash-command.md` have zero mentions in `skills/`.
- [ ] Every remaining `~/.claude` in `skills/` sits within a three-backend enumeration block (the base-dir config-dir table or the slidev discovery hints) whose sibling entries cover codex and opencode.
- [ ] `/tmp/claude-code` has zero mentions in `skills/`, `docs/`, `SPEC.md`, `README.md`, and `.claude/`.
- [ ] The overlay files exist as `coding-agent/claude-overlay.md`, `codex-overlay.md`, `opencode-overlay.md`; `coding-agent/claude.md` does not exist; every overlay reference (skills and `.claude/`) uses the `coding-agent/<name>-overlay.md` scheme, and no `coding-agent/<name>.md` reference remains.
- [ ] `COMMENT(claude)`, `FIXME(claude)`, and `DONE(claude)` have zero mentions in `skills/`, `docs/`, `SPEC.md`, and `.claude/`; the coordination.md role table carries `user-relay` in place of `claude`.
- [ ] `Main Claude`, `main Claude`, `claude pane`, and the phrase `` `claude` process `` have zero mentions in `skills/`.
- [ ] `AskUserQuestion` appears in `skills/` only inside `coding-agent/claude-overlay.md` and the `allowed-tools` frontmatter of `skills/cafleet-design-doc/SKILL.md`.
- [ ] Every remaining `Claude` mention in `skills/` matches the by-design keep list (Specification § By-design keep list).
- [ ] The four execute-role spawn deltas and the lossless-rule bullet in `skills/cafleet/reference/director.md` carry the identical neutral member-Bash IMPORTANT line, verbatim.

---

## Background

The Director's neutrality audit (`audit-findings.md`, same directory) grouped the residue into tiers; the user confirmed the scope:

| Audit tier | Content | Decision |
|:--|:--|:--|
| §1 root cause | The member-Bash IMPORTANT line in 4 execute-role spawn deltas + the lossless-rule bullet | Replace with a neutral skill-relative pointer, one line per delta (option a) |
| §2 `~/.claude` hardcodes | base-dir.md resolution, execute.md:82 edge case, slidev.md discovery hints | Generalize functionally (per-backend config dirs), `{decision_surface}` token, three-backend slidev table |
| §3 Claude-first wording | 12 role-file shutdown boilerplates, org charts, Do-NOT lists, `CLAUDE.md` mention, example member names, misc. prose | Neutralize all |
| §4 borderline `.claude/rules/` references | presentation.md, visualization.md, execute reviewer.md | Layout-agnostic phrasing, repo path kept as concrete example |
| §5 by-design enumerations | Three-backend tables, identity-mapping examples, model tables | Keep untouched |
| §6 filename collision | `coding-agent/claude.md` ≡ `CLAUDE.md` on case-insensitive filesystems | Fix in this design: rename overlays, update lookup scheme |
| §7 marker grammar | `COMMENT(claude)` / `FIXME(claude)` role tokens | Rename in this design |
| — scratch path | `/tmp/claude-code` candidate | Rename to `/tmp/cafleet`, sweep every mention |

`skills/` in the repo is the source of truth; `~/.claude/skills` is an installed copy and is **not** an edit target — propagation happens through the normal skill-install flow. Verification is manual grep sweeps; no automated guard test.

---

## Specification

### 1. The neutral member-Bash line

The five occurrences of the Claude-specific line

> `IMPORTANT: Read and follow .claude/rules/bash-tool.md (CAFleet-member Bash protocol) and ~/.claude/rules/bash-command.md (general Bash hygiene) for all Bash commands.`

are replaced by this line, verbatim and identical in all locations:

> `IMPORTANT: For every Bash command, follow the member Bash protocol in the cafleet skill (its roles/member.md and reference/exec-routing.md), which you load at startup.`

Design decisions:

- **Skill-relative, never path-relative.** The pointer names the skill and its internal pages, resolvable on every backend via the member's skill loader. Spawn prompts never reference repo paths (`.claude/rules/...`) or operator-home paths (`~/.claude/...`).
- **One line per execute-role delta (Programmer, Tester, Verifier, Reviewer), not hoisted into the canonical skeleton.** Hoisting would silently extend the line to the create / interview / research / presentation teams, a behavior expansion the user declined.
- **The lossless-rule bullet** at `skills/cafleet/reference/director.md` ("All execute roles: …") is rewritten to quote the new line verbatim, keeping the reconstruction check accurate.
- **`.claude/rules/bash-tool.md` stays in the repo unchanged.** It is Claude-Code harness glue that auto-loads only for claude sessions in this repo; it simply disappears from every spawn prompt.

### 2. Base-dir resolution: per-backend config dirs and `{decision_surface}`

`skills/cafleet/reference/base-dir.md` currently special-cases `~/.claude` (`claude_subdir = $HOME/.claude`), names `AskUserQuestion` directly, and says "Claude's job is to…". Changes:

- **Functional generalization.** The Step 1 check becomes: the CWD is `$HOME` exactly, or under **any** coding agent's user-level config directory:

  | Backend | User-level config dir |
  |:--|:--|
  | claude | `~/.claude` |
  | codex | `~/.codex` |
  | opencode | `~/.config/opencode` |

  All three dirs are checked regardless of which backend is resolving — resolution then needs no backend identity, and a Director of one backend whose CWD sits inside another backend's config dir is still in an operator-config location, not a project. No new overlay token is introduced.

  base-dir.md's `~/.claude` occurrences are exhaustively at four lines, all rewritten to the config-dir-table phrasing: line 9 (Procedure intro, "(CWD is `$HOME` or under `~/.claude`)"), line 38 (the Step 1 check, ×2 on the line), line 39 (Step 1 branch 3), and line 49 (the Step 2 re-prompt note, "same `$HOME` / `~/.claude` CWD").
- **`{decision_surface}` token.** Every direct `AskUserQuestion` naming in base-dir.md (including the "Step 2. AskUserQuestion" heading, retitled "Step 2. Decision-surface prompt") becomes the `{decision_surface}` placeholder, resolved per the reader's overlay. The same substitution applies to the two other base-body namings: `skills/cafleet/reference/supervision.md` ("a pending `AskUserQuestion` is protected…") and `skills/cafleet/roles/monitor.md` ("…would cancel a Director's pending `AskUserQuestion`") — both become "a pending `{decision_surface}` prompt".
- **Neutral prose.** "Claude's job is to…" becomes "The resolving agent's job is to…".
- **`skills/cafleet-design-doc/execute/execute.md` Step 1 Phase 1** repeats the edge case ("the repo root is itself `$HOME` or under `~/.claude`") — rephrased to "under a coding agent's user-level config directory (per base-dir.md's table)".

### 3. Scratch directory rename: `/tmp/claude-code` → `/tmp/cafleet`

The decision-surface candidate and every mention of it are renamed. `/tmp` content is ephemeral; no migration of existing directories is performed. Confirmed occurrences (the full set — `docs/`, `SPEC.md`, `README.md`, and `.claude/settings*.json` have zero):

| File | Lines |
|:--|:--|
| `skills/cafleet/reference/base-dir.md` | 5, 39, 45 (×2), 55 |
| `skills/cafleet-design-doc/execute/execute.md` | 82 (×2) |
| `skills/cafleet-research/reference/visualization.md` | 17, 27, 44 |
| `.claude/skills/skill-author/SKILL.md` | 253 |

### 4. Overlay filename scheme: `coding-agent/<name>-overlay.md`

`coding-agent/claude.md` collides case-insensitively with `CLAUDE.md` (macOS default filesystem), so Claude Code auto-injects the overlay as project instructions for any session touching the directory. Fix: rename **all three** overlay files uniformly —

| Old | New |
|:--|:--|
| `coding-agent/claude.md` | `coding-agent/claude-overlay.md` |
| `coding-agent/codex.md` | `coding-agent/codex-overlay.md` |
| `coding-agent/opencode.md` | `coding-agent/opencode-overlay.md` |

`_template.md` keeps its name (it is authored-to, never backend-addressed). The deterministic lookup rule stays a single sentence — "your overlay is `reference/coding-agent/<name>-overlay.md`, where `<name>` is your spawn prompt's `CODING AGENT:` value" — with no per-backend special case. A claude-only rename (e.g. `claude-code.md`) would fix the collision but embed a special case into every Required-reading row; the uniform suffix preserves the one-rule lookup, and `claude-overlay.md` does not collide with `CLAUDE.md`. File contents are unchanged.

Every reference to the scheme updates from `<name>.md` to `<name>-overlay.md`. Confirmed reference inventory (all in `skills/` and `.claude/`; `docs/`, `SPEC.md`, `README.md` have zero):

- **Explicit `coding-agent/claude.md` links**: `skills/cafleet/reference/cli.md:43`, `skills/cafleet/reference/director.md:36` (both also link codex.md / opencode.md).
- **Required-reading row #1 scheme references** (`coding-agent/<name>.md`): `skills/cafleet/SKILL.md:24`; `skills/cafleet/roles/{director,member,monitor}.md:15`; `skills/cafleet-design-doc/SKILL.md:30`, `interview/interview.md:13`, `interview/roles/analyzer.md:13`, `execute/execute.md:13`, `execute/roles/{reviewer,verifier,programmer,tester,director}.md:13`, `create/create.md:13`, `create/roles/{reviewer,drafter,director}.md:13`; `skills/cafleet-research/SKILL.md:30`, `report/report.md:13`, `report/roles/{researcher,scout,manager,director}.md:13`, `presentation/presentation.md:13`, `presentation/roles/{presentation,transcript,director}.md:13`, `presentation/roles/visual-reviewer.md:15`.
- **Inline scheme mentions**: `skills/cafleet/reference/director.md:131,192`; `skills/cafleet/reference/supervision.md:5,157`; `skills/cafleet/reference/recovery.md:19`.
- **`.claude/` (repo-committed)**: `.claude/rules/coding-agent-overlay.md:7,11` (the directory-only mentions at lines 3 and 25 are unchanged); `.claude/skills/skill-author/SKILL.md:530,534`; `.claude/skills/historical-residue-cleanup/SKILL.md:55,192`, `roles/scanner.md:19`, `roles/reviewer.md:19`.
- **Python source / tests**: expected zero references; verified by search during implementation.

### 5. Marker-role rename: `claude` → `user-relay` / `agent`

The `claude` role token in the marker grammar names a backend where it should name a function. Two distinct usages, two renames:

| Old | New | Rationale |
|:--|:--|:--|
| `COMMENT(claude)` | `COMMENT(user-relay)` | The role is "the Director acting as user-mediator, carrying user-derived clarifications" — the marker relays the user's answer, on any backend. |
| `FIXME(claude)` / `DONE(claude)` | `FIXME(agent)` / `DONE(agent)` | These codebase markers are addressed to whichever coding agent executes the workflow, not to Claude specifically. |

This is a hard-break rename with no alias (per the removal rule): every repo mention updates in this change, and resume-mode detection greps only the new token. In-flight design docs carrying old-token markers (if any exist outside `design-docs/`, which is out of sweep scope anyway) are the operator's to resolve manually; git history is the record.

Confirmed occurrence inventory:

| File | Lines | Token |
|:--|:--|:--|
| `skills/cafleet-design-doc/reference/coordination.md` | 5, and the role table `claude` row (~90) | COMMENT |
| `skills/cafleet-design-doc/interview/interview.md` | 3, 16, 28, 32, 34, 203 | COMMENT |
| `skills/cafleet-design-doc/create/create.md` | 74, 75, 77, 78, 180 | COMMENT |
| `skills/cafleet-design-doc/execute/execute.md` | 143 (FIXME), 284 (COMMENT) | both |
| `skills/cafleet-design-doc/execute/roles/tester.md` | 49 | COMMENT |
| `skills/cafleet-design-doc/execute/roles/director.md` | 26 | FIXME |
| `skills/cafleet-design-doc/execute/roles/programmer.md` | 47, 49, 57, 62, 67 | FIXME + DONE |
| `docs/contributing.md` | 95 | COMMENT |

### 6. Claude-first wording sweep (base bodies)

| Old wording | New wording | Files |
|:--|:--|:--|
| ``Your `claude` process is terminated`` | `Your coding-agent process is terminated` | 12 role files: `cafleet-design-doc/interview/roles/analyzer.md:90`, `execute/roles/reviewer.md:74`, `verifier.md:82`, `tester.md:87`, `programmer.md:116`, `create/roles/reviewer.md:70`, `drafter.md:85`; `cafleet-research/report/roles/manager.md:110`, `scout.md:89`, `researcher.md:79`; `presentation/roles/transcript.md:86`, `presentation.md:102` (visual-reviewer.md has no such phrase — verified) |
| `Main Claude` (role-table Identity cells) | `Main agent` | `interview/interview.md:22`, `execute/execute.md:22`, `create/create.md:22`, `report/report.md:21`, `presentation/presentation.md:21` |
| `main Claude` (prose + org-chart trees) | `main agent` | `interview/interview.md:3,42`, `execute/execute.md:48`, `create/create.md:46`, `report/report.md:44`, `presentation/presentation.md:38` |
| `claude pane` | `member pane` | `report/report.md:22,23,24,45,46,47`, `presentation/presentation.md:22,23,24,39,40,41` |
| ``spawn subagents or run `claude` commands`` | `spawn subagents or run coding-agent CLI commands` | `execute/roles/programmer.md:37`, `tester.md:37`, `verifier.md:37` |
| ``Check project's `CLAUDE.md` `` | ``Check the project-instructions file (`CLAUDE.md` / `AGENTS.md`, per your harness)`` | `execute/roles/tester.md:47` |
| `Bash calls in Claude Code are ephemeral` | `Bash calls in coding-agent harnesses are ephemeral` | `cafleet-research/reference/visualization.md:12` |
| `complete enough that Claude can implement` | `complete enough that an agent can implement` | `cafleet-design-doc/reference/guidelines.md:55` |
| `--name Claude-B` / `--name Codex-A` spawn examples (and their `.prompts/claude-b-…` / `codex-a-…` file names) | Role-based names: `--name Reviewer-B` (first example) / `--name Reviewer-C --coding-agent codex` (second), file names to match | `cafleet/reference/director.md:12-18` |

### 7. `.claude/rules/` references: layout-agnostic phrasing

The repo-committed rules directory stays a legitimate referent, but the phrasing becomes layout-agnostic — "the host project's agent rules directory (`.claude/rules/` in this repo)" — keeping the concrete path as an example:

- `skills/cafleet-research/presentation/presentation.md:198,200,201`
- `skills/cafleet-research/reference/visualization.md:4,72`
- `skills/cafleet-design-doc/execute/roles/reviewer.md:23`

### 8. slidev.md: three-backend discovery table

The plugin-install-dir discovery hints (`skills/cafleet-research/reference/slidev.md:11-13`) currently enumerate Claude Code and Codex only. They become an explicit three-backend enumeration with neutral framing:

```
# Discovery hints (per coding-agent backend):
#   - claude:    ~/.claude/plugins/cache/cafleet/cafleet/<version>/   (run `claude plugin list` to find <version>)
#   - codex:     the path printed by `codex plugin list` for the cafleet plugin
#   - opencode:  ~/.config/opencode/skills/   (cafleet skills install dir; no plugin cache)
```

The opencode row points at the skills install dir from the setup contract (`SPEC.md`: `opencode` → `~/.config/opencode/skills`); the theme then resolves at `<install-dir>/cafleet-research/reference/slidev/theme`. Implementation verifies the concrete opencode path against `cafleet setup`'s install targets before landing the row.

### 9. Blast radius outside `skills/` (required enumeration)

The three scope expansions touch these non-`skills/` surfaces — the complete set, per repo-wide search:

| Surface | Touched by | Files |
|:--|:--|:--|
| `docs/` | marker-role rename | `docs/contributing.md:95` only |
| `SPEC.md` | nothing | zero hits for all three expansions (`SPEC.md:1344`'s `~/.claude/skills` is the by-design per-backend install-dir enumeration and is unchanged) |
| `README.md` | nothing | zero hits |
| `.claude/` | overlay rename; `/tmp` rename | `.claude/rules/coding-agent-overlay.md:7,11`; `.claude/skills/skill-author/SKILL.md:253,530,534`; `.claude/skills/historical-residue-cleanup/SKILL.md:55,192` + `roles/scanner.md:19` + `roles/reviewer.md:19` |
| `.claude/settings*.json` | nothing | zero `/tmp/claude-code` permission patterns — verified |

### 10. By-design keep list (untouched)

- Three-backend enumerations and per-backend delta pointers (`cafleet/reference/cli.md`, `director.md`, `exec-routing.md`, `roles/member.md`, `reference/supervision.md`).
- "e.g. Claude Code → `claude`" identity-mapping examples (10 confirmed sites: `interview/interview.md:108`, `execute/execute.md:160`, `execute/roles/director.md:25`, `create/create.md:95`, `create/roles/director.md:25`, `cafleet/reference/cli.md:138`, `report/report.md:81`, `report/roles/director.md:20`, `presentation/presentation.md:96`, `presentation/roles/director.md:20`).
- Per-backend model tables and the model-name-to-backend inference rules (`cafleet/reference/director.md:46,54,61`).
- The claude overlay's own content (`coding-agent/claude-overlay.md` post-rename), including its `AskUserQuestion` mentions.
- The `allowed-tools` frontmatter of `skills/cafleet-design-doc/SKILL.md` (harness metadata, ignored by other backends).
- `docs/quickstart.md:37` and `docs/spec/cli-options.md:160` (`~/.claude/settings.json` — operator docs describing claude-specific configuration).
- `SPEC.md:1344` (per-backend skills install dirs).
- `.claude/rules/bash-tool.md` content (kept as claude harness glue, § 1).
- `~/.claude` references inside `.claude/rules/` and `.claude/skills/historical-residue-cleanup/` that point at the operator's global rules (they describe the claude-backend operator setup of this repo, not agent-facing skill prose).

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

### Step 1: Replace the member-Bash IMPORTANT line

- [x] `skills/cafleet-design-doc/execute/execute.md`: replace the line in all four role deltas (Programmer :202, Tester :223, Verifier :246, Reviewer :363) with the § 1 neutral line, verbatim <!-- completed: 2026-07-18T11:11 -->
- [x] `skills/cafleet/reference/director.md:149`: rewrite the lossless-rule "All execute roles" bullet to quote the new line verbatim <!-- completed: 2026-07-18T11:11 -->

### Step 2: Generalize base-dir resolution

- [x] `skills/cafleet/reference/base-dir.md`: per-backend config-dir table replacing `claude_subdir`, all four `~/.claude` lines rewritten (9, 38, 39, 49 per § 2), `{decision_surface}` for every `AskUserQuestion` naming (incl. the Step 2 heading), "The resolving agent's job", `/tmp/cafleet` on lines 5, 39, 45, 55 <!-- completed: 2026-07-18T11:14 -->
- [x] `skills/cafleet-design-doc/execute/execute.md:82`: config-dir phrasing + `/tmp/cafleet` (×2) <!-- completed: 2026-07-18T11:14 -->
- [x] `skills/cafleet/reference/supervision.md:33`: "a pending `{decision_surface}` prompt" <!-- completed: 2026-07-18T11:14 -->
- [x] `skills/cafleet/roles/monitor.md:75`: "a Director's pending `{decision_surface}` prompt" <!-- completed: 2026-07-18T11:14 -->

### Step 3: Finish the `/tmp/cafleet` rename

- [x] `skills/cafleet-research/reference/visualization.md:17,27,44` <!-- completed: 2026-07-18T11:16 -->
- [x] `.claude/skills/skill-author/SKILL.md:253`; search skill-author for any other base-dir restatement (`~/.claude` branch, `AskUserQuestion`) and align it with § 2 <!-- completed: 2026-07-18T11:19 -->

### Step 4: Rename the overlay files and lookup scheme

- [x] `git mv` the three overlays to `claude-overlay.md` / `codex-overlay.md` / `opencode-overlay.md` (`_template.md` unchanged) <!-- completed: 2026-07-18T11:26 -->
- [x] cafleet skill references: `SKILL.md:24`; `roles/{director,member,monitor}.md:15`; `reference/cli.md:43`, `reference/director.md:36,131,192`, `reference/supervision.md:5,157`, `reference/recovery.md:19` <!-- completed: 2026-07-18T11:24 -->
- [x] cafleet-design-doc references: `SKILL.md:30` + the 12 workflow/role files listed in § 4 <!-- completed: 2026-07-18T11:24 -->
- [x] cafleet-research references: `SKILL.md:30` + the 10 workflow/role files listed in § 4 <!-- completed: 2026-07-18T11:24 -->
- [x] `.claude/` references: `rules/coding-agent-overlay.md:7,11`; `skills/skill-author/SKILL.md:530,534`; `skills/historical-residue-cleanup/SKILL.md:55,192`, `roles/scanner.md:19`, `roles/reviewer.md:19` <!-- completed: 2026-07-18T11:26 -->
- [x] Verify zero `coding-agent/claude.md` / `coding-agent/<name>.md` references in `cafleet/src/` and `cafleet/tests/` <!-- completed: 2026-07-18T11:24 -->

### Step 5: Rename the marker-role tokens

- [x] `skills/cafleet-design-doc/reference/coordination.md`: role table `claude` → `user-relay`, line 5 mention <!-- completed: 2026-07-18T11:29 -->
- [x] `skills/cafleet-design-doc/interview/interview.md:3,16,28,32,34,203` <!-- completed: 2026-07-18T11:29 -->
- [x] `skills/cafleet-design-doc/create/create.md:74,75,77,78,180` <!-- completed: 2026-07-18T11:29 -->
- [x] `skills/cafleet-design-doc/execute/execute.md:143` (FIXME→agent), `:284` (COMMENT→user-relay) <!-- completed: 2026-07-18T11:29 -->
- [x] `skills/cafleet-design-doc/execute/roles/tester.md:49`, `director.md:26`, `programmer.md:47,49,57,62,67` (FIXME/DONE→agent) <!-- completed: 2026-07-18T11:29 -->
- [x] `docs/contributing.md:95` <!-- completed: 2026-07-18T11:29 -->

### Step 6: Claude-first wording sweep

- [ ] Shutdown boilerplate in the 12 role files listed in § 6: "Your coding-agent process is terminated" <!-- completed: -->
- [ ] `Main Claude` → `Main agent` and `main Claude` → `main agent` in the 5 workflow bodies (§ 6 lines) <!-- completed: -->
- [ ] `claude pane` → `member pane` in `report/report.md` and `presentation/presentation.md` (§ 6 lines) <!-- completed: -->
- [ ] Do-NOT lists in `execute/roles/{programmer,tester,verifier}.md:37`: "run coding-agent CLI commands" <!-- completed: -->
- [ ] `execute/roles/tester.md:47`: project-instructions file (`CLAUDE.md` / `AGENTS.md`) <!-- completed: -->
- [ ] `cafleet-research/reference/visualization.md:12` and `cafleet-design-doc/reference/guidelines.md:55` neutral phrasing <!-- completed: -->
- [ ] `cafleet/reference/director.md:12-18`: role-based example member names (`Reviewer-B` / `Reviewer-C`) and matching `.prompts/` file names <!-- completed: -->

### Step 7: Layout-agnostic `.claude/rules/` phrasing

- [ ] `cafleet-research/presentation/presentation.md:198,200,201` <!-- completed: -->
- [ ] `cafleet-research/reference/visualization.md:4,72` <!-- completed: -->
- [ ] `cafleet-design-doc/execute/roles/reviewer.md:23` <!-- completed: -->

### Step 8: slidev.md three-backend discovery table

- [ ] Replace lines 11-13 with the § 8 three-backend hints; verify the opencode install path against `cafleet setup`'s install targets before landing <!-- completed: -->

### Step 9: Verification sweep

- [ ] Run every Success Criteria search, fix any straggler, and check the criteria boxes <!-- completed: -->

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-07-18 | Initial draft |
| 2026-07-18 | Review round 1: Success Criterion #2 rephrased to block scope; § 2 base-dir `~/.claude` line inventory (9, 38, 39, 49); `DONE(claude)` inventory extended to `programmer.md:62,67`; identity-mapping keep-list count corrected to 10 with sites enumerated |
| 2026-07-18 | Status → Approved |
