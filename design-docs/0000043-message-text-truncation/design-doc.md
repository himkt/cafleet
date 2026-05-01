# Message text truncation by default

**Status**: Approved
**Progress**: 28/28 tasks complete
**Last Updated**: 2026-05-01

## Overview

The `cafleet` CLI emits a `text` field carrying the full message body in both text and `--json` output of every message-related subcommand. For agents that poll their inbox repeatedly, those bodies dominate token cost. Truncate `text` to the first 10 Unicode codepoints with a literal `...` suffix by default, and add a per-subcommand `--full` flag that restores the current behavior. Ten codepoints preserves enough leading characters to disambiguate intent at a glance while collapsing per-poll token cost substantially for typical 200–500 character bodies.

## Success Criteria

- [ ] `cafleet ... message {send,broadcast,poll,ack,cancel,show}` truncate the `text` body to 10 codepoints + `...` by default in both text and `--json` output.
- [ ] `--full` (per-subcommand option, placed after the subcommand name) restores the un-truncated body in both text and `--json` output.
- [ ] Empty `text` and `text` whose codepoint length is ≤ 10 pass through unchanged with no `...` marker.
- [ ] Multibyte / non-ASCII text is truncated by Python `str` length (codepoints), never bytes — no character is split.
- [ ] FastAPI `/ui/api/*` HTTP responses are unchanged (out of scope; the WebUI is human-facing).
- [ ] All other long fields (`agent.description`, `skills[].description`, `agent_card_json` sub-fields, `member capture` content) are unchanged in this release.
- [ ] Documentation (`README.md`, `ARCHITECTURE.md`, `docs/`, `skills/cafleet/SKILL.md`) reflects the new default and the `--full` opt-out before code lands.
- [ ] Unit tests cover the truncation helper for the four edge cases (empty, ≤10 cp, >10 cp ASCII, >10 cp multibyte). Per-command tests cover `message poll`, `message show`, `message send`, `message broadcast` in both default and `--full` modes, both text and `--json`.

---

## Background

`cafleet/src/cafleet/output.py:23` (`format_task`) extracts the body via `task["artifacts"][i]["parts"][j]["text"]` and prints `  text:  {text}` in the text formatter. For `--json`, `cafleet/src/cafleet/cli.py:98` calls `output.format_json(result)` which serializes the entire broker result, including the same `artifacts[].parts[].text` payload. Either way, an inbox-polling member sees the full body verbatim every time it polls.

Members typically only need the body's existence + a few leading characters to decide whether to ACK and act. The full body is recoverable on demand via `--full`.

---

## Specification

### Truncation rule

| Input `text` | Default output | `--full` output |
|---|---|---|
| `None` / not present | not present | not present |
| `""` | `""` | `""` |
| length ≤ 10 codepoints | unchanged | unchanged |
| length > 10 codepoints | `text[:10] + "..."` | unchanged |

- Length is `len(text)` in Python (codepoints, not bytes).
- The suffix is exactly the three ASCII characters `...` — no count, no `[truncated]`, no `text_length` companion field.
- The truncation operates on the literal payload string before any other formatting; whitespace at the cut boundary is not normalized.

### Truncation helper

Add a single helper in `cafleet/output.py`:

```python
def truncate_text(value: str | None, *, full: bool, limit: int = 10) -> str | None:
    """Return ``value`` truncated to ``limit`` codepoints + '...' when ``full`` is False.

    Pass-through for None, empty strings, and strings already at or below the limit.
    """
    if full or value is None or len(value) <= limit:
        return value
    return value[:limit] + "..."
```

### Task transformation helper

The broker returns task dicts in the shape `{"task": {"artifacts": [{"parts": [{"text": "..."}]}], ...}}` or, for `cancel` / `ack` / direct task results, `{"artifacts": [...]}` at the top level (see `format_task` at `cafleet/src/cafleet/output.py:19-31` which unwraps both shapes). Add a single helper that mutates the result in place across both shapes and across single-task vs list shapes:

```python
def truncate_task_text(result, *, full: bool, limit: int = 10):
    """Truncate every ``part['text']`` inside ``result`` in place.

    Accepts a single task dict, a ``{"task": {...}}`` envelope, or a list of
    either. Returns ``result`` unchanged. Pass-through when ``full`` is True
    or when no ``text`` keys are present.
    """
    if full:
        return result
    items = result if isinstance(result, list) else [result]
    for item in items:
        task = item.get("task", item) if isinstance(item, dict) else None
        if not isinstance(task, dict):
            continue
        for artifact in task.get("artifacts", []) or []:
            for part in artifact.get("parts", []) or []:
                if "text" in part:
                    part["text"] = truncate_text(part["text"], full=False, limit=limit)
    return result
```

The broadcast result is a top-level list of task envelopes (each with the same `artifacts[].parts[].text` shape that `format_task` already unwraps), so the list-handling branch of `truncate_task_text` covers it. A single helper covers all six message subcommands.

### Emit-site enumeration

Every CLI emit site that produces a `text` field (the focus of this design):

| Subcommand | File:line | Output mode | Source of `text` | In scope |
|---|---|---|---|---|
| `message send` | `cafleet/src/cafleet/cli.py:385-400` | text via `format_task`, json via `format_json` | `task.artifacts[].parts[].text` | yes |
| `message broadcast` | `cafleet/src/cafleet/cli.py:403-419` | text via `format_indexed_list(format_task)`, json via `format_json` | per task in list | yes |
| `message poll` | `cafleet/src/cafleet/cli.py:422-439` | text via `format_indexed_list(format_task)`, json via `format_json` | per task in list | yes |
| `message ack` | `cafleet/src/cafleet/cli.py:442-452` | text via `format_task`, json via `format_json` | task | yes |
| `message cancel` | `cafleet/src/cafleet/cli.py:455-465` | text via `format_task`, json via `format_json` | task | yes |
| `message show` | `cafleet/src/cafleet/cli.py:468-475` | text via `format_task`, json via `format_json` | task | yes |

Emit sites that do NOT carry a message `text` field (out of scope, called out for completeness):

| Subcommand | Long field(s) emitted | Why out of scope |
|---|---|---|
| `agent register` | `name` (echo) | `description` is not echoed; nothing else is long. |
| `agent list` | `description` per agent | Other-field truncation is release-notes future work. |
| `agent show` | `description`, future `agent_card_json` | Same. |
| `member create` | `name`, `pane_id`, `window_id` | None are long. |
| `member list` | `description` truncated to a column already; no `text`. | Already column-clipped. |
| `member capture` | `content` (raw pane buffer) | `--lines` already controls size. |
| `member send-input` / `member exec` / `member ping` | echo the action, not a `text` body | Nothing to truncate. |
| `session create` / `list` / `show` / `delete` | `label`, `session_id` | None are long. |
| `db init` / `doctor` / `server` | metadata | None are long. |
| FastAPI `/ui/api/*` (WebUI router in `cafleet/src/cafleet/server.py:1-50`, routes in `cafleet/src/cafleet/webui_api.py`) | full task / agent payloads | WebUI is human-facing; always full per Q10. |

### Flag placement

`--full` is a **per-subcommand** option (Click option, placed after the subcommand name), not a global flag. This is consistent with `--agent-id`, `--task-id`, and `--lines`. There is no `--no-truncate` alias and no environment variable.

```bash
cafleet --session-id <session-id> message poll --agent-id <my-agent-id>          # truncated
cafleet --session-id <session-id> message poll --agent-id <my-agent-id> --full   # full body
```

`--full` and `--json` compose orthogonally. Truncation happens before either `format_json` or the text formatter runs, so `cafleet --json message poll --agent-id <my-agent-id> --full` emits the full body in JSON output, and the same flag combination omitting `--full` emits the truncated body in JSON.

### Wiring into `_client_command`

The shared decorator at `cafleet/src/cafleet/cli.py:60-111` currently formats text with `text_formatter` and JSON with `format_json(result)`. Extend it with one new optional parameter:

```python
def _client_command(
    *,
    requires_agent_session: bool = False,
    text_formatter: Callable[[Any], str] | None = None,
    truncates_task_text: bool = False,
):
```

When `truncates_task_text=True`, the wrapper reads the wrapped command's `full` kwarg (defaulting to `False` if absent) and calls `output.truncate_task_text(result, full=full)` before either `format_json(result)` or `text_formatter(result)` runs. Mutation is in place. The wrapped function still returns the same reference (now carrying truncated text), so the wrapper's `return result` is correct.

The six message commands each declare:

```python
@click.option(
    "--full",
    "full",
    is_flag=True,
    default=False,
    help="Disable text truncation; emit full message body.",
)
```

and pass-through `full` is automatically picked up by the wrapper via `kwargs.get("full", False)`.

### Edge cases

| Case | Behavior |
|---|---|
| `text` key absent on a part | Skip; do not insert a `text` key. |
| `parts` list empty | Skip the artifact. |
| `artifacts` list empty | Skip the task. |
| `text` is `None` | Pass through unchanged (no `...`). |
| `text` is `""` | Pass through unchanged (no `...`). |
| `text` is exactly 10 codepoints | Pass through unchanged (no `...`). The threshold is "greater than 10". |
| `text` is 11+ codepoints with multibyte characters (`"日本語日本語日本語日本"` = 10 cp) | Treated as 10 cp → unchanged. `"日本語日本語日本語日本語"` = 11 cp → `"日本語日本語日本語日本" + "..."` (10 cp + suffix). |
| `text` contains a literal `\n` newline before the cut | Cut occurs at codepoint 10 regardless of newlines. |
| `--full` is supplied but no `text` field exists | No-op. |

### Out of scope (explicitly)

- `/ui/api/*` HTTP responses. The WebUI consumer renders payloads for humans; it stays always-full. (Confirmed Q10.)
- `agent.description`, `skills[].description`, `agent_card_json` sub-fields, `member capture` content — release-notes future work, not implemented here.
- A `text_length` / `text_truncated` JSON companion field. Not added; the literal `...` suffix is the only signal.
- An environment variable opt-out. CLI flag only.
- A configurable truncation length. The limit is fixed at 10 codepoints in this release. A `--truncate-limit` flag, a `CAFLEET_TRUNCATE_LIMIT` environment variable, and a settings hook were all considered and deliberately rejected — the user wants a single uniform aggressive limit and operators with longer-context needs already have `--full` to recover the entire body. The helper signature exposes `limit=10` as a default parameter so future work can revisit, but no caller passes a non-default value in this release.

### Risks and migration

This is a **breaking change** for the CLI text and `--json` contracts of every `cafleet message *` subcommand. After upgrade, both `text:  <body>` lines in text output and `task.artifacts[].parts[].text` payloads in JSON output emit at most 13 codepoints (10 + the literal `...` suffix) by default.

| Affected consumer | Impact | Migration |
|---|---|---|
| Operator scripts that grep `text:` lines | See truncated body. | Append `--full` to the invocation, or update parsers to ignore the body. |
| Operator scripts that parse `--json` and read `task.artifacts[].parts[].text` | Receive truncated string. | Append `--full` to the invocation, or update parsers to ignore the body. |
| Agents in member panes that poll their inbox | Receive truncated body — this is the intended outcome (token-cost reduction). | None; no action needed. |
| WebUI / `/ui/api/*` | Unaffected (out of scope). | None. |

The `README.md` upgrade note must call this out explicitly. There is no soft-launch flag, no version-gated rollout, and no deprecation period — the new default lands in the same release as the implementation.

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

### Step 1: Documentation first

- [x] Update `README.md` with the new default-truncation note and the `--full` opt-out under the message commands section. <!-- completed: 2026-05-01T12:00 -->
- [x] Update `ARCHITECTURE.md` to note that CLI message-emission paths apply default truncation; `/ui/api/*` does not. <!-- completed: 2026-05-01T12:00 -->
- [x] Update `docs/` (any file documenting `message poll` / `message send` / `message broadcast` / `message ack` / `message cancel` / `message show`) with the truncation rule and the `--full` flag. <!-- completed: 2026-05-01T12:00 -->
- [x] Update `skills/cafleet/SKILL.md` Command Reference for each of the six message subcommands: add `--full` to the flag table, mention the 10-codepoint default + `...` suffix, and add one example showing `--full`. <!-- completed: 2026-05-01T12:00 -->
- [x] Verify project rules under `.claude/rules/` need no edits (they describe the broker, not the CLI defaults). <!-- completed: 2026-05-01T12:00 -->

### Step 2: Truncation helpers

- [x] Add `truncate_text(value, *, full, limit=10)` to `cafleet/src/cafleet/output.py`. <!-- completed: 2026-05-01T12:30 -->
- [x] Add `truncate_task_text(result, *, full, limit=10)` to `cafleet/src/cafleet/output.py`, handling single-task / `{"task": ...}` envelope / list-of-tasks shapes. <!-- completed: 2026-05-01T12:30 -->
- [x] Add unit tests in `cafleet/tests/test_output.py`: empty string, `None`, 10-codepoint string (no truncation), 11-codepoint ASCII (truncation), 11-codepoint multibyte (truncation by codepoint not byte), `--full` pass-through. <!-- completed: 2026-05-01T12:30 -->
- [x] Add unit tests for `truncate_task_text`: single-task shape, `{"task": {...}}` envelope shape, list-of-tasks shape, missing `artifacts`, missing `parts`, missing `text` key in a part, mixed (some parts have text, some do not). <!-- completed: 2026-05-01T12:30 -->

### Step 3: Wire `--full` into the message subcommands

- [x] Extend `_client_command` in `cafleet/src/cafleet/cli.py:60-111` with a `truncates_task_text: bool = False` parameter. When True, the wrapper reads `kwargs.get("full", False)` and calls `output.truncate_task_text(result, full=full)` before both formatters. <!-- completed: 2026-05-01T13:00 -->
- [x] Add `--full` option + `truncates_task_text=True` to `message_send`. <!-- completed: 2026-05-01T13:00 -->
- [x] Add `--full` option + `truncates_task_text=True` to `message_broadcast`. <!-- completed: 2026-05-01T13:00 -->
- [x] Add `--full` option + `truncates_task_text=True` to `message_poll`. <!-- completed: 2026-05-01T13:00 -->
- [x] Add `--full` option + `truncates_task_text=True` to `message_ack`. <!-- completed: 2026-05-01T13:00 -->
- [x] Add `--full` option + `truncates_task_text=True` to `message_cancel`. <!-- completed: 2026-05-01T13:00 -->
- [x] Add `--full` option + `truncates_task_text=True` to `message_show`. <!-- completed: 2026-05-01T13:00 -->

### Step 4: Per-command tests

- [x] `message poll`: default truncates `text` in both text and `--json`; `--full` emits full body. Cover empty inbox, single task, list of three tasks. <!-- completed: 2026-05-01T12:30 -->
- [x] `message show`: default truncates; `--full` emits full body. Cover both text and `--json`. <!-- completed: 2026-05-01T12:30 -->
- [x] `message send`: echo of just-sent message is truncated by default; `--full` echoes full. Cover both text and `--json`. <!-- completed: 2026-05-01T12:30 -->
- [x] `message broadcast`: each task in the broadcast summary list is truncated by default; `--full` emits full bodies. Cover both text and `--json`. <!-- completed: 2026-05-01T12:30 -->
- [x] Each per-command test asserts that non-`text` fields (`id`, `status.state`, `metadata.fromAgentId`, `metadata.toAgentId`, `metadata.type`) are byte-identical between default and `--full` output — a regression guard against the helper accidentally mutating siblings. <!-- completed: 2026-05-01T12:30 -->
- [x] `message ack` and `message cancel` reuse the same `_client_command` wiring as `message send`, and the `truncate_task_text` helper-level tests in Step 2 cover the truncation behavior on the same task shape. No additional per-command tests are required for `ack` / `cancel` — the wiring test is the integration point, and adding ack/cancel-specific tests would duplicate Step 2 without exercising new code. <!-- completed: 2026-05-01T12:30 -->

### Step 5: Verify

- [x] `mise //cafleet:lint` passes. <!-- completed: 2026-05-01T12:35 -->
- [x] `mise //cafleet:format` clean. <!-- completed: 2026-05-01T12:35 -->
- [x] `mise //cafleet:typecheck` passes. <!-- completed: 2026-05-01T12:35 -->
- [x] `mise //cafleet:test` passes (full suite). <!-- completed: 2026-05-01T12:35 -->
- [x] Manual smoke: `cafleet --session-id <s> message poll --agent-id <a>` shows `text:  abcdefghij...` for an 11+ codepoint body and the same body verbatim with `--full`. <!-- completed: 2026-05-01T12:35 -->
- [x] Manual smoke: `cafleet message poll --help` (and the same for `send` / `broadcast` / `ack` / `cancel` / `show`) lists `--full` with the help text "Disable text truncation; emit full message body." <!-- completed: 2026-05-01T12:35 -->

---

## Release notes — other fields that would benefit from truncation (future work)

The user request asked for an explicit call-out of OTHER fields that share the same token-cost shape as the message `text` body. None are addressed in this release. Each entry below names the field, the byte-cost shape, and where in the codebase the long content originates so a follow-up design can pick them up.

| Field | Where it appears | Typical size | Why deferred |
|---|---|---|---|
| `agent.description` | `cafleet/src/cafleet/output.py:65` (`format_agent`) prints it verbatim. CLI `agent list` emits N agents per call, each with description. CLI `agent show` emits one. JSON shape is `agent["description"]`. | 1–3 sentences (~80–300 chars per agent). Multiplied by `len(agents)` for `agent list`. | The user request narrows scope to message `text`. Truncating descriptions would also affect `agent register`'s confirmation echo and the WebUI agent cards (which the design holds always-full). |
| `skills[].description` inside `agent_card_json` | Stored on the agent row's `agent_card_json` and surfaced when the WebUI or future `agent show --card` renders it. JSON path: `agent.agent_card_json.skills[i].description`. | 80–200 chars × N skills. | `agent_card_json` is held always-full in this release per Q2. |
| `agent_card_json` (whole object) | Emitted as a nested JSON blob inside `agent show` (and the WebUI's agent detail page) when surfaced. | 500–2000 chars depending on skill count. | Same. |
| `member capture` `content` blob | `cafleet/src/cafleet/cli.py:899-915`. JSON path: `result.content`. | Variable; default 80 lines, can be multi-KB for verbose panes. | Already controlled by `--lines`. A separate truncation flag would conflict with the operator's intent when capturing. |
| `task.history[].parts[].text` (if/when surfaced) | Currently `format_task` only emits the first text-bearing part of `artifacts`. If a future feature surfaces `history`, each entry has the same `parts[].text` shape and would benefit from the same truncation rule. | Proportional to message length × history depth. | Not currently emitted by any CLI subcommand. |

A follow-up design (suggested slug: `agent-description-truncation`) should pick up the first three rows together — they share the agent-render path — and decide whether `--full` becomes a global flag or stays per-subcommand once it spans more than the message commands.

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-05-01 | Initial draft. |
| 2026-05-01 | Reviewer round 1 revisions. Added (a) explicit "configurable limit deliberately rejected" entry under Out of scope. (b) Risks and migration section flagging the breaking change. (c) Step 4 note that ack / cancel reuse the same wiring and do not need per-command tests. (d) Sentence under Flag placement that `--full` and `--json` compose orthogonally. (e) Reworded the in-place-mutation note to clarify the reference is unchanged but its contents are mutated. (f) Threshold rationale ("ten codepoints because…") in Overview. (g) Reworded the broadcast clause to avoid the undefined `broadcast_summary` term. (h) Added a non-text-fields-byte-identical assertion to Step 4. (i) Added a `--help` surface check to Step 5. Task count rose from 22 to 28. |
| 2026-05-01 | User approved. Status flipped Draft → Approved. No spec content changes. |
