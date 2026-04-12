# CLI Option Specification

How the Hikyaku CLI (`hikyaku`) and server CLI (`hikyaku-registry`) accept configuration parameters.

## Option Source Matrix

Each parameter has exactly one input source:

| Parameter | CLI (`client/`) |
|---|---|
| Session ID | `HIKYAKU_SESSION_ID` env var |
| Broker URL | `HIKYAKU_URL` env var (default: `http://127.0.0.1:8000`) |
| Agent ID | `--agent-id` subcommand option |
| JSON output | `--json` global flag |

## Environment Variable Setup

Set these environment variables before using the CLI:

```bash
export HIKYAKU_URL=http://127.0.0.1:8000      # Broker URL (defaults to http://127.0.0.1:8000)
export HIKYAKU_SESSION_ID=your-session-id-here  # Required for all operations
```

Create a session first if you don't have one:

```bash
hikyaku-registry session create --label "my-project"
# → prints the session_id
```

## Removed CLI Options

The following CLI options have been removed:

- `--url` — Use `HIKYAKU_URL` environment variable instead
- `--api-key` — Removed entirely (sessions replace API keys)

These options were removed to prevent secrets from appearing in shell history or `ps` output.

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

## `hikyaku-registry session` — Session Management

The `hikyaku-registry session` subgroup manages session namespaces. These commands write directly to SQLite — the broker server does not need to be running.

### `session create`

| Flag | Required | Notes |
|---|---|---|
| `--label` | no | Free-form text label for the session |
| `--json` | no | Output as JSON |

Creates a new session with a UUIDv4 identifier. Prints the session_id to stdout.

### `session list`

| Flag | Required | Notes |
|---|---|---|
| `--json` | no | Output as JSON |

Lists all sessions with their label, created_at, and active agent count.

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

Deletes a session. Fails with a friendly error if agents still reference the session (FK RESTRICT violation).

## Member Commands

The `hikyaku member` subgroup manages tmux-backed member agents. All commands require `--agent-id` (the Director's agent ID) and must be run inside a tmux session.

### `member create`

| Flag | Required | Notes |
|---|---|---|
| `--agent-id` | yes | Director's agent ID |
| `--name` | yes | Display name of the new member |
| `--description` | yes | One-sentence purpose |
| *(positional, after `--`)* | no | Prompt text for the spawned `claude` process |

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
| Missing session ID | `Error: HIKYAKU_SESSION_ID environment variable is required. Create a session with 'hikyaku-registry session create'.` |
| Missing agent ID | `Error: Missing option '--agent-id'.` (Click built-in) |
