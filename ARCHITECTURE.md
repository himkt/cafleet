# CAFleet — Architecture

A message broker and agent registry for coding agents. All CLI commands and the admin WebUI access SQLite directly through a shared `broker` module (`cafleet/broker.py`) — no HTTP server is needed for agent operations. Agents are organized into **sessions** — a non-secret namespace created via `cafleet session create`. Agents sharing the same session can discover and message each other; agents in different sessions are invisible to one another.

## Architecture Diagram

```
CLI (click)  ──→  broker.py (sync SQLAlchemy)  ──→  SQLite
                                                      ↑
Admin WebUI  ──→  server.py (minimal FastAPI)         │
                  └─ webui_api.py  ──→  broker.py  ───┘
                  └─ static files (/ui/)

┌─────────────────────────────────────────────────────┐
│  SQLite (single file)                               │
│  ┌────────────────┐                                 │
│  │ sessions         │                                │
│  │ agents           │                                │
│  │ tasks            │                                │
│  │ agent_placements │                                │
│  │ alembic_version  │                                │
│  └──────────────────┘                                │
└─────────────────────────────────────────────────────┘
```

`broker.py` is the single data access layer. Both CLI and Admin WebUI call it. No async stores, no HTTP client, no A2A protocol layer.

## Session Isolation

The `session_id` serves as the namespace boundary. Sessions are created via `cafleet session create`. All agents registered with the same `session_id` form one namespace. The broker does not perform authentication — it performs namespace routing only.

No bearer tokens, no API keys, no Auth0. The `session_id` is a non-secret namespace identifier. Sessions are namespaces for tidiness, not security boundaries.

**Registration** requires a valid `session_id`. Sessions are created via `cafleet session create` before agents can register.

**Isolation rules**: Every operation that reads or writes agent/task data enforces session boundaries. Cross-session requests always produce "not found" errors indistinguishable from the resource not existing.

## Component Layout

| Component | Location | Description |
|---|---|---|
| `broker.py` | `cafleet/src/cafleet/` | Single data access layer — sync SQLAlchemy operations for CLI + WebUI |
| `server.py` | `cafleet/src/cafleet/` | Minimal FastAPI app: `webui_router` + static file serving |
| `config.py` | `cafleet/src/cafleet/` | Settings via pydantic-settings; owns `~` expansion of `database_url` |
| `cli.py` | `cafleet/src/cafleet/` | Unified `cafleet` console script: click group with `db` (Alembic schema management), `session` (session namespace CRUD), and all agent/messaging commands (`register`, `send`, `poll`, `ack`, etc.) plus `member` subgroup. Calls `broker` directly. |
| `db/__init__.py` | `cafleet/src/cafleet/db/` | DB sub-package marker |
| `db/models.py` | `cafleet/src/cafleet/db/` | SQLAlchemy declarative models: `Base`, `Session`, `Agent`, `Task`; column indexes |
| `db/engine.py` | `cafleet/src/cafleet/db/` | `get_sync_engine()`, `get_sync_sessionmaker()`, SQLite PRAGMA listener |
| `alembic.ini` | `cafleet/src/cafleet/` | Alembic config (bundled into the wheel) |
| `alembic/env.py` | `cafleet/src/cafleet/alembic/` | Alembic environment; swaps URL to sync `pysqlite` driver |
| `alembic/versions/` | `cafleet/src/cafleet/alembic/versions/` | Migration scripts (`0001_initial_schema.py`, …) |
| `webui_api.py` | `cafleet/src/cafleet/` | WebUI API router (`/ui/api/*`) — calls `broker` for all data access |
| `output.py` | `cafleet/src/cafleet/` | CLI output formatting (tables + JSON) |
| `coding_agent.py` | `cafleet/src/cafleet/` | `CodingAgentConfig` dataclass, `CLAUDE`/`CODEX` built-in configs, `CODING_AGENTS` registry, `get_coding_agent()` helper |
| `tmux.py` | `cafleet/src/cafleet/` | tmux subprocess helper: `ensure_tmux_available`, `director_context`, `split_window`, `select_layout`, `send_exit`, `capture_pane` |
| `admin/` | Project root | WebUI SPA (Vite + React + TypeScript + Tailwind CSS) |

## Operation Mapping

All operations go through `broker.py` (sync SQLAlchemy). No HTTP server is involved for CLI commands.

| CLI Command | `broker` Function |
|---|---|
| `register` | `broker.register_agent()` → INSERT agents [+ agent_placements] |
| `send` | `broker.send_message()` → validate dest + INSERT tasks |
| `broadcast` | `broker.broadcast_message()` → list agents + INSERT tasks per recipient + summary |
| `poll` | `broker.poll_tasks()` → SELECT tasks WHERE context_id |
| `ack` | `broker.ack_task()` → verify recipient + UPDATE status → completed |
| `cancel` | `broker.cancel_task()` → verify sender + UPDATE status → canceled |
| `get-task` | `broker.get_task()` → SELECT task + verify session |
| `agents` (list) | `broker.list_agents()` → SELECT agents WHERE active |
| `agents --id` | `broker.get_agent()` → SELECT agent + placement |
| `deregister` | `broker.deregister_agent()` → UPDATE status + DELETE placement |
| `db init` | Alembic `upgrade head` |

## Storage Layer

### Backend

Everything is persisted in a single SQLite database accessed through SQLAlchemy 2.x with the sync `pysqlite` driver. Schema changes are managed by Alembic, bundled inside the `cafleet` wheel and applied via `cafleet db init`. There is no separate database daemon to operate, monitor, or back up — the database is a single file.

The default database path is `~/.local/share/cafleet/registry.db` (XDG state directory), expanded once at config load time. Override with the `CAFLEET_DATABASE_URL` environment variable, e.g. `sqlite:////var/lib/cafleet/registry.db`.

**Concurrency**: `PRAGMA busy_timeout=5000` is set on every connection. SQLite retries internally for up to 5 seconds before returning `SQLITE_BUSY`. Expected contention is low — CLI operations are short transactions (single INSERT or UPDATE), and multiple agents polling concurrently is read-only.

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

`PRAGMA foreign_keys=ON` and `PRAGMA busy_timeout=5000` are issued on every new connection via a SQLAlchemy engine `connect` event listener so the FK declarations in `models.py` are enforced and concurrent access is handled gracefully. A regression test verifies the PRAGMAs are active on a fresh connection.

### Session ownership

`broker.py` uses module-level `get_sync_sessionmaker()` from `db/engine.py`. Each function opens a fresh session, executes within a transaction, and returns dicts. No async, no store classes, no dependency injection — just plain function calls.

### Schema management

Alembic revisions are committed to the repository: `0001_initial_schema.py`, `0002_add_origin_task_id.py`, and `0003_add_agent_placements.py`. Operators run `cafleet db init` once before starting the server. The command is idempotent across six DB states:

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

The `cafleet member` CLI subgroup wraps the two-step "register an agent + spawn a tmux pane" recipe behind a single command and persists the agent-to-pane mapping in the registry SQLite store via the `agent_placements` table.

**Terminology**: A "member" is an agent spawned by a Director via `cafleet member create`. It has an associated placement row linking it to a specific tmux pane, window, and session. The Director itself is NOT a member — it registers with plain `cafleet register`.

**Atomic create flow** (`cafleet member create`):

1. Register the member agent with a pending placement (`tmux_pane_id = NULL`, `coding_agent` field) via `broker.register_agent(placement=...)`.
2. Spawn the coding agent (Claude or Codex, selected via `--coding-agent`) in the Director's own tmux window via `tmux split-window -t <window_id>`, capturing the new pane ID.
3. Patch the placement row with the real pane ID via `broker.update_placement_pane_id()`.
4. Rebalance the window layout via `tmux select-layout main-vertical`.

If step 2 fails, the registered agent is rolled back via `broker.deregister_agent()`. If step 3 fails, the pane is `/exit`'d and the agent rolled back.

**Delete ordering** (`cafleet member delete`): Deregister the agent first, THEN `/exit` the pane. This preserves the pane for retry if deregister fails.

**Multi-runner support**: The `--coding-agent` option on `member create` selects which coding agent binary to spawn (`claude` or `codex`, default: `claude`). Agent-specific configuration (binary name, extra args, default prompt template) is encapsulated in `CodingAgentConfig` dataclasses in `cafleet/src/cafleet/coding_agent.py`. The `agent_placements` table tracks which coding agent was spawned via a `coding_agent` column (default: `"claude"`). The `tmux.split_window()` function accepts a generic `command: list[str]` instead of a hardcoded Claude prompt, making it agent-agnostic.

**Commands**: `member create`, `member delete`, `member list`, `member capture`. All require `--agent-id` (the Director's ID). The tmux helper module (`cafleet/src/cafleet/tmux.py`) isolates all subprocess interaction with tmux.

**Supervision skill**: The Director's monitoring obligations are defined in `.claude/skills/cafleet-monitoring/SKILL.md`. This skill must be loaded (`Skill(cafleet-monitoring)`) before spawning any members. It provides a 2-stage health check protocol (message poll then terminal capture) and a ready-to-use `/loop` prompt template.

## tmux Push Notifications

CAFleet uses a pull-based delivery model by default: recipients discover messages via `cafleet poll`. To reduce latency, the broker can also push a poll trigger into a recipient's tmux pane immediately after persisting a message.

After `broker` saves a delivery task, it looks up the recipient's `agent_placements` row. If the recipient has a non-null `tmux_pane_id` and is not the sender, the broker runs:

```
tmux send-keys -t <tmux_pane_id> "cafleet poll --agent-id <recipient_agent_id>" Enter
```

The injected text lands in the coding agent's input prompt. If the agent is idle, it interprets the command immediately. If the agent is busy, tmux buffers the keystrokes until the agent returns to its prompt. Since `cafleet poll` is idempotent, duplicate or late-arriving triggers are harmless.

**Design principles**:

- **Best-effort**: The message queue remains the sole source of truth. Push notification is an optimization — if it fails, the message is still available for normal polling.
- **Self-send skip**: When sender == recipient, the notification is suppressed.
- **Silent failure**: Missing placements, null `tmux_pane_id`, dead panes, and absent `tmux` binary all result in `False` — no exceptions propagate to the caller.
- **No `TMUX` env var required**: `tmux send-keys -t <pane>` works from any process on the same host as long as the tmux server socket is accessible.

**Response annotations**: Unicast responses include a top-level `notification_sent` boolean. Broadcast summary tasks include `notificationsSentCount` in their metadata, reflecting how many recipient panes were successfully triggered; the top-level response exposes this value as `notifications_sent_count`.

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

### CLI Option Sources

Each CLI parameter has exactly one input source:

| Parameter | Source |
|---|---|
| Session ID | `CAFLEET_SESSION_ID` env var |
| Database URL | `CAFLEET_DATABASE_URL` env var (optional; default: `sqlite:///~/.local/share/cafleet/registry.db`) |
| Agent ID | `--agent-id` subcommand option |
| JSON output | `--json` global flag |

Session ID uses an environment variable for convenience in tmux multi-pane workflows. Agent ID is a CLI argument because it's an operational parameter that changes per invocation. No broker URL is needed — CLI commands access SQLite directly.

## WebUI

A browser-based dashboard served as a SPA at `/ui/`. No login is required. The first-load lands on a session picker at `/ui/#/sessions`; selecting a session navigates to a Discord-style unified timeline for that session — a sidebar listing every active (top) and deregistered (muted) agent in the session, a center timeline rendering unicast and broadcast messages ordered newest-at-bottom with auto-scroll, reactions-as-ACKs chips that reveal per-recipient ACK time on CSS hover, and a bottom input that parses `@<agent> text` for unicast and `@all text` for broadcast. The admin is NOT a CAFleet agent; a header dropdown (sender selector) picks which real in-session active agent is used as `from_agent_id` on every send, persisted per-session in `localStorage` under `cafleet.sender.<session_id>`.

- **Frontend**: `admin/` — Vite + React 19 + TypeScript + Tailwind CSS 4
- **Backend API**: `/ui/api/*` endpoints in `webui_api.py` — all endpoints call `broker` for data access (sync `def` handlers, FastAPI runs them in a thread pool)
- **Server**: `server.py` is a minimal FastAPI app — just `webui_router` + static files. No A2A handler, no JSON-RPC, no executor. Only needed for the WebUI; CLI commands work without it.
- **Session scoping**: Session-scoped endpoints require `X-Session-Id` header. No authentication.
- **Static serving**: `StaticFiles` mount at `/ui` serves the SPA bundled inside the package at `cafleet/src/cafleet/webui/` (production build). `mise //admin:build` must be run before `mise //cafleet:dev` for `/ui/` to be populated; without it the server starts cleanly and `/ui/` simply 404s.

## Package Structure

A uv workspace with a single Python package and a frontend app:

- **`cafleet/`** — `cafleet`: FastAPI + SQLAlchemy + Alembic + click (server + CLI). Ships the unified `cafleet` console script for all operations: `db init`, `session` management, agent registration, messaging, and member lifecycle. CLI commands access SQLite directly via `broker.py`; the FastAPI server is only needed for the admin WebUI.
- **`admin/`** — WebUI SPA: Vite + React + TypeScript + Tailwind CSS

A single `pip install cafleet` gives users both the broker server and the agent CLI.
