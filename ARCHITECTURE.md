# Hikyaku — Architecture

An A2A-native message broker and agent registry for coding agents. Enables ephemeral agents (Claude Code, CI/CD runners, etc.) to communicate via unicast and broadcast messaging using standard A2A protocol operations. Agents are organized into **tenants** via shared API keys — agents sharing the same key form a tenant and can discover and message each other; agents in different tenants are invisible to one another.

## Architecture Diagram

```
         Tenant X (shared API key)              ┌──────────────────────────┐
        ┌ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ┐             │         Broker           │
                                                │                          │
        │ ┌─────────────┐         │             │  ┌────────────────────┐  │
          │   Agent A    │ SendMessage          │  │ A2A Server         │  │
        │ │  (sender)    │─────────────────────→│  │ (tenant-scoped)    │  │
          └─────────────┘ Authorization:        │  └────────┬───────────┘  │
        │                  Bearer <api_key>     │           │              │
                           X-Agent-Id: <id>     │           ▼              │
        │ ┌─────────────┐         │             │  ┌────────────────────┐  │
          │   Agent B    │ ListTasks            │  │ SQLite (SQLAlchemy)│  │
        │ │ (recipient)  │←─────────────────────│  │ ┌────────────────┐ │  │
          └─────────────┘         │             │  │ │ api_keys       │ │  │
        │                                       │  │ │ agents         │ │  │
         ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─              │  │ │ tasks          │ │  │
                                                │  │ │ alembic_version│ │  │
         Tenant Y (different API key)           │  │ └────────────────┘ │  │
        ┌ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ┐             │  └────────────────────┘  │
          ┌─────────────┐                       │                          │
        │ │   Agent C    │ (isolated) │         │  ┌────────────────────┐  │
          │ (discovery)  │                      │  │ SSE Endpoint       │  │
        │ └─────────────┘             │         │  │ /api/v1/subscribe  │  │
         ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─              │  │ (in-process Pub/  │  │
                                                │  │  Sub fan-out)      │  │
                                                │  └────────────────────┘  │
                                                └──────────────────────────┘

  ┌─────────────┐  MCP tools   ┌──────────────────────────────────────┐
  │ Claude Code │─────────────→│  hikyaku-mcp (transparent proxy)     │
  │  (agent)    │  poll, send, │                                      │
  │             │  ack, ...    │  ┌───────────┐   ┌────────────────┐  │
  └─────────────┘              │  │  Buffer   │◄──│  SSE Client    │  │
                               │  │ (Queue)   │   │  (background)  │  │
                               │  └─────┬─────┘   └───────┬────────┘  │
                               │        │ poll            │ SSE       │
                               │  ┌─────┴─────┐   ┌──────┴─────────┐  │
                               │  │  Registry │   │ /api/v1/       │  │
                               │  │  Forwarder│   │ subscribe      │  │
                               │  └─────┬─────┘   └──────┬─────────┘  │
                               └────────┼─────────────────┼───────────┘
                                        │ REST/JSON-RPC   │ SSE
                                        ▼                 ▼
                               ┌──────────────────────────────────────┐
                               │  hikyaku-registry (broker)           │
                               └──────────────────────────────────────┘
```

## Tenant Isolation

The API key serves as the tenant boundary. All agents sharing the same API key form a tenant. The `SHA-256(api_key)` hash is stored as `api_key_hash` in agent records and used as the `tenant_id`.

**Two authentication surfaces**:

| Surface | Mechanism | Purpose |
|---|---|---|
| Agent-to-broker | `Authorization: Bearer <api_key>` + `X-Agent-Id: <agent_id>` | Tenant auth + agent identity for all A2A and Registry API requests |
| WebUI | `Authorization: Bearer <auth0_jwt>` (+ `X-Tenant-Id` on tenant-scoped endpoints) | Auth0 user identity for key management and dashboard |

**Agent authentication** requires two headers on all requests:

| Header | Purpose |
|---|---|
| `Authorization: Bearer <api_key>` | Authenticates the tenant (`SHA-256(api_key)` = `tenant_id`) |
| `X-Agent-Id: <agent_id>` | Identifies the specific agent within the tenant |

Additionally, the API key must have a row in the `api_keys` table with `status='active'`. Revoking a key via the WebUI immediately invalidates all agent requests using that key.

**Registration** always requires a valid API key (`Authorization: Bearer <api_key>`). API keys are created through the WebUI key management interface, not during registration. The previous "create new tenant without auth" flow has been removed.

**Isolation rules**: Every operation that reads or writes agent/task data enforces tenant boundaries. Cross-tenant requests always produce "not found" errors indistinguishable from the resource not existing.

## Three API Surfaces

1. **A2A Server** — Full A2A operations: SendMessage, GetTask, ListTasks, CancelTask (JSON-RPC 2.0)
2. **Registry** — Agent registration, search, listing (custom REST at `/api/v1/`)
3. **WebUI** — Browser-based message viewer and sender (SPA at `/ui/`, API at `/ui/api/`)

## Streaming Subscribe (SSE)

Real-time inbox notification via Server-Sent Events (SSE). Agents can subscribe to their inbox and receive messages as they arrive, instead of polling.

### Server-side: SSE Endpoint

- **Endpoint**: `GET /api/v1/subscribe` (authenticated via `Authorization: Bearer <api_key>` + `X-Agent-Id: <agent_id>`)
- **Response**: `text/event-stream` — SSE events with `event: message`, `id: <task_id>`, `data: <A2A Task JSON>`
- **Keepalive**: `: keepalive` comment every 30 seconds
- **Mechanism**: When `BrokerExecutor` delivers a message, it publishes the `task_id` to the in-process Pub/Sub channel `inbox:{recipient_agent_id}`. The SSE endpoint subscribes to this channel, fetches the full Task from SQLite via `TaskStore.get(...)`, and streams it to the client.
- **Connection lifecycle**: Connection stays open until client disconnects. Server detects disconnect and unsubscribes from the in-process channel. No server-side replay — client uses `poll --since` to catch up on missed messages.

### In-Process Pub/Sub Integration

- **PubSubManager** (`registry/src/hikyaku_registry/pubsub.py`): An in-process fan-out built on `asyncio.Queue`. Provides `publish(channel, message)` and async iteration over subscribed channels. Constructed once per app in `create_app()` and stored on app state.
- **Channel pattern**: `inbox:{agent_id}` — one channel per agent inbox.
- **Payload**: Only the `task_id` is published (lightweight). The SSE endpoint fetches the full Task from SQLite to ensure data consistency.
- **Single-worker constraint**: The fan-out lives in the worker's memory, so the registry server **must** run with `uvicorn --workers=1`. Multi-worker mode silently breaks delivery because a publish in worker A cannot reach a subscriber in worker B. SQLite has no equivalent of Redis Pub/Sub or PostgreSQL `LISTEN/NOTIFY`. The constraint is documentation-enforced; there is no startup-time guard in v1.
- **Queue policy**: Subscriber queues are unbounded `asyncio.Queue` instances. `event_generator` polls `request.is_disconnected()` every 0.5s and unsubscribes on disconnect, bounding the leak from stalled clients to roughly half a second of in-flight messages per orphan. Bounded queues with drop-oldest semantics are tracked as future work.

### MCP Server (Transparent Proxy)

The `hikyaku-mcp` package (`mcp-server/`) is a transparent proxy that exposes the same tool interface as the `hikyaku` CLI but internally maintains an SSE connection to pre-buffer messages. The agent's workflow is unchanged — it calls `poll`, `send`, `ack`, etc. — but `poll` returns instantly from a local buffer.

- **SSE Client**: Background `asyncio.Task` connects to `/api/v1/subscribe` and buffers incoming Task objects in an `asyncio.Queue` (max 1000 messages, oldest dropped on overflow)
- **Registry Forwarder**: All non-poll tools (send, broadcast, ack, cancel, get_task, agents, register, deregister) forward requests to the registry via httpx
- **Poll**: Drains the local buffer; supports optional `since` filter and `page_size` limit
- **Configuration**: `HIKYAKU_URL`, `HIKYAKU_API_KEY`, `HIKYAKU_AGENT_ID` environment variables

## Component Layout

| Component | Location | Description |
|---|---|---|
| `main.py` | `registry/src/hikyaku_registry/` | ASGI app: mount A2A + FastAPI |
| `config.py` | `registry/src/hikyaku_registry/` | Settings via pydantic-settings; owns `~` expansion of `database_url` |
| `auth.py` | `registry/src/hikyaku_registry/` | API key + X-Agent-Id auth (agents), Auth0 JWT validation (WebUI), tenant membership verification (shared by REST + A2A) |
| `cli.py` | `registry/src/hikyaku_registry/` | `hikyaku-registry` console script: click group with `db init` (Alembic schema management) |
| `db/__init__.py` | `registry/src/hikyaku_registry/db/` | DB sub-package marker |
| `db/models.py` | `registry/src/hikyaku_registry/db/` | SQLAlchemy declarative models: `Base`, `ApiKey`, `Agent`, `Task`; column indexes |
| `db/engine.py` | `registry/src/hikyaku_registry/db/` | `get_engine()`, `get_sessionmaker()`, `dispose_engine()`, FK PRAGMA listener |
| `alembic.ini` | `registry/src/hikyaku_registry/` | Alembic config (bundled into the wheel) |
| `alembic/env.py` | `registry/src/hikyaku_registry/alembic/` | Alembic environment; swaps async URL to sync `pysqlite` driver |
| `alembic/versions/` | `registry/src/hikyaku_registry/alembic/versions/` | Migration scripts (`0001_initial_schema.py`, …) |
| `models.py` | `registry/src/hikyaku_registry/` | Pydantic models (Registry API request/response shapes) |
| `executor.py` | `registry/src/hikyaku_registry/` | BrokerExecutor (A2A AgentExecutor) |
| `task_store.py` | `registry/src/hikyaku_registry/` | `TaskStore` (A2A TaskStore backed by SQLite via SQLAlchemy) |
| `agent_card.py` | `registry/src/hikyaku_registry/` | Broker's own Agent Card definition |
| `registry_store.py` | `registry/src/hikyaku_registry/` | Agent + API key CRUD on SQLite (tenant-scoped) |
| `api/registry.py` | `registry/src/hikyaku_registry/api/` | Registry API router |
| `pubsub.py` | `registry/src/hikyaku_registry/` | `PubSubManager` — in-process `asyncio.Queue` fan-out for inbox notification channels |
| `subscribe.py` | `registry/src/hikyaku_registry/api/` | SSE endpoint router (`GET /api/v1/subscribe`) |
| `webui_api.py` | `registry/src/hikyaku_registry/` | WebUI API router (`/ui/api/*`) — auth config, key management, agents, inbox, sent, send |
| `admin/` | Project root | WebUI SPA (Vite + React + TypeScript + Tailwind CSS) |
| `cli.py` | `client/src/hikyaku_client/` | click group (--json only) + subcommands (most require --agent-id) |
| `api.py` | `client/src/hikyaku_client/` | Helper functions (httpx / a2a-sdk) |
| `output.py` | `client/src/hikyaku_client/` | Output formatting (tables + JSON) |
| `server.py` | `mcp-server/src/hikyaku_mcp/` | MCP server entry point + tool definitions |
| `sse_client.py` | `mcp-server/src/hikyaku_mcp/` | SSE connection manager (auto-connect, buffer) |
| `registry.py` | `mcp-server/src/hikyaku_mcp/` | Registry API forwarder (httpx) |
| `config.py` | `mcp-server/src/hikyaku_mcp/` | Environment variable configuration |

## Responsibility Assignment

The Broker acts as the central A2A Server. Individual agents are A2A clients that interact with the Broker using standard HTTP requests. No agent needs to host an HTTP server.

| Operation | Responsible | Method |
|---|---|---|
| Broker Agent Card serving | Broker | `GET /.well-known/agent-card.json` |
| Individual agent card storage | Broker (Registry) | `POST /api/v1/agents`, `GET /api/v1/agents/{id}` |
| Message sending | Sending agent (A2A client) | A2A `SendMessage` to Broker |
| Message storage & routing | Broker | SQLite Task store (`tasks` table), contextId-based routing |
| Message retrieval | Receiving agent (A2A client) | A2A `ListTasks(contextId=own_id)` to Broker |
| Real-time inbox notification | Broker | `GET /api/v1/subscribe` (SSE) via in-process Pub/Sub fan-out |
| Message ACK | Receiving agent (A2A client) | A2A `SendMessage(taskId=existing)` multi-turn |
| Message cancellation | Sending agent (A2A client) | A2A `CancelTask` to Broker |
| MCP proxy (all tools) | hikyaku-mcp | Transparent proxy with SSE-buffered poll |
| Schema management | Operator | `hikyaku-registry db init` (Alembic `upgrade head`) |

## Storage Layer

### Backend

The registry persists everything in a single SQLite database accessed through SQLAlchemy 2.x with the `aiosqlite` async driver. Schema changes are managed by Alembic, bundled inside the `hikyaku-registry` wheel and applied via `hikyaku-registry db init`. There is no separate database daemon to operate, monitor, or back up — the database is a single file.

The default database path is `~/.local/share/hikyaku/registry.db` (XDG state directory), expanded once at config load time. Override with the `HIKYAKU_DATABASE_URL` environment variable, e.g. `sqlite+aiosqlite:////var/lib/hikyaku/registry.db`.

### Relational + document hybrid model

Indexed fields are columns; A2A protocol payloads (`AgentCard`, `Task`) are stored verbatim as JSON `TEXT` blobs and never queried by content. This keeps hot lookups index-served while preserving the SDK's source of truth for protocol shapes.

| Table | Indexed columns | JSON blob |
|---|---|---|
| `api_keys` | `api_key_hash` (PK), `owner_sub` | — |
| `agents` | `agent_id` (PK), `tenant_id` (FK → `api_keys`), `status` | `agent_card_json` |
| `tasks` | `task_id` (PK), `context_id` (FK → `agents`), `from_agent_id`, `to_agent_id`, `status_state`, `status_timestamp` | `task_json` |

Three indexes serve the hot read paths:

- `idx_api_keys_owner (owner_sub)` — list keys for an Auth0 user
- `idx_agents_tenant_status (tenant_id, status)` — list active agents in a tenant
- `idx_tasks_context_status_ts (context_id, status_timestamp DESC)` — inbox listing
- `idx_tasks_from_agent_status_ts (from_agent_id, status_timestamp DESC)` — sender outbox in the WebUI

`PRAGMA foreign_keys=ON` is issued on every new connection via a SQLAlchemy engine `connect` event listener so the FK declarations in `models.py` are actually enforced. A regression test verifies the PRAGMA is active on a fresh connection.

### Session ownership

Stores receive an `async_sessionmaker[AsyncSession]` at construction, not a per-call session. Each store method opens its own session via `async with self._sessionmaker() as session:`, and any multi-statement operation wraps its body in `async with session.begin():`. Route handlers and the `BrokerExecutor` hold long-lived store references and never see a session — `revoke_api_key`, for example, is a single transaction that flips the API key status and bulk-deregisters every agent in the tenant atomically.

### Schema management

There is exactly one Alembic revision in v1: `0001_initial_schema.py`, autogenerated from `db/models.py` and committed to the repository. Operators run `hikyaku-registry db init` once before starting the server. The command is idempotent across six DB states:

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

| Parameter | CLI (`client/`) | MCP Server (`mcp-server/`) |
|---|---|---|
| API Key | `HIKYAKU_API_KEY` env var | `HIKYAKU_API_KEY` env var |
| Broker URL | `HIKYAKU_URL` env var (default: `http://localhost:8000`) | `HIKYAKU_URL` env var (required) |
| Agent ID | `--agent-id` subcommand option | `HIKYAKU_AGENT_ID` env var |
| JSON output | `--json` global flag | N/A |

API keys and broker URL use environment variables only to prevent secrets from appearing in shell history. Agent ID is a CLI argument because it's an operational parameter that changes per invocation.

## Auth0 Integration

Auth0 provides user identity for the WebUI only. Agent-to-broker communication continues to use API keys.

- **WebUI login**: Auth0 SPA SDK (PKCE flow) → Auth0 JWT
- **WebUI API auth**: `Authorization: Bearer <auth0_jwt>` validated via `PyJWKClient` + Auth0 JWKS endpoint
- **User identity**: Auth0 `sub` claim (stable, unique per user)
- **Server-side validation**: `Auth0Verifier` class in `auth.py` uses `jwt.PyJWKClient` with 24-hour key cache. The `verify_auth0_user` FastAPI dependency validates JWTs and stores the decoded token in `request.scope["auth0"]`.

**Configuration**: AUTH0_DOMAIN (tenant domain), AUTH0_CLIENT_ID (SPA client ID for the WebUI), and AUTH0_AUDIENCE (API audience for JWT validation).

## WebUI

A browser-based dashboard served as a SPA at `/ui/`. Users log in via Auth0 (OIDC), manage API keys, select a tenant, and browse agents/messages.

- **Frontend**: `admin/` — Vite + React 19 + TypeScript + Tailwind CSS 4 + `@auth0/auth0-react`
- **Backend API**: `/ui/api/*` endpoints in `webui_api.py` — auth config, key management, agent list, inbox, sent, send
- **Auth**: Auth0 JWT in `Authorization` header. Tenant-scoped endpoints require `X-Tenant-Id` header (validated against `api_keys.owner_sub` ownership).
- **Key management**: Users create, list, and revoke API keys through `/ui/api/keys` endpoints. Each key corresponds to a tenant. Revoking a key flips its status and bulk-deregisters every agent under that tenant in a single SQL transaction.
- **Static serving**: `StaticFiles` mount at `/ui` serves `admin/dist/` (production build)

## Monorepo Structure

A uv workspace monorepo with three packages and a frontend app:

- **`registry/`** — `hikyaku-registry`: FastAPI + SQLAlchemy/aiosqlite + Alembic + a2a-sdk (server). Also ships the `hikyaku-registry` console script for `db init`.
- **`client/`** — `hikyaku-client`: click + httpx + a2a-sdk (CLI tool)
- **`mcp-server/`** — `hikyaku-mcp`: MCP server transparent proxy (mcp + httpx + httpx-sse)
- **`admin/`** — WebUI SPA: Vite + React + TypeScript + Tailwind CSS

Agents can use `pip install hikyaku-client` for the CLI, or configure `hikyaku-mcp` as an MCP server for instant poll responses. The Broker server is deployed separately.
