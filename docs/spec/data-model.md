# SQLite Data Model Specification

All core data structures — `Task`, `Message`, `Part`, `Artifact`, `AgentCard`, `TaskStatus`, `TaskState` — use types defined by the A2A specification via `a2a-sdk` Pydantic models. No Broker-specific data models are created. SQLite stores `a2a-sdk` `Task` and `AgentCard` objects verbatim as JSON `TEXT` blobs and deserializes them back to Pydantic models on read. Broker-specific information (routing metadata, etc.) lives in indexed columns alongside the JSON blob.

The model is a **relational + document hybrid**: indexed fields are columns, while A2A protocol payloads are stored as opaque JSON `TEXT`. The columns are queried; the JSON blobs are not.

Schema management is handled by Alembic (`registry/src/hikyaku_registry/alembic/`); the runtime engine is SQLAlchemy 2.x with the `aiosqlite` async driver. Operators apply migrations once via `hikyaku-registry db init` before starting the server.

## SQL Schema

### `api_keys`

| Column | Type | Constraints | Notes |
|---|---|---|---|
| `api_key_hash` | `TEXT` | `PRIMARY KEY` | `SHA-256(raw_api_key)`. Doubles as `tenant_id`. |
| `owner_sub` | `TEXT` | `NOT NULL` | Auth0 `sub` claim of the user who created the key. |
| `key_prefix` | `TEXT` | `NOT NULL` | First 8 characters of the raw API key, for display in the WebUI. |
| `status` | `TEXT` | `NOT NULL` | `'active'` or `'revoked'`. |
| `created_at` | `TEXT` | `NOT NULL` | ISO-8601 timestamp. |

Indexes:

| Name | Columns | Purpose |
|---|---|---|
| `idx_api_keys_owner` | `(owner_sub)` | List API keys for a logged-in Auth0 user. |

The `api_keys` row is the source of truth for tenant existence and validity. Every authenticated request (agent and WebUI) checks `status='active'`. Revoking a key flips it to `'revoked'` and bulk-deregisters every agent in the tenant in the same transaction.

### `agents`

| Column | Type | Constraints | Notes |
|---|---|---|---|
| `agent_id` | `TEXT` | `PRIMARY KEY` | UUID v4. |
| `tenant_id` | `TEXT` | `NOT NULL`, `REFERENCES api_keys(api_key_hash) ON DELETE RESTRICT` | The owning tenant. SQLite enforces the FK once `PRAGMA foreign_keys=ON` is set. |
| `name` | `TEXT` | `NOT NULL` | |
| `description` | `TEXT` | `NOT NULL` | |
| `status` | `TEXT` | `NOT NULL` | `'active'` or `'deregistered'`. |
| `registered_at` | `TEXT` | `NOT NULL` | ISO-8601 timestamp. |
| `deregistered_at` | `TEXT` | nullable | ISO-8601 timestamp; populated on soft-delete. |
| `agent_card_json` | `TEXT` | `NOT NULL` | Full A2A `AgentCard` blob, stored verbatim. |

Indexes:

| Name | Columns | Purpose |
|---|---|---|
| `idx_agents_tenant_status` | `(tenant_id, status)` | List active agents in a tenant; covers the `WHERE tenant_id = ? AND status = 'active'` predicate. |

Deregistration is a soft-delete: `status='deregistered'` plus `deregistered_at` is set in a single statement. There is no row delete and no background cleanup loop. Active query paths filter `status='active'` so dead rows are invisible to normal traffic.

### `tasks`

| Column | Type | Constraints | Notes |
|---|---|---|---|
| `task_id` | `TEXT` | `PRIMARY KEY` | UUID v4. |
| `context_id` | `TEXT` | `NOT NULL`, `REFERENCES agents(agent_id) ON DELETE RESTRICT` | The recipient agent for unicast/broadcast deliveries; the broadcaster for `broadcast_summary`; the preserved original `context_id` for ACK/cancel. Always a registered `agent_id`. |
| `from_agent_id` | `TEXT` | `NOT NULL` | Sender agent. **Not** a foreign key — historical tasks may outlive their sender. |
| `to_agent_id` | `TEXT` | `NOT NULL` | Recipient agent. Empty string `''` for `broadcast_summary` rows. |
| `type` | `TEXT` | `NOT NULL` | `'unicast'` or `'broadcast_summary'`. |
| `created_at` | `TEXT` | `NOT NULL` | ISO-8601 timestamp; first-write only, preserved across UPSERT. |
| `status_state` | `TEXT` | `NOT NULL` | A2A `TaskState` enum value (e.g., `TASK_STATE_INPUT_REQUIRED`). |
| `status_timestamp` | `TEXT` | `NOT NULL` | ISO-8601 timestamp; updated on every state change. Used for `ORDER BY DESC`. |
| `task_json` | `TEXT` | `NOT NULL` | Full A2A `Task` blob, stored verbatim. |

Indexes:

| Name | Columns | Purpose |
|---|---|---|
| `idx_tasks_context_status_ts` | `(context_id, status_timestamp DESC)` | Inbox listing: `WHERE context_id = ? ORDER BY status_timestamp DESC`. |
| `idx_tasks_from_agent_status_ts` | `(from_agent_id, status_timestamp DESC)` | WebUI sender outbox: `WHERE from_agent_id = ? ORDER BY status_timestamp DESC`. |

`status_state` and `status_timestamp` are promoted to columns so filtering and ordering execute on the database, not in Python after fetching every blob. The two task indexes serve the inbox listing query (`WHERE context_id = ? ORDER BY status_timestamp DESC`) and the WebUI sender outbox query (`WHERE from_agent_id = ? ORDER BY status_timestamp DESC`) directly from the index.

### Foreign key enforcement

SQLite ignores foreign key declarations unless `PRAGMA foreign_keys=ON` is issued on every connection. The registry installs a SQLAlchemy engine `connect` event listener that runs the PRAGMA on every new DBAPI connection. A regression test verifies the PRAGMA is active on a fresh connection.

The two foreign keys (`agents.tenant_id → api_keys.api_key_hash`, `tasks.context_id → agents.agent_id`) both use `ON DELETE RESTRICT`. There is no path in v1 that physically deletes an agent or an API key — revoke is a soft-status flip — so RESTRICT is the safest default.

## Operation mapping

Every storage operation is implemented as a single SQL statement (or, where atomicity matters, a single transaction). The following tables enumerate the public store methods and the SQL they execute.

### `RegistryStore` (agents + tenants)

| Method | SQL operation |
|---|---|
| `create_agent` | `INSERT INTO agents (...)` (single statement; `tenant_id` FK enforces the API key exists). |
| `get_agent` | `SELECT … FROM agents WHERE agent_id = ?`. |
| `list_active_agents(tenant_id)` | `SELECT agent_id, name, description, registered_at, agent_card_json FROM agents WHERE tenant_id = ? AND status = 'active'` (uses `idx_agents_tenant_status`). |
| `list_active_agents(None)` | `SELECT … FROM agents WHERE status = 'active'` (rare; only used by tests). |
| `deregister_agent` | `UPDATE agents SET status='deregistered', deregistered_at=? WHERE agent_id=? AND status='active'` (single statement; returns affected row count). |
| `verify_agent_tenant` | `SELECT 1 FROM agents WHERE agent_id = ? AND tenant_id = ?`. |
| `is_api_key_active` | `SELECT 1 FROM api_keys WHERE api_key_hash = ? AND status = 'active'`. |
| `is_key_owner` | `SELECT 1 FROM api_keys WHERE api_key_hash = ? AND owner_sub = ?`. |
| `get_agent_name` | `SELECT name FROM agents WHERE agent_id = ?` (returns `''` if absent). |
| `list_deregistered_agents_with_tasks(tenant_id)` | `SELECT a.agent_id, a.name, a.description, a.registered_at FROM agents a WHERE a.tenant_id = ? AND a.status = 'deregistered' AND EXISTS (SELECT 1 FROM tasks t WHERE t.context_id = a.agent_id LIMIT 1)`. |

### `RegistryStore` (API keys)

| Method | SQL operation |
|---|---|
| `create_api_key` | `INSERT INTO api_keys (...)`. |
| `list_api_keys` | Single query: `SELECT k.api_key_hash, k.key_prefix, k.created_at, k.status, COUNT(a.agent_id) AS agent_count FROM api_keys k LEFT JOIN agents a ON a.tenant_id = k.api_key_hash AND a.status = 'active' WHERE k.owner_sub = ? GROUP BY k.api_key_hash`. Replaces the previous N+1 (`SMEMBERS` + per-key `HGETALL` + `SCARD`). |
| `revoke_api_key` | Single transaction: `UPDATE api_keys SET status='revoked' WHERE api_key_hash=? AND owner_sub=?` then `UPDATE agents SET status='deregistered', deregistered_at=? WHERE tenant_id=? AND status='active'`. Both statements run inside `async with session.begin():` so the API key flip and the per-tenant cascade are atomic. |
| `get_api_key_status` | `SELECT status FROM api_keys WHERE api_key_hash = ?`. |

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

## Tenant Lifecycle

Tenants are created when a user creates an API key via the WebUI. The `api_keys` row is the source of truth for tenant existence. Agents join the tenant by registering with the API key.

When all agents in a tenant deregister, the tenant remains valid — new agents can still register using the API key as long as its `status` is `'active'`.

Revoking a key (via `DELETE /ui/api/keys/{tenant_id}`) flips its `status` to `'revoked'` and bulk-deregisters every active agent in the tenant in a single transaction. A revoked key cannot be used for agent registration or authentication.

## Task Visibility Rules

| Caller | Can Access |
|---|---|
| Recipient (same-tenant agent matching `to_agent_id`) | `ListTasks` by contextId (= their agent_id), `GetTask`, `SendMessage` (ACK) |
| Sender (same-tenant agent matching `from_agent_id`) | `GetTask` by known taskId, `CancelTask` |

`ListTasks` enforces that `contextId` must equal the caller's `agent_id`. If a different `contextId` is provided, the Broker returns an error. This prevents inbox snooping — even within the same tenant. `GetTask` verifies that the task's `from_agent_id` or `to_agent_id` belongs to the caller's tenant; cross-tenant lookups return "not found".

## Deregistered Agents

Deregistration is a soft-delete only. There is no background cleanup loop and no physical removal of agent or task rows. Deregistered agents continue to exist as rows with `status='deregistered'` indefinitely; their inbox tasks remain readable by the WebUI (the only consumer that surfaces deregistered agents). Active query paths filter `status='active'` so dead rows are invisible to normal A2A traffic.

If physical cleanup becomes necessary in the future, it can be reintroduced as an opt-in admin command (e.g., `hikyaku-registry db purge --older-than 30d`) without disturbing the runtime. The previous `DEREGISTERED_TASK_TTL_DAYS` and `CLEANUP_INTERVAL_SECONDS` settings have been removed.
