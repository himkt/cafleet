# SQLite Data Model Specification

All core data structures — `Task`, `Message`, `Part`, `Artifact`, `AgentCard`, `TaskStatus`, `TaskState` — follow an internal A2A-inspired shape, but CAFleet does not maintain Pydantic models for them. There is no dependency on `a2a-sdk` or any external A2A library. SQLite stores the `Task` and `AgentCard` payloads as JSON `TEXT` blobs that the broker layer serializes via `json.dumps` and reads back as plain Python dicts via `json.loads` (see `cafleet/src/cafleet/broker.py`). Broker-specific information (routing metadata, etc.) lives in indexed columns alongside the JSON blob.

The model is a **relational + document hybrid**: indexed fields are columns, while the A2A-inspired payloads are stored as opaque JSON `TEXT`. The columns are queried; the JSON blobs are not.

Schema management is handled by Alembic (`cafleet/src/cafleet/alembic/`); the runtime engine is SQLAlchemy 2.x with the synchronous `pysqlite` driver (see `cafleet/src/cafleet/db/engine.py`'s `get_sync_engine` / `get_sync_sessionmaker`). Operators apply migrations once via `cafleet db init` before starting the server.

## SQL Schema

### `sessions`

| Column | Type | Constraints | Notes |
|---|---|---|---|
| `session_id` | `TEXT` | `PRIMARY KEY` | Opaque string. New sessions receive a UUIDv4; migrated sessions reuse the original `api_key_hash` value (64-char hex). |
| `label` | `TEXT` | nullable | Optional free-form text for human bookkeeping (e.g. `"PR-42 review"`). |
| `created_at` | `TEXT` | `NOT NULL` | ISO-8601 timestamp. |
| `deleted_at` | `TEXT` | nullable | `NULL` = active; non-NULL ISO-8601 timestamp = soft-deleted. Written by `broker.delete_session`; never cleared. |
| `director_agent_id` | `TEXT` | nullable (DB), app-enforced NOT NULL after bootstrap; `REFERENCES agents(agent_id) ON DELETE RESTRICT` | Points at the session's root Director (the agent auto-registered by `cafleet session create`). DB-nullable so the 4-step bootstrap can INSERT `sessions` before the Director's `agents` row exists; post-bootstrap every non-deleted session has a non-NULL value. |

Session deletion is a **soft-delete**: `broker.delete_session` sets `deleted_at=now`, then deregisters every active agent in the session (including the root Director) and physically deletes their `agent_placements` rows in the same transaction. Tasks are preserved. Re-running `session delete` on an already-soft-deleted session is a no-op that reports `Deregistered 0 agents.` because the initial `UPDATE` guard `WHERE deleted_at IS NULL` short-circuits the cascade.

`broker.get_session` always returns the row (regardless of `deleted_at`) and exposes the field so callers can decide; `broker.list_sessions` filters `WHERE deleted_at IS NULL`, so `cafleet session list` hides soft-deleted sessions (no `--all` flag in v1). `broker.register_agent` inspects `get_session(...)["deleted_at"]` and rejects a soft-deleted session with `Error: session X is deleted` (distinct from the `Session 'X' not found.` path).

#### Root Director bootstrap

`cafleet session create` executes a single transaction that performs five ordered operations — all-or-nothing:

1. `INSERT INTO sessions (...)` with `deleted_at=NULL`, `director_agent_id=NULL`.
2. `INSERT INTO agents (...)` for the hardcoded root Director (`name='Director'`, `description='Root Director for this session'`, `status='active'`).
3. `INSERT INTO agent_placements (...)` for the Director with `director_agent_id=NULL` (the root has no parent Director) and `coding_agent='unknown'` (auto-detection is deferred via a `FIXME(claude)` comment).
4. `UPDATE sessions SET director_agent_id = <director's agent_id> WHERE session_id = <new>`.
5. `INSERT INTO agents (...)` for the built-in `Administrator` (per design 0000025) with `agent_card_json.cafleet.kind == 'builtin-administrator'`. The Administrator never gets an `agent_placements` row.

The `tmux` context (`session`, `window_id`, `pane_id`) is read **before** the transaction opens, so any tmux failure surfaces as `Error: cafleet session create must be run inside a tmux session` and exit 1 without touching the DB. Rollback covers all five operations — a failure in any step leaves the DB unchanged.

`broker.deregister_agent` refuses to deregister an agent whose `agent_id` matches any `sessions.director_agent_id` and raises `Error: cannot deregister the root Director; use 'cafleet session delete' instead.` (exit 1). `session delete` remains the only supported teardown path for the root Director.

### `agents`

| Column | Type | Constraints | Notes |
|---|---|---|---|
| `agent_id` | `TEXT` | `PRIMARY KEY` | UUID v4. |
| `session_id` | `TEXT` | `NOT NULL`, `REFERENCES sessions(session_id) ON DELETE RESTRICT` | The owning session. SQLite enforces the FK once `PRAGMA foreign_keys=ON` is set. |
| `name` | `TEXT` | `NOT NULL` | |
| `description` | `TEXT` | `NOT NULL` | |
| `status` | `TEXT` | `NOT NULL` | `'active'` or `'deregistered'`. |
| `registered_at` | `TEXT` | `NOT NULL` | ISO-8601 timestamp. |
| `deregistered_at` | `TEXT` | nullable | ISO-8601 timestamp; populated on soft-delete. |
| `agent_card_json` | `TEXT` | `NOT NULL` | AgentCard-shaped blob (A2A-inspired, internal schema). |

Indexes:

| Name | Columns | Purpose |
|---|---|---|
| `idx_agents_session_status` | `(session_id, status)` | List active agents in a session; covers the `WHERE session_id = ? AND status = 'active'` predicate. |

Deregistration is a soft-delete: `status='deregistered'` plus `deregistered_at` is set in a single statement. There is no row delete and no background cleanup loop. Active query paths filter `status='active'` so dead rows are invisible to normal traffic.

#### Built-in Administrator agent

Each session owns exactly one built-in `Administrator` agent, marked by a flag inside `agent_card_json`:

```json
{
  "name": "Administrator",
  "description": "Built-in administrator agent for session <short-id>",
  "skills": [],
  "cafleet": {
    "kind": "builtin-administrator"
  }
}
```

The `cafleet.*` namespace inside `agent_card_json` is reserved for broker-owned flags. `broker.register_agent` always builds the card itself from `(name, description, skills)`, so callers cannot smuggle `cafleet.kind` through any current public path. A module-level constant `ADMINISTRATOR_KIND = "builtin-administrator"` in `broker.py` plus two helpers (`_administrator_agent_card(session_id)` builder, `_is_administrator_card(agent_card_json)` predicate) centralize Python-side construction and predicate checks for this flag; some hot read paths (e.g. `broker.list_session_agents`, `broker.broadcast_message`) also probe `agent_card_json.cafleet.kind` directly in SQL via `json_extract(...)` to avoid shipping the full blob into Python.

**Creation paths**:

- `broker.create_session(label, director_context)` inserts the Administrator row as the final operation of the 5-step root-Director bootstrap transaction (see "Root Director bootstrap" under the `sessions` table above); `registered_at` matches `sessions.created_at` exactly. The result dict exposes `administrator_agent_id` alongside the `director` sub-object for the caller.
- Alembic revision `0006_seed_administrator_agent.py` backfills one Administrator into each pre-existing session. The migration generates `agent_id = str(uuid.uuid4())` in Python (matching the broker's idiom — no SQL-side `gen_random_uuid()`), probes for an existing Administrator via `json_extract(agent_card_json, '$.cafleet.kind') = 'builtin-administrator'`, and is idempotent by construction (a second `upgrade` finds the existing row and skips the INSERT). Downgrade is provided for empty sessions only and is forward-only in practice — `tasks.context_id` uses `ON DELETE RESTRICT`, so downgrading a session that has tasks addressed to or from the Administrator raises `IntegrityError`. (`agent_placements.agent_id` uses `ON DELETE CASCADE`, but Administrators never receive a placement anyway.)

**Invariant**: Every session has exactly one active `Administrator` agent. Both `broker.list_session_agents` and `broker.get_agent` surface a derived `kind` field (`"builtin-administrator"` | `"user"`) so the WebUI can locate the Administrator without matching on the name. `list_session_agents` derives the discriminator in SQL via `json_extract(agent_card_json, '$.cafleet.kind')` and never fetches the full card blob; `get_agent` already loads the full ORM row and computes the discriminator via `_is_administrator_card`.

**Protection**: A single `AdministratorProtectedError` class in `broker.py` guards two write paths today:

| Operation | Guard |
|---|---|
| `broker.deregister_agent` | SELECTs the target's `agent_card_json` before the UPDATE and raises `AdministratorProtectedError("Administrator cannot be deregistered")` if the card matches. |
| `broker.register_agent(..., placement=...)` | The existing director-validation SELECT is extended to load `agent_card_json`; if the director row matches, raises `AdministratorProtectedError("Administrator cannot be a director")`. The Administrator never gets handed a tmux pane. |

A future `rename_agent` broker function MUST apply the same guard. `broker.broadcast_message` filters Administrators out of the recipient set (they are write-only identities), but the sender itself may be an Administrator.

### `tasks`

| Column | Type | Constraints | Notes |
|---|---|---|---|
| `task_id` | `TEXT` | `PRIMARY KEY` | UUID v4. |
| `context_id` | `TEXT` | `NOT NULL`, `REFERENCES agents(agent_id) ON DELETE RESTRICT` | The recipient agent for unicast/broadcast deliveries; the broadcaster for `broadcast_summary`; the preserved original `context_id` for ACK/cancel. Always a registered `agent_id`. |
| `from_agent_id` | `TEXT` | `NOT NULL` | Sender agent. **Not** a foreign key — historical tasks may outlive their sender. |
| `to_agent_id` | `TEXT` | `NOT NULL` | Recipient agent. Empty string `''` for `broadcast_summary` rows. |
| `type` | `TEXT` | `NOT NULL` | `'unicast'` or `'broadcast_summary'`. |
| `created_at` | `TEXT` | `NOT NULL` | ISO-8601 timestamp; first-write only, preserved across UPSERT. |
| `status_state` | `TEXT` | `NOT NULL` | TaskState enum value (e.g., `TASK_STATE_INPUT_REQUIRED`). |
| `status_timestamp` | `TEXT` | `NOT NULL` | ISO-8601 timestamp; updated on every state change. Used for `ORDER BY DESC`. |
| `origin_task_id` | `TEXT` | nullable | Broadcast grouping link. `NULL` on unicast deliveries. On broadcast delivery rows, holds the summary task's `task_id`, shared across every delivery row in the same broadcast. On the broadcast summary row itself, holds its own `task_id` (self-reference) so the delivery rows and the summary row all share a single grouping value. Historical rows from before the migration are `NULL`. |
| `task_json` | `TEXT` | `NOT NULL` | Task-shaped blob (A2A-inspired, internal schema). |

Indexes:

| Name | Columns | Purpose |
|---|---|---|
| `idx_tasks_context_status_ts` | `(context_id, status_timestamp DESC)` | Inbox listing: `WHERE context_id = ? ORDER BY status_timestamp DESC`. |
| `idx_tasks_from_agent_status_ts` | `(from_agent_id, status_timestamp DESC)` | WebUI sender outbox: `WHERE from_agent_id = ? ORDER BY status_timestamp DESC`. |

`status_state` and `status_timestamp` are promoted to columns so filtering and ordering execute on the database, not in Python after fetching every blob. The two task indexes serve the inbox listing query (`WHERE context_id = ? ORDER BY status_timestamp DESC`) and the WebUI sender outbox query (`WHERE from_agent_id = ? ORDER BY status_timestamp DESC`) directly from the index.

### `agent_placements`

| Column | Type | Constraints | Notes |
|---|---|---|---|
| `agent_id` | `TEXT` | `PRIMARY KEY`, `REFERENCES agents(agent_id) ON DELETE CASCADE` | The member agent. CASCADE ensures hard-delete of an agent (if any future path adds one) also removes the placement. |
| `director_agent_id` | `TEXT` | nullable, `REFERENCES agents(agent_id) ON DELETE RESTRICT` | The Director that spawned this member. RESTRICT prevents hard-deleting a Director with live placements. **NULL** for the session's root Director (it has no parent), set by `broker.create_session` at bootstrap time. Member placements always have a non-NULL value. |
| `tmux_session` | `TEXT` | `NOT NULL` | e.g. `'main'`, from `tmux display-message '#{session_name}'`. |
| `tmux_window_id` | `TEXT` | `NOT NULL` | e.g. `'@3'`, from `#{window_id}`. |
| `tmux_pane_id` | `TEXT` | nullable | e.g. `'%7'`. `NULL` = pending (row inserted at register time, pane not yet spawned). Set via `PATCH /api/v1/agents/{id}/placement` after `tmux split-window` succeeds. |
| `coding_agent` | `TEXT` | `NOT NULL`, `DEFAULT 'claude'` | Which coding agent binary is running in this pane. Always `"claude"` for new rows — codex support was deprecated in design 0000034 §15 and `cafleet member create` no longer accepts a `--coding-agent` flag. The column type, nullability, and `DEFAULT 'claude'` server default are unchanged: pre-existing rows with `coding_agent='codex'` from before round 6 are preserved verbatim for forensic visibility and for the §13 Future Work restoration plan. Server default ensures existing rows are backfilled on migration. |
| `created_at` | `TEXT` | `NOT NULL` | ISO-8601 timestamp, set server-side to match `agents.registered_at`. |

Indexes:

| Name | Columns | Purpose |
|---|---|---|
| `idx_placements_director` | `(director_agent_id)` | List all members spawned by a specific Director. |

Placement rows are hard-deleted (not soft-deleted) when the agent is deregistered through any path. They have no historical value and must not outlive the agent they describe. Deregistration is handled in `RegistryStore.deregister_agent`.

If a user kills a pane manually without going through `cafleet member delete`, the placement row stays until the next `member delete` resolves it. `send_exit(..., ignore_missing=True)` handles the "pane already gone" case gracefully.

### Foreign key enforcement

SQLite ignores foreign key declarations unless `PRAGMA foreign_keys=ON` is issued on every connection. The registry installs a SQLAlchemy engine `connect` event listener that runs the PRAGMA on every new DBAPI connection. A regression test verifies the PRAGMA is active on a fresh connection.

The two foreign keys (`agents.session_id → sessions.session_id`, `tasks.context_id → agents.agent_id`) both use `ON DELETE RESTRICT`. There is no path in v1 that physically deletes an agent — deregistration is a soft-status flip — so RESTRICT is the safest default. The added `sessions.director_agent_id → agents.agent_id` FK also uses `ON DELETE RESTRICT` for the same reason: it should never point at a row that can vanish. Session deletion uses a soft-delete (`deleted_at`) — it never physically removes rows, so the RESTRICTs are never triggered by the delete path.

## Operation mapping

Every storage operation is implemented as a single SQL statement (or, where atomicity matters, a single transaction). The following tables enumerate the public store methods and the SQL they execute.

### `RegistryStore` (agents + sessions)

| Method | SQL operation |
|---|---|
| `create_agent` | `INSERT INTO agents (...)` (single statement; `session_id` FK enforces the session exists). |
| `get_agent` | `SELECT … FROM agents WHERE agent_id = ?`. |
| `list_active_agents(session_id)` | `SELECT agent_id, name, description, registered_at, agent_card_json FROM agents WHERE session_id = ? AND status = 'active'` (uses `idx_agents_session_status`). |
| `list_active_agents(None)` | `SELECT … FROM agents WHERE status = 'active'` (rare; only used by tests). |
| `deregister_agent` | `UPDATE agents SET status='deregistered', deregistered_at=? WHERE agent_id=? AND status='active'` (single statement; returns affected row count). |
| `verify_agent_session` | `SELECT 1 FROM agents WHERE agent_id = ? AND session_id = ?`. |
| `get_agent_name` | `SELECT name FROM agents WHERE agent_id = ?` (returns `''` if absent). |
| `list_deregistered_agents_with_tasks(session_id)` | `SELECT a.agent_id, a.name, a.description, a.registered_at FROM agents a WHERE a.session_id = ? AND a.status = 'deregistered' AND EXISTS (SELECT 1 FROM tasks t WHERE t.context_id = a.agent_id LIMIT 1)`. |
| `list_sessions` | `SELECT s.session_id, s.label, s.created_at, COUNT(a.agent_id) FROM sessions s LEFT JOIN agents a ON ... GROUP BY s.session_id`. |
| `get_session` | `SELECT * FROM sessions WHERE session_id = ?`. |
| `create_agent_with_placement(…, placement)` | Single transaction: `INSERT INTO agents (…)` + optional `INSERT INTO agent_placements (…)`. Superset of `create_agent` (which delegates with `placement=None`). |
| `get_placement(agent_id)` | `SELECT * FROM agent_placements WHERE agent_id = ?`. |
| `update_placement_pane_id(agent_id, pane_id)` | `UPDATE agent_placements SET tmux_pane_id = ? WHERE agent_id = ?`. |
| `list_placements_for_director(session_id, director_agent_id)` | `SELECT a.*, p.* FROM agents a JOIN agent_placements p ON a.agent_id = p.agent_id WHERE a.session_id = ? AND p.director_agent_id = ? AND a.status = 'active'`. |

### `TaskStore`

| Method | SQL operation |
|---|---|
| `save` | `INSERT … ON CONFLICT(task_id) DO UPDATE SET status_state=excluded.status_state, status_timestamp=excluded.status_timestamp, task_json=excluded.task_json` (preserves the original `created_at`). |
| `get` | `SELECT task_json FROM tasks WHERE task_id = ?`. |
| `delete` | `DELETE FROM tasks WHERE task_id = ?` (indexes are auto-cleaned by SQLite). |
| `list(context_id)` | `SELECT task_json FROM tasks WHERE context_id = ? ORDER BY status_timestamp DESC`. |
| `list_by_sender(agent_id)` | `SELECT task_json FROM tasks WHERE from_agent_id = ? ORDER BY status_timestamp DESC` — used by the WebUI sender outbox. |
| `get_endpoints(task_id)` | `SELECT from_agent_id, to_agent_id FROM tasks WHERE task_id = ?` — used by `_handle_get_task` for authorization. |
| `get_created_at(task_id)` | `SELECT created_at FROM tasks WHERE task_id = ?`. |

### Session ownership

Stores receive an `async_sessionmaker[AsyncSession]` at construction time, **not** a per-call session. Each store method opens its own session via `async with self._sessionmaker() as session:` and any multi-statement operation wraps its body in `async with session.begin():`. Route handlers and the `BrokerExecutor` only ever see the store; they never construct or close a session.

## Session Lifecycle

Sessions are created via `cafleet session create` (direct SQLite write, no HTTP; must be run inside a tmux session). `create_session` performs the 5-step transactional bootstrap described above — the session row, the root Director agent + placement, the `director_agent_id` back-reference, and the built-in Administrator are written atomically.

When member agents in a session deregister, the session remains valid — new members can still be spawned.

Deleting a session (via `cafleet session delete <id>`) is a **soft-delete**. `broker.delete_session` runs a single transaction that stamps `sessions.deleted_at = now`, deregisters every `status='active'` agent in the session (root Director included), and physically deletes every associated `agent_placements` row. `tasks` rows are preserved — the message history remains queryable. Re-running on an already-soft-deleted session is a no-op that reports `Deregistered 0 agents.` because the initial `UPDATE sessions SET deleted_at = now WHERE session_id = X AND deleted_at IS NULL` short-circuits the cascade. There is no `--force` flag, no un-delete path, and no cascade into `tasks`.

Soft-deleted sessions are hidden from `cafleet session list` (`broker.list_sessions` filters `WHERE deleted_at IS NULL`) and new registrations are rejected by `broker.register_agent` with `Error: session X is deleted`. `broker.get_session` always returns the row regardless of `deleted_at` and exposes the field so callers can distinguish "not found" vs. "soft-deleted".

`broker.deregister_agent` refuses to deregister the root Director (detected by a match on `sessions.director_agent_id`). Use `session delete` for teardown.

## Task Visibility Rules

| Caller | Can Access |
|---|---|
| Recipient (same-session agent matching `to_agent_id`) | `ListTasks` by contextId (= their agent_id), `GetTask`, `SendMessage` (ACK) |
| Sender (same-session agent matching `from_agent_id`) | `GetTask` by known taskId, `CancelTask` |

`ListTasks` enforces that `contextId` must equal the caller's `agent_id`. If a different `contextId` is provided, the Broker returns an error. This prevents inbox snooping — even within the same session. `GetTask` verifies that the task's `from_agent_id` or `to_agent_id` belongs to the caller's session; cross-session lookups return "not found".

## Broadcast Grouping

Broadcast fan-out in `BrokerExecutor._handle_broadcast` produces N+1 rows per `SendMessage(destination="*")` — one delivery task per active recipient plus one `broadcast_summary` task — and the admin WebUI timeline needs to present all of them as a single entry. The `tasks.origin_task_id` column is the grouping link:

| Row kind | `origin_task_id` value |
|---|---|
| Unicast delivery (today's `_handle_unicast`) | `NULL` |
| Broadcast delivery row (one per recipient) | The summary task's `task_id` (shared across all N delivery rows in the same broadcast) |
| Broadcast summary row | Its own `task_id` (self-reference) |
| Historical row from before the `0002_add_origin_task_id` migration | `NULL` (no backfill) |

The column is populated by pre-allocating the summary task's UUID **before** the delivery loop in `_handle_broadcast`, then threading that UUID into every delivery task's metadata as `originTaskId`. `TaskStore.save` reads `metadata.get("originTaskId")` and writes it into the column on both `INSERT` and `ON CONFLICT DO UPDATE` so idempotent re-saves preserve the value. `_handle_unicast` is NOT touched — the absence of `originTaskId` in its metadata writes the column as `NULL`.

The grouping predicate on the wire is `origin_task_id IS NOT NULL`, which cleanly partitions the timeline into "standalone unicast entry" vs "part of a broadcast group". The summary task's `metadata["recipientIds"]` is extended from the existing `recipientCount` to carry the full recipient list so readers that need sender-side fan-out introspection (not the timeline itself) can reconstruct the recipient set.

### Known design debt — ACK timestamp inference

The timeline UI renders a per-recipient ACK time in each broadcast's hover tooltip. That time is read from the `status_timestamp` of the matching delivery row whose `status_state == 'completed'`. This works today because **delivery tasks make exactly one state transition over their lifetime**: `input_required → completed` via `BrokerExecutor._handle_ack`, which overwrites `status_timestamp` with the ACK moment. Consequently, for any completed delivery row, `status_timestamp` IS the ACK timestamp.

**No dedicated `acknowledged_at` column is added.** If any future change introduces a second state transition on a delivery task — retry, resurrect, a metadata-only re-save that moves `status_timestamp`, or any other path that rewrites `status_timestamp` after ACK — this invariant breaks and the reaction tooltip silently starts showing wrong times. At that point a dedicated `acknowledged_at` TEXT column MUST be added to `tasks`, populated in `_handle_ack`, and the WebUI tooltip code MUST be switched to read it instead of `status_timestamp`. This is accepted residual risk for the first cut of the Discord-style timeline and is explicitly flagged here so the next contributor who breaks the invariant knows exactly which column to add.

## Deregistered Agents

Deregistration is a soft-delete only. There is no background cleanup loop and no physical removal of agent or task rows. Deregistered agents continue to exist as rows with `status='deregistered'` indefinitely; their inbox tasks remain readable by the WebUI (the only consumer that surfaces deregistered agents). Active query paths filter `status='active'` so dead rows are invisible to normal traffic.

If physical cleanup becomes necessary in the future, it can be reintroduced as an opt-in admin command (e.g., `cafleet db purge --older-than 30d`) without disturbing the runtime. The previous `DEREGISTERED_TASK_TTL_DAYS` and `CLEANUP_INTERVAL_SECONDS` settings have been removed.
