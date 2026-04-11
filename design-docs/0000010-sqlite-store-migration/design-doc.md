# SQLite Store Migration

**Status**: Complete
**Progress**: 70/70 tasks complete
**Last Updated**: 2026-04-11

## Overview

Replace the Hikyaku registry's Redis-backed storage with a SQLite-backed store using SQLAlchemy + Alembic, and replace the Redis Pub/Sub inbox channel with an in-process `asyncio.Queue` fan-out. The data model is a relational + document hybrid: indexed fields are columns, while A2A `AgentCard` and `Task` payloads are stored verbatim as JSON `TEXT` blobs. A new `hikyaku-registry db init` CLI is added for schema management.

## Success Criteria

- [ ] `redis[hiredis]` and `fakeredis` are removed from `registry/pyproject.toml`; `sqlalchemy`, `alembic`, `aiosqlite` are added
- [ ] No `import redis` or `redis.asyncio` references remain anywhere under `registry/`
- [ ] Every Redis key/operation in `registry_store.py`, `task_store.py`, `pubsub.py`, `cleanup.py`, and the `_redis.*` direct calls in `main.py` / `auth.py` / `webui_api.py` has a documented SQL or in-process replacement
- [ ] `RegistryStore`, `TaskStore`, and `PubSubManager` expose the same public surface to their callers (signature-compatible) so the executor and route handlers do not need branch logic
- [ ] No call site under `registry/src/hikyaku_registry/` accesses `store._redis` or any private DB attribute — all storage access goes through store methods
- [ ] `hikyaku-registry db init` exists as a console script and is idempotent across the six DB states (file-missing, empty-schema, at-head, behind, ahead, legacy-no-version-table)
- [ ] All registry tests pass against an in-memory aiosqlite engine via the rewritten `conftest.py` fixtures
- [ ] A session-scoped Alembic smoke test runs `alembic upgrade head` against a real tempfile DB and exits 0
- [ ] FOREIGN KEY constraints are active on every connection (verified by a regression test)
- [ ] `ARCHITECTURE.md`, `docs/`, both `CLAUDE.md` files, and the `/hikyaku` skill are updated to reference SQLite/SQLAlchemy/Alembic instead of Redis BEFORE any code change is made

---

## Background

The current registry persists everything in Redis using hand-rolled keyspaces. This works but the application is doing work that a relational store would do for free, and it requires running a Redis daemon alongside the FastAPI process.

**Concrete pain points in the current code:**

| # | Pain point | Where |
|---|---|---|
| 1 | Manual index synchronization across `agents:active`, `tenant:{hash}:agents`, `tasks:sender:{id}`, `tasks:ctx:{id}` requires multi-key pipelines that can desync if a step is added and a corresponding cleanup is forgotten | `registry_store.py:create_agent`, `deregister_agent`, `task_store.py:save`, `delete` |
| 2 | `cleanup_expired_agents` is a procedural `SCAN agent:* → HGETALL → check status → ZRANGE → per-task DEL` loop that would be a single `DELETE … WHERE status='deregistered' AND deregistered_at < ?` in SQL | `cleanup.py` |
| 3 | `revoke_api_key` performs a non-atomic per-agent cascade by calling `deregister_agent` in a Python loop after flipping the API key status | `registry_store.py:revoke_api_key` |
| 4 | `list_api_keys` is N+1: one `SMEMBERS` followed by `HGETALL + SCARD` per key | `registry_store.py:list_api_keys` |
| 5 | The Redis abstraction has leaked: `main.py`, `auth.py`, and `webui_api.py` all reach into `store._redis` directly to call `hget` / `sismember` / `scan`, making the store non-substitutable and obscuring which code paths actually need new SQL methods | `main.py:186-187,322,331`, `auth.py:43,51,77`, `webui_api.py:92-94,102,134,153,169,288,322` |
| 6 | Redis is a separate daemon to operate, monitor, and back up — for an internal coding-agent broker this is more operational overhead than the workload justifies |

The Pub/Sub inbox channel (`inbox:{agent_id}`) does not have a SQLite equivalent (SQLite has no LISTEN/NOTIFY), so the migration also replaces it with an in-process `asyncio.Queue` fan-out. This is acceptable because the registry already runs as a single uvicorn worker in practice; the design doc makes that constraint explicit.

---

## Specification

### Goals

- Eliminate the operational dependency on a Redis daemon
- Replace hand-rolled secondary indexes with SQL `INDEX` declarations
- Replace the procedural cleanup scan with declarative SQL (or remove it entirely — see Behavioral Changes)
- Make `revoke_api_key` atomic via a single transaction
- Eliminate the N+1 in `list_api_keys` via a `LEFT JOIN` + `GROUP BY` query
- Stop leaking the Redis abstraction into route handlers; route handlers must only call store methods
- Provide schema-management tooling (`hikyaku-registry db init`) so a fresh deployment is one command away

### Non-goals

| Non-goal | Reason |
|---|---|
| Multi-process / multi-worker registry server | The new in-process Pub/Sub fan-out cannot route across processes. Out of scope for v1; revisit with PostgreSQL `LISTEN/NOTIFY` later if needed. |
| Redis-to-SQLite data migration tool | Hard cutover. Existing agents must re-register. The project is pre-1.0 and there is no production data preservation requirement. |
| Multi-backend DB support (PostgreSQL, MySQL) | SQLite is the only target. SQLAlchemy is used for ORM ergonomics, not for portability. The schema uses SQLite-friendly types (`TEXT` everywhere). |
| `db revision` / `db downgrade` / `db current` CLI commands | Only `db init` is implemented in v1. The `db` click subgroup is scaffolded so future commands can be added without restructuring. |
| WAL journaling mode | Not enabled for v1. Single-process server + low write throughput + rare CLI use mean WAL is premature optimization. Revisit if write contention shows up. |
| Bounded queues / drop-oldest semantics for the in-process fan-out | Unbounded `asyncio.Queue` for v1. Matches existing Redis Pub/Sub risk profile. |
| Auto-stamp legacy schemas | If a SQLite file has tables but no `alembic_version` row, `db init` errors out and asks the operator to run `alembic stamp head` manually. |

### Data Model

The model is a **relational + document hybrid**:

- **Indexed fields are columns** (`agent_id`, `tenant_id`, `from_agent_id`, `to_agent_id`, `context_id`, `created_at`, `status`, `status_state`, `status_timestamp`)
- **A2A protocol payloads are JSON blobs** (`agent_card_json`, `task_json`) stored as `TEXT` and never queried by content

Foreign keys are **enabled** via a SQLAlchemy engine event that issues `PRAGMA foreign_keys=ON` on every new connection. A regression test verifies the PRAGMA is active on a fresh connection.

#### Schema

```
api_keys
  api_key_hash       TEXT PRIMARY KEY        -- = tenant_id (sha256 of raw API key)
  owner_sub          TEXT NOT NULL           -- Auth0 sub claim
  key_prefix         TEXT NOT NULL           -- first 8 chars of raw key, for display
  status             TEXT NOT NULL           -- 'active' | 'revoked'
  created_at         TEXT NOT NULL           -- ISO-8601
  INDEX idx_api_keys_owner  (owner_sub)      -- list_api_keys by owner

agents
  agent_id           TEXT PRIMARY KEY        -- UUID v4
  tenant_id          TEXT NOT NULL
                       REFERENCES api_keys(api_key_hash)
                       ON DELETE RESTRICT
  name               TEXT NOT NULL
  description        TEXT NOT NULL
  status             TEXT NOT NULL           -- 'active' | 'deregistered'
  registered_at      TEXT NOT NULL           -- ISO-8601
  deregistered_at    TEXT                    -- nullable
  agent_card_json    TEXT NOT NULL           -- full A2A AgentCard blob
  INDEX idx_agents_tenant_status  (tenant_id, status)

tasks
  task_id            TEXT PRIMARY KEY        -- UUID v4
  context_id         TEXT NOT NULL
                       REFERENCES agents(agent_id)
                       ON DELETE RESTRICT
  from_agent_id      TEXT NOT NULL           -- not FK: can outlive an agent's row
  to_agent_id        TEXT NOT NULL           -- '' for broadcast_summary
  type               TEXT NOT NULL           -- 'unicast' | 'broadcast_summary'
  created_at         TEXT NOT NULL           -- ISO-8601, first-write timestamp
  status_state       TEXT NOT NULL           -- TaskState enum value
  status_timestamp   TEXT NOT NULL           -- ISO-8601, used for ORDER BY DESC
  task_json          TEXT NOT NULL           -- full A2A Task blob
  INDEX idx_tasks_context_status_ts      (context_id, status_timestamp DESC)
  INDEX idx_tasks_from_agent_status_ts   (from_agent_id, status_timestamp DESC)
```

> **Index declaration in SQLAlchemy.** The pseudo-DDL above uses inline `INDEX` syntax for readability; SQLAlchemy declarative models declare indexes via `__table_args__ = (Index('idx_name', column, ...), ...)` instead. The pseudo-DDL is illustrative — see `models.py` for the canonical Python declarations.

**Notes:**

- `status_state` and `status_timestamp` are promoted to columns. `_handle_list_tasks` filters by `status` and the WebUI sorts the sender's outbox by status timestamp; both must execute on the database, not in Python after fetching every blob.
- `agents.tenant_id` is a real foreign key — SQLite enforces it once `PRAGMA foreign_keys=ON` is set. `ON DELETE RESTRICT` because there is no scenario today where deleting a tenant should cascade-delete agents (revoke is a soft `status='revoked'` flip, not a row delete).
- `tasks.context_id` is a real foreign key. `ON DELETE RESTRICT` for the same reason: no path deletes agent rows in v1 (see Behavioral Changes — physical cleanup is removed). All five task-creation paths in `BrokerExecutor` (`_handle_unicast`, `_handle_broadcast` per-recipient delivery, `_handle_broadcast` summary, `_handle_ack`, `cancel`) set `tasks.context_id` to a registered `agent_id` — the recipient for unicast/broadcast deliveries, the broadcaster for the summary, and the preserved original `context_id` for ACK/cancel. The broadcaster of a `broadcast_summary` must still be in the `agents` table; since this is the sender of the enclosing request, the invariant is trivially satisfied. The FK therefore holds for every current and planned task type.
- `tasks.from_agent_id` is intentionally **not** a foreign key. A historical task may reference an agent that has since been deregistered, and there is no row delete to worry about, but keeping it FK-free leaves room for future workflows that don't require sender survival.
- Index `idx_tasks_context_status_ts` replaces the Redis sorted set `tasks:ctx:{ctx}` and serves the inbox listing query `SELECT … WHERE context_id = ? ORDER BY status_timestamp DESC`.
- Index `idx_tasks_from_agent_status_ts` is the symmetric counterpart for the WebUI sender outbox: `SELECT … WHERE from_agent_id = ? ORDER BY status_timestamp DESC` is fully index-served, eliminating the in-Python sort that exists today in `webui_api.get_sent`.

#### SQLAlchemy declarative models

Models live at `registry/src/hikyaku_registry/db/models.py`. The `Base` is a `DeclarativeBase` subclass exported from the same module so Alembic's `env.py` can import its `metadata` for autogenerate. Models use `Mapped[str]` / `mapped_column()` style.

#### Engine + session factory

`registry/src/hikyaku_registry/db/engine.py` exposes:

- `get_engine() -> AsyncEngine` — singleton constructed from `settings.database_url`
- `get_sessionmaker() -> async_sessionmaker[AsyncSession]` — singleton bound to the engine
- An `event.listens_for(Engine, "connect")` listener that issues `cursor.execute("PRAGMA foreign_keys=ON")` on every raw DBAPI connection (this is the standard SQLAlchemy + SQLite FK enablement pattern; without it, the FK constraints declared in `models.py` are silently ignored)
- `dispose_engine()` for lifespan teardown

**Default URL — `~` expansion is owned by `config.py`, not `engine.py`.** The default is constructed at config load time, not as a class attribute literal, so `os.path.expanduser` runs once with the actual home directory. Engine and Alembic both consume the *already-expanded* `settings.database_url`. As a result, the runtime URL is in the SQLAlchemy 4-slash absolute form (`sqlite+aiosqlite:////home/<user>/.local/share/hikyaku/registry.db`) — the first `//` introduces the scheme and the second `//` is the absolute path's leading `/`. No downstream code re-expands `~`.

```python
# registry/src/hikyaku_registry/db/engine.py
from sqlalchemy import event
from sqlalchemy.engine import Engine
from sqlalchemy.ext.asyncio import (
    AsyncEngine, async_sessionmaker, create_async_engine,
)
from hikyaku_registry.config import settings

@event.listens_for(Engine, "connect")
def _enable_sqlite_foreign_keys(dbapi_conn, _):
    cursor = dbapi_conn.cursor()
    cursor.execute("PRAGMA foreign_keys=ON")
    cursor.close()

_engine: AsyncEngine | None = None
_sessionmaker: async_sessionmaker[AsyncSession] | None = None

def get_engine() -> AsyncEngine:
    global _engine
    if _engine is None:
        # settings.database_url is already-expanded, e.g.
        # "sqlite+aiosqlite:////home/alice/.local/share/hikyaku/registry.db"
        _engine = create_async_engine(settings.database_url, future=True)
    return _engine

def get_sessionmaker() -> async_sessionmaker[AsyncSession]:
    global _sessionmaker
    if _sessionmaker is None:
        _sessionmaker = async_sessionmaker(get_engine(), expire_on_commit=False)
    return _sessionmaker
```

#### Store ownership of sessions

Stores receive an `async_sessionmaker[AsyncSession]` at construction time — **not** a per-call `AsyncSession`. Each store method internally opens a fresh session via `async with self._sessionmaker() as session:`, and any method that needs a transaction wraps its body in `async with session.begin():`. This keeps the call sites (route handlers, executor) free of any session lifecycle concern: they hold a long-lived store reference, and the store owns its own sessions.

Pseudocode for the canonical multi-statement-transaction pattern (`revoke_api_key`):

```python
class RegistryStore:
    def __init__(self, sessionmaker: async_sessionmaker[AsyncSession]) -> None:
        self._sessionmaker = sessionmaker

    async def revoke_api_key(self, tenant_id: str, owner_sub: str) -> bool:
        async with self._sessionmaker() as session:
            async with session.begin():               # single atomic txn
                result = await session.execute(
                    update(ApiKey)
                    .where(ApiKey.api_key_hash == tenant_id,
                           ApiKey.owner_sub == owner_sub,
                           ApiKey.status == "active")
                    .values(status="revoked")
                )
                if result.rowcount == 0:
                    return False
                await session.execute(
                    update(Agent)
                    .where(Agent.tenant_id == tenant_id,
                           Agent.status == "active")
                    .values(status="deregistered",
                            deregistered_at=datetime.now(UTC).isoformat())
                )
                return True
```

The same pattern applies to any future multi-statement operation. Single-statement reads (e.g. `get_agent`, `is_api_key_active`) use `async with self._sessionmaker() as session:` without an explicit `session.begin()` block.

### Operation Mapping

Every storage operation that exists today must have an explicit replacement. This table is the source of truth — if an entry is missing, the migration is incomplete.

#### `RegistryStore` (agents + tenants)

| Current Redis op | Where | New SQL op |
|---|---|---|
| `HSET agent:{id} mapping`, `SADD agents:active`, `SADD tenant:{hash}:agents` | `create_agent` | `INSERT INTO agents (...)` (single statement; `tenant_id` FK enforces the API key exists) |
| `HGETALL agent:{id}` | `get_agent` | `SELECT … FROM agents WHERE agent_id = ?` |
| `SMEMBERS tenant:{hash}:agents` + per-id `HGETALL` | `list_active_agents(tenant_id)` | `SELECT agent_id, name, description, registered_at, agent_card_json FROM agents WHERE tenant_id = ? AND status = 'active'` (uses `idx_agents_tenant_status`) |
| `SMEMBERS agents:active` + per-id `HGETALL` | `list_active_agents(None)` (legacy) | `SELECT … FROM agents WHERE status = 'active'` (rare path; only used by tests) |
| `EXISTS agent:{id}` + `HGET api_key_hash` + `HSET status,deregistered_at` + `SREM agents:active` + `SREM tenant:{hash}:agents` | `deregister_agent` | `UPDATE agents SET status='deregistered', deregistered_at=? WHERE agent_id=? AND status='active'` (single statement, returns affected row count) |
| `HGET agent:{id} api_key_hash` | `verify_agent_tenant` | `SELECT 1 FROM agents WHERE agent_id = ? AND tenant_id = ?` |

#### `RegistryStore` (API keys)

| Current Redis op | Where | New SQL op |
|---|---|---|
| `HSET apikey:{hash} mapping`, `SADD account:{sub}:keys` | `create_api_key` | `INSERT INTO api_keys (...)` (single statement) |
| `SMEMBERS account:{sub}:keys` + per-hash `HGETALL` + per-hash `SCARD tenant:{hash}:agents` (N+1) | `list_api_keys` | Single query: `SELECT k.api_key_hash, k.key_prefix, k.created_at, k.status, COUNT(a.agent_id) AS agent_count FROM api_keys k LEFT JOIN agents a ON a.tenant_id = k.api_key_hash AND a.status = 'active' WHERE k.owner_sub = ? GROUP BY k.api_key_hash` |
| `SISMEMBER account:{sub}:keys`, `HSET apikey:{hash} status='revoked'`, then per-agent `deregister_agent` loop | `revoke_api_key` | Single transaction: `UPDATE api_keys SET status='revoked' WHERE api_key_hash=? AND owner_sub=?` then `UPDATE agents SET status='deregistered', deregistered_at=? WHERE tenant_id=? AND status='active'` (both inside `async with session.begin():`) |
| `HGET apikey:{hash} status` | `get_api_key_status` | `SELECT status FROM api_keys WHERE api_key_hash = ?` |

#### `TaskStore` (renamed from `RedisTaskStore`)

| Current Redis op | Where | New SQL op |
|---|---|---|
| `HGET task:{id} created_at` (preserve), `HSET task:{id} mapping`, `ZADD tasks:ctx:{ctx} score=ts`, `SADD tasks:sender:{from}` | `save` | `INSERT … ON CONFLICT(task_id) DO UPDATE SET status_state=excluded.status_state, status_timestamp=excluded.status_timestamp, task_json=excluded.task_json` (preserves the original `created_at`) |
| `HGET task:{id} task_json` | `get` | `SELECT task_json FROM tasks WHERE task_id = ?` |
| `HGETALL task:{id}` + `DEL task:{id}` + `ZREM tasks:ctx:{ctx}` + `SREM tasks:sender:{from}` | `delete` | `DELETE FROM tasks WHERE task_id = ?` (single statement; indexes are auto-cleaned by SQLite) |
| `ZREVRANGE tasks:ctx:{ctx} 0 -1` + per-id `HGET task_json` | `list(context_id)` | `SELECT task_json FROM tasks WHERE context_id = ? ORDER BY status_timestamp DESC` |
| (used by webui) `SMEMBERS tasks:sender:{id}` + per-id `HGET task_json` | new `list_by_sender(agent_id)` | `SELECT task_json FROM tasks WHERE from_agent_id = ? ORDER BY status_timestamp DESC` (replaces the in-Python sort in `webui_api.get_sent`) |

#### `PubSubManager`

| Current Redis op | Where | New in-process op |
|---|---|---|
| `redis.publish("inbox:{id}", task_id)` | `publish` | Look up the channel's subscriber set and `put_nowait(task_id)` on each `asyncio.Queue` |
| `redis.pubsub().subscribe("inbox:{id}")` | `subscribe` | Create a fresh `asyncio.Queue`, register it in `_subscribers[channel]`, return an async iterator wrapping it |
| `pubsub.unsubscribe()` + `aclose()` | `unsubscribe` | Remove the queue from `_subscribers[channel]`; if the channel set is empty, drop the channel entry |

#### Direct `_redis` leakage in `main.py` / `auth.py` / `webui_api.py`

The migration **eliminates** every direct `_redis.*` call by adding the missing store methods. After this migration, no module under `registry/src/hikyaku_registry/` may access `store._redis` (the attribute itself does not exist on the new SQL stores).

| Current leak | File | New store method |
|---|---|---|
| `store._redis.hget(f"apikey:{tenant_id}", "status")` | `auth.py:43`, `auth.py:77`, `main.py:322` | `RegistryStore.is_api_key_active(api_key_hash) -> bool` |
| `store._redis.hget(f"agent:{agent_id}", "api_key_hash")` | `auth.py:51`, `main.py:331` | `RegistryStore.verify_agent_tenant(agent_id, tenant_id) -> bool` (already exists; the leak is replaced with the existing method call) |
| `registry_store._redis.hget(f"task:{task_id}", "from_agent_id")` and `to_agent_id` | `main.py:186-187` | `TaskStore.get_endpoints(task_id) -> tuple[str, str] \| None` returning `(from_agent_id, to_agent_id)` |
| `store._redis.scan(cursor=…, match="agent:*")` + `HGETALL` filter to find deregistered agents in a tenant that still have messages | `webui_api.py:92-114` | `RegistryStore.list_deregistered_agents_with_tasks(tenant_id) -> list[dict]` implemented as `SELECT a.agent_id, a.name, a.description, a.registered_at FROM agents a WHERE a.tenant_id = ? AND a.status = 'deregistered' AND EXISTS (SELECT 1 FROM tasks t WHERE t.context_id = a.agent_id LIMIT 1)` |
| `store._redis.sismember(f"account:{user_id}:keys", tenant_id)` | `webui_api.py:134` | `RegistryStore.is_key_owner(api_key_hash, owner_sub) -> bool` (`SELECT 1 FROM api_keys WHERE api_key_hash = ? AND owner_sub = ?`) |
| `store._redis.hget(f"agent:{agent_id}", "name")` | `webui_api.py:153` | `RegistryStore.get_agent_name(agent_id) -> str` (returns `''` if the agent does not exist, matching today's `or ""` fallback) |
| `task_store._redis.hget(f"task:{task.id}", "created_at")` | `webui_api.py:169` | `TaskStore.get_created_at(task_id) -> str \| None` |
| `task_store._redis.smembers(f"tasks:sender:{agent_id}")` + per-id fetch + in-Python sort | `webui_api.py:288-302` | `TaskStore.list_by_sender(agent_id)` (see TaskStore table above; returns Task objects already sorted DESC by `status_timestamp`) |
| `store._redis.sismember(f"tenant:{tenant_id}:agents", body.from_agent_id)` | `webui_api.py:322` | `RegistryStore.verify_agent_tenant(from_agent_id, tenant_id)` (already exists) |

### Cleanup loop removal

Today the registry runs a background `_cleanup_loop` task that periodically calls `cleanup_expired_agents` to physically delete agent rows whose `status='deregistered'` and `deregistered_at` is older than `deregistered_task_ttl_days`. This is removed in v1:

- `registry/src/hikyaku_registry/cleanup.py` is **deleted**
- `_cleanup_loop` and the `lifespan` task that schedules it are **removed** from `main.py`
- `settings.deregistered_task_ttl_days` and `settings.cleanup_interval_seconds` are **removed** from `config.py`
- Deregistered agents continue to exist as rows with `status='deregistered'` indefinitely; their inbox tasks remain readable by the WebUI (which is the only consumer that surfaces deregistered agents)
- All "active" query paths (`list_active_agents`, broadcast recipient enumeration, `verify_agent_tenant` semantics in JSON-RPC) already filter by `status='active'`, so dead rows are invisible to normal traffic

This is a **behavior change**. It is called out again in the Behavioral Changes section. If physical cleanup is needed in the future, it can be reintroduced as an opt-in admin command (e.g., `hikyaku-registry db purge --older-than 30d`) without disturbing the runtime.

### Alembic Integration

#### Layout

```
registry/src/hikyaku_registry/
  alembic.ini                    # on disk; bundled into the wheel
  alembic/
    env.py                       # imports Base from hikyaku_registry.db.models
    script.py.mako               # standard Alembic template
    versions/
      0001_initial_schema.py     # autogenerated, then committed
```

Both `alembic.ini` and the `alembic/` directory live **inside** the package so they ship with the wheel. They are discoverable at runtime via `importlib.resources.files("hikyaku_registry")`.

`pyproject.toml` is updated to include the Alembic assets:

```toml
[tool.hatch.build.targets.wheel]
packages = ["src/hikyaku_registry"]
include = [
  "src/hikyaku_registry/alembic.ini",
  "src/hikyaku_registry/alembic/**/*",
]
```

(Hatchling already picks up everything under `src/hikyaku_registry/` by default; the `include` is defensive in case `.ini` files are filtered.)

#### env.py strategy

`env.py` imports `Base` from `hikyaku_registry.db.models` and `settings` from `hikyaku_registry.config`, then:

1. Reads the application database URL from `settings.database_url`. **`config.py` is the sole owner of `~` expansion**, so the value here is already absolute (e.g. `sqlite+aiosqlite:////home/alice/.local/share/hikyaku/registry.db` — four slashes total, the second pair being the absolute-path leading `/`). env.py does NOT call `os.path.expanduser`.
2. **Swaps the driver to the sync `pysqlite` driver** so the URL becomes `sqlite:////home/alice/.local/share/hikyaku/registry.db`. This is done via `make_url(...).set(drivername="sqlite")` (more robust than string `replace`, because it handles edge cases like an absent `+aiosqlite` suffix). Alembic's autogenerate and `op.execute` paths run synchronously, so the migration tooling uses the sync driver while the runtime app uses `aiosqlite`. This is the standard SQLAlchemy + Alembic + async pattern.
3. Configures Alembic with `target_metadata=Base.metadata` so autogenerate can detect schema drift.
4. Runs migrations using `context.configure(url=sync_url, target_metadata=target_metadata, render_as_batch=True)`. `render_as_batch=True` is required because SQLite cannot `ALTER TABLE` arbitrary columns; Alembic emulates this by creating a new table, copying data, and renaming.

```python
# registry/src/hikyaku_registry/alembic/env.py (excerpt)
from alembic import context
from sqlalchemy import engine_from_config, pool
from sqlalchemy.engine.url import make_url

from hikyaku_registry.config import settings
from hikyaku_registry.db.models import Base

target_metadata = Base.metadata

# settings.database_url is already-expanded (config.py owns expansion).
sync_url = make_url(settings.database_url).set(drivername="sqlite")

def run_migrations_offline() -> None:
    context.configure(
        url=str(sync_url),
        target_metadata=target_metadata,
        literal_binds=True,
        render_as_batch=True,
    )
    with context.begin_transaction():
        context.run_migrations()

def run_migrations_online() -> None:
    cfg = context.config.get_section(context.config.config_ini_section) or {}
    cfg["sqlalchemy.url"] = str(sync_url)
    connectable = engine_from_config(cfg, prefix="sqlalchemy.", poolclass=pool.NullPool)
    with connectable.connect() as connection:
        context.configure(
            connection=connection,
            target_metadata=target_metadata,
            render_as_batch=True,
        )
        with context.begin_transaction():
            context.run_migrations()
```

#### Autogenerate workflow

The development workflow is:

1. Edit `db/models.py`
2. Run `cd registry && uv run alembic -c src/hikyaku_registry/alembic.ini revision --autogenerate -m "describe change"` (developer-only; not exposed via the CLI in v1)
3. Review the generated `versions/000N_*.py` file, hand-edit if needed, commit
4. End users run `hikyaku-registry db init` to apply

For v1 there is exactly one revision: `0001_initial_schema.py`, which creates `api_keys`, `agents`, `tasks` and their indexes.

### CLI Specification

#### Entry point

Added to `registry/pyproject.toml`:

```toml
[project.scripts]
hikyaku-registry = "hikyaku_registry.cli:main"
```

The CLI module lives at `registry/src/hikyaku_registry/cli.py` and uses `click`, matching the existing client CLI for consistency.

#### Command tree

```
hikyaku-registry
└── db                               # click.Group, scaffolded for future expansion
    └── init                         # implemented in v1
        # future (out of scope for v1):
        # ├── current                # show current revision
        # ├── revision               # create a new revision (developer command)
        # └── downgrade              # rollback (operator command, dangerous)
```

#### `db init` behavior matrix

`db init` resolves the configured `database_url`, runs `Path(db_file).parent.mkdir(parents=True, exist_ok=True)` to create the directory if missing, and connects via the **sync** driver (the same one Alembic uses). It then inspects the DB and dispatches:

| State | Detection | Action | Exit |
|---|---|---|---|
| **DB file does not exist** | `Path.exists()` is False on the resolved file path | Create parent directories. Run `command.upgrade(cfg, "head")`. Print `"Created {path} and applied N migration(s) to head ({head_rev})"`. | 0 |
| **Empty schema** | DB exists, no tables present | Run `command.upgrade(cfg, "head")`. Print `"Applied N migration(s) to head ({head_rev})"`. | 0 |
| **At head** | `MigrationContext.get_current_revision()` equals `ScriptDirectory.get_current_head()` | No-op. Print `"Already at head ({head_rev}); nothing to do"`. | 0 |
| **Behind head** | Current revision is in the migration history but not at head | Run `command.upgrade(cfg, "head")`. Print `"Upgraded from {current_rev} to {head_rev}"`. | 0 |
| **Ahead of head** | Current revision exists but is not in the local script directory's history (or is downstream of head) | Print `"ERROR: DB schema is at revision {current_rev} which is unknown to this version of hikyaku-registry. Refusing to downgrade automatically."` to stderr. | 1 |
| **Legacy (tables exist, no `alembic_version` table)** | DB has tables but `MigrationContext.get_current_revision()` returns `None` | Print `"ERROR: DB has existing tables but no alembic_version. Run 'alembic stamp head' manually if you are sure the schema matches."` to stderr. **No auto-stamp.** | 1 |

`db init` is idempotent: running it twice on a fresh DB applies migrations once and then becomes a no-op.

### In-Process Pub/Sub Design

#### `PubSubManager` shape

```python
class PubSubManager:
    def __init__(self) -> None:
        self._subscribers: dict[str, set[asyncio.Queue[str]]] = {}

    async def publish(self, channel: str, message: str) -> None:
        for queue in self._subscribers.get(channel, ()):
            queue.put_nowait(message)   # unbounded; see Risks

    async def subscribe(self, channel: str) -> _Subscription:
        queue: asyncio.Queue[str] = asyncio.Queue()
        self._subscribers.setdefault(channel, set()).add(queue)
        return _Subscription(queue, self, channel)

    async def unsubscribe(self, channel: str, queue: asyncio.Queue[str]) -> None:
        subs = self._subscribers.get(channel)
        if subs is not None:
            subs.discard(queue)
            if not subs:
                self._subscribers.pop(channel, None)
```

`_Subscription` wraps the queue and exposes the same async iterator protocol as the current Redis-backed `_Subscription` so `event_generator` in `api/subscribe.py` does not need to change shape — only the import of `PubSubManager` is rewired.

#### Lifecycle

- A new `PubSubManager` is constructed once in `create_app()` and stored on the app state.
- The constructor takes no arguments — there is no Redis client to inject.
- `event_generator` calls `subscribe(channel)` on entry and `unsubscribe(channel, queue)` in its `finally` block, exactly as today. The cleanup is critical because abandoned queues leak memory.
- Multiple SSE clients may subscribe to the same channel (e.g., a developer connects two browser tabs to the same agent's inbox). Each gets an independent queue; `publish` writes to all of them.

#### Wire protocol — unchanged

The SSE event format yielded by `event_generator` (`event: message\nid: {task_id}\ndata: {task_json}\n\n`) is byte-for-byte identical to today. Subscribers (the existing client and WebUI) need no changes.

#### Single-process invariant

The fan-out is **in-process only**. If the registry is started with `uvicorn --workers N` where `N > 1`, a publish in worker A will not reach a subscriber in worker B, silently breaking message delivery.

| Constraint | Scope |
|---|---|
| Server worker count must be 1 | The registry **server** (`mise //registry:dev` and any uvicorn invocation) — the `lifespan`-managed `PubSubManager` lives in worker memory |
| CLI invocations | **Unconstrained**. `hikyaku-registry db init` connects to SQLite directly and does not interact with Pub/Sub. Multiple CLI processes can run concurrently with the server without issue. SQLite handles cross-process locking. |

The constraint is enforced by **documentation only** for v1 — there is no startup-time guard. `docs/` and `ARCHITECTURE.md` will state the single-worker requirement explicitly. A future enhancement could detect `--workers N > 1` at startup and refuse to start, but it is out of scope for this design doc.

#### Overflow policy

Queues are **unbounded** (`asyncio.Queue()` with no `maxsize`). This matches the existing Redis Pub/Sub risk profile: a client that subscribes and stops reading can grow the in-memory backlog. The existing `event_generator` polls `request.is_disconnected()` and breaks out of the loop on disconnect, which calls `unsubscribe` and drops the queue, so realistic disconnects are bounded by the disconnect-poll interval (`_poll_interval = 0.5s`).

A bounded queue with drop-oldest semantics is recorded in **Risks & Open Questions** as a follow-up.

### Behavioral Changes

This migration is a **hard cutover** with several behavior changes that operators need to know about. They are enumerated here so they can be lifted into release notes.

| # | Change | Rationale |
|---|---|---|
| 1 | **Hard cutover from Redis.** No data is migrated. All existing tenants and agents must re-register after the upgrade. | This is a pre-1.0 internal tool; building a Redis-to-SQLite import script is more work than re-registering. |
| 2 | **No physical cleanup.** Deregistered agents and their tasks remain in the DB forever. The `_cleanup_loop` background task is removed; `deregistered_task_ttl_days` and `cleanup_interval_seconds` settings are removed. | User chose soft-delete only. Active query paths already filter `status='active'` so dead rows are invisible to normal traffic. |
| 3 | **Single-worker server.** The registry server must run with `uvicorn --workers=1` (the default). Multi-worker mode breaks Pub/Sub fan-out. | SQLite has no equivalent of Redis Pub/Sub or PostgreSQL `LISTEN/NOTIFY`. |
| 4 | **First-run UX changes.** Operators now run `hikyaku-registry db init` once before starting the server. Without it, the first request fails with `OperationalError: no such table: agents`. | Schema must exist before the app accepts traffic. |
| 5 | **`REDIS_URL` env var is removed.** Replaced with `HIKYAKU_DATABASE_URL`. Default: `sqlite+aiosqlite:///~/.local/share/hikyaku/registry.db` (with `~` expanded at config load time). | Aligns with the new backend. |
| 6 | **`fakeredis` is no longer a test dependency.** The test fixture stack uses an in-memory aiosqlite engine. | Tests align with production backend. |

### Testing Strategy

#### Fixture stack

`registry/tests/conftest.py` is fully rewritten:

```python
@pytest.fixture(scope="session")
async def db_engine():
    engine = create_async_engine("sqlite+aiosqlite:///:memory:")
    # Enable FK PRAGMA on the in-memory engine too
    @event.listens_for(engine.sync_engine, "connect")
    def _fk_on(dbapi_conn, _):
        dbapi_conn.execute("PRAGMA foreign_keys=ON")
    async with engine.begin() as conn:
        await conn.run_sync(Base.metadata.create_all)
    yield engine
    await engine.dispose()

@pytest.fixture(scope="session")
def db_sessionmaker(db_engine) -> async_sessionmaker[AsyncSession]:
    return async_sessionmaker(db_engine, expire_on_commit=False)

@pytest.fixture
async def db_session(db_sessionmaker):
    """Raw session for tests that need direct DB access (NOT passed to stores)."""
    async with db_sessionmaker() as session:
        yield session
        await session.rollback()

@pytest.fixture
def store(db_sessionmaker) -> RegistryStore:
    return RegistryStore(db_sessionmaker)

@pytest.fixture
def task_store(db_sessionmaker) -> TaskStore:
    return TaskStore(db_sessionmaker)

@pytest.fixture
def pubsub_manager() -> PubSubManager:
    return PubSubManager()
```

**Why stores take the sessionmaker, not a session.** Stores own their session lifecycle (see `RegistryStore.revoke_api_key` pseudocode in §"Store ownership of sessions"). The `store` and `task_store` fixtures therefore receive `db_sessionmaker` and pass it through. The `db_session` fixture remains for tests that need to inspect raw DB state (e.g. asserting that a row was actually committed) — it is **not** passed to store constructors.

**Test isolation under in-memory `:memory:`.** The session-scoped engine + `create_all` produces one shared in-memory database for the whole test session. Tests must clean up state they create, or rely on transactional rollback inside `db_session` for assertions that touch raw rows. Since stores own their own sessions, store-driven test mutations are committed and visible across tests. If isolation matters for a specific test file, that test fixture can override `db_engine` with a function-scoped variant.

**Trade-off:** the in-memory fixture uses `Base.metadata.create_all`, which bypasses Alembic. A schema change that is correct in models but missing from a migration would slip through this fixture stack. To catch that, a **separate session-level Alembic smoke test** runs against a real tempfile DB:

```python
# registry/tests/test_alembic_smoke.py
def test_alembic_upgrade_head(tmp_path):
    db_file = tmp_path / "smoke.db"
    cfg = build_alembic_config(f"sqlite:///{db_file}")
    command.upgrade(cfg, "head")
    # Verify expected tables exist
    engine = create_engine(f"sqlite:///{db_file}")
    insp = inspect(engine)
    assert {"api_keys", "agents", "tasks", "alembic_version"} <= set(insp.get_table_names())
```

This is the **only** test that exercises Alembic. It runs once per session, is fast (< 1s), and catches the model-vs-migration drift case.

#### Test file impact

Every test file under `registry/tests/` that imports `fakeredis` or constructs `RegistryStore(redis_client)` is rewritten to use the new fixtures. The list is finite and known: `test_registry_store.py`, `test_registry_api.py`, `test_subscribe.py`, `test_task_store.py`, `test_webui_api.py`, `test_webui_auth_migration.py`, `test_webui_mount.py`, `test_a2a.py`, `test_agent_auth_changes.py`, `test_api_keys.py`, `test_auth.py`, `test_cleanup.py` (deleted entirely), `test_e2e_subscribe.py`, `test_executor.py`, `test_key_endpoints.py`, `test_pubsub.py`.

`test_cleanup.py` is **deleted** since cleanup is gone.

#### New regression tests

| Test | Verifies |
|---|---|
| `test_db_engine_fk_pragma` | A fresh connection from the engine has `PRAGMA foreign_keys` returning 1 |
| `test_create_agent_rejects_unknown_tenant` | `INSERT INTO agents` with a `tenant_id` that does not exist in `api_keys` raises `IntegrityError` |
| `test_revoke_api_key_atomic` | Revoking a key with N agents results in all N agents `status='deregistered'` after a single transaction; injecting a failure mid-loop rolls back the API key flip too |
| `test_list_api_keys_no_n_plus_one` | Snapshot the SQL emitted by `list_api_keys` and assert it is exactly one query |
| `test_db_init_idempotent` | Running the CLI twice on a tmp DB applies once and then no-ops |
| `test_db_init_legacy_errors` | A DB with hand-created tables but no `alembic_version` row exits non-zero |
| `test_pubsub_fanout_two_subscribers` | Two subscribers on the same channel both receive a published message |
| `test_pubsub_unsubscribe_releases_queue` | After unsubscribe, the channel entry is removed and a republish does not raise |

### Alternatives Considered

| Alternative | Why rejected |
|---|---|
| **Stay on Redis** | Does not solve any of the six pain points enumerated in Background; keeps the operational dependency. |
| **PostgreSQL + LISTEN/NOTIFY** | Overkill for a single-process internal tool. Adds an even heavier daemon than Redis. The SQLite path can be replaced with PostgreSQL later if multi-process is ever needed; the SQLAlchemy ORM layer makes that swap mostly mechanical. |
| **DuckDB** | Optimized for OLAP workloads; the registry is OLTP (small row-level reads/writes). Wrong tool. |
| **Document DB (TinyDB, etc.)** | We already have one document store (Redis). The whole point is to gain SQL indexes; switching to another schemaless store would defeat the purpose. |
| **Valkey / KeyDB** | Drop-in Redis replacements. Still require a daemon — fails the "remove operational dependency" goal. |
| **Enable WAL journaling** | Premature optimization for v1: single-process server, low write throughput, rare CLI use. Default rollback journal is sufficient. Recorded as a future toggle in Risks. |
| **Bounded queue with drop-oldest fan-out** | The existing Redis Pub/Sub path is also unbounded in practice; matching that risk profile keeps the migration scoped. Recorded as a future enhancement. |
| **Migrations as a separate `hikyaku-registry-migrations` package** | Extra packaging surface for no operator benefit. Bundling Alembic inside the wheel keeps the deployment artifact count to one. |

### Risks & Open Questions

| Risk | Mitigation |
|---|---|
| **Write contention under default journal mode.** SQLite serializes writes; concurrent SendMessage + cleanup + WebUI could contend. | Single-worker server keeps the writer count low. Monitor in practice; enable WAL if `database is locked` errors appear. |
| **Schema-vs-migration drift.** The in-memory test fixture uses `create_all`, not Alembic. A model change without a migration would pass tests but fail at runtime. | The session-level Alembic smoke test catches this on every CI run. |
| **Unbounded Pub/Sub queue under stalled SSE clients.** A subscriber that stops reading but does not disconnect grows memory monotonically. | `request.is_disconnected()` polling in `event_generator` (`_poll_interval = 0.5s`) limits the leak to ~half a second of in-flight messages per orphan. Bounded queues are tracked as future work. |
| **Operator forgets `db init` on first run.** First request fails with `no such table`. | Document prominently in `docs/` and the README, and have `mise //registry:dev` print a one-line reminder in its banner. |
| **`render_as_batch=True` overhead** when migrations need to alter existing columns. | Acceptable; SQLite has no native ALTER for column types and batch is the canonical workaround. |
| **`db init` with a DB ahead of head** is treated as an error. An operator who genuinely downgraded code (rolled back a deployment) cannot recover via `db init`. | Out of scope for v1. Future `db downgrade` command or manual `alembic downgrade` is the recovery path. |

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

> **Documentation-first ordering is mandatory.** Per `.claude/rules/design-doc-numbering.md`, Step 0 must complete BEFORE any code change in steps 1-13. Reviewers should reject any PR that touches `registry/src/` without first updating `ARCHITECTURE.md`, `docs/`, both `CLAUDE.md` files, and the `/hikyaku` skill in the same change set.

### Step 0: Documentation updates (MUST land first)

- [x] Update `ARCHITECTURE.md`: replace Redis section with SQLite + SQLAlchemy + Alembic; describe the relational+document hybrid; describe the in-process Pub/Sub fan-out; document the single-worker constraint <!-- completed: 2026-04-11T06:40 -->
- [x] Update `docs/spec/data-model.md` (or equivalent): replace Redis key tables with SQL schema tables; document the operation mapping <!-- completed: 2026-04-11T06:45 -->
- [x] Update `docs/spec/registry-api.md` (or equivalent): note that `db init` is a prerequisite for first run; remove any Redis references <!-- completed: 2026-04-11T06:48 -->
- [x] Update root `CLAUDE.md`: change Tech Stack from "FastAPI + Redis + a2a-sdk" to "FastAPI + SQLAlchemy/aiosqlite + Alembic + a2a-sdk"; add `hikyaku-registry` as a registry CLI command <!-- completed: 2026-04-11T06:55 -->
- [x] Update `.claude/CLAUDE.md`: same Tech Stack change <!-- completed: 2026-04-11T06:55 -->
- [x] Update `.claude/rules/commands.md` if it references Redis (it should not, but verify) <!-- completed: 2026-04-11T06:56 -->
- [x] Update the `/hikyaku` skill documentation in `.claude/skills/hikyaku/` if it mentions Redis <!-- completed: 2026-04-11T06:56 -->
- [x] Update README.md (run `/update-readme` after the docs above land) <!-- completed: 2026-04-11T07:00 -->

### Step 1: Dependencies + config

- [x] `registry/pyproject.toml`: remove `redis[hiredis]`; add `sqlalchemy>=2.0`, `alembic`, `aiosqlite` <!-- completed: 2026-04-11T07:35 -->
- [x] `registry/pyproject.toml`: add `[project.scripts]` table with `hikyaku-registry = "hikyaku_registry.cli:main"` <!-- completed: 2026-04-11T07:35 -->
- [x] `registry/pyproject.toml`: add `[tool.hatch.build.targets.wheel] include` entries for `alembic.ini` and `alembic/**/*` <!-- completed: 2026-04-11T07:35 -->
- [x] Workspace root: remove `fakeredis` from any test dependency group that includes it <!-- completed: 2026-04-11T07:40 -->
- [x] `registry/src/hikyaku_registry/config.py`: remove `redis_url`, `deregistered_task_ttl_days`, `cleanup_interval_seconds`; add `database_url: str` with the XDG default constructed at `__init__` time using `os.path.expanduser` <!-- completed: 2026-04-11T07:42 -->
- [x] Run `uv sync` from the project root to lock the new dependencies <!-- completed: 2026-04-11T07:55 -->

### Step 2: Schema + Base + engine

- [x] Create `registry/src/hikyaku_registry/db/__init__.py` <!-- completed: 2026-04-11T08:30 -->
- [x] Create `registry/src/hikyaku_registry/db/models.py` with `Base = DeclarativeBase`, `ApiKey`, `Agent`, `Task` mapped classes matching the schema in this design doc, including the three indexes <!-- completed: 2026-04-11T08:30 -->
- [x] Create `registry/src/hikyaku_registry/db/engine.py` exposing `get_engine() -> AsyncEngine`, `get_sessionmaker() -> async_sessionmaker[AsyncSession]`, `dispose_engine()`, and an `event.listens_for(Engine, "connect")` callback that issues `PRAGMA foreign_keys=ON` <!-- completed: 2026-04-11T08:30 -->
- [x] Add a regression test `test_db_engine_fk_pragma` confirming the PRAGMA is active on a fresh connection <!-- completed: 2026-04-11T08:30 -->

### Step 3: Alembic scaffolding

- [x] Create `registry/src/hikyaku_registry/alembic.ini` with `script_location = %(here)s/alembic` (resolved relative to the .ini file's location) and an empty `sqlalchemy.url` (env.py reads it from settings) <!-- completed: 2026-04-11T16:35 -->
- [x] Create `registry/src/hikyaku_registry/alembic/env.py` that imports `Base.metadata` and `settings.database_url`, uses `make_url(settings.database_url).set(drivername='sqlite')` to obtain a sync URL (no `~` expansion — config.py owns that), and runs `context.configure(url=sync_url, target_metadata=Base.metadata, render_as_batch=True)`. <!-- completed: 2026-04-11T16:35 -->
- [x] Create `registry/src/hikyaku_registry/alembic/script.py.mako` (standard template) <!-- completed: 2026-04-11T16:35 -->
- [x] Generate `registry/src/hikyaku_registry/alembic/versions/0001_initial_schema.py` via `alembic revision --autogenerate -m "initial schema"` and review/commit it <!-- completed: 2026-04-11T16:38 -->
- [x] Add `test_alembic_smoke.py` (session-scoped) that runs `command.upgrade(cfg, "head")` against a tempfile DB and asserts the four expected tables exist <!-- completed: 2026-04-11T16:40 -->

### Step 4: CLI `db init`

- [x] Create `registry/src/hikyaku_registry/cli.py` with click `main()`, `db` subgroup, and `init` command <!-- completed: 2026-04-11T09:10 -->
- [x] Implement the six-state behavior matrix from the CLI Specification section (empty file, empty schema, at head, behind, ahead, legacy-no-version-table) <!-- completed: 2026-04-11T09:10 -->
- [x] `db init` must `Path(db_file).parent.mkdir(parents=True, exist_ok=True)` before connecting <!-- completed: 2026-04-11T09:10 -->
- [x] Add `test_db_init_creates_schema`, `test_db_init_idempotent`, `test_db_init_legacy_errors`, `test_db_init_ahead_errors` <!-- completed: 2026-04-11T09:10 -->
- [x] Verify the entry point installs by running `uv run hikyaku-registry db init --help` and confirming output <!-- completed: 2026-04-11T09:10 -->

### Step 5: Rewrite `RegistryStore`

- [x] Replace `registry/src/hikyaku_registry/registry_store.py` with a class that takes an `async_sessionmaker` bound to the app's async engine instead of a Redis client. No `_redis` attribute. Each method opens its own session via `async with self._sessionmaker() as session:`. <!-- completed: 2026-04-11T17:20 -->
- [x] Implement `create_agent`, `get_agent`, `list_active_agents`, `deregister_agent`, `verify_agent_tenant`, `create_api_key`, `list_api_keys` (single JOIN+GROUP BY), `revoke_api_key` (single transaction), `get_api_key_status` per the Operation Mapping table <!-- completed: 2026-04-11T17:20 -->
- [x] Add the new methods needed to remove `_redis` leaks: `is_api_key_active`, `is_key_owner`, `get_agent_name`, `list_deregistered_agents_with_tasks` <!-- completed: 2026-04-11T17:20 -->
- [x] Rewrite `registry/tests/test_registry_store.py` to use `db_session` and `store` fixtures <!-- completed: 2026-04-11T18:00 (e3c643b — 40 tests covering all 13 RegistryStore methods) -->
- [x] Add `test_revoke_api_key_atomic` and `test_list_api_keys_no_n_plus_one` <!-- completed: 2026-04-11T17:20 (in tests/test_registry_store_sql.py, commit 6872433) -->

### Step 6: Rewrite task store (`RedisTaskStore` → `TaskStore`)

- [x] Rename the file's main class from `RedisTaskStore` to `TaskStore`. Keep the file at `task_store.py` <!-- completed: 2026-04-11T18:10 -->
- [x] Implement `save` (UPSERT preserving `created_at`), `get`, `delete`, `list(context_id)`, and the new `list_by_sender(agent_id)`, `get_endpoints(task_id)`, `get_created_at(task_id)` <!-- completed: 2026-04-11T18:10 -->
- [x] Update every importer to use `TaskStore` instead of `RedisTaskStore` <!-- completed: 2026-04-11T18:10 -->
- [x] Rewrite `registry/tests/test_task_store.py` to use the new fixtures <!-- completed: 2026-04-11T18:10 (Phase A, commit 6d923b1) -->

### Step 7: Rewrite `PubSubManager`

- [x] Replace `registry/src/hikyaku_registry/pubsub.py` with the in-process implementation from the Pub/Sub Design section <!-- completed: 2026-04-11T09:30 -->
- [x] Constructor takes no arguments <!-- completed: 2026-04-11T09:30 -->
- [x] `_Subscription` exposes the same async-iterator interface as today so `event_generator` does not need a structural change <!-- completed: 2026-04-11T09:30 -->
- [x] Rewrite `registry/tests/test_pubsub.py` to use the new fixtures; add `test_pubsub_fanout_two_subscribers` and `test_pubsub_unsubscribe_releases_queue` <!-- completed: 2026-04-11T09:30 -->

### Step 8: Rewrite `main.py`

- [x] Replace `from hikyaku_registry.redis_client import close_pool, get_redis` with engine factory imports <!-- completed: 2026-04-11T19:00 -->
- [x] Construct `engine`, `sessionmaker`, `RegistryStore`, `TaskStore`, `PubSubManager` (no-arg) in `create_app()` <!-- completed: 2026-04-11T19:00 -->
- [x] Update `lifespan` to dispose the engine on shutdown; **remove** the `_cleanup_loop` task and its cancellation block entirely <!-- completed: 2026-04-11T19:00 -->
- [x] Replace `registry_store._redis.hget(f"task:{task_id}", "from_agent_id")` and `to_agent_id` in `_handle_get_task` with `task_store.get_endpoints(task_id)` <!-- completed: 2026-04-11T19:00 -->
- [x] Replace `registry_store._redis.hget(f"apikey:{tenant_id}", "status")` and `agent:{id} api_key_hash` in `jsonrpc_endpoint` with `registry_store.is_api_key_active(...)` and `registry_store.verify_agent_tenant(...)` <!-- completed: 2026-04-11T19:00 -->
- [x] Delete `_cleanup_loop` function definition <!-- completed: 2026-04-11T19:00 -->

### Step 9: Rewrite `auth.py`

- [x] Replace `store._redis.hget(f"apikey:{tenant_id}", "status")` calls in `get_authenticated_agent` and `get_registration_tenant` with `store.is_api_key_active(...)` <!-- completed: 2026-04-11T19:30 -->
- [x] Replace `store._redis.hget(f"agent:{agent_id}", "api_key_hash")` with `store.verify_agent_tenant(agent_id, tenant_id)` <!-- completed: 2026-04-11T19:30 -->
- [x] Confirm the tuple return shape `(agent_id, tenant_id)` is unchanged <!-- completed: 2026-04-11T19:30 -->
- [x] Update `registry/tests/test_auth.py` accordingly <!-- completed: 2026-04-11T19:30 -->

### Step 10: Rewrite `api/subscribe.py` and `api/registry.py`

- [x] `api/subscribe.py`: drop the `redis` import; the file already takes `PubSubManager` and `TaskStore` via Depends, so the only change is the import path. Verify `event_generator` still passes its tests. <!-- completed: 2026-04-11T20:00 (no redis import existed; fixed the `pubsub.unsubscribe(channel)` call site to pass `subscription` per the new PubSubManager API) -->
- [x] `api/registry.py`: replace `from hikyaku_registry.redis_client import get_redis` with `from hikyaku_registry.db.engine import get_sessionmaker`. The `get_registry_store()` dependency returns `RegistryStore(get_sessionmaker())` by default. At runtime, `create_app()` continues to override this dependency with the pre-constructed `registry_store` singleton (same pattern as today via `app.dependency_overrides[get_registry_store] = _get_store`), so the override path stays unchanged — only the underlying store shape differs. <!-- completed: 2026-04-11T20:00 -->

### Step 11: Rewrite `webui_api.py`

- [x] Replace `get_webui_store`, `get_webui_task_store`, `get_webui_executor` to use the engine/session factory <!-- completed: 2026-04-11T21:00 -->
- [x] Replace the `_get_tenant_agents` SCAN loop with `store.list_active_agents(tenant_id)` plus `store.list_deregistered_agents_with_tasks(tenant_id)` <!-- completed: 2026-04-11T21:00 -->
- [x] Replace `store._redis.sismember(...)` in `get_webui_tenant` with `store.is_key_owner(...)` <!-- completed: 2026-04-11T21:00 -->
- [x] Replace `_resolve_agent_name`'s `_redis.hget` with `store.get_agent_name(agent_id)` <!-- completed: 2026-04-11T21:00 -->
- [x] Replace `task_store._redis.hget(f"task:{id}", "created_at")` in `_format_message` with `task_store.get_created_at(task_id)` <!-- completed: 2026-04-11T21:00 -->
- [x] Replace `task_store._redis.smembers(f"tasks:sender:{id}")` + Python sort in `get_sent` with `task_store.list_by_sender(agent_id)` <!-- completed: 2026-04-11T21:00 -->
- [x] Replace `store._redis.sismember(f"tenant:{tenant_id}:agents", body.from_agent_id)` in `send_message` with `store.verify_agent_tenant(body.from_agent_id, tenant_id)` <!-- completed: 2026-04-11T21:00 -->
- [x] Confirm no `_redis` references remain anywhere in `webui_api.py` <!-- completed: 2026-04-11T21:00 -->
- [x] Rewrite `registry/tests/test_webui_api.py` accordingly <!-- completed: 2026-04-11T21:00 -->

### Step 12: Rewrite `conftest.py` and remaining tests

- [x] Replace `registry/tests/conftest.py` with the fixture stack documented in Testing Strategy <!-- completed: 2026-04-11T22:00 -->
- [x] Update every test file to use the new fixtures: `test_registry_api.py`, `test_subscribe.py`, `test_webui_api.py`, `test_webui_auth_migration.py`, `test_webui_mount.py`, `test_a2a.py`, `test_agent_auth_changes.py` (deleted — subsumed by `test_auth.py`), `test_api_keys.py` (deleted — subsumed by `test_registry_store.py`), `test_auth.py`, `test_e2e_subscribe.py`, `test_executor.py`, `test_key_endpoints.py`; `test_cleanup.py` deleted (cleanup.py removed in Step 13); removed transitional `addopts = "--continue-on-collection-errors"` from `registry/pyproject.toml` <!-- completed: 2026-04-11T22:00 -->
- [x] Run `mise //registry:test` and confirm all tests pass <!-- completed: 2026-04-11T22:00 (384/384 pass) -->

### Step 13: Remove dead modules

- [x] Delete `registry/src/hikyaku_registry/redis_client.py` <!-- completed: 2026-04-11T23:00 -->
- [x] Delete `registry/src/hikyaku_registry/cleanup.py` <!-- completed: 2026-04-11T23:00 -->
- [x] Delete `registry/tests/test_cleanup.py` <!-- completed: 2026-04-11T22:00 (pulled forward into Step 12) -->
- [x] Run `Grep` for `import redis`, `from redis`, `redis.asyncio`, `fakeredis`, `_redis`, `redis_url`, `cleanup_expired_agents`, `_cleanup_loop`, `deregistered_task_ttl_days`, `cleanup_interval_seconds` under `registry/` and confirm zero matches <!-- completed: 2026-04-11T23:00 (zero matches in registry/src; remaining matches in registry/tests/*.py are historical docstring references only) -->
- [x] Run `mise //:lint`, `mise //:format`, `mise //:typecheck`, `mise //registry:test` from the project root and confirm all pass <!-- completed: 2026-04-11T23:00 (384/384 tests pass) -->

---

## Changelog (optional -- spec revisions only, not implementation progress)

| Date | Changes |
|------|---------|
| 2026-04-11 | Initial draft |
| 2026-04-11 | Address reviewer feedback: session lifecycle, env.py URL, FK rationale, index symmetry, pseudocode snippets. |
| 2026-04-11 | Clean up leftover Option B / `~`-expansion references in step descriptions. |
| 2026-04-11 | Approved by user. COMMENT(claude) resolved: zero physical cleanup ever (no future purge command planned). |
| 2026-04-11 | Implementation complete. 384 tests passing, 32 commits on feat/sqlite-store-migration. |
