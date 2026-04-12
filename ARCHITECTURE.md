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
          └─────────────┘         │             │  │ │ api_keys         │ │  │
        │                                       │  │ │ agents           │ │  │
         ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─              │  │ │ tasks            │ │  │
                                                │  │ │ agent_placements │ │  │
                                                │  │ │ alembic_version  │ │  │
         Tenant Y (different API key)           │  │ └──────────────────┘ │  │
        ┌ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ┐             │  └────────────────────┘  │
          ┌─────────────┐                       └──────────────────────────┘
        │ │   Agent C    │ (isolated) │
          │ (discovery)  │
        │ └─────────────┘             │
         ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─
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
| `webui_api.py` | `registry/src/hikyaku_registry/` | WebUI API router (`/ui/api/*`) — auth config, key management, agents, inbox, sent, send |
| `admin/` | Project root | WebUI SPA (Vite + React + TypeScript + Tailwind CSS) |
| `cli.py` | `client/src/hikyaku_client/` | click group (--json only) + subcommands (most require --agent-id); includes `member` subgroup for Director lifecycle commands |
| `api.py` | `client/src/hikyaku_client/` | Helper functions (httpx / a2a-sdk) |
| `output.py` | `client/src/hikyaku_client/` | Output formatting (tables + JSON) |
| `tmux.py` | `client/src/hikyaku_client/` | tmux subprocess helper: `ensure_tmux_available`, `director_context`, `split_window`, `select_layout`, `send_exit`, `capture_pane` |

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
| `agent_placements` | `agent_id` (PK, FK → `agents` CASCADE), `director_agent_id` (FK → `agents` RESTRICT), `tmux_session`, `tmux_window_id`, `tmux_pane_id` (nullable) | — |

Four indexes serve the hot read paths:

- `idx_api_keys_owner (owner_sub)` — list keys for an Auth0 user
- `idx_agents_tenant_status (tenant_id, status)` — list active agents in a tenant
- `idx_tasks_context_status_ts (context_id, status_timestamp DESC)` — inbox listing
- `idx_tasks_from_agent_status_ts (from_agent_id, status_timestamp DESC)` — sender outbox in the WebUI
- `idx_placements_director (director_agent_id)` — list members spawned by a Director

`PRAGMA foreign_keys=ON` is issued on every new connection via a SQLAlchemy engine `connect` event listener so the FK declarations in `models.py` are actually enforced. A regression test verifies the PRAGMA is active on a fresh connection.

### Session ownership

Stores receive an `async_sessionmaker[AsyncSession]` at construction, not a per-call session. Each store method opens its own session via `async with self._sessionmaker() as session:`, and any multi-statement operation wraps its body in `async with session.begin():`. Route handlers and the `BrokerExecutor` hold long-lived store references and never see a session — `revoke_api_key`, for example, is a single transaction that flips the API key status and bulk-deregisters every agent in the tenant atomically.

### Schema management

Alembic revisions are committed to the repository: `0001_initial_schema.py`, `0002_add_origin_task_id.py`, and `0003_add_agent_placements.py`. Operators run `hikyaku-registry db init` once before starting the server. The command is idempotent across six DB states:

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

**Commands**: `member create`, `member delete`, `member list`, `member capture`. All require `--agent-id` (the Director's ID). The tmux helper module (`client/src/hikyaku_client/tmux.py`) isolates all subprocess interaction with tmux.

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

| Parameter | CLI (`client/`) |
|---|---|
| API Key | `HIKYAKU_API_KEY` env var |
| Broker URL | `HIKYAKU_URL` env var (default: `http://localhost:8000`) |
| Agent ID | `--agent-id` subcommand option |
| JSON output | `--json` global flag |

API keys and broker URL use environment variables only to prevent secrets from appearing in shell history. Agent ID is a CLI argument because it's an operational parameter that changes per invocation.

## Auth0 Integration

Auth0 provides user identity for the WebUI only. Agent-to-broker communication continues to use API keys.

- **WebUI login**: Auth0 SPA SDK (PKCE flow) → Auth0 JWT
- **WebUI API auth**: `Authorization: Bearer <auth0_jwt>` validated via `PyJWKClient` + Auth0 JWKS endpoint
- **User identity**: Auth0 `sub` claim (stable, unique per user)
- **Server-side validation**: `Auth0Verifier` class in `auth.py` uses `jwt.PyJWKClient` with 24-hour key cache. The `verify_auth0_user` FastAPI dependency validates JWTs and stores the decoded token in `request.scope["auth0"]`.

**Configuration**: AUTH0_DOMAIN (tenant domain), AUTH0_CLIENT_ID (SPA client ID for the WebUI), and AUTH0_AUDIENCE (API audience for JWT validation).

## WebUI

A browser-based dashboard served as a SPA at `/ui/`. Users log in via Auth0 (OIDC), manage API keys, select a tenant, and then interact with the tenant through a Discord-style unified timeline — a sidebar listing every active (top) and deregistered (muted) agent in the tenant, a center timeline rendering unicast and broadcast messages ordered newest-at-bottom with auto-scroll, reactions-as-ACKs chips that reveal per-recipient ACK time on CSS hover, and a bottom input that parses `@<agent> text` for unicast and `@all text` for broadcast. The admin itself is Auth0-authenticated but is NOT a Hikyaku agent; a header dropdown (sender selector) picks which real in-tenant active agent is used as `from_agent_id` on every send, persisted per-tenant in `localStorage` under `hikyaku.sender.<tenant_id>`.

- **Frontend**: `admin/` — Vite + React 19 + TypeScript + Tailwind CSS 4 + `@auth0/auth0-react`
- **Backend API**: `/ui/api/*` endpoints in `webui_api.py` — auth config, key management, agent list, inbox, sent, timeline (`GET /ui/api/timeline`), send (accepts `to_agent_id="*"` for broadcast)
- **Auth**: Auth0 JWT in `Authorization` header. Tenant-scoped endpoints require `X-Tenant-Id` header (validated against `api_keys.owner_sub` ownership).
- **Key management**: Users create, list, and revoke API keys through `/ui/api/keys` endpoints. Each key corresponds to a tenant. Revoking a key flips its status and bulk-deregisters every agent under that tenant in a single SQL transaction.
- **Static serving**: `StaticFiles` mount at `/ui` serves the SPA bundled inside the registry package at `registry/src/hikyaku_registry/webui/` (production build). `mise //admin:build` must be run before `mise //registry:dev` for `/ui/` to be populated; without it the server starts cleanly and `/ui/` simply 404s.

## Monorepo Structure

A uv workspace monorepo with two packages and a frontend app:

- **`registry/`** — `hikyaku-registry`: FastAPI + SQLAlchemy/aiosqlite + Alembic + a2a-sdk (server). Also ships the `hikyaku-registry` console script for `db init`.
- **`client/`** — `hikyaku-client`: click + httpx + a2a-sdk (CLI tool)
- **`admin/`** — WebUI SPA: Vite + React + TypeScript + Tailwind CSS

Agents use `pip install hikyaku-client` for the CLI. The Broker server is deployed separately.
