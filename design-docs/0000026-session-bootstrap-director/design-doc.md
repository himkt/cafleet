# Auto-bootstrap root Director on session create

**Status**: Approved
**Progress**: 6/35 tasks complete
**Last Updated**: 2026-04-16

## Overview

Make `cafleet session create` atomically create the session, register a root Director agent, and write a placement row pointing at the current tmux pane — all in a single DB transaction. This guarantees every session has exactly one root Director with a placement, which in turn enables Member → Director tmux push notifications (the broker's existing notification path keys off the recipient's placement row).

## Success Criteria

- [ ] `cafleet session create` completes 4 operations (INSERT sessions, INSERT agents, INSERT agent_placements, UPDATE sessions.director_agent_id) in one DB transaction
- [ ] Failure in any step rolls back the whole transaction — no partial session/agent/placement rows persist
- [ ] Root Director's placement has `director_agent_id IS NULL`
- [ ] Running `cafleet session create` outside tmux fails with `Error: cafleet session create must be run inside a tmux session` and exit code 1 (no DB changes)
- [ ] After `cafleet session create`, sending a message from a member to the root Director triggers a tmux push notification to the Director's pane
- [ ] `cafleet session delete` logically deletes the session, deregisters all active agents in it (including the root Director), and physically deletes their placement rows — tasks are preserved
- [ ] `cafleet session delete` is idempotent: re-running on an already-deleted session is a no-op that prints `Deregistered 0 agents.` and exits 0
- [ ] `cafleet register` on a soft-deleted session fails with `Error: session <id> is deleted`
- [ ] `cafleet session list` hides soft-deleted sessions
- [ ] `cafleet deregister` against the root Director agent_id fails with an explicit error (see G3 resolution in Specification)

---

## Background

`cafleet session create` currently only inserts a row into `sessions` ([broker.py:69-83](../../cafleet/src/cafleet/broker.py)). Callers then manually invoke `cafleet register` to create a Director agent, and there is no CLI path that writes an `agent_placements` row for a root Director. Because root Directors lack a placement, the broker's notification helper `_try_notify_recipient` ([broker.py:35-61](../../cafleet/src/cafleet/broker.py)) silently skips them — push notifications therefore only work Director → Member, never Member → Director.

The fix is to fold Director bootstrap into `session create` itself. The `agent_placements.director_agent_id` column is set to `NULL` for the root Director (it has no parent). The broker's notification code already resolves a pane by `agent_id` only, so bidirectional push notification starts working automatically once the root Director has a placement row.

See [design-doc 0000020](../0000020-tmux-push-notification/design-doc.md) for the push notification mechanism this change completes.

---

## Specification

### Schema changes (migration 0007)

| Table | Change | Nullable | Default | Notes |
|---|---|---|---|---|
| `sessions` | ADD COLUMN `deleted_at` (String) | yes | NULL | NULL = active, non-NULL ISO8601 timestamp = soft-deleted |
| `sessions` | ADD COLUMN `director_agent_id` (String) | yes (DB) | NULL | App-enforced NOT NULL after bootstrap. FK → `agents.agent_id`, `ondelete=RESTRICT` |
| `agent_placements` | DROP NOT NULL on `director_agent_id` | yes | — | NULL value represents "this placement is for a root Director (no parent)" |

Rationale for DB-level nullability on `sessions.director_agent_id` (Q3 resolution):

- The 4-step bootstrap inserts `sessions` first (with `director_agent_id=NULL`), then `agents` (whose `session_id` FK requires the `sessions` row to exist), then `agent_placements`, then UPDATEs `sessions.director_agent_id`.
- Post-bootstrap invariant: every non-deleted `sessions` row has a non-NULL `director_agent_id`. Enforced by broker code, not by a schema NOT NULL constraint.
- The alternative (`PRAGMA defer_foreign_keys=ON`) was rejected as more intrusive.

`idx_placements_director` is unchanged — SQLite indexes handle NULL keys (NULLs are grouped but do not violate uniqueness in a non-unique index).

### Transactional contract for `broker.create_session`

```python
def create_session(label: str | None, director_context: DirectorContext) -> dict:
    """Atomically create a session + root Director + placement.

    All four ops run in a single ``with session.begin():`` block.
    Any exception inside triggers automatic SQLAlchemy rollback.
    ``director_context`` is read from tmux BEFORE the transaction opens,
    so any tmux failure surfaces without touching the DB.
    """
```

Constants used inside this function:

```python
_DIRECTOR_NAME = "director"
_DIRECTOR_DESCRIPTION = "Root Director for this session"
# FIXME(claude): auto-detect from $CLAUDECODE / $CLAUDE_CODE_ENTRYPOINT / codex env vars.
_ROOT_DIRECTOR_CODING_AGENT = "unknown"
```

Transaction order (exact sequence inside one `with session.begin():` block):

1. INSERT `sessions` (session_id=new_uuid, label=label, created_at=now, deleted_at=NULL, director_agent_id=NULL)
2. INSERT `agents` (agent_id=new_uuid, session_id=session.session_id, name="director", description="Root Director for this session", status="active", registered_at=now, agent_card_json=...)
3. INSERT `agent_placements` (agent_id=agent.agent_id, director_agent_id=NULL, tmux_session=ctx.session, tmux_window_id=ctx.window_id, tmux_pane_id=ctx.pane_id, coding_agent="unknown", created_at=now)
4. UPDATE `sessions` SET director_agent_id=agent.agent_id WHERE session_id=session.session_id

Return shape (matches Q6 JSON spec exactly):

```json
{
  "session_id": "…",
  "label": "…",
  "created_at": "…",
  "administrator_agent_id": "…",
  "director": {
    "agent_id": "…",
    "name": "director",
    "description": "Root Director for this session",
    "registered_at": "…",
    "placement": {
      "director_agent_id": null,
      "tmux_session": "main",
      "tmux_window_id": "@3",
      "tmux_pane_id": "%0",
      "coding_agent": "unknown",
      "created_at": "…"
    }
  }
}
```

Note: `agent_placements.coding_agent` has `server_default="claude"`. The INSERT must explicitly set `coding_agent="unknown"` to override that default for the root Director.

Note: `administrator_agent_id` is preserved from design 0000025 (built-in Administrator seeded per session). The Administrator seeding step runs inside the same transaction after the Director bootstrap — the 4-step transactional contract becomes a 5-step one: (1) INSERT sessions, (2) INSERT Director agent, (3) INSERT Director placement, (4) UPDATE sessions.director_agent_id, (5) INSERT Administrator agent with `agent_card_json.cafleet.kind == "builtin-administrator"`. Rollback covers all five steps.

### CLI surface — `cafleet session create`

```
cafleet session create [--label LABEL] [--json]
```

| Flag | Required | Notes |
|---|---|---|
| `--label` | no | Optional human-readable label |
| `--json` | no | Emit the full nested JSON shape from above |

No `--name` / `--description` flags. Director name and description are hardcoded.

Text (non-JSON) output:

```
<session_id>
<director_agent_id>
label:            <label or empty>
created_at:       <iso8601>
director_name:    director
pane:             <tmux_session>:<tmux_window_id>:<tmux_pane_id>
administrator:    <administrator_agent_id>
```

Line 1 is `session_id` (preserves backward-compatible script usage that parses only the first line). Line 2 is the Director's `agent_id`. The `administrator:` line exposes the built-in Administrator's `agent_id` (seeded per 0000025) so callers can address the Administrator without a second `cafleet agents` call.

### CLI surface — `cafleet session delete`

```
cafleet session delete <session_id>
```

Behavior (all in one `with session.begin():` block, idempotent):

1. `UPDATE sessions SET deleted_at=now WHERE session_id=X AND deleted_at IS NULL`
2. `UPDATE agents SET status='deregistered', deregistered_at=now WHERE session_id=X AND status='active'`
3. `DELETE FROM agent_placements WHERE agent_id IN (SELECT agent_id FROM agents WHERE session_id=X)`
4. Tasks untouched (audit history preserved).

Output:

```
Deleted session <session_id>. Deregistered N agents.
```

`N` counts **all** agents that were active in the session at the moment of deletion — this explicitly includes the root Director. On re-run, `N=0` because step 1's `WHERE deleted_at IS NULL` clause short-circuits the cascade (idempotent).

No `--force` flag. Re-running on an already-deleted session prints `Deleted session X. Deregistered 0 agents.` and exits 0.

Error on unknown `session_id`: exit 1 with `Error: session 'X' not found.` (same wording as `session show`).

### CLI surface — `cafleet session list`

Default behavior: hide soft-deleted sessions (`WHERE deleted_at IS NULL`). No `--all` flag in this revision.

### `broker.register_agent` change

`get_session` is **unchanged** — it still returns the row regardless of `deleted_at` (and now exposes `deleted_at` in the returned dict). Only `list_sessions` learns the `deleted_at IS NULL` filter. `register_agent` inspects the returned `deleted_at` value itself and distinguishes the two error paths:

```python
sess = get_session(session_id)
if sess is None:
    raise click.UsageError(f"Session '{session_id}' not found.")
if sess["deleted_at"] is not None:
    raise click.UsageError(f"session {session_id} is deleted")
```

User-visible errors:

| Condition | Error |
|---|---|
| `session_id` row does not exist | `Session 'X' not found.` |
| `session_id` row exists with non-NULL `deleted_at` | `session X is deleted` |

### `cafleet register` and `cafleet member create` — no behavior changes

- `cafleet register` — unchanged at CLI layer. Only the broker-side `register_agent` gains the soft-delete guard.
- `cafleet member create` — no changes. It already fetches `director_context()` and creates a child placement row.

### `cafleet member list` — root Director excluded from its own list

The root Director's placement has `director_agent_id=NULL`. `broker.list_members` filters by `AgentPlacement.director_agent_id == director_agent_id` — so `cafleet member list --agent-id <root_director>` returns an empty list. This is the desired semantics: "members I spawned" excludes self. No code change required.

### `cafleet deregister` on the root Director — rejected

Manually running `cafleet deregister --agent-id <root-director-id>` would leave `sessions.director_agent_id` pointing at a deregistered agent with no placement, breaking Member → Director notifications. `broker.deregister_agent` therefore detects the root-Director case and refuses:

```python
# Inside deregister_agent, before the UPDATE:
sess = session.execute(
    select(Session.director_agent_id).where(Session.director_agent_id == agent_id)
).first()
if sess is not None:
    raise click.UsageError(
        "cannot deregister the root Director; use `cafleet session delete` instead"
    )
```

Text error: `Error: cannot deregister the root Director; use 'cafleet session delete' instead.` Exit code 1. This corresponds to option (b) from the G3 feedback — an explicit rejection that points the user at the correct operation. `session delete` performs the same deregistration as part of its cascade, so no functionality is removed.

### Member tmux panes after `session delete` — surviving pane orphaning is intentional

`session delete` logically deletes the session and its placement rows, but does **not** send `/exit` to the surviving member panes. The rationale: `session delete` is a bulk teardown and must not depend on tmux being reachable for every member pane. Directors who want a clean shutdown call `cafleet member delete` on each member first (which does send `/exit`), then call `session delete`. The orphaned `claude`/`codex` processes remain alive with no placement row, their next `cafleet` call will fail on the soft-deleted session, and the user/Director can terminate them manually with `tmux kill-pane`.

This is explicitly an Out-of-Scope item for graceful member shutdown — see below.

### tmux availability check — CLI translates the error

`session_create` reuses `tmux.ensure_tmux_available()` (which today raises `TmuxError("cafleet member commands must be run inside a tmux session")`). `tmux.ensure_tmux_available()` itself is **not modified**. The `session_create` CLI handler catches `TmuxError` and prints the session-create-specific message `Error: cafleet session create must be run inside a tmux session` before exiting with code 1.

### `coding_agent` "unknown" — validation surface check (U3 resolution)

The only validator on `coding_agent` values today is `click.Choice(["claude", "codex"])` on the `--coding-agent` flag of `cafleet member create` ([cli.py:526](../../cafleet/src/cafleet/cli.py)). That validator is on **input** for user-facing pane spawning and does not fire for the root Director path (which writes `"unknown"` internally via `broker.create_session`). Output / rendering code (`output.py:82`, `output.py:111`) and the WebUI API do **not** assert on specific values. Therefore `"unknown"` can be stored and read back safely without widening any allowed-value set.

### Error handling

| Failure | Behavior |
|---|---|
| `TMUX` env var unset or `tmux` binary missing | Exit 1 with `Error: cafleet session create must be run inside a tmux session` before any DB work |
| `tmux display-message` fails inside `director_context()` | Same tmux-error path, exit 1, no DB work |
| Exception during any of the 4 INSERT/UPDATE ops | `with session.begin():` rolls back automatically — no partial rows |
| User runs `session delete` on a non-existent session_id | Exit 1 with `Error: session 'X' not found.` |
| User runs `session delete` twice | Second run is idempotent: "Deleted session X. Deregistered 0 agents." |
| User runs `register` into a soft-deleted session | Exit 1 with `Error: session X is deleted` |

### Notification correctness (regression check)

After this change, the broker's existing `_try_notify_recipient` ([broker.py:35-61](../../cafleet/src/cafleet/broker.py)) finds the root Director's placement row when a Member sends to the Director. `send_poll_trigger` then injects a `cafleet poll` command into the Director's pane. No modification to `_try_notify_recipient` is needed.

### Out of scope

- Auto-detection of `coding_agent` at session create (deferred to a future change via `FIXME(claude)` comment)
- Non-tmux session creation (CI / WebUI)
- Backward compatibility with the developer-local `registry.db`. Migration 0007 is **structural only**: it adds columns but does not backfill `sessions.director_agent_id` for pre-existing rows. Any pre-existing sessions row would violate the app-level NOT NULL invariant on `director_agent_id`. Developers MUST remove the local registry database (`rm ~/.local/share/cafleet/registry.db`, or whatever path `CAFLEET_DATABASE_URL` points at) before running `cafleet db init` against the new migration. This is a manual step, not automated.
- `--force` flag on `session delete`
- Resurrection / un-deletion of soft-deleted sessions
- Multiple Directors per session or moving the root Director's pane after creation
- Graceful shutdown of member `claude`/`codex` processes during `session delete`. Surviving-pane orphaning is intentional — see the "Member tmux panes after `session delete`" subsection above. Directors should call `cafleet member delete` per member before `session delete` for a clean teardown.

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

### Step 1: Documentation (must complete before code)

- [x] Update `ARCHITECTURE.md` to describe the 4-step transactional session bootstrap and the logical-delete session model <!-- completed: 2026-04-16T09:10 -->
- [x] Update `README.md` `cafleet session create` / `cafleet session delete` usage and output examples to reflect the new JSON/text shapes <!-- completed: 2026-04-16T09:15 -->
- [x] Update `docs/spec/cli-options.md` with the new CLI surface (no `--name` / `--description`, Director hardcoded) <!-- completed: 2026-04-16T09:20 -->
- [x] Update `docs/spec/data-model.md` with `sessions.deleted_at`, `sessions.director_agent_id`, and the relaxed `agent_placements.director_agent_id` nullability <!-- completed: 2026-04-16T09:05 -->
- [x] Update `.claude/skills/cafleet/SKILL.md` `session create` example to show the new nested JSON output (with `director` + `placement` sub-objects) and document the soft-delete semantics on `session delete` <!-- completed: 2026-04-16T09:25 -->
- [x] Scan `.claude/skills/cafleet-monitoring/SKILL.md`, `.claude/skills/cafleet-design-doc-create/SKILL.md`, `.claude/skills/cafleet-design-doc-execute/SKILL.md`, and their `roles/*.md` for any `session create` / `session delete` examples and update them to match the new output <!-- completed: 2026-04-16T09:35 -->

### Step 2: Migration 0007

- [ ] Create `cafleet/src/cafleet/alembic/versions/0007_session_bootstrap_director.py` with `revision="0007"`, `down_revision="0006"` <!-- completed: -->
- [ ] In `upgrade()` use `op.batch_alter_table("sessions")` to add `deleted_at` (String, nullable) and `director_agent_id` (String, nullable, FK to `agents.agent_id`, `ondelete="RESTRICT"`) <!-- completed: -->
- [ ] In `upgrade()` use `op.batch_alter_table("agent_placements")` to set `director_agent_id` nullable=True <!-- completed: -->
- [ ] Implement `downgrade()` reversing the two `batch_alter_table` calls <!-- completed: -->

### Step 3: SQLAlchemy models (`cafleet/src/cafleet/db/models.py`)

- [ ] `Session`: add `deleted_at: Mapped[str | None]` and `director_agent_id: Mapped[str | None]` (ForeignKey `agents.agent_id`, `ondelete="RESTRICT"`) <!-- completed: -->
- [ ] `AgentPlacement`: change `director_agent_id` to `Mapped[str | None]` (nullable) <!-- completed: -->

### Step 4: Broker bootstrap logic (`cafleet/src/cafleet/broker.py`)

- [ ] Add module-level constants `_DIRECTOR_NAME = "director"`, `_DIRECTOR_DESCRIPTION = "Root Director for this session"`, `_ROOT_DIRECTOR_CODING_AGENT = "unknown"` with the FIXME(claude) comment next to the coding-agent constant <!-- completed: -->
- [ ] Rewrite `create_session(label, director_context)` to perform the 5-step transactional bootstrap (INSERT sessions, INSERT Director agent, INSERT Director placement, UPDATE sessions.director_agent_id, INSERT built-in Administrator agent per 0000025) and return the nested `{session_id, label, created_at, administrator_agent_id, director: {…, placement: {…}}}` dict shape <!-- completed: -->
- [ ] Extend `get_session` to include `deleted_at` in its returned dict (do NOT add a `WHERE deleted_at IS NULL` filter — callers must inspect the field themselves) <!-- completed: -->
- [ ] Update `list_sessions` to filter `WHERE sessions.deleted_at IS NULL` <!-- completed: -->
- [ ] Update `register_agent` to inspect `get_session(...)["deleted_at"]` and reject soft-deleted sessions with `session X is deleted` while keeping the `Session 'X' not found.` path intact <!-- completed: -->
- [ ] Update `deregister_agent` to reject the root-Director case with `cannot deregister the root Director; use 'cafleet session delete' instead` (detect via `Session.director_agent_id == agent_id`) <!-- completed: -->
- [ ] Rewrite `delete_session` to perform the 3-step logical-delete cascade (UPDATE sessions SET deleted_at, UPDATE agents SET status=deregistered, DELETE placements) in one transaction, returning `{"deregistered_count": N}` (N counts the root Director too) for the CLI to render <!-- completed: -->

### Step 5: CLI (`cafleet/src/cafleet/cli.py`)

- [ ] `session_create`: call `tmux.ensure_tmux_available()` and `tmux.director_context()` before broker call. On any `TmuxError` print `Error: cafleet session create must be run inside a tmux session` and exit 1 <!-- completed: -->
- [ ] `session_create`: call the new `broker.create_session(label, director_context)` and render the 2-line + human-friendly text output (or JSON when `--json` is set) <!-- completed: -->
- [ ] `session_delete`: render `Deleted session X. Deregistered N agents.` using the broker return value <!-- completed: -->

### Step 6: Output helpers (`cafleet/src/cafleet/output.py`)

- [ ] Add / update formatter(s) for the new `session create` text shape (session_id, director agent_id, label, created_at, director_name, pane, administrator) <!-- completed: -->

### Step 7: Tests

- [ ] New file `cafleet/tests/test_session_bootstrap.py`: mock `tmux.director_context()` and verify a successful `broker.create_session` writes one session row, two agent rows (Director + built-in Administrator), one Director placement row, and updates `sessions.director_agent_id` — all in one transaction <!-- completed: -->
- [ ] In the same file: verify partial-failure rollback by injecting an exception after step 2 (INSERT agents) and confirming no sessions/agents/placement rows persist <!-- completed: -->
- [ ] In the same file: verify `broker.delete_session` marks `deleted_at`, deregisters all active agents (including the root Director), deletes their placements, preserves tasks, and is idempotent on re-run <!-- completed: -->
- [ ] In the same file: verify `broker.register_agent` rejects a soft-deleted session with the exact `session X is deleted` error string <!-- completed: -->
- [ ] In the same file: verify `broker.list_sessions` filters out rows with non-NULL `deleted_at` <!-- completed: -->
- [ ] In the same file: verify `broker.deregister_agent` on the root-Director `agent_id` raises the `cannot deregister the root Director` error and leaves all rows unchanged <!-- completed: -->
- [ ] In the same file: Member → Director notification path — bootstrap a session, register a member with a placement (pane_id set), call `broker.send_message(session_id, member_agent_id, to=root_director_agent_id, text="hi")` with `tmux.send_poll_trigger` patched to a `Mock(return_value=True)`, assert the response's `notification_sent` is `True` and `send_poll_trigger` was called with the root Director's `tmux_pane_id` <!-- completed: -->
- [ ] Extend `cafleet/tests/test_cli.py` (or `test_cli_session.py` if already present): `session create` text output — line 1 == `session_id`, line 2 == director `agent_id`, subsequent lines contain `label:`, `director_name: director`, `pane:` <!-- completed: -->
- [ ] `session create --json` produces the nested `{session_id, label, created_at, administrator_agent_id, director: {agent_id, name, description, registered_at, placement: {...}}}` shape with `placement.director_agent_id` null, `placement.coding_agent` == `"unknown"`, and `administrator_agent_id` non-null and matching the seeded Administrator's `agent_id` <!-- completed: -->
- [ ] `session list` hides rows whose `deleted_at IS NOT NULL` by default (no `--all` flag is accepted) <!-- completed: -->
- [ ] `session create` outside tmux (unset `TMUX`) exits 1 with `Error: cafleet session create must be run inside a tmux session` and writes nothing to the DB <!-- completed: -->
- [ ] `session delete` on an unknown `session_id` exits 1 with `Error: session 'X' not found.` <!-- completed: -->

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-04-15 | Initial draft |
| 2026-04-15 | Reviewer round 1: fix register_agent pseudocode (I1), add root-Director deregister rejection (G3), specify member-pane orphaning (G4), clarify tmux error translation (G5), clarify developer-local DB wipe (U1), clarify `N` in session-delete output includes root Director (U2), confirm `coding_agent="unknown"` has no validator on the bootstrap path (U3), add tests for Member→Director notification (G1), `list_sessions` filtering, `session delete` not-found, and split the CLI test bucket (G2, IM4). Added idempotency Success Criteria (IM1). Note to script authors: the `session create` text output now contains content on line 2+ (director `agent_id`, label, created_at, director name, pane); scripts that parse only line 1 remain compatible. |
| 2026-04-15 | Approved by user. Status: Approved. |
| 2026-04-16 | Director: bump migration slot from 0006 to 0007 (0006 is taken by 0006_seed_administrator_agent from design 0000025). Preserve administrator_agent_id from 0000025 additively in both JSON and text output of session create. create_session is now a 5-step transactional bootstrap (adds INSERT built-in Administrator). Step 7 tests updated accordingly. |
