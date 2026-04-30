# CAFleet — Architecture

A message broker and agent registry for coding agents. All CLI commands and the admin WebUI access SQLite directly through a shared `broker` module (`cafleet/broker.py`) — no HTTP server is needed for agent operations. Agents are organized into **sessions** identified by a non-secret `session_id` created via `cafleet session create`. Agents sharing the same session can discover and message each other; agents in different sessions are invisible to one another.

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

The `session_id` serves as the session boundary. Sessions are created via `cafleet session create`. All agents registered with the same `session_id` form one session. The broker does not perform authentication — it performs session routing only.

The `session_id` is a non-secret session identifier. Sessions are partitions for tidiness, not security boundaries.

**Registration** requires a valid, non-soft-deleted `session_id`. Sessions are created via `cafleet session create` before any members can be spawned.

**Isolation rules**: Every operation that reads or writes agent/task data enforces session boundaries. Cross-session requests always produce "not found" errors indistinguishable from the resource not existing.

**Session bootstrap (transactional)**: `cafleet session create` must be run inside a tmux session. It reads the caller's tmux context (`session`, `window_id`, `pane_id`) via `tmux.director_context()` **before** opening any DB work and then executes a single transaction with five ordered operations: (1) INSERT `sessions` with `deleted_at=NULL` and `director_agent_id=NULL`; (2) INSERT `agents` for the hardcoded root Director (`name="Director"`, `description="Root Director for this session"`, `status="active"`); (3) INSERT `agent_placements` with `director_agent_id=NULL` (the root has no parent Director) and `coding_agent="unknown"` (auto-detection is deferred); (4) UPDATE `sessions.director_agent_id` to point at the newly inserted agent; (5) INSERT the built-in Administrator agent for the session. Any failure in the transaction rolls the whole thing back — no partial session/agent/placement rows can persist. Outside tmux the CLI fails with `Error: cafleet session create must be run inside a tmux session` and exit code 1 before touching the DB.

The post-bootstrap invariant is that every non-deleted `sessions` row has a non-NULL `director_agent_id`. The column itself is DB-nullable because the 5-step insert order requires `sessions` to exist before the agent row it will eventually reference, so the NOT NULL constraint is enforced by the broker code path — not by the schema.

**Session soft-delete**: `cafleet session delete <id>` runs a single transaction: (1) `UPDATE sessions SET deleted_at=now WHERE session_id=X AND deleted_at IS NULL`, (2) `UPDATE agents SET status='deregistered', deregistered_at=now WHERE session_id=X AND status='active'` (this sweeps the root Director and every member in one statement), (3) `DELETE FROM agent_placements WHERE agent_id IN (SELECT agent_id FROM agents WHERE session_id=X)`. Tasks are never touched — the message history remains queryable. The command is idempotent: re-running against an already-deleted session prints `Deleted session X. Deregistered 0 agents.` and exits 0 because step 1's `WHERE deleted_at IS NULL` clause short-circuits the cascade. It is **not** transactional with tmux: surviving member panes are orphaned intentionally. Directors that want a clean shutdown run `cafleet member delete` per member first (which does send `/exit`), then `session delete`.

**Soft-delete visibility**: `broker.get_session` exposes the `deleted_at` field but otherwise returns the row regardless of its value; `broker.list_sessions` filters `WHERE deleted_at IS NULL` so the CLI's `session list` hides deleted rows. `broker.register_agent` inspects `get_session(...)["deleted_at"]` and rejects a soft-deleted session with `Error: session X is deleted` (distinct from the `Session 'X' not found.` path for an unknown ID).

**Root Director protection**: `broker.deregister_agent` refuses to deregister the root Director (detected by `sessions.director_agent_id == agent_id`) and exits 1 with `Error: cannot deregister the root Director; use 'cafleet session delete' instead.`. This keeps `sessions.director_agent_id` from pointing at a deregistered, placement-less agent, which would otherwise silently break Member → Director tmux push notifications.

**Built-in Administrator agent**: `cafleet session create` inserts a single `Administrator` agent into the new session in the same transaction as the session row. The Administrator is an ordinary `agents` row distinguished only by `agent_card_json.cafleet.kind == "builtin-administrator"` — no schema change, no separate table. Every session has exactly one Administrator, and Alembic revision `0006_seed_administrator_agent.py` backfills one into each pre-existing session on `cafleet db init` (idempotent via a `json_extract` probe). The Admin WebUI Send control always submits messages with `from_agent_id = administrator.agent_id`, so there is no sender dropdown. Protection lives entirely in `broker.py`: a single `AdministratorProtectedError` class is raised from `broker.deregister_agent` (preventing deregister) and from `broker.register_agent` (preventing `placement.director_agent_id` from pointing at an Administrator — the Administrator never receives a tmux pane). `broker.broadcast_message` filters Administrators out of the recipient set, so they are write-only identities. The CLI handles `AdministratorProtectedError` by printing `Error: ...` to stderr and exiting with status 1; any future WebUI deregister endpoint maps it to HTTP 409.

## Component Layout

| Component | Location | Description |
|---|---|---|
| `broker.py` | `cafleet/src/cafleet/` | Single data access layer — sync SQLAlchemy operations for CLI + WebUI |
| `server.py` | `cafleet/src/cafleet/` | Minimal FastAPI app: `webui_router` + static file serving |
| `config.py` | `cafleet/src/cafleet/` | Settings via pydantic-settings; owns `~` expansion of `database_url` |
| `cli.py` | `cafleet/src/cafleet/` | Unified `cafleet` console script: click group with `db` (Alembic schema management), `session` (session CRUD), `agent` (registry: `register` / `deregister` / `list` / `show`), `message` (broker: `send` / `broadcast` / `poll` / `ack` / `cancel` / `show`), and `member` (lifecycle: `create` / `delete` / `list` / `capture` / `send-input` / `exec`) subgroups. Also exposes `cafleet server [--host <addr>] [--port <int>]` and `cafleet doctor` as top-level meta-command exceptions. Calls `broker` directly. |
| `db/__init__.py` | `cafleet/src/cafleet/db/` | DB sub-package marker |
| `db/models.py` | `cafleet/src/cafleet/db/` | SQLAlchemy declarative models: `Base`, `Session`, `Agent`, `Task`; column indexes |
| `db/engine.py` | `cafleet/src/cafleet/db/` | `get_sync_engine()`, `get_sync_sessionmaker()`, SQLite PRAGMA listener |
| `alembic.ini` | `cafleet/src/cafleet/` | Alembic config (bundled into the wheel) |
| `alembic/env.py` | `cafleet/src/cafleet/alembic/` | Alembic environment; swaps URL to sync `pysqlite` driver |
| `alembic/versions/` | `cafleet/src/cafleet/alembic/versions/` | Migration scripts (`0001_initial_schema.py`, …) |
| `webui_api.py` | `cafleet/src/cafleet/` | WebUI API router (`/ui/api/*`) — calls `broker` for all data access |
| `output.py` | `cafleet/src/cafleet/` | CLI output formatting (tables + JSON) |
| `coding_agent.py` | `cafleet/src/cafleet/` | claude-only spawn config — `CodingAgentConfig` dataclass plus a single `CLAUDE` instance carrying binary, extra args, default prompt template, display-name flag, and `permission_args=("--permission-mode", "dontAsk")` injected into every spawn. |
| `tmux.py` | `cafleet/src/cafleet/` | tmux subprocess helper: `ensure_tmux_available`, `director_context`, `split_window`, `select_layout`, `send_exit`, `capture_pane`, `send_choice_key`, `send_freetext_and_submit`, `send_bash_command` |
| `admin/` | Project root | WebUI SPA (Vite + React + TypeScript + Tailwind CSS) |

## Operation Mapping

All operations go through `broker.py` (sync SQLAlchemy). No HTTP server is involved for CLI commands.

| CLI Command | `broker` Function |
|---|---|
| `agent register` | `broker.register_agent()` → INSERT agents [+ agent_placements] |
| `message send` | `broker.send_message()` → validate dest + INSERT tasks |
| `message broadcast` | `broker.broadcast_message()` → list agents + INSERT tasks per recipient + summary |
| `message poll` | `broker.poll_tasks()` → SELECT tasks WHERE context_id |
| `message ack` | `broker.ack_task()` → verify recipient + UPDATE status → completed |
| `message cancel` | `broker.cancel_task()` → verify sender + UPDATE status → canceled |
| `message show --task-id <x>` | `broker.get_task()` → SELECT task + verify session |
| `agent list` | `broker.list_agents()` → SELECT agents WHERE active |
| `agent show --id <x>` | `broker.get_agent()` → SELECT agent + placement |
| `agent deregister` | `broker.deregister_agent()` → UPDATE status + DELETE placement |
| `member send-input` | `broker.get_agent()` → authorization check + `tmux.send_choice_key` / `tmux.send_freetext_and_submit`. Director-side workflow is AskUserQuestion-delegated (capture → Director-side `AskUserQuestion` → direct Bash invocation of the resolved command). See [`skills/cafleet/SKILL.md`](skills/cafleet/SKILL.md) "Answer a member's AskUserQuestion prompt" for the canonical workflow. |
| `member exec` | `broker.get_agent()` → authorization check + `tmux.send_bash_command`. Director-only shell-dispatch primitive that keystrokes `! <cmd>` + Enter into the member's pane via Claude Code's `!` shortcut. See [`skills/cafleet/SKILL.md`](skills/cafleet/SKILL.md) "Routing Bash via the Director". |
| `member ping` | `broker.get_agent()` → authorization check + `tmux.send_poll_trigger`. Director-only manual inbox-poll nudge (counterpart to `broker._try_notify_recipient` auto-fire). |
| `db init` | Alembic `upgrade head` |

## Storage Layer

### Backend

Everything is persisted in a single SQLite database accessed through SQLAlchemy 2.x with the sync `pysqlite` driver. Schema changes are managed by Alembic, bundled inside the `cafleet` wheel and applied via `cafleet db init`. There is no separate database daemon to operate, monitor, or back up — the database is a single file.

The default database path is `~/.local/share/cafleet/registry.db` (XDG state directory), expanded once at config load time. Override with the `CAFLEET_DATABASE_URL` environment variable, e.g. `sqlite:////var/lib/cafleet/registry.db`.

**Concurrency**: `PRAGMA busy_timeout=5000` is set on every connection. SQLite retries internally for up to 5 seconds before returning `SQLITE_BUSY`. Expected contention is low — CLI operations are short transactions (single INSERT or UPDATE), and multiple agents polling concurrently is read-only.

### Relational + document hybrid model

Indexed fields are columns; A2A-inspired payloads (`AgentCard`-shaped, `Task`-shaped) are stored verbatim as JSON `TEXT` blobs and never queried by content. This keeps hot lookups index-served while preserving the canonical internal shape for these payloads.

| Table | Indexed columns | JSON blob |
|---|---|---|
| `sessions` | `session_id` (PK) | — |
| `agents` | `agent_id` (PK), `session_id` (FK → `sessions`), `status` | `agent_card_json` |
| `tasks` | `task_id` (PK), `context_id` (FK → `agents`), `from_agent_id`, `to_agent_id`, `status_state`, `status_timestamp` | `task_json` |
| `agent_placements` | `agent_id` (PK, FK → `agents` CASCADE), `director_agent_id` (nullable, FK → `agents` RESTRICT), `tmux_session`, `tmux_window_id`, `tmux_pane_id` (nullable) | — |

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

**Terminology**: A "member" is an agent spawned by a Director via `cafleet member create`. It has an associated placement row linking it to a specific tmux pane, window, and session. The Director itself is NOT a member — it registers with plain `cafleet agent register`.

**Atomic create flow** (`cafleet member create`):

1. Register the member agent with a pending placement (`tmux_pane_id = NULL`, `coding_agent` field) via `broker.register_agent(placement=...)`.
2. Spawn the claude member pane in the Director's own tmux window via `tmux split-window -t <window_id>`, capturing the new pane ID.
3. Patch the placement row with the real pane ID via `broker.update_placement_pane_id()`.
4. Rebalance the window layout via `tmux select-layout main-vertical`.

If step 2 fails, the registered agent is rolled back via `broker.deregister_agent()`. If step 3 fails, the pane is `/exit`'d and the agent rolled back.

**Delete ordering** (default path): send `/exit`, poll `list-panes` until the pane disappears (15 s timeout), then deregister, then rebalance layout. On timeout, capture the pane tail and fail loudly with exit code 2; operator reruns with `--force` for an atomic kill+deregister. This overrides the 0000014 deregister-first invariant — see `design-docs/0000032-robust-member-teardown/design-doc.md` §4.

**Pane display-name propagation**: `CodingAgentConfig` carries a `display_name_args: tuple[str, ...]` field that encodes which CLI flag the spawned coding agent accepts for a session display name. `CLAUDE.display_name_args = ("--name",)`. `member_create` calls `CLAUDE.build_command(prompt, display_name=name)` so the spawned process becomes `claude --name <member-name> <prompt>`, and Claude Code re-emits the name via the terminal title escape sequence so `tmux display-message -p -t <pane> "#{pane_title}"` returns the member name for the lifetime of the pane.

**Commands**: `member create`, `member delete`, `member list`, `member capture`, `member send-input`, `member exec`, `member ping`. All require `--agent-id` (the Director's ID). The tmux helper module (`cafleet/src/cafleet/tmux.py`) isolates all subprocess interaction with tmux. Primitives for pane lifecycle inspection and forced teardown — `pane_exists`, `kill_pane`, and `wait_for_pane_gone` — live here so the CLI never calls tmux directly. Director-side usage of `member send-input --choice` / `--freetext` is AskUserQuestion-delegated (capture → Director-side `AskUserQuestion` → direct Bash invocation of the resolved command); see [`skills/cafleet/SKILL.md`](skills/cafleet/SKILL.md) "Answer a member's AskUserQuestion prompt" for the canonical three-beat workflow and pane-shapes table. `member exec` is the bash-routing primitive — see § Routing Bash via the Director below.

**Write-path authorization mirrors the read path**: `cafleet member send-input` — a safe `tmux send-keys` wrapper for answering an `AskUserQuestion` prompt — reuses the exact `member capture` authorization boundary (`placement.director_agent_id == --agent-id`, non-null `tmux_pane_id`, placement row present). The CLI accepts exactly one of `--choice {1,2,3}` (sends the matching digit key) or `--freetext "<text>"` (sends `4`, the literal text via tmux's `-l` flag, then `Enter` — `4` selects the "Type something" option). Both modes are AskUserQuestion-only. `--freetext` rejects values whose first non-whitespace character is `!` so the AskUserQuestion path cannot smuggle a Claude Code `!`-shortcut; shell dispatch goes through `cafleet member exec` instead. Newlines are rejected for `--freetext` at both the CLI layer and the helper, so each call is exactly one prompt submission. The helper never invokes a shell (`subprocess.run([...], shell=False)`), so shell meta, backticks, `$VAR`, and multi-byte characters pass through as literal input.

`cafleet member exec <command>` is the Director-only shell-dispatch subcommand. It accepts a single required positional `CMD` argument, reuses the same authorization boundary as `member send-input`, and keystrokes `! <command>` + `Enter` into the member's pane via `tmux.send_bash_command` so Claude Code's `!` shortcut runs the command natively. Empty / whitespace-only / newline-containing commands are rejected at the CLI handler with exit 2; cross-Director, missing-placement, and pending-placement rejections share their wording with `member send-input`.

**Operator diagnostics**: `cafleet doctor` prints the calling pane's session/window/pane identifiers (plus `$TMUX_PANE`) for operators diagnosing placement issues without reaching for raw tmux commands. It is a top-level command — not a member-family command — but reuses `tmux.ensure_tmux_available()` so the TMUX-required wording stays consistent with the member surface.

**Supervision skill**: The Director's monitoring obligations are defined in `skills/cafleet-monitoring/SKILL.md`. This skill must be loaded (`Skill(cafleet-monitoring)`) before spawning any members. It provides a 2-stage health check protocol (message poll then terminal capture) and a ready-to-use `/loop` prompt template.

## Bash Routing via Director

Members spawn with `--permission-mode dontAsk` (always, no flag pair to opt in/out). The Bash tool is enabled and permission prompts auto-resolve silently, so members run cafleet (and any shell command) directly via the Bash tool. The CLAUDE spawn-prompt template tells the member explicitly that its harness runs in dontAsk mode.

The bash-via-Director protocol is the **fallback** for the harness deny-list: dontAsk does not auto-resolve everything — destructive operations such as `git push` and `rm -rf` are still rejected at the Claude Code harness layer. When a member's Bash invocation is denied, the member auto-routes by sending a plain CAFleet message to its Director, and the Director dispatches the command into the member's pane via `cafleet member exec "<cmd>"`, which keystrokes literal `! <cmd>` + `Enter` and triggers Claude Code's `!` CLI shortcut on the receiving side. Members must first reconsider whether the rejected command is correct and necessary — most denials are caused by a wrong command, not a missing privilege. See [`skills/cafleet/SKILL.md`](skills/cafleet/SKILL.md) § Routing Bash via the Director for the full convention.

## Design Document Orchestration Skills

CAFleet ships CAFleet-native replicas of the global Agent Teams design document workflows. They replace Claude Code's `TeamCreate` / `Agent(team_name=...)` / `SendMessage` primitives with `cafleet agent register`, `cafleet member create`, and `cafleet message send`, so every inter-agent message is persisted in SQLite and visible in the admin WebUI timeline.

| Skill | Location | Purpose |
|---|---|---|
| `design-doc` | `skills/design-doc/` | Plugin-local copy of the global `/design-doc` skill (template + guidelines). Spawned members load this instead of the global skill so the plugin is self-contained. |
| `design-doc-create` | `skills/design-doc-create/` | Create a design document through CAFleet-orchestrated Director / Drafter / Reviewer roles. Mirrors the process of `/design-doc-create`. |
| `design-doc-execute` | `skills/design-doc-execute/` | Execute a design document through CAFleet-orchestrated Director / Programmer / Tester / (optional) Verifier roles with per-step TDD cycle. Mirrors the process of `/design-doc-execute`. |

**Role files**: Each `*-create` and `*-execute` skill ships a `roles/` directory with one Markdown file per role. The Director reads the relevant role file and embeds its content verbatim in the `cafleet member create` spawn prompt.

**Communication pattern**: Director → member messages are delivered via `cafleet message send`, which triggers a tmux push notification that injects `cafleet message poll` into the member's pane. Member → Director replies use the same `cafleet message send` path. The Director runs the `Skill(cafleet-monitoring)` `/loop` to watch for incoming messages and stalled panes.

**Coexistence**: The global `/design-doc-create` and `/design-doc-execute` Agent Teams skills remain functional. A user picks between them based on whether they want ephemeral in-memory coordination (Agent Teams) or a persistent, auditable message trail in SQLite + WebUI (CAFleet).

## tmux Push Notifications

CAFleet uses a pull-based delivery model by default: recipients discover messages via `cafleet message poll`. To reduce latency, the broker can also push a poll trigger into a recipient's tmux pane immediately after persisting a message.

After `broker` saves a delivery task, it looks up the recipient's `agent_placements` row. Every agent spawned by `cafleet member create` has a placement row, and every session's root Director also gets one at `cafleet session create` time (its placement carries `director_agent_id=NULL` to indicate "no parent"). Because `_try_notify_recipient` resolves a pane by `agent_id` alone, Member → Director notifications work automatically once the root Director has a placement row. If the recipient has a non-null `tmux_pane_id` and is not the sender, the broker runs:

```
tmux send-keys -t <tmux_pane_id> "cafleet --session-id <session_id> message poll --agent-id <recipient_agent_id>" Enter
```

The injected text lands in the coding agent's input prompt. If the agent is idle, it interprets the command immediately. If the agent is busy, tmux buffers the keystrokes until the agent returns to its prompt. Since `cafleet message poll` is idempotent, duplicate or late-arriving triggers are harmless.

**Design principles**:

- **Best-effort**: The message queue remains the sole source of truth. Push notification is an optimization — if it fails, the message is still available for normal polling.
- **Self-send skip**: When sender == recipient, the notification is suppressed.
- **Silent failure**: Missing placements, null `tmux_pane_id`, dead panes, and absent `tmux` binary all result in `False` — no exceptions propagate to the caller.
- **No `TMUX` env var required**: `tmux send-keys -t <pane>` works from any process on the same host as long as the tmux server socket is accessible.

**Response annotations**: Unicast responses include a top-level `notification_sent` boolean. Broadcast summary tasks include `notificationsSentCount` in their metadata, reflecting how many recipient panes were successfully triggered; the top-level response exposes this value as `notifications_sent_count`.

**Manual entry-point**: `tmux.send_poll_trigger` has two callers — `broker._try_notify_recipient` (the auto-fire path triggered after every `cafleet message send`, best-effort and silent on failure) and `cafleet member ping` (the Director-only manual nudge subcommand, which converts a `False` return to exit 1 so an operator or monitoring loop sees the failure). Both inject the same `cafleet --session-id <s> message poll --agent-id <r>` keystroke + Enter; only the failure-handling differs.

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
| Session ID | `--session-id` global flag (UUID; required for client + member subcommands) |
| Database URL | `CAFLEET_DATABASE_URL` env var (optional; default builds `sqlite:///<path>` from `~/.local/share/cafleet/registry.db` with `~` expanded at load time via `Path(...).expanduser()`) |
| Agent ID | `--agent-id` subcommand option |
| JSON output | `--json` global flag |

Session ID and Agent ID are passed as literal CLI flags (not environment variables) so a single Claude Code `permissions.allow` pattern of the form `cafleet --session-id <literal-uuid> *` matches every subcommand for that session, eliminating per-invocation permission prompts. `--session-id` is global (placed before the subcommand) and required for every client + member subcommand; it is silently accepted (and ignored) on `db init` / `session *` / `server` so one allow pattern stays usable everywhere. No broker URL is needed — CLI commands access SQLite directly.

The `cafleet server` bind address and port are configured via `--host` / `--port` flags (defaults sourced from `settings.broker_host` = `127.0.0.1` and `settings.broker_port` = `8000`) or via the `CAFLEET_BROKER_HOST` / `CAFLEET_BROKER_PORT` environment variables. Pydantic-settings wires these env vars through explicit `validation_alias` on `Settings.broker_host` and `Settings.broker_port`, matching the `CAFLEET_`-prefixed convention already used by `CAFLEET_DATABASE_URL`. CLI flags win over env vars when both are supplied. The `127.0.0.1` default matches CAFleet's local-only stance; users who need external binding pass `--host 0.0.0.0` or set `CAFLEET_BROKER_HOST=0.0.0.0`.

## WebUI

A browser-based dashboard served as a SPA at `/ui/`. No login is required. The first-load lands on a session picker at `/ui/#/sessions`; selecting a session navigates to a Discord-style unified timeline for that session — a sidebar listing every active (top) and deregistered (muted) agent in the session, a center timeline rendering unicast and broadcast messages ordered newest-at-bottom with auto-scroll, reactions-as-ACKs chips that reveal per-recipient ACK time on CSS hover, and a bottom input that parses `@<agent> text` for unicast and `@all text` for broadcast. The admin is NOT a CAFleet agent; a header dropdown (sender selector) picks which real in-session active agent is used as `from_agent_id` on every send, persisted per-session in `localStorage` under `cafleet.sender.<session_id>`.

- **Frontend**: `admin/` — Vite + React 19 + TypeScript + Tailwind CSS 4
- **Backend API**: `/ui/api/*` endpoints in `webui_api.py` — all endpoints call `broker` for data access (sync `def` handlers, FastAPI runs them in a thread pool)
- **Server**: `server.py` is a minimal FastAPI app — just `webui_router` + static files. No A2A handler, no JSON-RPC, no executor. Only needed for the WebUI; CLI commands work without it.
- **Session scoping**: Session-scoped endpoints require `X-Session-Id` header. No authentication.
- **Static serving**: `StaticFiles` mount at `/ui` serves the SPA bundled inside the package at `cafleet/src/cafleet/webui/` (production build). `mise //admin:build` must be run before `cafleet server` / `mise //cafleet:dev` for `/ui/` to be populated; without it, `create_app()` emits a one-line `warning: admin WebUI is not built. /ui/ will return 404. Run 'mise //admin:build'.` to stderr at startup, the server starts cleanly, and `/ui/` 404s until the SPA is built. The warning fires from `create_app()` so every startup path (`cafleet server`, `mise //cafleet:dev`, and any `uv run uvicorn cafleet.server:app`) sees it identically.

## Package Structure

A single Python package and a frontend app:

- **`cafleet/`** — `cafleet`: FastAPI + SQLAlchemy + Alembic + click (server + CLI). Ships the unified `cafleet` console script for all operations: `db init`, `session` management, agent registration, messaging, and member lifecycle. CLI commands access SQLite directly via `broker.py`; the FastAPI server is only needed for the admin WebUI.
- **`admin/`** — WebUI SPA: Vite + React + TypeScript + Tailwind CSS

A single `pip install cafleet` gives users both the broker server and the agent CLI.
