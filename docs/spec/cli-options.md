# CLI Option Specification

How the Hikyaku CLI (`hikyaku`) accepts configuration parameters.

## Option Source Matrix

Each parameter has exactly one input source:

| Parameter | CLI (`client/`) |
|---|---|
| API Key | `HIKYAKU_API_KEY` env var |
| Broker URL | `HIKYAKU_URL` env var (default: `http://localhost:8000`) |
| Agent ID | `--agent-id` subcommand option |
| JSON output | `--json` global flag |

## Environment Variable Setup

Set these environment variables before using the CLI:

```bash
export HIKYAKU_URL=http://localhost:8000    # Broker URL (defaults to http://localhost:8000)
export HIKYAKU_API_KEY=your-api-key-here    # Required for all operations
```

## Removed CLI Options

The following CLI options have been removed:

- `--url` — Use `HIKYAKU_URL` environment variable instead
- `--api-key` — Use `HIKYAKU_API_KEY` environment variable instead

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
- `agents` — List agents in the tenant
- `deregister` — Deregister an agent
- `member create` — Register a new member and spawn its claude pane (Director only)
- `member delete` — Deregister a member and close its pane (Director only)
- `member list` — List members spawned by this Director
- `member capture` — Capture the last N lines of a member's pane (Director only)

### Commands that do NOT require `--agent-id`

- `register` — Register a new agent (returns an agent ID)

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
| Missing API key | `Error: HIKYAKU_API_KEY environment variable is required` |
| Missing API key (register) | `Error: HIKYAKU_API_KEY environment variable is required. Create an API key at the Hikyaku WebUI.` |
| Missing agent ID | `Error: Missing option '--agent-id'.` (Click built-in) |
