# 0000057 — Claude Configuration Cleanup

**Status**: Approved
**Progress**: 4/6 tasks complete
**Last Updated**: 2026-05-13

## Overview

Unify the two CLAUDE.md files by deleting `.claude/CLAUDE.md` (a near-duplicate of root `CLAUDE.md` that was left behind when design 0000056 deferred the cleanup), fix three broken `rules/bash-command.md` references in `skills/design-doc-execute/SKILL.md`, and record the placement-audit decisions for `.claude/rules/`, `~/.claude/rules/`, `skills/`, `~/.claude/CLAUDE.md`, and `.claude/settings.json` so future contributors do not re-litigate them.

## Success Criteria

- [ ] `/home/himkt/work/himkt/cafleet/.claude/CLAUDE.md` no longer exists.
- [ ] No tracked file in the repo references `.claude/CLAUDE.md`. Definition of "tracked file": the set returned by `git ls-files`. This cleanly excludes `.git/` internals AND gitignored working-tree artifacts (member-spawn audit files, scratch question/answer files), while still scanning `design-docs/` — which is acceptable because `design-docs/` matches are historical and need not change.
- [ ] No tracked file in the repo references the **bare** path `rules/bash-command.md` (no `.claude/` or `~/.claude/` prefix). The three sites in `skills/design-doc-execute/SKILL.md` (lines 400, 445, 491) point to the two real files: `.claude/rules/bash-tool.md` and `~/.claude/rules/bash-command.md`. Fully-qualified matches are correct and expected.
- [ ] Root `CLAUDE.md` is unchanged in heading, structure, and section order — only the deletion of `.claude/CLAUDE.md` is performed.
- [ ] Zero deprecation notices, removal markers, or "see design 0000057" pointers exist in any tracked file. The repo reads as if `.claude/CLAUDE.md` never existed.

---

## Background

Three forces converged to motivate this change.

| Force | Source |
|---|---|
| `.claude/CLAUDE.md` and root `CLAUDE.md` are ~85 % identical and have drifted together through every prior design doc that updated either one (0000009, 0000010, 0000011, 0000016, 0000017, 0000019, 0000021, 0000022, 0000023, 0000024, 0000045, 0000046, 0000047, 0000049, 0000051, 0000052, 0000053, 0000054). The doubled-edit cost is paid every time. |
| Design 0000049 §11 Step 14 approved deleting `.claude/CLAUDE.md` outright. The delete itself was deferred to a later cleanup batch (line: `[x] Delete .claude/CLAUDE.md. <!-- completed: 2026-05-05T08:50 (deferred …) -->`). |
| Design 0000056 surfaced the same duplication, but deferred it again on the grounds that the user's Q3 answer had described the overlap as "intentional plugin self-containment" (0000056 line 162). The user has now explicitly overridden that classification: this cleanup is authorized. |

The placement audit was prompted by an earlier 3-agent screening that found every file in `.claude/rules/` (project-specific) and `~/.claude/rules/` (general) is in the correct home. Recording that result in this design doc prevents the same screening from being re-run by a future contributor who notices the two parallel directories.

The three stale `rules/bash-command.md` references in `skills/design-doc-execute/SKILL.md` are unrelated to the CLAUDE.md merge but were discovered during the same audit cycle. They are bundled into this design because they are small, mechanical, and would otherwise need a one-off micro-design doc each.


---

## Specification

### 1. CLAUDE.md unification

**Action.** Delete `/home/himkt/work/himkt/cafleet/.claude/CLAUDE.md`. Make no edit to root `CLAUDE.md`.

**Heading and structure decision.** Root `CLAUDE.md` keeps its current heading `## Skills` (placed at the top of the file, before `## Project: CAFleet`). The deleted `.claude/CLAUDE.md` used `## Project Skills` (placed at the bottom of the file) — that ordering and heading are dropped along with the file. Rationale: the smaller diff is safer, root `CLAUDE.md` has been the canonical entry point since the repo's inception, and the design 0000049 §11 Step 14 record that picked `## Project Skills` as the unified heading was scoped to a merge *into* `.claude/CLAUDE.md` (now reversed by the delete-not-merge decision below).

**Delete vs. merge.** Per Q3 of the clarification round and `~/.claude/rules/removal.md`: the cleanup is **total**. No removal marker, no `<!-- file moved -->` comment, no "see design 0000057" pointer, no callout, no section-header mention. The git log + this design doc are the only historical record.

**Pre-flight cross-reference scan (executed during drafting; no implementer rescan required).** Grep for the literal string `.claude/CLAUDE.md` across the live documentation surface, **excluding** `design-docs/` (historical, immutable per `~/.claude/rules/removal.md`) and `.git/`. Files scoped: `README.md`, `ARCHITECTURE.md`, `docs/**`, `skills/**` (every `SKILL.md`, every `roles/*.md`, every `reference/*.md`), `.claude-plugin/**`, `.codex-plugin/**`, `mise.toml`, `pyproject.toml`, `.pre-commit-config.yaml`, `cafleet/**` (Python source). Result: **zero live references**. The only matches in the entire repo are inside `design-docs/` (historical record, preserved) and inside this design's own untracked scratch artifacts (`drafter-questions-0000057.md`, `drafter-answers-0000057.md` — working-tree-only, not staged, not in `design-docs/`). No implementer-side rewrites are required when the file is deleted.


### 2. Stale Bash-rule references in `skills/design-doc-execute/SKILL.md`

**Symptom.** Three sites carry the identical broken line:

```
IMPORTANT: Read and follow rules/bash-command.md for all Bash commands.
```

| Site | Line | Spawn prompt for |
|---|---|---|
| 1 | `skills/design-doc-execute/SKILL.md:400` | Programmer |
| 2 | `skills/design-doc-execute/SKILL.md:445` | Tester |
| 3 | `skills/design-doc-execute/SKILL.md:491` | Verifier |

The path `rules/bash-command.md` resolves to no file in this repo. Two real files cover the topic, with non-overlapping content:

| Real path | Scope | Content summary |
|---|---|---|
| `.claude/rules/bash-tool.md` | project, CAFleet-member-specific | Bash is enabled under `dontAsk`; do NOT fabricate output; auto-route harness-denied commands to the Director via `cafleet member exec`; Director-side `member ping` vs `member exec` distinction. |
| `~/.claude/rules/bash-command.md` | global, general | No `&&` / `;` chaining; no redirects; tool-substitution table (Glob/Grep/Read/Edit/Write instead of find/grep/cat/sed/echo); no command substitution. |

**Action.** Replace each of the three sites with the line below (Q4 Option C from the clarification round — spawned CAFleet members need both files; they are non-overlapping):

```
IMPORTANT: Read and follow .claude/rules/bash-tool.md (CAFleet-member Bash protocol) and ~/.claude/rules/bash-command.md (general Bash hygiene) for all Bash commands.
```

Fix all three sites (including the Verifier site at line 491, which was not named in the original brief but carries the identical bug — Q5).

### 3. Placement audit — every file is in the correct home (no moves)

The two parallel rule directories have caused intermittent confusion over which file belongs where. A focused screening confirmed every file is in its correct home. Record the result here so the screening does not have to be re-run.

**`~/.claude/rules/` — 5 files, all general (correct home):**

| File | Purpose | Why general |
|---|---|---|
| `removal.md` | Total-cleanup rule when removing features. | Applies to any project, not cafleet-specific. |
| `git-workflow.md` | Commit message format, no HEREDOC, no `-C`. | General version-control hygiene. |
| `criticism-response.md` | How to react to user mid-task critique vs. halt signals. | General interaction protocol. |
| `bash-command.md` | No `&&`/`;`, no redirects, tool substitution table. | General Bash hygiene; no cafleet identifiers. |
| `tool-discovery.md` | Beyond dedicated-tool guidance, check skills + MCP. | General tool-use protocol. |

**`.claude/rules/` — 6 files, all project-specific or a necessary override (correct home):**

| File | Purpose | Why project-local |
|---|---|---|
| `skill-discovery.md` | Skill-first for `gh` operations; authorization-scope guard; CAFleet supervision cross-reference. | Names `gh` reviewer slugs, `cafleet member create`, `skills/agent-team-supervision/SKILL.md`. |
| `bash-tool.md` | CAFleet-member Bash behavior (dontAsk, auto-route, `member exec`, `member ping`). | Entirely about CAFleet members; meaningless outside this project. |
| `git-workflow.md` | **Project-specific override**: design-docs/ IS committed in this repo (inverts the global rule). | Override authority — must live next to the file it overrides, in the project, not globally. |
| `code-quality.md` | No meaningless `.get(..., default)`, no unnecessary comments, no `cast()`. | References SQLAlchemy `.returning()` and Python-specific patterns in the cafleet codebase. |
| `design-doc-numbering.md` | 7-digit kebab slug format, docs-first implementation order, `ARCHITECTURE.md` / `docs/` / `README.md` / `SKILL.md` as documentation targets. | Names project-specific docs and folder layout (`design-docs/`, `ARCHITECTURE.md`). |
| `commands.md` | `mise //cafleet:*` task reference, no bypassing mise with the underlying tool. | Lists mise tasks specific to this project's `mise.toml` files. |

**Decision: no moves, no renames.** Recording this audit closes the question for future contributors. The `.claude/rules/` directory MUST NOT be renamed or moved — 53 historical `design-docs/*.md` entries reference paths inside it as immutable records (per the brief's hard constraint).

### 4. Expanded-scope audits

The user expanded the audit to `skills/`, `~/.claude/CLAUDE.md`, and `.claude/settings.json`. Findings:

**`skills/` audit.** Clean except for the three stale Bash-rule references already captured in §2. Specifically:

> **Note on ambient member-spawn audit copies.** `cafleet member create` writes a working-tree audit file at `${BASE}/<role>.md` for each spawned member (e.g. `programmer.md`, `tester.md`, `verifier.md` — all gitignored per `.gitignore:37-48`). The current working-tree copies (written by earlier spawns from the still-broken `SKILL.md`) carry the same `IMPORTANT: Read and follow rules/bash-command.md ...` line. These files refresh naturally on the next spawn from the fixed `SKILL.md` and need not be touched by this design. They are gitignored, so they do not appear in `git ls-files` and do not affect the Success Criteria or Step 3 verification (which both scope to tracked files).


| Dimension | Finding |
|---|---|
| Stale path references | Only the three `rules/bash-command.md` lines in `skills/design-doc-execute/SKILL.md`. No other stale paths. |
| Duplicate / contradictory content | The three intentional plugin self-containment duplications (design-doc-create ↔ design-doc-execute Verb Vocabulary / Pointer Forms tables; cafleet ↔ agent-team-monitoring placeholder convention; design-doc-interview's inlined `COMMENT(role)` convention) are explicitly documented as intentional per design 0000050 Step 7 (plugin-install self-containment). No NEW duplications found. |
| Dead skills / dead references | All 12 skills enumerated in `.claude-plugin/plugin.json` exist as `skills/<name>/` directories. The 5 non-user-invocable helpers (`base-dir`, `create-figure`, `my-slidev`, `research-presentation`, `research-report`) are correctly omitted from the user-invocable list in root `CLAUDE.md`. |
| Casing inconsistency | All skill names match across root `CLAUDE.md`, `.claude-plugin/plugin.json`, and `skills/<name>/` directory names. |
| Plugin manifest enumeration | `.claude-plugin/marketplace.json` and `.claude-plugin/plugin.json` are in sync with the live `skills/` tree. |

No edits to `skills/` are required beyond §2.

**`~/.claude/CLAUDE.md` audit.** 9 lines total. Full contents are two sections: `## Skills` (skill invocation protocol — invoke when description matches, do not bypass with Write/Edit/Bash) and an embedded `EMERGENCY RULE` (terminate the conversation if the skill list fails to load). Both sections are universal — they describe Claude Code skill-loading behavior, not cafleet behavior. No stale skill names, no stale file paths, no cafleet-specific content. No edits required.

**`.claude/settings.json` audit.** One file (settings.json), no `settings.local.json`. Contents are 41 `permissions.allow` entries, 19 `permissions.deny` entries, 1 `permissions.ask` entry. No `hooks` key, no `env` key, no other top-level keys.

| Section | Finding |
|---|---|
| `permissions.allow` | All 41 entries are LIVE — `bun run agent-browser …` patterns match the live VR workflow; `cafleet *`, `mise //cafleet*`, `mise //admin*`, `mise //:bun-install`, `mise //:uv-sync`, `mise //:slidev *`, `mise tasks`, `sleep *`, `uv run --frozen --group research *` all match active commands; the 12 `Skill(...)` entries match the 12 skills in `.claude-plugin/plugin.json` (11 cafleet-namespaced + 1 global `update-readme`). |
| `permissions.deny` | All 19 entries are LIVE and justified — they block direct tool invocation (`uv run pytest`, `uv run python -m`, `mise run *`, `sqlite3 *`) per `.claude/rules/commands.md`, block dangerous `agent-browser` operations (`eval`, `wait --load networkidle`, generic `open *`, `set *`), and block `bun install` without `--frozen-lockfile`. |
| `permissions.ask` | The single entry `Bash(cafleet * member exec *)` matches the bash-via-Director fallback protocol documented in `.claude/rules/bash-tool.md`. |
| Hooks | None defined. No action. |
| Env | None defined. The four `CAFLEET_*` env vars consumed by `cafleet.config.Settings` (DATABASE_URL, BROKER_HOST, BROKER_PORT, MAX_TEXT_LEN) are not in `settings.json` because they have safe defaults. |

No edits to `.claude/settings.json` are required.

### 5. Constraints from the user's clarifying answers

| Constraint | Source | Enforcement in this design |
|---|---|---|
| Audit prose lives **only** in this design doc. No "this was kept because…" comments in actual code/config/skill files. | Q1(b) answer. | §3 and §4 record findings only here; the implementation steps in §Implementation do not write justification comments into any file. |
| Zero deprecation notices after the merge. | Q3 answer + `~/.claude/rules/removal.md`. | Step 2 below deletes the file; no notices are added. |
| Do not rename or move `.claude/rules/`. | Brief hard constraint (53 historical design-doc references). | §3 explicitly forbids it; no implementation step touches the directory layout. |
| `skills/` restructure, `~/.claude/CLAUDE.md` edits, and `settings.json` edits are now in-scope only as audits. | User scope expansion during clarification round. | §4 records the audit; the implementation introduces zero edits to those surfaces because the audits found nothing actionable. |

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

### Step 1: Fix stale Bash-rule references in `skills/design-doc-execute/SKILL.md`

Per `.claude/rules/design-doc-numbering.md`, documentation surface changes come first. `SKILL.md` is a first-class documentation target.

All three sites carry the **identical** `old_string` and receive the **identical** `new_string`. Implementation hint: a single `Edit` call with `replace_all=true` resolves all three sites atomically and is line-number-shift safe. If the implementer prefers per-site review, the three tasks below are equivalent — the replacement is single-line to single-line, so line numbers remain stable across the three edits.

- [x] Edit `skills/design-doc-execute/SKILL.md:400` (Programmer spawn prompt) — replace `IMPORTANT: Read and follow rules/bash-command.md for all Bash commands.` with `IMPORTANT: Read and follow .claude/rules/bash-tool.md (CAFleet-member Bash protocol) and ~/.claude/rules/bash-command.md (general Bash hygiene) for all Bash commands.` <!-- completed: 2026-05-14T10:25 -->
- [x] Edit `skills/design-doc-execute/SKILL.md:445` (Tester spawn prompt) — replace the identical line with the same replacement string. <!-- completed: 2026-05-14T10:25 -->
- [x] Edit `skills/design-doc-execute/SKILL.md:491` (Verifier spawn prompt) — replace the identical line with the same replacement string. <!-- completed: 2026-05-14T10:25 -->


### Step 2: Delete `.claude/CLAUDE.md`

- [x] Delete the file `/home/himkt/work/himkt/cafleet/.claude/CLAUDE.md`. No surrounding file (root `CLAUDE.md`, any `SKILL.md`, any `docs/*.md`, any `README.md`) needs an accompanying edit — the §Specification pre-flight scan confirmed zero live references outside `design-docs/`. <!-- completed: 2026-05-14T10:27 -->

### Step 3: Post-implementation verification

Both checks scope to **tracked files only** via `git ls-files`. This cleanly excludes `.git/` internals, the gitignored member-spawn audit copies (`programmer.md`, `tester.md`, `verifier.md`, etc. — see the §4 ambient-audit-copies note), and the untracked drafter/reviewer scratch artifacts. Hits inside `design-docs/` are acceptable — those are historical and need not change.

- [ ] Run `git ls-files | xargs grep -l '\.claude/CLAUDE\.md'`. Expected result: zero matches outside `design-docs/*.md`. <!-- completed: -->
- [ ] Run `git ls-files | xargs grep -nE '(^|[^./~])rules/bash-command\.md'` (the leading-anchor regex matches the **bare** path only — it rejects the fully-qualified `.claude/rules/bash-command.md` and `~/.claude/rules/bash-command.md` forms introduced by Step 1). Expected result: zero matches outside `design-docs/*.md`. <!-- completed: -->


---

## Changelog

| Date | Changes |
|------|---------|
| 2026-05-13 | Initial draft. |
