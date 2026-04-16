# Add `cafleet member send-input` — safe `tmux send-keys` wrapper for member panes

**Status**: Approved
**Progress**: 5/18 tasks complete
**Last Updated**: 2026-04-16

## Overview

Add a `cafleet member send-input` subcommand that wraps `tmux send-keys` with a fixed, validated set of keystroke actions, intended for answering an `AskUserQuestion` prompt (or any prompt of the same shape) rendered in a member's Claude Code pane. The caller may send `1`, `2`, or `3` (for the first three choices) or `4` + literal free text + `Enter` (for the "Type something" escape hatch) — no other keys, no arbitrary key names, no shell-meta interpretation.

## Success Criteria

- [ ] `cafleet --session-id <s> member send-input --agent-id <d> --member-id <m> --choice {1,2,3}` sends the matching digit key to the member's pane via `tmux send-keys -t <pane> <digit>`
- [ ] `cafleet --session-id <s> member send-input --agent-id <d> --member-id <m> --freetext "<text>"` sends three tmux invocations in order: `<pane> 4`, `<pane> -l <text>`, `<pane> Enter`
- [ ] Exactly one of `--choice` / `--freetext` must be supplied; neither or both → click UsageError (exit 2) with `"Must supply exactly one of --choice or --freetext"`
- [ ] `--choice` is restricted to `1`, `2`, `3` via `click.IntRange(1, 3)`; values `0`, `4`, `5`, `"a"` → exit 2 via click's built-in range validator
- [ ] `--freetext` accepts any non-newline string (including empty string, Japanese, `$`, backtick, `;`, `&&`, `Enter`, `C-c`); newline characters (`\n` or `\r`) → exit 2 with `"free text may not contain newlines"` (single-action contract — one prompt submission per call)
- [ ] Cross-Director rejection: if `placement.director_agent_id != --agent-id`, exit 1 with `"agent <m> is not a member of your team"` (mirrors `member capture` wording)
- [ ] Pending-pane rejection: if `placement.tmux_pane_id is None`, exit 1 with `"member <m> has no pane yet (pending placement) — nothing to send"`
- [ ] Missing-placement rejection: if the target agent has no placement row, exit 1 with `"agent <m> has no placement row; it was not spawned via 'cafleet member create'."`
- [ ] `tmux` availability check uses `tmux.ensure_tmux_available()` (same guard every other `member *` command uses); missing tmux binary or missing `TMUX` env var → exit 1 with the `TmuxError` message
- [ ] Backend-agnostic: works identically for `claude` and `codex` member panes (the CLI never inspects `placement.coding_agent`)
- [ ] Text output: `Sent choice 1 to member <name> (%7).` / `Sent free text to member <name> (%7).`
- [ ] JSON output (`cafleet --json ... member send-input ...`): `{"member_agent_id": "<uuid>", "pane_id": "%7", "action": "choice", "value": "1"}` or `{"action": "freetext", "value": "<text>"}`
- [ ] Docs updated **before** code: `ARCHITECTURE.md`, `docs/spec/cli-options.md`, `README.md`, `.claude/skills/cafleet/SKILL.md`, `.claude/skills/cafleet-monitoring/SKILL.md`
- [ ] `mise //cafleet:test`, `mise //:lint`, `mise //:format`, `mise //:typecheck` all pass

---

## Background

### Current state

The Director already has one read-only way to inspect a stalled member: `cafleet member capture`. There is no corresponding write path — answering an `AskUserQuestion` prompt rendered in a member's pane currently requires raw `tmux send-keys` calls typed by the Director or the end user, which has three problems:

| Problem | Impact |
|---|---|
| Correct argument construction is error-prone | Forgetting `-l` interprets `Enter` / `C-c` / `Esc` as keys; forgetting `Enter` leaves the prompt hanging; mixing literal and key-name args in one call is not supported by tmux |
| No authorization boundary | `tmux send-keys -t %16 ...` works against any pane in any tmux session the OS user can reach, not just the Director's own members |
| Not covered by `permissions.allow` | Raw `tmux send-keys ...` invocations force per-call permission prompts; a dedicated `cafleet member send-input ...` command matches a single literal pattern |

### Feasibility verification (2026-04-15)

Run through a disposable Director/member pair on session `87fb26bd-...`. The member rendered an `AskUserQuestion` with options `1. red` / `2. green` / `3. blue` / `4. Type something.` / `5. Chat about this`. Three separate `tmux send-keys` calls:

1. `tmux send-keys -t %16 4` — jumped the cursor to option 4 and opened the text-input field
2. `tmux send-keys -t %16 -l "test"` — typed the literal string `test` into the field
3. `tmux send-keys -t %16 Enter` — submitted the prompt

The member ACK'd with `"ユーザー回答は 'test' (自由入力) でした"`. This confirms:

- Digit keys (`1`, `2`, `3`, `4`) select an option directly — no `↓` navigation needed
- The `-l` literal flag delivers free text as plain characters; shell meta, key names, and multi-byte sequences pass through untouched
- `Enter` as a separate call (no `-l`) is interpreted as the Return key

### Why "4" and not `↓ ↓ ↓`

With exactly three numbered choices, the "Type something" row is always at position 4 in Claude Code's `AskUserQuestion` UI. Pressing `4` directly:

- Is idempotent regardless of current cursor position (arrow navigation depends on where the cursor already is)
- Cannot over-shoot when the UI renders fewer rows than expected
- Matches the verified happy-path flow above

Arrow-key navigation would be needed only if the UI layout changes — out of scope for this doc. A future doc can extend the wrapper with an `--arrows N` mode if needed.

### Why a single-action contract (no newlines in free text)

`tmux send-keys -l "line1\nline2"` would deliver `line1`, a literal newline (which Claude's input treats as Enter), then `line2` — two submissions, only one of which is followed by the wrapper's explicit `Enter`. That splits the action into a half-submitted second prompt. Rejecting newlines keeps each CLI call = exactly one prompt submission, matching the mental model of "send one answer."

---

## Specification

### CLI surface

```
cafleet --session-id <s> member send-input --agent-id <d> --member-id <m>
    (--choice <1|2|3> | --freetext "<string>")
```

| Flag | Required | Notes |
|---|---|---|
| `--agent-id` | yes | The Director's agent ID. Used for the cross-Director authorization check. |
| `--member-id` | yes | The target member's agent ID. |
| `--choice` | one-of | Integer 1 / 2 / 3. Sends the matching digit key to the pane (no Enter). |
| `--freetext` | one-of | Free-text string to type into the "Type something" field. Sends `4`, then literal text, then `Enter`. |

### Key sequence sent to the pane

| Flag | Sequence |
|---|---|
| `--choice 1` | `tmux send-keys -t <pane> 1` |
| `--choice 2` | `tmux send-keys -t <pane> 2` |
| `--choice 3` | `tmux send-keys -t <pane> 3` |
| `--freetext "X"` | `tmux send-keys -t <pane> 4` **then** `tmux send-keys -t <pane> -l "X"` **then** `tmux send-keys -t <pane> Enter` |

Three separate tmux invocations for `--freetext` because tmux's `-l` (literal) flag is per-invocation: every key in a single `send-keys` call is either literal or key-name interpreted, never a mix. Splitting the sequence guarantees shell meta, key names (`Enter`, `C-c`, `Esc`), backslash-escapes, and multi-byte characters in the user's text are delivered as plain characters.

### Validation rules

| Input | Result |
|---|---|
| Neither `--choice` nor `--freetext` | Exit 2 with `Error: Must supply exactly one of --choice or --freetext.` |
| Both `--choice` and `--freetext` | Exit 2 with the same message |
| `--choice 0` / `--choice 4` / `--choice "a"` | Exit 2 via click's `IntRange(1, 3)` built-in validator |
| `--freetext ""` (empty) | Allowed. Sends `4` + empty literal + `Enter` (equivalent to submitting an empty answer). |
| `--freetext` containing `\n` or `\r` | Exit 2 with `Error: free text may not contain newlines.` |
| Any input with tmux unavailable | Exit 1 via `tmux.ensure_tmux_available()` (same error surface as `member capture`) |

### Authorization boundary

Mirrors `cafleet member capture` at `cafleet/src/cafleet/cli.py:795-843`:

1. Resolve the target via `broker.get_agent(member_id, session_id)`. If None, exit 1 with `Error: Agent <member_id> not found`.
2. Read `target["placement"]`. If None, exit 1 with `"agent <m> has no placement row; it was not spawned via 'cafleet member create'."`
3. If `placement["director_agent_id"] != agent_id`, exit 1 with `"agent <m> is not a member of your team (director_agent_id=<actual>)."`
4. If `placement.get("tmux_pane_id") is None`, exit 1 with `"member <m> has no pane yet (pending placement) — nothing to send."`

Reuse the exact error message shapes from `member_capture` so SKILL docs / operator muscle memory transfers.

### Click implementation sketch

```python
@member.command("send-input")
@click.option("--agent-id", required=True, help="Director's agent ID")
@click.option("--member-id", required=True, help="Target member's agent ID")
@click.option(
    "--choice",
    type=click.IntRange(1, 3),
    default=None,
    help="Select option 1, 2, or 3. Mutually exclusive with --freetext.",
)
@click.option(
    "--freetext",
    type=str,
    default=None,
    help='Send "4" + literal text + Enter. Mutually exclusive with --choice.',
)
@click.pass_context
def member_send_input(ctx, agent_id, member_id, choice, freetext):
    """Safely forward a restricted keystroke to a member pane."""
    from cafleet import tmux

    _require_session_id(ctx)
    session_id = ctx.obj["session_id"]

    if (choice is None) == (freetext is None):
        click.echo(
            "Error: Must supply exactly one of --choice or --freetext.",
            err=True,
        )
        ctx.exit(2)
        return

    if freetext is not None and ("\n" in freetext or "\r" in freetext):
        click.echo("Error: free text may not contain newlines.", err=True)
        ctx.exit(2)
        return

    try:
        tmux.ensure_tmux_available()
    except tmux.TmuxError as exc:
        click.echo(f"Error: {exc}", err=True)
        ctx.exit(1)
        return

    target = broker.get_agent(member_id, session_id)
    if target is None:
        click.echo(f"Error: Agent {member_id} not found", err=True)
        ctx.exit(1)
        return

    placement = target.get("placement")
    if placement is None:
        click.echo(
            f"Error: agent {member_id} has no placement row; it was not "
            f"spawned via `cafleet member create`.",
            err=True,
        )
        ctx.exit(1)
        return
    if placement["director_agent_id"] != agent_id:
        click.echo(
            f"Error: agent {member_id} is not a member of your team "
            f"(director_agent_id={placement['director_agent_id']}).",
            err=True,
        )
        ctx.exit(1)
        return
    pane_id = placement.get("tmux_pane_id")
    if pane_id is None:
        click.echo(
            f"Error: member {member_id} has no pane yet (pending placement) "
            f"— nothing to send.",
            err=True,
        )
        ctx.exit(1)
        return

    try:
        if choice is not None:
            tmux.send_choice_key(target_pane_id=pane_id, digit=choice)
            action, value = "choice", str(choice)
        else:
            tmux.send_freetext_and_submit(target_pane_id=pane_id, text=freetext)
            action, value = "freetext", freetext
    except tmux.TmuxError as exc:
        click.echo(f"Error: send failed: {exc}", err=True)
        ctx.exit(1)
        return

    if ctx.obj["json_output"]:
        click.echo(
            output.format_json(
                {
                    "member_agent_id": member_id,
                    "pane_id": pane_id,
                    "action": action,
                    "value": value,
                }
            )
        )
    else:
        label = f"choice {value}" if action == "choice" else "free text"
        click.echo(f"Sent {label} to member {target['name']} ({pane_id}).")
```

### `tmux.py` additions

Add two public helpers alongside the existing `send_exit` / `send_poll_trigger` / `capture_pane` functions. Each is a thin wrapper around `_run` so they can be unit-tested by monkey-patching `_run`.

```python
def send_choice_key(*, target_pane_id: str, digit: int) -> None:
    """Send a single digit key (1, 2, or 3) to a tmux pane. No Enter."""
    if digit not in (1, 2, 3):
        raise TmuxError(
            f"send_choice_key: digit must be 1, 2, or 3 (got {digit})"
        )
    _run(["tmux", "send-keys", "-t", target_pane_id, str(digit)])


def send_freetext_and_submit(*, target_pane_id: str, text: str) -> None:
    """Send "4" + literal text + Enter to a tmux pane.

    Three separate send-keys invocations because tmux's -l (literal) flag
    is per-invocation: one call cannot mix literal characters with the
    Enter key name. Splitting the sequence guarantees the literal text is
    delivered as plain characters (no shell-meta interpretation, no key
    name confusion for "Enter" / "C-c" / "Esc" embedded in the text).
    """
    if "\n" in text or "\r" in text:
        raise TmuxError(
            "send_freetext_and_submit: text may not contain newlines"
        )
    _run(["tmux", "send-keys", "-t", target_pane_id, "4"])
    _run(["tmux", "send-keys", "-t", target_pane_id, "-l", text])
    _run(["tmux", "send-keys", "-t", target_pane_id, "Enter"])
```

The newline-check is duplicated at two layers (CLI + helper) so callers that import the helper directly still get the guarantee.

### Output format

Text:

```
Sent choice 1 to member Claude-B (%7).
Sent free text to member Claude-B (%7).
```

JSON:

```json
{
  "member_agent_id": "<uuid>",
  "pane_id": "%7",
  "action": "choice",
  "value": "1"
}
```

```json
{
  "member_agent_id": "<uuid>",
  "pane_id": "%7",
  "action": "freetext",
  "value": "<user text as-sent>"
}
```

### Typical Director workflow (documentation only — not automated)

The CLI is deliberately one-shot; the surrounding choose-and-answer loop stays in the Director's control. Documented in `SKILL.md` as:

1. `cafleet --session-id <s> member capture --agent-id <d> --member-id <m> --lines 120` — read the current prompt options off the pane
2. Call `AskUserQuestion` on the end user with the observed labels
3. Based on the answer:
   - Option 1 / 2 / 3 → `cafleet --session-id <s> member send-input --agent-id <d> --member-id <m> --choice N`
   - Free-text → `cafleet --session-id <s> member send-input --agent-id <d> --member-id <m> --freetext "<user text>"`

Capture parsing is intentionally left manual — prompt layouts differ across Claude Code versions and other TUIs the wrapper may eventually target. The CLI's job is to *send* the restricted keystrokes safely; reading and presenting options is the Director's.

### Documentation updates

| File | Change |
|---|---|
| `ARCHITECTURE.md` | In the "Member commands" / tmux-integration section, add `send-input` alongside `capture` / `create` / `delete` / `list`. Mention the write-path authorization mirrors the read-path (`capture`). |
| `docs/spec/cli-options.md` | Add a `cafleet member send-input` subsection with the flag table, validation rules, authorization boundary, and text + JSON output samples. |
| `README.md` | In the CLI reference / member commands section, add a one-line bullet: "`cafleet member send-input` — forward a restricted keystroke (1/2/3 or free text) to a member pane." Link to the spec section. |
| `.claude/skills/cafleet/SKILL.md` | Add a "### Member Send-Input" subsection in Command Reference, between `Member Capture` and `Server`. Include both invocation forms, the flag table, output samples, and the "why three tmux calls" note. Also add the Director workflow snippet (capture → AskUserQuestion → send-input) under the "Multi-Session Coordination" section. |
| `.claude/skills/cafleet-monitoring/SKILL.md` | In "Stall Response" add a fourth step option: "If the member is blocked on an `AskUserQuestion` prompt, use `cafleet member send-input` to forward the operator's choice." Append to the response-channel table. |

### Tests

New file: `cafleet/tests/test_cli_member_send_input.py`. Mirror `test_cli_member.py` fixture style.

| Test class | Coverage |
|---|---|
| `TestFlagValidation` | (a) neither flag → exit 2 with the usage message; (b) both flags → exit 2; (c) `--choice 0` / `--choice 4` / `--choice "a"` → exit 2 via click; (d) `--freetext "line1\nline2"` → exit 2 with the newline message; (e) `--freetext ""` → accepted (dispatches to `send_freetext_and_submit` with empty text) |
| `TestAuthorizationBoundary` | With `broker.get_agent` monkey-patched: (a) missing agent → exit 1; (b) placement None → exit 1 with the "no placement row" message; (c) `director_agent_id` mismatch → exit 1 with the "not a member of your team" message; (d) `tmux_pane_id is None` → exit 1 with the "pending placement" message |
| `TestChoiceDispatch` | With `tmux.send_choice_key` monkey-patched to record args: `--choice 1` / `--choice 2` / `--choice 3` each dispatch once with matching digit and the resolved `tmux_pane_id` |
| `TestFreetextDispatch` | With `tmux.send_freetext_and_submit` monkey-patched to record args: `--freetext "hello"` dispatches with the exact text; `--freetext "$(echo pwn)"` dispatches with the literal string (no shell expansion — pinned by the subprocess.list-args contract); `--freetext "日本語 !@#"` dispatches with the exact multi-byte string |
| `TestOutputFormat` | Text output matches `"Sent choice 1 to member <name> (%7)."` / `"Sent free text to member <name> (%7)."`. JSON output contains the four documented keys. |
| `TestTmuxHelpers` | Direct unit tests for `tmux.send_choice_key` and `tmux.send_freetext_and_submit` with `_run` monkey-patched: choice helper rejects digits outside `{1,2,3}`; freetext helper rejects `"\n"` / `"\r"`; both record the exact `["tmux", "send-keys", ...]` argv. |

Every test uses `CliRunner` with `ctx.obj["session_id"]` set (reuse the pattern from `test_cli_session_flag.py`). No test spins up a real tmux server — all tmux calls are mocked at `tmux._run` or at the `send_*` helpers.

### Edge cases

| Case | Behavior |
|---|---|
| `--freetext` text equals a key name like `"Enter"` or `"C-c"` | Delivered as literal characters via `-l`, not interpreted as a key (verified by the `-l` flag semantics and pinned in `TestFreetextDispatch`) |
| `--freetext` contains `$VAR` / backtick / `$(...)` | Delivered as literal — Python's `subprocess.run([...], shell=False)` never invokes a shell |
| `--freetext ""` | Sends `4` + empty literal + `Enter`. Equivalent to pressing Enter on an empty text-input field — AskUserQuestion's own UI decides whether to accept or re-prompt |
| Member pane closed between validation and send (TOCTOU) | `tmux._run` raises `TmuxError` with tmux's `can't find pane` message; the handler surfaces it as `Error: send failed: ...` and exits 1 |
| `--json` with a successful send | JSON object with four keys (see "Output format") printed to stdout, nothing on stderr |
| Two `send-input` calls racing against the same pane | Not defensed. tmux serializes its own input; the second call's keys just land after the first. Callers who need strict serialization coordinate externally (same guarantee every other `member *` command gives). |
| Codex backend | Works identically — the Codex TUI's equivalent prompt has the same digit-key shortcut behavior. No backend sniffing in the CLI. |

---

## Implementation

> Documentation must be updated **before** any code change (per `.claude/rules/design-doc-numbering.md`).
> Task format: `- [x] Done task <!-- completed: 2026-04-15T14:30 -->`

### Step 1: Documentation — top-level docs

- [x] Update `ARCHITECTURE.md`: add `send-input` to the member-command enumeration and note it shares the `capture` authorization boundary. <!-- completed: 2026-04-16T09:15 -->
- [x] Update `docs/spec/cli-options.md`: add a `cafleet member send-input` subsection (flag table, validation rules, authorization boundary, text + JSON output samples). <!-- completed: 2026-04-16T09:20 -->
- [x] Update `README.md`: add the one-line bullet under member commands and link to the spec section. <!-- completed: 2026-04-16T09:22 -->

### Step 2: Documentation — skills

- [x] Update `.claude/skills/cafleet/SKILL.md`: add `### Member Send-Input` with both invocation forms, flag table, output samples, the "why three tmux calls" note, and the capture → AskUserQuestion → send-input workflow. <!-- completed: 2026-04-16T09:28 -->
- [x] Update `.claude/skills/cafleet-monitoring/SKILL.md`: add `send-input` to the Stall Response channels table and mention the AskUserQuestion case in the 2-stage health check. <!-- completed: 2026-04-16T09:30 -->

### Step 3: Code — tmux helpers

- [ ] Add `send_choice_key(*, target_pane_id: str, digit: int) -> None` to `cafleet/src/cafleet/tmux.py` with the digit-range check. <!-- completed: -->
- [ ] Add `send_freetext_and_submit(*, target_pane_id: str, text: str) -> None` to the same file with the newline check and three `_run` calls. <!-- completed: -->

### Step 4: Code — CLI subcommand

- [ ] Add `@member.command("send-input")` in `cafleet/src/cafleet/cli.py`, immediately after `member_capture`. Follow the Click sketch in the Specification. Lazy-import `cafleet.tmux` inside the handler to keep CLI startup cheap. <!-- completed: -->
- [ ] Verify `_require_session_id(ctx)` is called as the first line after kwargs are unpacked (every other `member *` command follows this pattern). <!-- completed: -->

### Step 5: Tests

- [ ] Create `cafleet/tests/test_cli_member_send_input.py` with the six test classes listed in the Specification. Use `CliRunner` + `monkeypatch` — never call a real tmux subprocess. <!-- completed: -->
- [ ] Extend `cafleet/tests/test_tmux.py` (or add a new `test_tmux_send_helpers.py`, matching the repo's per-concern test-file convention) with the `TestTmuxHelpers` cases. <!-- completed: -->
- [ ] Run `mise //cafleet:test` — must pass with zero failures. <!-- completed: -->

### Step 6: Quality gates

- [ ] Run `mise //:lint` — must pass. <!-- completed: -->
- [ ] Run `mise //:format` — must pass. <!-- completed: -->
- [ ] Run `mise //:typecheck` — must pass. <!-- completed: -->
- [ ] Manual smoke: spawn a disposable `cafleet member create` with an `AskUserQuestion`-producing prompt; run each of `--choice 1`, `--choice 2`, `--choice 3`, `--freetext "hello"`, `--freetext "日本語 $(echo pwn) 'quotes' \`backticks\`"`, `--freetext ""`; verify `cafleet member capture` shows each key landed correctly and the prompt submitted once per free-text call. Tear down with `cafleet member delete` + `cafleet deregister`. <!-- completed: -->

### Step 7: Finalize

- [ ] Update Status to Complete and refresh Last Updated. <!-- completed: -->
- [ ] Add a Changelog entry. <!-- completed: -->

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-04-15 | Initial draft. Feasibility verified via disposable Director/member pair earlier the same day: `tmux send-keys -t <pane> 4`, `-l "test"`, `Enter` successfully round-tripped through AskUserQuestion. |
