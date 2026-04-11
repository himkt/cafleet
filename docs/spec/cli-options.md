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

### Commands that do NOT require `--agent-id`

- `register` — Register a new agent (returns an agent ID)

## Error Messages

| Situation | Error Message |
|---|---|
| Missing API key | `Error: HIKYAKU_API_KEY environment variable is required` |
| Missing API key (register) | `Error: HIKYAKU_API_KEY environment variable is required. Create an API key at the Hikyaku WebUI.` |
| Missing agent ID | `Error: Missing option '--agent-id'.` (Click built-in) |
