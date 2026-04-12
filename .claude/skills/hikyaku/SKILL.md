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
- Spawning and managing member agents in tmux panes (Director only)
- Inspecting a stalled member's terminal output (Director only)

## Environment Variables

The CLI reads both variables from the environment — they are the **only** way to configure the CLI. There are no `--url` / `--session-id` flags.

- `HIKYAKU_URL` — Broker URL, must include the `http://` / `https://` scheme (default: `http://127.0.0.1:8000`). The CLI errors with "Request URL is missing an 'http://' or 'https://' protocol" if the scheme is missing.
- `HIKYAKU_SESSION_ID` — Session namespace ID created via `hikyaku-registry session create`. The CLI exits with `Error: HIKYAKU_SESSION_ID environment variable is required. Create a session with 'hikyaku-registry session create'.` if this is not set.

## Agent ID

Every command **except `register`** requires `--agent-id <id>`. `register` returns the new `agent_id` — save it and pass it to every subsequent command.

## Global Options

Only `--json` exists, and it must be placed **before** the subcommand:

```bash
hikyaku --json register --name "My Agent" --description "..."
hikyaku --json agents --agent-id <agent-id>
```

`hikyaku agents --json` will fail with `No such option: --json`.

## Command Reference

### Register

Register a new agent with the broker. `HIKYAKU_SESSION_ID` must be set.

```bash
hikyaku register --name "My Agent" --description "What this agent does"
hikyaku register --name "My Agent" --description "Frontend dev" --skills '[{"id":"react","name":"React Dev","description":"React/TS"}]'
```

Returns the newly created `agent_id`. Record it; every other command needs it via `--agent-id`.

#### Self-registration recipe

Use `--json` so the output is machine-parseable, and capture `agent_id` for every subsequent call:

```bash
hikyaku --json register \
  --name "<short-label>" \
  --description "<one-sentence purpose>"
```

JSON response (field order is not guaranteed):

```json
{
  "agent_id": "<uuid>",
  "name": "<short-label>",
  "registered_at": "<iso8601>"
}
```

Rules:

- **Name**: short, human-identifiable label (`Claude-A`, `reviewer-bot`, …). Not `test`, `foo`, etc.
- **Description**: one sentence stating who the agent is and what it is for.
- **Capture `agent_id` immediately.** It is required for every subsequent call; losing it forces re-registration.
- Non-`--json` output prints `Agent registered successfully!` followed by `  agent_id:  <uuid>` and `  name:      <name>`. Parse the `agent_id:` line if `--json` is not an option.
- Call `hikyaku deregister --agent-id <id>` at end of session so stale registrations do not accumulate.

### List Agents

List all registered agents, or get detail for a specific agent.

```bash
hikyaku agents --agent-id <self-agent-id>
hikyaku agents --agent-id <self-agent-id> --id <target-agent-id>
```

### Send (Unicast)

Send a message to a specific agent by ID.

```bash
hikyaku send --agent-id <self-agent-id> --to <target-agent-id> --text "Did the API schema change?"
```

### Broadcast

Send a message to all registered agents (except self).

```bash
hikyaku broadcast --agent-id <self-agent-id> --text "Build failed on main branch"
```

### Poll (Check Inbox)

Poll for incoming messages. Returns tasks addressed to this agent.

```bash
hikyaku poll --agent-id <self-agent-id>
hikyaku poll --agent-id <self-agent-id> --since "2026-03-28T12:00:00Z"
hikyaku poll --agent-id <self-agent-id> --page-size 10
```

### Acknowledge (ACK)

Acknowledge receipt of a message. Moves the task from INPUT_REQUIRED to COMPLETED.

```bash
hikyaku ack --agent-id <self-agent-id> --task-id <task-id>
```

### Cancel (Retract)

Cancel a sent message that hasn't been acknowledged yet. Only the sender can cancel.

```bash
hikyaku cancel --agent-id <self-agent-id> --task-id <task-id>
```

### Get Task

Get details of a specific task by ID.

```bash
hikyaku get-task --agent-id <self-agent-id> --task-id <task-id>
```

### Deregister

Remove this agent's registration from the broker.

```bash
hikyaku deregister --agent-id <self-agent-id>
```

### Member Create

Register a new member agent and spawn its `claude` pane in the Director's own tmux window. Must be run inside a tmux session. The command atomically registers the agent, creates a placement row, spawns the pane, and patches the placement with the real pane ID.

```bash
hikyaku member create --agent-id $DIRECTOR_ID --name Claude-B \
  --description "Reviewer for PR #42"

hikyaku member create --agent-id $DIRECTOR_ID --name Claude-B \
  --description "Reviewer for PR #42" \
  -- "Review PR #42, post feedback via send, and deregister on completion."
```

| Flag | Required | Notes |
|---|---|---|
| `--agent-id` | yes | The Director's agent ID (sent as `X-Agent-Id`) |
| `--name` | yes | Display name of the new member |
| `--description` | yes | One-sentence purpose |
| *(positional, after `--`)* | no | Prompt for the spawned `claude` process. If omitted, a default prompt is generated. |

If the tmux `split-window` fails, the registered agent is rolled back. If the placement PATCH fails, the pane is `/exit`'d and the agent rolled back.

Output (text):
```
Member registered and spawned.
  agent_id:  <new-uuid>
  name:      Claude-B
  pane_id:   %7
  window_id: @3
```

Output (`--json`):
```json
{
  "agent_id": "<uuid>",
  "name": "Claude-B",
  "registered_at": "2026-04-12T10:15:00Z",
  "placement": {
    "director_agent_id": "<director-uuid>",
    "tmux_session": "main",
    "tmux_window_id": "@3",
    "tmux_pane_id": "%7",
    "created_at": "2026-04-12T10:15:00Z"
  }
}
```

### Member Delete

Deregister a member agent and close its tmux pane. The agent is deregistered FIRST, then `/exit` is sent to the pane — so a deregister failure leaves both intact for retry.

```bash
hikyaku member delete --agent-id $DIRECTOR_ID --member-id <member-agent-id>
```

| Flag | Required | Notes |
|---|---|---|
| `--agent-id` | yes | The Director's agent ID |
| `--member-id` | yes | The target member's agent ID |

Output (text):
```
Member deleted.
  agent_id:  <target-uuid>
  pane_id:   %7 (closed)
```

### Member List

List all members spawned by this Director. Returns agents with placement rows whose `director_agent_id` matches the given `--agent-id`.

```bash
hikyaku member list --agent-id $DIRECTOR_ID
hikyaku --json member list --agent-id $DIRECTOR_ID
```

| Flag | Required | Notes |
|---|---|---|
| `--agent-id` | yes | The Director's agent ID |

Output columns: `agent_id`, `name`, `status`, `session`, `window_id`, `pane_id`, `created_at`. A pending placement (pane not yet spawned) shows `(pending)` for `pane_id` in text mode and `null` in JSON.

Output (`--json`):
```json
[
  {
    "agent_id": "<uuid>",
    "name": "Claude-B",
    "status": "active",
    "registered_at": "2026-04-12T10:15:00Z",
    "placement": {
      "director_agent_id": "<director-uuid>",
      "tmux_session": "main",
      "tmux_window_id": "@3",
      "tmux_pane_id": "%7",
      "created_at": "2026-04-12T10:15:00Z"
    }
  }
]
```

### Member Capture

Capture the last N lines of a member's tmux pane terminal buffer. This is the canonical way to inspect a stalled teammate — it replaces raw `tmux capture-pane` invocations for any project using Hikyaku.

```bash
hikyaku member capture --agent-id $DIRECTOR_ID --member-id $MEMBER_ID
hikyaku member capture --agent-id $DIRECTOR_ID --member-id $MEMBER_ID --lines 200
hikyaku --json member capture --agent-id $DIRECTOR_ID --member-id $MEMBER_ID | jq -r .content
```

| Flag | Required | Notes |
|---|---|---|
| `--agent-id` | yes | The Director's agent ID |
| `--member-id` | yes | The target member's agent ID |
| `--lines` | no | Number of trailing lines to capture (default: 80) |

Cross-Director capture is rejected: the CLI verifies `placement.director_agent_id` matches `--agent-id` before making any tmux call.

Output (text): raw captured terminal buffer, printed to stdout with no framing.

Output (`--json`):
```json
{
  "member_agent_id": "<uuid>",
  "pane_id": "%7",
  "lines": 80,
  "content": "...<raw buffer>..."
}
```

**Note on external `agent-team-supervision` skill**: The external `agent-team-supervision` skill (user-level, outside this repo) still documents raw `tmux capture-pane`. For projects using Hikyaku, prefer `hikyaku member capture` as it enforces the cross-Director boundary. The external skill alignment is tracked as follow-up work.

## Typical Workflow

1. **Create a session** (if one does not already exist):
   ```bash
   hikyaku-registry session create --label "my-project"
   # → prints the session_id; export it
   export HIKYAKU_SESSION_ID=<session_id>
   ```

2. **Register** with the broker (`HIKYAKU_SESSION_ID` must already be set):
   ```bash
   hikyaku register --name "Code Review Agent" --description "Reviews pull requests"
   # → record the returned agent_id as $MY_ID
   ```

3. **Discover** other agents:
   ```bash
   hikyaku agents --agent-id $MY_ID
   ```

4. **Send** a message to another agent:
   ```bash
   hikyaku send --agent-id $MY_ID --to <target-agent-id> --text "Please review PR #42"
   ```

5. **Poll** for incoming messages:
   ```bash
   hikyaku poll --agent-id $MY_ID
   ```

6. **Acknowledge** received messages:
   ```bash
   hikyaku ack --agent-id $MY_ID --task-id <task-id>
   ```

7. **Repeat** steps 4-6 as needed. Use `hikyaku --json <cmd>` when parsing output programmatically.

## Multi-Session Coordination

### Roles

- **Director** — the Claude Code session that first runs `hikyaku register` in this project. It owns the team lifecycle: spawning members, driving the exchange, and cleaning up.
- **Member** — any peer Claude Code session the Director spawns via `hikyaku member create`. Each member is automatically registered and receives `HIKYAKU_*` env vars via tmux `-e` flags.

### Monitoring mandate (Director only)

Before spawning **any** member, the Director MUST load `Skill(agent-team-supervision)` and start a `/loop` monitor as that skill instructs. Members do not act autonomously — if the Director stops supervising, the team stalls silently. Keep the `/loop` active until the final shutdown step.

To inspect a stalled member, use `hikyaku member capture`:

```bash
hikyaku member capture --agent-id $DIRECTOR_ID --member-id $MEMBER_ID
```

### Layout discipline

`hikyaku member create` automatically maintains `main-vertical` layout:

- Director occupies the full-height left "main" pane.
- Every member is stacked in the right column at equal height.
- Every `member create` and `member delete` runs `tmux select-layout main-vertical` internally.

### Spawn a member

```bash
hikyaku member create --agent-id $DIRECTOR_ID --name Claude-B \
  --description "Reviewer for PR #42"
```

The command handles everything atomically: registering the agent, forwarding `HIKYAKU_URL`, `HIKYAKU_SESSION_ID`, and `HIKYAKU_AGENT_ID` to the new pane via `-e` flags, spawning `claude` with the prompt, and rebalancing the layout. No `printenv` step is needed.

### Shut down a member

```bash
hikyaku member delete --agent-id $DIRECTOR_ID --member-id <member-agent-id>
```

The command deregisters the agent first (so a failure preserves the pane for retry), then sends `/exit` to the pane, then rebalances the layout.

After every member is shut down, the Director deregisters itself and stops the `/loop` monitor:

```bash
hikyaku deregister --agent-id <director-agent-id>
```

## Message Lifecycle

Messages are modeled as A2A Tasks with this lifecycle:
- **INPUT_REQUIRED** — Message delivered, waiting for recipient to ACK
- **COMPLETED** — Recipient acknowledged the message
- **CANCELED** — Sender retracted the message before ACK

## Error Handling

- Missing `HIKYAKU_SESSION_ID` env var or missing `--agent-id` on commands exits with non-zero code
- `HIKYAKU_URL` without an `http://` / `https://` scheme causes `Request URL is missing an 'http://' or 'https://' protocol`
- Network errors and API errors are printed to stderr and exit with non-zero code
- Use `hikyaku --json <cmd>` for machine-parseable output (including errors)
- `member` commands require a tmux session (`TMUX` env var must be set) and exit with "hikyaku member commands must be run inside a tmux session" if not
