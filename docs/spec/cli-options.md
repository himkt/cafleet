# CLI Option Specification

How the unified CAFleet CLI (`cafleet`) accepts configuration parameters.

## Option Source Matrix

Each parameter has exactly one input source:

| Parameter | Source |
|---|---|
| Fleet ID | `--fleet-id <int>` global flag |
| Database URL | `CAFLEET_DATABASE_URL` env var (optional; default builds `sqlite:///<path>` from `~/.local/share/cafleet/cafleet.db` with `~` expanded at load time. When setting `CAFLEET_DATABASE_URL` yourself, use an absolute path — SQLAlchemy does not expand `~` in SQLite URLs.) |
| Agent ID | `--agent-id <int>` subcommand option |
| JSON output | `--json` global flag |

> **Why `--fleet-id` is a literal CLI flag, not an environment variable.** Claude Code's `permissions.allow` matches Bash invocations as literal command strings. A literal `cafleet --fleet-id <int> ...` invocation matches a single `permissions.allow` pattern of the same shape across every subcommand for that fleet. Shell-expansion patterns (`export VAR=...` followed by `$VAR` substitution) break that matching and force per-invocation permission prompts that interrupt agent work. Substitute the literal integer ids printed by `cafleet fleet create` and `cafleet agent register` — do not use shell variables to hold them.

## Global Options

Placed **before** the subcommand:

| Flag | Required | Notes |
|---|---|---|
| `--json` | no | Emit JSON output. JSON encoding is compact (`json.dumps(..., separators=(",",":"), ensure_ascii=False)` — non-ASCII like the `…` truncation suffix is emitted as UTF-8, not `\uXXXX`). |
| `--fleet-id <int>` | yes for `agent *`, `message *`, `member create/delete/list/capture/send-input/exec/ping` subcommands; no for `db *`, `fleet *`, `server`, `doctor` | Fleet identifier (integer, typed `int`; new fleets receive a DB-assigned id). Also called the namespace identifier. Passing a non-integer fails with Click's standard `Error: Invalid value for '--fleet-id': '<x>' is not a valid integer.` (exit 2). Silently accepted (and ignored) when supplied to subcommands that do not need it, so a single `permissions.allow` pattern of the form `cafleet --fleet-id <literal-id> *` works for every subcommand. |
| `--version` | no | Print `cafleet <version>` and exit 0. Bypasses the `--fleet-id` requirement. Sourced from the installed package metadata via `importlib.metadata`. |

### `--full` semantics (cross-subcommand escape hatch) {#full-semantics}

`--full` is the global "give me every field cafleet has, untruncated, unfiltered" escape hatch. A single flag covers four overloaded surfaces — deliberately one flag rather than `--full-envelope` / `--full-recipients` / `--full-card` / `--full-body` variants:

| Subcommand | Default behavior | `--full` behavior |
|---|---|---|
| `message {send,poll,ack,cancel,show}` | `text` truncated to `CAFLEET_MAX_TEXT_LEN` codepoints + `…` suffix (see [Message Body Truncation](#message-body-truncation)). Compact rendered envelope: `id`, `from`, `ts`, `text`, plus `kind`/`origin` only when present (ids are full integers). | Untruncated `text` AND the full typed-column envelope (`task_id`, `context_id`, `from_agent_id`, `to_agent_id`, `type`, `status_state`, `status_timestamp`, `origin_task_id`, `text`). |
| `message broadcast` | One-line summary (`broadcast id=<id> recipients=<count>`). The broker only ever returns the single `broadcast_summary` task plus the top-level `notifications_sent_count` wrapper — there are no per-recipient envelopes or `recipient_ids` list in the response. | Renders the single `broadcast_summary` task as the full verbose envelope (typed-column dict in `--json`) instead of the one-line summary. It never adds per-recipient envelopes or a `recipient_ids` list — the response is always that one summary task plus `notifications_sent_count`. |
| `agent list` / `agent show` | One row per agent (`<id> <name> <status>`); `description` truncated to 60 codepoints. JSON projects each agent to `id` / `name` / `description` / `status` (plus `coding_agent` when a placement is present). | Four-line per-agent block: full `agent_id`, `name`, `description` (still truncated to 60 codepoints), `status`. JSON returns the broker agent dict unchanged. No `agent_card_json` — the agent surfaces never load it. |
| `member capture` | Default `--lines 30` (down from 80); ANSI escape sequences stripped in post-process unless `--ansi` is supplied. | No effect on `--lines` (use `--lines N` explicitly); no effect on ANSI stripping (use `--ansi` explicitly). `--full` is accepted on `member capture` for surface consistency but is a no-op there. |

### Subcommands that require `--fleet-id`

`agent register`, `agent deregister`, `agent list`, `agent show`, `message send`, `message broadcast`, `message poll`, `message ack`, `message cancel`, `message show`, `member create`, `member delete`, `member list`, `member capture`, `member send-input`, `member exec`, `member ping`.

### Subcommands that do NOT require `--fleet-id`

`db init`, `db *`, `fleet create`, `fleet list`, `fleet show`, `fleet delete`, `server`, `doctor`.

The top-level `--version` flag also short-circuits this check: it is an eager Click option whose callback runs during option parsing and exits before any subcommand (and the `_require_fleet_id` guard) is reached, so `cafleet --version` succeeds with no `--fleet-id`.

Create a fleet first if you don't have one:

```bash
cafleet fleet create --label "my-project"
# → prints the fleet_id
```

Then pass the printed id as `--fleet-id <id>` on every client + member command.

## Agent ID (`--agent-id`)

`--agent-id` is a **per-subcommand option** (not a global option). It identifies which agent is acting and must be specified on each invocation. It is typed `int`; a non-integer fails with Click's standard `Error: Invalid value for '--agent-id': '<x>' is not a valid integer.` (exit 2). The same `type=int` applies to every id option — `--to`, `--id` (`agent show`), `--member-id`, and `--task-id` — so each rejects a non-integer the same way. Ids are short by construction (DB-assigned integers, typically 1–4 digits), so they are pasted in full; there is no prefix resolution.

### Commands that require `--agent-id`

- `agent deregister` — Deregister an agent
- `agent show` — Show detail for a specific agent
- `message send` — Send a message to another agent
- `message broadcast` — Broadcast a message to all agents
- `message poll` — Poll for un-acked (`input_required`) incoming messages
- `message ack` — Acknowledge a received message
- `message cancel` — Cancel a sent message
- `message show` — Get task details
- `member create` — Register a new member and spawn its coding-agent pane (the spawning Director's id, validated to equal the fleet root)

### Commands that do NOT require `--agent-id`

- `agent register` — Register a new agent (returns an agent ID)
- `agent list` — List agents in the fleet (scoped by the global `--fleet-id`)

The director-side member subcommands identify their target by `--member-id` (scoped to the global `--fleet-id`), not `--agent-id`:

- `member delete` — Deregister a member and close its pane
- `member list` — List the fleet's members
- `member capture` — Capture the last N lines of a member's pane
- `member send-input` — Forward a restricted keystroke (digit 1/2/3 or free text) to a member's pane
- `member exec` — Dispatch a shell command into a member's pane via the coding agent's `!` shortcut
- `member ping` — Inject an inbox-poll keystroke into a member's pane

## Message Body Truncation

The five subcommands that emit a user-supplied delivery body — `cafleet message {send,poll,ack,cancel,show}` — truncate the `text` body to the first `CAFLEET_MAX_TEXT_LEN` Unicode codepoints (default `200`) plus a single `…` codepoint suffix by default. The truncation applies in both text and `--json` output and is implemented in `cafleet/src/cafleet/output.py` (`truncate_text`, `truncate_task_text`) wired into the shared `_client_command` decorator.

| Variable | Settings field | Default | Notes |
|---|---|---|---|
| `CAFLEET_MAX_TEXT_LEN` | `Settings.max_text_len` | `200` | Maximum codepoint length of the rendered `text` body before the `…` suffix is appended. Wired via `Field(validation_alias="CAFLEET_MAX_TEXT_LEN")` on `Settings`, matching the `CAFLEET_`-prefixed convention already used by `CAFLEET_DATABASE_URL`, `CAFLEET_BROKER_HOST`, and `CAFLEET_BROKER_PORT`. Also used by `agent.description` truncation (limit `60`, hard-coded) and metadata-string truncation (limit `80`, hard-coded). |

The suffix is the single Unicode codepoint `…` (U+2026 HORIZONTAL ELLIPSIS) — exactly one codepoint with no count and no companion `text_length` field.

`cafleet message broadcast` is different — `broker.broadcast_message` returns a `broadcast_summary` task whose top-level `text` column is a broker-generated summary string (e.g. `Broadcast sent to N recipients`), not the original body. `message_broadcast` runs with `truncates_task_text=True`: by default the summary renders as the one-line `broadcast id=<id> recipients=<count>`, while `--full` renders the single `broadcast_summary` task as the full typed-column envelope. The default summary string is short, so compact truncation only applies if `CAFLEET_MAX_TEXT_LEN` is set below its length. `--full` never adds per-recipient envelopes or a `recipient_ids` list. The task envelope is a flat typed-column dict with no `metadata` / `artifacts` wrappers; see [message-envelope.md](./message-envelope.md) for the canonical schema.

The table describes the resulting `text` value AFTER truncation. Text mode omits the `text:` line entirely when the resulting value is empty, while `--json` always includes it.

| Input `text` | Default output | `--full` output |
|---|---|---|
| `None` / not present | not present | not present |
| `""` | text mode: `text:` line omitted, `--json`: empty string | text mode: `text:` line omitted, `--json`: empty string |
| length ≤ `CAFLEET_MAX_TEXT_LEN` codepoints | unchanged | unchanged |
| length > `CAFLEET_MAX_TEXT_LEN` codepoints | `text[:CAFLEET_MAX_TEXT_LEN] + "…"` | unchanged |

| Flag | Required | Notes |
|---|---|---|
| `--full` | no | Per-subcommand option (placed after the subcommand name, like `--agent-id` and `--task-id`). Disables truncation; emits the full message body and the full typed-column envelope. Composes orthogonally with `--json`. See [`--full` semantics](#full-semantics) for the cross-subcommand summary. |
| `--quiet` | no | On `message send`, `message ack`, and `member ping`: emit only the new task id on stdout, nothing else. Mutually exclusive with `--full`; the two are not expected to be combined. |

Length is measured in Python `str` codepoints, never bytes — multibyte characters are never split.

```bash
cafleet --fleet-id <fleet-id> message poll --agent-id <my-agent-id>          # default: text truncated to 200 cp + "…"
cafleet --fleet-id <fleet-id> message poll --agent-id <my-agent-id> --full   # full body
```

This applies to CLI emit sites only. FastAPI `/api/*` responses (see [webui-api.md](./webui-api.md)) are unchanged — the WebUI is human-facing and renders full bodies. `agent.description`, `skills[].description`, `agent_card_json` sub-fields, and `member capture` content are also untouched in this release.

## `cafleet fleet` — Fleet Management

The `cafleet fleet` subgroup manages fleets. These commands write directly to SQLite — the broker server does not need to be running.

### `fleet create`

| Flag | Required | Notes |
|---|---|---|
| `--label` | no | Free-form text label for the fleet |
| `--coding-agent` | no | One of `claude` (default), `codex`, or `opencode`. Operator-declared metadata only — `fleet create` does not spawn the root Director's coding-agent process and cannot auto-detect the binary running in the calling pane. The value is recorded as `placement.coding_agent` for the root Director. Validated via `click.Choice(list(CODING_AGENTS.keys()))` — the choice set is registry-driven (currently `["claude", "codex", "opencode"]`) so adding a future backend is one entry in `cafleet.coding_agent.CODING_AGENTS`. Help text: `Coding-agent binary to spawn / declare for the placement.` (Click appends `[default: claude]` automatically via `show_default=True`.) |
| `--json` | no | Output as JSON |

There are no `--name` / `--description` flags. The root Director's name and description are hardcoded (`name="Director"`, `description="Root Director for this fleet"`).

Creates a new fleet with a DB-assigned integer identifier. **Must be run inside a tmux session** — outside tmux the command exits 1 with `Error: cafleet fleet create must be run inside a tmux session` and writes nothing to the DB. The command atomically performs five writes in a single transaction:

1. `INSERT INTO fleets (...)` with `deleted_at=NULL`, `director_agent_id=NULL`.
2. `INSERT INTO agents (...)` for the hardcoded root Director.
3. `INSERT INTO agent_placements (...)` for the Director with `director_agent_id=NULL` and `coding_agent=<value of --coding-agent>` (default `"claude"`).
4. `UPDATE fleets SET director_agent_id = <director_agent_id>`.
5. `INSERT INTO agents (...)` for the built-in `Administrator` (see [data-model.md](./data-model.md) for the Administrator's distinguishing `agent_card_json.cafleet.kind` flag).

Any exception inside the transaction rolls back all five writes.

**Non-JSON output** — line 1 is `fleet_id` (preserves backward-compatible scripts that parse only the first line), line 2 is the root Director's `agent_id`:

```
<fleet_id>
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
  "fleet_id": 1,
  "label": "my-project",
  "created_at": "2026-04-15T10:00:00+00:00",
  "administrator_agent_id": 3,
  "director": {
    "agent_id": 2,
    "name": "Director",
    "description": "Root Director for this fleet",
    "registered_at": "2026-04-15T10:00:00+00:00",
    "placement": {
      "director_agent_id": null,
      "tmux_session": "main",
      "tmux_window_id": "@3",
      "tmux_pane_id": "%0",
      "coding_agent": "claude",
      "created_at": "2026-04-15T10:00:00+00:00"
    }
  }
}
```

`placement.director_agent_id` is `null` because the root Director has no parent. `placement.coding_agent` is the value of `--coding-agent` (default `"claude"`); operators running the codex CLI in the calling pane should pass `--coding-agent codex` so the placement metadata is accurate. cafleet does not spawn the root Director's coding-agent process and cannot auto-detect what is running in the calling pane.

Attempting `cafleet --fleet-id <fleet_id> agent deregister --agent-id <director_agent_id>` is rejected by the broker with `Error: cannot deregister the root Director; use 'cafleet fleet delete' instead.` and exits 1. Attempting `cafleet --fleet-id <fleet_id> agent deregister --agent-id <administrator_agent_id>` is rejected with `Error: Administrator cannot be deregistered` (exit 1) via the `AdministratorProtectedError` path.

### `fleet list`

| Flag | Required | Notes |
|---|---|---|
| `--json` | no | Output as JSON |

Lists all **non-soft-deleted** fleets with their `director_agent_id`, label, created_at, and active agent count. Soft-deleted fleets (`fleets.deleted_at IS NOT NULL`) are hidden.

Each row exposes the fleet's root `director_agent_id` so the Director's ID can be recovered from a list after `fleet create` output scrolls away. The `--json` output carries it as a `director_agent_id` field (integer). Text output renders it as a `DIRECTOR` column placed immediately after `FLEET_ID`:

```
FLEET_ID                               DIRECTOR                                 LABEL                AGENTS   CREATED_AT
```

### `fleet show`

| Argument | Required | Notes |
|---|---|---|
| `fleet_id` | yes | The fleet to show |
| `--json` | no | Output as JSON |

Shows details of a single fleet. Exits 1 with `Error: fleet 'X' not found.` if the row does not exist at all.

`broker.get_fleet` intentionally returns soft-deleted rows (to keep audit info reachable), so `fleet show` succeeds on a soft-deleted fleet. When the row's `deleted_at` is non-NULL, the text output adds a `deleted_at:` line so callers can distinguish a soft-deleted fleet from an active one without parsing JSON:

```
fleet_id: <id>
label:      example
created_at: 2026-04-16T09:00:00+00:00
deleted_at: 2026-04-16T10:00:00+00:00
```

The `--json` output always includes `deleted_at` (null when active).

### `fleet delete`

| Argument | Required | Notes |
|---|---|---|
| `fleet_id` | yes | The fleet to delete |

Soft-deletes a fleet. All three operations run in one transaction:

1. `UPDATE fleets SET deleted_at = now WHERE fleet_id = X AND deleted_at IS NULL`.
2. `UPDATE agents SET status = 'deregistered', deregistered_at = now WHERE fleet_id = X AND status = 'active'` (sweeps every active agent in the fleet — root Director included).
3. `DELETE FROM agent_placements WHERE agent_id IN (SELECT agent_id FROM agents WHERE fleet_id = X)`.

Tasks are untouched — the message history remains queryable. Output:

```
Deleted fleet <fleet_id>. Deregistered N agents.
```

`N` counts every agent that was active at the moment of deletion (root Director included). On re-run against an already-deleted fleet, the `WHERE deleted_at IS NULL` guard on step 1 short-circuits the cascade and the command prints `Deleted fleet <fleet_id>. Deregistered 0 agents.` and exits 0 — the command is idempotent.

There is no `--force` flag. Calling `fleet delete` on an unknown `fleet_id` exits 1 with `Error: fleet 'X' not found.`.

Member tmux panes spawned by `cafleet member create` are **not** automatically closed by `fleet delete`. For a clean teardown, call `cafleet member delete` per member first (which sends `/exit` to the pane). If a member pane refuses to close (e.g. blocked on a confirmation prompt), rerun `cafleet member delete` with `--force`, which kill-panes the target, sweeps the placement, and rebalances the layout.

## `cafleet doctor` — Placement Diagnostics

Prints the calling pane's tmux session/window/pane identifiers (plus `$TMUX_PANE`) for operators diagnosing placement issues without reaching for raw tmux commands. Intended as the home for future health checks (DB connectivity, orphan-placement scans, etc.); today it covers tmux metadata only.

| Flag | Required | Notes |
|---|---|---|
| `--json` | no | Global `--json`, placed before the subcommand (same pattern as every other CLI command). |
| `--fleet-id` | no | Silently accepted and ignored, matching `db init` / `fleet *` / `server`. |

Environment requirements:

- `TMUX` env var must be set — the command rejects otherwise with `Error: cafleet member commands must be run inside a tmux session` (reused verbatim from `MULTIPLEXERS.tmux.ensure_available()`).
- `TMUX_PANE` env var must be set — already required by `MULTIPLEXERS.tmux.context_discovery()`.

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

Starts the admin WebUI FastAPI app (the same app served by `mise //cafleet:dev`) via uvicorn. CLI commands do not require this server to be running — it is only needed when a user wants to view the WebUI at `/` or hit the `/api/*` endpoints from a browser.

`cafleet server` does NOT require `--fleet-id`. Supplying `--fleet-id` is silently accepted and ignored, matching the `db init` / `fleet *` pattern.

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
- On startup, if the bundled WebUI dist directory does not exist, `create_app()` emits a one-line warning to stderr: `warning: admin WebUI is not built. / will return 404. Run 'mise //admin:build'.`. The warning fires from `create_app()`, so `cafleet server`, `mise //cafleet:dev`, and any direct `uv run uvicorn cafleet.server:app` invocation all see it identically.
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

# --fleet-id is silently accepted and ignored
cafleet --fleet-id 1 server
```

## Member Commands

The `cafleet member` subgroup manages tmux-backed member agents and must be run inside a tmux session. `member create` takes `--agent-id` (the spawning Director's agent ID, validated to equal the fleet root); the other subcommands identify their target by `--member-id`, scoped to the global `--fleet-id`.

### `member create`

| Flag | Required | Notes |
|---|---|---|
| `--agent-id` | yes | Director's agent ID |
| `--name` | yes | Display name of the new member. Forwarded to the spawned `claude` process as `claude --name <member-name> <prompt>` so the resulting tmux pane title (`#{pane_title}`) shows the member name for the lifetime of the pane. Neither codex nor opencode has a `--name` analog — operators discover those panes via `cafleet member list`. |
| `--description` | yes | One-sentence purpose |
| `--coding-agent` | no | One of `claude` (default), `codex`, or `opencode`. The flag both selects the `cafleet.coding_agent.CODING_AGENTS` registry entry whose `build_spawn_argv` produces the spawn argv AND is recorded as `placement.coding_agent`. Validated via `click.Choice(list(CODING_AGENTS.keys()))` — the choice set is registry-driven (currently `["claude", "codex", "opencode"]`) so adding a future backend is one entry in `CODING_AGENTS`. Help text: `Coding-agent binary to spawn / declare for the placement.` (Click appends `[default: claude]` automatically via `show_default=True`.) Exits 1 with `Error: binary <name> not found on PATH` when the chosen binary is not on `PATH`. For the `opencode` backend, `OpencodeAgent.ensure_available()` also materializes `~/.opencode/agents/cafleet.md` from the in-source `CAFLEET_AGENT` preset on first spawn (skip-if-exists semantics) — see [Opencode members](../reference/coding-agents/opencode.md) for operational detail. |
| `--prompt-file` | no | Absolute path to a UTF-8 file whose contents are used as the spawn prompt. Mutually exclusive with the positional prompt argument. The file is read verbatim (no stripping) and passes through the same `str.format()` substitution (`fleet_id` / `agent_id` / `director_agent_id`) as the inline form. Relative paths, missing files, unreadable files, invalid UTF-8, and empty (zero-byte or whitespace-only) files all produce non-zero-exit errors — see the [Error Messages](#error-messages) table for the full surface. |
| *(positional, after `--`)* | no | Prompt text for the spawned coding-agent process. All three backends receive the same prompt; the prompt template is backend-neutral. Mutually exclusive with `--prompt-file`. |

#### Spawn command per backend

| Backend | Spawn command |
|---|---|
| `claude` | `claude --permission-mode dontAsk --name <member-name> <prompt>` |
| `codex`  | `codex --ask-for-approval never --sandbox workspace-write <prompt>` |
| `opencode` | `opencode --agent cafleet --prompt <prompt>` |

The `claude` spawn carries `--permission-mode dontAsk`; the `codex` spawn carries `--ask-for-approval never --sandbox workspace-write`; the `opencode` spawn carries `--agent cafleet` which binds the in-source `CAFLEET_AGENT` permission ruleset (catch-all-allow + specific-deny — every permission check resolves to `allow` or `deny`, never `ask`). In all three modes the member's Bash tool is enabled and routine permission prompts auto-resolve silently. Members run cafleet and any other shell command directly via the Bash tool — no Director routing required by default. The bash-via-Director protocol fires as a fallback when the harness deny-list rejects a Bash invocation (see [Bash routing](../concepts/bash-routing.md)). Operational details for codex members live in [Codex members](../reference/coding-agents/codex.md); the opencode equivalent (including the `CAFLEET_AGENT` preset materialization and refresh recipe) lives in [Opencode members](../reference/coding-agents/opencode.md).

#### Spawn-prompt input modes

`cafleet member create` accepts the spawn prompt in three mutually exclusive shapes:

| Inputs | Resulting spawn prompt |
|---|---|
| Neither `--prompt-file` nor positional `prompt_argv` | The built-in `_MEMBER_PROMPT_TEMPLATE` default, with `{fleet_id}` / `{agent_id}` / `{director_agent_id}` substituted. |
| Positional `prompt_argv` only | `" ".join(prompt_argv)` after the same `str.format()` substitution. |
| `--prompt-file PATH` only | The file contents, byte-for-byte, after the same `str.format()` substitution. Surrounding whitespace and trailing newlines are preserved verbatim. |
| Both positional `prompt_argv` and `--prompt-file` | `click.UsageError` (exit 2) — see [Error Messages](#error-messages). |

The `--prompt-file` path is BOTH the spawn input AND the permanent audit artifact. CAFleet-native team skills render the prompt to `<BASE>/prompts/<role>-<UTC-compact>.md` before invoking `member create --prompt-file`, so the on-disk file is the source of truth for what was spawned. Inline `-- "<prompt>"` invocation remains supported for trivial one-line ad-hoc spawns; long, templated identity blocks must use `--prompt-file` because the rendered text otherwise exceeds the documented `tmux split-window` argv ceiling (`tmux command failed: command too long` rolls back the agent registration once the shell-quoted prompt grows past a few KB).

#### Focus behavior

The spawn always invokes `tmux split-window` with `-d` so the Director's pane and active window keep focus — the new member pane is created in the Director's window but is not made active.

### `member delete`

| Flag | Required | Notes |
|---|---|---|
| `--member-id` | yes | Target member's agent ID |
| `--force` / `-f` | no | Skip the `/exit` wait. Immediately kill-pane the target, then deregister, then rebalance layout. Exit 0 even if the pane was already gone. |

The only boundary is fleet isolation: a `--member-id` that does not belong to `--fleet-id` resolves to `None` and exits 1 with `Error: Agent <member-id> not found`. There is no caller-auth check. (Deleting the root Director stays blocked downstream by `broker.deregister_agent`'s root-Director guard.)

#### Polling contract (default path)

The default path sends `/exit` via `tmux send-keys`, then polls `tmux list-panes -a -F "#{pane_id}"` for the target pane every **500 ms** until the pane disappears or a **15.0 s** timeout elapses. A typical coding-agent `/exit` completes in 1–3 s; operators who need faster escalation pass `--force`. On timeout, the pane buffer tail (last 80 lines) is captured via `tmux capture-pane` and printed on stderr, followed by a recovery hint, and the command exits **2**. The timeout output shape:

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
| `1` | Any non-timeout failure: missing fleet, unknown member-id (including cross-fleet), `broker.deregister_agent` failure (e.g. root-Director guard), `send_exit` tmux failure (pre-poll), `MULTIPLEXERS.tmux.wait_for_pane_gone` raising TmuxError (server crash mid-poll). |
| `2` | Default-path timeout — `/exit` was sent, the pane did not disappear within 15.0 s, buffer tail has been printed on stderr. |

### `member list`

Lists every member of the fleet identified by the global `--fleet-id`. The root Director is never surfaced in the output.

| Flag | Required | Notes |
|---|---|---|
| `--activity` | no | Aggregate per-member activity timestamps from the `tasks` table and render `last_sent`, `last_recv`, `last_ack`, and `idle` columns alongside the default member columns. The aggregation filters `Task.type != 'broadcast_summary'` for the `last_ack` proxy (mirrors `poll_tasks`). Primary inputs to the Director's `/loop` monitoring tick. |

#### `member list --activity` output

```
$ cafleet --fleet-id <s> member list --activity
3 members:
  agent_id  name      state   last_sent    last_recv    last_ack     idle
  --------  --------  ------  -----------  -----------  -----------  -----
  5         alice     active  12:34:56     12:34:50     12:34:50     6s
  6         bob       active  12:30:11     12:33:02     12:33:02     2m
  7         carol     idle    -            12:20:00     12:20:00     14m
```

`last_sent` / `last_recv` come from `MAX(tasks.status_timestamp)` filtered by `from_agent_id` / `context_id`; `last_ack` is `MAX(tasks.status_timestamp WHERE status_state='completed' AND type != 'broadcast_summary')`. `idle` is wall-time minus `MAX(last_sent, last_recv)`. Existing indexes `idx_tasks_context_status_ts` and `idx_tasks_from_agent_status_ts` cover the aggregation joins; benchmark target is < 100 ms at 1k messages.

### `member capture`

| Flag | Required | Notes |
|---|---|---|
| `--member-id` | yes | Target member's agent ID |
| `--lines` | no | Number of trailing lines to capture (default: **30**). |
| `--tail` | no | Alias for `--lines`, for muscle-memory consistency with `tail -n`. |
| `--ansi` / `--no-ansi` | no | Default `--no-ansi`: ANSI escape sequences are stripped via `re.sub(r'\x1b\[[0-?]*[ -/]*[@-~]', '', text)` and carriage-return fragments are de-fragmented for TUI redraws. Pass `--ansi` to disable post-processing and emit the raw tmux capture. |

The default is calibrated against per-tick cost — a raw 200-line capture per member would dominate Director token cost. `--lines 30` keeps stalled `AskUserQuestion` prompt headers visible; if a stalled-prompt fixture truncates the prompt header, the default is bumped to 50.

### `member send-input`

Forwards a restricted keystroke to a member's tmux pane. Two input modes, both AskUserQuestion-only — `--freetext` prepends the digit `4` (the "Type something" gate). For shell dispatch use [`member exec`](#member-exec) instead.

Exactly one of the two flags must be supplied.

| Flag | Required | Notes |
|---|---|---|
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
| `--freetext` whose first non-whitespace character is `!` | Exit 2 with `Error: --freetext may not start with '!' — that triggers the coding agent's shell-execution shortcut. Use 'cafleet member exec' for shell dispatch instead.` |
| `--freetext` containing `\n` or `\r` | Exit 2 with `Error: free text may not contain newlines.` (single-action contract — one prompt submission per call) |
| Any input with tmux unavailable | Exit 1 via `MULTIPLEXERS.tmux.ensure_available()` (same surface as `member capture`) |

#### Member resolution

Mirrors `cafleet member capture` step-for-step:

1. Load the target via `broker.get_agent(member_id, fleet_id)`. If `None`, exit 1 with `Error: Agent <member_id> not found` — a `--member-id` outside `--fleet-id` resolves to `None`, so cross-fleet access is the only rejection.
2. If `target.placement` is `None`, exit 1 with `Error: agent <member_id> has no placement row; it was not spawned via \`cafleet member create\`.`.
3. If `placement.tmux_pane_id` is `None` (pending placement), exit 1 with `Error: member <member_id> has no pane yet (pending placement) — nothing to send.`.

The only boundary is fleet isolation — any agent in `--fleet-id` (including the root Director) is a valid `--member-id`, and there is no caller-auth check. The error message shapes are reused verbatim from `member capture` so operator muscle memory transfers.

#### Output format

Text:

```
Sent choice 1 to member Claude-B (%7).
Sent free text to member Claude-B (%7).
```

JSON (`cafleet --json ... member send-input ...`):

```json
{
  "member_agent_id": <id>,
  "pane_id": "%7",
  "action": "choice",
  "value": "1"
}
```

```json
{
  "member_agent_id": <id>,
  "pane_id": "%7",
  "action": "freetext",
  "value": "<user text as-sent>"
}
```

#### Director-side usage pattern

The canonical Director-side workflow is three-beat and AskUserQuestion-delegated: (1) `cafleet member capture` to inspect the pane, (2) the Director's own `AskUserQuestion` tool call — with shape-matched options per the pane-shapes table — to put the decision in front of the user, (3) the Director invokes the resolved `cafleet member send-input` via its Bash tool, where Claude Code's native per-call permission prompt is the user-consent surface (never a fenced `bash` block for the user to paste). The canonical three-beat workflow, pane-shapes table (choice-routing / open-ended / other shapes), AskUserQuestion constraints (1–4 questions, 2–4 options, built-in "Other"), and "MUST NOT do" rules live in `skills/cafleet/SKILL.md` under "Answer a member's AskUserQuestion prompt" — that is canonical, and this CLI spec does not duplicate the table.

### `member exec`

Director-only shell-dispatch primitive. Keystrokes `! <command>` + `Enter` into a member's pane so the coding agent's `!` shortcut runs the command natively (bypassing the member's Bash tool permission system). All three backends (`claude`, `codex`, and `opencode`) honor the leading-`!` shortcut on their input line, so `member exec` works against any backend without modification. The fallback path for the bash-via-Director protocol — see [Bash routing](../concepts/bash-routing.md).

```bash
cafleet --fleet-id <fleet-id> member exec \
  --member-id <member-agent-id> "git log -1 --oneline"
```

| Flag / argument | Required | Notes |
|---|---|---|
| `--member-id` | yes | Target member's agent ID |
| *(positional `COMMAND`)* | yes | Single shell command. Leading and trailing whitespace are stripped before dispatch to `MULTIPLEXERS.tmux.send_bash_command` (the JSON `command` field and the text echo both reflect the trimmed form). Otherwise pipes, `&&`, `;`, `$(...)`, and backticks are not special-cased — the command is forwarded opaquely. |

#### Key sequence sent to the pane

| Invocation | tmux calls issued in order |
|---|---|
| `member exec "X"` | `tmux send-keys -t <pane> -l "! X"` → `tmux send-keys -t <pane> Enter` |

Two separate tmux invocations because tmux's `-l` (literal) flag is per-invocation: every key in a single `send-keys` call is either literal or key-name interpreted, never a mix. Splitting the sequence guarantees shell meta (`$VAR`, backticks, `$(...)`), key names embedded in the command (`Enter`, `C-c`, `Esc`), backslash-escapes, and multi-byte characters are delivered as plain characters. Because the CLI uses `subprocess.run(argv, shell=False)`, no shell ever evaluates the command before tmux types it.

#### Validation rules

| Input | Result |
|---|---|
| Missing positional `COMMAND` | Click built-in `Error: Missing argument 'COMMAND'.` (exit 2). |
| `command` empty after `.strip()` (`""` or whitespace-only) | `Error: command may not be empty.` (exit 2; `click.UsageError`). |
| `command` containing `\n` or `\r` | `Error: command may not contain newlines.` (exit 2; `click.UsageError`). |
| Outside a tmux session (`TMUX` env var unset) | Exit 1 with `Error: cafleet member commands must be run inside a tmux session` (raised from `MULTIPLEXERS.tmux.ensure_available()` and wrapped as a `ClickException`). |
| `tmux` binary not on `PATH` | Exit 1 with the corresponding "binary not found" error from `MULTIPLEXERS.tmux.ensure_available()`, wrapped as a `ClickException`. |

#### Member resolution

Mirrors `cafleet member send-input` step-for-step:

1. Load the target via `broker.get_agent(member_id, fleet_id)`. If `None`, exit 1 with `Error: Agent <member_id> not found` — a `--member-id` outside `--fleet-id` resolves to `None`, so cross-fleet access is the only rejection.
2. If `target.placement` is `None`, exit 1 with `Error: agent <member_id> has no placement row; it was not spawned via \`cafleet member create\`.`.
3. If `placement.tmux_pane_id` is `None` (pending placement), exit 1 with `Error: member <member_id> has no pane yet (pending placement) — nothing to exec.`.

The only boundary is fleet isolation — any agent in `--fleet-id` (including the root Director) is a valid `--member-id`, and there is no caller-auth check. Wording reuses the existing `_load_authorized_member` strings verbatim.

#### Output format

Text:

```
Sent bash command 'git log -1 --oneline' to member Claude-B (%7).
```

JSON (`cafleet --json ... member exec ...`):

```json
{
  "member_agent_id": <id>,
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
| `tmux` unavailable / `TMUX` env var missing | `1` | `MULTIPLEXERS.tmux.ensure_available()` → wrapped `ClickException` |
| Agent not found (including cross-fleet `--member-id`) | `1` | `_load_authorized_member` → wrapped `ClickException` |
| Missing placement row | `1` | `_load_authorized_member` (existing wording) |
| Pending placement (tmux_pane_id is None) | `1` | dedicated check in handler (existing wording) |
| `tmux send-keys` subprocess error | `1` | wrapped `ClickException` (`send failed: ...`) |

### `member ping`

Director-only manual inbox-poll nudge. Keystrokes the same `cafleet --fleet-id <s> message poll --agent-id <m>` + `Enter` sequence that `broker._try_notify_recipient` auto-fires today, but as an operator-driven entry-point: failures surface as exit 1 (the auto-fire path swallows `False` silently). The action is wholly determined by the subcommand name — there is no positional argument and no operator-controlled keystroke body, which is why this subcommand sits in `permissions.allow` while `member exec` stays in `permissions.ask`.

```bash
cafleet --fleet-id <fleet-id> member ping \
  --member-id <member-agent-id>
```

| Flag | Required | Notes |
|---|---|---|
| `--member-id` | yes | Target member's agent ID |

#### Key sequence sent to the pane

| Invocation | tmux calls issued in order |
|---|---|
| `member ping` | `MULTIPLEXERS.tmux.send_poll_trigger(target_pane_id=<pane>, fleet_id=<sid>, agent_id=<member_id>)` — types `cafleet --fleet-id <sid> message poll --agent-id <member_id>` + `Enter` into the pane. |

#### Validation rules

| Input | Result |
|---|---|
| Missing `--member-id` | Click built-in `Error: Missing option '--member-id'.` (exit 2). |
| Outside a tmux session (`TMUX` env var unset) | Exit 1 with `Error: cafleet member commands must be run inside a tmux session` (raised from `MULTIPLEXERS.tmux.ensure_available()` and wrapped as a `ClickException`). |
| `tmux` binary not on `PATH` | Exit 1 with the corresponding "binary not found" error from `MULTIPLEXERS.tmux.ensure_available()`, wrapped as a `ClickException`. |

The subcommand has no positional argument and no other flags. There is no operator-controlled keystroke body to validate.

#### Member resolution

Mirrors `cafleet member exec` step-for-step:

1. Load the target via `broker.get_agent(member_id, fleet_id)`. If `None`, exit 1 with `Error: Agent <member_id> not found` — a `--member-id` outside `--fleet-id` resolves to `None`, so cross-fleet access is the only rejection.
2. If `target.placement` is `None`, exit 1 with `Error: agent <member_id> has no placement row; it was not spawned via \`cafleet member create\`.`.
3. If `placement.tmux_pane_id` is `None` (pending placement), exit 1 with `Error: member <member_id> has no pane yet (pending placement) — nothing to ping.`.

The only boundary is fleet isolation — any agent in `--fleet-id` (including the root Director) is a valid `--member-id`, and there is no caller-auth check. Wording reuses the existing `_load_authorized_member` strings verbatim.

#### Output format

Text:

```
Pinged member Claude-B (%7) — poll keystroke dispatched.
```

JSON (`cafleet --json ... member ping ...`):

```json
{
  "member_agent_id": <id>,
  "pane_id": "%7"
}
```

Two keys: `member_agent_id`, `pane_id`. No `action` field (the subcommand name IS the action). No `polled` field — failures surface via exit 1, not via a `polled: false` field.

#### Exit code summary

| Outcome | Exit | Source |
|---|---|---|
| Dispatch success | `0` | normal return |
| Missing `--member-id` | `2` | Click built-in `Missing option` |
| `tmux` unavailable / `TMUX` env var missing | `1` | `MULTIPLEXERS.tmux.ensure_available()` → wrapped `ClickException` |
| Agent not found (including cross-fleet `--member-id`) | `1` | `_load_authorized_member` → wrapped `ClickException` |
| Missing placement row | `1` | `_load_authorized_member` (existing wording) |
| Pending placement (tmux_pane_id is None) | `1` | dedicated check in handler (existing wording) |
| `tmux send-keys` subprocess error | `1` | wrapped `ClickException` (`send failed: ...`) — covers both the `TmuxError` branch and the `send_poll_trigger` returning `False` branch |

## Error Messages

| Situation | Error Message |
|---|---|
| Missing `--fleet-id` on a client/member subcommand | `Error: --fleet-id <int> is required for this subcommand. Create a fleet with 'cafleet fleet create' and pass its id.` |
| Missing `--agent-id` | `Error: Missing option '--agent-id'.` (Click built-in) |
| `fleet create` run outside a tmux session | `Error: cafleet fleet create must be run inside a tmux session` (exit 1; no DB writes) |
| `fleet delete` on unknown fleet_id | `Error: fleet 'X' not found.` (exit 1) |
| `agent register` into a soft-deleted fleet | `Error: fleet X is deleted` (exit 1) |
| `agent deregister` against the root Director's `agent_id` | `Error: cannot deregister the root Director; use 'cafleet fleet delete' instead` (exit 2; raised as `click.UsageError`) |
| `agent deregister` against the Administrator's `agent_id` | `Error: Administrator cannot be deregistered` (exit 1) |
| `agent show` / `agent deregister` / `message send` / `message poll` / `message ack` / `message cancel` / `message show` with an `--agent-id` that is not a member of `--fleet-id` | `Error: agent <id> is not a member of fleet <sid>.` (exit 1) — gate is `broker.verify_agent_fleet` and runs before any read/write operation. Also fires for unknown `--agent-id` (the gate cannot tell "unknown" from "in a different fleet" apart and treats both as not-a-member). |
| `member send-input` with zero or both of `--choice` / `--freetext` | `Error: --choice and --freetext are mutually exclusive; supply exactly one.` (exit 2) |
| `member send-input --choice` outside `1..3` | Click `IntRange(1, 3)` built-in (exit 2) |
| `member send-input --freetext` whose first non-whitespace character is `!` | `Error: --freetext may not start with '!' — that triggers the coding agent's shell-execution shortcut. Use 'cafleet member exec' for shell dispatch instead.` (exit 2) |
| `member send-input --freetext` with `\n` or `\r` | `Error: free text may not contain newlines.` (exit 2) |
| `member send-input` on a member with pending placement | `Error: member <id> has no pane yet (pending placement) — nothing to send.` (exit 1) |
| `member exec` with missing positional `COMMAND` | Click built-in `Error: Missing argument 'COMMAND'.` (exit 2) |
| `member exec ""` (empty / whitespace-only) | `Error: command may not be empty.` (exit 2) |
| `member exec` with `\n` or `\r` | `Error: command may not contain newlines.` (exit 2) |
| `member exec` on a member with pending placement | `Error: member <id> has no pane yet (pending placement) — nothing to exec.` (exit 1) |
| `member ping` on a member with pending placement | `Error: member <id> has no pane yet (pending placement) — nothing to ping.` (exit 1) |
| `member ping` when `tmux send-keys` fails | `Error: send failed: tmux send-keys did not deliver the poll-trigger keystroke to pane <pane>.` (exit 1) |
| `member create` with both `--prompt-file` and positional `prompt_argv` | `Error: --prompt-file and the positional prompt argument are mutually exclusive.` (exit 2; `click.UsageError`) |
| `member create --prompt-file` with a relative path | ``Error: --prompt-file requires an absolute path (got '<input>'). Resolve relative paths against your BASE first — see the `cafleet-base-dir` skill.`` (exit 2; `click.UsageError`) |
| `member create --prompt-file` to a non-existent path or non-regular file (e.g. directory) | `Error: --prompt-file <path>: file does not exist or is not a regular file.` (exit 1; `click.ClickException`) |
| `member create --prompt-file` to an unreadable file | `Error: --prompt-file <path>: file is not readable.` (exit 1; `click.ClickException`) |
| `member create --prompt-file` to a file containing invalid UTF-8 | `Error: --prompt-file <path>: file is not valid UTF-8.` (exit 1; `click.ClickException`) |
| `member create --prompt-file` to a zero-byte or whitespace-only file | `Error: --prompt-file <path>: file is empty.` (exit 1; `click.ClickException`) |

