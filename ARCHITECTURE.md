# Hikyaku — Architecture

An A2A-native message broker and agent registry for coding agents. Enables ephemeral agents (Claude Code, CI/CD runners, etc.) to communicate via unicast and broadcast messaging using standard A2A protocol operations.

## Architecture Diagram

```
┌─────────────┐                        ┌──────────────────────┐
│   Agent A    │  A2A SendMessage       │      Broker          │
│  (sender)    │ ────────────────────→  │                      │
└─────────────┘  Authorization:         │  ┌────────────────┐  │
                  Bearer <api_key>      │  │ A2A Server     │  │
                                        │  │ (all ops)      │  │
┌─────────────┐  A2A ListTasks          │  └───────┬────────┘  │
│   Agent B    │ ←───────────────────── │          │            │
│ (recipient)  │  A2A GetTask           │          ▼            │
│              │  A2A SendMessage (ACK)  │  ┌────────────────┐  │
│              │  A2A CancelTask         │  │ Redis          │  │
└─────────────┘                         │  │ Task Store     │  │
                                        │  │ Agent Store    │  │
┌─────────────┐  GET /api/v1/agents     │  └────────────────┘  │
│   Agent C    │ ←───────────────────── │                      │
│ (discovery)  │                        └──────────────────────┘
└─────────────┘
```

## Two API Surfaces

1. **A2A Server** — Full A2A operations: SendMessage, GetTask, ListTasks, CancelTask (JSON-RPC 2.0)
2. **Registry** — Agent registration, search, listing (custom REST at `/api/v1/`)

## Component Layout

| Component | Location | Description |
|---|---|---|
| `main.py` | `registry/src/hikyaku_registry/` | ASGI app: mount A2A + FastAPI |
| `config.py` | `registry/src/hikyaku_registry/` | Settings via pydantic-settings |
| `auth.py` | `registry/src/hikyaku_registry/` | API key auth (shared by REST + A2A) |
| `redis_client.py` | `registry/src/hikyaku_registry/` | Redis connection pool |
| `models.py` | `registry/src/hikyaku_registry/` | Pydantic models (Registry API) |
| `executor.py` | `registry/src/hikyaku_registry/` | BrokerExecutor (A2A AgentExecutor) |
| `task_store.py` | `registry/src/hikyaku_registry/` | RedisTaskStore (A2A TaskStore for Redis) |
| `agent_card.py` | `registry/src/hikyaku_registry/` | Broker's own Agent Card definition |
| `registry_store.py` | `registry/src/hikyaku_registry/` | Agent CRUD on Redis |
| `api/registry.py` | `registry/src/hikyaku_registry/api/` | Registry API router |
| `cli.py` | `client/src/hikyaku_client/` | click group + subcommands |
| `api.py` | `client/src/hikyaku_client/` | Helper functions (httpx / a2a-sdk) |
| `output.py` | `client/src/hikyaku_client/` | Output formatting (tables + JSON) |

## Responsibility Assignment

The Broker acts as the central A2A Server. Individual agents are A2A clients that interact with the Broker using standard HTTP requests. No agent needs to host an HTTP server.

| Operation | Responsible | Method |
|---|---|---|
| Broker Agent Card serving | Broker | `GET /.well-known/agent-card.json` |
| Individual agent card storage | Broker (Registry) | `POST /api/v1/agents`, `GET /api/v1/agents/{id}` |
| Message sending | Sending agent (A2A client) | A2A `SendMessage` to Broker |
| Message storage & routing | Broker | Redis Task store, contextId-based routing |
| Message retrieval | Receiving agent (A2A client) | A2A `ListTasks(contextId=own_id)` to Broker |
| Message ACK | Receiving agent (A2A client) | A2A `SendMessage(taskId=existing)` multi-turn |
| Message cancellation | Sending agent (A2A client) | A2A `CancelTask` to Broker |

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

## Monorepo Structure

A uv workspace monorepo with two independent packages:

- **`registry/`** — `hikyaku-registry`: FastAPI + Redis + a2a-sdk (server)
- **`client/`** — `hikyaku-client`: click + httpx + a2a-sdk (CLI tool)

Agents only need `pip install hikyaku-client`. The Broker server is deployed separately.
