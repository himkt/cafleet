# Add `--prompt-file` option to `cafleet member create`

**Status**: Approved
**Progress**: 0/38 tasks complete
**Last Updated**: 2026-05-15

## Overview

Add a `--prompt-file PATH` option to `cafleet member create` so spawn prompts can be read from a file instead of inlined as a positional argument. This removes the documented ~2 KB tmux `split-window` size limit that blocks long, templated spawn prompts and lets every CAFleet-native team skill emit its spawn prompt to a single audit-grade file under `<BASE>/prompts/`.

## Success Criteria

- [ ] `cafleet member create --prompt-file <abs-path>` reads the file contents (UTF-8), runs the existing `str.format()` substitution over them with `session_id` / `agent_id` / `director_agent_id` kwargs, and uses the result as the spawn prompt — identical downstream behavior to the inline positional form.
- [ ] Passing both `--prompt-file` and a positional `prompt_argv` is a hard `UsageError` (mutually exclusive).
- [ ] Passing `--prompt-file` with a relative path exits non-zero with an actionable message telling the caller to pass an absolute path.
- [ ] Passing `--prompt-file` to a non-existent / not-readable file exits non-zero with the file-not-found / not-readable message defined in § 6.
- [ ] Passing `--prompt-file` to an empty file (zero bytes or whitespace-only after `.isspace()`) exits non-zero with a clear "file is empty" message.
- [ ] Passing `--prompt-file` to a file containing invalid UTF-8 exits non-zero with `Error: --prompt-file <path>: file is not valid UTF-8.`
- [ ] File contents are **not** stripped — trailing newlines and surrounding whitespace are preserved verbatim in the rendered prompt.
- [ ] Every CAFleet-native team skill (`cafleet`, `agent-team-monitoring`, `agent-team-supervision`, `design-doc-create`, `design-doc-execute`, `design-doc-interview`, `research-report`, `research-presentation`) writes its rendered spawn prompts to `<BASE>/prompts/<role>-<UTC-compact>.md` and invokes `member create` with `--prompt-file` pointing at that file.
- [ ] The retired `<BASE>/<role>.md` audit re-render convention is removed from every affected SKILL.md — the prompt-file IS the audit artifact, no second write.

---

## Background

### Current state

`cafleet member create` accepts the spawn prompt as a variadic positional argument (`prompt_argv`, `nargs=-1`, `cafleet/src/cafleet/cli.py:845`). When supplied, the words are joined with spaces and passed through `str.format()` with `session_id` / `agent_id` / `director_agent_id` as kwargs (`_resolve_prompt`, `cli.py:783–814`). The substituted result becomes a single positional argument to the spawned `claude` or `codex` binary, which `tmux split-window` invokes (`cli.py:895–898`).

### The size limit

Per `skills/cafleet/reference/director.md` § *Spawn prompt size limit*:

> cafleet hands the prompt to `tmux split-window` as a single positional argument. tmux fails with `tmux command failed: command too long` and cafleet rolls back the agent registration once the shell-quoted prompt grows past a few KB — well below `ARG_MAX`. Empirically a prompt that inlines a full role definition (~10 KB) already exceeds this threshold.

The mitigation today is *path-by-reference for role docs*: the skill keeps the inline spawn prompt to ~2 KB by writing `ROLE DEFINITION: Open <abs path>` instead of inlining the role text. This works but is fragile — adding any new identity block field (a richer COMMUNICATION PROTOCOL stanza, an extra `[INSERT ...]` marker, a longer initial-task description) creeps the prompt toward the ceiling.

### Why a file option

The cost the current ceiling actually pays is not on the `tmux split-window` argv hand-off — it is on the *caller-side* shell that has to hold the rendered prompt as a single quoted positional argument to `cafleet member create`. Three layers stack up there: the shell input-buffer limit on the line the caller writes, the kernel `ARG_MAX` budget the shell consumes when it `execve`'s `cafleet`, and the same `ARG_MAX` budget consumed again when cafleet `execve`'s `tmux split-window`. Each of those carries the full quoted prompt simultaneously; together they exhaust well below `ARG_MAX` on its own and produce the documented `tmux command failed: command too long` rollback.

With `--prompt-file`, the cafleet invocation's argv carries only the path string (tens of bytes). The prompt body lives on disk until `cafleet` opens the file, reads it into Python memory, runs `str.format()`, and hands the substituted text to `tmux.split_window` as one argv element of the `tmux split-window` subprocess (`cafleet/src/cafleet/tmux.py:52-63`). Only that final `execve` carries the body; the caller-side shell and the cafleet-process argv no longer compound the budget. The remaining single `execve` element sits comfortably above any realistic spawn-prompt size (kernel `ARG_MAX` is typically 128 KB or more; rendered spawn prompts top out in the tens of KB).

More importantly, the file is a permanent audit artifact, eliminating the *parallel* `<BASE>/<role>.md` re-render that every team-skill Director currently performs after `member create`.

### Existing audit-re-render convention (retired by this change)

Each team-skill Director today does TWO writes per spawn:

1. The inline `prompt_argv` argv passed to `member create`.
2. A separate post-hoc re-render at `<BASE>/<role>.md` — concretely `<BASE>/drafter.md`, `<BASE>/reviewer.md`, `<BASE>/programmer.md`, `<BASE>/tester.md`, `<BASE>/verifier.md`, `<BASE>/manager.md`, and `<BASE>/analyzer.md` — with the three kwargs bound, intended as an inspect-the-substituted-prompt audit artifact.

The re-render is overwritten on subsequent spawns of the same role, so historical audits are lost. The new convention replaces both writes with a single timestamped pre-spawn file that doubles as the CLI input and as the permanent audit artifact.

---

## Specification

### 1. CLI surface change

Add a `--prompt-file` option to `cafleet member create`:

```bash
cafleet --session-id <session-id> member create --agent-id <director-agent-id> \
  --name "Drafter" \
  --description "Writes and revises the design document" \
  --prompt-file /home/foo/.cafleet-base/prompts/drafter-20260514T145000Z.md
```

Click declaration (sketch):

```python
@click.option(
    "--prompt-file",
    "prompt_file",
    type=str,
    default=None,
    help="Read spawn prompt from FILE (absolute path; UTF-8). "
         "Mutually exclusive with the positional prompt argument.",
)
```

Notes:

- `type=str` (NOT `click.Path(exists=True, ...)`) so the CLI sees the user's raw input first and the helper `_read_prompt_file` (§ 5) can apply the absolute-path UsageError BEFORE existence is checked. Otherwise Click's built-in "Path does not exist" check fires for any relative + non-existent input (the natural Step 3 test `./foo.md`) and the absolute-path Success Criterion is unverifiable.
- The flag is `--prompt-file` (singular, kebab-case) to match `--coding-agent` naming.
- The existing variadic positional `prompt_argv` stays in place; no deprecation. Trivial one-line ad-hoc spawns may still inline.

### 2. Mutual exclusion

| Inputs | Behavior |
|--------|----------|
| Neither `--prompt-file` nor positional `prompt_argv` | Use the built-in default `_MEMBER_PROMPT_TEMPLATE` (current behavior, unchanged). |
| Positional `prompt_argv` only | Existing inline path (current behavior, unchanged). |
| `--prompt-file PATH` only | New path — read file, substitute, spawn. |
| Both positional `prompt_argv` and `--prompt-file` | `click.UsageError`: `--prompt-file and the positional prompt argument are mutually exclusive.` |

The check fires in `member_create` before any registration work begins so a misuse does not leave a half-created agent behind.

### 3. Path resolution

Relative paths are **rejected** at the CLI layer. The CLI does NOT call `cafleet.base_dir.resolve()` — that responsibility belongs to the caller (a skill) which has the BASE context.

| Input | Behavior |
|-------|----------|
| Absolute path (`pathlib.PurePath(p).is_absolute()`) | Accepted; passes to the file-read step. |
| Relative path | `click.UsageError`: `--prompt-file requires an absolute path (got '<input>'). Resolve relative paths against your BASE first — see Skill(cafleet:base-dir).` |

Rationale: the CLI is a thin process; baking base-dir semantics into it would couple `cafleet member create` to the calling Director's resolved `${BASE}` (which the CLI never sees on its own — `${BASE}` lives in the Director's harness context, not in cafleet's environment). Skills already resolve `${BASE}` at startup via `Skill(cafleet:base-dir)`; they can pass an absolute path with zero ambiguity.

The absolute-path check is the FIRST gate inside `_read_prompt_file` (§ 5) and fires BEFORE any existence or readability probe. This ordering matters because Click's built-in `click.Path(exists=True)` validator would otherwise short-circuit the relative-path UsageError for inputs like `./foo.md` (relative AND non-existent) — § 1 therefore declares the Click option as `type=str`, not `click.Path`.

### 4. File read and substitution

All validation steps are delegated to `_read_prompt_file(path)` (§ 5). After path-validation succeeds, the helper's return value (the raw file text) is passed to `_resolve_prompt` in place of `" ".join(prompt_argv)`. The same `str.format()` call substitutes `session_id` / `agent_id` / `director_agent_id`. The same double-literal-brace rule (`{{` / `}}`) applies. Trailing newlines and surrounding whitespace are preserved verbatim — the helper does NOT strip. After substitution, control returns to the existing `_build_claude_command` / `_build_codex_command` path unchanged.

### 5. `_resolve_prompt` refactor

Current signature (`cli.py:783`):

```python
def _resolve_prompt(
    ctx: click.Context,
    director_agent_id: str,
    new_agent_id: str,
    prompt_argv: tuple[str, ...],
) -> str:
    session_id = ctx.obj["session_id"]
    template = " ".join(prompt_argv) if prompt_argv else _MEMBER_PROMPT_TEMPLATE
    ...
```

New signature (single source of substitution; one new parameter):

```python
def _resolve_prompt(
    ctx: click.Context,
    director_agent_id: str,
    new_agent_id: str,
    prompt_argv: tuple[str, ...],
    prompt_file: str | None,
) -> str:
    session_id = ctx.obj["session_id"]
    if prompt_file is not None:
        template = _read_prompt_file(prompt_file)
    elif prompt_argv:
        template = " ".join(prompt_argv)
    else:
        template = _MEMBER_PROMPT_TEMPLATE
    try:
        return template.format(...)
    except ...
```

`_read_prompt_file(path: str) -> str` is the new helper. Order of checks (each raises `click.UsageError` or `click.ClickException` and short-circuits the rest):

1. **Absolute-path check** (`pathlib.PurePath(path).is_absolute()`). Relative → `UsageError`.
2. **Existence check** (`Path(path).is_file()`). Missing or not-a-file → `ClickException("--prompt-file <path>: file does not exist or is not a regular file.")`.
3. **Read + UTF-8 decode** (`Path(path).read_text(encoding="utf-8")`). `PermissionError` → `ClickException("--prompt-file <path>: file is not readable.")`. `UnicodeDecodeError` → `ClickException("--prompt-file <path>: file is not valid UTF-8.")`.
4. **Emptiness check** (`content == ""` or `content.isspace()`). Empty → `ClickException("--prompt-file <path>: file is empty.")`.

`_read_prompt_file` owns all four error surfaces. Click's `click.Path(exists=True, readable=True)` validator is intentionally NOT used — its checks fire before any option-callback or command body, so a relative + non-existent input would raise "Path does not exist" before the absolute-path UsageError could fire. The Success Criterion for relative-path rejection requires our own ordering.

### 6. Error message catalog

Messages below show the raw exception body. Click prepends `Error: ` to all `UsageError` / `ClickException` instances in actual stderr output, so the rendered surface is uniform.

| Trigger | Exit code | Message |
|---------|-----------|---------|
| Both `--prompt-file` and positional `prompt_argv` | 2 (UsageError) | `--prompt-file and the positional prompt argument are mutually exclusive.` |
| Relative path | 2 (UsageError) | `--prompt-file requires an absolute path (got '<input>'). Resolve relative paths against your BASE first — see Skill(cafleet:base-dir).` |
| File not found / not a regular file | 1 (ClickException) | `--prompt-file <path>: file does not exist or is not a regular file.` |
| File not readable | 1 (ClickException) | `--prompt-file <path>: file is not readable.` |
| Invalid UTF-8 | 1 (ClickException) | `--prompt-file <path>: file is not valid UTF-8.` |
| Empty / whitespace-only | 1 (ClickException) | `--prompt-file <path>: file is empty.` |
| Unknown `{placeholder}` in file content | 2 (UsageError, existing path) | `Unknown placeholder ... in custom prompt. Supported placeholders: {session_id}, {agent_id}, {director_agent_id}. Double literal braces ({{, }}) to keep them as text.` |

### 7. Skill-side convention

Every CAFleet-native team skill that spawns members via templated prompts MUST follow this protocol:

1. **Resolve `${BASE}`** via `Skill(cafleet:base-dir)` (already required at skill startup; no new step).
2. **Render the template** locally — substitute every `[INSERT ...]` marker. Leave `{session_id}` / `{agent_id}` / `{director_agent_id}` placeholders in place; the CLI's `str.format()` pass resolves them at member-create time using the newly-allocated `agent_id`.
3. **Write the rendered text** to `<BASE>/prompts/<role>-<UTC-compact>.md` where:
   - `<role>` is the lowercased value of `--name` (e.g., `drafter`, `reviewer`, `programmer`, `tester`, `verifier`, `manager`, `analyzer`).
   - `<UTC-compact>` is `YYYYMMDDTHHMMSSZ` (UTC, ISO 8601 compact, `Z` suffix; Python: `datetime.now(UTC).strftime("%Y%m%dT%H%M%SZ")`).
   - Create the `prompts/` subdirectory on first write (`Path("<BASE>/prompts").mkdir(parents=True, exist_ok=True)`).
   - Same-second collisions: skills MUST NOT overwrite an existing file. If the target path already exists, append `_2`, `_3`, … until the name is unique. In practice spawns are gap-separated by seconds; collision is rare.
4. **Invoke** `cafleet member create` with `--prompt-file <abs path>`. Do NOT pass an inline positional prompt alongside.
5. **No second audit write.** The pre-spawn file IS the audit artifact. The retired `<BASE>/<role>.md` write is gone.

### 7a. `${BASE} == <unset>` fallback

`Skill(cafleet:base-dir)` § *The `<unset>` sentinel* item 1 mandates that every BASE-derived write site is guarded; unguarded `Path(BASE) / …` computation under `<unset>` is a loud failure. The prompts/ write inherits that contract. Skills MUST follow this branch when their startup-time `${BASE}` resolution returned `<unset>`:

1. **Skip the file write.** Do NOT compute `<BASE>/prompts/<role>-<ts>.md`; treat the audit-file feature as disabled. (Equivalent to the existing `<BASE>/<role>.md` guarded-skip behavior.)
2. **Fall back to the inline positional `prompt_argv` form** of `cafleet member create`. The size limit explained in § *Background — The size limit* still applies — skills MUST keep the inline-form prompt under ~2 KB, which is the same constraint they live with today (path-by-reference for role docs, short identity block).
3. **Emit the anchorless status** `audit-disabled no BASE in spawn prompt` once per spawn cycle (per `Skill(cafleet:base-dir)` § *Missing-BASE-line anchorless status*) so the operator sees that the prompt-file audit channel is unavailable. The spawn itself still proceeds.

The fallback exists because the alternative — aborting the spawn entirely — would render every team-skill unusable in environments where the user explicitly chose absolute-path mode (the `<unset>` branch is reached only via the absolute-path argument branch in `cafleet base-dir resolve`, an intentional operator choice). The fallback preserves the team skill's ability to spawn while making the missing audit channel observable.

Inline `-- "<prompt>"` invocation is still permitted for trivial one-line ad-hoc spawns (e.g., test scripts, doctor flows). Any skill rendering a templated identity block + role-file-by-path prompt MUST use `--prompt-file` *unless* it is on the § 7a `<unset>` fallback branch.

### 8. Migration of affected skills

The following SKILL.md and role files render templated spawn prompts and must switch to `--prompt-file`. Each entry is a documentation update only; no code is moved between files.

Locations are cited by **section heading**, not by line number — line numbers drift every time anything above is edited.

| Skill | Files | Sections affected |
|-------|-------|-------------------|
| `cafleet` | `skills/cafleet/reference/director.md` | § *Member Create* example block + § *Member Create — Scratch and audit files* paragraph; expand to document the new `<BASE>/prompts/<role>-<ts>.md` convention and the § 7a `<unset>` fallback. |
| `agent-team-monitoring` | `skills/agent-team-monitoring/SKILL.md` | Update the `member create` example block (if present) to the `--prompt-file` form. |
| `agent-team-supervision` | `skills/agent-team-supervision/SKILL.md` | Same as monitoring. |
| `design-doc-create` | `skills/design-doc-create/SKILL.md` | § *1d. Spawn the Drafter*, § *1e. Spawn the Reviewer*, plus the resume-mode Analyzer block in `roles/director.md` (if present). |
| `design-doc-execute` | `skills/design-doc-execute/SKILL.md` | § *3e. Spawn each member via `cafleet member create`* (Programmer, Tester, Verifier sub-blocks). |
| `design-doc-interview` | `skills/design-doc-interview/SKILL.md` | § *2d. Spawn the Analyzer*. |
| `research-report` | `skills/research-report/SKILL.md` | § *2c. Spawn the Manager*, plus § *4b. Spawn each Researcher (Director)* for downstream Researcher spawns. |
| `research-presentation` | `skills/research-presentation/SKILL.md` | § *1d. Spawn Presentation + Transcript in parallel*. |

Migration pattern, applied to every "Spawn with:" block:

**Before:**

```bash
cafleet --session-id <session-id> --json member create --agent-id <director-agent-id> \
  --name "Drafter" \
  --description "Writes and revises the design document" \
  -- "<Drafter spawn prompt — role file referenced by absolute path>"
```

**After:**

```bash
# Step 1: render the prompt to a file under BASE
#   <prompt-path> = <BASE>/prompts/drafter-<YYYYMMDDTHHMMSSZ>.md
#   The Director writes the rendered prompt (with [INSERT ...] markers substituted
#   and {session_id}/{agent_id}/{director_agent_id} placeholders intact) to that path.

# Step 2: spawn with --prompt-file
cafleet --session-id <session-id> --json member create --agent-id <director-agent-id> \
  --name "Drafter" \
  --description "Writes and revises the design document" \
  --prompt-file <prompt-path>
```

The "After parsing `agent_id`:" post-spawn step (currently: re-render + write `<BASE>/drafter.md`) is **removed**. The pre-spawn file already captures the audit artifact.

### 9. Out of scope

- `cafleet message send` and `cafleet message broadcast` do NOT gain `--prompt-file`. Per user direction in clarification round, files are not needed for message bodies. Body truncation, persistence, and inline-preview behavior stay unchanged.
- `cafleet member exec` and `cafleet member send-input --freetext` do NOT gain `--prompt-file`. Both reject newlines and accept only single-line payloads, so file input adds no value.
- No deprecation of the inline positional form. It remains a first-class input.
- No automatic cleanup of `<BASE>/prompts/`. Files are audit-kept; operators delete manually if needed.
- No changes to the wider base-dir contract or `<BASE>/.cafleet-base-dir.json` anchor handling.

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

### Step 1: Documentation updates (must precede any code)

Per project rule `.claude/rules/design-doc-numbering.md`: documentation FIRST.

- [ ] Update `ARCHITECTURE.md` — add a paragraph under the CLI section documenting `--prompt-file` on `member create` (alternative input mechanism, absolute-path requirement, audit role). <!-- completed: -->
- [ ] Update `docs/` — add or extend the relevant CLI reference page (`docs/cli-reference.md` or equivalent) with the `--prompt-file` flag row, mutual-exclusion rule, and error-message catalog from § 6. <!-- completed: -->
- [ ] Update `README.md` — if README enumerates `member create` flags or describes spawn-prompt sizing, add the `--prompt-file` mention; otherwise leave unchanged (README change must be consistent with ARCHITECTURE.md / docs/ outcome). <!-- completed: -->
- [ ] Update `skills/cafleet/reference/director.md` § *Member Create*: add the `--prompt-file` row to the flag table, the new spawn-with-file example, and the `<BASE>/prompts/<role>-<ts>.md` audit convention paragraph (retiring the `<BASE>/<role>.md` pointer). <!-- completed: -->
- [ ] Update `skills/design-doc-create/SKILL.md` — convert the spawn blocks under § *1d. Spawn the Drafter* and § *1e. Spawn the Reviewer* to the two-step (render to file, then `--prompt-file`) pattern. Remove the post-spawn `<BASE>/drafter.md` and `<BASE>/reviewer.md` re-render steps; add the pre-spawn write step. <!-- completed: -->
- [ ] Update `skills/design-doc-create/roles/director.md` and `roles/reviewer.md` if they reference the old audit-file convention. <!-- completed: -->
- [ ] Update `skills/design-doc-execute/SKILL.md` — same conversion under § *3e. Spawn each member via `cafleet member create`* for the Programmer / Tester / Verifier sub-blocks. <!-- completed: -->
- [ ] Update `skills/design-doc-interview/SKILL.md` — convert § *2d. Spawn the Analyzer*. <!-- completed: -->
- [ ] Update `skills/research-report/SKILL.md` — convert § *2c. Spawn the Manager* and § *4b. Spawn each Researcher (Director)*. <!-- completed: -->
- [ ] Update `skills/research-presentation/SKILL.md` — convert Slidev-composer spawn block. <!-- completed: -->
- [ ] Update `skills/agent-team-monitoring/SKILL.md` and `skills/agent-team-supervision/SKILL.md` if either embeds a templated `member create` example. <!-- completed: -->
- [ ] Remove every standing `${BASE}/<role>.md` audit-write paragraph across the skill tree per `.claude/rules/removal.md`: the convention is retired, not deprecated, so no "previously written to" pointers remain. <!-- completed: -->

### Step 2: CLI implementation

- [ ] Add the `_read_prompt_file(path: str) -> str` helper to `cafleet/src/cafleet/cli.py`. Responsibilities: absolute-path check, `Path.read_text(encoding="utf-8")` with `UnicodeDecodeError` handling, emptiness check (`content == ""` or `content.isspace()`). Each failure raises `click.UsageError` or `click.ClickException` per § 6. <!-- completed: -->
- [ ] Extend `_resolve_prompt` signature with `prompt_file: str | None` and branch order: `prompt_file` → `prompt_argv` → default template (§ 5). <!-- completed: -->
- [ ] Add the `@click.option("--prompt-file", ...)` declaration to `member_create` with `type=str` (NOT `click.Path` — see § 1 rationale) and the help text from § 1. <!-- completed: -->
- [ ] Add the mutual-exclusion guard at the top of `member_create` (before `_require_session_id`): raise `click.UsageError` if both `prompt_file` and `prompt_argv` are non-empty. <!-- completed: -->
- [ ] Pass `prompt_file` through to `_resolve_prompt` in the `member_create` body. <!-- completed: -->
- [ ] Run `mise //cafleet:lint`, `mise //cafleet:format`, `mise //cafleet:typecheck` and resolve any failures. <!-- completed: -->

### Step 3: Tests

- [ ] Add a new test module `cafleet/tests/cli/test_member_create_prompt_file.py` (or extend an existing `test_member_create.py`). <!-- completed: -->
- [ ] Test: `--prompt-file` with absolute path containing only `{session_id}` placeholder → spawn argv ends with the substituted string. Parametrize both `claude` and `codex` backends. <!-- completed: -->
- [ ] Test: parity with positional form — given the same template text, `--prompt-file` and positional `prompt_argv` produce byte-identical spawn argvs (after substitution). <!-- completed: -->
- [ ] Test: mutual-exclusion error when both inputs supplied — assert exit code 2 and the message from § 6. <!-- completed: -->
- [ ] Test: relative path error — call with `--prompt-file ./foo.md`; assert exit code 2 and the absolute-path message. <!-- completed: -->
- [ ] Test: file-not-found error — call with `--prompt-file /tmp/does-not-exist`; assert exit code 1 and the file-not-found message from § 6. Add a companion test for a path pointing at a directory rather than a regular file (same expected message). <!-- completed: -->
- [ ] Test: empty-file error — create a zero-byte file, call `--prompt-file <that>`, assert exit code 1 and the empty message. Repeat with a whitespace-only (`\n   \t\n`) file. <!-- completed: -->
- [ ] Test: invalid UTF-8 error — create a file with bytes `b"\xff\xfe\xfd"`, assert exit code 1 and the UTF-8 message. <!-- completed: -->
- [ ] Test: trailing newline preservation — file content `"hello\n"` with no placeholders → spawn argv contains the literal `"hello\n"`. <!-- completed: -->
- [ ] Test: surrounding-whitespace preservation — file content `"   \n  hello world  \n   "` with no placeholders → spawn argv contains the identical leading/inner/trailing whitespace, byte-for-byte. Covers the second half of the no-strip Success Criterion. <!-- completed: -->
- [ ] Test: format-error path (unknown placeholder in file) — file content `"hi {unknown}"`, assert the existing `Unknown placeholder` UsageError fires unchanged. <!-- completed: -->
- [ ] Run `mise //cafleet:test` and verify all new tests pass alongside the existing suite. <!-- completed: -->

### Step 4: Verification

The full enumeration of retired audit-file basenames (from § *Existing audit-re-render convention*) is the regression checklist: `drafter.md`, `reviewer.md`, `programmer.md`, `tester.md`, `verifier.md`, `manager.md`, `analyzer.md`. After each smoke spawn below, confirm the corresponding `<BASE>/<role>.md` is NOT written.

- [ ] End-to-end manual: in a tmux session with `cafleet doctor` clean, render a 5 KB prompt to a temp file, spawn a member via `--prompt-file`, capture the pane and confirm the full prompt is visible. The same content as a positional argv would fail with `tmux command failed: command too long`; the file path succeeds. <!-- completed: -->
- [ ] Smoke: `/design-doc-create` on a throwaway slug — confirm `<BASE>/prompts/drafter-<ts>.md` + `<BASE>/prompts/reviewer-<ts>.md` are written (timestamped, non-overwriting) and neither `<BASE>/drafter.md` nor `<BASE>/reviewer.md` appears. <!-- completed: -->
- [ ] Smoke: `/design-doc-execute` on a single-step throwaway design doc — confirm `<BASE>/prompts/programmer-<ts>.md`, `<BASE>/prompts/tester-<ts>.md`, and (if the optional Verifier is spawned) `<BASE>/prompts/verifier-<ts>.md` are written and that none of `<BASE>/programmer.md` / `<BASE>/tester.md` / `<BASE>/verifier.md` appears. <!-- completed: -->
- [ ] Smoke: `/design-doc-interview` on a small design doc — confirm `<BASE>/prompts/analyzer-<ts>.md` is written and `<BASE>/analyzer.md` does NOT appear. <!-- completed: -->
- [ ] Smoke: `/cafleet:research-report` on a trivial topic — confirm `<BASE>/prompts/manager-<ts>.md` plus one `<BASE>/prompts/<researcher-name>-<ts>.md` per spawned Researcher are written, and `<BASE>/manager.md` does NOT appear. <!-- completed: -->
- [ ] Smoke: `/cafleet:research-presentation` on the report folder from the previous step — confirm `<BASE>/prompts/<presentation-role>-<ts>.md` is written and no retired audit basename appears. <!-- completed: -->
- [ ] `<unset>` fallback smoke: drive a single team-skill spawn with `${BASE} == <unset>` (resolved via `cafleet base-dir resolve --path <abs path>`). Confirm the skill emits the anchorless status `audit-disabled no BASE in spawn prompt`, that no `<BASE>/prompts/` directory is created, and that the spawn still succeeds via the inline-positional fallback (§ 7a). <!-- completed: -->
- [ ] Confirm `mise //cafleet:lint`, `mise //cafleet:format --check`, `mise //cafleet:typecheck`, `mise //cafleet:test` all pass on the final branch. <!-- completed: -->

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-05-14 | Initial draft |
| 2026-05-15 | Address Reviewer round-1 markers: explicit `--prompt-file` mechanism rationale; analyzer.md added to retired-audit-list; Click option declared as `type=str` so absolute-path UsageError fires before existence check; § 7a fallback contract for `${BASE} == <unset>`; migration table uses section headings; surrounding-whitespace test added; Step 4 expanded to one smoke task per affected team-skill plus an `<unset>` fallback smoke; error catalog caption added. |
| 2026-05-15 | User approval; Status flipped to Approved. |
