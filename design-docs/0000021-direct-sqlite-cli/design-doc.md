# Direct SQLite CLI

**Status**: Complete
**Progress**: 30/30 tasks complete
**Last Updated**: 2026-04-12

## Overview

Remove the HTTP/A2A server from the critical path entirely. All CLI commands and the admin WebUI access SQLite directly through a shared `broker` module (`cafleet/broker.py`). The A2A protocol layer (`executor.py`, `agent_card.py`, JSON-RPC handling, `broker_client.py`) and async stores (`registry_store.py`, `task_store.py`) are deleted. The admin WebUI remains as a minimal FastAPI app serving static files and calling `broker`.

## Success Criteria

- [x] All CLI commands operate without a running server
- [x] No `a2a-sdk` or `httpx` imports anywhere in the codebase
- [x] Admin WebUI endpoints call `broker` (not async stores or executor)
- [x] `broker` module is the single data access layer for both CLI and WebUI
- [x] `a2a-sdk`, `httpx`, and `aiosqlite` removed from package dependencies
- [x] Concurrent SQLite access handled via `PRAGMA busy_timeout=5000`
- [x] CLI output (human-readable and `--json`) is unchanged

---

## Background

CAFleet currently routes all agent operations through an HTTP server:

```
CLI (click) → broker_client.py (httpx) → server.py (FastAPI/JSON-RPC) → executor.py → async stores → SQLite
```

The `session` subgroup already bypasses this — `session create/list/show/delete` access SQLite directly with sync SQLAlchemy and raw SQL. The HTTP/A2A layer exists because the original design assumed remote agents communicating over the A2A protocol. Since all agents are local (same machine, tmux panes), the entire A2A server layer is unnecessary overhead: latency, operational complexity (must start server before any agent operation), and heavy dependencies (`a2a-sdk`, `httpx`) on the CLI path.

---

## Specification

### Architecture

**Before:**

```
CLI → broker_client.py (httpx) ─→ server.py (FastAPI) ─→ executor.py ─→ RegistryStore/TaskStore (async) → SQLite
                                   ↑ JSON-RPC + REST API                                                     ↑
                                   └─ webui_api.py ──────→ executor.py ─→ RegistryStore/TaskStore (async) ────┘
                                   └─ agent_card.py (A2A)
```

**After:**

```
CLI ──────→ broker.py (sync SQLAlchemy) → SQLite
                                            ↑
Admin WebUI → server.py (minimal FastAPI) → webui_api.py → broker.py ─┘
              (static files + /ui/api/*)
```

`broker.py` is the single data access layer. Both CLI and WebUI call it. The module IS the broker — `from cafleet import broker` reads naturally. No async stores, no executor, no A2A protocol.

### Task Data Format

Currently `task_json` stores `a2a.types.Task.model_dump_json()`. The new code stores `json.dumps(task_dict)` using plain dicts with camelCase keys:

```python
{
    "id": "<uuid>",
    "contextId": "<recipient_agent_id>",
    "status": {
        "state": "input_required",   # | "completed" | "canceled" | "failed"
        "timestamp": "<iso8601>"
    },
    "artifacts": [
        {
            "artifactId": "<uuid>",
            "parts": [{"kind": "text", "text": "<message>"}]
        }
    ],
    "metadata": {
        "fromAgentId": "<sender_agent_id>",
        "toAgentId": "<recipient_agent_id>",
        "type": "unicast",           # | "broadcast_summary"
        "originTaskId": "<uuid>"     # present on broadcast delivery tasks
    },
    "history": []
}
```

**Backward compatibility**: Existing `task_json` rows written by `a2a.types.Task` are valid camelCase dicts. `json.loads()` reads them correctly. No schema migration needed.

### Operation Mapping

| CLI Command | Current Path | New Path (`broker` function) |
|---|---|---|
| `register` | `broker_client.register_agent()` → POST /api/v1/agents | `broker.register_agent()` → INSERT agents [+ agent_placements] |
| `send` | `broker_client.send_message()` → JSON-RPC → `executor._handle_unicast()` | `broker.send_message()` → validate dest + INSERT tasks |
| `broadcast` | `broker_client.broadcast_message()` → JSON-RPC → `executor._handle_broadcast()` | `broker.broadcast_message()` → list agents + INSERT tasks per recipient + summary |
| `poll` | `broker_client.poll_tasks()` → JSON-RPC → `_handle_list_tasks()` | `broker.poll_tasks()` → SELECT tasks WHERE context_id |
| `ack` | `broker_client.ack_task()` → JSON-RPC → `executor._handle_ack()` | `broker.ack_task()` → verify recipient + UPDATE status → completed |
| `cancel` | `broker_client.cancel_task()` → JSON-RPC → `executor.cancel()` | `broker.cancel_task()` → verify sender + UPDATE status → canceled |
| `get-task` | `broker_client.get_task()` → JSON-RPC → `_handle_get_task()` | `broker.get_task()` → SELECT task + verify session |
| `agents` (list) | `broker_client.list_agents()` → GET /api/v1/agents | `broker.list_agents()` → SELECT agents WHERE active |
| `agents --id` | `broker_client.list_agents(agent_id=...)` → GET /api/v1/agents/{id} | `broker.get_agent()` → SELECT agent + placement |
| `deregister` | `broker_client.deregister_agent()` → DELETE /api/v1/agents/{id} | `broker.deregister_agent()` → UPDATE status + DELETE placement |
| `member create` | `register_agent()` + `patch_placement()` via HTTP | `broker.register_agent(placement=...)` + `broker.update_placement_pane_id()` |
| `member delete` | `list_agents(agent_id=...)` + `deregister_agent()` via HTTP | `broker.get_agent()` + `broker.deregister_agent()` |
| `member list` | `broker_client.list_members()` → GET /api/v1/agents?director_agent_id=... | `broker.list_members()` → SELECT agents JOIN placements |
| `member capture` | `broker_client.list_agents(agent_id=...)` → GET /api/v1/agents/{id} | `broker.get_agent()` → SELECT agent + placement |

### broker.py Module

Module-level functions using `get_sync_sessionmaker()` from `db/engine.py`. Each function opens a fresh session, executes within a transaction, and returns dicts. CLI functions return dicts matching `output.py` expectations. WebUI query functions return dicts matching `webui_api.py` response shapes.

Import: `from cafleet import broker`.

#### Session operations

```python
def create_session(label: str | None = None) -> dict:
    """INSERT into sessions. Returns {"session_id": ..., "label": ..., "created_at": ...}."""

def list_sessions() -> list[dict]:
    """SELECT sessions with active agent count.
    Returns [{"session_id": ..., "label": ..., "created_at": ..., "agent_count": ...}, ...].
    """

def get_session(session_id: str) -> dict | None:
    """SELECT single session. Returns dict or None."""

def delete_session(session_id: str) -> None:
    """DELETE session. Raises click.UsageError if FK constraint blocks deletion."""
```

#### Agent registry operations

```python
def register_agent(
    session_id: str,
    name: str,
    description: str,
    skills: list[dict] | None = None,
    placement: dict | None = None,
) -> dict:
    """INSERT into agents [+ agent_placements].
    Returns {"agent_id": ..., "name": ..., "registered_at": ...}.
    Validates session_id exists. When placement is provided, validates director
    exists and is active in the same session.
    """

def get_agent(agent_id: str, session_id: str) -> dict | None:
    """Single agent detail with optional placement.
    Returns {"agent_id": ..., "name": ..., "description": ..., "status": ...,
    "registered_at": ..., "placement": {...} | None} or None.
    Filters by session and excludes deregistered agents.
    """

def list_agents(session_id: str) -> list[dict]:
    """Active agents in session.
    Returns [{"agent_id": ..., "name": ..., "description": ...,
    "status": "active", "registered_at": ...}, ...].
    """

def deregister_agent(agent_id: str) -> bool:
    """UPDATE status='deregistered', deregistered_at=now, DELETE placement.
    Returns True if agent was active and got deregistered.
    """

def update_placement_pane_id(agent_id: str, pane_id: str) -> dict | None:
    """UPDATE agent_placements SET tmux_pane_id.
    Returns placement dict or None if no placement exists.
    """

def list_members(session_id: str, director_agent_id: str) -> list[dict]:
    """Member agents with placement info for a director.
    SELECT agents JOIN agent_placements WHERE director_agent_id.
    Returns [{"agent_id": ..., "name": ..., "status": ...,
    "placement": {...}}, ...].
    """

def verify_agent_session(agent_id: str, session_id: str) -> bool:
    """Check if agent belongs to session. Used by WebUI session validation."""
```

#### Messaging operations

```python
def send_message(session_id: str, agent_id: str, to: str, text: str) -> dict:
    """Unicast. Returns {"task": <camelCase task dict>}.
    Validation (mirrors executor._handle_unicast):
      1. Destination is valid UUID
      2. Destination agent exists and status='active'
      3. Destination agent is in the same session
    Creates task: status.state='input_required', context_id=destination.
    """

def broadcast_message(session_id: str, agent_id: str, text: str) -> list[dict]:
    """Broadcast. Returns [{"task": <summary task dict>}].
    Lists active agents in session (excluding sender). Creates one delivery task
    per recipient (type='unicast', originTaskId=summary_id) plus one summary
    (type='broadcast_summary', context_id=sender).
    """

def poll_tasks(
    agent_id: str,
    since: str | None = None,
    page_size: int | None = None,
    status: str | None = None,
) -> list[dict]:
    """Inbox query. Returns [<camelCase task dict>, ...].
    SELECT WHERE context_id=agent_id, ORDER BY status_timestamp DESC.
    Filters out type='broadcast_summary'.
    """

def ack_task(agent_id: str, task_id: str) -> dict:
    """ACK. Returns {"task": <updated task dict>}.
    Verifies context_id == agent_id. Verifies state == 'input_required'.
    Transitions to 'completed'.
    """

def cancel_task(agent_id: str, task_id: str) -> dict:
    """Cancel. Returns {"task": <updated task dict>}.
    Verifies metadata.fromAgentId == agent_id. Verifies state == 'input_required'.
    Transitions to 'canceled'.
    """

def get_task(session_id: str, task_id: str) -> dict:
    """Get task. Returns {"task": <task dict>}.
    Verifies fromAgentId or toAgentId belongs to session.
    """
```

#### WebUI query operations

```python
def list_session_agents(session_id: str) -> list[dict]:
    """Active agents + deregistered agents that have tasks.
    Returns [{"agent_id": ..., "name": ..., "description": ...,
    "status": "active"|"deregistered", "registered_at": ...}, ...].
    """

def list_inbox(agent_id: str) -> list[dict]:
    """Inbox tasks as raw dicts. SELECT WHERE context_id=agent_id,
    ORDER BY status_timestamp DESC. Filters out broadcast_summary.
    """

def list_sent(agent_id: str) -> list[dict]:
    """Sent tasks as raw dicts. SELECT WHERE from_agent_id=agent_id,
    ORDER BY status_timestamp DESC. Filters out broadcast_summary.
    """

def list_timeline(session_id: str, limit: int = 200) -> list[dict]:
    """Session-wide timeline. Returns [{"task": <dict>, "origin_task_id": ...,
    "created_at": ...}, ...]. Joins tasks with agents on session_id,
    filters broadcast_summary, ORDER BY status_timestamp DESC.
    """

def get_agent_names(agent_ids: list[str]) -> dict[str, str]:
    """Batch agent_id → name lookup. Returns {agent_id: name, ...}."""

def get_task_created_ats(task_ids: list[str]) -> dict[str, str]:
    """Batch task_id → created_at lookup. Returns {task_id: created_at, ...}."""
```

#### Internal helpers

```python
def _save_task(session, task_dict: dict) -> None:
    """INSERT with UPSERT. Promotes indexed fields to columns.
    Preserves created_at on re-save.
    """
    metadata = task_dict.get("metadata", {})
    stmt = sqlite_insert(TaskModel).values(
        task_id=task_dict["id"],
        context_id=task_dict["contextId"],
        from_agent_id=metadata.get("fromAgentId", ""),
        to_agent_id=metadata.get("toAgentId", ""),
        type=metadata.get("type", ""),
        created_at=_now_iso(),
        status_state=task_dict["status"]["state"],
        status_timestamp=task_dict["status"]["timestamp"],
        origin_task_id=metadata.get("originTaskId"),
        task_json=json.dumps(task_dict),
    )
    stmt = stmt.on_conflict_do_update(
        index_elements=["task_id"],
        set_={
            "status_state": stmt.excluded.status_state,
            "status_timestamp": stmt.excluded.status_timestamp,
            "origin_task_id": stmt.excluded.origin_task_id,
            "task_json": stmt.excluded.task_json,
        },
    )
    session.execute(stmt)

def _read_task(session, task_id: str) -> dict | None:
    """SELECT task_json by task_id. Returns parsed dict or None."""
```

### Sync Engine Infrastructure

Replace the async engine code in `db/engine.py` with sync-only equivalents. The async engine (`get_engine`, `get_sessionmaker`, `dispose_engine`) and `aiosqlite` import become dead code after all callers are migrated to `broker.py`.

Add to `db/engine.py`:

```python
_sync_engine: Engine | None = None
_sync_sessionmaker: sessionmaker[Session] | None = None

def get_sync_engine() -> Engine:
    global _sync_engine
    if _sync_engine is None:
        sync_url = str(make_url(settings.database_url).set(drivername="sqlite"))
        _sync_engine = create_engine(sync_url)
    return _sync_engine

def get_sync_sessionmaker() -> sessionmaker[Session]:
    global _sync_sessionmaker
    if _sync_sessionmaker is None:
        _sync_sessionmaker = sessionmaker(get_sync_engine(), expire_on_commit=False)
    return _sync_sessionmaker
```

Extend the existing event listener with `busy_timeout`:

```python
@event.listens_for(Engine, "connect")
def _enable_sqlite_pragmas(dbapi_connection, _connection_record) -> None:
    if not isinstance(dbapi_connection, sqlite3.Connection):
        return
    cursor = dbapi_connection.cursor()
    cursor.execute("PRAGMA foreign_keys=ON")
    cursor.execute("PRAGMA busy_timeout=5000")
    cursor.close()
```

### CLI Changes

1. **Drop `CAFLEET_URL`**: Remove from `cli()` context. CLI only needs `CAFLEET_SESSION_ID`.

2. **Update `env` command**: Print `CAFLEET_DATABASE_URL` and `CAFLEET_SESSION_ID`.

3. **Remove `_run()` wrapper**: No async calls remain in client commands.

4. **Replace imports**: `from cafleet import broker_client as api` → `from cafleet import broker`.

5. **Refactor session commands**: Call `broker.create_session()`, `broker.list_sessions()`, etc. instead of inline raw SQL with ad-hoc engine.

6. **member create env forwarding**: Replace `CAFLEET_URL` with optional `CAFLEET_DATABASE_URL`:

    ```python
    env = {
        "CAFLEET_SESSION_ID": session_id,
        "CAFLEET_AGENT_ID": new_agent_id,
    }
    db_url = os.environ.get("CAFLEET_DATABASE_URL")
    if db_url:
        env["CAFLEET_DATABASE_URL"] = db_url
    ```

### Admin WebUI Rewrite

`server.py` is rewritten as a minimal FastAPI app:

```python
# server.py (rewritten)
app = FastAPI(title="CAFleet Admin", version="0.1.0")
app.include_router(webui_router)     # /ui/api/* endpoints
app.mount("/ui", SPAStaticFiles(...))  # admin SPA static files
```

Removed from `server.py`: JSON-RPC endpoint (`POST /`), A2A handling, `agent_card` endpoint, `executor` instantiation, `RegistryStore`/`TaskStore` instantiation, all dependency overrides, `lifespan` async engine disposal.

`webui_api.py` is rewritten to call `broker`:

| Endpoint | Current Implementation | New Implementation |
|---|---|---|
| `GET /ui/api/sessions` | `await store.list_sessions()` | `broker.list_sessions()` |
| `GET /ui/api/agents` | `_get_session_agents()` via async store | `broker.list_session_agents()` |
| `GET /ui/api/agents/{id}/inbox` | `task_store.list()` + `_format_messages()` | `broker.list_inbox()` + `_format_messages()` |
| `GET /ui/api/agents/{id}/sent` | `task_store.list_by_sender()` + `_format_messages()` | `broker.list_sent()` + `_format_messages()` |
| `GET /ui/api/timeline` | `task_store.list_timeline()` + `_format_messages()` | `broker.list_timeline()` + `_format_messages()` |
| `POST /ui/api/messages/send` | `executor.execute()` via A2A types | `broker.send_message()` or `broker.broadcast_message()` |

The `_format_messages()` helper is rewritten to work with plain dicts instead of `a2a.types.Task` objects. It calls `broker.get_agent_names()` and `broker.get_task_created_ats()` for batch lookups. The `_extract_body()` helper changes from `part.root.text` (Pydantic model traversal) to `part.get("text", "")` (dict access).

Since `broker` functions are sync, the FastAPI endpoints are defined as `def` (not `async def`). FastAPI/Starlette runs sync handlers in a thread pool automatically — no `run_in_executor` boilerplate needed.

Session validation (`get_webui_session` dependency) becomes a sync function calling `broker.get_session()` and `broker.verify_agent_session()`.

### Concurrency Model

- **Journal mode**: SQLite default (DELETE). No WAL.
- **Locking**: Database-level. One writer at a time; readers do not block.
- **Contention handling**: `PRAGMA busy_timeout=5000` on every connection. SQLite retries internally for up to 5 seconds before returning `SQLITE_BUSY`.
- **Expected contention**: Low. CLI operations are short transactions (single INSERT or UPDATE). Multiple agents polling concurrently is read-only.

### Files Changed

| File | Action | Description |
|---|---|---|
| `cafleet/src/cafleet/broker.py` | **Create** | Single data access layer — sync SQLAlchemy operations for CLI + WebUI |
| `cafleet/src/cafleet/db/engine.py` | **Rewrite** | Replace async engine with sync-only; add `busy_timeout` pragma |
| `cafleet/src/cafleet/cli.py` | **Modify** | Replace `broker_client` with `broker`, drop `CAFLEET_URL`, refactor session commands |
| `cafleet/src/cafleet/server.py` | **Rewrite** | Minimal FastAPI: static files + `webui_router` only |
| `cafleet/src/cafleet/webui_api.py` | **Rewrite** | Call `broker` instead of async stores/executor, sync `def` handlers |
| `cafleet/src/cafleet/broker_client.py` | **Delete** | HTTP client, no longer needed |
| `cafleet/src/cafleet/executor.py` | **Delete** | A2A BrokerExecutor, logic moved to `broker` |
| `cafleet/src/cafleet/agent_card.py` | **Delete** | A2A Agent Card, no longer served |
| `cafleet/src/cafleet/registry_store.py` | **Delete** | Async store, replaced by `broker` |
| `cafleet/src/cafleet/task_store.py` | **Delete** | Async store, replaced by `broker` |
| `cafleet/src/cafleet/auth.py` | **Delete** | Async auth dependencies, replaced by sync in `webui_api.py` |
| `cafleet/src/cafleet/models.py` | **Delete** | REST API Pydantic models (RegisterAgentRequest, etc.), no longer needed |
| `cafleet/src/cafleet/api/registry.py` | **Delete** | REST API router, endpoints replaced by `broker` |
| `cafleet/pyproject.toml` | **Modify** | Remove `a2a-sdk`, `httpx`, `aiosqlite` from dependencies |
| `ARCHITECTURE.md` | **Modify** | New architecture, remove A2A/HTTP layer (see details below) |
| `README.md` | **Modify** | Remove server requirement for CLI |
| `.claude/skills/cafleet/SKILL.md` | **Modify** | Remove `CAFLEET_URL`, update examples |
| `CLAUDE.md`, `.claude/CLAUDE.md` | **Modify** | Update project description |

Files **not** changed: `db/models.py`, `output.py`, `config.py`, `tmux.py`, `coding_agent.py`, all Alembic migrations, `admin/` (SPA).

### Dependency Changes

Remove from `cafleet/pyproject.toml` `[project.dependencies]`:

| Package | Reason for Removal |
|---|---|
| `a2a-sdk` | Only used by deleted modules (`executor.py`, `task_store.py`, `webui_api.py`, `agent_card.py`) |
| `httpx` | Only used by deleted `broker_client.py` |
| `aiosqlite` | Only used by deleted async engine in `db/engine.py` |

Retained: `fastapi`, `uvicorn`, `sqlalchemy`, `alembic`, `click`, `pydantic`, `pydantic-settings`.

Update tech stack description in `CLAUDE.md` and `.claude/CLAUDE.md` from:
`FastAPI + SQLAlchemy/aiosqlite + Alembic + a2a-sdk + click + httpx`
to:
`FastAPI + SQLAlchemy + Alembic + click`

### ARCHITECTURE.md Updates

Specific sections to update:

- **Remove** "Three API Surfaces" section (A2A JSON-RPC and Registry REST are gone; only Admin WebUI API remains)
- **Remove** "ASGI Mount Strategy" section (no more A2A app mounting)
- **Remove** "Component Responsibility Matrix" entries for BrokerExecutor, RegistryStore, TaskStore
- **Update** "Component Layout" table: replace `broker_client.py`, `executor.py`, `registry_store.py`, `task_store.py` with `broker.py`
- **Update** "Storage Layer" section: replace async engine description with sync-only engine
- **Add** new architecture description: CLI and Admin WebUI both call `broker.py` → SQLite
- **Update** "Session Ownership" section: stores no longer receive `async_sessionmaker`; `broker.py` uses module-level sync `sessionmaker`
- **Update** "Design Decisions" section: remove A2A protocol rationale, add direct SQLite rationale

### Migration

- **Strategy**: Hard cutover. No backward-compatibility shim.
- **Schema**: Unchanged. No new Alembic migration.
- **Data**: Existing `task_json` rows are valid camelCase dicts. `json.loads()` reads them. Agents and sessions carry over.
- **Behavioral change**: CLI commands no longer require a running server. `cafleet dev` is only needed for the admin WebUI.

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

### Step 1: Documentation

- [x] Update `ARCHITECTURE.md` — remove "Three API Surfaces", "ASGI Mount Strategy", update Component Layout, Storage Layer, Design Decisions (see ARCHITECTURE.md Updates section) <!-- completed: 2026-04-13T13:35 -->
- [x] Update `README.md` — remove server requirement for CLI, update getting started <!-- completed: 2026-04-13T13:35 -->
- [x] Update `.claude/skills/cafleet/SKILL.md` — remove `CAFLEET_URL`, update CLI workflow <!-- completed: 2026-04-13T13:35 -->
- [x] Update `CLAUDE.md` and `.claude/CLAUDE.md` — reflect architecture change <!-- completed: 2026-04-13T13:35 -->
- [x] Update `.claude/rules/commands.md` — note `cafleet dev` is WebUI-only <!-- completed: 2026-04-13T13:35 -->

### Step 2: Sync engine infrastructure

- [x] Add `PRAGMA busy_timeout=5000` to event listener in `db/engine.py` (rename to `_enable_sqlite_pragmas`) <!-- completed: 2026-04-13T13:42 -->
- [x] Add `get_sync_engine()`, `get_sync_sessionmaker()` to `db/engine.py` <!-- completed: 2026-04-13T13:42 -->
- [x] Remove async engine functions (`get_engine`, `get_sessionmaker`, `dispose_engine`) and `aiosqlite` import from `db/engine.py` <!-- completed: 2026-04-13T13:42 -->

### Step 3: broker.py — session + registry operations

- [x] Create `broker.py` with session operations: `create_session`, `list_sessions`, `get_session`, `delete_session` <!-- completed: 2026-04-13T13:50 -->
- [x] Add agent registry operations: `register_agent`, `get_agent`, `list_agents`, `verify_agent_session` <!-- completed: 2026-04-13T13:50 -->
- [x] Add `deregister_agent`, `update_placement_pane_id`, `list_members` <!-- completed: 2026-04-13T13:50 -->

### Step 4: broker.py — messaging operations

- [x] Add internal helpers `_save_task` (UPSERT) and `_read_task` <!-- completed: 2026-04-13T13:55 -->
- [x] Add `send_message` with unicast validation and `broadcast_message` with fan-out <!-- completed: 2026-04-13T13:55 -->
- [x] Add `poll_tasks`, `ack_task`, `cancel_task`, `get_task` <!-- completed: 2026-04-13T13:55 -->

### Step 5: broker.py — WebUI query operations

- [x] Add `list_session_agents` (active + deregistered with tasks) <!-- completed: 2026-04-13T14:10 -->
- [x] Add `list_inbox`, `list_sent`, `list_timeline`, `get_agent_names`, `get_task_created_ats` <!-- completed: 2026-04-13T14:10 -->

### Step 6: CLI rewrite

- [x] Replace `broker_client` import with `broker`, drop `CAFLEET_URL`, remove `_run()` wrapper <!-- completed: 2026-04-13T14:20 -->
- [x] Rewrite session commands to call `broker` instead of inline raw SQL <!-- completed: 2026-04-13T14:20 -->
- [x] Rewrite client commands: `register`, `send`, `broadcast`, `poll`, `ack`, `cancel`, `get-task`, `agents`, `deregister` <!-- completed: 2026-04-13T14:20 -->
- [x] Rewrite member commands: `create` (env forwarding), `delete`, `list`, `capture` <!-- completed: 2026-04-13T14:20 -->
- [x] Update `env` command to print `CAFLEET_DATABASE_URL` instead of `CAFLEET_URL` <!-- completed: 2026-04-13T14:20 -->

### Step 7: Admin WebUI rewrite

- [x] Rewrite `server.py` — minimal FastAPI app with `webui_router` + static files only <!-- completed: 2026-04-13T14:30 -->
- [x] Rewrite `webui_api.py` — all endpoints call `broker`, sync `def` handlers, rewrite `_format_messages` for dicts <!-- completed: 2026-04-13T14:30 -->

### Step 8: Delete dead code + update dependencies

- [x] Delete `broker_client.py`, `executor.py`, `agent_card.py`, `auth.py`, `models.py`, `api/registry.py` <!-- completed: 2026-04-13T14:40 -->
- [x] Delete `registry_store.py`, `task_store.py` <!-- completed: 2026-04-13T14:40 -->
- [x] Remove `a2a-sdk`, `httpx`, `aiosqlite` from `pyproject.toml` dependencies <!-- completed: 2026-04-13T14:40 -->
- [x] Verify no remaining imports of deleted modules in any code path <!-- completed: 2026-04-13T14:40 -->

### Step 9: Tests

- [x] Unit tests for `broker` session + registry operations <!-- completed: 2026-04-13T13:42 -->
- [x] Unit tests for `broker` messaging operations (`send_message`, `broadcast_message`, `poll_tasks`, `ack_task`, `cancel_task`, `get_task`) <!-- completed: 2026-04-13T13:46 -->
- [x] Unit tests for `broker` WebUI query operations (`list_session_agents`, `list_inbox`, `list_sent`, `list_timeline`) <!-- completed: 2026-04-13T13:47 -->

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-04-12 | Initial draft |
| 2026-04-12 | Revision: rename local_ops→broker, add dependency removal (a2a-sdk/httpx/aiosqlite), add async engine cleanup, add models.py deletion, add ARCHITECTURE.md update specifics |
| 2026-04-13 | Implementation complete. All 30/30 tasks done. All 7 success criteria verified. 249 tests passing. Status → Complete |
