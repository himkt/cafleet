# Hikyaku — Architecture

An A2A-native message broker and agent registry for coding agents. Enables ephemeral agents (Claude Code, CI/CD runners, etc.) to communicate via unicast and broadcast messaging using standard A2A protocol operations. Agents are organized into **sessions** — a non-secret namespace created via `hikyaku session create`. Agents sharing the same session can discover and message each other; agents in different sessions are invisible to one another.

## Architecture Diagram

```
         Session X (shared session_id)          ┌──────────────────────────┐
        ┌ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ┐             │         Broker           │
                                                │                          │
        │ ┌─────────────┐         │             │  ┌────────────────────┐  │
          │   Agent A    │ SendMessage          │  │ A2A Server         │  │
        │ │  (sender)    │─────────────────────→│  │ (session-scoped)   │  │
          └─────────────┘ X-Agent-Id: <id>      │  └────────┬───────────┘  │
        │                                       │           │              │
                                                │           ▼              │
        │ ┌─────────────┐         │             │  ┌────────────────────┐  │
          │   Agent B    │ ListTasks            │  │ SQLite (SQLAlchemy)│  │
        │ │ (recipient)  │←─────────────────────│  │ ┌────────────────┐ │  │
          └─────────────┘         │             │  │ │ sessions         │ │  │
        │                                       │  │ │ agents           │ │  │
         ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─              │  │ │ tasks            │ │  │
                                                │  │ │ agent_placements │ │  │
                                                │  │ │ alembic_version  │ │  │
         Session Y (different session_id)       │  │ └──────────────────┘ │  │
        ┌ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ┐             │  └────────────────────┘  │
          ┌─────────────┐                       └──────────────────────────┘
        │ │   Agent C    │ (isolated) │
          │ (discovery)  │
        │ └─────────────┘             │
         ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─
```

## Session Isolation

The `session_id` serves as the namespace boundary. Sessions are created via `hikyaku session create` (direct SQLite write, no HTTP). All agents registered with the same `session_id` form one namespace. The broker does not perform authentication — it performs namespace routing only.

**Request headers**:

| Header | Purpose |
|---|---|
| `X-Session-Id: <session_id>` | Selects the session namespace (passed via header, body, or query depending on endpoint) |
| `X-Agent-Id: <agent_id>` | Identifies the specific agent within the session |

No bearer tokens, no API keys, no Auth0. The `session_id` is a non-secret namespace identifier. Sessions are namespaces for tidiness, not security boundaries.

**Registration** requires a valid `session_id` (passed in the POST body). Sessions are created via `hikyaku session create` before agents can register.

**Isolation rules**: Every operation that reads or writes agent/task data enforces session boundaries. Cross-session requests always produce "not found" errors indistinguishable from the resource not existing. Cross-session JSON-RPC sends are rejected with error code `-32003` ("Session mismatch").

## Three API Surfaces

1. **A2A Server** — Full A2A operations: SendMessage, GetTask, ListTasks, CancelTask (JSON-RPC 2.0)
2. **Registry** — Agent registration, search, listing (custom REST at `/api/v1/`)
3. **WebUI** — Browser-based message viewer and sender (SPA at `/ui/`, API at `/ui/api/`)

## Component Layout

| Component | Location | Description |
|---|---|---|
| `server.py` | `hikyaku/src/hikyaku/` | ASGI app: mount A2A + FastAPI |
| `config.py` | `hikyaku/src/hikyaku/` | Settings via pydantic-settings; owns `~` expansion of `database_url` |
| `auth.py` | `hikyaku/src/hikyaku/` | Session + agent-id resolution: `get_session_from_header` (X-Session-Id lookup), `get_session_from_agent_id` (X-Agent-Id → session_id lookup) |
| `cli.py` | `hikyaku/src/hikyaku/` | Unified `hikyaku` console script: click group with `db` (Alembic schema management), `session` (session namespace CRUD), and all agent/messaging commands (`register`, `send`, `poll`, `ack`, etc.) plus `member` subgroup |
| `db/__init__.py` | `hikyaku/src/hikyaku/db/` | DB sub-package marker |
| `db/models.py` | `hikyaku/src/hikyaku/db/` | SQLAlchemy declarative models: `Base`, `Session`, `Agent`, `Task`; column indexes |
| `db/engine.py` | `hikyaku/src/hikyaku/db/` | `get_engine()`, `get_sessionmaker()`, `dispose_engine()`, FK PRAGMA listener |
| `alembic.ini` | `hikyaku/src/hikyaku/` | Alembic config (bundled into the wheel) |
| `alembic/env.py` | `hikyaku/src/hikyaku/alembic/` | Alembic environment; swaps async URL to sync `pysqlite` driver |
| `alembic/versions/` | `hikyaku/src/hikyaku/alembic/versions/` | Migration scripts (`0001_initial_schema.py`, …) |
| `models.py` | `hikyaku/src/hikyaku/` | Pydantic models (Registry API request/response shapes) |
| `executor.py` | `hikyaku/src/hikyaku/` | BrokerExecutor (A2A AgentExecutor) |
| `task_store.py` | `hikyaku/src/hikyaku/` | `TaskStore` (A2A TaskStore backed by SQLite via SQLAlchemy) |
| `agent_card.py` | `hikyaku/src/hikyaku/` | Broker's own Agent Card definition |
| `registry_store.py` | `hikyaku/src/hikyaku/` | Agent + session CRUD on SQLite (session-scoped) |
| `api/registry.py` | `hikyaku/src/hikyaku/api/` | Registry API router |
| `webui_api.py` | `hikyaku/src/hikyaku/` | WebUI API router (`/ui/api/*`) — session list, agents, inbox, sent, send |
| `broker_client.py` | `hikyaku/src/hikyaku/` | httpx helpers for CLI agent operations |
| `output.py` | `hikyaku/src/hikyaku/` | CLI output formatting (tables + JSON) |
| `tmux.py` | `hikyaku/src/hikyaku/` | tmux subprocess helper: `ensure_tmux_available`, `director_context`, `split_window`, `select_layout`, `send_exit`, `capture_pane` |
| `admin/` | Project root | WebUI SPA (Vite + React + TypeScript + Tailwind CSS) |

## Responsibility Assignment

The Broker acts as the central A2A Server. Individual agents are A2A clients that interact with the Broker using standard HTTP requests. No agent needs to host an HTTP server.

| Operation | Responsible | Method |
|---|---|---|
| Broker Agent Card serving | Broker | `GET /.well-known/agent-card.json` |
| Individual agent card storage | Broker (Registry) | `POST /api/v1/agents`, `GET /api/v1/agents/{id}` |
| Message sending | Sending agent (A2A client) | A2A `SendMessage` to Broker |
| Message storage & routing | Broker | SQLite Task store (`tasks` table), contextId-based routing |
| Message retrieval | Receiving agent (A2A client) | A2A `ListTasks(contextId=own_id)` to Broker |
| Message ACK | Receiving agent (A2A client) | A2A `SendMessage(taskId=existing)` multi-turn |
| Message cancellation | Sending agent (A2A client) | A2A `CancelTask` to Broker |
| Schema management | Operator | `hikyaku db init` (Alembic `upgrade head`) |

## Storage Layer

### Backend

The registry persists everything in a single SQLite database accessed through SQLAlchemy 2.x with the `aiosqlite` async driver. Schema changes are managed by Alembic, bundled inside the `hikyaku` wheel and applied via `hikyaku db init`. There is no separate database daemon to operate, monitor, or back up — the database is a single file.

The default database path is `~/.local/share/hikyaku/registry.db` (XDG state directory), expanded once at config load time. Override with the `HIKYAKU_DATABASE_URL` environment variable, e.g. `sqlite+aiosqlite:////var/lib/hikyaku/registry.db`.

### Relational + document hybrid model

Indexed fields are columns; A2A protocol payloads (`AgentCard`, `Task`) are stored verbatim as JSON `TEXT` blobs and never queried by content. This keeps hot lookups index-served while preserving the SDK's source of truth for protocol shapes.

| Table | Indexed columns | JSON blob |
|---|---|---|
| `sessions` | `session_id` (PK) | — |
| `agents` | `agent_id` (PK), `session_id` (FK → `sessions`), `status` | `agent_card_json` |
| `tasks` | `task_id` (PK), `context_id` (FK → `agents`), `from_agent_id`, `to_agent_id`, `status_state`, `status_timestamp` | `task_json` |
| `agent_placements` | `agent_id` (PK, FK → `agents` CASCADE), `director_agent_id` (FK → `agents` RESTRICT), `tmux_session`, `tmux_window_id`, `tmux_pane_id` (nullable) | — |

Four indexes serve the hot read paths:

- `idx_agents_session_status (session_id, status)` — list active agents in a session
- `idx_tasks_context_status_ts (context_id, status_timestamp DESC)` — inbox listing
- `idx_tasks_from_agent_status_ts (from_agent_id, status_timestamp DESC)` — sender outbox in the WebUI
- `idx_placements_director (director_agent_id)` — list members spawned by a Director

`PRAGMA foreign_keys=ON` is issued on every new connection via a SQLAlchemy engine `connect` event listener so the FK declarations in `models.py` are actually enforced. A regression test verifies the PRAGMA is active on a fresh connection.

### Session ownership

Stores receive an `async_sessionmaker[AsyncSession]` at construction, not a per-call session. Each store method opens its own session via `async with self._sessionmaker() as session:`, and any multi-statement operation wraps its body in `async with session.begin():`. Route handlers and the `BrokerExecutor` hold long-lived store references and never see a session.

### Schema management

Alembic revisions are committed to the repository: `0001_initial_schema.py`, `0002_add_origin_task_id.py`, and `0003_add_agent_placements.py`. Operators run `hikyaku db init` once before starting the server. The command is idempotent across six DB states:

| State | Action |
|---|---|
| File missing | Create parent directory; `command.upgrade(cfg, "head")` |
| Empty schema | `command.upgrade(cfg, "head")` |
| At head | No-op; print "already at head" |
| Behind head | `command.upgrade(cfg, "head")`; print "upgraded from X to Y" |
| Ahead of head | Error; refuse to downgrade automatically |
| Legacy (tables exist, no `alembic_version`) | Error; instruct operator to run `alembic stamp head` manually |

Without `db init`, the first request fails with `OperationalError: no such table: agents`. The development workflow uses `alembic revision --autogenerate` directly; the `revision` and `downgrade` commands are not exposed via the CLI in v1.

### No physical cleanup

Deregistered agents and their tasks remain in the database forever. There is no background cleanup loop. Active query paths filter `status='active'` so dead rows are invisible to normal traffic; the WebUI is the only consumer that surfaces deregistered agents (so their inbox history can be inspected). If physical cleanup becomes necessary later, it can be added as an opt-in admin command without disturbing the runtime.

## Member Lifecycle

The `hikyaku member` CLI subgroup wraps the two-step "register an agent + spawn a tmux pane" recipe behind a single command and persists the agent-to-pane mapping in the registry SQLite store via the `agent_placements` table.

**Terminology**: A "member" is an agent spawned by a Director via `hikyaku member create`. It has an associated placement row linking it to a specific tmux pane, window, and session. The Director itself is NOT a member — it registers with plain `hikyaku register`.

**Atomic create flow** (`hikyaku member create`):

1. Register the member agent with a pending placement (`tmux_pane_id = NULL`) via `POST /api/v1/agents` with a `placement` object.
2. Spawn `claude <prompt>` in the Director's own tmux window via `tmux split-window -t <window_id>`, capturing the new pane ID.
3. Patch the placement row with the real pane ID via `PATCH /api/v1/agents/{id}/placement`.
4. Rebalance the window layout via `tmux select-layout main-vertical`.

If step 2 fails, the registered agent is rolled back via `DELETE /api/v1/agents/{id}`. If step 3 fails, the pane is `/exit`'d and the agent rolled back.

**Delete ordering** (`hikyaku member delete`): Deregister the agent first, THEN `/exit` the pane. This preserves the pane for retry if deregister fails.

**Commands**: `member create`, `member delete`, `member list`, `member capture`. All require `--agent-id` (the Director's ID). The tmux helper module (`hikyaku/src/hikyaku/tmux.py`) isolates all subprocess interaction with tmux.

**Supervision skill**: The Director's monitoring obligations are defined in `.claude/skills/hikyaku-monitoring/SKILL.md`. This skill must be loaded (`Skill(hikyaku-monitoring)`) before spawning any members. It provides a 2-stage health check protocol (message poll then terminal capture) and a ready-to-use `/loop` prompt template.

## Key Design Decisions

### contextId Convention

The Broker sets `contextId = recipient_agent_id` on every delivery Task. This enables inbox discovery — recipients call `ListTasks(contextId=myAgentId)` to find all messages addressed to them. This trades per-conversation grouping (the typical contextId use case) for simple inbox discovery, which suits the fire-and-forget messaging pattern of coding agents. The A2A spec (Section 3.4.1) states that server-generated contextId values should be treated as opaque identifiers by clients, so this usage is compliant.

### Task Lifecycle Mapping

Each message delivery is modeled as an A2A Task:

| Task State | Message Meaning |
|---|---|
| `TASK_STATE_INPUT_REQUIRED` | Message queued, awaiting recipient pickup (unread) |
| `TASK_STATE_COMPLETED` | Message acknowledged by recipient |
| `TASK_STATE_CANCELED` | Message retracted by sender before ACK |
| `TASK_STATE_FAILED` | Routing error (returned immediately to sender) |

### ASGI Mount Strategy

FastAPI is the parent ASGI application. The A2A SDK's `A2AStarletteApplication` is mounted at the root path. FastAPI routes (`/api/v1/*`) take priority; A2A protocol paths fall through to the mounted Starlette app.

```python
from fastapi import FastAPI
from a2a.server.apps.starlette import A2AStarletteApplication

fastapi_app = FastAPI()
fastapi_app.include_router(registry_router, prefix="/api/v1")

a2a_app = A2AStarletteApplication(agent_card=broker_card, http_handler=handler)
fastapi_app.mount("/", a2a_app.build())
```

### CLI Option Sources

Each CLI parameter has exactly one input source:

| Parameter | Source |
|---|---|
| Session ID | `HIKYAKU_SESSION_ID` env var |
| Broker URL | `HIKYAKU_URL` env var (default: `http://127.0.0.1:8000`) |
| Agent ID | `--agent-id` subcommand option |
| JSON output | `--json` global flag |

Session ID and broker URL use environment variables for convenience in tmux multi-pane workflows. Agent ID is a CLI argument because it's an operational parameter that changes per invocation.

## WebUI

A browser-based dashboard served as a SPA at `/ui/`. No login is required. The first-load lands on a session picker at `/ui/#/sessions`; selecting a session navigates to a Discord-style unified timeline for that session — a sidebar listing every active (top) and deregistered (muted) agent in the session, a center timeline rendering unicast and broadcast messages ordered newest-at-bottom with auto-scroll, reactions-as-ACKs chips that reveal per-recipient ACK time on CSS hover, and a bottom input that parses `@<agent> text` for unicast and `@all text` for broadcast. The admin is NOT a Hikyaku agent; a header dropdown (sender selector) picks which real in-session active agent is used as `from_agent_id` on every send, persisted per-session in `localStorage` under `hikyaku.sender.<session_id>`.

- **Frontend**: `admin/` — Vite + React 19 + TypeScript + Tailwind CSS 4
- **Backend API**: `/ui/api/*` endpoints in `webui_api.py` — session list, agent list, inbox, sent, timeline (`GET /ui/api/timeline`), send (accepts `to_agent_id="*"` for broadcast)
- **Session scoping**: Session-scoped endpoints require `X-Session-Id` header. No authentication.
- **Static serving**: `StaticFiles` mount at `/ui` serves the SPA bundled inside the package at `hikyaku/src/hikyaku/webui/` (production build). `mise //admin:build` must be run before `mise //hikyaku:dev` for `/ui/` to be populated; without it the server starts cleanly and `/ui/` simply 404s.

## Package Structure

A uv workspace with a single Python package and a frontend app:

- **`hikyaku/`** — `hikyaku`: FastAPI + SQLAlchemy/aiosqlite + Alembic + a2a-sdk + click + httpx (server + CLI). Ships the unified `hikyaku` console script for all operations: `db init`, `session` management, agent registration, messaging, and member lifecycle.
- **`admin/`** — WebUI SPA: Vite + React + TypeScript + Tailwind CSS

A single `pip install hikyaku` gives users both the broker server and the agent CLI.
