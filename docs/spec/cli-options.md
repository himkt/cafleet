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

> **Why `--session-id` is a literal CLI flag, not an environment variable.** Claude Code's `permissions.allow` matches Bash invocations as literal command strings. A literal `cafleet --session-id <uuid> ...` invocation matches a single `permissions.allow` pattern of the same shape across every subcommand for that session. Shell-expansion patterns (`export VAR=...` followed by `$VAR` substitution) break that matching and force per-invocation permission prompts that interrupt agent work. Substitute the literal UUIDs printed by `cafleet session create` and `cafleet agent register` — do not use shell variables to hold them.

## Global Options

Placed **before** the subcommand:

| Flag | Required | Notes |
|---|---|---|
| `--json` | no | Emit JSON output. |
| `--session-id <id>` | yes for `agent *`, `message *`, `member create/delete/list/capture/send-input/exec/ping` subcommands; no for `db *`, `session *`, `server`, `doctor` | Session identifier (opaque string; new sessions receive a UUIDv4). Also called the namespace identifier. Silently accepted (and ignored) when supplied to subcommands that do not need it, so a single `permissions.allow` pattern of the form `cafleet --session-id <literal-id> *` works for every subcommand. |
| `--version` | no | Print `cafleet <version>` and exit 0. Bypasses the `--session-id` requirement. Sourced from the installed package metadata via `importlib.metadata`. |

### Subcommands that require `--session-id`

`agent register`, `agent deregister`, `agent list`, `agent show`, `message send`, `message broadcast`, `message poll`, `message ack`, `message cancel`, `message show`, `member create`, `member delete`, `member list`, `member capture`, `member send-input`, `member exec`, `member ping`.

### Subcommands that do NOT require `--session-id`

`db init`, `db *`, `session create`, `session list`, `session show`, `session delete`, `server`, `doctor`.

The top-level `--version` flag also short-circuits this check: it is an eager Click option whose callback runs during option parsing and exits before any subcommand (and the `_require_session_id` guard) is reached, so `cafleet --version` succeeds with no `--session-id`.

Create a session first if you don't have one:

```bash
cafleet session create --label "my-project"
# → prints the session_id
```

Then pass the printed UUID as `--session-id <uuid>` on every client + member command.

## Agent ID (`--agent-id`)

`--agent-id` is a **per-subcommand option** (not a global option). It identifies which agent is acting and must be specified on each invocation.

### Commands that require `--agent-id`

- `agent deregister` — Deregister an agent
- `agent list` — List agents in the session
- `agent show` — Show detail for a specific agent
- `message send` — Send a message to another agent
- `message broadcast` — Broadcast a message to all agents
- `message poll` — Poll for incoming messages
- `message ack` — Acknowledge a received message
- `message cancel` — Cancel a sent message
- `message show` — Get task details
- `member create` — Register a new member and spawn its claude pane (Director only)
- `member delete` — Deregister a member and close its pane (Director only)
- `member list` — List members spawned by this Director
- `member capture` — Capture the last N lines of a member's pane (Director only)
- `member send-input` — Forward a restricted keystroke (digit 1/2/3 or free text) to a member's pane (Director only)
- `member exec` — Dispatch a shell command into a member's pane via Claude Code's `!` shortcut (Director only)
- `member ping` — Inject an inbox-poll keystroke into a member's pane (Director only)

### Commands that do NOT require `--agent-id`

- `agent register` — Register a new agent (returns an agent ID)

## `cafleet session` — Session Management

The `cafleet session` subgroup manages sessions. These commands write directly to SQLite — the broker server does not need to be running.

### `session create`

| Flag | Required | Notes |
|---|---|---|
| `--label` | no | Free-form text label for the session |
| `--json` | no | Output as JSON |

There are no `--name` / `--description` flags. The root Director's name and description are hardcoded (`name="Director"`, `description="Root Director for this session"`).

Creates a new session with a UUIDv4 identifier. **Must be run inside a tmux session** — outside tmux the command exits 1 with `Error: cafleet session create must be run inside a tmux session` and writes nothing to the DB. The command atomically performs five writes in a single transaction:

1. `INSERT INTO sessions (...)` with `deleted_at=NULL`, `director_agent_id=NULL`.
2. `INSERT INTO agents (...)` for the hardcoded root Director.
3. `INSERT INTO agent_placements (...)` for the Director with `director_agent_id=NULL` and `coding_agent="unknown"`.
4. `UPDATE sessions SET director_agent_id = <director_agent_id>`.
5. `INSERT INTO agents (...)` for the built-in `Administrator` (see [data-model.md](./data-model.md) for the Administrator's distinguishing `agent_card_json.cafleet.kind` flag).

Any exception inside the transaction rolls back all five writes.

**Non-JSON output** — line 1 is `session_id` (preserves backward-compatible scripts that parse only the first line), line 2 is the root Director's `agent_id`:

```
<session_id>
<director_agent_id>
label:            <label or empty>
created_at:       <iso8601>
director_name:    Director
pane:             <tmux_session>:<tmux_window_id>:<tmux_pane_id>
administrator:    <administrator_agent_id>
```

**`--json` output** — nested shape with `administrator_agent_id` at the top level alongside `director`:

```json
{
  "session_id": "550e8400-e29b-41d4-a716-446655440000",
  "label": "my-project",
  "created_at": "2026-04-15T10:00:00+00:00",
  "administrator_agent_id": "3c4d5e6f-7890-1234-5678-90abcdef1234",
  "director": {
    "agent_id": "7ba91234-5678-90ab-cdef-112233445566",
    "name": "Director",
    "description": "Root Director for this session",
    "registered_at": "2026-04-15T10:00:00+00:00",
    "placement": {
      "director_agent_id": null,
      "tmux_session": "main",
      "tmux_window_id": "@3",
      "tmux_pane_id": "%0",
      "coding_agent": "unknown",
      "created_at": "2026-04-15T10:00:00+00:00"
    }
  }
}
```

`placement.director_agent_id` is `null` because the root Director has no parent. `placement.coding_agent` is the string `"unknown"` — auto-detection of the actual coding agent binary at bootstrap time is deferred.

Attempting `cafleet --session-id <session_id> agent deregister --agent-id <director_agent_id>` is rejected by the broker with `Error: cannot deregister the root Director; use 'cafleet session delete' instead.` and exits 1. Attempting `cafleet --session-id <session_id> agent deregister --agent-id <administrator_agent_id>` is rejected with `Error: Administrator cannot be deregistered` (exit 1) via the `AdministratorProtectedError` path from design 0000025.

### `session list`

| Flag | Required | Notes |
|---|---|---|
| `--json` | no | Output as JSON |

Lists all **non-soft-deleted** sessions with their label, created_at, and active agent count. There is no `--all` flag in this revision — soft-deleted sessions (`sessions.deleted_at IS NOT NULL`) are hidden.

### `session show`

| Argument | Required | Notes |
|---|---|---|
| `session_id` | yes | The session to show |
| `--json` | no | Output as JSON |

Shows details of a single session. Exits 1 with `Error: session 'X' not found.` if the row does not exist at all.

`broker.get_session` intentionally returns soft-deleted rows (to keep audit info reachable), so `session show` succeeds on a soft-deleted session. When the row's `deleted_at` is non-NULL, the text output adds a `deleted_at:` line so callers can distinguish a soft-deleted session from an active one without parsing JSON:

```
session_id: <uuid>
label:      example
created_at: 2026-04-16T09:00:00+00:00
deleted_at: 2026-04-16T10:00:00+00:00
```

The `--json` output always includes `deleted_at` (null when active).

### `session delete`

| Argument | Required | Notes |
|---|---|---|
| `session_id` | yes | The session to delete |

Soft-deletes a session. All three operations run in one transaction:

1. `UPDATE sessions SET deleted_at = now WHERE session_id = X AND deleted_at IS NULL`.
2. `UPDATE agents SET status = 'deregistered', deregistered_at = now WHERE session_id = X AND status = 'active'` (sweeps every active agent in the session — root Director included).
3. `DELETE FROM agent_placements WHERE agent_id IN (SELECT agent_id FROM agents WHERE session_id = X)`.

Tasks are untouched — the message history remains queryable. Output:

```
Deleted session <session_id>. Deregistered N agents.
```

`N` counts every agent that was active at the moment of deletion (root Director included). On re-run against an already-deleted session, the `WHERE deleted_at IS NULL` guard on step 1 short-circuits the cascade and the command prints `Deleted session <session_id>. Deregistered 0 agents.` and exits 0 — the command is idempotent.

There is no `--force` flag. Calling `session delete` on an unknown `session_id` exits 1 with `Error: session 'X' not found.`.

Member tmux panes spawned by `cafleet member create` are **not** automatically closed by `session delete`. For a clean teardown, call `cafleet member delete` per member first (which sends `/exit` to the pane). If a member pane refuses to close (e.g. blocked on a confirmation prompt), rerun `cafleet member delete` with `--force`, which kill-panes the target, sweeps the placement, and rebalances the layout.

## `cafleet doctor` — Placement Diagnostics

Prints the calling pane's tmux session/window/pane identifiers (plus `$TMUX_PANE`) for operators diagnosing placement issues without reaching for raw tmux commands. Intended as the home for future health checks (DB connectivity, orphan-placement scans, etc.); today it covers tmux metadata only.

| Flag | Required | Notes |
|---|---|---|
| `--json` | no | Global `--json`, placed before the subcommand (same pattern as every other CLI command). |
| `--session-id` | no | Silently accepted and ignored, matching `db init` / `session *` / `server`. |

Environment requirements:

- `TMUX` env var must be set — the command rejects otherwise with `Error: cafleet member commands must be run inside a tmux session` (reused verbatim from `tmux.ensure_tmux_available()`).
- `TMUX_PANE` env var must be set — already required by `tmux.director_context()`.

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

Exit codes:

| Exit | When |
|---|---|
| `0` | Success — all four fields printed. |
| `1` | Any tmux or environment failure: `TMUX` env var unset, `tmux` binary not on PATH, `TMUX_PANE` env var unset, or a tmux subprocess (e.g. `display-message`) failure. |

## `cafleet server` — Admin WebUI Server

Starts the admin WebUI FastAPI app (the same app served by `mise //cafleet:dev`) via uvicorn. CLI commands do not require this server to be running — it is only needed when a user wants to view the WebUI at `/ui/` or hit the `/ui/api/*` endpoints from a browser.

`cafleet server` does NOT require `--session-id`. Supplying `--session-id` is silently accepted and ignored, matching the `db init` / `session *` pattern.

| Flag | Default | Notes |
|---|---|---|
| `--host` | `settings.broker_host` (default `127.0.0.1`) | Bind address. Overrides `CAFLEET_BROKER_HOST` when both are set. |
| `--port` | `settings.broker_port` (default `8000`) | Bind port. Overrides `CAFLEET_BROKER_PORT` when both are set. |

Environment variables (read by `cafleet.config.Settings` via explicit `validation_alias`, consistent with `CAFLEET_DATABASE_URL`):

| Variable | Settings field | Notes |
|---|---|---|
| `CAFLEET_BROKER_HOST` | `broker_host` | Wired via `Field(validation_alias="CAFLEET_BROKER_HOST")` on `Settings`. |
| `CAFLEET_BROKER_PORT` | `broker_port` | Wired via `Field(validation_alias="CAFLEET_BROKER_PORT")` on `Settings`. |

The CLI flag wins when both a flag and the matching env var are set; the env var wins when only it is set; the hardcoded default (`127.0.0.1` / `8000`) applies otherwise.

### Behavior

- Calls `uvicorn.run("cafleet.server:app", host=<resolved>, port=<resolved>)` with no `reload`, no custom `workers`, and no custom `log_level` — uvicorn defaults apply.
- On startup, if the bundled WebUI dist directory does not exist, `create_app()` emits a one-line warning to stderr: `warning: admin WebUI is not built. /ui/ will return 404. Run 'mise //admin:build'.`. The warning fires from `create_app()`, so `cafleet server`, `mise //cafleet:dev`, and any direct `uv run uvicorn cafleet.server:app` invocation all see it identically.
- Port-in-use errors are NOT wrapped — uvicorn's native `OSError: [Errno 98] Address already in use` (or the corresponding click/uvicorn traceback) propagates to the terminal.
- The `cafleet server` handler does not perform any disk check itself; the dist-directory warning is entirely owned by `create_app()`.

### No other flags

`--reload`, `--workers`, `--log-level`, and `--webui-dist-dir` are deliberately NOT exposed on `cafleet server`. Users who need them invoke uvicorn directly — which is exactly what `mise //cafleet:dev` does (it runs `uv run uvicorn cafleet.server:app --host 127.0.0.1 --port 8000` as an independent entry point, without delegating to `cafleet server`).

### Examples

```bash
# Defaults: 127.0.0.1:8000
cafleet server

# Override via flags
cafleet server --host 0.0.0.0 --port 9000

# Override via env vars
CAFLEET_BROKER_HOST=0.0.0.0 CAFLEET_BROKER_PORT=9000 cafleet server

# --session-id is silently accepted and ignored
cafleet --session-id 550e8400-e29b-41d4-a716-446655440000 server
```

## Member Commands

The `cafleet member` subgroup manages tmux-backed member agents. All commands require `--agent-id` (the Director's agent ID) and must be run inside a tmux session.

### `member create`

| Flag | Required | Notes |
|---|---|---|
| `--agent-id` | yes | Director's agent ID |
| `--name` | yes | Display name of the new member. Forwarded to the spawned process as `claude --name <member-name> <prompt>`, so the resulting tmux pane title (`#{pane_title}`) shows the member name for the lifetime of the pane. |
| `--description` | yes | One-sentence purpose |
| *(positional, after `--`)* | no | Prompt text for the spawned claude process |

The spawn argv always carries `--permission-mode dontAsk`, so the member's Bash tool is enabled and permission prompts auto-resolve silently. Members run cafleet and any other shell command directly via the Bash tool — no Director routing required by default. The bash-via-Director protocol fires as a fallback when the harness deny-list rejects a Bash invocation (see [`skills/cafleet/SKILL.md`](../../skills/cafleet/SKILL.md) § Routing Bash via the Director).

### `member delete`

| Flag | Required | Notes |
|---|---|---|
| `--agent-id` | yes | Director's agent ID (used for the cross-Director authorization check) |
| `--member-id` | yes | Target member's agent ID |
| `--force` / `-f` | no | Skip the `/exit` wait. Immediately kill-pane the target, then deregister, then rebalance layout. Exit 0 even if the pane was already gone. |

Cross-Director delete is rejected: the CLI verifies `placement.director_agent_id` matches `--agent-id` before calling `broker.deregister_agent` or sending `/exit` to the pane. An attempt to delete another Director's member in the same session exits 1 with `Error: agent <member-id> is not a member of your team (director_agent_id=<other-director>).` (mirrors `member capture` / `member send-input`).

#### Polling contract (default path)

The default path sends `/exit` via `tmux send-keys`, then polls `tmux list-panes -a -F "#{pane_id}"` for the target pane every **500 ms** until the pane disappears or a **15.0 s** timeout elapses. Typical `claude /exit` completes in 1–3 s; operators who need faster escalation pass `--force`. On timeout, the pane buffer tail (last 80 lines) is captured via `tmux capture-pane` and printed on stderr, followed by a recovery hint, and the command exits **2**. The timeout output shape:

```
Error: pane %7 did not close within 15.0s after /exit.
--- pane %7 tail (last 80 lines) ---
<captured terminal buffer>
---
Recovery: inspect with `cafleet member capture`, answer any prompt with `cafleet member send-input`, then re-run `cafleet member delete`. Or re-run with `--force` to skip the wait and kill the pane.
```

#### Exit codes

| Exit | When |
|---|---|
| `0` | Success — default path pane-gone confirmed, `--force` pane killed, or pending-placement deregister. |
| `1` | Any non-timeout failure: auth rejection, missing session, unknown member-id, `broker.deregister_agent` failure, `send_exit` tmux failure (pre-poll), `tmux.wait_for_pane_gone` raising TmuxError (server crash mid-poll). |
| `2` | Default-path timeout — `/exit` was sent, the pane did not disappear within 15.0 s, buffer tail has been printed on stderr. |

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

### `member send-input`

Forwards a restricted keystroke to a member's tmux pane. Two input modes, both AskUserQuestion-only — `--freetext` prepends the digit `4` (the "Type something" gate). For shell dispatch use [`member exec`](#member-exec) instead.

Exactly one of the two flags must be supplied.

| Flag | Required | Notes |
|---|---|---|
| `--agent-id` | yes | Director's agent ID (used for the cross-Director authorization check) |
| `--member-id` | yes | Target member's agent ID |
| `--choice` | one-of | Integer `1`, `2`, or `3`. Sends the matching digit key to the pane (no Enter). Validated via `click.IntRange(1, 3)`. |
| `--freetext` | one-of | Free-text string to type into the "Type something" field. Sends `4`, then the literal text via `tmux send-keys -l`, then `Enter`. AskUserQuestion-only. Rejected if the first non-whitespace character is `!` (use `member exec` for shell dispatch). |

Exactly one of `--choice` / `--freetext` must appear. Supplying zero or both exits 2 with `Error: --choice and --freetext are mutually exclusive; supply exactly one.`.

#### Key sequence sent to the pane

| Invocation | tmux calls issued in order |
|---|---|
| `--choice 1` | `tmux send-keys -t <pane> 1` |
| `--choice 2` | `tmux send-keys -t <pane> 2` |
| `--choice 3` | `tmux send-keys -t <pane> 3` |
| `--freetext "X"` | `tmux send-keys -t <pane> 4` → `tmux send-keys -t <pane> -l "X"` → `tmux send-keys -t <pane> Enter` |

Three separate tmux invocations for `--freetext` because tmux's `-l` (literal) flag is per-invocation: every key in a single `send-keys` call is either literal or key-name interpreted, never a mix. Splitting the sequence guarantees shell meta (`$VAR`, backticks, `$(...)`), key names (`Enter`, `C-c`, `Esc`), backslash-escapes, and multi-byte characters are delivered as plain characters. Because the CLI uses `subprocess.run([...], shell=False)`, no shell ever evaluates the text.

#### Validation rules

| Input | Result |
|---|---|
| Zero or both of `--choice` / `--freetext` | Exit 2 with `Error: --choice and --freetext are mutually exclusive; supply exactly one.` |
| `--choice 0` / `--choice 4` / `--choice a` | Exit 2 via click's built-in `IntRange(1, 3)` validator |
| `--freetext ""` (empty) | Allowed — sends `4` + empty literal + `Enter` (submits an empty answer; AskUserQuestion's own UI decides whether to accept it) |
| `--freetext "   "` (whitespace-only) | Allowed — `lstrip()` empties the string before the `startswith("!")` check, so the bang-prefix guard does not fire. |
| `--freetext` whose first non-whitespace character is `!` | Exit 2 with `Error: --freetext may not start with '!' — that triggers Claude Code's shell-execution shortcut. Use 'cafleet member exec' for shell dispatch instead.` |
| `--freetext` containing `\n` or `\r` | Exit 2 with `Error: free text may not contain newlines.` (single-action contract — one prompt submission per call) |
| Any input with tmux unavailable | Exit 1 via `tmux.ensure_tmux_available()` (same surface as `member capture`) |

#### Authorization boundary

Mirrors `cafleet member capture` step-for-step:

1. Resolve the target via `broker.get_agent(member_id, session_id)`. If `None`, exit 1 with `Error: Agent <member_id> not found`.
2. If `target["placement"]` is `None`, exit 1 with `Error: agent <member_id> has no placement row; it was not spawned via \`cafleet member create\`.`.
3. If `placement["director_agent_id"] != --agent-id`, exit 1 with `Error: agent <member_id> is not a member of your team (director_agent_id=<actual>).`.
4. If `placement["tmux_pane_id"]` is `None` (pending placement), exit 1 with `Error: member <member_id> has no pane yet (pending placement) — nothing to send.`.

Cross-Director write attempts are rejected before any tmux call is made. The error message shapes are reused verbatim from `member capture` so operator muscle memory transfers.

#### Output format

Text:

```
Sent choice 1 to member Claude-B (%7).
Sent free text to member Claude-B (%7).
```

JSON (`cafleet --json ... member send-input ...`):

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

#### Typical Director workflow

> **Note**: Superseded by the canonical **Director-side usage pattern** subsection below. The canonical pattern requires the Director to delegate the decision to the user via `AskUserQuestion` FIRST and then invoke the resolved `cafleet member send-input` via its own Bash tool — AskUserQuestion is required, not optional. This older subsection is retained for historical context only; new readers should follow the canonical pattern.

The CLI is deliberately one-shot — the surrounding choose-and-answer loop stays in the Director's control:

1. `cafleet --session-id <s> member capture --agent-id <d> --member-id <m> --lines 120` — read the current prompt options off the pane.
2. Ask the end user (for example via `AskUserQuestion`) with the observed labels.
3. Based on the answer, either:
   - Option 1 / 2 / 3 → `cafleet --session-id <s> member send-input --agent-id <d> --member-id <m> --choice N`
   - Free-text → `cafleet --session-id <s> member send-input --agent-id <d> --member-id <m> --freetext "<user text>"`

Capture parsing is intentionally left manual because prompt layouts differ across Claude Code versions. The CLI's job is to *send* restricted keystrokes safely; reading and presenting options belongs to the Director.

#### Director-side usage pattern

The canonical Director-side workflow is three-beat and AskUserQuestion-delegated: (1) `cafleet member capture` to inspect the pane, (2) the Director's own `AskUserQuestion` tool call — with shape-matched options per the pane-shapes table — to put the decision in front of the user, (3) the Director invokes the resolved `cafleet member send-input` via its Bash tool, where Claude Code's native per-call permission prompt is the user-consent surface (never a fenced `bash` block for the user to paste). The canonical three-beat workflow, pane-shapes table (choice-routing / open-ended / other shapes), AskUserQuestion constraints (1–4 questions, 2–4 options, built-in "Other"), and "MUST NOT do" rules live in [`skills/cafleet/SKILL.md`](../../skills/cafleet/SKILL.md) under "Answer a member's AskUserQuestion prompt" — that is canonical, and this CLI spec does not duplicate the table.

### `member exec`

Director-only shell-dispatch primitive. Keystrokes `! <command>` + `Enter` into a member's pane via `tmux.send_bash_command` so Claude Code's `!` shortcut runs the command natively (bypassing the member's Bash tool permission system). The fallback path for the bash-via-Director protocol — see [Routing Bash via the Director](../../skills/cafleet/SKILL.md#routing-bash-via-the-director).

```bash
cafleet --session-id <session-id> member exec --agent-id <director-agent-id> \
  --member-id <member-agent-id> "git log -1 --oneline"
```

| Flag / argument | Required | Notes |
|---|---|---|
| `--agent-id` | yes | Director's agent ID (used for the cross-Director authorization check) |
| `--member-id` | yes | Target member's agent ID |
| *(positional `COMMAND`)* | yes | Single shell command. Leading and trailing whitespace are stripped before dispatch to `tmux.send_bash_command` (the JSON `command` field and the text echo both reflect the trimmed form). Otherwise pipes, `&&`, `;`, `$(...)`, and backticks are not special-cased — the command is forwarded opaquely. |

#### Key sequence sent to the pane

| Invocation | tmux calls issued in order |
|---|---|
| `member exec "X"` | `tmux send-keys -t <pane> -l "! X"` → `tmux send-keys -t <pane> Enter` |

Two separate tmux invocations because tmux's `-l` (literal) flag is per-invocation: every key in a single `send-keys` call is either literal or key-name interpreted, never a mix. Splitting the sequence guarantees shell meta (`$VAR`, backticks, `$(...)`), key names embedded in the command (`Enter`, `C-c`, `Esc`), backslash-escapes, and multi-byte characters are delivered as plain characters. Because the CLI uses `subprocess.run([...], shell=False)`, no shell ever evaluates the command before tmux types it.

#### Validation rules

| Input | Result |
|---|---|
| Missing positional `COMMAND` | Click built-in `Error: Missing argument 'COMMAND'.` (exit 2). |
| `command` empty after `.strip()` (`""` or whitespace-only) | `Error: command may not be empty.` (exit 2; `click.UsageError`). |
| `command` containing `\n` or `\r` | `Error: command may not contain newlines.` (exit 2; `click.UsageError`). |
| Outside a tmux session (`TMUX` env var unset) | Exit 1 with `Error: cafleet member commands must be run inside a tmux session` (raised from `tmux.ensure_tmux_available()` and wrapped as a `ClickException`). |
| `tmux` binary not on `PATH` | Exit 1 with the corresponding "binary not found" error from `tmux.ensure_tmux_available()`, wrapped as a `ClickException`. |

#### Authorization boundary

Mirrors `cafleet member send-input` step-for-step:

1. Resolve the target via `broker.get_agent(member_id, session_id)`. If `None`, exit 1 with `Error: Agent <member_id> not found`.
2. If `target["placement"]` is `None`, exit 1 with `Error: agent <member_id> has no placement row; it was not spawned via \`cafleet member create\`.`.
3. If `placement["director_agent_id"] != --agent-id`, exit 1 with `Error: agent <member_id> is not a member of your team (director_agent_id=<actual>).`.
4. If `placement["tmux_pane_id"]` is `None` (pending placement), exit 1 with `Error: member <member_id> has no pane yet (pending placement) — nothing to send.`.

Cross-Director write attempts are rejected before any tmux call is made. Wording reuses the existing `_load_authorized_member` strings verbatim.

#### Output format

Text:

```
Sent bash command 'git log -1 --oneline' to member Claude-B (%7).
```

JSON (`cafleet --json ... member exec ...`):

```json
{
  "member_agent_id": "<uuid>",
  "pane_id": "%7",
  "command": "<command as-sent>"
}
```

Three keys: `member_agent_id`, `pane_id`, `command`. No `action` field — the subcommand name IS the action.

#### Exit code summary

| Outcome | Exit | Source |
|---|---|---|
| Dispatch success | `0` | normal return |
| Missing positional `COMMAND` | `2` | Click built-in |
| `command` empty / whitespace-only | `2` | `click.UsageError` raised by handler |
| `command` contains `\n` or `\r` | `2` | `click.UsageError` raised by handler |
| `tmux` unavailable / `TMUX` env var missing | `1` | `tmux.ensure_tmux_available()` → wrapped `ClickException` |
| Agent not found | `1` | `_load_authorized_member` → wrapped `ClickException` |
| Missing placement row | `1` | `_load_authorized_member` (existing wording) |
| Cross-Director (placement.director_agent_id mismatch) | `1` | `_load_authorized_member` (existing wording) |
| Pending placement (tmux_pane_id is None) | `1` | dedicated check in handler (existing wording) |
| `tmux send-keys` subprocess error | `1` | wrapped `ClickException` (`send failed: ...`) |

### `member ping`

Director-only manual inbox-poll nudge. Keystrokes the same `cafleet --session-id <s> message poll --agent-id <m>` + `Enter` sequence that `broker._try_notify_recipient` auto-fires today, but as an operator-driven entry-point: failures surface as exit 1 (the auto-fire path swallows `False` silently). The action is wholly determined by the subcommand name — there is no positional argument and no operator-controlled keystroke body, which is why this subcommand sits in `permissions.allow` while `member exec` stays in `permissions.ask`.

```bash
cafleet --session-id <session-id> member ping --agent-id <director-agent-id> \
  --member-id <member-agent-id>
```

| Flag | Required | Notes |
|---|---|---|
| `--agent-id` | yes | Director's agent ID (used for the cross-Director authorization check) |
| `--member-id` | yes | Target member's agent ID |

#### Key sequence sent to the pane

| Invocation | tmux calls issued in order |
|---|---|
| `member ping` | `tmux.send_poll_trigger(target_pane_id=<pane>, session_id=<sid>, agent_id=<member_id>)` — types `cafleet --session-id <sid> message poll --agent-id <member_id>` + `Enter` into the pane (same helper as the broker auto-fire). |

#### Validation rules

| Input | Result |
|---|---|
| Missing `--agent-id` | Click built-in `Error: Missing option '--agent-id'.` (exit 2). |
| Missing `--member-id` | Click built-in `Error: Missing option '--member-id'.` (exit 2). |
| Outside a tmux session (`TMUX` env var unset) | Exit 1 with `Error: cafleet member commands must be run inside a tmux session` (raised from `tmux.ensure_tmux_available()` and wrapped as a `ClickException`). |
| `tmux` binary not on `PATH` | Exit 1 with the corresponding "binary not found" error from `tmux.ensure_tmux_available()`, wrapped as a `ClickException`. |

The subcommand has no positional argument and no other flags. There is no operator-controlled keystroke body to validate.

#### Authorization boundary

Mirrors `cafleet member exec` step-for-step:

1. Resolve the target via `broker.get_agent(member_id, session_id)`. If `None`, exit 1 with `Error: Agent <member_id> not found`.
2. If `target["placement"]` is `None`, exit 1 with `Error: agent <member_id> has no placement row; it was not spawned via \`cafleet member create\`.`.
3. If `placement["director_agent_id"] != --agent-id`, exit 1 with `Error: agent <member_id> is not a member of your team (director_agent_id=<actual>).`.
4. If `placement["tmux_pane_id"]` is `None` (pending placement), exit 1 with `Error: member <member_id> has no pane yet (pending placement) — nothing to send.`.

Cross-Director write attempts are rejected before any tmux call is made. Wording reuses the existing `_load_authorized_member` strings verbatim.

#### Output format

Text:

```
Pinged member Claude-B (%7) — poll keystroke dispatched.
```

JSON (`cafleet --json ... member ping ...`):

```json
{
  "member_agent_id": "<uuid>",
  "pane_id": "%7"
}
```

Two keys: `member_agent_id`, `pane_id`. No `action` field (the subcommand name IS the action). No `polled` field — failures surface via exit 1, not via a `polled: false` field.

#### Exit code summary

| Outcome | Exit | Source |
|---|---|---|
| Dispatch success | `0` | normal return |
| Missing `--agent-id` or `--member-id` | `2` | Click built-in `Missing option` |
| `tmux` unavailable / `TMUX` env var missing | `1` | `tmux.ensure_tmux_available()` → wrapped `ClickException` |
| Agent not found | `1` | `_load_authorized_member` → wrapped `ClickException` |
| Missing placement row | `1` | `_load_authorized_member` (existing wording) |
| Cross-Director (placement.director_agent_id mismatch) | `1` | `_load_authorized_member` (existing wording) |
| Pending placement (tmux_pane_id is None) | `1` | dedicated check in handler (existing wording) |
| `tmux send-keys` subprocess error | `1` | wrapped `ClickException` (`send failed: ...`) — covers both the `TmuxError` branch and the `send_poll_trigger` returning `False` branch |

## Error Messages

| Situation | Error Message |
|---|---|
| Missing `--session-id` on a client/member subcommand | `Error: --session-id <uuid> is required for this subcommand. Create a session with 'cafleet session create' and pass its id.` |
| Missing `--agent-id` | `Error: Missing option '--agent-id'.` (Click built-in) |
| `session create` run outside a tmux session | `Error: cafleet session create must be run inside a tmux session` (exit 1; no DB writes) |
| `session delete` on unknown session_id | `Error: session 'X' not found.` (exit 1) |
| `agent register` into a soft-deleted session | `Error: session X is deleted` (exit 1) |
| `agent deregister` against the root Director's `agent_id` | `Error: cannot deregister the root Director; use 'cafleet session delete' instead.` (exit 1) |
| `agent deregister` against the Administrator's `agent_id` | `Error: Administrator cannot be deregistered` (exit 1) |
| `agent list` / `agent show` / `agent deregister` / `message poll` / `message ack` / `message cancel` / `message show` with an `--agent-id` that is not a member of `--session-id` | `Error: agent <id> is not a member of session <sid>.` (exit 1) — gate is `broker.verify_agent_session` and runs before any read/write operation. Also fires for unknown `--agent-id` (the gate cannot tell "unknown" from "in a different session" apart and treats both as not-a-member). |
| `member send-input` with zero or both of `--choice` / `--freetext` | `Error: --choice and --freetext are mutually exclusive; supply exactly one.` (exit 2) |
| `member send-input --choice` outside `1..3` | Click `IntRange(1, 3)` built-in (exit 2) |
| `member send-input --freetext` whose first non-whitespace character is `!` | `Error: --freetext may not start with '!' — that triggers Claude Code's shell-execution shortcut. Use 'cafleet member exec' for shell dispatch instead.` (exit 2) |
| `member send-input --freetext` with `\n` or `\r` | `Error: free text may not contain newlines.` (exit 2) |
| `member send-input` on a member with pending placement | `Error: member <id> has no pane yet (pending placement) — nothing to send.` (exit 1) |
| `member send-input` across Directors | `Error: agent <id> is not a member of your team (director_agent_id=<actual>).` (exit 1) |
| `member exec` with missing positional `COMMAND` | Click built-in `Error: Missing argument 'COMMAND'.` (exit 2) |
| `member exec ""` (empty / whitespace-only) | `Error: command may not be empty.` (exit 2) |
| `member exec` with `\n` or `\r` | `Error: command may not contain newlines.` (exit 2) |
| `member exec` on a member with pending placement | `Error: member <id> has no pane yet (pending placement) — nothing to send.` (exit 1) |
| `member exec` across Directors | `Error: agent <id> is not a member of your team (director_agent_id=<actual>).` (exit 1) |
| `member ping` on a member with pending placement | `Error: member <id> has no pane yet (pending placement) — nothing to send.` (exit 1) |
| `member ping` across Directors | `Error: agent <id> is not a member of your team (director_agent_id=<actual>).` (exit 1) |
| `member ping` when `tmux send-keys` fails | `Error: send failed: tmux send-keys did not deliver the poll-trigger keystroke to pane <pane>.` (exit 1) |
