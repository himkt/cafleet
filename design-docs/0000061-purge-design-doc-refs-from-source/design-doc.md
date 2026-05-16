# Purge Design-Doc References from Source, Tests, and User-Facing Docs

**Status**: Approved
**Progress**: 63/63 tasks complete
**Last Updated**: 2026-05-16

## Overview

Source code, tests, user-facing docs, and skill documentation in the cafleet repo contain dozens of citations to specific past design documents (`design 0000049 Surface 14`, `per design 0000060 §Spec 2`, `design-docs/0000032-robust-member-teardown/`, etc.). The global removal rule at `~/.claude/rules/removal.md` mandates that these surfaces describe the current state only — the git history and the design docs themselves are the historical record. This document scopes a one-time cleanup that strips every such citation outside of legitimate workflow paths.

## Success Criteria

- [x] Every design-doc citation outside the exempted paths (see § Exemptions) is removed from the repo.
- [x] After the cleanup, Step 7's verification grep (the full command with `--include` / `--exclude-dir` filters, reproduced verbatim in Step 7 below) returns matches only inside: `design-docs/`, the four design-doc-workflow skills (`skills/design-doc/`, `skills/design-doc-create/`, `skills/design-doc-execute/`, `skills/design-doc-interview/`), `CLAUDE.md` lines 21-25, `.claude/rules/design-doc-numbering.md`, and the three intentional resolver-example sites — `cafleet/tests/test_base_dir*.py` (the `0000099` synthetic-slug fixtures), `cafleet/src/cafleet/base_dir.py:213` (the docstring example), and `skills/base-dir/SKILL.md:25` (the CLI usage example).
- [x] `mise //cafleet:test` passes.
- [x] `mise //cafleet:lint` passes.
- [x] `mise //cafleet:typecheck` passes.

---

## Background

The removal rule, distilled from `~/.claude/rules/removal.md`:

> The git history and the design document (if any) are the historical record. Source code, user-facing docs, skills, and examples should describe only the current state.

Forbidden patterns include `# X was deprecated in design NNNN`, `(See §13 for the restoration plan)`, `**X deprecated**: see design 0000NNN`, and test docstrings shaped like `"""Tests for the cafleet --version flag (design 0000031)."""`. The cafleet repo has accumulated ~50 such citations as design docs landed one after another and authors recorded the provenance inline.

This cleanup is **one-time, sweeping, and not paired with a regression guard** — manual review at design-doc-execution time is the agreed-upon prevention mechanism.

---

## Specification

### Scope: in-scope vs. exempt

In-scope patterns (any of these in a non-exempt file is a cleanup target):

| Pattern | Example |
|:--|:--|
| `design NNNNNNN` | `design 0000049 Surface 14` |
| `design NNNNNNN §X` | `design 0000025 §B` |
| `per design NNNNNNN` | `per design 0000060 §Spec 2` |
| `see design NNNNNNN` | `see design 0000049 Concerns §1` |
| `added in design NNNNNNN` | `Added by design 0000049 Surface 1` |
| `deprecated in design NNNNNNN` | (none currently in tree, listed for completeness) |
| `design-docs/NNNNNNN-<slug>/` | `design-docs/0000032-robust-member-teardown/design-doc.md §4` |
| Markdown link `[design 0000NNN ...](../../design-docs/...)` | `[design 0000049 Surface 14](../../design-docs/0000049-token-reduction/design-doc.md)` |

### Exemptions

The following paths are exempt — they are the historical record itself, the workflow that creates the historical record, or the registry that lists the historical record:

| Exempt path | Reason |
|:--|:--|
| `design-docs/**` | The historical record. |
| `skills/design-doc/**` | Generic design-doc format skill. |
| `skills/design-doc-create/**` | Workflow skill for creating design docs. |
| `skills/design-doc-execute/**` | Workflow skill for executing design docs. |
| `skills/design-doc-interview/**` | Workflow skill for interviewing on design docs. |
| `.claude/rules/design-doc-numbering.md` | Rules for the design-doc workflow. |
| `CLAUDE.md` lines 21-25 (the design-doc registry; any other `design 0000NNN` mention elsewhere in `CLAUDE.md` is in-scope) | Intentional registry listing active/complete design docs. |
| `cafleet/tests/test_base_dir.py`, `cafleet/tests/test_base_dir_spawn_flow.py` (the `0000099` synthetic-slug fixtures) | Exercises the resolver's general behavior on the `design-docs/` bucket convention; `0000099` is a placeholder slug, not a citation to a real past design. |
| `cafleet/src/cafleet/base_dir.py:213` (the `design-docs/0000060-foo/design-doc.md` docstring example) | Same rationale — illustrates resolver behavior on the bucket convention. |
| `skills/base-dir/SKILL.md:25` (the `design-docs/0000060-skill-task-scoped-base-dir` resolver CLI-usage example) | Same rationale — illustrates resolver behavior on the bucket convention. |
| The OUTPUT PATH of this design doc itself (`design-docs/0000061-purge-design-doc-refs-from-source/design-doc.md`) | Trivially exempt; it describes its own cleanup. |

### Rewrite style

Every citation is stripped. The surrounding sentence is rewritten per site:

- **Surgical strip (style A)** — when the surrounding sentence carries useful information independent of the design citation, delete the citation only and keep the sentence. Example:
  - Before: `Default lowered from 80 to 30 (Surface 9, design 0000049): per-tick cost dominated Director token usage.`
  - After: `Default is 30 lines; per-tick cost would otherwise dominate Director token usage.`
- **Current-state rewrite (style B)** — when the sentence exists only to record the historical contrast, rewrite to describe current behavior only. Example:
  - Before: `This overrides the 0000014 deregister-first invariant — see design-docs/0000032-robust-member-teardown/design-doc.md §4.`
  - After: (delete the sentence; the preceding sentence already describes the current ordering)

### Test docstrings (the largest single category)

Many test modules open with a one-line docstring of the form:

```python
"""Tests for the ``cafleet --version`` global CLI flag (design 0000031)."""
```

Treatment: strip the `(design NNNNNN[§X])` parenthetical. The docstring becomes:

```python
"""Tests for the ``cafleet --version`` global CLI flag."""
```

When the docstring is multi-line and the design citation occupies a standalone line (e.g., `test_alembic_typed_columns_upgrade.py` opens with `"""Alembic migration tests for the Surface-14 typed-column upgrade (design 0000049 Step 2).`), drop the `(design …)` parenthetical and any `Surface N` / `Step N` shorthand that exists only to map back to the design doc.

### Inline `# (design NNNNNNN §X)` comments

Strip the parenthetical. When the comment contains genuine WHY information beyond the citation, keep the WHY. Example:

- Before: `# Three entries: root Director + Administrator + user-agent (design 0000026).`
- After: `# Three entries: root Director + Administrator + user-agent.`

### `.gitignore`

The three `.gitignore` comments retain their explanatory sentence; only the `(design 0000NNN)` parenthetical is removed.

Example:

- Before: `# Anchor file written by 'cafleet base-dir record' (design 0000055).`
- After: `# Anchor file written by 'cafleet base-dir record'.`

### Out of scope

- Adding a lint script, pre-commit hook, or CI check. Manual review at design-doc-execution time is the agreed-upon prevention mechanism.
- Touching the design-doc-workflow skills (`skills/design-doc*`).
- Touching the design-doc registry in `CLAUDE.md` lines 21-25.
- Replacing the synthetic `0000099` slugs in `test_base_dir*.py`, the `0000060` docstring example in `base_dir.py:213`, or the `0000060` resolver-example usage in `skills/base-dir/SKILL.md:25` — all three are exempt as resolver examples.
- Renaming Alembic migration files (their numeric prefixes are migration sequence numbers, not design-doc numbers).

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

Each step below operates on a coherent file group. Steps are independent — they can be applied in any order. The verification step at the end runs the grep query from Success Criteria and runs the project test/lint/typecheck commands.

### Step 1: Clean ARCHITECTURE.md

Apply per-site rewrites at the following lines (citations may shift after edits — use the pattern, not the line number, to locate the second pass).

- [x] `ARCHITECTURE.md:112` — strip `in design 0000049 Surface 14` from the `tasks.text` paragraph; keep the rationale about typed columns. Also rewrite the operator-backup filename example `registry.db.pre-0049.bak` to a design-doc-free form: `registry.db.pre-typed-columns.bak`. <!-- completed: 2026-05-16T11:55 -->
- [x] `ARCHITECTURE.md:164` — drop the trailing `This overrides the 0000014 deregister-first invariant — see design-docs/0000032-robust-member-teardown/design-doc.md §4.` sentence entirely. <!-- completed: 2026-05-16T11:55 -->
- [x] `ARCHITECTURE.md:168` — strip `, design 0000049 Surface 8` parenthetical from the `member list --activity` description; keep the description. <!-- completed: 2026-05-16T11:55 -->
- [x] `ARCHITECTURE.md:255` — rewrite the sentence to current-state form, replacing the historical-contrast verb `switched to` as well. `Auto-fire on every cafleet message send switched to tmux.send_inline_preview in design 0000049 Surface 15;` becomes `Auto-fire on every cafleet message send uses tmux.send_inline_preview;`. <!-- completed: 2026-05-16T11:55 -->
- [x] `ARCHITECTURE.md:257` — rename heading `## Token Reduction (design 0000049)` to `## Token Reduction`. <!-- completed: 2026-05-16T11:55 -->
- [x] `ARCHITECTURE.md:259` — rewrite the paragraph: drop the `Design 0000049 enumerates 19 ...` sentence and the trailing `the full list with measured savings lives in design-docs/0000049-token-reduction/design-doc.md` clause. Keep the opening sentence about per-byte cost; replace the dropped material with a one-line summary of the architectural-shape changes that ARCHITECTURE.md actually documents below the heading. <!-- completed: 2026-05-16T11:55 -->

### Step 2: Clean docs/spec/

- [x] `docs/spec/message-envelope.md:5` — rewrite the opening paragraph in current-state-only form. Remove `post-design-0049 shape` framing, the parenthetical legacy-shape inventory, and the trailing `See [design-docs/0000001-a2a-registry-broker/]...` clause. The result describes the current typed-column envelope as the only shape. <!-- completed: 2026-05-16T12:02 -->
- [x] `docs/spec/message-envelope.md:9` — strip `After [design 0000049 Surface 14](...)`; the sentence becomes a plain description of the current shape. <!-- completed: 2026-05-16T12:02 -->
- [x] `docs/spec/message-envelope.md:146-148` — delete both paragraphs at lines 146 and 148 (and the blank line 147 between them). Line 146 is the "previous nested envelope shape ... so design 0000049 dropped it entirely" paragraph (historical-contrast framing for a shape no longer in use); line 148 is the "historical record of the inherited convention lives in design-docs/0000001-..." pointer. The surrounding text already describes the current envelope as the only shape. <!-- completed: 2026-05-16T12:02 -->
- [x] `docs/spec/data-model.md:3` — strip `After [design 0000049 Surface 14](...)`; keep the description of the typed-column shape. <!-- completed: 2026-05-16T12:02 -->
- [x] `docs/spec/data-model.md:33` — strip `(per design 0000025)` from the Administrator insert step. <!-- completed: 2026-05-16T12:02 -->
- [x] `docs/spec/data-model.md:106` — strip `(design 0000049 Surface 14)` from the `tasks.text` row description; keep the Alembic revision reference (revision file names are not design-doc citations). <!-- completed: 2026-05-16T12:02 -->
- [x] `docs/spec/data-model.md:119` — strip `in design 0000049 Surface 14`; rewrite the sentence to describe the current state ("`task_json` is not part of the schema; the typed columns above ...") without the historical contrast. <!-- completed: 2026-05-16T12:02 -->
- [x] `docs/spec/cli-options.md:25,110,401,424,425` — strip `Added by design 0000049 Surface N` / `Added by design 0000049 Surface N for muscle-memory consistency...` clauses from each flag row. The "muscle-memory consistency with `tail -n`" rationale on `--tail` is kept; only the citation is dropped. <!-- completed: 2026-05-16T12:02 -->
- [x] `docs/spec/cli-options.md:31` — rewrite the introductory sentence about `--full` to drop `design 0000049 Concerns §1 documents the deliberate decision...`. The "single flag, not four `--full-X` variants" decision can stand as a stated design choice without a citation. <!-- completed: 2026-05-16T12:02 -->
- [x] `docs/spec/cli-options.md:94` — strip `in design 0000049 Surface 5`. The sentence becomes a description of the current ellipsis suffix. <!-- completed: 2026-05-16T12:02 -->
- [x] `docs/spec/cli-options.md:184` — strip `from design 0000025` from the `AdministratorProtectedError` reference. <!-- completed: 2026-05-16T12:02 -->
- [x] `docs/spec/cli-options.md:423,427` — strip `(Surface 9, design 0000049)` from the `--lines` default; rewrite the trailing per-tick rationale paragraph to drop `the per-tick cost analysis in design 0000049 (...)` framing — keep the description of why 30 is the chosen default. <!-- completed: 2026-05-16T12:02 -->

### Step 3: Clean .gitignore

- [x] `.gitignore:31` — strip `(design 0000055)` parenthetical. <!-- completed: 2026-05-16T12:04 -->
- [x] `.gitignore:33` — strip `(design 0000060)` parenthetical. <!-- completed: 2026-05-16T12:04 -->
- [x] `.gitignore:43` — strip `design 0000053: ` prefix from inside the parenthetical; keep the `${BASE}/<role>.md — one per spawned member-role per invocation` rationale. <!-- completed: 2026-05-16T12:04 -->

### Step 4: Clean cafleet source comments / docstrings

- [x] `cafleet/src/cafleet/base_dir.py:1` — rewrite the module docstring opening line. Remove `(design 0000055; task-scope branch design 0000060)` parenthetical; the docstring becomes `"""Base directory resolver for CAFleet."""` followed by the remaining content. <!-- completed: 2026-05-16T12:07 -->
- [x] `cafleet/src/cafleet/base_dir.py:369` — strip `(per §Specification 5 item 2 of design 0000055)`; keep any surrounding behavioral description. <!-- completed: 2026-05-16T12:07 -->
- [x] `cafleet/src/cafleet/cli.py:474` — rewrite the comment `# Task-scope branch: plain-text stderr per design 0000060 §Spec 2.` to `# Task-scope branch: plain-text stderr.` <!-- completed: 2026-05-16T12:07 -->
- [x] `cafleet/src/cafleet/cli.py:802` — rewrite the docstring line `Owns the five error surfaces from design 0000059 § 6: ...`; keep the list of error surfaces, drop the citation. <!-- completed: 2026-05-16T12:07 -->
- [x] `cafleet/src/cafleet/output.py:127,314` — strip `(design 0000049)` from both `Surface N` references inside `format_*` docstrings. The `Surface N` shorthand should also be dropped — without the design-doc anchor it carries no meaning. Rewrite each docstring to describe what the function does, not why. <!-- completed: 2026-05-16T12:07 -->
- [x] `cafleet/src/cafleet/broker.py:322` — strip `(design 0000026)`; keep the surrounding `placement` description. <!-- completed: 2026-05-16T12:07 -->
- [x] `cafleet/src/cafleet/broker.py:755` — drop the trailing `design 0000025 §E.` line. <!-- completed: 2026-05-16T12:07 -->
- [x] `cafleet/src/cafleet/tmux.py:180` — strip `(Surface 9, design 0000049)`; rewrite the docstring to state the current default and the per-tick-cost rationale without the citation. <!-- completed: 2026-05-16T12:07 -->
- [x] `cafleet/src/cafleet/alembic/versions/0009_drop_task_json_add_text.py:7` — rewrite the docstring opening line. `Surface 14 of design 0000049: replace the redundant ...` becomes `Replace the redundant Task.task_json JSON blob ...`. <!-- completed: 2026-05-16T12:07 -->
- [x] `cafleet/src/cafleet/alembic/versions/0009_drop_task_json_add_text.py:57` — rewrite the pre-flight check error message. `"Pre-flight check failed for design 0000049 Surface 14 migration: "` becomes `"Pre-flight check failed for tasks.task_json drop migration: "`. <!-- completed: 2026-05-16T12:07 -->

### Step 5: Clean cafleet test docstrings and inline comments

Each entry follows the docstring-stripping rule: drop the `(design NNNNNN[§X|Step N])` parenthetical, drop standalone `Surface N` / `Step N` shorthand that has no meaning without the design-doc anchor, and keep the descriptive part of the docstring.

- [x] `cafleet/tests/test_alembic_0002_upgrade.py:1` — strip `(design 0000015)`. <!-- completed: 2026-05-16T12:20 --> (file no longer exists)
- [x] `cafleet/tests/test_alembic_0006_upgrade.py:1` — strip `(design 0000025 §C)`. <!-- completed: 2026-05-16T12:20 --> (file no longer exists)
- [x] `cafleet/tests/test_alembic_typed_columns_upgrade.py:1` — rewrite the multi-line docstring. Drop `Surface-14 typed-column upgrade (design 0000049 Step 2)` framing; the docstring becomes a plain description of what the migration test covers. <!-- completed: 2026-05-16T12:20 --> (file no longer exists)
- [x] `cafleet/tests/test_broker_inline_preview.py:1` — drop the `Surface 15 — ` prefix and the `(design 0000049 Step 4)` parenthetical. <!-- completed: 2026-05-16T12:20 -->
- [x] `cafleet/tests/test_broker_registry.py:136,187,195,215,475,568` — strip `(design 0000026)` from each inline comment. <!-- completed: 2026-05-16T12:20 --> (line citations were already removed in an earlier refactor; line 3 multi-line `Per principle ... of design 0000061` framing cleaned)
- [x] `cafleet/tests/test_broker_typed_columns.py:1` — drop the `Post-Surface-14 ... (design 0000049 Step 2)` framing. The docstring becomes a plain description of the typed-column broker shape under test. <!-- completed: 2026-05-16T12:20 -->
- [x] `cafleet/tests/test_cli_claude_helpers.py:1` — strip `(design 0000046)`. <!-- completed: 2026-05-16T12:20 -->
- [x] `cafleet/tests/test_cli_client_command.py:1` — strip `(design 0000041 §A.1)`. <!-- completed: 2026-05-16T12:20 -->
- [x] `cafleet/tests/test_cli_compact_echo.py:1` — drop `Surface 3 — ` prefix and `(design 0000049 Step 6)` parenthetical. <!-- completed: 2026-05-16T12:20 -->
- [x] `cafleet/tests/test_cli_doctor.py:1` — strip `(design 0000032 §2)`. <!-- completed: 2026-05-16T12:20 -->
- [x] `cafleet/tests/test_cli_member.py:341,390` — rewrite the section-divider comment and the test docstring to drop the `design 0000046 §N` citations; keep the description of what each test asserts. <!-- completed: 2026-05-16T12:20 --> (line citations were already removed in an earlier refactor)
- [x] `cafleet/tests/test_cli_member_delete.py:525` — rewrite `Under design 0000032 §3, send_exit TmuxError is a hard exit-1.` to a current-state assertion: `send_exit TmuxError is a hard exit-1.` <!-- completed: 2026-05-16T12:20 -->
- [x] `cafleet/tests/test_cli_member_send_input.py:421` — strip `design 0000046 §5/§10` from the section-divider comment. <!-- completed: 2026-05-16T12:20 --> (citation was already removed in an earlier refactor; module-level `Per principle ... of design 0000061` framing cleaned)
- [x] `cafleet/tests/test_cli_pretty_flag.py:2` — drop `(design 0000049 Step 3)` parenthetical from the multi-line docstring. <!-- completed: 2026-05-16T12:20 -->
- [x] `cafleet/tests/test_cli_session_bootstrap.py:1,201,209,241` — strip the four `(design 0000026)` / `(design 0000046 §N)` citations. <!-- completed: 2026-05-16T12:20 -->
- [x] `cafleet/tests/test_cli_session_flag.py:1` — strip `(design 0000023)`. <!-- completed: 2026-05-16T12:20 -->
- [x] `cafleet/tests/test_cli_version.py:1` — strip `(design 0000031)`. <!-- completed: 2026-05-16T12:20 -->
- [x] `cafleet/tests/test_output_compact_formatters.py:2` — drop `(design 0000049 Step 6)`. <!-- completed: 2026-05-16T12:20 -->
- [x] `cafleet/tests/test_output_render_broadcast_summary.py:1` — drop the `Surface 4 — ... (design 0000049 Step 7)` framing. <!-- completed: 2026-05-16T12:20 -->
- [x] `cafleet/tests/test_output_render_task.py:1` — drop the `Surface 1 — ... (design 0000049 Step 3)` framing. <!-- completed: 2026-05-16T12:20 -->
- [x] `cafleet/tests/test_output_truncation_settings.py:1` — drop the `Surface 5 — ... (design 0000049 Step 8)` framing. <!-- completed: 2026-05-16T12:20 -->
- [x] `cafleet/tests/test_session_bootstrap.py:91,98` — strip `(design 0000046 §1, §5)` from both inline comments. <!-- completed: 2026-05-16T12:20 --> (line citations were already removed; line 3 multi-line `Per principle ... of design 0000061` framing cleaned)
- [x] `cafleet/tests/test_session_cli.py:261` — strip `(design 0000025 §B guard that the text path prints exactly one line)`; rewrite to a plain assertion comment. <!-- completed: 2026-05-16T12:20 --> (citation was already removed in an earlier refactor; module-level `Per principle ... of design 0000061` framing cleaned)
- [x] `cafleet/tests/test_tmux_send_inline_preview.py:1` — drop the `Surface 15 — ... (design 0000049 Step 4)` framing. <!-- completed: 2026-05-16T12:20 -->

### Step 6: Clean non-workflow skill files

- [x] `skills/cafleet/reference/legacy-flags.md:7` — strip `— see design 0000049 Concerns §1` from the `--full` description. The "Per-subcommand granular variants ... were considered and rejected" decision stands without a citation. <!-- completed: 2026-05-16T12:22 -->
- [x] `skills/cafleet/roles/member.md:76` — strip `After design 0000049 Surface 15,`; rewrite the sentence to describe the inline-preview auto-fire behavior in current-state form. <!-- completed: 2026-05-16T12:22 -->

`skills/base-dir/SKILL.md:25` is intentionally NOT in this step — see § Exemptions. The line is a CLI usage example illustrating resolver behavior on the `design-docs/` bucket, parallel to the `base_dir.py:213` docstring example.

### Step 7: Verify the cleanup is complete

- [x] Run `grep -rEn 'design 0[0-9]{6}|per design [0-9]|see design [0-9]|added in design [0-9]|deprecated in design [0-9]|design-docs/[0-9]{7}' --include='*.md' --include='*.py' --include='*.ts' --include='*.tsx' --include='*.js' --include='*.json' --include='*.toml' --include='*.sh' --include='.gitignore' --exclude-dir=design-docs --exclude-dir=.git --exclude-dir=node_modules --exclude-dir=.venv --exclude-dir=dist .` from the repo root. <!-- completed: 2026-05-16T12:25 -->
- [x] Confirm the residual matches are confined to: `CLAUDE.md` lines 21-25 (the registry), `.claude/rules/design-doc-numbering.md`, `skills/design-doc/`, `skills/design-doc-create/`, `skills/design-doc-execute/`, `skills/design-doc-interview/`, `cafleet/src/cafleet/base_dir.py:213` (the docstring example), `cafleet/tests/test_base_dir*.py` (the `0000099` synthetic-slug fixtures), and `skills/base-dir/SKILL.md:25` (the resolver CLI-usage example). <!-- completed: 2026-05-16T12:25 -->

### Step 8: Validate the project still builds and tests pass

- [x] `mise //cafleet:lint` passes. <!-- completed: 2026-05-16T12:27 -->
- [x] `mise //cafleet:typecheck` passes. <!-- completed: 2026-05-16T12:27 -->
- [x] `mise //cafleet:test` passes. <!-- completed: 2026-05-16T12:27 (614 passed) -->

### Step 9: Commit the cleanup

- [x] Stage every modified file and create a single conventional commit: `chore: purge design-doc citations from source, tests, and user-facing docs`. The design doc at `design-docs/0000061-purge-design-doc-refs-from-source/design-doc.md` is committed in the same commit (per project rule: design docs are committed alongside the implementation that delivers them). <!-- completed: 2026-05-16T12:30 --> <!-- completed: -->
