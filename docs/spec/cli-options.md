# CLI Option Specification

How the unified CAFleet CLI (`cafleet`) accepts configuration parameters.

## Option Source Matrix

Each parameter has exactly one input source:

| Parameter | Source |
|---|---|
| Session ID | `--session-id <uuid>` global flag |
| Database URL | `CAFLEET_DATABASE_URL` env var (optional; default builds `sqlite:///<path>` from `~/.local/share/cafleet/registry.db` with `~` expanded at load time. When setting `CAFLEET_DATABASE_URL` yourself, use an absolute path — SQLAlchemy does not expand `~` in SQLite URLs.) |
| Agent ID | `--agent-id <uuid>` subcommand option |
| JSON output | `--json` global flag |

> **Why `--session-id` is a literal CLI flag, not an environment variable.** Claude Code's `permissions.allow` matches Bash invocations as literal command strings. A literal `cafleet --session-id <uuid> ...` invocation matches a single `permissions.allow` pattern of the same shape across every subcommand for that session. Shell-expansion patterns (`export VAR=...` followed by `$VAR` substitution) break that matching and force per-invocation permission prompts that interrupt agent work. Substitute the literal UUIDs printed by `cafleet session create` and `cafleet register` — do not use shell variables to hold them.

## Global Options

Placed **before** the subcommand:

| Flag | Required | Notes |
|---|---|---|
| `--json` | no | Emit JSON output. |
| `--session-id <id>` | yes for client + member subcommands; no for `db init` and `session *` | Session identifier (opaque string; new sessions get a UUIDv4, migrated sessions reuse a 64-char hex value). Also called the namespace identifier. Silently accepted (and ignored) when supplied to subcommands that do not need it, so a single `permissions.allow` pattern of the form `cafleet --session-id <literal-id> *` works for every subcommand. |

### Subcommands that require `--session-id`

`register`, `send`, `broadcast`, `poll`, `ack`, `cancel`, `get-task`, `agents`, `deregister`, `member create`, `member delete`, `member list`, `member capture`.

### Subcommands that do NOT require `--session-id`

`db init`, `db *`, `session create`, `session list`, `session show`, `session delete`, `server`.

Create a session first if you don't have one:

```bash
cafleet session create --label "my-project"
# → prints the session_id
```

Then pass the printed UUID as `--session-id <uuid>` on every client + member command.

## Removed CLI Options

The following CLI options, environment variables, and subcommands have been removed:

- `--url` flag and the corresponding broker-URL env var — CLI commands access SQLite directly; no broker URL is needed.
- `--api-key` flag — Removed entirely (sessions replace API keys).
- The session-id env var — Replaced by the `--session-id` global flag.
- The agent-id env var — Replaced by literal `--agent-id <uuid>` substitution at member-spawn time.
- `cafleet env` subcommand — Existed only to dump env vars; obsolete now that session/agent IDs are passed as flags.

These removals keep secrets out of shell history and let `permissions.allow` patterns match every invocation literally.

## Agent ID (`--agent-id`)

`--agent-id` is a **per-subcommand option** (not a global option). It identifies which agent is acting and must be specified on each invocation.

### Commands that require `--agent-id`

- `send` — Send a message to another agent
- `broadcast` — Broadcast a message to all agents
- `poll` — Poll for incoming messages
- `ack` — Acknowledge a received message
- `cancel` — Cancel a sent message
- `get-task` — Get task details
- `agents` — List agents in the session
- `deregister` — Deregister an agent
- `member create` — Register a new member and spawn its claude pane (Director only)
- `member delete` — Deregister a member and close its pane (Director only)
- `member list` — List members spawned by this Director
- `member capture` — Capture the last N lines of a member's pane (Director only)

### Commands that do NOT require `--agent-id`

- `register` — Register a new agent (returns an agent ID)

## `cafleet session` — Session Management

The `cafleet session` subgroup manages sessions. These commands write directly to SQLite — the broker server does not need to be running.

### `session create`

| Flag | Required | Notes |
|---|---|---|
| `--label` | no | Free-form text label for the session |
| `--json` | no | Output as JSON |

There are no `--name` / `--description` flags. The root Director's name and description are hardcoded (`name="director"`, `description="Root Director for this session"`).

Creates a new session with a UUIDv4 identifier. **Must be run inside a tmux session** — outside tmux the command exits 1 with `Error: cafleet session create must be run inside a tmux session` and writes nothing to the DB. The command atomically performs five writes in a single transaction:

1. `INSERT INTO sessions (...)` with `deleted_at=NULL`, `director_agent_id=NULL`.
2. `INSERT INTO agents (...)` for the hardcoded root Director.
3. `INSERT INTO agent_placements (...)` for the Director with `director_agent_id=NULL` and `coding_agent="unknown"`.
4. `UPDATE sessions SET director_agent_id = <director_agent_id>`.
5. `INSERT INTO agents (...)` for the built-in `Administrator` (see [data-model.md](./data-model.md) for the Administrator's distinguishing `agent_card_json.cafleet.kind` flag).

Any exception inside the transaction rolls back all five writes.

**Non-JSON output** — line 1 is `session_id` (preserves backward-compatible scripts that parse only the first line), line 2 is the root Director's `agent_id`:

```
<session_id>
<director_agent_id>
label:            <label or empty>
created_at:       <iso8601>
director_name:    director
pane:             <tmux_session>:<tmux_window_id>:<tmux_pane_id>
administrator:    <administrator_agent_id>
```

**`--json` output** — nested shape with `administrator_agent_id` at the top level alongside `director`:

```json
{
  "session_id": "550e8400-e29b-41d4-a716-446655440000",
  "label": "my-project",
  "created_at": "2026-04-15T10:00:00+00:00",
  "administrator_agent_id": "3c4d5e6f-7890-1234-5678-90abcdef1234",
  "director": {
    "agent_id": "7ba91234-5678-90ab-cdef-112233445566",
    "name": "director",
    "description": "Root Director for this session",
    "registered_at": "2026-04-15T10:00:00+00:00",
    "placement": {
      "director_agent_id": null,
      "tmux_session": "main",
      "tmux_window_id": "@3",
      "tmux_pane_id": "%0",
      "coding_agent": "unknown",
      "created_at": "2026-04-15T10:00:00+00:00"
    }
  }
}
```

`placement.director_agent_id` is `null` because the root Director has no parent. `placement.coding_agent` is the string `"unknown"` — auto-detection of the actual coding agent binary at bootstrap time is deferred (tracked via a `FIXME(claude)` comment in `broker.py`).

Attempting `cafleet --session-id <session_id> deregister --agent-id <director_agent_id>` is rejected by the broker with `Error: cannot deregister the root Director; use 'cafleet session delete' instead.` and exits 1. Attempting `cafleet --session-id <session_id> deregister --agent-id <administrator_agent_id>` is rejected with `Error: Administrator cannot be deregistered` (exit 1) via the `AdministratorProtectedError` path from design 0000025.

### `session list`

| Flag | Required | Notes |
|---|---|---|
| `--json` | no | Output as JSON |

Lists all **non-soft-deleted** sessions with their label, created_at, and active agent count. There is no `--all` flag in this revision — soft-deleted sessions (`sessions.deleted_at IS NOT NULL`) are hidden.

### `session show`

| Argument | Required | Notes |
|---|---|---|
| `session_id` | yes | The session to show |
| `--json` | no | Output as JSON |

Shows details of a single session. Exits non-zero if the session does not exist.

### `session delete`

| Argument | Required | Notes |
|---|---|---|
| `session_id` | yes | The session to delete |

Soft-deletes a session. All three operations run in one transaction:

1. `UPDATE sessions SET deleted_at = now WHERE session_id = X AND deleted_at IS NULL`.
2. `UPDATE agents SET status = 'deregistered', deregistered_at = now WHERE session_id = X AND status = 'active'` (sweeps every active agent in the session — root Director included).
3. `DELETE FROM agent_placements WHERE agent_id IN (SELECT agent_id FROM agents WHERE session_id = X)`.

Tasks are untouched — the message history remains queryable. Output:

```
Deleted session <session_id>. Deregistered N agents.
```

`N` counts every agent that was active at the moment of deletion (root Director included). On re-run against an already-deleted session, the `WHERE deleted_at IS NULL` guard on step 1 short-circuits the cascade and the command prints `Deleted session <session_id>. Deregistered 0 agents.` and exits 0 — the command is idempotent.

There is no `--force` flag. Calling `session delete` on an unknown `session_id` exits 1 with `Error: session 'X' not found.`.

Member tmux panes spawned by `cafleet member create` are **not** automatically closed by `session delete`. For a clean teardown, call `cafleet member delete` per member first (which sends `/exit` to the pane). Any surviving `claude` / `codex` processes with orphaned placements can be terminated manually with `tmux kill-pane`.

## `cafleet server` — Admin WebUI Server

Starts the admin WebUI FastAPI app (the same app served by `mise //cafleet:dev`) via uvicorn. CLI commands do not require this server to be running — it is only needed when a user wants to view the WebUI at `/ui/` or hit the `/ui/api/*` endpoints from a browser.

`cafleet server` does NOT require `--session-id`. Supplying `--session-id` is silently accepted and ignored, matching the `db init` / `session *` pattern.

| Flag | Default | Notes |
|---|---|---|
| `--host` | `settings.broker_host` (default `127.0.0.1`) | Bind address. Overrides `CAFLEET_BROKER_HOST` when both are set. |
| `--port` | `settings.broker_port` (default `8000`) | Bind port. Overrides `CAFLEET_BROKER_PORT` when both are set. |

Environment variables (read by `cafleet.config.Settings` via explicit `validation_alias`, consistent with `CAFLEET_DATABASE_URL`):

| Variable | Settings field | Notes |
|---|---|---|
| `CAFLEET_BROKER_HOST` | `broker_host` | Wired via `Field(validation_alias="CAFLEET_BROKER_HOST")` on `Settings`. |
| `CAFLEET_BROKER_PORT` | `broker_port` | Wired via `Field(validation_alias="CAFLEET_BROKER_PORT")` on `Settings`. |

The CLI flag wins when both a flag and the matching env var are set; the env var wins when only it is set; the hardcoded default (`127.0.0.1` / `8000`) applies otherwise.

### Behavior

- Calls `uvicorn.run("cafleet.server:app", host=<resolved>, port=<resolved>)` with no `reload`, no custom `workers`, and no custom `log_level` — uvicorn defaults apply.
- On startup, if the bundled WebUI dist directory does not exist, `create_app()` emits a one-line warning to stderr: `warning: admin WebUI is not built. /ui/ will return 404. Run 'mise //admin:build'.`. The warning fires from `create_app()`, so `cafleet server`, `mise //cafleet:dev`, and any direct `uv run uvicorn cafleet.server:app` invocation all see it identically.
- Port-in-use errors are NOT wrapped — uvicorn's native `OSError: [Errno 98] Address already in use` (or the corresponding click/uvicorn traceback) propagates to the terminal.
- The `cafleet server` handler does not perform any disk check itself; the dist-directory warning is entirely owned by `create_app()`.

### No other flags

`--reload`, `--workers`, `--log-level`, and `--webui-dist-dir` are deliberately NOT exposed on `cafleet server`. Users who need them invoke uvicorn directly — which is exactly what `mise //cafleet:dev` does (it runs `uv run uvicorn cafleet.server:app --host 127.0.0.1 --port 8000` as an independent entry point, without delegating to `cafleet server`).

### Examples

```bash
# Defaults: 127.0.0.1:8000
cafleet server

# Override via flags
cafleet server --host 0.0.0.0 --port 9000

# Override via env vars
CAFLEET_BROKER_HOST=0.0.0.0 CAFLEET_BROKER_PORT=9000 cafleet server

# --session-id is silently accepted and ignored
cafleet --session-id 550e8400-e29b-41d4-a716-446655440000 server
```

## Member Commands

The `cafleet member` subgroup manages tmux-backed member agents. All commands require `--agent-id` (the Director's agent ID) and must be run inside a tmux session.

### `member create`

| Flag | Required | Notes |
|---|---|---|
| `--agent-id` | yes | Director's agent ID |
| `--name` | yes | Display name of the new member |
| `--description` | yes | One-sentence purpose |
| `--coding-agent` | no | Coding agent to spawn: `claude` (default) or `codex`. Codex is spawned with `--approval-mode auto-edit`. |
| *(positional, after `--`)* | no | Prompt text for the spawned coding agent process |

### `member delete`

| Flag | Required | Notes |
|---|---|---|
| `--agent-id` | yes | Director's agent ID |
| `--member-id` | yes | Target member's agent ID |

### `member list`

| Flag | Required | Notes |
|---|---|---|
| `--agent-id` | yes | Director's agent ID |

### `member capture`

| Flag | Required | Notes |
|---|---|---|
| `--agent-id` | yes | Director's agent ID |
| `--member-id` | yes | Target member's agent ID |
| `--lines` | no | Number of trailing lines to capture (default: 80) |

## Error Messages

| Situation | Error Message |
|---|---|
| Missing `--session-id` on a client/member subcommand | `Error: --session-id <uuid> is required for this subcommand. Create a session with 'cafleet session create' and pass its id.` |
| Missing `--agent-id` | `Error: Missing option '--agent-id'.` (Click built-in) |
| `session create` run outside a tmux session | `Error: cafleet session create must be run inside a tmux session` (exit 1; no DB writes) |
| `session delete` on unknown session_id | `Error: session 'X' not found.` (exit 1) |
| `register` into a soft-deleted session | `Error: session X is deleted` (exit 1) |
| `deregister` against the root Director's `agent_id` | `Error: cannot deregister the root Director; use 'cafleet session delete' instead.` (exit 1) |
| `deregister` against the Administrator's `agent_id` | `Error: Administrator cannot be deregistered` (exit 1) |
