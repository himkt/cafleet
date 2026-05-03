# Permission-aware shell dispatch via `cafleet member safe-exec`

**Status**: Aborted
**Progress**: 0/20 tasks complete
**Last Updated**: 2026-05-03

## Overview

Replace the always-permitted `cafleet member send-input --bash` dispatch with a new permission-aware entry point, `cafleet member safe-exec --bash CMD`, that re-reads Claude Code's three-layer `settings.json` files on every invocation and decides allow / deny / ask for the inner CMD by matching it against `Bash(...)` allow and deny globs. The `--bash` flag on `cafleet member send-input` is removed entirely (hard rename, no alias). This puts the operator's existing `permissions.allow` / `permissions.deny` grammar in charge of every Director-dispatched shell command.

## Success Criteria

- [ ] `cafleet member send-input` no longer accepts a `--bash` flag. The flag is removed from `cafleet/src/cafleet/cli.py`. Click rejects the old form with `Error: No such option: '--bash'.` (exit 2).
- [ ] `cafleet member safe-exec --bash CMD` exists as a new Director-only subcommand, mutually exclusive with no other input mode (it has only `--bash` because shell dispatch is its single purpose).
- [ ] Every `safe-exec` invocation re-reads three settings files in the order project-local → project → user. No caching at any layer.
- [ ] Discovery honors `CLAUDE_CONFIG_DIR/settings.json` for the user layer when the env var is set, falling back to `~/.claude/settings.json` when unset.
- [ ] Allow lists and deny lists are unioned across all three layers. Deny wins on any conflict.
- [ ] Only `Bash(...)` patterns are honored; `Read(...)`, `WebFetch(...)`, and any other tool prefix is silently ignored.
- [ ] Allow path: dispatches the inner CMD into the member pane via existing `tmux.send_bash_command`. Exit 0.
- [ ] Deny path: command is NOT dispatched. Exit 2. Stderr names the matched deny pattern, the file it lives in, and the offending command substring.
- [ ] Ask path: command is NOT dispatched. Exit 3. Stderr lists the three searched files (with the resolved user path) and a suggested `Bash(...)` pattern the operator can add.
- [ ] `cafleet --json member safe-exec --bash CMD` emits a structured JSON payload for all three outcomes with keys `outcome`, `matched_pattern`, `matched_file`, `offending_substring`, `searched_files`.
- [ ] Cross-Director boundary: `safe-exec` rejects when `placement.director_agent_id != --agent-id` with the existing wording (`agent <id> is not a member of your team (director_agent_id=<other>)`).
- [ ] Pending placement (no `tmux_pane_id`) is rejected with the existing wording.
- [ ] Documentation is updated FIRST per `.claude/rules/design-doc-numbering.md`. The full target list is enumerated in Implementation Step 1.
- [ ] `mise //cafleet:test`, `mise //cafleet:lint`, `mise //cafleet:format`, `mise //cafleet:typecheck` all pass.
- [ ] `.claude/settings.json` removes the three obsolete `ask` entries that scoped the old `--bash` flag and adds `Bash(cafleet --session-id * member safe-exec *)` to `allow`.

---

## Background

Predecessor design **0000034** (`member-bash-via-director`) introduced `cafleet member send-input --bash CMD` as the Director's dispatch primitive when a member's harness deny-list rejects a Bash invocation. The dispatch is unconditional: any operator-typed `cafleet member send-input --bash <whatever>` keystrokes the command into the member pane. Claude Code's `permissions.allow` is the only filter, configured statically via three `ask` patterns in `.claude/settings.json`:

```json
"ask": [
  "Bash(cafleet --session-id * member send-input --bash *)",
  "Bash(cafleet --session-id * member send-input * --bash)",
  "Bash(cafleet --session-id * member send-input * --bash *)"
]
```

The static `ask` rules cover the *outer* `cafleet ...` invocation: every dispatch fires Claude Code's permission prompt regardless of what the inner CMD is. They cannot distinguish `git status` from `git push --force origin main` — both prompt the operator identically, both bypass the operator's careful `permissions.allow` setup for routine shell calls.

This design moves the inner-CMD permission decision from "always prompt" to "match against the same `Bash(...)` allow / deny patterns the operator already maintains for their own Bash tool". The decision becomes:

- The operator's existing allow rules (`Bash(git status:*)`, `Bash(npm test:*)`, ...) cover the inner CMD without a prompt.
- The operator's existing deny rules (`Bash(git push:*)`, `Bash(rm -rf *)`, ...) reject the inner CMD without a prompt.
- Anything the operator has not yet rule'd surfaces as a structured ask-output the Director relays back to the operator, who edits `settings.json` and re-runs.

---

## Specification

### 1. Subcommand surface

Two CLI changes are coupled into one design:

| Change | Before | After |
|---|---|---|
| Existing `member send-input` | accepts `--choice`, `--freetext`, `--bash` (mutually exclusive) | accepts `--choice`, `--freetext` (mutually exclusive). `--bash` is removed entirely — Click rejects the old form with `No such option`. |
| New `member safe-exec` | does not exist | accepts `--bash CMD` (the only input flag on this subcommand). Director-only. Permission-aware dispatch. |

There is no aliasing, no deprecation period, no wrapper-shim retaining the old name. Per `.claude/rules/removal.md`, every literal mention of the old `--bash` flag on `cafleet member send-input` is removed in the same change set. **Callers must rewrite to `cafleet member safe-exec --bash CMD`. No migration shim, no deprecation period — the old form starts erroring immediately on merge.**

### 2. `cafleet member safe-exec` constraints

The full Click signature and handler body live in §4 (single source of truth). This section captures the input-validation contract.

| Constraint | Behavior |
|---|---|
| Empty `--bash ""` | Reject before permission check: `Error: --bash command may not be empty.` (exit 2). |
| `--bash` containing `\n` or `\r` | Reject before permission check: `Error: --bash command may not contain newlines.` (exit 2). |
| Cross-Director (`placement.director_agent_id != agent_id`) | Reject with existing `_load_authorized_member` wording (exit 1). |
| Pending placement (`placement.tmux_pane_id is None`) | Reject with existing wording (exit 1). |
| Compound CMD (pipes, `&&`, `;`, `$(...)`, backticks) | NOT special-cased. Treated as opaque string and matched as-is against patterns. The outer Bash hook layer (`validate_bash.py` and Claude Code's own permission system on the Director's `cafleet member safe-exec ...` invocation) is the layer that catches compound commands before they reach `safe-exec`. `safe-exec` does not enforce or detect them. |

### 3. Permission discovery (`cafleet/src/cafleet/permissions.py`)

A new module, `cafleet/src/cafleet/permissions.py`, owns settings discovery and the glob matcher. The module has zero coupling to the broker, the CLI, or `Settings` (`config.py`) — it is a pure utility.

#### 3.1 Settings file order

```python
def discover_settings_paths() -> list[Path]:
    """
    Resolve the three settings file paths in matcher-precedence order.

    Returns paths in this order:
      [0] project-local: <cwd>/.claude/settings.local.json
      [1] project shared: <cwd>/.claude/settings.json
      [2] user: $CLAUDE_CONFIG_DIR/settings.json or ~/.claude/settings.json

    Each path is returned even if the file does not exist on disk; the
    caller treats a missing file as an empty {"permissions": {"allow": [],
    "deny": []}} document. The user path resolves $CLAUDE_CONFIG_DIR at
    call time (no caching) so live env changes take effect on the next
    invocation.
    """
```

CWD is `os.getcwd()` at the moment `safe-exec` runs, not the working directory the cafleet package was installed from. This matches Claude Code's own resolution.

The user-layer fallback when `CLAUDE_CONFIG_DIR` is unset is `~/.claude/settings.json` (with `~` expanded via `Path.expanduser()`).

#### 3.2 Loading and union semantics

```python
def load_bash_patterns(paths: list[Path]) -> tuple[list[Pattern], list[Pattern]]:
    """
    Returns (allow_patterns, deny_patterns), each annotated with the file
    they came from so deny / ask error messages can name the source.
    """
```

For each path:

| File state | Behavior |
|---|---|
| Does not exist | Treat as empty document. No error. |
| Exists, valid JSON, no `permissions` key | Treat as empty document. No error. |
| Exists, valid JSON, `permissions.allow` / `permissions.deny` are lists | Use them. |
| Exists, malformed JSON | Raise with the file path: `Error: failed to parse <path>: <json error>.` Surfaces in CLI as exit 1. |
| `permissions.allow` / `permissions.deny` contain non-`Bash(...)` entries | Silently filter them out. `Read(...)`, `WebFetch(...)`, etc. are ignored. |
| `permissions.ask` | Ignored entirely for `safe-exec` purposes — `safe-exec` has its own ask path. |

Allow lists are concatenated (union) across all three files. Deny lists are concatenated (union) across all three files. Each pattern is annotated with its source path.

#### 3.3 Glob matcher

```python
@dataclass(frozen=True)
class Pattern:
    raw: str            # the original "Bash(...)" string
    body: str           # the inner text between Bash( and )
    source_file: Path   # the settings.json that declared it

def match(pattern: Pattern, command: str) -> bool:
    """Return True if ``command`` matches ``pattern.body``."""
```

A pattern body is converted to a regex by classifying its trailing form first, then converting any remaining stars uniformly. Use `re.fullmatch` against the command string.

**Disambiguation algorithm** (apply in this order — each step either decides or moves on):

1. If `body == "*"`: match any command (return True). Done.
2. Classify the trailing form:
   - **WORD-BOUNDARY** if the body ends with `" *"` (literal space + star) OR `":*"` (literal colon + star). Strip the trailing two characters into `prefix_body`.
   - **NO-BOUNDARY** if the body ends with `"*"` AND the character immediately before it is neither a space nor a colon. Strip the single trailing star into `prefix_body`.
   - **NONE** otherwise. `prefix_body == body`.
3. Build the regex from `prefix_body`:
   - `re.escape` every non-`*` character.
   - Replace each remaining `*` with `.*` (interior or leading wildcard — greedy, no separator constraint).
4. Append the trailing-form fragment:
   - WORD-BOUNDARY → `(?:\s.*)?` (optional whitespace separator followed by anything; gives the prefix word-boundary semantics).
   - NO-BOUNDARY → `.*` (any tail, no separator required).
   - NONE → append nothing.
5. `re.fullmatch(constructed_regex, command)`.

**Worked examples** (each row shows the pattern body, classification, derived regex, and outcome against three command strings):

| body | classification | regex | `git status` | `git status --short` | `gitstatus` |
|---|---|---|---|---|---|
| `*` | rule 1 | `.*` | match | match | match |
| `git status` | rule 2 NONE (exact) | `git\ status` | match | NO | NO |
| `git status:*` | rule 2 WORD-BOUNDARY | `git\ status(?:\s.*)?` | match | match | NO |
| `git status *` | rule 2 WORD-BOUNDARY | `git\ status(?:\s.*)?` | match | match | NO |
| `gitstatus*` | rule 2 NO-BOUNDARY | `gitstatus.*` | NO | NO | match |
| `* install` | rule 2 NONE, interior `*` | `.*\ install` | NO | NO | NO |
| `git * main` | rule 2 NONE, interior `*` | `git\ .*\ main` | NO | NO | NO |

(In the last two rows the fullmatch is anchored, so the interior `*` becomes `.*` only for that interior position. There is no trailing wildcard. `Bash(* install)` fullmatches `npm install`. `Bash(git * main)` fullmatches `git checkout main` and `git push origin main`.)

The classification only inspects the LAST star in the body. Earlier stars are always rule-3-style interior wildcards. This is what makes `Bash(* install)` (no trailing star) NOT match `npm install --save-dev` — the missing trailing wildcard is intentional.

Out of scope for this design (explicit non-goals — see §6):
- Process-wrapper stripping (`timeout`, `time`, `nice`, `nohup`, `stdbuf`, `xargs`).
- Read-only command auto-allow (`ls`, `cat`, `grep`, `find`, `wc`, ...).
- Exec-wrapper auto-prompt (`watch`, `setsid`, `ionice`, `flock`).
- Compound-command split-and-match (pipes, logical operators, separators, subshells).

These Claude Code parity features are explicitly deferred. The matcher operates on the inner CMD as one opaque string.

#### 3.4 Decision

```python
@dataclass(frozen=True)
class Decision:
    outcome: Literal["allow", "deny", "ask"]
    matched_pattern: str | None     # the raw "Bash(...)" form
    matched_file: Path | None       # source settings.json
    offending_substring: str | None # for deny; equals the command itself
    searched_files: list[Path]      # all three resolved paths

def decide(command: str, paths: list[Path]) -> Decision:
    ...
```

Algorithm:

1. Resolve `paths` and load patterns via §3.2.
2. Test `command` against every deny pattern (across all three files, in any order). If any match: return `Decision("deny", first matching pattern, ...)`.
3. Else test `command` against every allow pattern. If any match: return `Decision("allow", first matching pattern, ...)`.
4. Else return `Decision("ask", None, None, None, paths)`.

Deny-wins is enforced by checking deny BEFORE allow. The order between deny patterns within the deny set does not matter for the binary outcome; the first matching deny is reported as the named pattern.

### 4. CLI surface for `safe-exec`

```python
@member.command("safe-exec")
@click.option("--agent-id", required=True)
@click.option("--member-id", required=True)
@click.option("--bash", "bash_command", required=True)
@click.pass_context
def member_safe_exec(ctx, agent_id, member_id, bash_command):
    _require_session_id(ctx)
    session_id = ctx.obj["session_id"]
    json_output = ctx.obj["json_output"]

    # Pre-flight validation
    if bash_command == "":
        raise click.UsageError("--bash command may not be empty.")
    if "\n" in bash_command or "\r" in bash_command:
        raise click.UsageError("--bash command may not contain newlines.")

    tmux.ensure_tmux_available()  # raises ClickException on failure

    target, placement = _load_authorized_member(
        session_id, agent_id, member_id,
        placement_missing_msg=(
            f"agent {member_id} has no placement row; it was not "
            f"spawned via `cafleet member create`."
        ),
    )
    pane_id = placement["tmux_pane_id"]
    if pane_id is None:
        raise click.ClickException(
            f"member {member_id} has no pane yet (pending placement) — nothing to send."
        )

    paths = permissions.discover_settings_paths()
    decision = permissions.decide(bash_command, paths)

    if decision.outcome == "allow":
        tmux.send_bash_command(target_pane_id=pane_id, command=bash_command)
        _emit_safe_exec_output(ctx, decision, member_id, pane_id, target["name"], bash_command)
        return  # exit 0

    if decision.outcome == "deny":
        _emit_safe_exec_output(ctx, decision, member_id, pane_id, target["name"], bash_command)
        ctx.exit(2)

    # outcome == "ask"
    _emit_safe_exec_output(ctx, decision, member_id, pane_id, target["name"], bash_command)
    ctx.exit(3)
```

`_emit_safe_exec_output` produces the text or JSON message per §5.

### 5. Output contract

#### 5.1 Text mode (default)

| Outcome | Stdout | Stderr | Exit |
|---|---|---|---|
| allow | `Sent bash command '<cmd>' to member <name> (<pane>).` (matches existing send-input wording) | (empty) | 0 |
| deny | (empty) | `Error: command rejected by deny pattern Bash(<body>) declared in <file>. Offending command: <cmd>` | 2 |
| ask | (empty) | `Error: no allow pattern matches "<cmd>". Add a Bash(...) pattern to one of:\n  - <project-local-path>\n  - <project-path>\n  - <user-path>\nFiles were re-read at this invocation. Suggested pattern: Bash(<first-token>:*)` | 3 |

`<first-token>` in the ask suggestion is the first whitespace-delimited token of `<cmd>` (e.g. `git` for `git status --short`). This is a hint, not authoritative — the operator may prefer a more or less specific pattern.

#### 5.2 JSON mode (`cafleet --json`)

For all three outcomes, stdout (allow) or stderr (deny / ask) carries:

```json
{
  "outcome": "allow" | "deny" | "ask",
  "matched_pattern": "Bash(git status:*)" | null,
  "matched_file": "/home/u/.claude/settings.json" | null,
  "offending_substring": "git status --short" | null,
  "searched_files": [
    "/home/u/work/proj/.claude/settings.local.json",
    "/home/u/work/proj/.claude/settings.json",
    "/home/u/.claude/settings.json"
  ]
}
```

| Outcome | `matched_pattern` | `matched_file` | `offending_substring` |
|---|---|---|---|
| allow | the matched allow pattern raw form | the file it came from | the dispatched cmd |
| deny | the matched deny pattern raw form | the file it came from | the dispatched cmd |
| ask | `null` | `null` | `null` |

`searched_files` is always populated with the three resolved paths (existing or not), in matcher precedence order.

The JSON key `offending_substring` carries the full dispatched cmd because the locked C3 schema fixes the key name. The text-mode stderr line uses the more accurate label `Offending command:` instead. The two surfaces are intentionally distinct — JSON consumers parse by key, humans read prose.

### 6. Out of scope (locked non-goals)

The following Claude Code permission-grammar features are explicitly NOT implemented in this design. If demand surfaces, they are a follow-up design.

| Feature | Why deferred |
|---|---|
| Process-wrapper stripping (`timeout`, `time`, `nice`, `nohup`, `stdbuf`, bare `xargs`) | Adds substantial parser surface; the operator can write explicit `Bash(timeout * git push:*)`-style patterns if needed. |
| Read-only command auto-allow set (`ls`, `cat`, `grep`, `find`, `wc`, ...) | Conflicts with operator-controlled allow lists; opinionated default. |
| Exec-wrapper auto-prompt (`watch`, `setsid`, `ionice`, `flock`) | Same reason as process wrappers. |
| Compound-command split-and-match (pipes, `&&`, `;`, `$(...)`, backticks) | Outer Bash hook layer (`validate_bash.py` + Claude Code's permission system on the Director's `cafleet ...` invocation) catches compound commands before `safe-exec` runs. `safe-exec` assumes (without enforcing) that the inner CMD is a single command in practice. |
| `permissions.ask` rule honoring | `safe-exec` has its own ask path. The operator's existing `permissions.ask` entries apply only to outer Claude Code Bash invocations. |
| Caching | Locked decision: every invocation re-reads all three files. |
| Runtime override flag (`--force`, `--allow-once`) | Locked decision: Claude Code grammar has no such concept; we mirror it. |

### 7. Error and exit-code matrix

| Path | Exit | Stream | Message anchor |
|---|---|---|---|
| Allow, dispatch succeeds | 0 | stdout | `Sent bash command '<cmd>' to member ...` |
| Deny | 2 | stderr | `command rejected by deny pattern <pat> declared in <file>. Offending command: <cmd>` |
| Ask | 3 | stderr | `no allow pattern matches "<cmd>"` |
| `--bash ""` (empty) | 2 | stderr | `--bash command may not be empty.` (Click UsageError) |
| `--bash` contains `\n` or `\r` | 2 | stderr | `--bash command may not contain newlines.` (Click UsageError) |
| Missing `--bash` | 2 | stderr | Click default `Missing option '--bash'.` |
| Cross-Director (placement.director_agent_id mismatch) | 1 | stderr | existing `_load_authorized_member` text |
| Pending pane (`tmux_pane_id is None`) | 1 | stderr | existing `member <id> has no pane yet (pending placement) — nothing to send.` |
| Settings file malformed JSON | 1 | stderr | `failed to parse <path>: <json error>` |
| `tmux.send_bash_command` raises TmuxError | 1 | stderr | existing `send failed: <details>` |
| `tmux` unavailable in env | 1 | stderr | existing `cafleet member commands must be run inside a tmux session` |

Exit-code rationale: `2` for deny aligns with Claude Code PreToolUse hook convention (exit 2 stops a tool call). `3` for ask is unique to `safe-exec`, signaling operator intervention required (edit settings.json). `1` is reserved for runtime / IO / authorization failures.

### 8. Cross-Director boundary

`safe-exec` reuses `_load_authorized_member(session_id, agent_id, member_id, ...)` exactly as the existing send-input subcommand uses it. No new authorization path. The check happens BEFORE settings discovery so we don't waste IO when the request is going to be rejected on identity grounds.

### 9. Documentation surface

Per `.claude/rules/design-doc-numbering.md`, documentation MUST be updated FIRST. Implementation Step 1 updates these surfaces in the order listed; tests and code follow only after Step 1 is complete.

| File | Change |
|---|---|
| `ARCHITECTURE.md` | Add a "Permission-aware shell dispatch" subsection under the CLI surface section. Describe three-file discovery, tri-state outcome, and the deferred Claude Code parity features. |
| `docs/spec/cli-options.md` | Drop the rows that documented the removed `--bash` flag on the existing send-input subcommand. Add a new `### member safe-exec` section with the flag table, exit codes, and the JSON output schema. |
| `README.md` | Update the CLI command list to add `member safe-exec` and remove the bash-flag mention from the existing send-input bullet. Add one example showing the allow / deny / ask paths in summary form. |
| `skills/cafleet/SKILL.md` | Rewrite the "Routing Bash via the Director" subsection: the dispatch primitive is `cafleet member safe-exec --bash CMD` (not the existing send-input subcommand). Drop the `--bash` row from the existing send-input flag table. Document the three-file discovery, tri-state outcome, and exit codes. |
| `skills/cafleet/roles/director.md` | Replace the `cafleet member send-input --bash` dispatch example with `cafleet member safe-exec --bash`. Add a paragraph on handling deny (relay the stderr block to the operator) and ask (relay the suggested pattern, ask the operator to add it to settings.json, then re-run). |
| `skills/cafleet/roles/member.md` | Update the trailing paragraph that names the Director's dispatch primitive: `cafleet member safe-exec --bash`. |
| `.claude/rules/bash-tool.md` | Two paragraphs change. (a) The "Director side (for completeness)" code block at the bottom of the file: replace `cafleet member send-input --bash <command>` with `cafleet member safe-exec --bash <command>`. (b) The "When your Bash tool denies a command" → operator-fallback paragraph: its surface-to-operator wording literally references `cafleet member send-input --bash <command> from your Director pane` and must be rewritten to point at `cafleet member safe-exec --bash <command>`. The default-path member-side text — "send a plain message asking the Director" — is genuinely unchanged. |
| `.claude/settings.json` | Remove the three obsolete `ask` entries that scoped the old bash-flag form on the existing send-input subcommand. Add `Bash(cafleet --session-id * member safe-exec *)` to `permissions.allow`. |

### 10. Test surface

| File | Change |
|---|---|
| `cafleet/tests/test_cli_member_send_input.py` (existing) | Delete the bash-flag test class, the bash recorder fixture, and any `--bash` argument in the flag-validation parametrizations. Add ONE regression test asserting that the old form produces Click `Error: No such option: '--bash'.` (exit 2). Per `.claude/rules/removal.md`, no positive sentinel — the absence is the test. |
| `cafleet/tests/test_cli_member_safe_exec.py` (new) | Full coverage: allow path dispatches via `tmux.send_bash_command`; deny path exits 2 with named pattern + file + offending command; ask path exits 3 with three resolved file paths and a suggested pattern; cross-Director rejection; pending-placement rejection; empty-string and newline rejection; `cafleet --json member safe-exec --bash CMD` emits a JSON object with exactly the five documented keys (`outcome`, `matched_pattern`, `matched_file`, `offending_substring`, `searched_files`) for each of the three outcomes. |
| `cafleet/tests/test_permissions.py` (new) | Settings discovery: `CLAUDE_CONFIG_DIR` set vs unset, missing files (treated as empty), malformed JSON (raises with file path), missing `permissions` key (treated as empty), `permissions.allow` / `deny` containing only non-`Bash` entries (filtered out), allow / deny union semantics, deny-wins-on-conflict. Glob matcher: `Bash(*)`, exact-match `Bash(npm test)`, trailing word-boundary `Bash(ls *)` and `Bash(npm test:*)`, no-boundary `Bash(ls*)`, mid-string `Bash(* install)`, multi-position `Bash(git * main)`. |

### 11. Settings-file change in this repo

`.claude/settings.json` (this project's own config) changes in lockstep:

```diff
   "ask": [
-    "Bash(cafleet --session-id * member send-input --bash *)",
-    "Bash(cafleet --session-id * member send-input * --bash)",
-    "Bash(cafleet --session-id * member send-input * --bash *)"
   ]
```

(The literal entries are removed; the `ask` array becomes empty or, if other entries are added in the future, retains those.)

```diff
   "allow": [
     ...
     "Bash(cafleet *)"
+    , "Bash(cafleet --session-id * member safe-exec *)"
   ]
```

The narrower `safe-exec` allow entry is REDUNDANT with the broader `Bash(cafleet *)` already in allow. It is added anyway to make the intent explicit and to give a stable hook for future tightening (e.g. removing the broad `Bash(cafleet *)` entry while keeping the safe-exec one).

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-04-29T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

### Step 1: Documentation FIRST

Update every documentation surface BEFORE writing any code, per `.claude/rules/design-doc-numbering.md`.

- [ ] Update `ARCHITECTURE.md` to add the "Permission-aware shell dispatch" subsection (§9). <!-- completed: -->
- [ ] Update `docs/spec/cli-options.md`: drop bash-flag rows on the existing send-input subcommand; add `### member safe-exec` section with flag table, exit codes, JSON schema. <!-- completed: -->
- [ ] Update `README.md`: replace bash-flag mention in the existing send-input bullet; add `member safe-exec` bullet; add a one-paragraph example showing allow / deny / ask in summary form. <!-- completed: -->
- [ ] Update `skills/cafleet/SKILL.md`: rewrite "Routing Bash via the Director"; drop bash-flag row from the existing send-input flag table. <!-- completed: -->
- [ ] Update `skills/cafleet/roles/director.md`: replace dispatch example, add deny / ask handling paragraphs. <!-- completed: -->
- [ ] Update `skills/cafleet/roles/member.md`: name `safe-exec` as the Director-side dispatch primitive in the closing paragraph. <!-- completed: -->
- [ ] Update `.claude/rules/bash-tool.md` per §9: (a) Director-side "for completeness" code block at the bottom of the file AND (b) operator-fallback paragraph in the "When your Bash tool denies a command" subsection — both must be rewritten to point at `cafleet member safe-exec --bash`. <!-- completed: -->
- [ ] Update `.claude/settings.json`: remove the three obsolete `ask` entries and add `Bash(cafleet --session-id * member safe-exec *)` under `permissions.allow`. <!-- completed: -->

### Step 2: Tests (TDD red phase)

Write all tests for `cafleet/permissions.py` and `cafleet member safe-exec` BEFORE writing implementation code. Tests fail until Step 3 lands.

- [ ] Create `cafleet/tests/test_permissions.py` covering settings discovery (env precedence, missing files, malformed JSON, missing `permissions` key) and the glob matcher (every pattern shape from §3.3). <!-- completed: -->
- [ ] Create `cafleet/tests/test_cli_member_safe_exec.py` covering allow / deny / ask paths, cross-Director rejection, pending-placement rejection, empty / newline rejection, and JSON output for all three outcomes. <!-- completed: -->
- [ ] Edit `cafleet/tests/test_cli_member_send_input.py`: delete the bash-flag class, the bash recorder fixture, and `--bash` mentions in flag-validation parametrizations. Add the single Click-no-such-option regression test. <!-- completed: -->

### Step 3: Implementation

- [ ] Create `cafleet/src/cafleet/permissions.py` implementing `discover_settings_paths`, `load_bash_patterns`, `match`, `decide`. <!-- completed: -->
- [ ] Edit `cafleet/src/cafleet/cli.py`: add `member safe-exec` Click command per §4. Wire the discovery + decision + dispatch + output flow. <!-- completed: -->
- [ ] Edit `cafleet/src/cafleet/cli.py`: remove the `--bash` Click option from the existing send-input subcommand. Update the mutual-exclusion message and the count-supplied check accordingly. <!-- completed: -->
- [ ] Run `mise //cafleet:test` and confirm every Step 2 test passes. <!-- completed: -->
- [ ] Run `mise //cafleet:lint`, `mise //cafleet:format --check`, `mise //cafleet:typecheck`. <!-- completed: -->

### Step 4: Cross-cutting verification

- [ ] Manual integration smoke: spawn a member via `cafleet member create`; from the Director, dispatch one allow-matching command (e.g. `git status`), one deny-matching command (e.g. `git push:*` if denied in user settings), and one unmatched command. Confirm exit codes 0 / 2 / 3 and the stderr / JSON shapes per §5. <!-- completed: -->
- [ ] Confirm the existing `Bash(cafleet *)` allow rule lets the Director's `cafleet member safe-exec ...` invocation through Claude Code without a per-call prompt. <!-- completed: -->
- [ ] Confirm `validate_bash.py` (project hook) blocks any compound-command attempt at the OUTER Director Bash invocation, demonstrating that `safe-exec`'s opaque pass-through assumption holds in practice. <!-- completed: -->
- [ ] Update this design document: flip Status to `Complete`, set `Last Updated` to the merge date. <!-- completed: -->

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-04-29 | Initial draft. |
| 2026-05-03 | Status flipped to Aborted. Superseded by 0000038-exec-subcommand, 0000039-member-ping-subcommand, and 0000042-member-exec-ping-chain, which shipped `cafleet member exec` (positional CMD under `permissions.ask`) instead of the proposed `safe-exec --bash` permission-matching approach. Not implemented. |
