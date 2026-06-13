---
icon: lucide/square-terminal
---

# CLI options

How the unified CAFleet CLI (`cafleet`) accepts configuration parameters.

## Subcommand summary

One row per subcommand. "Identity flag" is the per-subcommand option naming the acting agent (`--agent-id`) or the target member (`--member-id`).

| Subcommand | Purpose | `--fleet-id` | Identity flag | Section |
|---|---|---|---|---|
| `db init` | Create or migrate the SQLite schema to head | no | none | [db init](#db-init) |
| `fleet create` | Create a fleet with its root Director and Administrator | no | none | [fleet create](#fleet-create) |
| `fleet list` | List non-deleted fleets | no | none | [fleet list](#fleet-list) |
| `fleet show` | Show one fleet (soft-deleted included) | no | none | [fleet show](#fleet-show) |
| `fleet delete` | Soft-delete a fleet and deregister its agents | no | none | [fleet delete](#fleet-delete) |
| `doctor` | Print the calling pane's tmux identifiers | no | none | [doctor](#cafleet-doctor) |
| `server` | Start the admin WebUI server | no | none | [server](#cafleet-server) |
| `agent register` | Register a new agent | yes | none | [agent register](#agent-register) |
| `agent deregister` | Deregister an agent | yes | `--agent-id` | [agent deregister](#agent-deregister) |
| `agent list` | List the fleet's agents | yes | none | [agent list](#agent-list) |
| `agent show` | Show one agent's detail | yes | `--agent-id` | [agent show](#agent-show) |
| `message send` | Send a unicast message | yes | `--agent-id` | [message send](#message-send) |
| `message broadcast` | Broadcast a message to all fleet agents | yes | `--agent-id` | [message broadcast](#message-broadcast) |
| `message poll` | Fetch un-acked incoming messages | yes | `--agent-id` | [message poll](#message-poll) |
| `message ack` | Acknowledge a received message | yes | `--agent-id` | [message ack](#message-ack) |
| `message cancel` | Retract an un-acked sent message | yes | `--agent-id` | [message cancel](#message-cancel) |
| `message show` | Show one task | yes | `--agent-id` | [message show](#message-show) |
| `member create` | Register a member and spawn its coding-agent pane | yes | `--agent-id` | [member create](#member-create) |
| `member delete` | Close a member's pane and deregister it | yes | `--member-id` | [member delete](#member-delete) |
| `member list` | List the fleet's members | yes | none | [member list](#member-list) |
| `member capture` | Capture the tail of a member's pane | yes | `--member-id` | [member capture](#member-capture) |
| `member send-input` | Forward a restricted keystroke to a member's pane | yes | `--member-id` | [member send-input](#member-send-input) |
| `member exec` | Dispatch a shell command into a member's pane | yes | `--member-id` | [member exec](#member-exec) |
| `member ping` | Inject an inbox-poll keystroke into a member's pane | yes | `--member-id` | [member ping](#member-ping) |
| `monitor start` | Run the per-fleet scheduler loop in-process (launch as a background task) | yes | none | [monitor start](#monitor-start) |
| `monitor status` | Show monitor liveness and the per-agent schedule | yes | none | [monitor status](#monitor-status) |
| `monitor config` | Show or edit an agent's monitor schedule | yes | `--agent-id` | [monitor config](#monitor-config) |

## Option Source Matrix

Each parameter has exactly one input source:

| Parameter | Source |
|---|---|
| Fleet ID | `--fleet-id <int>` global flag |
| Database URL | `CAFLEET_DATABASE_URL` env var (optional; default builds `sqlite:///<path>` from `~/.local/share/cafleet/cafleet.db` with `~` expanded at load time. When setting `CAFLEET_DATABASE_URL` yourself, use an absolute path — SQLAlchemy does not expand `~` in SQLite URLs.) |
| Agent ID | `--agent-id <int>` subcommand option |
| JSON output | `--json` global flag |

> **Why `--fleet-id` is a literal CLI flag, not an environment variable.** Claude Code's `permissions.allow` matches Bash invocations as literal command strings. A literal `cafleet --fleet-id <int> ...` invocation matches a single `permissions.allow` pattern of the same shape across every subcommand for that fleet. Shell-expansion patterns (`export VAR=...` followed by `$VAR` substitution) break that matching and force per-invocation permission prompts that interrupt agent work. Substitute the literal integer ids printed by `cafleet fleet create` and `cafleet agent register` — do not use shell variables to hold them.

Create a fleet first if you don't have one:

```bash
cafleet fleet create --label "my-project"
# → prints the fleet_id
```

Then pass the printed id as `--fleet-id <id>` on every client + member command.

## Global Options

Placed **before** the subcommand:

| Flag | Required | Notes |
|---|---|---|
| `--json` | no | Emit JSON output. JSON encoding is compact single-line JSON; non-ASCII (like the `…` truncation suffix) is emitted as UTF-8, not escaped. |
| `--fleet-id <int>` | see the [Subcommand summary](#subcommand-summary) | Fleet identifier (integer, typed `int`; new fleets receive a DB-assigned id). Also called the namespace identifier. Passing a non-integer fails with Click's standard `Error: Invalid value for '--fleet-id': '<x>' is not a valid integer.` (exit 2). Silently accepted (and ignored) when supplied to subcommands that do not need it, so a single `permissions.allow` pattern of the form `cafleet --fleet-id <literal-id> *` works for every subcommand. |
| `--version` | no | Print `cafleet <version>` and exit 0. Bypasses the `--fleet-id` requirement. |

### `--full` semantics (cross-subcommand escape hatch) {#full-semantics}

`--full` is the global "give me every field cafleet has, untruncated, unfiltered" escape hatch. A single flag covers four overloaded surfaces:

| Subcommand | Default behavior | `--full` behavior |
|---|---|---|
| `message {send,poll,ack,cancel,show}` | `text` truncated to `CAFLEET_MAX_TEXT_LEN` codepoints + `…` suffix (see [Message Body Truncation](#message-body-truncation)). Compact rendered envelope: `id`, `from`, `ts`, `text`, plus `kind`/`origin` only when present (ids are full integers). | Untruncated `text`. In `--json`, emits the full typed-column task dict (`task_id`, `context_id`, `from_agent_id`, `to_agent_id`, `type`, `status_state`, `status_timestamp`, `origin_task_id`, `text`); in text mode, switches to the verbose labeled block (see [Message envelope](./message-envelope.md#text-mode)). |
| `message broadcast` | One-line summary (`broadcast id=<id> recipients=<count>`). | Renders the single `broadcast_summary` task as the full verbose envelope (typed-column dict in `--json`) instead of the one-line summary. It never adds per-recipient envelopes or a `recipient_ids` list — the response is always that one summary task plus `notifications_sent_count`. |
| `agent list` / `agent show` | One row per agent (`<id> <name> <status>`); `description` truncated to 60 codepoints. JSON projects each agent to `id` / `name` / `description` / `status`. `agent show` additionally emits `coding_agent` when the agent has a placement; `agent list` never does — it does not load placement data. | Four-line per-agent block: full `agent_id`, `name`, `description` (still truncated to 60 codepoints), `status`. JSON returns the broker agent dict unchanged. No `agent_card_json` — the agent surfaces never emit it. |
| `member capture` | Default `--lines 30`; ANSI escape sequences stripped in post-process unless `--ansi` is supplied. | No effect on `--lines` (use `--lines N` explicitly); no effect on ANSI stripping (use `--ansi` explicitly). `--full` is accepted on `member capture` for surface consistency but is a no-op there. |

## Agent ID (`--agent-id`)

`--agent-id` is a **per-subcommand option** (not a global option). It identifies which agent is acting and must be specified on each invocation. It is typed `int`; a non-integer fails with Click's standard `Error: Invalid value for '--agent-id': '<x>' is not a valid integer.` (exit 2). The same `type=int` applies to every id option — `--to`, `--id` (`agent show`), `--member-id`, and `--task-id` — so each rejects a non-integer the same way. Ids are short by construction (DB-assigned integers, typically 1–4 digits), so they are pasted in full; there is no prefix resolution. Which subcommand takes which identity flag is in the [Subcommand summary](#subcommand-summary).

## Message Body Truncation

The five subcommands that emit a user-supplied delivery body — `cafleet message {send,poll,ack,cancel,show}` — truncate the `text` body to the first `CAFLEET_MAX_TEXT_LEN` Unicode codepoints (default `200`) plus a single `…` codepoint suffix by default. The truncation applies uniformly in both text and `--json` output.

| Variable | Settings field | Default | Notes |
|---|---|---|---|
| `CAFLEET_MAX_TEXT_LEN` | `max_text_len` | `200` | Maximum codepoint length of the rendered `text` body before the `…` suffix is appended. It also bounds the broker's inline-preview truncation before the preview is keystroked into a recipient's tmux pane. (`agent.description` truncation uses a separate hard-coded 60-codepoint limit, independent of this env var.) |

The suffix is the single Unicode codepoint `…` (U+2026 HORIZONTAL ELLIPSIS) — exactly one codepoint with no count and no companion `text_length` field.

`cafleet message broadcast` is different — the broker returns a `broadcast_summary` task whose top-level `text` column is a broker-generated summary string (e.g. `Broadcast sent to N recipients`), not the original body. By default the summary renders as the one-line `broadcast id=<id> recipients=<count>` — see the [`--full` semantics](#full-semantics) `message broadcast` row.

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
| `--quiet` | no | On `message send`, `message ack`, and `member ping`: emit only the new task id on stdout, nothing else. Mutually exclusive with `--full`. |

Length is measured in Python `str` codepoints, never bytes — multibyte characters are never split.

```bash
cafleet --fleet-id <fleet-id> message poll --agent-id <my-agent-id>          # default: text truncated to 200 cp + "…"
cafleet --fleet-id <fleet-id> message poll --agent-id <my-agent-id> --full   # full body
```

This applies to CLI emit sites only. FastAPI `/api/*` responses (see [webui-api.md](./webui-api.md)) are unchanged — the WebUI is human-facing and renders full bodies. `agent.description`, `skills[].description`, `agent_card_json` sub-fields, and `member capture` content are also untouched.

## `cafleet db` — Schema Management

### `db init`

No flags. Creates the database file if missing and applies Alembic migrations
to the head revision; idempotent, so re-run it after every package upgrade.
Prints which action was taken (created, upgraded, or already at head).
Install-time usage and the upgrade warning for pre-integer-PK databases are
canonical on the [Install](../get-started/install.md) page.

## `cafleet fleet` — Fleet Management

The `cafleet fleet` subgroup manages fleets. These commands write directly to SQLite — the broker server does not need to be running.

### `fleet create`

| Flag | Required | Notes |
|---|---|---|
| `--label` | no | Free-form text label for the fleet |
| `--coding-agent` | no | One of `claude` (default), `codex`, or `opencode`, recorded as the root Director's placement `coding_agent` — see [Coding agents](../concepts/coding-agents.md). |
| `--json` | no | Output as JSON |
| `--full` | no | Hidden flag (accepted but not shown in `--help`): switches the non-JSON output from the compact one-line form to the 7-line block below. |

There are no `--name` / `--description` flags. The root Director's name and description are hardcoded (`name="Director"`, `description="Root Director for this fleet"`).

Creates a new fleet with a DB-assigned integer identifier. **Must be run inside a tmux session** — outside tmux the command exits 1 with `Error: cafleet fleet create must be run inside a tmux session` and writes nothing to the DB. It creates the fleet, its root Director (and placement), and the built-in Administrator atomically (all-or-nothing) — see [data-model.md](./data-model.md) for the Administrator's distinguishing `agent_card_json.cafleet.kind` flag.

**Non-JSON output (default)** — one compact line carrying the new `fleet_id`, the root Director's `agent_id`, and the built-in Administrator's `agent_id`:

```
<fleet_id> director=<director_agent_id> admin=<administrator_agent_id>
```

**Non-JSON output (hidden `--full` flag)** — line 1 is `fleet_id`, line 2 is the root Director's `agent_id`:

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

Attempting `cafleet --fleet-id <fleet_id> agent deregister --agent-id <director_agent_id>` is rejected by the broker with `Error: cannot deregister the root Director; use 'cafleet fleet delete' instead.` and exits 1. Attempting `cafleet --fleet-id <fleet_id> agent deregister --agent-id <administrator_agent_id>` is rejected with `Error: Administrator cannot be deregistered` (exit 1).

### `fleet list`

| Flag | Required | Notes |
|---|---|---|
| `--json` | no | Output as JSON |

Lists all **non-soft-deleted** fleets with their `director_agent_id`, label, created_at, and active agent count. Soft-deleted fleets (`fleets.deleted_at IS NOT NULL`) are hidden.

Each row exposes the fleet's root `director_agent_id` so the Director's ID can be recovered from a list after `fleet create` output scrolls away. The `--json` output carries it as a `director_agent_id` field (integer). Text output renders it as a `DIRECTOR` column placed immediately after `FLEET_ID`:

```
FLEET_ID  DIRECTOR  LABEL       AGENTS  CREATED_AT
1         2         my-project  3       2026-04-15T10:00:00+00:00
```

### `fleet show`

| Argument | Required | Notes |
|---|---|---|
| `fleet_id` | yes | The fleet to show |
| `--json` | no | Output as JSON |

Shows details of a single fleet. Exits 1 with `Error: fleet 'X' not found.` if the row does not exist at all.

`fleet show` intentionally returns soft-deleted rows (to keep audit info reachable), so it succeeds on a soft-deleted fleet. When the row's `deleted_at` is non-NULL, the text output adds a `deleted_at:` line so callers can distinguish a soft-deleted fleet from an active one without parsing JSON:

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

Soft-deletes a fleet in one transaction: it stamps the fleet as deleted, deregisters every agent that was active at the moment of deletion (root Director included), and removes their placement rows. Tasks are untouched — the message history remains queryable. Output:

```
Deleted fleet <fleet_id>. Deregistered N agents.
```

`N` counts every agent that was active at the moment of deletion (root Director included). On re-run against an already-deleted fleet, the command prints `Deleted fleet <fleet_id>. Deregistered 0 agents.` and exits 0 — the command is idempotent.

There is no `--force` flag. Calling `fleet delete` on an unknown `fleet_id` exits 1 with `Error: fleet 'X' not found.`.

Member tmux panes spawned by `cafleet member create` are **not** automatically closed by `fleet delete`. For a clean teardown, call `cafleet member delete` per member first (which sends `/exit` to the pane). If a member pane refuses to close (e.g. blocked on a confirmation prompt), rerun `cafleet member delete` with `--force`, which kill-panes the target, sweeps the placement, and rebalances the layout.

## `cafleet doctor` — Placement Diagnostics {#cafleet-doctor}

Prints the calling pane's tmux session/window/pane identifiers (plus `$TMUX_PANE`) for operators diagnosing placement issues without reaching for raw tmux commands.

| Flag | Required | Notes |
|---|---|---|
| `--json` | no | Global `--json`, placed before the subcommand (same pattern as every other CLI command). |
| `--fleet-id` | no | Silently accepted and ignored, matching `db init` / `fleet *` / `server`. |

Environment requirements:

- `TMUX` env var must be set — the command rejects otherwise with `Error: cafleet member commands must be run inside a tmux session` (the same message used by `cafleet member` commands).
- `TMUX_PANE` env var must be set — already required for pane discovery.

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

## `cafleet server` — Admin WebUI Server {#cafleet-server}

Starts the admin WebUI FastAPI app (the same app served by `mise //cafleet:dev`) via uvicorn. CLI commands do not require this server to be running — it is only needed when a user wants to view the WebUI at `/` or hit the `/api/*` endpoints from a browser.

`cafleet server` does NOT require `--fleet-id`. Supplying `--fleet-id` is silently accepted and ignored, matching the `db init` / `fleet *` pattern.

| Flag | Default | Notes |
|---|---|---|
| `--host` | `settings.broker_host` (default `127.0.0.1`) | Bind address. Overrides `CAFLEET_BROKER_HOST` when both are set. |
| `--port` | `settings.broker_port` (default `8000`) | Bind port. Overrides `CAFLEET_BROKER_PORT` when both are set. |

Environment variables:

| Variable | Settings field | Default |
|---|---|---|
| `CAFLEET_BROKER_HOST` | `broker_host` | `127.0.0.1` |
| `CAFLEET_BROKER_PORT` | `broker_port` | `8000` |

The CLI flag wins when both a flag and the matching env var are set; the env var wins when only it is set; the hardcoded default (`127.0.0.1` / `8000`) applies otherwise.

### Behavior

- Runs uvicorn with its defaults — no reload, no custom workers, no custom log level.
- On startup, if the bundled WebUI dist directory does not exist, the app emits a one-line warning to stderr: `warning: admin WebUI is not built. / will return 404. Run 'mise //admin:build'.`. The warning fires from the app factory, so `cafleet server`, `mise //cafleet:dev`, and any direct uvicorn invocation all see it identically.
- Port-in-use errors are NOT wrapped — uvicorn's native `OSError: [Errno 98] Address already in use` (or the corresponding uvicorn traceback) propagates to the terminal.

### No other flags

`--reload`, `--workers`, `--log-level`, and `--webui-dist-dir` are deliberately NOT exposed on `cafleet server`. Users who need them invoke uvicorn directly — which is exactly what `mise //cafleet:dev` does (it runs `uv run --package cafleet uvicorn cafleet.webui.app:app --host 127.0.0.1 --port 8000` as an independent entry point, without delegating to `cafleet server`).

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

## `cafleet agent` — Agent Registry

All four subcommands require the global `--fleet-id`. The default-vs-`--full`
output projection shared by `agent list` and `agent show` is documented in
[`--full` semantics](#full-semantics) and is not restated per subcommand.

### `agent register`

| Flag | Required | Notes |
|---|---|---|
| `--name` | yes | Short human-identifiable label. |
| `--description` | yes | One-sentence purpose statement. |
| `--skills` | no | Skill descriptors as a JSON array string, persisted into the agent's `agent_card_json`. Invalid JSON exits 1 with `Error: Invalid JSON in --skills: <detail>`. |

No identity flag — registering is how an agent obtains its id. Text output:

```
Agent registered successfully!
  agent_id:  <agent_id>
  name:      <name>
```

`--json` returns `{"agent_id":<id>,"name":"<name>","registered_at":"<iso8601>"}`.

### `agent deregister`

| Flag | Required | Notes |
|---|---|---|
| `--agent-id` | yes | The agent to deregister. |

Text output: `Agent deregistered successfully.`; `--json` returns
`{"status":"deregistered"}`. The root Director and the Administrator are
protected — see [Error Messages](#error-messages).

### `agent list`

| Flag | Required | Notes |
|---|---|---|
| `--full` | no | See [`--full` semantics](#full-semantics). |

No identity flag — the listing is scoped by the global `--fleet-id` alone.
Default text output is one `<agent_id> <name> <status>` line per agent,
blank-line separated; an empty fleet prints `No agents found.`:

```
2 Director active

3 Administrator active

4 demo-member active
```

### `agent show`

| Flag | Required | Notes |
|---|---|---|
| `--agent-id` | yes | The acting agent (fleet-membership gate). |
| `--id` | yes | The target agent to show. |
| `--full` | no | See [`--full` semantics](#full-semantics). |

Default text output is the same one-line `<agent_id> <name> <status>` row as
`agent list`; the default `--json` projection additionally carries
`coding_agent` when the target has a placement (see
[`--full` semantics](#full-semantics)).

## `cafleet message` — Message Broker

All six subcommands require the global `--fleet-id` and act as `--agent-id`.
The task envelope schema is canonical in
[Message envelope](./message-envelope.md); body truncation and the
`--full` / `--quiet` flags are canonical in
[Message Body Truncation](#message-body-truncation) — neither is restated
per subcommand. The compact text render is `[<id> | from:<from> | <ts>]` on
line 1 (plus `kind` / `origin` segments only when present) and the body on
line 2.

### `message send`

| Flag | Required | Notes |
|---|---|---|
| `--agent-id` | yes | Sender. |
| `--to` | yes | Recipient agent id. |
| `--text` | yes | Message body. |
| `--full` / `--quiet` | no | See [Message Body Truncation](#message-body-truncation). |

Text output is `Message sent.` followed by the compact rendered envelope;
`--quiet` prints only the new task id.

### `message broadcast`

| Flag | Required | Notes |
|---|---|---|
| `--agent-id` | yes | Broadcaster. |
| `--text` | yes | Message body. |
| `--full` | no | See [`--full` semantics](#full-semantics). |

Default text output is the one-line summary
`broadcast id=<task_id> recipients=<count>`.

### `message poll`

| Flag | Required | Notes |
|---|---|---|
| `--agent-id` | yes | Recipient whose inbox is fetched. |
| `--full` | no | See [Message Body Truncation](#message-body-truncation). |

Returns only un-acked (`input_required`) deliveries addressed to the agent.
Text output is the compact envelopes blank-line separated; an empty inbox
prints `No messages found.`.

### `message ack`

| Flag | Required | Notes |
|---|---|---|
| `--agent-id` | yes | Recipient acknowledging the message. |
| `--task-id` | yes | Task to acknowledge. |
| `--full` / `--quiet` | no | See [Message Body Truncation](#message-body-truncation). |

Text output is `Message acknowledged.` followed by the compact envelope;
`--quiet` prints only the task id.

### `message cancel`

| Flag | Required | Notes |
|---|---|---|
| `--agent-id` | yes | Sender retracting the message (sender-only). |
| `--task-id` | yes | Task to cancel. |
| `--full` | no | See [Message Body Truncation](#message-body-truncation). |

Text output is `Task canceled.` followed by the compact envelope.

### `message show`

| Flag | Required | Notes |
|---|---|---|
| `--agent-id` | yes | The acting agent (fleet-membership gate). |
| `--task-id` | yes | Task to fetch. |
| `--full` | no | See [Message Body Truncation](#message-body-truncation). |

Text output is the compact envelope alone.

## Member Commands

The `cafleet member` subgroup manages tmux-backed member agents and must be run inside a tmux session. `member create` takes `--agent-id` (the spawning Director's agent ID, validated to equal the fleet root); the other subcommands identify their target by `--member-id`, scoped to the global `--fleet-id`.

### `member create`

| Flag | Required | Notes |
|---|---|---|
| `--agent-id` | yes | Director's agent ID |
| `--name` | yes | Display name of the new member — see [Known asymmetries](../concepts/coding-agents.md#known-asymmetries-intentional-non-goals) for pane-title behavior. |
| `--description` | yes | One-sentence purpose |
| `--coding-agent` | no | One of `claude` (default), `codex`, or `opencode`; exits 1 with `Error: binary <name> not found on PATH` when the chosen binary is not on `PATH` — see [Opencode members](../reference/coding-agents/opencode.md) for the preset note. |
| `--model` | no | Model forwarded to the backend binary's `--model` flag; omitted by default — see [Model selection](../concepts/coding-agents.md#model-selection). |
| `--prompt-file` | no | Absolute path to a UTF-8 file used as the spawn prompt; mutually exclusive with the positional prompt. |
| *(positional, after `--`)* | no | Prompt text for the spawned coding-agent process. All three backends receive the same prompt; the prompt template is backend-neutral. Mutually exclusive with `--prompt-file`. |

#### Spawn command per backend

| Backend | Spawn command (`--model` omitted) | Spawn command (`--model <m>`) |
|---|---|---|
| `claude` | `claude --permission-mode dontAsk --name <member-name> <prompt>` | `claude --permission-mode dontAsk --name <member-name> --model <m> <prompt>` |
| `codex`  | `codex --ask-for-approval never --sandbox workspace-write <prompt>` | `codex --ask-for-approval never --sandbox workspace-write --model <m> <prompt>` |
| `opencode` | `opencode --agent cafleet --prompt <prompt>` | `opencode --agent cafleet --model <m> --prompt <prompt>` |

In all three modes the member's Bash tool is enabled and routine permission prompts auto-resolve — see [Bash routing](../concepts/bash-routing.md) for the fallback protocol, and [Codex members](../reference/coding-agents/codex.md) / [Opencode members](../reference/coding-agents/opencode.md) for backend-specific detail.

#### Spawn-prompt input modes

`cafleet member create` accepts the spawn prompt in three mutually exclusive shapes:

| Inputs | Resulting spawn prompt |
|---|---|
| Neither `--prompt-file` nor positional prompt | The built-in default prompt template, with `{fleet_id}` / `{agent_id}` / `{director_agent_id}` substituted. |
| Positional prompt only | The positional argument(s) joined by spaces, after the same `str.format()` substitution. |
| `--prompt-file PATH` only | The file contents, byte-for-byte, after the same `str.format()` substitution. Surrounding whitespace and trailing newlines are preserved verbatim. |
| Both a positional prompt and `--prompt-file` | Error (exit 2) — see [Error Messages](#error-messages). |

The substituted placeholders are `fleet_id` / `agent_id` / `director_agent_id`. For `--prompt-file`, relative paths, missing files, unreadable files, invalid UTF-8, and empty (zero-byte or whitespace-only) files all produce non-zero-exit errors — see the [Error Messages](#error-messages) table for the full surface. Inline prompts beyond a few KB exceed tmux's argv ceiling (`tmux command failed: command too long` rolls back the registration) — use `--prompt-file` for long prompts.

#### Focus behavior

The spawn always invokes `tmux split-window` with `-d` so the Director's pane and active window keep focus — the new member pane is created in the Director's window but is not made active.

#### Output format

Text (default) — one compact line; `pane` renders `(pending)` when the pane
id has not been patched onto the placement yet:

```
<agent_id> <name> backend=<coding_agent> pane=<pane_id>
```

Text (hidden `--full` flag, accepted but not shown in `--help`) — 6-line block:

```
Member registered and spawned.
  agent_id:  <agent_id>
  name:      <name>
  backend:   <coding_agent>
  pane_id:   <pane_id>
  window_id: <window_id>
```

`--json` returns
`{"agent_id":<id>,"name":"<name>","registered_at":"<iso8601>","placement":{...}}`
where `placement` carries `director_agent_id`, `tmux_session`,
`tmux_window_id`, `tmux_pane_id`, `coding_agent`, and `created_at` (the same
shape as `fleet create`'s `director.placement`).

### `member delete`

| Flag | Required | Notes |
|---|---|---|
| `--member-id` | yes | Target member's agent ID |
| `--force` / `-f` | no | Skip the `/exit` wait. Immediately kill-pane the target, then deregister, then rebalance layout. Exit 0 even if the pane was already gone. |

The only boundary is fleet isolation: a `--member-id` that does not belong to `--fleet-id` resolves to `None` and exits 1 with `Error: Agent <member-id> not found`. There is no caller-auth check. Targeting the root Director is rejected **early — before any tmux pane mutation** — with `Error: cannot deregister the root Director; use 'cafleet fleet delete' instead` (exit 1); this prevents `member delete --member-id <root-director-id>` from injecting `/exit` into (or killing) the Director's own pane. Use `cafleet fleet delete` to tear down the fleet.

#### Polling contract (default path)

The default path sends `/exit` as two separate `tmux send-keys` invocations — literal `/exit` (`-l`), a 0.12 s gap, then Enter — because opencode's slash-command autocomplete popup needs the gap to settle before Enter submits. It then polls `tmux list-panes -a -F "#{pane_id}"` for the target pane every **500 ms** until the pane disappears or a **15.0 s** timeout elapses. A typical coding-agent `/exit` completes in 1–3 s; operators who need faster escalation pass `--force`. On timeout, the pane buffer tail (last 80 lines) is captured via `tmux capture-pane` and printed on stderr, followed by a recovery hint, and the command exits **2**. The timeout output shape:

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
| `1` | Any non-timeout failure: missing fleet, unknown member-id (including cross-fleet), deregister failure (e.g. root-Director guard), a tmux failure sending `/exit` (pre-poll), or a tmux failure while waiting for the pane to disappear (server crash mid-poll). |
| `2` | Default-path timeout — `/exit` was sent, the pane did not disappear within 15.0 s, buffer tail has been printed on stderr. |

### `member list`

Lists every member of the fleet identified by the global `--fleet-id`. The root Director is never surfaced in the output.

| Flag | Required | Notes |
|---|---|---|
| `--activity` | no | Aggregate per-member activity timestamps from the `tasks` table and render `last_sent`, `last_recv`, `last_ack`, and `idle` columns alongside the default member columns; broadcast summary rows are excluded from `last_ack`. |

#### `member list --activity` output {#member-list-activity-output}

```
cafleet --fleet-id 1 member list --activity
3 members:
  agent_id        name      status  last_sent  last_recv  last_ack   idle
  --------------  --------  ------  ---------  ---------  ---------  -----
  4               alice     active  -          12:20:00   12:20:00   14m
  5               bob       active  12:30:11   12:33:02   12:33:02   2m
  6               carol     active  12:34:56   12:34:50   12:34:50   6s
```

`last_sent` is the member's most recent outgoing message; `last_recv` is its most recent delivery; `last_ack` is the most recent delivery it acknowledged (broadcast summaries excluded); `idle` is wall-time since the latest of `last_sent` / `last_recv`.

### `member capture`

| Flag | Required | Notes |
|---|---|---|
| `--member-id` | yes | Target member's agent ID |
| `--lines` | no | Number of trailing lines to capture (default: **30**). |
| `--tail` | no | Alias for `--lines`, for muscle-memory consistency with `tail -n`. |
| `--ansi` / `--no-ansi` | no | Default `--no-ansi`: ANSI escape sequences are stripped and carriage-return redraw fragments cleaned up. Pass `--ansi` to disable post-processing and emit the raw tmux capture. |

### Member targeting and key delivery

`member send-input`, `member exec`, and `member ping` all target a member by `--member-id` (scoped to the global `--fleet-id`) and deliver keystrokes into that member's tmux pane. They share the resolution, key-delivery, and exit-code rules below; each subcommand's own section documents only its unique flags, key sequence, validation, and output.

#### Member resolution

1. Load the active in-fleet target. A cross-fleet, unknown, or inactive (deregistered) `--member-id` all resolve to "not found" and exit 1 with `Error: Agent <member-id> not found`. There is no caller-auth check beyond fleet membership.
2. If the agent has no placement row, exit 1 with ``Error: agent <member-id> has no placement row; it was not spawned via `cafleet member create`.``.
3. If the placement's pane id is `None` (pending placement), exit 1 — each subcommand uses its own "nothing to …" wording (see its section).

The only boundary is fleet isolation: any **active** in-fleet agent **with a placement row** (the root Director included) is a valid `--member-id`.

#### Literal key delivery

Each key sequence is delivered literally — shell meta (`$VAR`, backticks, `$(...)`), key names (`Enter`, `C-c`, `Esc`), backslash-escapes, and multi-byte characters all arrive as plain characters. The CLI runs each `send-keys` with `shell=False`, so no shell ever evaluates the text.

#### Common exit codes

| Exit | When |
|---|---|
| `0` | Dispatch success. |
| `1` | tmux unavailable / `TMUX` env var missing; agent not found (including cross-fleet `--member-id`); missing placement row; pending placement; `tmux send-keys` subprocess failure. |
| `2` | Per-subcommand argument/validation errors (see each subcommand). |

### `member send-input`

Forwards a restricted keystroke to a member's tmux pane. Two input modes, both AskUserQuestion-only — `--freetext` prepends the digit `4` (the "Type something" gate). For shell dispatch use [`member exec`](#member-exec) instead.

Exactly one of the two flags must be supplied.

| Flag | Required | Notes |
|---|---|---|
| `--member-id` | yes | Target member's agent ID |
| `--choice` | one-of | Integer `1`, `2`, or `3`. Sends the matching digit key to the pane (no Enter). Values outside 1–3 are rejected (exit 2). |
| `--freetext` | one-of | Free-text string to type into the "Type something" field. Sends `4`, then the literal text via `tmux send-keys -l`, then `Enter`. AskUserQuestion-only. Rejected if the first non-whitespace character is `!` (use `member exec` for shell dispatch). |

Exactly one of `--choice` / `--freetext` must appear. Supplying zero or both exits 2 with `Error: --choice and --freetext are mutually exclusive; supply exactly one.`.

#### Key sequence sent to the pane

| Invocation | tmux calls issued in order |
|---|---|
| `--choice 1` | `tmux send-keys -t <pane> 1` |
| `--choice 2` | `tmux send-keys -t <pane> 2` |
| `--choice 3` | `tmux send-keys -t <pane> 3` |
| `--freetext "X"` | `tmux send-keys -t <pane> 4` → `tmux send-keys -t <pane> -l "X"` → `tmux send-keys -t <pane> Enter` |

#### Validation rules

| Input | Result |
|---|---|
| Zero or both of `--choice` / `--freetext` | Exit 2 with `Error: --choice and --freetext are mutually exclusive; supply exactly one.` |
| `--choice 0` / `--choice 4` / `--choice a` | Exit 2 — values outside 1–3 are rejected |
| `--freetext ""` (empty) | Allowed — sends `4` + empty literal + `Enter` (submits an empty answer; AskUserQuestion's own UI decides whether to accept it) |
| `--freetext "   "` (whitespace-only) | Allowed — `lstrip()` empties the string before the `startswith("!")` check, so the bang-prefix guard does not fire. |
| `--freetext` whose first non-whitespace character is `!` | Exit 2 with `Error: --freetext may not start with '!' — that triggers the coding agent's shell-execution shortcut. Use 'cafleet member exec' for shell dispatch instead.` |
| `--freetext` containing `\n` or `\r` | Exit 2 with `Error: free text may not contain newlines.` (single-action contract — one prompt submission per call) |

#### Member resolution

Follows [Member targeting and key delivery](#member-targeting-and-key-delivery). On pending placement, exit 1 with `Error: member <member_id> has no pane yet (pending placement) — nothing to send.`.

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

The canonical three-beat workflow (`member capture` → AskUserQuestion → `member send-input`) lives in `skills/cafleet/SKILL.md` under "Answer a member's AskUserQuestion prompt". This page documents only the CLI surface.

### `member exec`

Director-only shell-dispatch primitive. Keystrokes `! <command>` + `Enter` into a member's pane so the coding agent's `!` shortcut runs the command natively (bypassing the member's Bash tool permission system). All three backends (`claude`, `codex`, and `opencode`) honor the leading-`!` shortcut on their input line, so `member exec` works against any backend without modification. The fallback path for the bash-via-Director protocol — see [Bash routing](../concepts/bash-routing.md).

```bash
cafleet --fleet-id <fleet-id> member exec \
  --member-id <member-agent-id> "git log -1 --oneline"
```

| Flag / argument | Required | Notes |
|---|---|---|
| `--member-id` | yes | Target member's agent ID |
| *(positional `COMMAND`)* | yes | Single shell command. Leading and trailing whitespace are stripped before dispatch into the pane (the JSON `command` field and the text echo both reflect the trimmed form). Otherwise pipes, `&&`, `;`, `$(...)`, and backticks are not special-cased — the command is forwarded opaquely. |

#### Key sequence sent to the pane

| Invocation | tmux calls issued in order |
|---|---|
| `member exec "X"` | `tmux send-keys -t <pane> -l "! X"` → `tmux send-keys -t <pane> Enter` |

#### Validation rules

| Input | Result |
|---|---|
| Missing positional `COMMAND` | `Error: Missing argument 'COMMAND'.` (exit 2). |
| `command` empty after `.strip()` (`""` or whitespace-only) | `Error: command may not be empty.` (exit 2). |
| `command` containing `\n` or `\r` | `Error: command may not contain newlines.` (exit 2). |

(tmux-unavailable and binary-not-found errors are common — see [Member targeting and key delivery](#member-targeting-and-key-delivery).)

#### Member resolution

Follows [Member targeting and key delivery](#member-targeting-and-key-delivery). On pending placement, exit 1 with `Error: member <member_id> has no pane yet (pending placement) — nothing to exec.`.

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

Three keys: `member_agent_id`, `pane_id`, `command`.

#### Exit code summary

See [Member targeting and key delivery](#member-targeting-and-key-delivery) for the common exit codes. The argument errors unique to `member exec` exit 2: missing positional `COMMAND`, `command` empty / whitespace-only, and `command` containing `\n` or `\r`.

### `member ping`

Director-only manual inbox-poll nudge. Keystrokes `cafleet --fleet-id <fleet-id> message poll --agent-id <member-id>` + `Enter` into the member's pane so the member drains its inbox via a normal poll. This is the manual re-poke for a pane that missed the broker's automatic on-delivery notification — which keystrokes a 2-line inline preview (not a poll command) — so the two keystroke paths are distinct. As an operator-driven entry-point, failures surface as exit 1 (the auto-fire path swallows failures silently). The action is wholly determined by the subcommand name — there is no positional argument and no operator-controlled keystroke body, which is why this subcommand sits in `permissions.allow` while `member exec` stays in `permissions.ask`.

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
| `member ping` | Types `cafleet --fleet-id <fleet-id> message poll --agent-id <member-id>` + `Enter` into the pane. |

#### Validation rules

| Input | Result |
|---|---|
| Missing `--member-id` | `Error: Missing option '--member-id'.` (exit 2). |

(tmux-unavailable and binary-not-found errors are common — see [Member targeting and key delivery](#member-targeting-and-key-delivery).)

The subcommand has no positional argument and no other flags.

#### Member resolution

Follows [Member targeting and key delivery](#member-targeting-and-key-delivery). On pending placement, exit 1 with `Error: member <member_id> has no pane yet (pending placement) — nothing to ping.`.

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

Two keys: `member_agent_id`, `pane_id`. Failures surface via exit 1.

#### Exit code summary

See [Member targeting and key delivery](#member-targeting-and-key-delivery) for the common exit codes. Unique to `member ping`: a missing `--member-id` exits 2, and a `tmux send-keys` failure exits 1 with `Error: send failed: tmux send-keys did not deliver the poll-trigger keystroke to pane <pane>.`.

## `cafleet monitor` — Supervision Scheduler {#cafleet-monitor}

The `cafleet monitor` subgroup is the per-fleet scheduler that wakes due agents on a fixed cadence — the heartbeat behind a Director's supervision loop. All three subcommands require the global `--fleet-id`. `start` runs the loop in-process (a coding agent launches it as a **background task** and owns its lifetime); `status` and `config` view and edit the schedule. The conceptual model (heartbeat-vs-facilitation boundary, the director-vs-member ping split, the tick-precision floor, single-instance via the DB heartbeat) is canonical on the [Monitoring](../concepts/monitoring.md) concepts page; this page documents the CLI surface.

There is no `monitor stop` command and no detached process: the launching agent stops the loop by stopping its background task (or deleting the monitoring member), and the loop also self-terminates when the fleet is torn down. Launching/stopping the loop is **CLI-only** by nature; the schedule-view and schedule-edit surfaces are at WebUI/CLI parity ([WebUI API](./webui-api.md)).

### `monitor start`

| Flag | Required | Notes |
|---|---|---|
| `--tick` | no | Scan-tick cadence in seconds (`click.IntRange(min=1)`, default **5**). Stored in `monitor_runtime.tick_seconds` so `status` can report it. The tick is the floor on per-agent interval precision — see [Monitoring](../concepts/monitoring.md#cadence-and-tick-precision). |

Runs the `scan → ping due agents → heartbeat → sleep` loop **in-process** via `run_monitor_loop` — a coding agent launches it as a background task (the loop blocks the task). On startup it runs the tmux precondition guard (the same `TMUX`-env check the `member` commands use), then atomically claims the single-instance `monitor_runtime` row, installs `SIGTERM`/`SIGINT` handlers (a clean stop clears the row), and loops until signalled or the fleet is torn down (`monitor_tick` returns `STOP` once the fleet is soft-deleted). There is no detached subprocess, no PID file, and no log file — the loop writes to the launching task's own stdout.

Every enrolled agent — Director and member alike — is pinged unconditionally once its interval has elapsed (the ping is **not** gated on the agent having pending inbox items; `pending_count` is informational, shown in `status`). The keystroke differs by role: the **Director** receives a bare `cafleet … message poll`, while a **member** receives a single-line *resume nudge* (the poll command plus a review-your-task-and-continue instruction), so a stopped member resumes rather than going idle on an empty inbox. Each dispatched ping is logged to stdout as `<iso-ts> ping agent <id> (<name>)`, so the launching background task's output shows live heartbeat activity.

Exit codes: `0` clean exit (signalled, or the fleet was torn down); `1` already running (`monitor already running for fleet N`), unknown or soft-deleted fleet, or tmux unreachable; `2` click usage errors (e.g. `--tick 0`).

### `monitor status`

No flags beyond the global `--fleet-id`. Reports runtime liveness derived from the DB heartbeat (true even when the process died silently) plus the per-agent schedule table.

Text output:

```
monitor: running (pid 4821, last tick 2s ago, tick 5s, started 2026-06-13T04:50:00+00:00)
  agent_id  name         role      interval  last_ping             enabled  pending
  --------  -----------  --------  --------  -------------------  -------  -------
  2         Director     director  60s       2026-06-13T04:51:00   yes      0
  4         alice        member    60s       -                    yes      2
  5         bob          member    30s       2026-06-13T04:50:30   no       0
```

When no monitor is running the first line reads `monitor: stopped`; the schedule table still renders. JSON output:

```json
{
  "runtime": {"running": true, "pid": 4821, "tick_seconds": 5,
              "last_tick_at": "2026-06-13T04:51:02+00:00", "last_tick_age_seconds": 2,
              "started_at": "2026-06-13T04:50:00+00:00"},
  "agents": [
    {"agent_id": 4, "name": "alice", "role": "member", "interval_seconds": 60,
     "last_ping_at": null, "enabled": true, "pending_count": 2}
  ]
}
```

Exit `0` (`2` for click usage errors). On an unknown or soft-deleted fleet, exit 1.

### `monitor config`

| Flag | Required | Notes |
|---|---|---|
| `--agent-id` | yes | The agent whose schedule is shown or edited. Must be an enrolled, in-fleet agent. |
| `--interval` | no | New ping interval in seconds (`click.IntRange(min=1)` — the same `>= 1` lower bound the WebUI `PATCH` enforces). |
| `--enable` / `--disable` | no | Enable or disable monitoring for the agent. Mutually exclusive. |

With no edit flag, prints the agent's current config. With `--interval` / `--enable` / `--disable`, applies the update and prints the new config. Exits 1 if the agent is not in the fleet or not enrolled (the Administrator and card-only agents are never enrolled). `--enable` and `--disable` together exit 2.

Text output:

```
agent 4: interval 60s, enabled, last_ping 2026-06-13T04:51:00
```

JSON output: `{"agent_id": 4, "interval_seconds": 60, "last_ping_at": "<iso8601>|null", "enabled": true}`.

## Error Messages

| Situation | Error Message |
|---|---|
| Missing `--fleet-id` on a client/member subcommand | `Error: --fleet-id <int> is required for this subcommand. Create a fleet with 'cafleet fleet create' and pass its id.` |
| Missing `--agent-id` | `Error: Missing option '--agent-id'.` (exit 2) |
| `fleet create` run outside a tmux session | `Error: cafleet fleet create must be run inside a tmux session` (exit 1; no DB writes) |
| `fleet delete` on unknown fleet_id | `Error: fleet 'X' not found.` (exit 1) |
| `agent register` into a soft-deleted fleet | `Error: fleet X is deleted` (exit 1) |
| `agent deregister` against the root Director's `agent_id` | `Error: cannot deregister the root Director; use 'cafleet fleet delete' instead` (exit 2) |
| `agent deregister` against the Administrator's `agent_id` | `Error: Administrator cannot be deregistered` (exit 1) |
| `agent show` / `agent deregister` / `message send` / `message poll` / `message ack` / `message cancel` / `message show` with an `--agent-id` that is not a member of `--fleet-id` | `Error: agent <id> is not a member of fleet <fleet-id>.` (exit 1) — the fleet-membership gate runs before any read/write operation. Also fires for unknown `--agent-id` (the gate cannot tell "unknown" from "in a different fleet" apart and treats both as not-a-member). |
| `member send-input` with zero or both of `--choice` / `--freetext` | `Error: --choice and --freetext are mutually exclusive; supply exactly one.` (exit 2) |
| `member send-input --choice` outside `1..3` | Values outside 1–3 are rejected (exit 2) |
| `member send-input --freetext` whose first non-whitespace character is `!` | `Error: --freetext may not start with '!' — that triggers the coding agent's shell-execution shortcut. Use 'cafleet member exec' for shell dispatch instead.` (exit 2) |
| `member send-input --freetext` with `\n` or `\r` | `Error: free text may not contain newlines.` (exit 2) |
| `member send-input` on a member with pending placement | `Error: member <id> has no pane yet (pending placement) — nothing to send.` (exit 1) |
| `member exec` with missing positional `COMMAND` | `Error: Missing argument 'COMMAND'.` (exit 2) |
| `member exec ""` (empty / whitespace-only) | `Error: command may not be empty.` (exit 2) |
| `member exec` with `\n` or `\r` | `Error: command may not contain newlines.` (exit 2) |
| `member exec` on a member with pending placement | `Error: member <id> has no pane yet (pending placement) — nothing to exec.` (exit 1) |
| `member ping` on a member with pending placement | `Error: member <id> has no pane yet (pending placement) — nothing to ping.` (exit 1) |
| `member ping` when `tmux send-keys` fails | `Error: send failed: tmux send-keys did not deliver the poll-trigger keystroke to pane <pane>.` (exit 1) |
| `member create` with both `--prompt-file` and a positional prompt argument | `Error: --prompt-file and the positional prompt argument are mutually exclusive.` (exit 2) |
| `member create --prompt-file` with a relative path | ``Error: --prompt-file requires an absolute path (got '<input>'). Resolve relative paths against your BASE first — see the `cafleet-base-dir` skill.`` (exit 2) |
| `member create --prompt-file` to a non-existent path or non-regular file (e.g. directory) | `Error: --prompt-file <path>: file does not exist or is not a regular file.` (exit 1) |
| `member create --prompt-file` to an unreadable file | `Error: --prompt-file <path>: file is not readable.` (exit 1) |
| `member create --prompt-file` to a file containing invalid UTF-8 | `Error: --prompt-file <path>: file is not valid UTF-8.` (exit 1) |
| `member create --prompt-file` to a zero-byte or whitespace-only file | `Error: --prompt-file <path>: file is empty.` (exit 1) |
| `member create --coding-agent opencode --model` with a value violating the `<provider-id>/<model-id>` format | `Error: --model for the opencode backend must be '<provider-id>/<model-id>' (got '<value>').` (exit 2; fires before any agent registration or tmux side effect) |
| `monitor start` for a fleet that already has a live monitor | `Error: monitor already running for fleet <id>` (exit 1) |
| `monitor start` / `monitor status` against an unknown or soft-deleted fleet | `Error: fleet <id> not found` (exit 1) |
| `monitor config` with both `--enable` and `--disable` | `Error: --enable and --disable are mutually exclusive.` (exit 2) |
| `monitor config` against an agent not in the fleet or not enrolled | `Error: agent <id> is not enrolled in monitoring for fleet <fleet-id>.` (exit 1) |

