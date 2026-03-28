# Hikyaku

A2A-native message broker and agent registry for coding agents.

Hikyaku enables ephemeral agents -- such as Claude Code sessions, CI/CD runners, and other coding agents -- to discover each other and exchange messages using the standard [A2A (Agent-to-Agent) protocol](https://github.com/google/A2A). Agents do not need to host HTTP servers; the broker handles all message routing and storage. Agents are organized into **tenants** via shared API keys -- agents sharing the same key form a tenant and can discover and message each other; agents in different tenants are invisible to one another.

## Features

- **Agent Registry** -- Register, discover, and deregister agents via REST API
- **Tenant Isolation** -- Shared API key defines a tenant boundary; cross-tenant agents are fully invisible to each other
- **Unicast Messaging** -- Send messages to a specific agent by ID (same-tenant only)
- **Broadcast Messaging** -- Send messages to all agents in the same tenant
- **Inbox Polling** -- Agents poll for new messages at their own pace
- **Message Lifecycle** -- Acknowledge, cancel (retract), and track message status
- **Two-Header Auth** -- API key (tenant) + Agent-Id (identity) required on all authenticated requests
- **CLI Tool** -- Full-featured command-line client for all broker operations

## Architecture

```
     Tenant X (shared API key)              +---------------------------+
    +- - - - - - - - - - - - -+             |        Broker             |
                                            |                           |
    | +-----------+            |            |  +-----------------+      |
      |  Agent A  | SendMessage             |  | A2A Server      |      |
    | | (sender)  |--------------------------->| (tenant-scoped) |      |
      +-----------+ Authorization:          |  +-------+---------+      |
    |               Bearer <api_key>        |          |                |
                    X-Agent-Id: <id>        |          v                |
    | +-----------+            |            |  +-----------------+      |
      |  Agent B  | ListTasks               |  | Redis           |      |
    | |(recipient)|<------------------------   | Agent Store     |      |
      +-----------+            |            |  | Task Store      |      |
    +- - - - - - - - - - - - -+            |  | Tenant Sets     |      |
                                            |  +-----------------+      |
     Tenant Y (different API key)           +---------------------------+
    +- - - - - - - - - - - - -+
      +-----------+
    | |  Agent C  | (isolated) |
      | (no access|
    | +-----------+            |
    +- - - - - - - - - - - - -+
```

Key design decisions:

- The API key is the tenant boundary. `SHA-256(api_key)` is stored as `tenant_id`. All agents with the same key form one tenant.
- The `contextId` field is set to the recipient's agent ID on every delivery Task, enabling inbox discovery via `ListTasks(contextId=myAgentId)`.
- Task states map to message lifecycle: `INPUT_REQUIRED` (unread), `COMPLETED` (acknowledged), `CANCELED` (retracted), `FAILED` (routing error).
- FastAPI is the ASGI parent; the A2A SDK handler is mounted at the root path. FastAPI routes (`/api/v1/*`) take priority.

## Quick Start

### Prerequisites

- Python 3.12+
- Redis (running locally or via Docker)
- [uv](https://docs.astral.sh/uv/)

### Start the Broker Server

```bash
cd registry
uv run uvicorn hikyaku_registry.main:app
```

The broker will be available at `http://localhost:8000`.

### Install the CLI Client

```bash
cd client
uv tool install .
```

### Register an Agent (new tenant)

```bash
hikyaku register --name "my-agent" --description "A coding assistant"
```

This creates a new tenant and returns an agent ID and API key. Save both -- the API key is shown only once.

### Register a Second Agent (join existing tenant)

```bash
hikyaku --api-key "hky_..." register --name "my-second-agent" --description "Another agent"
```

Providing `--api-key` (or `HIKYAKU_API_KEY`) at registration joins the existing tenant.

### Set Credentials

```bash
export HIKYAKU_API_KEY="hky_..."
export HIKYAKU_AGENT_ID="<your-agent-id>"
```

### Send a Message

```bash
hikyaku send --to <recipient-agent-id> --text "Hello from my agent"
```

### Poll for Messages

```bash
hikyaku poll
```

### Acknowledge a Message

```bash
hikyaku ack --task-id <task-id>
```

## CLI Usage

All commands accept `--url` (default: `http://localhost:8000`), `--api-key`, `--agent-id`, and `--json` flags. Credentials can also be set via environment variables (`HIKYAKU_URL`, `HIKYAKU_API_KEY`, `HIKYAKU_AGENT_ID`).

| Command | Description |
|---|---|
| `hikyaku register` | Register a new agent; omit `--api-key` to create a new tenant, include it to join an existing one |
| `hikyaku send` | Send a unicast message to another agent in the same tenant |
| `hikyaku broadcast` | Broadcast a message to all agents in the same tenant |
| `hikyaku poll` | Poll inbox for incoming messages |
| `hikyaku ack` | Acknowledge receipt of a message |
| `hikyaku cancel` | Cancel (retract) a sent message before it is acknowledged |
| `hikyaku get-task` | Get details of a specific task/message |
| `hikyaku agents` | List agents in the tenant or get detail for a specific agent |
| `hikyaku deregister` | Deregister this agent from the broker |

## API Overview

### Authentication

All requests (except initial registration and the Agent Card endpoint) require two headers:

| Header | Purpose |
|---|---|
| `Authorization: Bearer <api_key>` | Authenticates the tenant (`SHA-256(api_key)` = `tenant_id`) |
| `X-Agent-Id: <agent_id>` | Identifies the specific agent within the tenant |

### Registry API (REST)

Base path: `/api/v1`

| Method | Endpoint | Auth | Description |
|---|---|---|---|
| POST | `/api/v1/agents` | Optional | Register a new agent; no auth creates a new tenant, auth header joins existing tenant |
| GET | `/api/v1/agents` | Bearer + Agent-Id | List agents in the caller's tenant |
| GET | `/api/v1/agents/{id}` | Bearer + Agent-Id | Get agent detail (404 if not in same tenant) |
| DELETE | `/api/v1/agents/{id}` | Bearer + Agent-Id | Deregister an agent (self only) |

### A2A Operations (JSON-RPC 2.0)

| Method | Description |
|---|---|
| `SendMessage` | Send unicast (`metadata.destination=<agent-uuid>`) or broadcast (`metadata.destination=*`) |
| `ListTasks` | Poll inbox -- use `contextId=<own-agent-id>` to retrieve messages addressed to this agent |
| `GetTask` | Retrieve a specific message by task ID |
| `CancelTask` | Retract an unread message (sender only, `INPUT_REQUIRED` state only) |

## Tech Stack

- **Python 3.12+** with uv workspace
- **Server**: FastAPI + Redis + a2a-sdk + Pydantic + pydantic-settings
- **CLI**: click + httpx + a2a-sdk

## Project Structure

```
hikyaku/
  pyproject.toml          # Workspace root (uv workspace)
  registry/               # hikyaku-registry server package
    src/hikyaku_registry/
    tests/
    pyproject.toml
  client/                 # hikyaku-client CLI package
    src/hikyaku_client/
    tests/
    pyproject.toml
```

## Development

```bash
# Clone the repository
git clone https://github.com/himkt/hikyaku.git
cd hikyaku

# Install all workspace dependencies
uv sync

# Run registry tests
cd registry
uv run pytest tests/ -v

# Run client tests
cd client
uv run pytest tests/ -v
```

## License

MIT
