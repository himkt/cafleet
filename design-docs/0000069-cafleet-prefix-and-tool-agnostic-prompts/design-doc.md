# CAFleet Skill Prefix and Tool-Agnostic Prompt References

**Status**: Approved
**Progress**: 33/33 tasks complete
**Last Updated**: 2026-05-24

## Overview

Rename 11 of the 12 plugin-shipped skills to carry a `cafleet-` prefix so every CAFleet-shipped skill is uniquely identifiable by its bare name, and rewrite every in-repo reference to those skills so prompts use a single tool-agnostic phrasing (`` the `cafleet-foo` skill ``) instead of Claude Code's `/cafleet:foo` or codex's `$cafleet:foo` slash-prefix forms. The broker-CLI skill itself (`cafleet`) keeps its current name; the plugin name and marketplace name stay `cafleet`.

## Success Criteria

- [ ] Every CAFleet-shipped skill except the broker-CLI skill is registered under a `cafleet-`-prefixed name (`name:` frontmatter and `skills/` directory both match).
- [ ] `.claude-plugin/plugin.json` lists the 11 renamed skill directories; `.codex-plugin/plugin.json` is unchanged because it loads `./skills/` as a glob.
- [ ] `grep -rnE '(Skill\(|\$|(^|[^/[:alnum:]_])/)cafleet:[a-z-]+\b' skills/ README.md CONTRIBUTING.md docs/ cafleet/src/ cafleet/tests/ .claude/skills/ .claude/rules/` returns zero hits. The regex catches all three reference forms — `Skill(cafleet:foo)`, `$cafleet:foo`, and `/cafleet:foo` (anchored at start-of-line or after a non-`/`, non-alphanumeric, non-`_` character so that `mise //cafleet:<task>` package paths — where the preceding character is the first `/` — are excluded). The grep MUST first be smoke-tested against the known-bad fixture from Step 6 task 1 so a zero-hits result on the real codebase is a true pass and not a regex bug masquerading as cleanliness.

- [ ] Every member-spawn-prompt template (every `cafleet member create --prompt-file` payload and every spawn-prompt audit text) references cross-skills via the tool-agnostic phrase ``the `cafleet-foo` skill`` (or ``the `cafleet` skill`` for the broker-CLI skill).
- [ ] `mise //cafleet:test`, `mise //cafleet:lint`, and `mise //cafleet:typecheck` all pass.

---

## Background

CAFleet ships a `cafleet` plugin to Claude Code and codex (and now opencode). Inside each backend, individual skills become invocable by their declared `name:` (or, when `name:` is absent, by their directory name). Across Claude Code, codex, and opencode, two unrelated forms have grown up in the docs and in the prompts the Director injects into spawned members:

| Backend | User-facing slash form | In-doc tool-call form |
|---|---|---|
| Claude Code | `/cafleet:design-doc-create` | `Skill(cafleet:design-doc-create)` |
| codex | `$cafleet:design-doc-create` | (codex Reads SKILL.md files directly) |
| opencode | (no slash form) | (opencode Reads SKILL.md files directly) |

Two problems follow:

1. **Skill names collide with other plugins.** A skill named `design-doc-create` is at risk of name collision with any other plugin that ships a skill of the same name. Adding the `cafleet-` prefix to every skill guarantees a globally unique name, which lets every backend call the skill without needing to resolve a plugin namespace.
2. **Prompts are tool-specific.** The same spawn-prompt template gets injected into Claude Code, codex, and opencode members. When the template embeds `/cafleet:design-doc-create` (a Claude Code slash form) or `Skill(cafleet:base-dir)` (a Claude Code tool-call form), the codex / opencode member sees text it cannot execute. A single tool-agnostic phrasing — ``the `cafleet-foo` skill`` — works uniformly across every backend, because each backend already knows how to resolve a named skill into its own loader (Claude Code's `Skill` tool, codex's plugin auto-discovery, opencode's SKILL.md Read).

This design captures the rename and the prompt rewrite as a single coordinated change.

---

## Specification

### Skill rename map

The 12 plugin-shipped skills are mapped as follows. The broker-CLI skill keeps its name (its bare name is already `cafleet`, so the prefix would be redundant); every other skill gains the `cafleet-` prefix.

| Old skill name (and directory) | New skill name (and directory) |
|---|---|
| `cafleet` (`skills/cafleet/`) | `cafleet` (unchanged) |
| `agent-team-monitoring` (`skills/agent-team-monitoring/`) | `cafleet-agent-team-monitoring` (`skills/cafleet-agent-team-monitoring/`) |
| `agent-team-supervision` (`skills/agent-team-supervision/`) | `cafleet-agent-team-supervision` (`skills/cafleet-agent-team-supervision/`) |
| `base-dir` (`skills/base-dir/`) | `cafleet-base-dir` (`skills/cafleet-base-dir/`) |
| `create-figure` (`skills/create-figure/`) | `cafleet-create-figure` (`skills/cafleet-create-figure/`) |
| `design-doc` (`skills/design-doc/`) | `cafleet-design-doc` (`skills/cafleet-design-doc/`) |
| `design-doc-create` (`skills/design-doc-create/`) | `cafleet-design-doc-create` (`skills/cafleet-design-doc-create/`) |
| `design-doc-execute` (`skills/design-doc-execute/`) | `cafleet-design-doc-execute` (`skills/cafleet-design-doc-execute/`) |
| `design-doc-interview` (`skills/design-doc-interview/`) | `cafleet-design-doc-interview` (`skills/cafleet-design-doc-interview/`) |
| `my-slidev` (`skills/my-slidev/`) | `cafleet-my-slidev` (`skills/cafleet-my-slidev/`) |
| `research-presentation` (`skills/research-presentation/`) | `cafleet-research-presentation` (`skills/cafleet-research-presentation/`) |
| `research-report` (`skills/research-report/`) | `cafleet-research-report` (`skills/cafleet-research-report/`) |

Directory renames use `git mv` so history is preserved.

Every renamed skill's `SKILL.md` MUST carry an explicit `name:` frontmatter field set to the new name. The `cafleet` (broker-CLI) skill currently has no `name:` field (the directory name `cafleet` is the source of truth); leave it without a `name:` field — its directory remains `skills/cafleet/`, so the implicit name keeps working.

### Plugin manifests

- `.claude-plugin/plugin.json` — update the `skills` array to point at the 11 renamed directories. The broker-CLI entry (`./skills/cafleet`) stays. Order alphabetical by new name for diff-friendliness.
- `.claude-plugin/marketplace.json` — no change (plugin name stays `cafleet`).
- `.codex-plugin/plugin.json` — no change. The current value `"skills": "./skills/"` loads every subdirectory; the directory renames are picked up automatically.

### Reference transformation rules

Every textual reference to a CAFleet-shipped skill must be rewritten according to its **context**, not by a single global pattern. The classifier below covers every reference site found in the current repo.

| Context | Old form | New form | Rationale |
|---|---|---|---|
| **In-skill cross-reference inside `SKILL.md` / `roles/*.md` / `reference/*.md`** — prose mentioning another skill | `` `Skill(cafleet:base-dir)` `` | ``the `cafleet-base-dir` skill`` | Tool-agnostic phrasing per user's third ask. Applies even when the surrounding context is Claude Code-aware, because the same SKILL.md is consumed by codex and opencode members too. |
| **In-skill cross-reference to the broker-CLI skill** | `` `Skill(cafleet)` `` | ``the `cafleet` skill`` | Same rule; the broker-CLI skill is named `cafleet`. |
| **Pseudo-path reference into the broker skill's own files** (e.g. `Skill(cafleet:roles/member)` in `skills/agent-team-supervision/SKILL.md`) | `` `Skill(cafleet:roles/member)` `` | ``the `cafleet` skill's `roles/member.md` reference file`` | Not a real `Skill()` call; the form `cafleet:roles/member` is a documentation shorthand for "open `roles/member.md` inside the cafleet skill". Rewrite as a plain English file pointer. |
| **Spawn-prompt template literal** (text that becomes part of a `cafleet member create --prompt-file` payload, including `[INSERT abs BASE path the Director resolved via Skill(cafleet:base-dir)]`) | `Skill(cafleet:base-dir)` | ``the `cafleet-base-dir` skill`` | Spawn prompts are injected verbatim into the spawned member's first turn. Members may be Claude Code, codex, or opencode; only the tool-agnostic phrasing is safe across all three. |
| **User-facing slash-invocation example in `README.md` / `CONTRIBUTING.md`** (per Q5-sub (s2)) | `/cafleet:design-doc-create I want to ...` and `$cafleet:design-doc-create I want to ...` shown as two separate code-fenced blocks | A single tool-agnostic line such as ``Invoke the `cafleet-design-doc-create` skill with your one-line request — e.g., `I want to create a simple TUI calculator`.`` followed by a short trailing sentence pointing readers at each backend's own skill-invocation documentation for the literal slash / `$` / load syntax (Claude Code's `/skills`, codex's `/skills`, opencode's skill discovery). No per-tool fenced blocks. | The README is read by Claude Code, codex, and opencode users alike; one backend-neutral instruction plus a one-line "see your backend's docs for the literal invocation syntax" pointer keeps the README short without stranding users who do not know how their backend resolves a named skill. |
| **Claude Code `permissions.allow` JSON example in `README.md`** | `Skill(cafleet:design-doc-create)` | The plugin-namespaced long form `Skill(cafleet:cafleet-design-doc-create)` (broker entry stays `Skill(cafleet:cafleet)`). | The Claude Code permission matcher's behavior for the bare un-namespaced form on plugin-shipped skills is not documented; defaulting to the namespaced long form guarantees the entry matches the actual Skill tool call that the loader emits at runtime. Step 6 task 6 smoke-tests one renamed entry end-to-end — if it confirms the bare form also matches, a follow-up cleanup can shorten the README entries; until then the long form is the safe default. |
| **CLI error message and source-code `BASE_MARKER` literal** (`cafleet/src/cafleet/cli.py`, `cafleet/src/cafleet/base_dir.py`, `cafleet/tests/test_base_dir_spawn_flow.py`) | `see Skill(cafleet:base-dir).` and `[INSERT abs BASE path the Director resolved via Skill(cafleet:base-dir)]` | ``see the `cafleet-base-dir` skill.`` and ``[INSERT abs BASE path the Director resolved via the `cafleet-base-dir` skill]`` | These literals end up in spawn prompts and end-user-facing CLI error output; use the tool-agnostic phrasing. |
| **`docs/spec/cli-options.md` error-message row** | `see Skill(cafleet:base-dir).` | ``see the `cafleet-base-dir` skill.`` | The doc must match the source-code literal verbatim. |
| **`mise //cafleet:<task>` package paths** (every occurrence in `README.md`, `CONTRIBUTING.md`, `ARCHITECTURE.md`, `.claude/rules/commands.md`, `docs/spec/cli-options.md`) | (unchanged) | (unchanged) | These are mise package paths, not skill names. Out of scope. |
| **Opencode agent preset file `~/.opencode/agents/cafleet.md` and `OpencodeAgent`'s `--agent cafleet` flag** | (unchanged) | (unchanged) | This is an opencode-agent-name, not a CAFleet skill. Out of scope. |

### Canonical tool-agnostic phrasing

The single canonical form is:

> the `` `<skill-name>` `` skill

That is, an English determiner (`the`) — the backtick-quoted skill name — the literal word `skill`. The backticks render the skill name as inline code in Markdown, which makes the identifier visually distinct from surrounding prose and copy-paste-safe.

Examples:

- ``the `cafleet-base-dir` skill``
- ``the `cafleet-design-doc-create` skill``
- ``the `cafleet` skill`` (broker-CLI)

Where prose already uses a different determiner (e.g., "Load" or "loads"), preserve the determiner and substitute the phrase:

- Old: ``Load `Skill(cafleet:base-dir)` for the no-bypass write protocol.``
- New: ``Load the `cafleet-base-dir` skill for the no-bypass write protocol.``

Where a skill is referenced parenthetically:

- Old: ``loads Skill(cafleet:my-slidev) + Skill(cafleet:create-figure)``
- New: ``loads the `cafleet-my-slidev` and `cafleet-create-figure` skills``

### Out-of-scope items

The following are deliberately untouched:

- **Historical design docs under `design-docs/0000001/` through `design-docs/0000068/`.** These document the state at the time they were written; they are the canonical historical record per the project's removal rule. Leave every `cafleet:foo` / `Skill(cafleet:foo)` / `/cafleet:foo` reference inside `design-docs/*/design-doc.md`, `design-docs/*/question.md`, and `design-docs/*/prompts/` alone.
- **`mise //cafleet:<task>` package paths.** These are mise full-path task names, not skill names. Every occurrence in `README.md`, `CONTRIBUTING.md`, `ARCHITECTURE.md`, `.claude/rules/commands.md`, `.claude/skills/update-readme/SKILL.md`, and `docs/spec/cli-options.md` stays as-is.
- **Opencode agent preset file** (`~/.opencode/agents/cafleet.md`) and the `--agent cafleet` flag in `cafleet/src/cafleet/coding_agent/opencode.py`.
- **Plugin name and marketplace name** (both stay `cafleet`).
- **`ARCHITECTURE.md`** — verified to contain only `mise //cafleet:dev` (mise package paths, listed above) and no skill-name references. The project's `.claude/rules/design-doc-numbering.md` § *Implementation Order* requires ARCHITECTURE.md to stay in sync; here, "in sync" means "no change required", because ARCHITECTURE.md describes the architecture (broker, sessions, FastAPI app), not individual skill names.

### File inventory

The complete set of files that change in this design (counted from the snapshot taken during clarification):

| Bucket | Path | Number of references / lines touched |
|---|---|---|
| Plugin manifests | `.claude-plugin/plugin.json` | 11 path strings updated |
| Plugin manifests | `.codex-plugin/plugin.json` | 0 (glob load) |
| User-facing docs | `README.md` | permissions.allow block (12 `Skill()` lines) + "Real world usage" section (2 fenced blocks → 1 prose line) |
| User-facing docs | `CONTRIBUTING.md` | 4 lines (3 slash refs + 1 sentence about `$cafleet:` prefix family) |
| User-facing docs | `docs/spec/cli-options.md` | 1 error-message row |
| Skills — broker | `skills/cafleet/reference/director.md` | 2 references (lines 54, 62) |
| Skills — renamed (`skills/cafleet-<old>/`) | every `SKILL.md` and every file under `roles/`, `reference/`, etc. | All `Skill(cafleet:foo)` / `/cafleet:foo` / `cafleet:foo` references; counts per file: see pre-flight grep below. |
| Project-local skill | `.claude/skills/skill-author/SKILL.md` | 2 references (lines 10, 168 — `Skill(cafleet:base-dir)`) |
| Project-local rules | `.claude/rules/commands.md` | 3 lines (the "Skill artifact runners" table rows that mention `/cafleet:create-figure` and `/cafleet:research-presentation` — lines 50, 51, 52). The `mise //cafleet:<task>` entries in the same file are out of scope per the rule above. |
| Source code | `cafleet/src/cafleet/cli.py` | 1 line (791) |
| Source code | `cafleet/src/cafleet/base_dir.py` | 1 line (32 — `BASE_MARKER`) |
| Tests | `cafleet/tests/test_base_dir_spawn_flow.py` | 2 lines (42, 222) |

The implementer should regenerate the per-file count with `grep -rnE '(Skill\(|\$|(^|[^/[:alnum:]_])/)cafleet:[a-z-]+\b' skills/ README.md CONTRIBUTING.md docs/ cafleet/src/ cafleet/tests/ .claude/skills/ .claude/rules/` as a pre-flight pass; the success-criteria grep (same regex, same paths) is the post-flight check.

### Migration / compatibility note

There is no deprecation period. Per the project's removal rule (`~/.claude/rules/removal.md`), once the rename lands the repository must read as if the renamed skills had always carried the `cafleet-` prefix. No old-name aliases, no compatibility shims, no notices in user-facing docs about the prior names. Users who had the plugin installed before this change will pick up the new skill names automatically when they `/plugin update cafleet@cafleet` (Claude Code) or refresh the codex marketplace.

The 12 `Skill(cafleet:<old>)` entries in `~/.claude/settings.json` `permissions.allow` lists across users' machines will silently stop matching after the rename — every renamed skill becomes a new identifier (`cafleet:cafleet-<old>`). The README install block documents the new long-form entries; users with auto-installed allow lists will see permission prompts on first use of the renamed skills and approve them then. This is acceptable — no migration script is required. Step 6 task 6 smoke-tests the long-form permission entry end-to-end to confirm the matcher actually accepts it; if the smoke test reveals that the bare un-namespaced form `Skill(cafleet-<new>)` also matches, a follow-up cleanup can shorten the README entries, but that is out of scope for this design.

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

### Step 1: User-facing docs

- [x] Update `README.md` install-section `permissions.allow` JSON block: replace each `Skill(cafleet:<old>)` line with the plugin-namespaced long form `Skill(cafleet:cafleet-<new>)`; the broker entry becomes `Skill(cafleet:cafleet)`; preserve alphabetical order. <!-- completed: 2026-05-24T04:24 -->
- [x] Update `README.md` "Real world usage; Design-doc-driven development" section: collapse the two fenced blocks (`/cafleet:design-doc-create ...` and `$cafleet:design-doc-create ...`) into a single tool-agnostic prose line that names the `cafleet-design-doc-create` skill using the canonical phrasing, followed by a one-line trailing sentence telling readers to consult their backend's skill documentation for the literal invocation syntax. <!-- completed: 2026-05-24T04:24 -->
- [x] Update `CONTRIBUTING.md`: replace each `/cafleet:design-doc-*` slash reference with the canonical tool-agnostic phrasing; rewrite the trailing sentence about `$cafleet:design-doc-*` so it no longer documents per-tool slash prefixes. <!-- completed: 2026-05-24T04:24 -->
- [x] Update `docs/spec/cli-options.md` error-message row to use ``see the `cafleet-base-dir` skill.``. <!-- completed: 2026-05-24T04:24 -->
- [x] Update `.claude/rules/commands.md` "Skill artifact runners" table: rewrite the three rows that reference `/cafleet:create-figure` and `/cafleet:research-presentation` (lines 50, 51, 52) using the canonical tool-agnostic phrasing. The `mise //cafleet:<task>` rows in the same file stay unchanged. <!-- completed: 2026-05-24T04:24 -->

### Step 2: Plugin manifests

- [x] Update `.claude-plugin/plugin.json`: rewrite the `skills` array to list `./skills/cafleet`, `./skills/cafleet-agent-team-monitoring`, `./skills/cafleet-agent-team-supervision`, `./skills/cafleet-base-dir`, `./skills/cafleet-create-figure`, `./skills/cafleet-design-doc`, `./skills/cafleet-design-doc-create`, `./skills/cafleet-design-doc-execute`, `./skills/cafleet-design-doc-interview`, `./skills/cafleet-my-slidev`, `./skills/cafleet-research-presentation`, `./skills/cafleet-research-report` (alphabetical). <!-- completed: 2026-05-24T04:26 -->
- [x] Confirm `.codex-plugin/plugin.json` requires no change (still `"skills": "./skills/"`). <!-- completed: 2026-05-24T04:26 -->
- [x] Confirm `.claude-plugin/marketplace.json` requires no change (plugin name stays `cafleet`). <!-- completed: 2026-05-24T04:26 -->

### Step 3: Skill directory renames + frontmatter

Each substep uses `git mv` so directory history is preserved. After the move, edit the SKILL.md frontmatter in the new location to set an explicit `name:` field.

- [x] `git mv skills/agent-team-monitoring skills/cafleet-agent-team-monitoring` and set `name: cafleet-agent-team-monitoring` in the new SKILL.md frontmatter. <!-- completed: 2026-05-24T04:33 -->
- [x] `git mv skills/agent-team-supervision skills/cafleet-agent-team-supervision` and set `name: cafleet-agent-team-supervision`. <!-- completed: 2026-05-24T04:33 -->
- [x] `git mv skills/base-dir skills/cafleet-base-dir` and set `name: cafleet-base-dir`. <!-- completed: 2026-05-24T04:33 -->
- [x] `git mv skills/create-figure skills/cafleet-create-figure` and set `name: cafleet-create-figure`. <!-- completed: 2026-05-24T04:33 -->
- [x] `git mv skills/design-doc skills/cafleet-design-doc` and set `name: cafleet-design-doc`. <!-- completed: 2026-05-24T04:33 -->
- [x] `git mv skills/design-doc-create skills/cafleet-design-doc-create` and set `name: cafleet-design-doc-create`. <!-- completed: 2026-05-24T04:33 -->
- [x] `git mv skills/design-doc-execute skills/cafleet-design-doc-execute` and set `name: cafleet-design-doc-execute`. <!-- completed: 2026-05-24T04:33 -->
- [x] `git mv skills/design-doc-interview skills/cafleet-design-doc-interview` and set `name: cafleet-design-doc-interview`. <!-- completed: 2026-05-24T04:33 -->
- [x] `git mv skills/my-slidev skills/cafleet-my-slidev` and set `name: cafleet-my-slidev`. <!-- completed: 2026-05-24T04:33 -->
- [x] `git mv skills/research-presentation skills/cafleet-research-presentation` and set `name: cafleet-research-presentation`. <!-- completed: 2026-05-24T04:33 -->
- [x] `git mv skills/research-report skills/cafleet-research-report` and set `name: cafleet-research-report`. <!-- completed: 2026-05-24T04:33 -->

(The `cafleet` broker-CLI skill has no directory move and no frontmatter change.)

### Step 4: Rewrite in-skill references

Apply the *Reference transformation rules* table from the Specification to every file under `skills/`. The substitutions are file-by-file; the implementer should use `grep -rnE 'cafleet:[a-z-]+' skills/` to confirm zero residue after the pass. The transformation is mechanical for the "in-skill cross-reference" and "spawn-prompt template literal" rows; the "pseudo-path reference" row (`Skill(cafleet:roles/member)` in `skills/cafleet-agent-team-supervision/SKILL.md` after the rename) requires the manual rewrite to a plain-English file pointer.

- [x] Rewrite every `Skill(cafleet:<X>)` cross-reference inside every file under `skills/` to ``the `cafleet-<X>` skill`` (or ``the `cafleet` skill`` when `<X>` is empty). Skip none. <!-- completed: 2026-05-24T05:12 -->
- [x] Rewrite every `/cafleet:<X>` slash reference inside every file under `skills/` to ``the `cafleet-<X>` skill``. Skip none. <!-- completed: 2026-05-24T05:12 -->
- [x] Discover every spawn-prompt template literal that embeds `Skill(cafleet:base-dir)` with `grep -rnE 'Skill\(cafleet:base-dir\)' skills/ .claude/skills/` and rewrite every match to ``the `cafleet-base-dir` skill``. The grep is the source of truth for which files this task touches — do not maintain a parallel enumeration. <!-- completed: 2026-05-24T05:12 -->

- [x] Rewrite `Skill(cafleet:roles/member)` in the renamed `skills/cafleet-agent-team-supervision/SKILL.md` to a plain-English file pointer: ``the `cafleet` skill's `roles/member.md` reference file``. <!-- completed: 2026-05-24T05:12 -->

### Step 5: Source code and tests

- [x] Update `cafleet/src/cafleet/base_dir.py`: change the `BASE_MARKER` literal on line 32 from ``"[INSERT abs BASE path the Director resolved via Skill(cafleet:base-dir)]"`` to ``"[INSERT abs BASE path the Director resolved via the `cafleet-base-dir` skill]"`` (literal backticks inside the string). <!-- completed: 2026-05-24T05:18 -->
- [x] Update `cafleet/src/cafleet/cli.py`: change the `--prompt-file` relative-path error message on line 791 from ``"see Skill(cafleet:base-dir)."`` to ``"see the `cafleet-base-dir` skill."``. <!-- completed: 2026-05-24T05:18 -->
- [x] Update `cafleet/tests/test_base_dir_spawn_flow.py`: change the `BASE_MARKER` constant on line 42 and the embedded literal on line 222 to match the new `base_dir.py` value verbatim. <!-- completed: 2026-05-24T05:18 -->

### Step 6: Verification

- [ ] Smoke-test the verification regex against a known-bad fixture before running it on the real codebase. Create a temporary file containing exactly these seven lines (one per shape, with all three positional contexts for the slash form represented):

      ```
      Skill(cafleet:base-dir) in a Skill call
      /cafleet:design-doc-create at start of line
        /cafleet:foo after leading whitespace
      see `/cafleet:bar` for details (backtick-quoted slash form)
      $cafleet:design-doc-create codex form
      mise //cafleet:test mise package control
      no skill reference here (negative control)
      ```

      Then run `grep -nE '(Skill\(|\$|(^|[^/[:alnum:]_])/)cafleet:[a-z-]+\b' <fixture>`. Assert exactly **five** hits (1 `Skill(`, 3 `/cafleet:` across all three positional contexts, 1 `$cafleet:`) and zero hits on both control lines (`mise //cafleet:test` and the negative control). If any count disagrees, the regex is broken in a way Step 6 task 2 would not catch; fix the regex before treating the real-codebase pass as a true pass. <!-- completed: -->

- [ ] Run `grep -rnE '(Skill\(|\$|(^|[^/[:alnum:]_])/)cafleet:[a-z-]+\b' skills/ README.md CONTRIBUTING.md docs/ cafleet/src/ cafleet/tests/ .claude/skills/ .claude/rules/`; assert zero matches. <!-- completed: -->
- [ ] Run `mise //cafleet:test`; assert pass. <!-- completed: -->

COMMENT(verifier): mise //cafleet:test reports 707 passed, 1 failed. The failure is `tests/test_base_dir.py::test_cli_resolve_task_name_outside_git_repo_exits_1_no_json` at line 540 (`assert result.exit_code != 0` got `0`). Out-of-scope for this design doc: (a) `tests/test_base_dir.py` is unmodified on this branch (`git log main..HEAD -- cafleet/tests/test_base_dir.py` is empty); (b) the Step 5 commit `4d86c4b` changes `cafleet/src/cafleet/base_dir.py` lines 31-33 only — a pure string-literal swap of `_BASE_INSERT_MARKER`, no resolver-logic change; (c) the only ways `exit_code` could be `0` are an unexpected `.git` ancestor up the pytest `/tmp` parent chain in this sandbox, or pre-existing breakage on main. Suggested follow-up (outside this design): run the same test on `main` to classify as pre-existing vs sandbox-specific; if sandbox-specific, fix the test's `_infer_repo_root` walk to bottom-cap at `tmp_path` rather than `/`.

- [ ] Run `mise //cafleet:lint`; assert pass. <!-- completed: -->

COMMENT(verifier): mise //cafleet:lint reports 1 error in `cafleet/tests/test_skill_rename_step5_markers.py:18` — ruff `I001 [*] Import block is un-sorted or un-formatted`. The file was added in Step 5 commit `ce754fc`. Auto-fixable via `ruff check --fix`. Suggested fix: collapse the two `from`-import groups (`from cafleet.base_dir ...`, `from cafleet.cli ...`, and `from tests.test_base_dir_spawn_flow ...`) per the project's isort/ruff first-party grouping, or run `ruff --fix` directly. This is real residue from this design doc — the failing file is a Step-5-introduced test.

- [ ] Run `mise //cafleet:typecheck`; assert pass. <!-- completed: -->
- [ ] Spot-check one Claude Code skill load (`cafleet-design-doc-create`) and one codex skill load (`cafleet-design-doc-create` via codex's plugin loader) to confirm both backends discover the renamed skill by its new name. <!-- completed: -->
- [ ] Smoke-test the README `permissions.allow` long-form entry end-to-end: in a fresh Claude Code session with only `Skill(cafleet:cafleet-design-doc-create)` in `~/.claude/settings.json` `permissions.allow`, invoke the `cafleet-design-doc-create` skill and confirm the matcher accepts it without a permission prompt. If this passes, optionally re-run with the bare `Skill(cafleet-design-doc-create)` form to record whether the short form also matches; the README entries stay on the long form regardless. <!-- completed: -->
