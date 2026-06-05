# CLI ID/UX Cleanup: Prefix Resolution, Session-List Director, Drop `--pretty`

**Status**: Approved
**Progress**: 34/34 tasks complete
**Last Updated**: 2026-06-05T13:19

## Overview

Three independent CLI ID/UX fixes shipped as one coherent change: (a)+(c) let the ID **input** options accept a unique prefix so an 8-char displayed ID can be pasted straight into the next command; (b) add the missing `director_agent_id` to `session list` output; (c) delete the global `--pretty` flag entirely. Workstreams (a)+(c)-prefix, (b)-field, and `--pretty`-removal are distinct root causes implemented as separate steps.

## Success Criteria

- [x] A unique prefix of an agent ID is accepted for `--to` (message send), `--id` (agent show), and `--member-id` (member delete/capture/send-input/exec/ping); a unique prefix of a task ID is accepted for `--task-id` (message ack/cancel/show).
- [x] A full UUID continues to be accepted on every one of those inputs (no regression).
- [x] The acting `--agent-id` is **not** prefix-resolved; passing a prefix is rejected by the existing acting-agent validation (a `requires_agent_session` command yields the "not a member of session" error; `message send` yields the "Sender agent not found or not active" error).
- [x] An ambiguous prefix and a no-match prefix each exit `1` with a distinct, actionable message.
- [x] Prefix resolution is scoped to the supplied `--session-id` (agents active in the session; tasks with an endpoint in the session), so a prefix in another session is invisible.
- [x] `cafleet --json session list` and the text `session list` both expose `director_agent_id` (full UUID).
- [x] `--pretty` no longer exists anywhere in source, docs, skills, or tests; JSON output is compact-only; the repo reads as if `--pretty` never existed (no deprecation notice).
- [x] `mise //cafleet:format`, `lint`, `typecheck`, and `test` are all green.

---

## Background

**(a)+(c) — truncation without round-trip.** Human-facing and default JSON output truncate IDs to 8 chars: task id `output.py:81`, agent id `output.py:130`, session-create director/admin `output.py:256-257`, member id `output.py:283`. But every **input** option resolves by exact full-UUID equality in `broker.py`: `Task.task_id == task_id` (`broker.py:785`), `Agent.agent_id == to` (`broker.py:854`), and `send_message` even parse-gates `to` with `uuid.UUID(to)` (`broker.py:841-843`), rejecting an 8-char prefix before any lookup. So a value the CLI just printed cannot be pasted back.

**(b) — a field that was never selected.** `list_sessions` (`broker.py:215-247`) selects only `session_id` / `label` / `created_at` / `agent_count`. `sessions.director_agent_id` exists in the schema (and `get_session` returns it), but `session list` never projects it, so the root Director's ID cannot be recovered from a list after `session create` scrolls away. This is a missing-field root cause, not truncation — `session list` has no `--full` flag and `--full` would not help.

**(c) — `--pretty`.** A global flag (before the subcommand) switching JSON from compact to `indent=2`. It is removed completely. Per `config/claude/rules/removal.md`, after removal the repository must read as if `--pretty` never existed: no deprecation notices, no "for history" pointers. This design doc and git history are the only record.

---

## Specification

### Workstream A: ID prefix resolution (covers brief items a + c)

#### Decisions (confirmed with the user)

| Question | Decision |
|---|---|
| Which inputs accept a prefix? | **Targets only**: `--to`, `--id` (agent show), `--member-id`, `--task-id`. The acting `--agent-id` stays full-UUID-only. |
| Scope across sessions? | **Session-scoped**: resolve only among rows visible to the supplied `--session-id`. Task lookup for ack/cancel is tightened to session-visible tasks as a consequence. |
| Minimum prefix length? | **No floor.** Any prefix that uniquely matches resolves; a full UUID is always accepted. |
| Failure behavior? | Ambiguous prefix **and** no-match both raise `ValueError` in the broker, surfaced as a `ClickException` (exit `1`) with distinct messages. |

Rationale for excluding the acting `--agent-id`: an agent always knows its own full UUID (returned at `register`, baked into the spawn prompt), so the acting ID is never re-typed from an 8-char display. Prefixes are pasted from `agent list`, poll envelopes, and `session create` — all **target** IDs.

#### Shared resolver (`broker.py`)

One private core plus two entity-specific public wrappers. The core does an exact-match short-circuit **before** any prefix scan, so a full UUID returns immediately and is never reported ambiguous.

```python
def _resolve_id_prefix(session, *, id_column, base_where, ref: str, entity: str) -> str:
    """Resolve ``ref`` (a full UUID or unique prefix) to a full id.

    1. Exact-match short-circuit: if a row has ``id_column == ref`` within
       ``base_where``, return ``ref`` unchanged. A full UUID always takes this
       path (an 8-char prefix cannot equal a 36-char id, so it falls through).
    2. Prefix scan: rows where ``id_column`` STARTS WITH ``ref`` —
       ``id_column.startswith(ref, autoescape=True)`` so ``%`` / ``_`` in a
       malformed ``ref`` are matched literally, never as LIKE wildcards —
       bounded by ``base_where`` and ``.limit(2)``:
         0 rows  -> raise ValueError(f"no {entity} matches id '{ref}' in this session.")
         1 row   -> return that full id
         2 rows  -> raise ValueError(
                        f"id prefix '{ref}' is ambiguous; "
                        f"supply more characters or the full UUID.")
    """

def resolve_agent_ref(session_id: str, ref: str) -> str:
    """Full agent UUID or unique prefix -> full agent_id, scoped to ACTIVE
    agents in ``session_id``."""
    # base_where = (Agent.session_id == session_id, Agent.status == "active")
    # id_column  = Agent.agent_id ; entity = "agent"

def resolve_task_ref(session_id: str, ref: str) -> str:
    """Full task UUID or unique prefix -> full task_id, scoped to tasks with at
    least one endpoint agent in ``session_id`` (mirrors get_task visibility)."""
    # base_where = (exists().where(
    #     Agent.session_id == session_id,
    #     or_(Agent.agent_id == Task.from_agent_id,
    #         Agent.agent_id == Task.to_agent_id)),)
    # id_column = Task.task_id ; entity = "task"
```

Notes:
- `.limit(2)` keeps resolution to a single bounded query even for a pathologically short prefix; the operator's remedy ("type more characters") is the same regardless of the exact match count, so the message omits a count.
- No new imports are needed: `broker.py:8` already imports everything the resolver uses — `from sqlalchemy import and_, delete, exists, func, or_, select, update`.
- The agent base_where uses `status == "active"` (matches `get_agent`); the task base_where mirrors `get_task`'s endpoint-in-session visibility (status-agnostic), so a task that is resolvable is exactly a task that is showable.

#### Call sites

Resolution happens at the CLI boundary — each handler turns the user-supplied ref into a full id, then passes the full id to the existing broker function. Broker entry points (`send_message`, `get_agent`, `ack_task`, `cancel_task`, `get_task`) keep operating on full ids exactly as today.

| Input | Subcommand(s) | Edit location | Resolver |
|---|---|---|---|
| `--to` | `message send` | `message_send` handler body, before `broker.send_message` | `resolve_agent_ref` |
| `--id` | `agent show` | `agent_show` handler, before `broker.get_agent` | `resolve_agent_ref` |
| `--member-id` | `member delete` / `capture` / `send-input` / `exec` / `ping` | top of `_load_authorized_member` (single shared helper covering all five) | `resolve_agent_ref` |
| `--task-id` | `message ack` / `cancel` / `show` | each handler, before `broker.ack_task` / `cancel_task` / `get_task` | `resolve_task_ref` |

Exit-code wiring:
- `message_send`, `agent_show`, `message_ack`, `message_cancel`, `message_show` are wrapped by `_client_command`; a `ValueError` raised inside the decorated function is caught by its `except Exception` branch and re-raised as `ClickException` → **exit 1**.
- `_load_authorized_member` is a plain helper that already raises `ClickException`; wrap the resolver `ValueError` → `ClickException` there → **exit 1**.

Behavioral consequences (intended):
- `send_message`'s `uuid.UUID(to)` gate now always passes (the handler hands it a resolved full id); the gate stays as cheap defense-in-depth for direct callers. No broker signature changes.
- `ack_task` / `cancel_task` keep their global `_read_task`, but the `task_id` they receive was resolved within session visibility, so cross-session task IDs are rejected at resolution time — satisfying the "tighten to session-visible tasks" decision without changing those signatures.
- The acting `--agent-id` is never prefix-resolved, so a prefix there is rejected by the existing acting-agent validation — but the exact message differs by command. On the `requires_agent_session=True` commands (`agent show`, `message ack` / `cancel` / `show`), `_client_command` runs `verify_agent_session` before the handler and yields `Error: agent <prefix> is not a member of session <sid>.`. On `message send` (not `requires_agent_session`-wrapped, cli.py:552-560), validation happens inside `broker.send_message` via `_agent_is_active_in_session` (broker.py:847), yielding `Error: Sender agent not found or not active in session: <prefix>`. Criterion #3 (acting id never prefix-resolved) holds in both cases; only the error string differs.

#### Error messages (final wording)

| Situation | Message (printed as `Error: <msg>`) | Exit |
|---|---|---|
| Agent prefix matches >1 active agent in session | `id prefix '<ref>' is ambiguous; supply more characters or the full UUID.` | 1 |
| Agent prefix / full UUID matches 0 in session | `no agent matches id '<ref>' in this session.` | 1 |
| Task prefix matches >1 visible task in session | `id prefix '<ref>' is ambiguous; supply more characters or the full UUID.` | 1 |
| Task prefix / full UUID matches 0 visible task | `no task matches id '<ref>' in this session.` | 1 |

### Workstream B: `session list` exposes `director_agent_id`

- `broker.list_sessions` (`broker.py:215-247`): add `Session.director_agent_id` to the `select(...)` columns (and to `group_by` for portability — it is functionally determined by `session_id`), and add `"director_agent_id": row.director_agent_id` to the returned dict. JSON output then carries the field automatically.
- `cli.session_list` text mode: add a `DIRECTOR` column rendering the **full** director UUID (decision Q4), placed immediately after `SESSION_ID`:

  ```
  SESSION_ID                               DIRECTOR                                 LABEL                AGENTS   CREATED_AT
  ```

  Header: `f"{'SESSION_ID':<40} {'DIRECTOR':<40} {'LABEL':<20} {'AGENTS':<8} {'CREATED_AT'}"`.
  Row: `f"{r['session_id']:<40} {r['director_agent_id'] or '':<40} {r['label'] or '':<20} {r['agent_count']:<8} {r['created_at']}"`.

  Full UUID (not an 8-char prefix) because the value is pasted into `--agent-id`, which stays full-only per Workstream A, and `session list` already prints the full 40-char `session_id`.

Out of scope: `session show` text already omits `director_agent_id` (a pre-existing, separate inconsistency) — not touched here.

### Workstream C: remove `--pretty` entirely

JSON output becomes compact-only. `output.format_json` loses its `pretty` parameter and the `indent=2` branch; there is no replacement flag and no deprecation note. Full removal surface (verified by repo grep, excluding `design-docs/`, `node_modules/`, and the generated untracked `site/`):

| File | What to remove / change |
|---|---|
| `cafleet/src/cafleet/cli.py` | The `@click.option("--pretty", ...)` block (lines 154-160); the `pretty` parameter of `cli()` and `ctx.obj["pretty"] = pretty` (line 173); every `pretty=` argument on `output.format_json(...)` calls. In the per-command handlers these read as the literal `pretty=ctx.obj["pretty"]` (`session_create`, `session_list`, `session_show`, `doctor`, `base_dir_resolve`, `base_dir_record`, `member_create`, `member_delete` ×2, `member_list`, `member_capture`, `member_send_input`, `member_exec`, `member_ping`). Inside `_client_command` the form differs — a local read `pretty = ctx.obj["pretty"]` (cli.py:114) consumed as `pretty=pretty` (cli.py:123, 133); remove the local and both uses. Every call becomes `output.format_json(<data>)`. |
| `cafleet/src/cafleet/output.py` | `format_json`: drop the `*, pretty: bool = False` parameter and the `if pretty:` branch; keep only `json.dumps(data, separators=(",", ":"))`; remove the `pretty` mention from the docstring. |
| `docs/spec/cli-options.md` | Remove the `--pretty` row from the Global Options table (line 25); drop "pair with `--pretty` for indented output" from the `--json` row (line 24). |
| `docs/spec/message-envelope.md` | Remove the `--json --pretty` table row (line 73); delete the `--pretty` example subsection (lines 86-97); drop `--pretty` from the `--full` example command (line 99); reword the JSON-output intro (line 68) to "governed by the `--json` flag"; remove the `--pretty` cross-reference bullet (line 138). |
| `docs/concepts/token-reduction.md` | Reword the Surface row (line 18) to drop "New global `--pretty` flag;" while keeping "default JSON output is compact"; add a one-line note that the 8-char IDs are now pasteable via prefix resolution. |
| `skills/cafleet/SKILL.md` | Lines 11, 17, 64, 69, 74, 288: remove all `--pretty` mentions (global-options list, the `--json --pretty` example, the legacy-flags pointer, the "pair with `--pretty`" clause). Add a short note that target ID inputs accept a unique prefix (see Workstream A docs task). |
| `skills/cafleet/reference/legacy-flags.md` | Retitle line 1 and reword line 3 to drop `--pretty`; delete the `## --pretty` section (lines 16-23); in the `## --json` section remove the "Compose with `--pretty`" sentence (line 27) and the two `--pretty` example lines (31, 33). The file remains (it still documents `--full` / `--json` / `--quiet` / `CAFLEET_MAX_TEXT_LEN`). |
| `skills/cafleet/reference/director.md` | Line 236: drop `--pretty` from the opt-back-in pointer. |
| `skills/cafleet/roles/director.md` | Line 13: drop `--pretty` from the opt-back-in pointer. |
| `cafleet/tests/test_cli_pretty_flag.py` | Delete the whole file (the dedicated `--pretty` test). |
| `cafleet/tests/test_cli_client_command.py` | Remove the test harness's own `--pretty` option, `pretty` param, and `ctx.obj["pretty"]` plumbing (lines 24, 27, 31). |
| `cafleet/tests/test_output_render_task.py` | Drop `pretty=` kwargs (lines 131, 230, 245, 255); delete `test_format_json__pretty_indented_and_longer` (134-141); rewrite the budget test (225) against a **compact full** baseline (see below); update the module docstring (line 4). |
| `cafleet/tests/token_budget/test_envelope_size.py` | Drop `pretty=` kwargs (lines 98, 116, 165); rewrite `test_compact_slim_envelope_at_most_30pct_of_pretty_full` against a **compact full** baseline (see below); update the module docstring (line 3). |

Token-budget test rewrite: both tests currently prove "compact slim envelope ≤ 30% of the **pretty full** envelope," using `pretty=True` as the verbose baseline. With `--pretty` gone, recompose the comparison as compact-slim vs **compact-full** (the full typed-column, untruncated envelope rendered with the same compact `format_json`). The regression intent (slim+projected is materially smaller than full) is preserved, but the ratio threshold must be recomputed empirically during implementation — render both, read the actual ratio, and set the assertion threshold with headroom above it. Rename the tests to drop "pretty".

Removal-rule compliance: no `--pretty` deprecation note anywhere. One regression guard is permitted and added — a test asserting `cafleet --pretty ...` exits 2 with Click's built-in `No such option: --pretty` — because that tests the **absence** of the flag, not a deprecated→error sentinel.

Not in the commit surface:
- `site/search.json` — generated by the Zensical docs build and untracked (git-ignored); regenerated, never hand-edited.
- The promoted copies under `~/.claude/skills/cafleet/` — out-of-band copies of the repo skill, re-synced by the promotion flow after merge; the repo `skills/cafleet/` files are the canonical edit target here.

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

### Step 1: Documentation first (per `.claude/rules/design-doc-numbering.md`)

- [x] `docs/spec/cli-options.md`: add an "ID prefix resolution" subsection — which inputs accept a prefix (targets only), session scope, exact-match short-circuit, ambiguous/not-found messages + exit 1, full-UUID still accepted, acting `--agent-id` excluded. <!-- completed: 2026-06-05T11:56 -->
- [x] `docs/spec/cli-options.md`: in the `session list` section, document the new `director_agent_id` JSON field and the full-UUID `DIRECTOR` text column. <!-- completed: 2026-06-05T11:56 -->
- [x] `docs/spec/cli-options.md`: add the four prefix-resolution rows to the Error Messages table. <!-- completed: 2026-06-05T11:56 -->
- [x] `docs/spec/cli-options.md`: remove the `--pretty` Global Options row and the "pair with `--pretty`" note. <!-- completed: 2026-06-05T11:56 -->
- [x] `docs/spec/message-envelope.md`: remove all `--pretty` content (table row, example subsection, `--full` example command, JSON-output intro, cross-ref bullet). <!-- completed: 2026-06-05T11:56 -->
- [x] `docs/concepts/token-reduction.md`: reword the Surface row to drop `--pretty`; add the prefix-resolution-round-trip note. <!-- completed: 2026-06-05T11:56 -->
- [x] `skills/cafleet/SKILL.md`: remove all `--pretty` mentions (lines 11, 17, 64, 69, 74, 288); add a short note that `--to` / `--id` / `--member-id` / `--task-id` accept a unique prefix. <!-- completed: 2026-06-05T11:56 -->
- [x] `skills/cafleet/reference/legacy-flags.md`: retitle, remove the `--pretty` section and the `--pretty` mentions in the `--json` section. <!-- completed: 2026-06-05T11:56 -->
- [x] `skills/cafleet/reference/director.md` + `skills/cafleet/roles/director.md`: drop `--pretty` from the opt-back-in pointers. Also add a one-line note in `reference/director.md` that `--member-id` accepts a unique prefix (Director-side member commands are where 8-char member IDs from `member list` get pasted). <!-- completed: 2026-06-05T11:56 -->
- [x] `README.md`: verify no `--pretty` drift and that no session-list/ID example needs the new director field or prefix note (expected no-op; reflect if any exists). <!-- completed: 2026-06-05T11:56 -->

### Step 2: Workstream A — ID prefix resolution

- [x] `broker.py`: add `_resolve_id_prefix(session, *, id_column, base_where, ref, entity)` with exact short-circuit + bounded prefix scan + distinct `ValueError`s (no new imports — `and_`, `or_`, `exists`, `select` already imported). <!-- completed: 2026-06-05T12:44 -->
- [x] `broker.py`: add `resolve_agent_ref(session_id, ref)` (active agents in session). <!-- completed: 2026-06-05T12:44 -->
- [x] `broker.py`: add `resolve_task_ref(session_id, ref)` (tasks with an endpoint in session). <!-- completed: 2026-06-05T12:44 -->
- [x] `cli.py` `message_send`: resolve `--to` via `broker.resolve_agent_ref` before `broker.send_message`. <!-- completed: 2026-06-05T12:44 -->
- [x] `cli.py` `agent_show`: resolve `--id` via `broker.resolve_agent_ref` before `broker.get_agent`. <!-- completed: 2026-06-05T12:44 -->
- [x] `cli.py` `_load_authorized_member`: resolve `--member-id` via `broker.resolve_agent_ref` at the top, and have all five member handlers use the **resolved** id (`target["agent_id"]`) for downstream operations (`deregister_agent`, tmux dispatch, output) — reassigning the helper's local param does NOT propagate to the caller, so a pasted prefix would otherwise still reach `broker.deregister_agent(member_id)` (cli.py:1081) and "succeed" without deregistering. Wrap the resolver `ValueError` → `ClickException` so its **raw** message surfaces (`Error: id prefix … is ambiguous` / `no agent matches id …`), NOT the generic get_agent "failed to fetch member" wrapper. Covers all five member subcommands. <!-- completed: 2026-06-05T12:44 -->
- [x] `cli.py` `message_ack` / `message_cancel` / `message_show`: resolve `--task-id` via `broker.resolve_task_ref` before the broker call. <!-- completed: 2026-06-05T12:44 -->
- [x] Broker tests: exact full-UUID returns unchanged; unique prefix resolves; ambiguous prefix → `ValueError`; no-match → `ValueError`; a row in another session is invisible; a `%`/`_` ref is matched literally (autoescape). <!-- completed: 2026-06-05T12:44 -->
- [x] CLI tests: prefix accepted on `--to` / `--id` / `--member-id` / `--task-id`; full UUID still accepted on each; ambiguous → exit 1 with the ambiguous message; no-match → exit 1 with the no-match message; a prefix on the acting `--agent-id` is rejected — assert against a `requires_agent_session` command (e.g. `agent show`) for the exact `not a member of session` wording, so the test is independent of `message send`'s different "Sender agent not found" string. <!-- completed: 2026-06-05T12:44 -->

### Step 3: Workstream B — `session list` director field

- [x] `broker.list_sessions`: select `Session.director_agent_id` (+ `group_by`) and include it in the row dict. <!-- completed: 2026-06-05T12:53 -->
- [x] `cli.session_list`: add the full-UUID `DIRECTOR` column to the text header and rows. <!-- completed: 2026-06-05T12:53 -->
- [x] Tests: `--json session list` includes `director_agent_id`; text output shows the `DIRECTOR` column with the full director UUID. <!-- completed: 2026-06-05T12:53 -->

### Step 4: Workstream C — remove `--pretty`

- [x] `cli.py`: remove the `--pretty` option, the `pretty` param, `ctx.obj["pretty"]`, and every `pretty=ctx.obj["pretty"]` argument. <!-- completed: 2026-06-05T13:12 -->
- [x] `output.py`: make `format_json` compact-only (drop the `pretty` param, the `indent=2` branch, and the docstring mention). <!-- completed: 2026-06-05T13:12 -->
- [x] Delete `cafleet/tests/test_cli_pretty_flag.py`. <!-- completed: 2026-06-05T13:12 -->
- [x] `cafleet/tests/test_cli_client_command.py`: remove the harness `--pretty` option / param / `ctx.obj` plumbing. <!-- completed: 2026-06-05T13:12 -->
- [x] `cafleet/tests/test_output_render_task.py`: drop `pretty=` kwargs; delete the pretty-indent test; rewrite the budget test against a compact-full baseline (recompute threshold, rename); update the module docstring. <!-- completed: 2026-06-05T13:12 -->
- [x] `cafleet/tests/token_budget/test_envelope_size.py`: drop `pretty=` kwargs; rewrite the slim-vs-full budget test against a compact-full baseline (recompute threshold, rename); update the module docstring. <!-- completed: 2026-06-05T13:12 -->
- [x] Add a single regression guard: `cafleet --pretty ...` exits 2 with Click's `No such option: --pretty` (tests the absence). <!-- completed: 2026-06-05T13:12 -->

### Step 5: Verification

- [x] `mise //cafleet:format`, `mise //cafleet:lint`, `mise //cafleet:typecheck` all clean. <!-- completed: 2026-06-05T13:19 -->
- [x] `mise //cafleet:test` green. <!-- completed: 2026-06-05T13:19 -->
- [x] Repo grep for `pretty` (excluding `design-docs/`, `node_modules/`, `site/`, `.git/`) returns zero matches in `cafleet/src`, `docs`, `skills`, and `cafleet/tests`. <!-- completed: 2026-06-05T13:19 -->
- [x] Manual smoke: `agent list` → paste an 8-char agent id into `message send --to <prefix>` (resolves); an ambiguous prefix exits 1; `session list` (text and `--json`) shows the director ID. <!-- completed: 2026-06-05T13:19 — verified via in-process CliRunner suite (test_cli_prefix_resolution + test_session_list_director + test_cli_version), no separate real-binary run -->
- [x] Stage the design doc with the implementation commits (project git-workflow override: `design-docs/` is committed here). <!-- completed: 2026-06-05T13:19 -->

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-06-05 | Initial draft |
