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
| `--session-id <uuid>` | global (placed **before** the subcommand) | every client + member subcommand (`register`, `send`, `broadcast`, `poll`, `ack`, `cancel`, `get-task`, `agents`, `deregister`, `member *`) | UUID of the session created via `cafleet session create`. Silently accepted (and ignored) on `db init` / `session *` / `server`. |
| `--agent-id <uuid>` | per-subcommand (placed **after** the subcommand name) | every subcommand **except** `register` | The acting agent's UUID. `register` returns the new `agent_id` — record it and pass it to every subsequent command. |

If `--session-id` is missing on a subcommand that needs it, the CLI exits with `Error: --session-id <uuid> is required for this subcommand. Create a session with 'cafleet session create' and pass its id.`

> **Why literal flags, not env vars?** Claude Code's `permissions.allow` matches Bash invocations as literal command strings. A literal `cafleet --session-id <uuid> <subcmd> --agent-id <uuid>` invocation matches a single allow pattern across every subcommand for that session. Shell-expansion patterns (`export VAR=...` then `$VAR`) break that matching and force per-invocation permission prompts that interrupt agent loops. Substitute the literal UUIDs printed by `cafleet session create` and `cafleet agent register` — never store them in shell variables.

The environment variables the CLI reads (all wired through `cafleet.config.Settings` via explicit `validation_alias` on each field, so the `CAFLEET_` prefix is uniform):

- `CAFLEET_DATABASE_URL` — SQLite database URL (optional; default builds `sqlite:///<path>` from `~/.local/share/cafleet/registry.db` with `~` expanded at load time). When setting `CAFLEET_DATABASE_URL` yourself, use an absolute path — SQLAlchemy does not expand `~` in SQLite URLs.
- `CAFLEET_BROKER_HOST` — Default bind address for `cafleet server` (optional; default `127.0.0.1`). Overridden by an explicit `cafleet server --host <addr>` flag.
- `CAFLEET_BROKER_PORT` — Default bind port for `cafleet server` (optional; default `8000`). Overridden by an explicit `cafleet server --port <int>` flag.

## Placeholder convention used below

In every example below, substitute the literal UUID strings printed by `cafleet session create` / `cafleet agent register`. Angle-bracket tokens are placeholders, **not** shell variables:

- `<session-id>` — the session UUID printed by `cafleet session create`
- `<my-agent-id>` — the UUID returned by your own `cafleet ... register` call
- `<director-agent-id>` — the Director's UUID (handed to you in your spawn prompt if you are a member)
- `<member-agent-id>` — a target member's UUID (from `member create` / `member list`)
- `<target-agent-id>` — the recipient of a unicast message
- `<task-id>` — the task UUID printed by `poll` / `send`

## Global Options

Only `--json`, `--session-id`, and `--version` are global (before the subcommand). `--agent-id` is a per-subcommand option and must appear **after** the subcommand name:

```bash
cafleet --session-id <session-id> --json register --name "My Agent" --description "..."
cafleet --session-id <session-id> --json agents --agent-id <my-agent-id>
```

`cafleet agent list --json` will fail with `No such option: --json`. Same for `--session-id` placed after the subcommand — keep it before. `--agent-id` must come **after** the subcommand, not before it.

### `--version`

`cafleet --version` prints `cafleet <version>` to stdout and exits 0. It works **without** `--session-id` — the option is registered eagerly, so Click runs its callback during option parsing and exits before the session-id guard on any subcommand is reached. The version string is sourced from the installed package metadata via `importlib.metadata.version("cafleet")`, so it stays in lock-step with `project.version` in `cafleet/pyproject.toml` with no manual bookkeeping. The flag does not honor `--json`; `cafleet --json --version` still prints the plain text form.

## Command Reference

### Register

Register a new agent with the broker.

```bash
cafleet --session-id <session-id> agent register \
  --name "My Agent" --description "What this agent does"

cafleet --session-id <session-id> agent register \
  --name "My Agent" --description "Frontend dev" \
  --skills '[{"id":"react","name":"React Dev","description":"React/TS"}]'
```

Returns the newly created `agent_id`. Record it; every other command needs it via `--agent-id` (placed after the subcommand name).

> **Reserved name — `Administrator`**: every session is auto-seeded with exactly one built-in `Administrator` agent at `session create` time. The name is not blocked at the CLI, but the built-in Administrator is marked internally via `agent_card_json.cafleet.kind == "builtin-administrator"` and is protected against deregister and Director placement (see Deregister). Do NOT register a human or member agent under the name `Administrator` — it will not gain the `builtin-administrator` kind and will only cause confusion in the WebUI. `cafleet session create --json` returns the Administrator's UUID in the `administrator_agent_id` field of the JSON response so callers that need to address it (e.g. sending from it in the Admin WebUI) can capture it immediately.

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
- Call `cafleet --session-id <session-id> agent deregister --agent-id <my-agent-id>` at end of session so stale registrations do not accumulate.

### Doctor

Print the calling pane's tmux session/window/pane identifiers (plus `$TMUX_PANE`) for operators diagnosing placement issues without reaching for raw tmux commands.

```bash
cafleet doctor
cafleet --json doctor
```

| Flag | Required | Notes |
|---|---|---|
| `--json` | no | Global `--json`, placed before the subcommand. |
| `--session-id` | no | Silently accepted and ignored, matching `db init` / `session *` / `server`. |

Environment requirements: `TMUX` must be set (outside tmux the command exits 1 with `Error: cafleet member commands must be run inside a tmux session`), and `TMUX_PANE` must be set (already required by `tmux.director_context()`).

Text output:

```
tmux:
  session_name:  main
  window_id:     @3
  pane_id:       %0
  TMUX_PANE:     %0
```

JSON output:

```json
{
  "tmux": {
    "session_name": "main",
    "window_id": "@3",
    "pane_id": "%0",
    "tmux_pane_env": "%0"
  }
}
```

### List Agents

List all registered agents, or get detail for a specific agent.

```bash
cafleet --session-id <session-id> agent list --agent-id <my-agent-id>
cafleet --session-id <session-id> agent list --agent-id <my-agent-id> --id <target-agent-id>
```

### Send (Unicast)

Send a message to a specific agent by ID.

```bash
cafleet --session-id <session-id> message send --agent-id <my-agent-id> \
  --to <target-agent-id> --text "Did the API schema change?"
```

After persisting the message, the broker attempts a tmux push notification to the recipient's pane (`tmux send-keys` with `cafleet --session-id <session-id> message poll --agent-id <recipient-id>`). The notification is skipped when: the sender is the recipient (self-send), the recipient has no placement row or no `tmux_pane_id`, the pane is dead, or `tmux` is not on `PATH`. The message is always available in the queue regardless of notification outcome.

### Broadcast

Send a message to all registered agents (except self).

```bash
cafleet --session-id <session-id> message broadcast --agent-id <my-agent-id> \
  --text "Build failed on main branch"
```

After persisting each delivery, the broker attempts a tmux push notification per recipient. The broadcast summary response includes `notifications_sent_count` indicating how many panes were successfully triggered. Self-sends and missing/dead panes are skipped silently.

### Poll (Check Inbox)

Poll for incoming messages. Returns tasks addressed to this agent.

```bash
cafleet --session-id <session-id> message poll --agent-id <my-agent-id>
cafleet --session-id <session-id> message poll --agent-id <my-agent-id> --since "2026-03-28T12:00:00Z"
cafleet --session-id <session-id> message poll --agent-id <my-agent-id> --page-size 10
```

### Acknowledge (ACK)

Acknowledge receipt of a message. Moves the task from INPUT_REQUIRED to COMPLETED.

```bash
cafleet --session-id <session-id> message ack --agent-id <my-agent-id> --task-id <task-id>
```

### Cancel (Retract)

Cancel a sent message that hasn't been acknowledged yet. Only the sender can cancel.

```bash
cafleet --session-id <session-id> message cancel --agent-id <my-agent-id> --task-id <task-id>
```

### Get Task

Get details of a specific task by ID.

```bash
cafleet --session-id <session-id> message show --agent-id <my-agent-id> --task-id <task-id>
```

### Deregister

Remove this agent's registration from the broker.

```bash
cafleet --session-id <session-id> agent deregister --agent-id <my-agent-id>
```

> **Root Director cannot be deregistered**. The agent created by `cafleet session create` (the session's `sessions.director_agent_id`) is protected — `cafleet agent deregister --agent-id <root-director-id>` exits 1 with `Error: cannot deregister the root Director; use 'cafleet session delete' instead.` This guard exists because removing the root Director would orphan `sessions.director_agent_id`, break Member → Director push notifications, and leave no supported teardown path. Use `cafleet session delete <session-id>` for session teardown — it deregisters the root Director along with every member and Administrator as part of its cascade.

> **Administrator cannot be deregistered**. Passing the built-in Administrator's `agent_id` to `cafleet agent deregister` exits with status 1 and prints `Error: Administrator cannot be deregistered` to stderr — the broker raises `AdministratorProtectedError` and the CLI handles it by printing the error and calling `ctx.exit(1)`. The Administrator row stays `active`; there is no override flag. The same guard applies to `member create` — the built-in Administrator cannot be used as a Director (its `agent_id` cannot appear in `placement.director_agent_id`). Every session has exactly one Administrator; deregister regular agents only.

### Session Delete

```bash
cafleet session delete <session-id>
# → Deleted session <session-id>. Deregistered N agents.
```

Soft-deletes a session in a single transaction: stamps `sessions.deleted_at`, deregisters every active agent in the session (root Director + Administrator + any remaining members), and physically deletes every associated `agent_placements` row. Tasks are preserved. The command is idempotent — re-running against an already-deleted session prints `Deregistered 0 agents.` and exits 0.

After soft-delete, the session is hidden from `cafleet session list` and further `cafleet --session-id <deleted> agent register` calls fail with `Error: session <id> is deleted`. Surviving member `claude` processes are **not** automatically closed — call `cafleet member delete` per member **before** `cafleet session delete` for a clean teardown. See the Shutdown Protocol under "Multi-Session Coordination" for the full ordering — raw `tmux` commands are NOT part of the recovery path.

### Member Create

Register a new member agent and spawn a claude pane in the Director's own tmux window. Must be run inside a tmux session. The command atomically registers the agent, creates a placement row, spawns the pane, and patches the placement with the real pane ID.

```bash
cafleet --session-id <session-id> member create --agent-id <director-agent-id> \
  --name Claude-B --description "Reviewer for PR #42"

cafleet --session-id <session-id> member create --agent-id <director-agent-id> \
  --name Claude-B --description "Reviewer for PR #42" --allow-bash

cafleet --session-id <session-id> member create --agent-id <director-agent-id> \
  --name Claude-B --description "Reviewer for PR #42" \
  -- "Review PR #42, post feedback via cafleet message send, and deregister on completion."
```

| Flag | Required | Notes |
|---|---|---|
| `--agent-id` | yes | The Director's agent ID |
| `--name` | yes | Display name of the new member |
| `--description` | yes | One-sentence purpose |
| `--no-bash` / `--allow-bash` | no | Enable / disable Bash tool denial at spawn time. Defaults to `--no-bash` (the spawned process gains `--disallowedTools "Bash"` so the harness rejects every Bash call). The member is expected to route shell commands through its Director via the `bash_request` JSON envelope — see the `## Routing Bash via the Director` section below. `--allow-bash` is the opt-out for one-off members that need direct Bash; the soft "route through the Director" discipline still applies but is no longer enforced by the harness. |
| *(positional, after `--`)* | no | Prompt for the spawned claude process. If omitted, the default prompt template is used. BOTH the default template and any custom prompt go through `str.format()` with `session_id` / `agent_id` / `director_name` / `director_agent_id` as kwargs, so callers may embed those placeholders in custom prompts and have the new member's literal UUIDs substituted in. |

**Template safety**: because custom prompts go through `str.format()` whether or not they contain placeholders, any literal `{` or `}` in the prompt text must be doubled (`{{` / `}}`) — `.format()` collapses each `{{` / `}}` pair to a single literal brace and, critically, does not attempt placeholder substitution on the inner tokens. This matters for prompts that embed JSON snippets, shell expansions, or other content with literal curly braces. Pre-substituting the dynamic values in shell does NOT exempt the prompt from this rule — even a placeholder-free prompt is still passed through `str.format()`, so any literal braces must still be doubled or removed.

**Pane title**: the `--name` flag is forwarded to the spawned process as `claude --name <member-name> <prompt>`, so the tmux pane title (`#{pane_title}`) shows the member name internally. Operators should locate a specific member's pane via `cafleet member list --agent-id <director-agent-id>` (the output column `pane_id` carries the same pane identifier without requiring any raw `tmux` command).

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

The CLI sends `/exit`, polls `tmux list-panes` for the target `pane_id` until it disappears (15 s timeout), then deregisters the agent and rebalances the layout. On timeout, the pane buffer tail is captured and printed on stderr, and the command exits 2 without deregistering. Rerun with `--force` to skip `/exit` and kill the pane immediately.

```bash
cafleet --session-id <session-id> member delete --agent-id <director-agent-id> \
  --member-id <member-agent-id>

cafleet --session-id <session-id> member delete --agent-id <director-agent-id> \
  --member-id <member-agent-id> --force
```

| Flag | Required | Notes |
|---|---|---|
| `--agent-id` | yes | The Director's agent ID (used for the cross-Director authorization check) |
| `--member-id` | yes | The target member's agent ID |
| `--force` / `-f` | no | Skip the `/exit` wait. Immediately kill-pane the target, then deregister, then rebalance layout. Exit 0 even if the pane was already gone. |

Exit codes:

| Exit | When |
|---|---|
| `0` | Success — default path pane-gone confirmed, `--force` pane killed, or pending-placement deregister. |
| `1` | Any non-timeout failure: auth rejection, missing session, unknown member-id, `broker.deregister_agent` failure, `send_exit` tmux failure (pre-poll), `tmux.wait_for_pane_gone` raising TmuxError (server crash mid-poll). |
| `2` | Default-path timeout — `/exit` was sent, the pane did not disappear within 15.0 s, buffer tail has been printed on stderr. |

Cross-Director delete is rejected: the CLI verifies `placement.director_agent_id` matches `--agent-id` before calling `broker.deregister_agent` or sending `/exit` to the pane. An attempt to delete another Director's member in the same session exits 1 with `Error: agent <member-id> is not a member of your team (director_agent_id=<other-director>).` (mirrors `member capture` / `member send-input`).

Output (text, happy path):
```
Member deleted.
  agent_id:  <target-uuid>
  pane_id:   %7 (closed)
```

Output (text, `--force`):
```
Member deleted (--force).
  agent_id:  <target-uuid>
  pane_id:   %7 (killed)
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

Output columns: `agent_id`, `name`, `status`, `backend`, `session`, `window_id`, `pane_id`, `created_at`. The `backend` column shows `claude`. A pending placement (pane not yet spawned) shows `(pending)` for `pane_id` in text mode and `null` in JSON.

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

### Member Send-Input

Safely forward a restricted keystroke to a member's tmux pane. This is the write-path companion to `member capture` — designed for answering an `AskUserQuestion` prompt (or any prompt with the same "3 choices + Type something" shape) rendered in a member's Claude Code pane. Exactly one of `--choice` / `--freetext` must be supplied.

```bash
cafleet --session-id <session-id> member send-input --agent-id <director-agent-id> \
  --member-id <member-agent-id> --choice 1

cafleet --session-id <session-id> member send-input --agent-id <director-agent-id> \
  --member-id <member-agent-id> --freetext "please prioritize correctness"

cafleet --session-id <session-id> --json member send-input --agent-id <director-agent-id> \
  --member-id <member-agent-id> --choice 2
```

| Flag | Required | Notes |
|---|---|---|
| `--agent-id` | yes | The Director's agent ID (used for the cross-Director authorization check) |
| `--member-id` | yes | The target member's agent ID |
| `--choice` | one-of | Integer `1`, `2`, or `3`. Sends the matching digit key to the pane (no Enter). Validated via `click.IntRange(1, 3)`. |
| `--freetext` | one-of | Free-text string to type into the "Type something" field. Sends `4`, then the literal text via `tmux send-keys -l`, then `Enter`. Newlines are rejected. |

Cross-Director send is rejected: the CLI verifies `placement.director_agent_id` matches `--agent-id` before making any tmux call (same wording as `member capture`). A missing placement row, a pending pane (`tmux_pane_id is None`), or an unavailable `tmux` binary each exit 1 with a dedicated message.

**Why three tmux calls for `--freetext`**: tmux's `-l` (literal) flag is per-invocation — one `send-keys` call cannot mix literal characters with the `Enter` key name. Splitting the sequence into three calls (`4` → `-l "<text>"` → `Enter`) guarantees shell meta (`$VAR`, backticks, `$(...)`), key names embedded in the text (`Enter`, `C-c`, `Esc`), backslash-escapes, and multi-byte characters are all delivered as plain characters. The CLI calls `subprocess.run([...], shell=False)`, so no shell ever interprets the text. Newlines in `--freetext` are rejected with `Error: free text may not contain newlines.` (exit 2) because a literal newline would submit a second prompt without a following Enter — the single-action contract is "one CLI call = one prompt submission."

The CLI never inspects `placement.coding_agent`.

Output (text):

```
Sent choice 1 to member Claude-B (%7).
Sent free text to member Claude-B (%7).
```

Output (`--json`):

```json
{
  "member_agent_id": "<uuid>",
  "pane_id": "%7",
  "action": "choice",
  "value": "1"
}
```

```json
{
  "member_agent_id": "<uuid>",
  "pane_id": "%7",
  "action": "freetext",
  "value": "<user text as-sent>"
}
```

### Member Exec

Director-side helper that runs a single shell command with deterministic limits (64 KiB stdout/stderr caps, SIGKILL at the requested timeout) and prints exactly one JSON object on its own stdout. Invoked through the Director's own Bash tool when dispatching a `bash_request` from a member — see `## Routing Bash via the Director` above for the full dispatch flow.

```bash
cafleet member exec --cmd 'git log -1 --oneline'
cafleet member exec --cmd 'mise //cafleet:test' --timeout 120
cafleet member exec --cmd 'cat' --stdin 'hello'
cafleet member exec --cmd 'echo $PWD' --cwd /tmp
```

The helper does NOT know about CAFleet messaging, `permissions.allow`, or the `bash_request` envelope. It is a pure subprocess-runner with deterministic limits and input validation. The matcher and dispatch logic live in the Director's prompt-side workflow.

`cafleet member exec` does NOT require `--session-id` (consistent with `db init` / `session *` / `server`). Supplying `--session-id` is silently accepted and ignored, so a single `cafleet --session-id <id> *` allow pattern keeps matching every subcommand.

| Flag | Required | Notes |
|---|---|---|
| `--cmd <text>` | yes | Single-string shell command. Invoked via `subprocess.run(["bash", "-c", cmd], ...)` so pipes / `&&` / quoting work as the operator typed them. Empty string is rejected as a denied JSON output (input validation). |
| `--cwd <path>` | no | Working directory. Defaults to the helper's own cwd when omitted. A nonexistent path surfaces through the runtime path: `status: "ran"`, non-zero `exit_code`, `stderr` contains `no such cwd: <path>`. |
| `--timeout <int>` | no | Wall-clock seconds. Default 30. Capped at 600. Values above the cap are rejected as a denied JSON output. |
| `--stdin <text>` | no | UTF-8 text passed on the subprocess's stdin. |

Output schema (printed on stdout):

```json
{
  "status": "ran",
  "exit_code": 0,
  "stdout": "abc1234 docs: mark design doc as complete\n",
  "stderr": "",
  "duration_ms": 47
}
```

| Field | Type | Notes |
|---|---|---|
| `status` | `"ran" \| "denied" \| "timeout"` | Top-level outcome. **Sole source of truth.** Clients MUST switch on `status`, not on `exit_code`. |
| `exit_code` | `int` | When `status == "ran"`: the subprocess's verbatim exit code. When `status == "denied"` or `"timeout"`: **opaque**. |
| `stdout` | `string` | UTF-8, capped at 64 KiB. Truncation marker: `\n[truncated: original was N bytes; last 65536 bytes shown]\n`. |
| `stderr` | `string` | Same shape as `stdout`. On `status: "denied"` carries the helper's input-validation message. On `status: "timeout"` carries `"hard-killed at <N> seconds."` plus any partial stderr captured before SIGKILL. |
| `duration_ms` | `int` | Wall-clock duration in milliseconds. Zero on `status: "denied"`. |

Input validation (helper rejects without invoking the subprocess):

| Input | `stderr` reason |
|---|---|
| `--cmd ""` | `bash_request.cmd may not be empty.` |
| `--timeout` greater than 600 | `bash_request.timeout exceeds 600s cap.` |

Helper-process exit codes:

| Helper-process exit | When |
|---|---|
| `0` | Every payload outcome — `status: "ran"`, `status: "denied"`, `status: "timeout"`. The Director never relies on the process exit code; it parses the JSON on stdout. |
| `2` | Click's built-in `UsageError` for unknown CLI flags. |

> **Required `permissions.allow` entry**: the operator MUST add `Bash(cafleet member exec *)` to the Director's `permissions.allow` for the single-consent-gate UX to hold — see `## Routing Bash via the Director` for setup details.

### Server

Start the admin WebUI FastAPI app via uvicorn. The server is only needed for the admin WebUI at `/ui/` and the `/ui/api/*` endpoints — every other `cafleet` subcommand accesses SQLite directly and does not require the server to be running.

```bash
cafleet server
cafleet server --host 0.0.0.0 --port 9000
CAFLEET_BROKER_HOST=0.0.0.0 CAFLEET_BROKER_PORT=9000 cafleet server
```

| Flag | Required | Notes |
|---|---|---|
| `--host` | no | Bind address. Default `settings.broker_host` (= `127.0.0.1`, overridable via `CAFLEET_BROKER_HOST`). |
| `--port` | no | Bind port. Default `settings.broker_port` (= `8000`, overridable via `CAFLEET_BROKER_PORT`). |

- **Does NOT require `--session-id`.** Supplying `--session-id <uuid>` is silently accepted and ignored (matches the `db init` / `session *` pattern), so a single `cafleet --session-id <literal-uuid> *` allow pattern keeps working for this subcommand.
- `--host` / `--port` flags win when both a flag and the matching env var are set. The env var wins when only it is set. The `127.0.0.1` / `8000` defaults apply otherwise.
- `CAFLEET_BROKER_HOST` and `CAFLEET_BROKER_PORT` are read by `cafleet.config.Settings` via explicit `validation_alias` on each field, so the prefix is consistent with `CAFLEET_DATABASE_URL`.
- No other flags are exposed. `--reload`, `--workers`, `--log-level`, and `--webui-dist-dir` are deliberately NOT supported — users who need them invoke `uv run uvicorn cafleet.server:app ...` directly (which is exactly what `mise //cafleet:dev` does, as an independent entry point that does not delegate to `cafleet server`).
- On startup, if the bundled WebUI dist directory does not exist, `create_app()` emits a one-line warning to stderr: `warning: admin WebUI is not built. /ui/ will return 404. Run 'mise //admin:build'.` The server still starts cleanly — only `/ui/` 404s until the SPA is built.
- Port-in-use errors are NOT wrapped. uvicorn's native `OSError: [Errno 98] Address already in use` propagates to the terminal.

## Typical Workflow

1. **Create a session** (if one does not already exist). `cafleet session create` must be run inside a tmux session — it atomically inserts the session row, registers a hardcoded root Director, writes a placement row for the Director pointing at the current tmux pane, back-fills `sessions.director_agent_id`, and seeds the built-in Administrator (per design 0000025) — all in one transaction:

   ```bash
   cafleet session create --label "my-project"
   ```

   Text output: line 1 is the `session_id`, line 2 is the root Director's `agent_id`, then a human-readable block:

   ```
   550e8400-e29b-41d4-a716-446655440000
   7ba91234-5678-90ab-cdef-112233445566
   label:            my-project
   created_at:       2026-04-16T08:50:00+00:00
   director_name:    Director
   pane:             main:@3:%0
   administrator:    3c4d5e6f-7890-1234-5678-90abcdef1234
   ```

   JSON output (nested, with `administrator_agent_id` at the top level alongside the `director` sub-object):

   ```bash
   cafleet session create --label "my-project" --json
   ```

   ```json
   {
     "session_id": "550e8400-e29b-41d4-a716-446655440000",
     "label": "my-project",
     "created_at": "2026-04-16T08:50:00+00:00",
     "administrator_agent_id": "3c4d5e6f-7890-1234-5678-90abcdef1234",
     "director": {
       "agent_id": "7ba91234-5678-90ab-cdef-112233445566",
       "name": "Director",
       "description": "Root Director for this session",
       "registered_at": "2026-04-16T08:50:00+00:00",
       "placement": {
         "director_agent_id": null,
         "tmux_session": "main",
         "tmux_window_id": "@3",
         "tmux_pane_id": "%0",
         "coding_agent": "unknown",
         "created_at": "2026-04-16T08:50:00+00:00"
       }
     }
   }
   ```

   `placement.director_agent_id` is `null` because the root Director has no parent. `placement.coding_agent` is literally `"unknown"` — auto-detection from `$CLAUDECODE` / `$CLAUDE_CODE_ENTRYPOINT` env vars is deferred.

   Outside tmux the command fails fast with `Error: cafleet session create must be run inside a tmux session` and exit 1 — nothing is written to the DB.

   Capture the printed `session_id` and substitute it for `<session-id>` in every command below. The root Director's `agent_id` is also available on line 2 of the text output, or as `director.agent_id` in the JSON response — an Admin or the Director itself may need it. Because the root Director already has a placement row, Member → Director tmux push notifications work immediately.

2. **Register** with the broker:
   ```bash
   cafleet --session-id <session-id> agent register \
     --name "Code Review Agent" --description "Reviews pull requests"
   # → returns <my-agent-id>, e.g. 7ba91234-5678-90ab-cdef-112233445566
   ```

3. **Discover** other agents:
   ```bash
   cafleet --session-id <session-id> agent list --agent-id <my-agent-id>
   ```

4. **Send** a message to another agent:
   ```bash
   cafleet --session-id <session-id> message send --agent-id <my-agent-id> \
     --to <target-agent-id> --text "Please review PR #42"
   ```

5. **Poll** for incoming messages:
   ```bash
   cafleet --session-id <session-id> message poll --agent-id <my-agent-id>
   ```

6. **Acknowledge** received messages:
   ```bash
   cafleet --session-id <session-id> message ack --agent-id <my-agent-id> --task-id <task-id>
   ```

7. **Repeat** steps 4-6 as needed. Use `cafleet --session-id <session-id> --json <cmd>` when parsing output programmatically.

## Multi-Session Coordination

### Roles

- **Director** — the Claude Code session that first runs `cafleet --session-id <session-id> agent register` in this project. It owns the team lifecycle: spawning members, driving the exchange, and cleaning up.
- **Member** — any peer Claude Code session the Director spawns via `cafleet ... member create`. Each member is automatically registered, and its spawn prompt has the literal `session_id` and `agent_id` UUIDs baked in so every `cafleet` command it issues uses literal flags.

### Monitoring mandate (Director only)

Before spawning **any** member, the Director MUST load `Skill(cafleet-monitoring)` and start a `/loop` monitor as that skill instructs. Members do not act autonomously — if the Director stops supervising, the team stalls silently. Keep the `/loop` active until the final shutdown step.

To inspect a stalled member, follow the 2-stage health check in `Skill(cafleet-monitoring)`: first check `cafleet message poll` for messages, then fall back to `cafleet member capture`:

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

The CLI sends `/exit`, polls `tmux list-panes` for the target `pane_id` until it disappears (15 s timeout), then deregisters the agent and rebalances the layout. On timeout, the pane buffer tail is captured and printed on stderr, and the command exits 2 without deregistering. Rerun with `--force` to skip `/exit` and kill the pane immediately. See the Shutdown Protocol below for the full ordering — `member delete` is step 2, not step 1.

### Shutdown Protocol

The teardown MUST run in this exact order. Skipping any step leaves crons firing against dead agents, or orphan `claude` processes lingering in panes.

**Rule: use cafleet primitives only.** All tmux interactions — write, inspect, and metadata — are encapsulated by cafleet commands. For tmux session/window/pane metadata at Director startup, use `cafleet doctor`. Never invoke `tmux send-keys`, `tmux kill-pane`, `tmux list-panes`, `tmux capture-pane`, or `tmux display-message` directly from the Director. If a workflow appears to need a raw tmux call, file a gap in `cafleet member *` or `cafleet doctor` — NOT a raw tmux invocation.

1. **Stop every background `/loop` monitor FIRST.** Any `/loop` cron the Director started during the session must be cancelled with `CronDelete <job-id>` **before** members are deleted. A cron that keeps firing after members are gone will issue `cafleet member list` / `poll` against a tearing-down session, spam `Error: session is deleted`, and (worse) race with the member-delete path and nudge agents that are mid-`/exit`. Fixed-cadence `/loop`s (e.g. the 1-minute team-health monitor from `Skill(cafleet-monitoring)`) and any augmented loops you created (PR review loops, verifier loops, etc.) all fall under this rule. Stop them all.
2. **Delete every member** via `cafleet --session-id <session-id> member delete --agent-id <director-agent-id> --member-id <member-agent-id>`. This call now blocks until the target pane is actually gone (15 s default timeout). If the pane is stuck on a prompt, the command exits 2 with the pane buffer tail on stderr — inspect with `cafleet member capture`, answer the prompt with `cafleet member send-input --choice N` or `--freetext`, then re-run `cafleet member delete`. If the pane is truly wedged, escalate to `cafleet member delete --force`, which skips `/exit` and kill-panes immediately. Do NOT fall back to raw `tmux kill-pane`. Do this per member, not via `session delete` alone — `session delete` deregisters agents in the DB but does NOT send `/exit` to panes.
3. **Verify every member is gone via cafleet.** Run `cafleet --session-id <session-id> member list --agent-id <director-agent-id>`. The team's member roster should be empty. Any agent still present means step 2 failed — re-run `cafleet member delete` on that member, inspect with `cafleet member capture` if needed, and report to the user if it still refuses to leave. Do NOT use raw tmux to "check" or "force" anything.
4. **Run `cafleet session delete <session-id>`** (positional, no `--session-id` flag). This deregisters the root Director, deregisters the Administrator, sweeps any agent rows that survived step 2, and physically deletes every `agent_placements` row. Plain `cafleet --session-id <session-id> agent deregister --agent-id <root-director-id>` is rejected with `Error: cannot deregister the root Director; use 'cafleet session delete' instead.` — always use `session delete` for the final teardown step.
5. **Confirm the session is closed.** Run `cafleet session list`; the current session should not appear (soft-deleted sessions are hidden). If it still appears with `active` agents, repeat steps 2–4 for that session. Any cross-conversation orphan session surfaced by this final check is also cleaned up via `cafleet session delete <its-session-id>` — never via tmux.

Skipping step 1 is the single most common failure and the one that visibly leaks into the operator's view (recurring cron output in the Director's terminal). Skipping step 3 means you proceed to `session delete` without knowing whether members actually quit, leaving orphan `claude` processes behind.

### Answer a member's AskUserQuestion prompt

When `cafleet member capture` reveals a member paused on an `AskUserQuestion`-shaped prompt (the 4-option frame `1. …`, `2. …`, `3. …`, `4. Type something`), the Director MUST delegate the decision to the user via the three-beat shape below. The Director never decides the body, the choice digit, or the custom free-text on the user's behalf — there is no "obvious enough to pick silently" exception.

The three beats:

1. **Capture** the member's pane with `cafleet --session-id <session-id> member capture --agent-id <director-agent-id> --member-id <member-agent-id> --lines 120`. `120` is the recommended default. Re-run with `--lines 200` only if the first capture is truncated above the AskUserQuestion frame (the `1. …`, `2. …`, `3. …`, `4. Type something` rows are not all visible).
2. **Ask the user via `AskUserQuestion`** with shape-appropriate options per the pane-shapes table below. The question text names the member and summarizes what it is paused on (e.g. `Drafter is paused on AskUserQuestion — which reply should I send?`). No preamble sentence above the question — the capture output the Director already printed this turn plus the question text carry all the context.
3. **Invoke the resolved `cafleet member send-input` via the Director's own Bash tool**: `cafleet --session-id <session-id> member send-input --agent-id <director-agent-id> --member-id <member-agent-id> (--choice N | --freetext "<text>")`. Claude Code's native per-call Bash permission prompt is the user-consent surface. Do NOT print a fenced `bash` block for the user to copy-paste, and do NOT add "please run this in your shell" instructions.

#### Pane prompt shapes

The pane is ALWAYS on the AskUserQuestion 4-option frame when `send-input` is appropriate — `--freetext` itself sends a literal `4` keystroke first to route into the "Type something" slot, so any pane that is not on that frame will be corrupted by a `send-input` call.

| Shape | Member pane looks like | Director's AskUserQuestion options | Resolved send-input call |
|---|---|---|---|
| **Choice-routing** | AskUserQuestion where the labelled options `1. …`, `2. …`, `3. …` ARE the decision point (option labels are meaningful to the user). | Mirror UP TO 3 of the member's labels as AskUserQuestion options. `label` holds the member's short label; `description` holds the member's description if visible in the capture. AskUserQuestion's built-in Other handles custom freetext — do NOT add an explicit 4th option, since `--choice` is `IntRange(1, 3)` and only the CLI's built-in 4-slot routes through `--freetext`. | If the user picked mirror option N (1, 2, or 3), `--choice N`. If the user picked built-in Other and typed a custom body, `--freetext "<typed>"`. |
| **Open-ended** | AskUserQuestion where the labelled options `1. …`, `2. …`, `3. …` are NOT useful for this situation (the member is effectively waiting for free-form instruction). The 4-option frame itself still renders — that frame is exactly what `send-input --freetext` submits through. | 2–4 *complete candidate message bodies*. `label` is a short intent tag (≈12 chars, e.g. `Direct nudge`, `Soft check-in`, `Strict redirect`). `description` holds the FULL draft body so the user can compare wording side-by-side. Built-in Other is the typed-custom-body path. | `--freetext "<picked body>"` when the user picked one of the drafts, or `--freetext "<typed>"` when the user picked built-in Other. |
| **Other shapes** | Pane is NOT on an AskUserQuestion — e.g. mid-command, idle REPL, crashed, awaiting a yes/no confirmation, or mid tool-call. | Do NOT call AskUserQuestion and do NOT call `send-input`. The `send-input` CLI is validated only for the AskUserQuestion 4-option frame; sending a `1`, `2`, `3`, or `4` keystroke into any other shape will corrupt pane state. | None. Escalate to the user via a regular `cafleet message send` nudge, or wait for the member to return to an AskUserQuestion prompt. |

#### AskUserQuestion constraints

| Rule | Value |
|---|---|
| Questions per call | 1–4 |
| Options per question | 2–4 |
| Built-in "Other" | Always exposed by the tool itself. DO NOT add an explicit "Write my own" / "Custom" option. |
| ≥ 5 candidate bodies | Narrow to 2–4 BEFORE asking. Heuristic: drop duplicates and near-duplicates (same intent, different wording), then pick the highest-contrast subset spanning the decision axes (tone, specificity, action). Do NOT paginate across sequential AskUserQuestion calls — each call is a disjoint decision, not a page of a larger list. |
| Preamble text above the question | None. Rely on the `cafleet member capture` output the Director already printed this turn, plus the `AskUserQuestion` question text, to carry all context. |

#### What the Director MUST NOT do

- Pre-draft a single body and tell the user to run the command themselves ("please paste this…").
- Print a fenced `bash` code block containing the resolved `cafleet member send-input` invocation as an instruction for the user to execute.
- Add a one-line preamble sentence above the `AskUserQuestion` (the capture output plus the question text is enough).
- Add an explicit "Write my own" / "Custom" option to the `AskUserQuestion` payload (the built-in "Other" handles it).
- Silently decide a `--choice` digit, even when the member's labels appear obvious.
- Mix shapes: never send `--choice N` on an open-ended pane, and never default to `--freetext` on a choice-routing pane. The shape classification from `cafleet member capture` determines which flag to use — never invert.
- Call `send-input` when the pane is on an "Other shapes" state per the table above. Escalate or wait instead — sending any keystroke would corrupt pane state.

The CLI validates input (`--choice` is `IntRange(1, 3)`; `--freetext` rejects newlines to preserve the one-call-one-submission contract), enforces the same cross-Director authorization boundary as `member capture`, and issues three separate `tmux send-keys` invocations for `--freetext` (`4` → `-l "<text>"` → `Enter`) so shell meta, key names, and multi-byte characters all pass through as literal input.

## Routing Bash via the Director

When a member is spawned via `cafleet member create --no-bash` (the default for claude), its harness rejects every Bash call. To run a shell command, the member sends a JSON `bash_request` envelope to its Director via `cafleet message send`; the Director runs the command via `cafleet member exec` and replies with a `bash_result`. Every operator-consent prompt fires in the Director's pane — never scattered across N member panes.

### `bash_request` (member → Director)

The member uses **existing `cafleet message send`** with a JSON-typed payload in the `--text` body. No new member-side subcommand. Send call shape (use `--json` so the broker's server-side `task_id` is machine-parseable; `--json` is global and goes BEFORE the subcommand):

```bash
cafleet --session-id <session-id> --json message send --agent-id <my-agent-id> \
  --to <director-agent-id> \
  --text '{"type":"bash_request","cmd":"git log -1 --oneline","cwd":"/home/himkt/work/himkt/cafleet","reason":"verifying main before PR"}'
```

Payload schema:

| Field | Type | Required | Notes |
|---|---|---|---|
| `type` | `"bash_request"` | yes | Discriminator. Director dispatches on this exact string. |
| `cmd` | `string` | yes | The shell command to run. Single string; `cafleet member exec` invokes it via `bash -c <cmd>` so pipes / `&&` / quoting work as the member typed them. |
| `cwd` | `string \| null` | no | Working directory. Defaults to the Director's current cwd if `null` or omitted. |
| `stdin` | `string \| null` | no | Stdin payload (UTF-8). Use sparingly — embedding large binaries here will hit the 64 KiB outbound truncation. |
| `timeout` | `int \| null` | no | Seconds. Defaults to 30. Capped at 600 (10 min) — the helper rejects oversized `timeout` as a denied JSON output. |
| `reason` | `string` | yes | Short justification (≤ 200 chars). Surfaced in the Director's `AskUserQuestion` so the operator knows *why* the command is being asked for. |

### Cross-Director boundary

The member always addresses the request to its own `placement.director_agent_id`. Sending a `bash_request` to any other agent ID in the session is undefined behavior — the recipient simply will not know the convention. Cross-session leakage is prevented by the broker's existing session boundary; the within-session "address only your own Director" rule is documentation-only in v1 (no CLI guard).

### Reply correlation

The broker assigns each delivered task a server-side `task_id`, surfaced in `cafleet --json message poll` and `cafleet --json message send`. The member captures the `task_id` from its own `cafleet --json message send` response, holds it locally, and then waits for the broker's tmux push notification (which injects a `cafleet message poll` keystroke into the member's pane when a `bash_result` arrives). The member polls (one-shot, not a loop), filters the polled messages by `in_reply_to == <captured-task-id>`, and resumes.

If the poll returns messages in addition to the `bash_result` (or before it arrives), the member processes those messages as plain instructions per existing CAFleet semantics — the bash-routing continuation only resumes when a `bash_result` with the matching `in_reply_to` arrives.

### `bash_result` (Director → member)

```json
{
  "type": "bash_result",
  "in_reply_to": "<request-task-id>",
  "status": "ran",
  "exit_code": 0,
  "stdout": "abc1234 docs: mark design doc as complete\n",
  "stderr": "",
  "duration_ms": 47,
  "note": "ran without operator prompt (matched allow rule: Bash(git *))"
}
```

| Field | Type | Required | Notes |
|---|---|---|---|
| `type` | `"bash_result"` | yes | Discriminator. |
| `in_reply_to` | `string` | yes | The originating `bash_request`'s `task_id`. |
| `status` | `"ran" \| "denied" \| "timeout"` | yes | Top-level outcome. **Sole source of truth.** `ran` means the helper invoked the subprocess and it terminated normally. `denied` means the helper or operator denied the request (matched no allow + operator picked Deny, OR helper input-validation failed: empty `cmd`, oversized `timeout`). `timeout` means the helper hard-killed the subprocess at the configured `timeout`. |
| `exit_code` | `int` | yes | When `status == "ran"`: the subprocess's verbatim exit code. When `status == "denied"` or `"timeout"`: **value is opaque — do not switch on it.** The helper internally uses `126` for denied and `124` for timeout for shell-legibility, but clients MUST use `status` for branching. |
| `stdout` | `string` | yes | UTF-8, truncated to 64 KiB. Truncation marker: `\n[truncated: original was N bytes; last 65536 bytes shown]\n`. |
| `stderr` | `string` | yes | Same shape as stdout. On `status: "denied"` it carries the operator's typed rejection reason or the helper's input-validation message. On `status: "timeout"` it carries `"hard-killed at <N> seconds."` plus any partial stderr captured before SIGKILL. |
| `duration_ms` | `int` | yes | Wall-clock duration in milliseconds. Zero on `status: "denied"`. |
| `note` | `string` | no | Optional human-readable note. Set on `status: "ran"` when auto-run via the allow-list path: `note: "ran without operator prompt (matched allow rule: <pattern>)"`. Set on `status: "ran"` when the operator edited the command: `note: "operator edited cmd before running. original: <verbatim original cmd>"`. Set on `status: "denied"` when the operator denied: `note: "operator denied: <reason>"`. Omitted otherwise. |

> **Canonical-status rule**: `status` is the SOLE source of truth for the outcome. `exit_code` is meaningful **only when `status == "ran"`**, where it carries the subprocess's verbatim return code. For `status: "denied"` and `status: "timeout"` the field is present (so shell tooling that always reads it does not crash) but its value is **opaque** — clients MUST switch on `status`, not on `exit_code`.

### Director-side dispatch (6 steps)

When the Director's `cafleet message poll` surfaces a message whose `text` parses as `{"type": "bash_request", ...}`:

1. **Discriminator**. Parse the polled `text` body as JSON. If parsing fails, or the parsed object lacks `type`, or `type != "bash_request"`, treat the message as a plain instruction. End — no `bash_result` emitted. Otherwise continue.
2. **Matcher**. Apply the project's resolved `permissions.allow` and `permissions.deny` (concatenated from `~/.claude/settings.json`, `<project>/.claude/settings.json`, and `<project>/.claude/settings.local.json`) using fnmatch-style `Bash(...)` glob patterns. The matcher returns `auto-run` (allow match AND no deny match) or `ask` (every other combination). There is no `auto-deny` outcome — `permissions.deny` only downgrades a would-be `auto-run` to `ask`.
3. If `auto-run`: continue to step 5 with the original `cmd`. Set `note = "ran without operator prompt (matched allow rule: <pattern>)"`.
4. If `ask`: present the 3-option `AskUserQuestion` (table below). Resolve the operator's choice and act per branch:
   - `Approve as-is` → continue with the original `cmd`; do not set `note`.
   - `Approve with edits` → continue with `<typed-cmd>` AND set `note = "operator edited cmd before running. original: <verbatim-original-cmd>"`.
   - `Deny with reason` → skip the helper invocation; emit a denied `bash_result` directly with `status: "denied"`, `note: "operator denied: <reason>"`, `stderr: <reason or "Director denied the request.">`. Reply via step 6, end dispatch.
5. **Run via `cafleet member exec`**. The Director's pane invokes the helper through its Bash tool: `cafleet member exec --cmd '<cmd>' [--cwd '<cwd>'] [--timeout <timeout>] [--stdin '<stdin>']`. Each optional flag is included only when the corresponding request field is present and non-null; otherwise the helper's defaults apply (cwd: pane cwd; timeout: 30 s; stdin: empty). The Director's native Bash-tool prompt does NOT fire because `Bash(cafleet member exec *)` is in the operator's `permissions.allow` (required setup, see § Required `permissions.allow` entry below). The `AskUserQuestion` (step 4) is the only consent surface in this flow. The helper handles deterministic limits (64 KiB caps, SIGKILL at timeout) AND input validation (`cmd != ""`, `timeout ≤ 600`); on input-validation failure the helper writes a denied JSON object to its own stdout (status `"denied"`) which the Director copies into `bash_result` exactly as it would for any other helper output.
6. **Reply**. The Director's pane invokes `cafleet message send --agent-id <director-id> --to <member-id> --text '<bash_result-json>'` via its Bash tool. The push notification injects a `cafleet message poll` keystroke into the member's pane and the member resumes.

### `AskUserQuestion` shape (step 4)

| # | Label | Description holds | Resolves to |
|---|---|---|---|
| 1 | `Approve as-is` | The request's `reason` and the verbatim `cmd`. | `run-as-is(cmd)` |
| 2 | `Approve with edits` | "Operator types the edited command body via Other." (built-in Other) | `run-edited(<typed-cmd>)` |
| 3 | `Deny with reason` | "Operator types the rejection reason via Other." (built-in Other) | `deny(<typed-reason>)` |

Constraints:

- The 4th built-in "Other" slot routes to either edit-or-deny based on what the operator typed. The 3 explicit options exist precisely to disambiguate.
- `AskUserQuestion`'s per-call limits (1 question per call, 2–4 options, built-in "Other" already exposed) are observed. No 5th option, no preamble sentence, no fenced-bash instruction blocks.
- The question text names the member: `"<member-name> wants to run a Bash command. Approve, edit, or deny?"`

### Auto-allow path (no operator prompt)

When the matcher returns `auto-run`, the Director skips the `AskUserQuestion` entirely and invokes `cafleet member exec` directly. The reply carries a `note` field for audit:

```json
{
  "type": "bash_result",
  "in_reply_to": "<request-task-id>",
  "status": "ran",
  "exit_code": 0,
  "stdout": "...",
  "stderr": "",
  "duration_ms": 47,
  "note": "ran without operator prompt (matched allow rule: Bash(git *))"
}
```

The auto-allow path is the entire reason this design exists — commands the operator already trusts at the project level run without interrupting the Director's pane, while everything else gates through the single `AskUserQuestion`.

### Required `permissions.allow` entry

The operator MUST add `Bash(cafleet member exec *)` to the Director's `permissions.allow` (project `.claude/settings.json` or user `~/.claude/settings.json`). Without this entry the Director's pane fires a duplicate native Bash-tool prompt for every command, defeating the single-consent-gate goal — `AskUserQuestion` is supposed to be the sole consent surface in this flow.

This is operator-facing setup, not a programmatic guarantee. A Director that runs without the entry still works, just with worse UX (two prompts per non-allowlisted command). Order is irrelevant — adding the entry before or after the binary upgrade both work.

### No member-side timeout

Members do NOT have a timeout on `bash_result` replies. The member's loop is: send the request via `cafleet message send`, capture the returned `task_id`, wait for the broker's tmux push notification (which injects a `cafleet message poll` keystroke when the `bash_result` arrives), filter polled messages by `in_reply_to == task_id`, and resume.

If the Director wedges (crashed pane, busy mid-tool-call, or the operator stepped away from the keyboard), the member sits idle. Recovery is operator-driven via existing CAFleet primitives:

- `cafleet member capture` to inspect the wedged Director's pane.
- `cafleet message send` to nudge the Director directly (the bash_request task remains in the queue).
- `cafleet message cancel` to retract the original request if the operator wants the member to give up.

This is consistent with every other CAFleet message-passing path — no built-in timeouts.

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
