# CAFleet

A2A-native message broker and agent registry for coding agents.

> **CAFleet is a local-only tool.** It is designed to run on a single developer machine and does not perform authentication. Do not expose the broker on a shared network unless you accept that every listener can see and act within every session.

CAFleet enables ephemeral agents -- such as Claude Code sessions, CI/CD runners, and other coding agents -- to discover each other and exchange messages using the standard [A2A (Agent-to-Agent) protocol](https://github.com/google/A2A). Agents do not need to host HTTP servers; the broker handles all message routing and storage. Agents are organized into **sessions** -- a non-secret namespace created via `cafleet session create`. Agents sharing the same session can discover and message each other; agents in different sessions are invisible to one another.

## Features

- **Agent Registry** -- Register, discover, and deregister agents via REST API
- **Session Isolation** -- A `session_id` namespace defines a session boundary; cross-session agents are fully invisible to each other
- **Unicast Messaging** -- Send messages to a specific agent by ID (same-session only)
- **Broadcast Messaging** -- Send messages to all agents in the same session
- **Inbox Polling** -- Agents poll for new messages at their own pace; supports delta polling via `statusTimestampAfter`
- **Message Lifecycle** -- Acknowledge, cancel (retract), and track message status
- **Session-Based Routing** -- `X-Session-Id` (namespace) + `X-Agent-Id` (identity) headers on all requests; no authentication or bearer tokens
- **WebUI** -- Browser-based dashboard; session picker at `/ui/#/sessions`, then a Discord-style unified timeline per session (sidebar of active/deregistered agents, message timeline with broadcasts collapsed to one entry + per-recipient ACK reactions on hover, and an `@<agent>` / `@all` input)
- **Member Lifecycle** -- `cafleet member create/delete/list/capture` commands wrap tmux pane spawning + agent registration into atomic operations; the `agent_placements` table persists the agent-to-pane mapping in the registry
- **Multi-Runner Support** -- `--coding-agent claude|codex` flag on `member create` selects which coding agent to spawn; defaults to `claude` for backward compatibility. Codex runs with `--approval-mode auto-edit`
- **Director Monitoring Skill** -- `.claude/skills/cafleet-monitoring/SKILL.md` defines mandatory supervision protocol for Directors: 2-stage health check (poll inbox → capture terminal), spawn protocol, stall response, and a `/loop` prompt template
- **Unified CLI** -- Single `cafleet` command for all operations: server admin (`db init`, `session`), agent messaging (`register`, `send`, `poll`, `ack`), and member lifecycle (`member create/delete/list/capture`)
- **SQLite Storage** -- Single-file database; no daemon required. Schema managed by Alembic via `cafleet db init`

## Architecture

```
     Session X (shared session_id)        +----------------------------+
    + - - - - - - - - - - - - +           |          Broker            |
                                          |                            |
    | +-------------+         |           |  +--------------------+    |
      |  Agent A     | SendMessage        |  | A2A Server         |    |
    | |  (sender)    |---------------------> | (session-scoped)   |    |
      +-------------+ X-Agent-Id: <id>    |  +--------+-----------+    |
    |                                     |           |                |
                                          |           v                |
    | +-------------+         |           |  +--------------------+    |
      |  Agent B     | ListTasks          |  | SQLite (SQLAlchemy)|    |
    | | (recipient)  |<-------------------+  | +----------------+ |    |
      +-------------+         |           |  | | sessions         | |    |
    |                                     |  | | agents           | |    |
     - - - - - - - - - - - - -            |  | | tasks            | |    |
                                          |  | | agent_placements | |    |
                                          |  | | alembic_version  | |    |
     Session Y (different session_id)     |  | +------------------+ |    |
    + - - - - - - - - - - - - +           |  +--------------------+    |
      +-------------+                     +----------------------------+
    | |  Agent C     | (isolated) |
      | (discovery)  |
    | +-------------+            |
     - - - - - - - - - - - - -
```

Key design decisions:

- The `session_id` is the namespace boundary. Sessions are created via `cafleet session create` and are non-secret identifiers for organizing agents. All agents registered with the same session form one namespace.
- The `contextId` field is set to the recipient's agent ID on every delivery Task, enabling inbox discovery via `ListTasks(contextId=myAgentId)`.
- Task states map to message lifecycle: `INPUT_REQUIRED` (unread), `COMPLETED` (acknowledged), `CANCELED` (retracted), `FAILED` (routing error).
- FastAPI is the ASGI parent; the A2A SDK handler is mounted at the root path. FastAPI routes (`/api/v1/*`) take priority.
- Sessions are created via `cafleet session create` (direct SQLite write, no HTTP). Deleting a session is rejected while agents still reference it (FK `RESTRICT`). An empty session (no agents) remains valid indefinitely.
- The broker exposes three API surfaces: A2A Server (JSON-RPC 2.0), Registry REST API (`/api/v1/`), and WebUI (`/ui/`).
- The WebUI requires no login. A session picker at `/ui/#/sessions` lets the user select which session to view.
- **Storage layer**: All data is persisted in a single SQLite file (`~/.local/share/cafleet/registry.db` by default). Indexed fields are columns; A2A protocol payloads (`AgentCard`, `Task`) are stored as JSON blobs. No physical cleanup loop -- deregistered agents and tasks persist forever and are invisible to normal traffic via `status='active'` filters.

## Quick Start

### Prerequisites

- Python 3.12+
- SQLite (built into Python via `aiosqlite`; no daemon needed)
- [uv](https://docs.astral.sh/uv/)

### Initialize the Schema (one-time)

Before starting the server for the first time, apply the database schema:

```bash
cafleet db init
```

This command is idempotent -- running it on a database that is already at head is a no-op. The database file is created at `~/.local/share/cafleet/registry.db` by default. Override with `CAFLEET_DATABASE_URL` (e.g. `sqlite+aiosqlite:////var/lib/cafleet/registry.db`).

### Create a Session

Before starting the broker, create at least one session namespace:

```bash
cafleet session create --label "my-project"
# → prints: 550e8400-e29b-41d4-a716-446655440000
```

### Start the Broker Server

```bash
mise //cafleet:dev
```

The broker will be available at `http://127.0.0.1:8000`.

### Set Session

```bash
export CAFLEET_SESSION_ID="550e8400-e29b-41d4-a716-446655440000"
export CAFLEET_URL="http://127.0.0.1:8000"   # optional, defaults to http://127.0.0.1:8000
```

### Register an Agent

```bash
cafleet register --name "my-agent" --description "A coding assistant"
```

Save the returned `agent_id` for subsequent commands. Registration requires a valid `CAFLEET_SESSION_ID`.

### Send a Message

```bash
cafleet send --agent-id <your-agent-id> --to <recipient-agent-id> --text "Hello from my agent"
```

### Poll for Messages

```bash
cafleet poll --agent-id <your-agent-id>
```

### Acknowledge a Message

```bash
cafleet ack --agent-id <your-agent-id> --task-id <task-id>
```

## CLI Usage

The unified `cafleet` CLI handles both server administration and agent operations.

Configuration is set via environment variables:

| Variable | Required | Description |
|---|---|---|
| `CAFLEET_SESSION_ID` | Yes (for agent commands) | Session namespace for agent routing |
| `CAFLEET_URL` | No | Broker URL (default: `http://127.0.0.1:8000`) |

The `--agent-id` option is a per-subcommand option required by most agent commands. The global `--json` flag enables JSON output.

### Server Administration

| Command | Description |
|---|---|
| `cafleet db init` | Apply Alembic migrations to bring the schema to head (idempotent) |
| `cafleet session create [--label TEXT]` | Create a new session namespace; prints the session_id |
| `cafleet session list` | List all sessions with agent counts |
| `cafleet session show <id>` | Show details of a single session |
| `cafleet session delete <id>` | Delete a session (fails if agents still reference it) |

`cafleet db init` must be run once before the server starts. It handles six database states: missing file (creates it), empty schema, at head (no-op), behind head (upgrades), ahead of head (error), and legacy tables without Alembic version (error with manual instructions).

### Agent Commands

| Command | `--agent-id` | Description |
|---|---|---|
| `cafleet env` | Not required | Print current CAFLEET_URL and CAFLEET_SESSION_ID values |
| `cafleet register` | Not required | Register a new agent; returns an agent ID |
| `cafleet send` | Required | Send a unicast message to another agent in the same session |
| `cafleet broadcast` | Required | Broadcast a message to all agents in the same session |
| `cafleet poll` | Required | Poll inbox for incoming messages |
| `cafleet ack` | Required | Acknowledge receipt of a message |
| `cafleet cancel` | Required | Cancel (retract) a sent message before it is acknowledged |
| `cafleet get-task` | Required | Get details of a specific task/message |
| `cafleet agents` | Required | List agents in the session or get detail for a specific agent |
| `cafleet deregister` | Required | Deregister this agent from the broker |
| `cafleet member create` | Required | Register a member agent and spawn its tmux pane (Director only). `--coding-agent claude|codex` selects the backend (default: `claude`) |
| `cafleet member delete` | Required | Deregister a member and close its pane (Director only) |
| `cafleet member list` | Required | List members spawned by this Director |
| `cafleet member capture` | Required | Capture the last N lines of a member's pane (Director only) |

## API Overview

### Request Headers

The broker does not perform authentication. Two headers provide namespace routing and agent identification:

| Header | Purpose |
|---|---|
| `X-Session-Id: <session_id>` | Selects the session namespace (required on most endpoints, or passed in body/query) |
| `X-Agent-Id: <agent_id>` | Identifies the specific agent within the session |

No bearer tokens, no API keys. The `session_id` is a non-secret namespace identifier.

### Registry API (REST)

Base path: `/api/v1`

| Method | Endpoint | Headers / Params | Description |
|---|---|---|---|
| POST | `/api/v1/agents` | Body: `{session_id, name, description, skills}` | Register a new agent in the given session |
| GET | `/api/v1/agents` | Query: `?session_id=<uuid>` | List agents in the specified session |
| GET | `/api/v1/agents/{id}` | `X-Session-Id` | Get full A2A AgentCard JSON (404 if not in same session) |
| DELETE | `/api/v1/agents/{id}` | `X-Agent-Id` | Deregister an agent (self or Director); soft-delete, row is not physically removed |
| PATCH | `/api/v1/agents/{id}/placement` | `X-Agent-Id` | Update placement pane ID (Director only) |
| GET | `/.well-known/agent-card.json` | None | Broker's own A2A Agent Card |

Registry API errors use a consistent JSON envelope:

```json
{
  "error": {
    "code": "AGENT_NOT_FOUND",
    "message": "Agent with id '...' not found"
  }
}
```

| Error Code | HTTP Status | Description |
|---|---|---|
| `SESSION_REQUIRED` | 400 | Missing `session_id` from required header, body, or query parameter |
| `SESSION_NOT_FOUND` | 404 | `session_id` does not exist in the `sessions` table |
| `AGENT_ID_REQUIRED` | 400 | Missing `X-Agent-Id` header on an endpoint that requires it |
| `AGENT_NOT_FOUND` | 404 | Agent does not exist, is deregistered, or belongs to a different session |
| `INVALID_REQUEST` | 400 | Missing required fields or invalid values |

### A2A Operations (JSON-RPC 2.0)

| Method | Description |
|---|---|
| `SendMessage` | Send unicast (`metadata.destination=<agent-uuid>`) or broadcast (`metadata.destination=*`) |
| `ListTasks` | Poll inbox -- use `contextId=<own-agent-id>` to retrieve messages addressed to this agent; supports `statusTimestampAfter` for delta polling |
| `GetTask` | Retrieve a specific message by task ID; accessible by sender or recipient within the same session |
| `CancelTask` | Retract an unread message (sender only, `INPUT_REQUIRED` state only); returns `TaskNotCancelableError` (JSON-RPC code `-32002`) if already completed or canceled |

`ListTasks` enforces that `contextId` must equal the caller's agent ID. Providing a different `contextId` returns an `InvalidParams` error (code `-32602`) to prevent inbox snooping. `GetTask` and `CancelTask` return `TaskNotFoundError` (code `-32001`) for cross-session access, indistinguishable from "not found". Cross-session sends are rejected with JSON-RPC error `-32003` ("Session mismatch").

### Message Lifecycle

| Task State | Meaning |
|---|---|
| `TASK_STATE_INPUT_REQUIRED` | Message queued, awaiting recipient pickup (unread) |
| `TASK_STATE_COMPLETED` | Message acknowledged by recipient |
| `TASK_STATE_CANCELED` | Message retracted by sender before ACK |
| `TASK_STATE_FAILED` | Routing error (returned immediately to sender) |

### WebUI API

Base path: `/ui/api`

The WebUI API is consumed by the browser SPA. No authentication is required. Session-scoped endpoints require an `X-Session-Id` header.

| Method | Endpoint | Headers / Params | Description |
|---|---|---|---|
| GET | `/ui/api/sessions` | None | List all sessions with agent counts |
| GET | `/ui/api/agents` | `X-Session-Id` | List agents in the selected session |
| GET | `/ui/api/agents/{id}/inbox` | `X-Session-Id` | Inbox messages for an agent (newest first) |
| GET | `/ui/api/agents/{id}/sent` | `X-Session-Id` | Sent messages for an agent (newest first) |
| GET | `/ui/api/timeline` | `X-Session-Id` | Unified session timeline (up to 200 most-recent non-`broadcast_summary` tasks, newest first, each row carrying `origin_task_id` + `created_at` + `status_timestamp` for client-side broadcast grouping) |
| POST | `/ui/api/messages/send` | `X-Session-Id` | Send a message from a same-session sender. `to_agent_id=<uuid>` is unicast; `to_agent_id="*"` triggers a broadcast to every active agent in the session |

The WebUI SPA is served as static files at `/ui/`. It is built from `admin/` (Vite + React + TypeScript + Tailwind CSS) and the build output is bundled inside the package at `cafleet/src/cafleet/webui/`, which ships inside the `cafleet` wheel -- a single `pip install cafleet` produces a runnable broker that serves `/ui/` without any external file lookup.

## Tech Stack

- **Python 3.12+** with uv workspace
- **Server**: FastAPI + SQLAlchemy/aiosqlite + Alembic + a2a-sdk + Pydantic + pydantic-settings
- **CLI**: click + httpx + a2a-sdk
- **WebUI**: Vite + React 19 + TypeScript + Tailwind CSS 4

## Project Structure

```
cafleet/                    # Repository root (uv workspace)
  pyproject.toml            # Workspace root (virtual, no [project] table)
  cafleet/                  # cafleet package (server + CLI)
    src/cafleet/
      server.py             # ASGI app entry point
      cli.py                # Unified CLI (db, session, agent, member commands)
      config.py             # Settings via pydantic-settings
      broker_client.py      # httpx helpers for CLI agent operations
      db/                   # SQLAlchemy models, engine, Alembic env
      alembic/              # Alembic migration scripts (versions/)
      alembic.ini           # Alembic config (bundled into wheel)
      api/                  # Registry REST router
      tmux.py               # tmux subprocess helper (member lifecycle)
    tests/
    pyproject.toml
    mise.toml
  admin/                    # WebUI SPA (Vite + React + TypeScript + Tailwind CSS)
  docs/
    spec/                   # API and data model specifications
      registry-api.md
      a2a-operations.md
      data-model.md
      webui-api.md
      cli-options.md
  ARCHITECTURE.md           # System architecture and design decisions
```

## Development

```bash
# Clone the repository
git clone https://github.com/himkt/hikyaku.git
cd hikyaku

# Install all workspace dependencies
uv sync

# Initialize the database schema (one-time)
cafleet db init

# Run tests
mise //cafleet:test
```

### Build the WebUI

The broker serves the SPA at `/ui/`, but the build is a separate manual step so backend-only contributors are not forced to install bun. Run these two commands in order:

```bash
# 1. Build the SPA into cafleet/src/cafleet/webui/
mise //admin:build

# 2. Start the broker — it serves the freshly built SPA at http://localhost:8000/ui/
mise //cafleet:dev
```

If step 1 is skipped, the server still starts and the JSON-RPC and Registry REST surfaces remain functional; only `/ui/` 404s until you run `mise //admin:build`.

**Release maintainers**: run `mise //admin:build` before any `uv build`. The wheel only includes whatever is currently sitting in `cafleet/src/cafleet/webui/`, so a stale or missing build will produce a wheel without the SPA. After building, verify the wheel contents with `unzip -l dist/cafleet-*.whl | grep webui/index.html`.

## License

MIT
