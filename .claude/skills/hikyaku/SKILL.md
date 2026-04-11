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

The CLI reads both variables from the environment — they are the **only** way to configure the CLI. There are no `--url` / `--api-key` flags.

- `HIKYAKU_URL` — Broker URL, must include the `http://` / `https://` scheme (e.g. `http://localhost:8000`). The CLI errors with "Request URL is missing an 'http://' or 'https://' protocol" if the scheme is missing.
- `HIKYAKU_API_KEY` — API key created via the Hikyaku WebUI key-management page (format: `hky_` + 32 hex chars). Keys are shown only once at creation. The CLI exits with an error if this is not set.

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

Register a new agent with the broker. `HIKYAKU_API_KEY` must be set.

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

## Typical Workflow

1. **Register** with the broker (`HIKYAKU_API_KEY` must already be set; create the key in the Hikyaku WebUI first):
   ```bash
   hikyaku register --name "Code Review Agent" --description "Reviews pull requests"
   # → record the returned agent_id as $MY_ID
   ```

2. **Discover** other agents:
   ```bash
   hikyaku agents --agent-id $MY_ID
   ```

3. **Send** a message to another agent:
   ```bash
   hikyaku send --agent-id $MY_ID --to <target-agent-id> --text "Please review PR #42"
   ```

4. **Poll** for incoming messages:
   ```bash
   hikyaku poll --agent-id $MY_ID
   ```

5. **Acknowledge** received messages:
   ```bash
   hikyaku ack --agent-id $MY_ID --task-id <task-id>
   ```

6. **Repeat** steps 3-5 as needed. Use `hikyaku --json <cmd>` when parsing output programmatically.

## Multi-Session Coordination

### Roles

- **Director** — the Claude Code session that first runs `hikyaku register` in this project. It owns the team lifecycle: spawning members, driving the exchange, and cleaning up.
- **Member** — any peer Claude Code session the Director spawns in a tmux split-pane. Each member registers itself with `hikyaku`, runs its assigned exchange, and deregisters before exit.

### Monitoring mandate (Director only)

Before spawning **any** member, the Director MUST load `Skill(agent-team-supervision)` and start a `/loop` monitor as that skill instructs. Members do not act autonomously — if the Director stops supervising, the team stalls silently. Keep the `/loop` active until the final shutdown step.

### Layout discipline

Only the Director calls `tmux split-window`. The window is kept in `main-vertical` layout at all times:

- Director occupies the full-height left "main" pane.
- Every member is stacked in the right column at equal height.
- Every spawn and every shutdown is followed by `tmux select-layout main-vertical`, so the right column is always evenly divided regardless of how many members are currently active.

### Spawn a member

Spawning a member is a **two-step** sequence. Shell variable expansion in the `tmux split-window` call (e.g. `-e "HIKYAKU_URL=$HIKYAKU_URL"`) is unreliable under the Bash tool's permission validator, so the Director must first read the literal values with `printenv` and then paste them into the `-e` flags verbatim.

Step 1 — read the literal env values:

```bash
printenv HIKYAKU_URL HIKYAKU_API_KEY
```

This returns two lines: the URL on the first, the API key on the second. Capture both as plain strings.

Step 2 — spawn `claude` in a new pane with those literal values forwarded via `-e`, then rebalance the window to `main-vertical`:

```bash
tmux split-window \
  -e "HIKYAKU_URL=http://localhost:8000" \
  -e "HIKYAKU_API_KEY=hky_0123456789abcdef0123456789abcdef" \
  claude "Load Skill(hikyaku), register as Claude-B, send a round-trip ping to <director-agent-id>, poll for a reply, ack, and deregister."
tmux select-layout main-vertical
```

Substitute the literal strings from Step 1 for the two placeholders above. Do **not** write `$HIKYAKU_URL` / `$HIKYAKU_API_KEY` in the `tmux split-window` command — always paste the concrete values.

Rules:

- Always run `printenv HIKYAKU_URL HIKYAKU_API_KEY` first and paste the literal output into the `-e` flags. Shell variable expansion inside the `tmux split-window` Bash call is blocked by the validator in some configurations, and even when allowed it fails silently if the variable is unset in the Director's shell.
- Pass `claude "<prompt>"` as the trailing command of `tmux split-window`. Do **not** use `tmux send-keys` to type `claude` into the new pane — it races with shell startup and loses keystrokes.
- Call `tmux select-layout main-vertical` immediately after every `split-window`. Without it, the right column is not guaranteed to be evenly divided and may even push the Director off the left-main slot.
- `HIKYAKU_*` env vars in the current pane are NOT inherited by new panes (the tmux server uses its own frozen environment). Forwarding with `-e` is the only reliable way to give the member access to the broker.
- Project-level `.claude/settings.local.json` must already allow `Bash(hikyaku:*)`. Do **not** pass `--allowedTools` or `--permission-mode bypassPermissions` — rely on project settings, which the peer inherits by being launched in the same project directory.
- The prompt must be inline and self-contained. List the exact role, the target `agent_id`, and the expected cleanup. Do not stage the prompt through a temp file and do not write chat-style instructions.

### Shut down a member

After the member has deregistered from the broker, the Director shuts down its Claude Code session gracefully by sending `/exit` to the member's pane, then rebalances the layout:

```bash
tmux send-keys -t <member-pane-id> '/exit' Enter
tmux select-layout main-vertical
```

`/exit` quits `claude` cleanly, which in turn closes the tmux pane because `claude` was the pane's root process. The immediately-following `select-layout main-vertical` re-equalizes the remaining members in the right column. Do **not** use `tmux kill-pane` — that terminates the session abruptly and bypasses any in-flight cleanup inside the peer.

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

- Missing `HIKYAKU_API_KEY` env var or missing `--agent-id` on authenticated commands exits with non-zero code
- `HIKYAKU_URL` without an `http://` / `https://` scheme causes `Request URL is missing an 'http://' or 'https://' protocol`
- Network errors and API errors are printed to stderr and exit with non-zero code
- Use `hikyaku --json <cmd>` for machine-parseable output (including errors)
