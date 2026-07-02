# Unified `--text` / `--text-file` Input and `send-input` Removal

**Status**: Approved
**Progress**: 35/35 tasks complete
**Last Updated**: 2026-07-03

## Overview

Give every text-body CLI command (`message send`, `message broadcast`, `member nudge`, `member create`) an identical `--text` / `--text-file` input pair backed by one shared reader helper, so long or multi-line bodies bypass the shell's `ARG_MAX` limit (GitHub issue #153). In the same change, delete the structurally-unreachable `member send-input` command and its supporting tmux keystroke helpers.

## Success Criteria

- [x] All four text-body commands accept exactly `--text <str>` and `--text-file <path>`, mutually exclusive, exactly one required.
- [x] `--text-file` accepts an absolute or CWD-relative path, reads UTF-8, and treats `-` as "read the whole body from stdin".
- [x] A single shared helper is the sole source of truth for mutual-exclusivity, path resolution, UTF-8 decoding, stdin handling, empty-body rejection, and the associated error strings.
- [x] `member create` exposes the same pair as the other three: `--prompt-file` is hard-renamed to `--text-file` (no alias), `--text` is added, and the positional prompt argument is removed.
- [x] `member send-input` and the `send_choice_key` / `send_freetext_and_submit` tmux helpers no longer exist anywhere in source, tests, docs, SPEC, or skills.
- [x] `mise //cafleet:test`, `mise //cafleet:lint`, and `mise //cafleet:typecheck` pass.

---

## Background

`message send --text` (and its siblings) pass the body to the process as a single positional shell argument, so a large body fails at the OS `ARG_MAX` boundary. The full body is not the problem downstream — the broker persists it whole in SQLite and delivers it whole; only the ~200-char inline **preview** is truncated (display-only, design 0000043). The fix is an input path that never puts the body on the command line: read it from a file or from stdin.

The repository already has exactly one such reader: `cafleet/src/cafleet/cli/_prompt.py` (`read_prompt_file` / `resolve_prompt`), used only by `member create` to read the **spawn prompt**. This design generalizes that reader into one helper shared by all four commands.

**One `--prompt-file` in the codebase.** In this repository `member create` *is* the agent-spawn command; its `--prompt-file` / positional prompt argument feed `resolve_prompt`, which additionally runs `str.format` placeholder substitution (`{fleet_id}`, `{agent_id}`, `{director_agent_id}`, `{coding_agent}`). There is no separate `agent spawn --prompt-file` flag. Renaming `member create`'s `--prompt-file` to `--text-file` is therefore the *only* `--prompt-file` rename in the repo, and it carries every skill spawn recipe with it. (Some skill/doc pages skin the installed `member *` surface as `agent spawn` / `pane input` — legacy naming from design 0000111; the removal/rename covers **both** surface names wherever they appear.)

**`send-input` is dead code.** Every member spawns with unconditional auto-approve flags — claude `--permission-mode dontAsk` (which *denies* the `AskUserQuestion` tool outright), codex `--ask-for-approval never --sandbox workspace-write`, opencode `--agent cafleet` — and codex/opencode have no in-pane prompt surface at all. The 4-option `AskUserQuestion` pane frame that `send-input` keystrokes into is therefore unreachable by any member, so the command, its `--choice` / `--freetext` options, and the two tmux helpers that back it are unreachable code.

**Breaking change, accepted.** Removing `send-input`, removing `--prompt-file`, and removing `member create`'s positional argument are breaking CLI changes. The project is pre-1.0 (currently `0.14.0`), so breaking changes are acceptable and are called out here rather than aliased.

---

## Specification

### 1. The shared text-input helper

Refactor `cafleet/src/cafleet/cli/_prompt.py` into a generic text-input reader (rename the module to `_text_input.py`). It owns *all* body resolution and validation for the four commands; the spawn-prompt placeholder substitution stays a separate, `member create`-only concern.

```python
def read_text_input(text: str | None, text_file: str | None) -> str:
    """Resolve a command body from the --text / --text-file pair.

    Enforces exactly-one-of, resolves the file path (absolute, or relative to
    CWD; '-' reads all of stdin), decodes UTF-8, and rejects an empty or
    whitespace-only body. Returns the body verbatim (no stripping).
    """
```

Resolution and validation, in order:

| Condition | Behavior |
|-----------|----------|
| Neither `--text` nor `--text-file` given | `UsageError`: `Provide exactly one of --text or --text-file.` |
| Both given | `UsageError`: `--text and --text-file are mutually exclusive.` |
| `--text <s>` | body = `s` |
| `--text-file -` | body = all of stdin (`sys.stdin.buffer.read().decode("utf-8")`) |
| `--text-file <path>` | body = `Path(path).read_bytes().decode("utf-8")` — absolute path used as-is; relative path resolved against CWD |
| `--text` value empty/whitespace-only | `UsageError`: `text may not be empty.` |
| `--text-file <path>` file empty/whitespace-only | `ClickException`: `--text-file <path>: file is empty.` |
| `--text-file -` stdin empty/whitespace-only | `ClickException`: `--text-file -: stdin is empty.` |

File error surfaces (all `ClickException`, keyed on `--text-file`; ride the `read_bytes()` exception surface with no `is_file()` pre-check, so a permission failure lands correctly):

| Failure | Message |
|---------|---------|
| Missing / non-regular file (`FileNotFoundError`, `IsADirectoryError`) | `--text-file <path>: file does not exist or is not a regular file.` |
| Not readable (`PermissionError`, other `OSError`) | `--text-file <path>: file is not readable.` |
| Invalid UTF-8 (`UnicodeDecodeError`) | `--text-file <path>: file is not valid UTF-8.` |

Notes:
- Read via `read_bytes().decode("utf-8")` (not `read_text()`) so universal-newline translation does not collapse CRLF/CR — the body reaches the caller byte-for-byte.
- The absolute-path requirement of the old `read_prompt_file` is **relaxed**: relative paths are now accepted and resolved against CWD.
- Empty-body rejection is now **uniform** across all four commands and across inline/file/stdin. This tightens `message send` and `message broadcast`, which previously accepted an empty `--text` — an accepted breaking change.
- `--text-file -` is for piped or redirected stdin. `sys.stdin.buffer.read()` reads until EOF; on an interactive TTY with no pipe it blocks until the user sends EOF (Ctrl-D). That blocking is acceptable — it is the standard `-`-means-stdin CLI convention — so no `isatty()` guard is added.

### 2. Spawn-prompt placeholder substitution (`member create` only)

The `.format` substitution that only `member create` needs stays in the helper module as its own function, applied to the body *after* `read_text_input` returns:

```python
def substitute_spawn_placeholders(
    body: str, *, fleet_id: int, agent_id: int,
    director_agent_id: int, coding_agent: str | None,
) -> str:
    """Run str.format on a spawn prompt. KeyError -> UsageError listing the
    supported placeholders; ValueError/IndexError/AttributeError -> malformed-
    prompt UsageError. (Unchanged from resolve_prompt's substitution behavior.)"""
```

`member create` calls `read_text_input(text, text_file)` then `substitute_spawn_placeholders(...)`. The other three commands call `read_text_input` only and never touch `.format`. The old `resolve_prompt` (which combined reading + substitution + the default-template fallback) is deleted, and so is the `MEMBER_PROMPT_TEMPLATE` constant (see §3).

### 3. Per-command surface

| Command | Before | After |
|---------|--------|-------|
| `message send` | `--text` (required) | `--text` / `--text-file` (xor, one required) |
| `message broadcast` | `--text` (required) | `--text` / `--text-file` (xor, one required) |
| `member nudge` | `--text` (required) | `--text` / `--text-file` (xor, one required) |
| `member create` | `--prompt-file` + positional `prompt_argv` (xor; default template if both omitted) | `--text` / `--text-file` (xor, one required); positional removed; no default template |

Every command declares both options with `default=None` (not Click-`required`); the helper enforces xor + required, mirroring the existing `member send-input` choice/freetext pattern. Each command replaces its ad-hoc body handling with a `read_text_input` call:
- `message send` / `message broadcast`: drop `required=True` on `--text`, add `--text-file`, resolve via the helper, pass the body to `broker.send_message` / `broker.broadcast_message`.
- `member nudge`: add `--text-file`, resolve via the helper (its existing `if not text.strip()` check is subsumed by the helper's empty-body rejection).
- `member create`: rename `--prompt-file` → `--text-file`, add `--text`, remove the `@click.argument("prompt_argv", nargs=-1)` and the `prompt_file`/`prompt_argv` mutual-exclusion guard, resolve via the helper, then apply `substitute_spawn_placeholders`. Removing the default template means a bare `member create` with neither flag is now a usage error.

**`member create` inline spawns.** The old inline one-line spawn form `member create ... -- "<prompt>"` becomes `member create ... --text "<prompt>"`; large templated prompts use `--text-file <path>` (unchanged rationale — the `tmux split-window` "command too long" cliff). Placeholder substitution applies to whichever form is used.

### 4. `send-input` removal surface

Delete every occurrence — no deprecation notice anywhere (per `.claude/rules/removal.md`). Designs 0000027 / 0000033 / 0000098 remain the historical record and are **not** edited.

| Area | What to remove |
|------|----------------|
| Command | `member_send_input` (`cafleet/src/cafleet/cli/member.py`, ~lines 535–605) incl. `--choice` / `--freetext` options |
| Source recovery hint | The `member delete` timeout handler (`cafleet/src/cafleet/cli/member.py` ~line 412) prints `answer any prompt with \`cafleet member send-input\`` as a stuck-pane recovery step — drop that clause so the hint reads `inspect with \`cafleet member capture\`, then re-run \`cafleet member delete\` … or \`--force\``. Its assertions live in `test_member_delete.py` (see Step 7). |
| tmux helpers | `send_choice_key` and `send_freetext_and_submit` in `cafleet/src/cafleet/multiplexer/tmux.py` (impl) and `cafleet/src/cafleet/multiplexer/base.py` (protocol) — verified: **no callers** outside `member_send_input` |
| Tests | `cafleet/tests/cli/test_member_send_input.py`; `cafleet/tests/multiplexer/test_tmux_send_helpers.py`; the send-input / send-helper slices of `test_tmux.py` (the Esc-first-safeguard assertions for these two helpers), `test_help_budget.py`, and `test_member_delete.py` |
| SPEC.md | `member send-input` option spec; the `send_choice_key` / `send_freetext_and_submit` protocol specs and method-list entries; the Esc-first note for these helpers; the `pane input` future-note |
| docs/spec/cli-options.md | the `### `pane input`` section (~lines 666–733), its error-message rows (~926–929), and its subcommand-summary line |
| docs/how-to/monitor-and-recover.md | the `pane input --choice/--freetext` recovery-ladder rung (renumber the ladder) |
| skills | `skills/cafleet/reference/director.md` § *Answering a member's relayed question* (strip the pane-keystroke relay); the `send-input` / 4-option-frame relay Note rows in `skills/cafleet/reference/coding-agent/claude.md`; the member-pane decision-relay references in `skills/cafleet/reference/supervision.md`; any `send-input` mention in `skills/cafleet/SKILL.md` |
| settings.json | none — verified no `permissions.allow`/`deny` pattern references `send-input` / `--choice` / `--freetext` |

**What is preserved.** The removal targets only the member-pane **decision-relay** (a Director keystroking an answer into a member's `AskUserQuestion` frame). The `{decision_surface}` = `AskUserQuestion` path by which the **Director asks the user** is intact: a member with a question still sends it to the Director via `cafleet message send`, the Director asks the user via `AskUserQuestion`, and the answer flows back to the member as a normal `cafleet message send` (not a pane keystroke). The claude overlay's question-shape Note row (Director→user) stays; only its two relay rows are removed. codex/opencode overlays already state "No in-pane prompt" and need no change.

### 5. Explicitly out of scope

| Item | Reason |
|------|--------|
| The ~200-char broker message preview truncation (design 0000043) | Display-only; the full body is always persisted in SQLite and delivered in full. It is not a payload limit, so `--text-file` already solves the real problem. No behavior change. |
| `POST /api/messages/send` | Already accepts long bodies via the JSON request body (no `ARG_MAX`). No change. |
| `member exec` | A separate single-line shell-dispatch control input, unaffected. |
| The `member` vs `agent`/`pane` command-name skin | A pre-existing naming duality (design 0000111 territory); this design renames/removes flags under whatever name each file uses, without reconciling the skin. |

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> Documentation-first: all doc/skill surfaces are updated before code (per `.claude/rules/documentation-maintenance.md`).

### Step 1: Concepts & user docs

- [x] `docs/concepts/member-lifecycle.md` — change the spawn-prompt line ("supplied inline (`-- "<prompt>"`) or via `--prompt-file`") to `--text` (inline) / `--text-file` (file or `-` stdin). <!-- completed: 2026-07-02T14:11 -->
- [x] `docs/spec/cli-options.md` — replace the `### agent spawn` `--prompt-file` + positional rows (~lines 493–571) with `--text` / `--text-file`; document xor + exactly-one-required, absolute/CWD-relative paths, `-` stdin, and the empty-body rejection. This also removes the "Neither `--prompt-file` nor positional prompt | The built-in default prompt template, verbatim." state row (~line 519) — the neither-flag case is now a usage error, so that phrase and its "default template" wording go. <!-- completed: 2026-07-02T14:11 -->
- [x] `docs/spec/cli-options.md` — add `--text-file` to `message send`, `message broadcast`, and `member nudge`; add the shared text-input error-message rows (mutual-exclusivity, empty, file-not-found/unreadable/non-UTF-8, empty-stdin). <!-- completed: 2026-07-02T14:11 -->
- [x] `docs/spec/cli-options.md` — remove the `### `pane input`` section (~666–733), its error rows (~926–929), and its subcommand-summary line. <!-- completed: 2026-07-02T14:11 -->
- [x] `docs/how-to/monitor-and-recover.md` — remove the `pane input --choice/--freetext` recovery-ladder rung and renumber the ladder; update the monitor-spawn example from `--prompt-file` to `--text-file`. <!-- completed: 2026-07-02T14:11 -->
- [x] `docs/concepts/token-reduction.md` — the "Slim member spawn prompt" row cites "The default spawn-prompt template is ~60 tokens"; reword to drop the removed default-template reference (spawn prompts are now always explicitly supplied via `--text` / `--text-file`; the slim-identity-via-env-vars point stays). <!-- completed: 2026-07-02T14:11 -->
- [x] `docs/concepts/bash-routing.md` — the "The default spawn-prompt template tells the member … Bash tool" sentence references the removed default template; reword so it does not name a `member create` default that no longer exists. <!-- completed: 2026-07-02T14:11 -->

### Step 2: README.md & SPEC.md

- [x] `README.md` — reflect the unified `--text` / `--text-file` pair on the four commands, the removal of `send-input`, and the removal of `--prompt-file` / the positional prompt argument. <!-- completed: 2026-07-02T14:28 -->
- [x] `SPEC.md` — update the spawn-prompt-resolution and `member create` surfaces to `--text` / `--text-file` (xor, required; no default template), including the placeholder-substitution note. <!-- completed: 2026-07-02T14:28 -->
- [x] `SPEC.md` — add `--text-file` and the xor/empty semantics to `message send`, `message broadcast`, and `member nudge`. <!-- completed: 2026-07-02T14:28 -->
- [x] `SPEC.md` — remove the `member send-input` option spec, the `send_choice_key` / `send_freetext_and_submit` protocol specs + method-list entries + Esc-first note, and the `pane input` future-note. <!-- completed: 2026-07-02T14:28 -->

### Step 3: Skills

- [x] Rename `--prompt-file` → `--text-file` in every spawn recipe: `skills/cafleet-design-doc/create/create.md`, `execute/execute.md`, `interview/interview.md`, `skills/cafleet-research/report/report.md`, `presentation/presentation.md`. <!-- completed: 2026-07-02T14:45 -->
- [x] `skills/cafleet/reference/director.md` — § Member Create table (`--prompt-file`/positional rows → `--text`/`--text-file`; drop the default-template clause), the spawn-size-limit guidance (line ~100), and the audit-file `--prompt-file` references → `--text-file`. <!-- completed: 2026-07-02T14:45 -->
- [x] `skills/cafleet/reference/director.md` — § *Answering a member's relayed question*: strip the pane-keystroke-relay clause; state the answer returns via `cafleet message send`. <!-- completed: 2026-07-02T14:45 -->
- [x] `skills/cafleet/reference/supervision.md` — `--prompt-file` → `--text-file` (lines ~88, 121, 190, 191); remove the member-pane decision-relay references (lines ~154, 168–172), keeping the Director→user `{decision_surface}` escalation. <!-- completed: 2026-07-02T14:45 -->
- [x] `skills/cafleet/reference/coding-agent/claude.md` — remove the two `{decision_surface}` relay Note rows (pane-capture-for-frame; `pane input --choice/--freetext` relay); keep the question-shape row. <!-- completed: 2026-07-02T14:45 -->
- [x] `skills/cafleet/SKILL.md` — remove any `send-input` / "Answering a member's AskUserQuestion prompt" references; keep § *Soliciting user reactions*. <!-- completed: 2026-07-02T14:45 -->
- [x] `.claude/skills/skill-author/SKILL.md` — `--prompt-file` → `--text-file` in §2.4 / §3.5; replace the positional `prompt_argv` fallback discussion with `--text` (inline) / `--text-file` (file). <!-- completed: 2026-07-02T14:48 (Director — .claude/ deny-list) -->
- [x] Add explicit guidance in `skills/cafleet/reference/director.md` and `skills/cafleet/reference/supervision.md` that long or multi-line **message bodies** (`message send` / `broadcast` / `nudge`) must be passed via `--text-file` (or `-` stdin), not `--text`, to avoid `ARG_MAX`. <!-- completed: 2026-07-02T14:45 -->

### Step 4: Shared helper (code)

- [x] Rename `cafleet/src/cafleet/cli/_prompt.py` → `_text_input.py`; implement `read_text_input(text, text_file)` per §1 (xor+required, abs/CWD-relative path, `-` stdin, UTF-8, uniform empty-body rejection, generic `--text-file` error strings). <!-- completed: 2026-07-02T15:30 -->
- [x] Add `substitute_spawn_placeholders(body, *, fleet_id, agent_id, director_agent_id, coding_agent)` per §2; delete `resolve_prompt`, `read_prompt_file`, and `MEMBER_PROMPT_TEMPLATE`. <!-- completed: 2026-07-02T15:30 -->

### Step 5: Wire the four commands (code)

- [x] `message.py` `message_send` — drop `required=True` on `--text`, add `--text-file`, resolve via `read_text_input`. <!-- completed: 2026-07-02T15:30 -->
- [x] `message.py` `message_broadcast` — drop `required=True` on `--text`, add `--text-file`, resolve via `read_text_input`. <!-- completed: 2026-07-02T15:30 -->
- [x] `member.py` `member_nudge` — add `--text-file`, resolve via `read_text_input`, drop the now-redundant `if not text.strip()` check. <!-- completed: 2026-07-02T15:30 -->
- [x] `member.py` `member_create` — rename `--prompt-file` → `--text-file`, add `--text`, remove `@click.argument("prompt_argv")` and the prompt mutual-exclusion guard, resolve via `read_text_input` then `substitute_spawn_placeholders`; update the import from `_prompt` → `_text_input`. <!-- completed: 2026-07-02T15:30 -->

### Step 6: Remove `send-input` (code)

- [x] `member.py` — delete the `member_send_input` command and its `--choice` / `--freetext` options. <!-- completed: 2026-07-02T15:39 -->
- [x] `member.py` — update the `member delete` timeout recovery hint (~line 412) to drop the `answer any prompt with \`cafleet member send-input\`` clause (leaving `inspect with \`cafleet member capture\`, then re-run … or \`--force\``). <!-- completed: 2026-07-02T15:39 -->
- [x] `multiplexer/tmux.py` — delete `send_choice_key` and `send_freetext_and_submit`. <!-- completed: 2026-07-02T15:39 -->
- [x] `multiplexer/base.py` — delete the `send_choice_key` / `send_freetext_and_submit` protocol entries. <!-- completed: 2026-07-02T15:39 -->

### Step 7: Tests

- [x] Delete `cafleet/tests/cli/test_member_send_input.py`, `cafleet/tests/multiplexer/test_tmux_send_helpers.py`, and `cafleet/tests/cli/test_member_prompt_template.py` (the three `MEMBER_PROMPT_TEMPLATE` tests fail at import once the constant is gone). <!-- completed: 2026-07-02T15:45 -->
- [x] Remove the `send_choice_key` / `send_freetext_and_submit` slices from `test_tmux.py` (incl. the Esc-first-safeguard assertions) and `test_help_budget.py`; in `test_member_delete.py` (`:297`, `:357`) update the recovery-hint output assertions to the new hint text (no `send-input`). <!-- completed: 2026-07-02T15:45 -->
- [x] Update `cafleet/tests/cli/test_member.py`: `--prompt-file` → `--text-file`; add `--text`; remove positional-prompt and default-template cases; add a regression that bare `member create` (neither flag) now errors, and that the positional argument no longer parses. <!-- completed: 2026-07-02T15:45 -->
- [x] Add shared-helper tests (new `test_text_input.py`): xor + required, absolute path, CWD-relative path, `-` stdin, UTF-8 decode, CRLF preservation, empty inline/file/stdin rejection, and each file error surface. <!-- completed: 2026-07-02T15:45 -->
- [x] Add `--text-file` / stdin / empty-body coverage to `message send`, `message broadcast`, and `member nudge` tests. <!-- completed: 2026-07-02T15:45 -->
- [x] Run `mise //cafleet:test`, `mise //cafleet:lint`, `mise //cafleet:typecheck`, and `mise //cafleet:lint-overlay`; fix any fallout. <!-- completed: 2026-07-02T15:45 -->

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-07-02 | Initial draft |
