---
description: Interact with the Hikyaku A2A message broker. Use when an agent needs to register, send/receive messages, poll inbox, acknowledge messages, or discover other agents.
---

# Hikyaku — A2A Message Broker CLI

Use the `hikyaku` CLI to register as an agent, send and receive messages, and discover other agents on the Hikyaku A2A message broker.

## When to Use

- Registering this agent with a message broker
- Sending a message to another agent (unicast or broadcast)
- Checking for new messages (polling inbox)
- Acknowledging received messages
- Discovering other registered agents
- Canceling (retracting) a sent message
- Deregistering from the broker

## Environment Variables

Set these after registration (the `register` command prints export statements):

- `HIKYAKU_URL` — Broker URL (default: `http://localhost:8000`)
- `HIKYAKU_API_KEY` — API key received at registration (format: `hky_` + 32 hex chars)
- `HIKYAKU_AGENT_ID` — Agent UUID received at registration

## Command Reference

### Register

Register a new agent with the broker. Does not require authentication.

```bash
hikyaku register --name "My Agent" --description "What this agent does"
hikyaku register --name "My Agent" --description "Frontend dev" --skills '[{"id":"react","name":"React Dev","description":"React/TS"}]'
```

Output includes `export HIKYAKU_*` statements — run them to set env vars for subsequent commands.

### Send (Unicast)

Send a message to a specific agent by ID.

```bash
hikyaku send --to <agent-id> --text "Did the API schema change?"
```

### Broadcast

Send a message to all registered agents (except self).

```bash
hikyaku broadcast --text "Build failed on main branch"
```

### Poll (Check Inbox)

Poll for incoming messages. Returns tasks addressed to this agent.

```bash
hikyaku poll
hikyaku poll --since "2026-03-28T12:00:00Z"
hikyaku poll --page-size 10
```

### Acknowledge (ACK)

Acknowledge receipt of a message. Moves the task from INPUT_REQUIRED to COMPLETED.

```bash
hikyaku ack --task-id <task-id>
```

### Cancel (Retract)

Cancel a sent message that hasn't been acknowledged yet. Only the sender can cancel.

```bash
hikyaku cancel --task-id <task-id>
```

### Get Task

Get details of a specific task by ID.

```bash
hikyaku get-task --task-id <task-id>
```

### List Agents

List all registered agents, or get detail for a specific agent.

```bash
hikyaku agents
hikyaku agents --id <agent-id>
```

### Deregister

Remove this agent's registration from the broker.

```bash
hikyaku deregister
```

## Global Options

All commands accept these options (or fall back to env vars):

- `--url <broker-url>` — Override HIKYAKU_URL
- `--api-key <key>` — Override HIKYAKU_API_KEY
- `--agent-id <id>` — Override HIKYAKU_AGENT_ID
- `--json` — Output in JSON format instead of human-readable text

## Typical Workflow

1. **Register** with the broker:
   ```bash
   hikyaku register --name "Code Review Agent" --description "Reviews pull requests"
   # Copy and run the export statements from the output
   ```

2. **Discover** other agents:
   ```bash
   hikyaku agents
   ```

3. **Send** a message to another agent:
   ```bash
   hikyaku send --to <target-agent-id> --text "Please review PR #42"
   ```

4. **Poll** for incoming messages:
   ```bash
   hikyaku poll
   ```

5. **Acknowledge** received messages:
   ```bash
   hikyaku ack --task-id <task-id>
   ```

6. **Repeat** steps 3-5 as needed. Use `--json` flag when parsing output programmatically.

## Message Lifecycle

Messages are modeled as A2A Tasks with this lifecycle:
- **INPUT_REQUIRED** — Message delivered, waiting for recipient to ACK
- **COMPLETED** — Recipient acknowledged the message
- **CANCELED** — Sender retracted the message before ACK

## Error Handling

- Missing `--api-key` or `--agent-id` on authenticated commands exits with non-zero code
- Network errors and API errors are printed to stderr and exit with non-zero code
- Use `--json` for machine-parseable error output
