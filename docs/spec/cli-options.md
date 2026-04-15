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

`db init`, `db *`, `session create`, `session list`, `session show`, `session delete`.

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
