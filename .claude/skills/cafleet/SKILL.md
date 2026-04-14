---
description: Interact with the CAFleet message broker. Use when an agent needs to register, send/receive messages, poll inbox, acknowledge messages, or discover other agents.
---

# CAFleet — Message Broker CLI

Use the `cafleet` CLI to register as an agent, send and receive messages, and discover other agents on the CAFleet message broker. CLI commands access SQLite directly — no running server is required.

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

## Required Flags

Every `cafleet` invocation that touches agents or messages must carry two literal UUIDs as flags. There is no env-var fallback.

| Flag | Scope | Required for | Notes |
|---|---|---|---|
| `--session-id <uuid>` | global (placed **before** the subcommand) | every client + member subcommand (`register`, `send`, `broadcast`, `poll`, `ack`, `cancel`, `get-task`, `agents`, `deregister`, `member *`) | UUID of the session created via `cafleet session create`. Silently accepted (and ignored) on `db init` / `session *`. |
| `--agent-id <uuid>` | per-subcommand (placed **after** the subcommand name) | every subcommand **except** `register` | The acting agent's UUID. `register` returns the new `agent_id` — record it and pass it to every subsequent command. |

If `--session-id` is missing on a subcommand that needs it, the CLI exits with `Error: --session-id <uuid> is required for this subcommand. Create a session with 'cafleet session create' and pass its id.`

> **Why literal flags, not env vars?** Claude Code's `permissions.allow` matches Bash invocations as literal command strings. A literal `cafleet --session-id <uuid> <subcmd> --agent-id <uuid>` invocation matches a single allow pattern across every subcommand for that session. Shell-expansion patterns (`export VAR=...` then `$VAR`) break that matching and force per-invocation permission prompts that interrupt agent loops. Substitute the literal UUIDs printed by `cafleet session create` and `cafleet register` — never store them in shell variables.

The only environment variable the CLI still reads is:

- `CAFLEET_DATABASE_URL` — SQLite database URL (optional; default builds `sqlite:///<path>` from `~/.local/share/cafleet/registry.db` with `~` expanded at load time). When setting `CAFLEET_DATABASE_URL` yourself, use an absolute path — SQLAlchemy does not expand `~` in SQLite URLs.

## Placeholder convention used below

In every example below, substitute the literal UUID strings printed by `cafleet session create` / `cafleet register`. Angle-bracket tokens are placeholders, **not** shell variables:

- `<session-id>` — the session UUID printed by `cafleet session create`
- `<my-agent-id>` — the UUID returned by your own `cafleet ... register` call
- `<director-agent-id>` — the Director's UUID (handed to you in your spawn prompt if you are a member)
- `<member-agent-id>` — a target member's UUID (from `member create` / `member list`)
- `<target-agent-id>` — the recipient of a unicast message
- `<task-id>` — the task UUID printed by `poll` / `send`

## Global Options

Only `--json` and `--session-id` are global (before the subcommand). `--agent-id` is a per-subcommand option and must appear **after** the subcommand name:

```bash
cafleet --session-id <session-id> --json register --name "My Agent" --description "..."
cafleet --session-id <session-id> --json agents --agent-id <my-agent-id>
```

`cafleet agents --json` will fail with `No such option: --json`. Same for `--session-id` placed after the subcommand — keep it before. `--agent-id` must come **after** the subcommand, not before it.

## Command Reference

### Register

Register a new agent with the broker.

```bash
cafleet --session-id <session-id> register \
  --name "My Agent" --description "What this agent does"

cafleet --session-id <session-id> register \
  --name "My Agent" --description "Frontend dev" \
  --skills '[{"id":"react","name":"React Dev","description":"React/TS"}]'
```

Returns the newly created `agent_id`. Record it; every other command needs it via `--agent-id` (placed after the subcommand name).

#### Self-registration recipe

Use `--json` so the output is machine-parseable, and capture `agent_id` for every subsequent call:

```bash
cafleet --session-id <session-id> --json register \
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
- Call `cafleet --session-id <session-id> deregister --agent-id <my-agent-id>` at end of session so stale registrations do not accumulate.

### List Agents

List all registered agents, or get detail for a specific agent.

```bash
cafleet --session-id <session-id> agents --agent-id <my-agent-id>
cafleet --session-id <session-id> agents --agent-id <my-agent-id> --id <target-agent-id>
```

### Send (Unicast)

Send a message to a specific agent by ID.

```bash
cafleet --session-id <session-id> send --agent-id <my-agent-id> \
  --to <target-agent-id> --text "Did the API schema change?"
```

After persisting the message, the broker attempts a tmux push notification to the recipient's pane (`tmux send-keys` with `cafleet --session-id <session-id> poll --agent-id <recipient-id>`). The notification is skipped when: the sender is the recipient (self-send), the recipient has no placement row or no `tmux_pane_id`, the pane is dead, or `tmux` is not on `PATH`. The message is always available in the queue regardless of notification outcome.

### Broadcast

Send a message to all registered agents (except self).

```bash
cafleet --session-id <session-id> broadcast --agent-id <my-agent-id> \
  --text "Build failed on main branch"
```

After persisting each delivery, the broker attempts a tmux push notification per recipient. The broadcast summary response includes `notifications_sent_count` indicating how many panes were successfully triggered. Self-sends and missing/dead panes are skipped silently.

### Poll (Check Inbox)

Poll for incoming messages. Returns tasks addressed to this agent.

```bash
cafleet --session-id <session-id> poll --agent-id <my-agent-id>
cafleet --session-id <session-id> poll --agent-id <my-agent-id> --since "2026-03-28T12:00:00Z"
cafleet --session-id <session-id> poll --agent-id <my-agent-id> --page-size 10
```

### Acknowledge (ACK)

Acknowledge receipt of a message. Moves the task from INPUT_REQUIRED to COMPLETED.

```bash
cafleet --session-id <session-id> ack --agent-id <my-agent-id> --task-id <task-id>
```

### Cancel (Retract)

Cancel a sent message that hasn't been acknowledged yet. Only the sender can cancel.

```bash
cafleet --session-id <session-id> cancel --agent-id <my-agent-id> --task-id <task-id>
```

### Get Task

Get details of a specific task by ID.

```bash
cafleet --session-id <session-id> get-task --agent-id <my-agent-id> --task-id <task-id>
```

### Deregister

Remove this agent's registration from the broker.

```bash
cafleet --session-id <session-id> deregister --agent-id <my-agent-id>
```

### Member Create

Register a new member agent and spawn a coding agent pane in the Director's own tmux window. Must be run inside a tmux session. The command atomically registers the agent, creates a placement row, spawns the pane, and patches the placement with the real pane ID.

```bash
cafleet --session-id <session-id> member create --agent-id <director-agent-id> \
  --name Claude-B --description "Reviewer for PR #42"

cafleet --session-id <session-id> member create --agent-id <director-agent-id> \
  --name Codex-B --description "Reviewer for PR #42" --coding-agent codex

cafleet --session-id <session-id> member create --agent-id <director-agent-id> \
  --name Claude-B --description "Reviewer for PR #42" \
  -- "Review PR #42, post feedback via send, and deregister on completion."
```

| Flag | Required | Notes |
|---|---|---|
| `--agent-id` | yes | The Director's agent ID |
| `--name` | yes | Display name of the new member |
| `--description` | yes | One-sentence purpose |
| `--coding-agent` | no | Coding agent to spawn: `claude` (default) or `codex`. Codex is spawned with `--approval-mode auto-edit`. |
| *(positional, after `--`)* | no | Prompt for the spawned coding agent process. If omitted, a default prompt is generated (agent-specific). BOTH the default template and any custom prompt go through `str.format()` with `session_id` / `agent_id` / `director_name` / `director_agent_id` as kwargs, so callers may embed those placeholders in custom prompts and have the new member's literal UUIDs substituted in. |

**Template safety**: because custom prompts go through `str.format()`, any literal `{` or `}` in the prompt text must be doubled (`{{` / `}}`) to survive formatting — `.format()` collapses each `{{` / `}}` pair to a single literal brace and, critically, does not attempt placeholder substitution on the inner tokens. This matters for prompts that embed JSON snippets, shell expansions, or other content with literal curly braces. If doubling is impractical (e.g., a large pasted blob), pre-substitute the dynamic values in shell and pass a fully-resolved prompt without placeholders.

If the tmux `split-window` fails, the registered agent is rolled back. If the placement PATCH fails, the pane is `/exit`'d and the agent rolled back.

Output (text):
```
Member registered and spawned.
  agent_id:  <new-uuid>
  name:      Claude-B
  backend:   claude
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
    "coding_agent": "claude",
    "created_at": "2026-04-12T10:15:00Z"
  }
}
```

### Member Delete

Deregister a member agent and close its tmux pane. The agent is deregistered FIRST, then `/exit` is sent to the pane — so a deregister failure leaves both intact for retry.

```bash
cafleet --session-id <session-id> member delete --agent-id <director-agent-id> \
  --member-id <member-agent-id>
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
cafleet --session-id <session-id> member list --agent-id <director-agent-id>
cafleet --session-id <session-id> --json member list --agent-id <director-agent-id>
```

| Flag | Required | Notes |
|---|---|---|
| `--agent-id` | yes | The Director's agent ID |

Output columns: `agent_id`, `name`, `status`, `backend`, `session`, `window_id`, `pane_id`, `created_at`. The `backend` column shows which coding agent is running (`claude` or `codex`). A pending placement (pane not yet spawned) shows `(pending)` for `pane_id` in text mode and `null` in JSON.

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
      "coding_agent": "claude",
      "created_at": "2026-04-12T10:15:00Z"
    }
  }
]
```

### Member Capture

Capture the last N lines of a member's tmux pane terminal buffer. This is the canonical way to inspect a stalled teammate — it replaces raw `tmux capture-pane` invocations for any project using CAFleet.

```bash
cafleet --session-id <session-id> member capture --agent-id <director-agent-id> \
  --member-id <member-agent-id>

cafleet --session-id <session-id> member capture --agent-id <director-agent-id> \
  --member-id <member-agent-id> --lines 200

cafleet --session-id <session-id> --json member capture --agent-id <director-agent-id> \
  --member-id <member-agent-id> | jq -r .content
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

**Note**: Projects using CAFleet use `Skill(cafleet-monitoring)` instead of the generic `agent-team-supervision` skill. The cafleet-monitoring skill uses `cafleet member capture` exclusively (no raw `tmux capture-pane`), enforcing the cross-Director boundary.

## Typical Workflow

1. **Create a session** (if one does not already exist):
   ```bash
   cafleet session create --label "my-project"
   # → prints the session_id, e.g. 550e8400-e29b-41d4-a716-446655440000
   ```
   Capture the printed UUID and substitute it for `<session-id>` in every command below.

2. **Register** with the broker:
   ```bash
   cafleet --session-id <session-id> register \
     --name "Code Review Agent" --description "Reviews pull requests"
   # → returns <my-agent-id>, e.g. 7ba91234-5678-90ab-cdef-112233445566
   ```

3. **Discover** other agents:
   ```bash
   cafleet --session-id <session-id> agents --agent-id <my-agent-id>
   ```

4. **Send** a message to another agent:
   ```bash
   cafleet --session-id <session-id> send --agent-id <my-agent-id> \
     --to <target-agent-id> --text "Please review PR #42"
   ```

5. **Poll** for incoming messages:
   ```bash
   cafleet --session-id <session-id> poll --agent-id <my-agent-id>
   ```

6. **Acknowledge** received messages:
   ```bash
   cafleet --session-id <session-id> ack --agent-id <my-agent-id> --task-id <task-id>
   ```

7. **Repeat** steps 4-6 as needed. Use `cafleet --session-id <session-id> --json <cmd>` when parsing output programmatically.

## Multi-Session Coordination

### Roles

- **Director** — the Claude Code session that first runs `cafleet --session-id <session-id> register` in this project. It owns the team lifecycle: spawning members, driving the exchange, and cleaning up.
- **Member** — any peer Claude Code session the Director spawns via `cafleet ... member create`. Each member is automatically registered, and its spawn prompt has the literal `session_id` and `agent_id` UUIDs baked in so every `cafleet` command it issues uses literal flags.

### Monitoring mandate (Director only)

Before spawning **any** member, the Director MUST load `Skill(cafleet-monitoring)` and start a `/loop` monitor as that skill instructs. Members do not act autonomously — if the Director stops supervising, the team stalls silently. Keep the `/loop` active until the final shutdown step.

To inspect a stalled member, follow the 2-stage health check in `Skill(cafleet-monitoring)`: first check `cafleet poll` for messages, then fall back to `cafleet member capture`:

```bash
cafleet --session-id <session-id> member capture --agent-id <director-agent-id> \
  --member-id <member-agent-id>
```

### Layout discipline

`cafleet member create` automatically maintains `main-vertical` layout:

- Director occupies the full-height left "main" pane.
- Every member is stacked in the right column at equal height.
- Every `member create` and `member delete` runs `tmux select-layout main-vertical` internally.

### Spawn a member

```bash
cafleet --session-id <session-id> member create --agent-id <director-agent-id> \
  --name Claude-B --description "Reviewer for PR #42"
```

The command handles everything atomically: registering the agent, baking the new member's literal `session_id` and `agent_id` UUIDs into the spawn prompt via `str.format()`, forwarding `CAFLEET_DATABASE_URL` (when set) to the new pane via `-e` flags, spawning `claude` with the prompt, and rebalancing the layout. No env-var injection is needed.

### Shut down a member

```bash
cafleet --session-id <session-id> member delete --agent-id <director-agent-id> \
  --member-id <member-agent-id>
```

The command deregisters the agent first (so a failure preserves the pane for retry), then sends `/exit` to the pane, then rebalances the layout.

After every member is shut down, the Director deregisters itself and stops the `/loop` monitor:

```bash
cafleet --session-id <session-id> deregister --agent-id <director-agent-id>
```

## Message Lifecycle

Messages are modeled as tasks with this lifecycle:
- **input_required** — Message delivered, waiting for recipient to ACK
- **completed** — Recipient acknowledged the message
- **canceled** — Sender retracted the message before ACK

## Error Handling

- Missing `--session-id` on a client/member subcommand exits with `Error: --session-id <uuid> is required for this subcommand. Create a session with 'cafleet session create' and pass its id.` (exit 1).
- Missing `--agent-id` on commands that need it exits with `Error: Missing option '--agent-id'.` (Click built-in).
- Errors are printed to stderr and exit with non-zero code.
- Use `cafleet --session-id <session-id> --json <cmd>` for machine-parseable output (including errors).
- `member` commands require a tmux session (`TMUX` env var must be set) and exit with "cafleet member commands must be run inside a tmux session" if not.
