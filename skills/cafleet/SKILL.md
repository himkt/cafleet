---
description: "Interact with the CAFleet message broker. Use when an agent needs to register, send/receive messages, poll inbox, acknowledge messages, or discover other agents."
---

# CAFleet — Message Broker CLI

Use the `cafleet` CLI to register as an agent, send and receive messages, and discover other agents on the CAFleet message broker. CLI commands access SQLite directly — no running server is required.

## Reference files

This file (the core) covers the identity / poll / send / ack / cancel / show lifecycle every agent uses. Director-only flows, broadcast semantics, the bash-via-Director fallback, recovery decision trees, and the `--full` opt-back-in live in dedicated reference files. Read on demand:

- For Director-only commands (`member create`, `member delete`, `member list --activity`, `member capture`, `member send-input`, `member exec`, `member ping`, plus the AskUserQuestion three-beat workflow), Read [`reference/director.md`](reference/director.md).
- For broadcast send/ack and threading via `origin_task_id`, Read [`reference/broadcast.md`](reference/broadcast.md).
- For the bash-via-Director fallback protocol (member-side reconsider-then-route, Director-side `member exec` dispatch, serialization, cross-fleet boundary), Read [`reference/exec-routing.md`](reference/exec-routing.md).
- For crash / disconnect / idle / wedged-pane recovery decision trees AND the Shutdown Protocol, Read [`reference/recovery.md`](reference/recovery.md).
- For `--full` / `--json` / `--quiet` opt-back-in semantics and `CAFLEET_MAX_TEXT_LEN`, Read [`reference/output-flags.md`](reference/output-flags.md).

If you are a member and your default Bash is denied on a specific command, the bash-via-Director fallback is in [`reference/exec-routing.md`](reference/exec-routing.md). If you are a Director, Read [`reference/director.md`](reference/director.md) before spawning your first member.

## When to Use

- Registering this agent with a fleet
- Sending a unicast message to another agent
- Polling for incoming messages
- Acknowledging received messages
- Canceling (retracting) a sent message
- Inspecting a single task by id
- Deregistering at end of fleet

For broadcast, member spawning, member capture, member ping, and member exec, see the reference files above.

## Required Flags

Every `cafleet` invocation that touches agents or messages must carry two literal integer ids as flags. There is no env-var fallback.

| Flag | Scope | Required for | Notes |
|---|---|---|---|
| `--fleet-id <int>` | global (placed **before** the subcommand) | every client + member subcommand (`register`, `send`, `broadcast`, `poll`, `ack`, `cancel`, `show`, `agent *`, `deregister`, `member *`) | Integer id of the fleet created via `cafleet fleet create`. Typed `int` — a non-integer fails with Click's standard "is not a valid integer" error. Silently accepted (and ignored) on `db init` / `fleet *` / `server` / `doctor`. |
| `--agent-id <int>` | per-subcommand (placed **after** the subcommand name) | every subcommand **except** `register` | The acting agent's integer id. `register` returns the new `agent_id` — record it and pass it to every subsequent command. |

If `--fleet-id` is missing on a subcommand that needs it, the CLI exits with `Error: --fleet-id <int> is required for this subcommand. Create a fleet with 'cafleet fleet create' and pass its id.`

> **Why literal flags, not env vars?** `permissions.allow` matches Bash invocations as literal command strings — a literal `cafleet --fleet-id <int> <subcmd> --agent-id <int>` matches one allow pattern, while shell-expansion (`export VAR=...` then `$VAR`) breaks the match and forces per-invocation permission prompts. Substitute the literal ids printed by `cafleet fleet create` and `cafleet agent register`; never store them in shell variables.

The environment variables the CLI reads (all wired through `cafleet.config.Settings` via explicit `validation_alias` on each field, so the `CAFLEET_` prefix is uniform):

- `CAFLEET_DATABASE_URL` — SQLite database URL (optional; default builds `sqlite:///<path>` from `~/.local/share/cafleet/cafleet.db`). Use an absolute path when overriding — SQLAlchemy does not expand `~` in SQLite URLs.
- `CAFLEET_BROKER_HOST` / `CAFLEET_BROKER_PORT` — defaults for `cafleet server` (`127.0.0.1` / `8000`).
- `CAFLEET_MAX_TEXT_LEN` — body truncation codepoint limit (default `200`); see [`reference/output-flags.md`](reference/output-flags.md).

## Placeholder convention

In every example below, substitute the literal integer ids printed by `cafleet fleet create` / `cafleet agent register`. Angle-bracket tokens are placeholders, **not** shell variables:

- `<fleet-id>` — the fleet id printed by `cafleet fleet create`
- `<my-agent-id>` — the id returned by your own `cafleet ... agent register` call
- `<director-agent-id>` — the Director's id (in your spawn prompt if you are a member)
- `<target-agent-id>` — the recipient of a unicast message
- `<task-id>` — the task id printed by `message poll` / `message send`

> **Ids are integers**: every id input (`--fleet-id`, `--agent-id`, `--to`, `--id`, `--member-id`, `--task-id`) is a DB-assigned integer, typically 1–4 digits. Each is typed `int` and is passed in full — there is no prefix resolution. A non-integer fails with Click's standard `Error: Invalid value for '...': '<x>' is not a valid integer.` (exit 2).

## Global Options

Only `--json`, `--fleet-id`, and `--version` are global (before the subcommand). `--agent-id` is a per-subcommand option and must appear **after** the subcommand name:

```bash
cafleet --fleet-id <fleet-id> --json agent register --name "My Agent" --description "..."
cafleet --fleet-id <fleet-id> --json agent list
cafleet --fleet-id <fleet-id> --json message poll --agent-id <my-agent-id>
```

`cafleet agent list --json` will fail with `No such option: --json`. Same for `--fleet-id` placed after the subcommand — keep it before. `--agent-id` must come **after** the subcommand, not before it.

`cafleet --version` prints `cafleet <version>` and exits 0 without `--fleet-id`.

## Coding-agent backends

Three backends are supported: `claude` (default), `codex`, and `opencode`. The Director picks per member at create time via `--coding-agent {claude,codex,opencode}`. An optional `--model <m>` on `member create` pins the member's LLM (pass-through for `claude` / `codex`; `<provider-id>/<model-id>` format required for `opencode`); the flag detail and the model-name-to-backend inference table for resolving requests like "create a member with sonnet" live in [`reference/director.md`](reference/director.md). All three backends honor a leading-`!` shortcut on the input line, so `cafleet member exec` and message-send inline previews work uniformly. Operational details for codex members live in [`docs/reference/coding-agents/codex.md`](../../docs/reference/coding-agents/codex.md); the opencode equivalent (including the `CAFLEET_AGENT` preset materialization at `~/.opencode/agents/cafleet.md` on first spawn and the refresh recipe) lives in [`docs/reference/coding-agents/opencode.md`](../../docs/reference/coding-agents/opencode.md).

## Self-registration recipe

Use `--json` so the output is machine-parseable, and capture `agent_id` for every subsequent call:

```bash
cafleet --fleet-id <fleet-id> --json agent register \
  --name "<short-label>" \
  --description "<one-sentence purpose>"
```

JSON response (field order is not guaranteed):

```json
{"agent_id":<id>,"name":"<short-label>","registered_at":"<iso8601>"}
```

Rules:

- **Name**: short, human-identifiable label (`Claude-A`, `reviewer-bot`, …). Not `test`, `foo`, etc.
- **Description**: one sentence stating who the agent is and what it is for.
- **Capture `agent_id` immediately.** It is required for every subsequent call; losing it forces re-registration.
- Non-`--json` output prints `Agent registered successfully!` followed by `  agent_id:  <id>` and `  name:      <name>`. Parse the `agent_id:` line if `--json` is not an option.
- Call `cafleet --fleet-id <fleet-id> agent deregister --agent-id <my-agent-id>` at end of fleet so stale registrations do not accumulate.

> **Reserved name — `Administrator`**: every fleet is auto-seeded with exactly one built-in `Administrator` agent at `fleet create` time. Do NOT register a human or member agent under the name `Administrator`. The built-in Administrator is marked internally via `agent_card_json.cafleet.kind == "builtin-administrator"` and is protected against deregister and Director placement (see Deregister below).

## Send (Unicast)

Send a message to a specific agent by ID.

```bash
cafleet --fleet-id <fleet-id> message send --agent-id <my-agent-id> \
  --to <target-agent-id> --text "Did the API schema change?"
```

| Flag | Required | Notes |
|---|---|---|
| `--to <agent-id>` | yes | Recipient agent id (integer). |
| `--text <body>` | yes | Message body. Truncated to `CAFLEET_MAX_TEXT_LEN` codepoints + `…` in the echoed response by default. |
| `--full` | no | Disable body truncation; emit the full typed-column envelope. See [`reference/output-flags.md`](reference/output-flags.md). |
| `--quiet` | no | Emit only the new task id. Useful in scripted loops. |

After persisting the message, the broker keystrokes a 2-line inline preview into the recipient's pane via `tmux.send_inline_preview`:

```
[cafleet msg <task_id> from <sender_id> <ts>]
<text-truncated-to-CAFLEET_MAX_TEXT_LEN>
```

The recipient's coding agent processes the keystroked text as a fresh user-turn input — no `cafleet message poll` invocation is in the auto-fire path. The recipient acks via `cafleet message ack --task-id <task_id>` once it has consumed the message. The notification is skipped when: the sender is the recipient (self-send), the recipient has no placement row or no `tmux_pane_id`, the pane is dead, or `tmux` is not on `PATH`. The message is always available in the queue regardless of notification outcome — recipients that miss an inline preview catch up on their next manual `message poll` (or via a Director-issued `cafleet member ping`).

## Poll (Check Inbox)

Poll for incoming messages. Returns only un-acked (`input_required`) deliveries addressed to this agent, newest first — once a task is ACKed it no longer appears in `poll` output.

```bash
cafleet --fleet-id <fleet-id> message poll --agent-id <my-agent-id>
cafleet --fleet-id <fleet-id> message poll --agent-id <my-agent-id> --full
```

| Flag | Required | Notes |
|---|---|---|
| `--full` | no | Disable body truncation; emit the full typed-column envelope for every task. |

## Acknowledge (ACK)

Acknowledge receipt of a message. Moves the task from `INPUT_REQUIRED` to `COMPLETED`.

```bash
cafleet --fleet-id <fleet-id> message ack --agent-id <my-agent-id> --task-id <task-id>
cafleet --fleet-id <fleet-id> message ack --agent-id <my-agent-id> --task-id <task-id> --quiet
```

| Flag | Required | Notes |
|---|---|---|
| `--task-id <int>` | yes | Task to acknowledge. |
| `--full` | no | Disable body truncation in the echoed task. |
| `--quiet` | no | Emit only the acked task id. |

## Cancel (Retract)

Cancel a sent message that has not been acknowledged yet. Only the sender can cancel.

```bash
cafleet --fleet-id <fleet-id> message cancel --agent-id <my-agent-id> --task-id <task-id>
```

| Flag | Required | Notes |
|---|---|---|
| `--task-id <int>` | yes | Task to cancel. |
| `--full` | no | Disable body truncation in the echoed task. |

## Show (Get Task)

Get details of a specific task by ID.

```bash
cafleet --fleet-id <fleet-id> message show --agent-id <my-agent-id> --task-id <task-id>
```

| Flag | Required | Notes |
|---|---|---|
| `--task-id <int>` | yes | Task to fetch. |
| `--full` | no | Disable body truncation; emit the full typed-column envelope. |

## List Agents

`agent list` returns all registered agents in the fleet. To fetch detail for a single agent, use `agent show --id <target-agent-id>`.

```bash
cafleet --fleet-id <fleet-id> agent list
cafleet --fleet-id <fleet-id> agent show --agent-id <my-agent-id> --id <target-agent-id>
```

Default output is one row per agent (`<id> <name> <status>`); `description` is truncated to 60 codepoints. Pass `--full` for the four-line per-agent block (full `agent_id`, `name`, `description` still truncated to 60, `status`); the agent surfaces do not carry `agent_card_json`, so `--full` does not expose it — see [`reference/output-flags.md`](reference/output-flags.md).

## Doctor

Print the calling pane's tmux session/window/pane identifiers (plus `$TMUX_PANE`) for operators diagnosing placement issues without reaching for raw tmux commands.

```bash
cafleet doctor
cafleet --json doctor
```

Does NOT require `--fleet-id`. Requires `TMUX` and `TMUX_PANE` env vars to be set (the standard tmux pane environment).

## Deregister

Remove this agent's registration from the broker.

```bash
cafleet --fleet-id <fleet-id> agent deregister --agent-id <my-agent-id>
```

> **Root Director cannot be deregistered**. The agent created by `cafleet fleet create` (the fleet's `fleets.director_agent_id`) is protected — `cafleet agent deregister --agent-id <root-director-id>` exits 1 with `Error: cannot deregister the root Director; use 'cafleet fleet delete' instead.` Use `cafleet fleet delete <fleet-id>` for fleet teardown.

> **Administrator cannot be deregistered**. Passing the built-in Administrator's `agent_id` to `cafleet agent deregister` exits 1 with `Error: Administrator cannot be deregistered`. The Administrator row stays `active`; there is no override flag. Every fleet has exactly one Administrator; deregister regular agents only.

## Fleet Delete

```bash
cafleet fleet delete <fleet-id>
# → Deleted fleet <fleet-id>. Deregistered N agents.
```

Soft-deletes a fleet in a single transaction: stamps `fleets.deleted_at`, deregisters every active agent in the fleet (root Director + Administrator + remaining members), and physically deletes every associated `agent_placements` row. Tasks are preserved. Idempotent.

After soft-delete, the fleet is hidden from `cafleet fleet list` and further `cafleet --fleet-id <deleted> agent register` calls fail with `Error: fleet <id> is deleted`. Surviving member coding-agent processes are **not** automatically closed — call `cafleet member delete` per member **before** `cafleet fleet delete` for a clean teardown. See the Shutdown Protocol in [`reference/recovery.md`](reference/recovery.md) for the full ordering.

## Typical Workflow

0. **Verify pane env** (Director / spawn-aware operator):
   ```bash
   cafleet doctor
   # tmux:
   #   session_name:  <name>
   #   window_id:     @<n>
   #   pane_id:       %<n>
   #   TMUX_PANE:     %<n>
   ```

   Confirms the calling shell has `TMUX` and `TMUX_PANE` set. Reach for this BEFORE `cafleet fleet create` and BEFORE any `cafleet member create` call — it is the canonical pane-identity probe, replacing raw `tmux display-message` and `TMUX` / `TMUX_PANE` env-var expansion. See § *Doctor* for the subcommand's `--fleet-id` and env-var requirements, plus the `--json` variant.

1. **Create a fleet** (if one does not already exist):
   ```bash
   cafleet fleet create --label "my-project"
   # text output line 1: <fleet-id>; line 2: <root-director-agent-id>
   cafleet fleet create --label "my-project" --json
   # JSON: { "fleet_id": "...", "director": {...}, "administrator_agent_id": "..." }
   ```

   Must be run inside a tmux session — outside tmux the command exits 1 with `Error: cafleet fleet create must be run inside a tmux session` and writes nothing.

2. **Register** with the broker:
   ```bash
   cafleet --fleet-id <fleet-id> agent register \
     --name "Code Review Agent" --description "Reviews pull requests"
   # → returns <my-agent-id>
   ```

3. **Discover** other agents:
   ```bash
   cafleet --fleet-id <fleet-id> agent list
   ```

4. **Send** a message:
   ```bash
   cafleet --fleet-id <fleet-id> message send --agent-id <my-agent-id> \
     --to <target-agent-id> --text "Please review PR #42"
   ```

5. **Poll** for incoming messages:
   ```bash
   cafleet --fleet-id <fleet-id> message poll --agent-id <my-agent-id>
   ```

6. **Acknowledge** received messages:
   ```bash
   cafleet --fleet-id <fleet-id> message ack --agent-id <my-agent-id> --task-id <task-id>
   ```

7. **Repeat** steps 4–6 as needed. Use `cafleet --fleet-id <fleet-id> --json <cmd>` when parsing output programmatically.

For Director-side spawn / capture / exec / ping flows, see [`reference/director.md`](reference/director.md). For shutdown ordering, see [`reference/recovery.md`](reference/recovery.md).

## Message Lifecycle

Messages are modeled as tasks with this lifecycle:

- **input_required** — Message delivered, waiting for recipient to ACK.
- **completed** — Recipient acknowledged the message.
- **canceled** — Sender retracted the message before ACK.

For broadcast threading (the `origin_task_id` self-reference shape), see [`reference/broadcast.md`](reference/broadcast.md).

## Error Handling

- Missing `--fleet-id` on a client/member subcommand exits with `Error: --fleet-id <int> is required for this subcommand. Create a fleet with 'cafleet fleet create' and pass its id.` (exit 1).
- Missing `--agent-id` on commands that need it exits with `Error: Missing option '--agent-id'.` (Click built-in, exit 2).
- Errors print to stderr and exit non-zero.
- Use `cafleet --fleet-id <fleet-id> --json <cmd>` for machine-parseable output (including errors).
- `member` commands require a tmux session (`TMUX` env var must be set) and exit 1 with `Error: cafleet member commands must be run inside a tmux session` if not.
