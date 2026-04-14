# CAFleet

A2A-native message broker and agent registry for coding agents.

> **CAFleet is a local-only tool.** It is designed to run on a single developer machine and does not perform authentication. Do not expose the broker on a shared network unless you accept that every listener can see and act within every session.

CAFleet enables ephemeral agents -- such as Claude Code sessions, CI/CD runners, and other coding agents -- to discover each other and exchange messages. All CLI commands access SQLite directly through a shared `broker` module -- no HTTP server is needed for agent operations. Agents are organized into **sessions** -- a non-secret namespace created via `cafleet session create`. Agents sharing the same session can discover and message each other; agents in different sessions are invisible to one another.

## Features

- **Agent Registry** -- Register, discover, and deregister agents via CLI
- **Session Isolation** -- A `session_id` namespace defines a session boundary; cross-session agents are fully invisible to each other
- **Unicast Messaging** -- Send messages to a specific agent by ID (same-session only)
- **Broadcast Messaging** -- Send messages to all agents in the same session
- **Inbox Polling** -- Agents poll for new messages at their own pace; supports delta polling via `statusTimestampAfter`
- **Message Lifecycle** -- Acknowledge, cancel (retract), and track message status
- **Session-Based Routing** -- `session_id` (namespace) + `agent_id` (identity) parameters on all operations; no authentication or bearer tokens
- **WebUI** -- Browser-based dashboard; session picker at `/ui/#/sessions`, then a Discord-style unified timeline per session (sidebar of active/deregistered agents, message timeline with broadcasts collapsed to one entry + per-recipient ACK reactions on hover, and an `@<agent>` / `@all` input)
- **Member Lifecycle** -- `cafleet member create/delete/list/capture` commands wrap tmux pane spawning + agent registration into atomic operations; the `agent_placements` table persists the agent-to-pane mapping in the registry
- **Multi-Runner Support** -- `--coding-agent claude|codex` flag on `member create` selects which coding agent to spawn; defaults to `claude` for backward compatibility. Codex runs with `--approval-mode auto-edit`
- **tmux Push Notifications** -- After persisting a message, the broker injects a `cafleet poll` command into each recipient's tmux pane via `tmux send-keys` for near-instant delivery. Best-effort: self-sends are skipped, missing/dead panes fail silently, and the message queue remains the source of truth
- **Director Monitoring Skill** -- `.claude/skills/cafleet-monitoring/SKILL.md` defines mandatory supervision protocol for Directors: 2-stage health check (poll inbox → capture terminal), spawn protocol, stall response, and a `/loop` prompt template
- **Design Document Orchestration Skills** -- `.claude/skills/cafleet-design-doc-create/` and `.claude/skills/cafleet-design-doc-execute/` replicate the global `/design-doc-create` and `/design-doc-execute` workflows using CAFleet primitives (register + `cafleet send` + `cafleet member create`). Every inter-agent message is persisted in SQLite and visible in the admin WebUI timeline. A plugin-local `cafleet-design-doc` template skill (copy of the global `/design-doc`) keeps the plugin self-contained. Exposed as `/cafleet:cafleet-design-doc-create` and `/cafleet:cafleet-design-doc-execute` to other projects via the `cafleet` plugin
- **Unified CLI** -- Single `cafleet` command for all operations: server admin (`db init`, `session`), agent messaging (`register`, `send`, `poll`, `ack`), and member lifecycle (`member create/delete/list/capture`)
- **SQLite Storage** -- Single-file database; no daemon required. Schema managed by Alembic via `cafleet db init`

## Architecture

```
CLI (click)  ──→  broker.py (sync SQLAlchemy)  ──→  SQLite
                                                      ↑
Admin WebUI  ──→  server.py (minimal FastAPI)         |
                  +- webui_api.py  ──→  broker.py  ───+
                  +- static files (/ui/)
```

Key design decisions:

- **Direct SQLite access**: CLI commands call `broker.py` directly — no HTTP server needed for agent operations. The FastAPI server is only used for the admin WebUI.
- The `session_id` is the namespace boundary. Sessions are created via `cafleet session create` and are non-secret identifiers for organizing agents. All agents registered with the same session form one namespace.
- The `contextId` field is set to the recipient's agent ID on every delivery task, enabling inbox discovery via `broker.poll_tasks(agent_id=myAgentId)`.
- Task states map to message lifecycle: `input_required` (unread), `completed` (acknowledged), `canceled` (retracted), `failed` (routing error).
- Sessions are created via `cafleet session create`. Deleting a session is rejected while agents still reference it (FK `RESTRICT`). An empty session (no agents) remains valid indefinitely.
- The WebUI requires no login. A session picker at `/ui/#/sessions` lets the user select which session to view.
- **Storage layer**: All data is persisted in a single SQLite file (`~/.local/share/cafleet/registry.db` by default). Indexed fields are columns; task payloads are stored as JSON blobs. `PRAGMA busy_timeout=5000` handles concurrent access. No physical cleanup loop -- deregistered agents and tasks persist forever and are invisible to normal traffic via `status='active'` filters.
- **tmux push notifications**: After persisting a message, the broker looks up the recipient's `agent_placements` row and, if a tmux pane is available, injects `cafleet --session-id <session-id> poll --agent-id <recipient-agent-id>` via `tmux send-keys`. This is best-effort -- failures are silent, and the queue remains the source of truth. Unicast responses include `notification_sent`; broadcast summaries include `notifications_sent_count`.

## Quick Start

### Prerequisites

- Python 3.12+
- SQLite (built into Python; no daemon needed)
- [uv](https://docs.astral.sh/uv/)

### Initialize the Schema (one-time)

Before starting the server for the first time, apply the database schema:

```bash
cafleet db init
```

This command is idempotent -- running it on a database that is already at head is a no-op. The database file is created at `~/.local/share/cafleet/registry.db` by default. Override with `CAFLEET_DATABASE_URL` (e.g. `sqlite:////var/lib/cafleet/registry.db`).

### Create a Session

Before registering any agents, create at least one session namespace:

```bash
cafleet session create --label "my-project"
# → prints: 550e8400-e29b-41d4-a716-446655440000
```

Capture the printed UUID and pass it as `--session-id <session-id>` (a global flag, placed before the subcommand) on every subsequent command. CLI commands access SQLite directly -- no server needed. Start `mise //cafleet:dev` only if you want the admin WebUI.

> **Why a literal flag, not an env var?** Claude Code's `permissions.allow` matches Bash invocations as literal command strings. Passing `--session-id <literal-uuid>` lets a single allow-list pattern match every subcommand for that session; shell-expansion patterns (`export VAR=...` followed by `$VAR` substitution) break that matching and force per-invocation permission prompts. Substitute the literal UUIDs printed by `cafleet session create` and `cafleet register` — do not introduce shell variables to hold them.

### Register an Agent

```bash
cafleet --session-id 550e8400-e29b-41d4-a716-446655440000 register \
  --name "my-agent" --description "A coding assistant"
# → prints: 7ba91234-5678-90ab-cdef-112233445566
```

Save the returned `agent_id` for subsequent commands.

### Send a Message

```bash
cafleet --session-id <session-id> send --agent-id <your-agent-id> \
  --to <recipient-agent-id> --text "Hello from my agent"
```

### Poll for Messages

```bash
cafleet --session-id <session-id> poll --agent-id <your-agent-id>
```

### Acknowledge a Message

```bash
cafleet --session-id <session-id> ack --agent-id <your-agent-id> --task-id <task-id>
```

## CLI Usage

The unified `cafleet` CLI handles both server administration and agent operations.

Global flags (placed **before** the subcommand):

| Flag | Required | Description |
|---|---|---|
| `--session-id <uuid>` | Yes (for client + member subcommands) | Session namespace UUID for agent routing. Required for `register`, `send`, `broadcast`, `poll`, `ack`, `cancel`, `get-task`, `agents`, `deregister`, `member *`. Silently accepted (and ignored) on `db init` / `session *`. |
| `--json` | No | Emit JSON output. |

Configuration via environment variables:

| Variable | Required | Description |
|---|---|---|
| `CAFLEET_DATABASE_URL` | No | SQLite database URL. Default builds `sqlite:///<path>` from `~/.local/share/cafleet/registry.db` with `~` expanded at load time. When setting this env var yourself, use an absolute path (SQLAlchemy does not expand `~` in SQLite URLs). |

The `--agent-id` option is a per-subcommand option required by most agent commands. CLI commands access SQLite directly -- no running server is required.

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

All commands below require the global `--session-id <uuid>` flag (placed before the subcommand). The `--agent-id` column indicates whether the per-subcommand `--agent-id <uuid>` flag is also required.

| Command | `--agent-id` | Description |
|---|---|---|
| `cafleet --session-id <id> register` | Not required | Register a new agent; returns an agent ID |
| `cafleet --session-id <id> send --agent-id <id>` | Required | Send a unicast message to another agent in the same session |
| `cafleet --session-id <id> broadcast --agent-id <id>` | Required | Broadcast a message to all agents in the same session |
| `cafleet --session-id <id> poll --agent-id <id>` | Required | Poll inbox for incoming messages |
| `cafleet --session-id <id> ack --agent-id <id>` | Required | Acknowledge receipt of a message |
| `cafleet --session-id <id> cancel --agent-id <id>` | Required | Cancel (retract) a sent message before it is acknowledged |
| `cafleet --session-id <id> get-task --agent-id <id>` | Required | Get details of a specific task/message |
| `cafleet --session-id <id> agents --agent-id <id>` | Required | List agents in the session or get detail for a specific agent |
| `cafleet --session-id <id> deregister --agent-id <id>` | Required | Deregister this agent from the broker |
| `cafleet --session-id <id> member create --agent-id <id>` | Required | Register a member agent and spawn its tmux pane (Director only). `--coding-agent claude\|codex` selects the backend (default: `claude`) |
| `cafleet --session-id <id> member delete --agent-id <id>` | Required | Deregister a member and close its pane (Director only) |
| `cafleet --session-id <id> member list --agent-id <id>` | Required | List members spawned by this Director |
| `cafleet --session-id <id> member capture --agent-id <id>` | Required | Capture the last N lines of a member's pane (Director only) |

## API Overview

### WebUI API

The admin WebUI is available when the server is running (`mise //cafleet:dev`). CLI commands do not use the server.

### Message Lifecycle

| Task State | Meaning |
|---|---|
| `input_required` | Message queued, awaiting recipient pickup (unread) |
| `completed` | Message acknowledged by recipient |
| `canceled` | Message retracted by sender before ACK |
| `failed` | Routing error (returned immediately to sender) |

## Tech Stack

- **Python 3.12+** with uv workspace
- **Server**: FastAPI + SQLAlchemy + Alembic + Pydantic + pydantic-settings (WebUI only)
- **CLI**: click (direct SQLite via `broker` module)
- **WebUI**: Vite + React 19 + TypeScript + Tailwind CSS 4

## Project Structure

```
cafleet/                    # Repository root (uv workspace)
  pyproject.toml            # Workspace root (virtual, no [project] table)
  cafleet/                  # cafleet package (server + CLI)
    src/cafleet/
      broker.py             # Single data access layer (sync SQLAlchemy)
      server.py             # Minimal FastAPI app (WebUI only)
      cli.py                # Unified CLI (db, session, agent, member commands)
      config.py             # Settings via pydantic-settings
      db/                   # SQLAlchemy models, engine, Alembic env
      alembic/              # Alembic migration scripts (versions/)
      alembic.ini           # Alembic config (bundled into wheel)
      tmux.py               # tmux subprocess helper (member lifecycle)
    tests/
    pyproject.toml
    mise.toml
  admin/                    # WebUI SPA (Vite + React + TypeScript + Tailwind CSS)
  docs/
    spec/                   # API and data model specifications
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

If step 1 is skipped, the server still starts; only `/ui/` 404s until you run `mise //admin:build`. Note: the server is only needed for the WebUI — CLI commands work without it.

**Release maintainers**: run `mise //admin:build` before any `uv build`. The wheel only includes whatever is currently sitting in `cafleet/src/cafleet/webui/`, so a stale or missing build will produce a wheel without the SPA. After building, verify the wheel contents with `unzip -l dist/cafleet-*.whl | grep webui/index.html`.

## License

MIT
