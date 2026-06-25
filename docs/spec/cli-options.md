---
icon: lucide/square-terminal
---

# CLI options

How the unified CAFleet CLI (`cafleet`) accepts configuration parameters.

## Subcommand summary

One row per subcommand. "Identity flag" is the per-subcommand option naming the acting agent (requester) or the target agent — both spelled `--agent-id`, with the polarity fixed per command (see [Agent ID](#agent-id)). In the `--fleet-id` column, **yes** means the flag is a required per-subcommand option (placed after the subcommand name); **no** means the subcommand rejects `--fleet-id` with `No such option`.

| Subcommand | Purpose | `--fleet-id` | Identity flag | Section |
|---|---|---|---|---|
| `setup` | Install the skills + create the database schema | no | none | [setup](#cafleet-setup) |
| `doctor` | Print the calling pane's tmux identifiers | no | none | [doctor](#cafleet-doctor) |
| `server` | Start the admin WebUI server | no | none | [server](#cafleet-server) |
| `fleet create` | Create a fleet with its root Director and Administrator | no | none | [fleet create](#fleet-create) |
| `fleet list` | List non-deleted fleets | no | none | [fleet list](#fleet-list) |
| `fleet show` | Show one fleet (soft-deleted included) | yes | none | [fleet show](#fleet-show) |
| `fleet delete` | Soft-delete a fleet and deregister its agents | yes | none | [fleet delete](#fleet-delete) |
| `agent register` | Register a new agent | yes | none | [agent register](#agent-register) |
| `agent list` | List the fleet's agents | yes | none | [agent list](#agent-list) |
| `agent show` | Show one agent's detail | yes | `--agent-id` (requester) | [agent show](#agent-show) |
| `agent deregister` | Tear down an agent's pane (if any) and deregister it | yes | `--agent-id` (target) | [agent deregister](#agent-deregister) |
| `agent spawn` | Register an agent and spawn its coding-agent pane | yes | `--agent-id` (Director) | [agent spawn](#agent-spawn) |
| `message send` | Send a unicast message | yes | `--agent-id` (requester) | [message send](#message-send) |
| `message broadcast` | Broadcast a message to all fleet agents | yes | `--agent-id` (requester) | [message broadcast](#message-broadcast) |
| `message poll` | Fetch un-acked incoming messages | yes | `--agent-id` (requester) | [message poll](#message-poll) |
| `message ack` | Acknowledge a received message | yes | `--agent-id` (requester) | [message ack](#message-ack) |
| `message cancel` | Retract an un-acked sent message | yes | `--agent-id` (requester) | [message cancel](#message-cancel) |
| `message show` | Show one task | yes | `--agent-id` (requester) | [message show](#message-show) |
| `pane capture` | Capture the tail of an agent's pane | yes | `--agent-id` (target) | [pane capture](#pane-capture) |
| `pane input` | Forward a restricted keystroke to an agent's pane | yes | `--agent-id` (target) | [pane input](#pane-input) |
| `pane exec` | Dispatch a shell command into an agent's pane | yes | `--agent-id` (target) | [pane exec](#pane-exec) |
| `pane wake` | Re-poll an agent's inbox (`--poll-only`) or deliver an ACKable task + preview (`--message`) | yes | `--agent-id` (target) | [pane wake](#pane-wake) |
| `monitor start` | Run the per-fleet scheduler loop in-process (launch as a background task) | yes | none | [monitor start](#monitor-start) |
| `monitor status` | Show monitor liveness and the per-agent schedule | yes | none | [monitor status](#monitor-status) |
| `monitor config` | Show or edit an agent's monitor schedule | yes | `--agent-id` | [monitor config](#monitor-config) |

## Option Source Matrix

Each parameter has exactly one input source:

| Parameter | Source |
|---|---|
| Fleet ID | `--fleet-id <int>` per-subcommand option (placed after the subcommand name), defaulting from `CAFLEET_FLEET_ID` when set |
| Database URL | `CAFLEET_DATABASE_URL` env var (optional) — see [config](../api/config.md) for its default and the absolute-path requirement. |
| Agent ID | `--agent-id <int>` subcommand option |
| JSON output | `--json` global flag |

> **`--fleet-id` is a literal CLI flag** (with an optional `CAFLEET_FLEET_ID` env default) — see [Fleet ID](#fleet-id) for why agents pass it literally, and how `permissions.allow` matching depends on the canonical flag order.

Create a fleet first if you don't have one:

```bash
cafleet fleet create --label "my-project"
# → prints the fleet_id
```

Then pass the printed id as `--fleet-id <id>` on every fleet-scoped command (or export `CAFLEET_FLEET_ID=<id>` to default it).

## Global Options

Placed **before** the subcommand:

| Flag | Required | Notes |
|---|---|---|
| `--json` | no | Emit JSON output. JSON encoding is compact single-line JSON; non-ASCII (like the `…` truncation suffix) is emitted as UTF-8, not escaped. |
| `--version` | no | Print `cafleet <version>` and exit 0. Bypasses the `--fleet-id` requirement. |

### `--full` semantics (cross-subcommand escape hatch) {#full-semantics}

`--full` is the global "give me every field cafleet has, untruncated, unfiltered" escape hatch — a **documented** flag on every subcommand that accepts it. A single flag covers four overloaded surfaces:

| Subcommand | Default behavior | `--full` behavior |
|---|---|---|
| `message {send,poll,ack,cancel,show}` | `text` truncated to `CAFLEET_MAX_TEXT_LEN` codepoints + `…` suffix (see [Message Body Truncation](#message-body-truncation)). Compact rendered envelope: `id`, `from`, `ts`, `text`, plus `kind`/`origin` only when present (ids are full integers). | Untruncated `text`. In `--json`, emits the full typed-column task dict (`task_id`, `context_id`, `from_agent_id`, `to_agent_id`, `type`, `status_state`, `status_timestamp`, `origin_task_id`, `text`); in text mode, switches to the verbose labeled block (see [Message envelope](./message-envelope.md#text-mode)). |
| `message broadcast` | One-line summary (`broadcast id=<id> recipients=<N> delivered=<k>`). | Renders the single `broadcast_summary` task as the full verbose envelope (typed-column dict in `--json`) instead of the one-line summary. It never adds per-recipient envelopes or a `recipient_ids` list — the response is always that one summary task plus the `recipients` and `delivered` counts. |
| `agent list` / `agent show` | One row per agent (`<id> <name> <status>`); `description` truncated to 60 codepoints. JSON projects each agent to `id` / `name` / `description` / `status`. `agent show` additionally emits `coding_agent` when the agent has a placement; `agent list` shows a placement/pane column for placed agents. | Four-line per-agent block: full `agent_id`, `name`, `description` (still truncated to 60 codepoints), `status`. JSON returns the broker agent dict unchanged. No `agent_card_json` — the agent surfaces never emit it. |
| `pane capture` | Default `--lines 20`; ANSI escape sequences stripped in post-process unless `--ansi` is supplied. | No effect on `--lines` (use `--lines N` explicitly); no effect on ANSI stripping (use `--ansi` explicitly). `--full` is accepted on `pane capture` for surface consistency but is a no-op there. |

## Fleet ID (`--fleet-id`) {#fleet-id}

`--fleet-id` is a **per-subcommand option** (not a top-level option). It names the fleet the command acts on and is placed immediately **after** the subcommand name (the canonical position), ahead of the other flags — e.g. `cafleet message poll --fleet-id <id> --agent-id <id>`. It is typed `int`; a non-integer fails with Click's standard `Error: Invalid value for '--fleet-id': '<x>' is not a valid integer.` (exit 2). Every `agent *`, `pane *`, `message *`, and `monitor *` command, plus `fleet show` and `fleet delete`, carries it; `setup`, `doctor`, `server`, `fleet create`, and `fleet list` do **not** accept it and reject it with `No such option: --fleet-id`. Which subcommand takes it is in the [Subcommand summary](#subcommand-summary).

`--fleet-id` is a **plain required option**: a missing value is a parser-native missing-required-option error — `Error: Missing option '--fleet-id'.` (exit 2) — with no custom callback. When `CAFLEET_FLEET_ID` is set in the environment it supplies the **default** for `--fleet-id`, so the flag need not be retyped on every call; an explicit `--fleet-id` overrides it, and a non-integer `CAFLEET_FLEET_ID` fails at parse time (exit 2). A spawned member receives `CAFLEET_FLEET_ID` in its pane environment (see [Coding agents](../concepts/coding-agents.md)), so its fleet is unambiguous without repeating the flag.

Agents still pass `--fleet-id` as a literal flag because Claude Code's `permissions.allow` matches Bash invocations as literal command strings: a literal `--fleet-id <int>` keeps the invocation a fixed string an allow pattern can match, while a shell-expanded variable (`$FLEET_ID`) breaks the match and forces per-invocation permission prompts that interrupt agent work. Substitute the literal integer ids printed by `cafleet fleet create` and `cafleet agent register` — never shell variables to hold them. Matching also depends on the canonical flag order (`--fleet-id` first, immediately after the subcommand name); a different order does not match — see [`permissions.allow` coverage](#permissionsallow-coverage).

## Agent ID (`--agent-id`)

`--agent-id` is a **per-subcommand option** (not a global option). It is typed `int`; a non-integer fails with Click's standard `Error: Invalid value for '--agent-id': '<x>' is not a valid integer.` (exit 2). The same `type=int` applies to every id option — `--to`, `--id` (`agent show`), `--from` (`pane wake --message`), and `--task-id` — so each rejects a non-integer the same way. Ids are short by construction (DB-assigned integers, typically 1–4 digits), so they are pasted in full; there is no prefix resolution.

**Polarity is per-command.** `--agent-id` names the **calling agent (requester)** on `agent show` and every `message *` command, but the **target agent** on `agent deregister` and every `pane *` command. This polarity is fixed by each command and is why only `--fleet-id` (never `--agent-id`) auto-defaults from the environment. Which subcommand takes which identity flag is in the [Subcommand summary](#subcommand-summary).

## `permissions.allow` coverage

The allow set is generated mechanically, one `Bash(...)` pattern per allow-listed subcommand, by this rule:

- **One pattern per subcommand**, each matching the canonical `--fleet-id`-first flag order (see [Fleet ID](#fleet-id)) — `Bash(cafleet <grp> <cmd> --fleet-id *)`. A different flag order does not match and prompts.
- **`pane exec` is excluded** so it stays under `permissions.ask` — its positional command body is operator-controlled.
- **Each subcommand an agent runs with `--json` needs a companion pattern**, because `--json` is a top-level option that precedes the subcommand name and breaks the prefix: `Bash(cafleet --json <grp> <cmd> --fleet-id *)`.

Three representative patterns:

```
Bash(cafleet message poll --fleet-id *)
Bash(cafleet agent spawn --fleet-id *)
Bash(cafleet --json message poll --fleet-id *)
```

Expanding the rule over every allow-listed subcommand (the fleet-scoped leaf subcommands minus `pane exec`, plus a `--json` companion per JSON-invoked subcommand) yields the full set mechanically. Apply the patterns to your user-level `~/.claude/settings.json` manually; the repo does **not** ship a committed `.claude/settings.json` permissions block.

## Message Body Truncation

The five subcommands that emit a user-supplied delivery body — `cafleet message {send,poll,ack,cancel,show}` — truncate the `text` body to the first `CAFLEET_MAX_TEXT_LEN` Unicode codepoints (default `200`) plus a single `…` codepoint suffix by default. The truncation applies uniformly in both text and `--json` output.

| Variable | Settings field | Default | Notes |
|---|---|---|---|
| `CAFLEET_MAX_TEXT_LEN` | `max_text_len` | `200` | Maximum codepoint length of the rendered `text` body before the `…` suffix is appended. It also bounds the broker's inline-preview truncation before the preview is keystroked into a recipient's tmux pane. (`agent.description` truncation uses a separate hard-coded 60-codepoint limit, independent of this env var.) |

The suffix is the single Unicode codepoint `…` (U+2026 HORIZONTAL ELLIPSIS) — exactly one codepoint with no count and no companion `text_length` field.

`cafleet message broadcast` is different — the broker returns a `broadcast_summary` task whose top-level `text` column is a broker-generated summary string (e.g. `Broadcast sent to N recipients`), not the original body. By default the summary renders as the one-line `broadcast id=<id> recipients=<N> delivered=<k>` — see the [`--full` semantics](#full-semantics) `message broadcast` row.

The table describes the resulting `text` value AFTER truncation. Text mode omits the `text:` line entirely when the resulting value is empty, while `--json` always includes it.

| Input `text` | Default output | `--full` output |
|---|---|---|
| `None` / not present | not present | not present |
| `""` | text mode: `text:` line omitted, `--json`: empty string | text mode: `text:` line omitted, `--json`: empty string |
| length ≤ `CAFLEET_MAX_TEXT_LEN` codepoints | unchanged | unchanged |
| length > `CAFLEET_MAX_TEXT_LEN` codepoints | `text[:CAFLEET_MAX_TEXT_LEN] + "…"` | unchanged |

| Flag | Required | Notes |
|---|---|---|
| `--full` | no | Documented per-subcommand option (placed after the subcommand name, like `--agent-id` and `--task-id`). Disables truncation; emits the full message body and the full typed-column envelope. Composes orthogonally with `--json`. See [`--full` semantics](#full-semantics) for the cross-subcommand summary. |

There is no `--quiet` flag on any subcommand.

Length is measured in Python `str` codepoints, never bytes — multibyte characters are never split.

```bash
cafleet message poll --fleet-id <fleet-id> --agent-id <my-agent-id>          # default: text truncated to 200 cp + "…"
cafleet message poll --fleet-id <fleet-id> --agent-id <my-agent-id> --full   # full body
```

This applies to CLI emit sites only. FastAPI `/api/*` responses (see [webui-api.md](./webui-api.md)) are unchanged — the WebUI is human-facing and renders full bodies. `agent.description`, `skills[].description`, `agent_card_json` sub-fields, and `pane capture` content are also untouched.

## `cafleet setup` — One-Step Onboarding {#cafleet-setup}

Installs the coding-agent skills **and** creates the database schema in one command —
the recommended end-user onboarding path (see
[Install](../get-started/install.md)). It always runs both halves; they are
independent, so a failure in one does not abort the other, and the command
exits non-zero if either half fails. `setup` is the single schema-management
entry point; there is no separate `db init` command.

The skills installed always match the **installed `cafleet` CLI version**:
setup reads that version, downloads the matching
`cafleet-skills-v<version>.zip` asset from the corresponding GitHub Release of
`himkt/cafleet`, and extracts the three skill directories (`cafleet`,
`cafleet-design-doc`, `cafleet-research`) into each target agent home,
replacing any existing copy. The database half creates the single-baseline
schema fresh (`CREATE TABLE IF NOT EXISTS`, so re-running against an
already-initialized DB is a no-op); there is no migration chain or in-place
upgrade path (see [Storage](../concepts/storage.md)).

`cafleet setup` does NOT accept `--fleet-id`. Supplying it exits 2 with
`No such option: --fleet-id`, matching the `fleet create` / `fleet list` /
`server` / `doctor` pattern.

| Flag | Required | Notes |
|---|---|---|
| `--agent` | no | One of `claude`, `codex`, or `opencode`; repeatable (`multiple=True`), duplicate values deduped silently. Scopes the **skills** targets to exactly the named agents; the database half runs regardless. Omitted → auto-detect every agent whose home directory exists (`~/.claude`, `~/.codex`, `~/.config/opencode`). An explicitly named agent's home/skills tree is created if missing; auto-detect installs only where the home already exists. An unknown value fails Click's choice check (exit 2). |

`setup` has no skills-only / db-only toggle. Failures surface as runtime error
messages — no release for the installed version, a missing or malformed skills
asset, the GitHub API unreachable, an unwritable target, or zero detected agent
homes — so there is no exit-codes table.

## `cafleet fleet` — Fleet Management

The `cafleet fleet` subgroup manages fleets. These commands write directly to SQLite — the broker server does not need to be running. `fleet show` and `fleet delete` take the required `--fleet-id`; `fleet create` and `fleet list` do not.

### `fleet create`

| Flag | Required | Notes |
|---|---|---|
| `--label` | no | Free-form text label for the fleet |
| `--coding-agent` | no | One of `claude` (default), `codex`, or `opencode`, recorded as the root Director's placement `coding_agent` — see [Coding agents](../concepts/coding-agents.md). |
| `--json` | no | Output as JSON |
| `--full` | no | Documented flag: switches the non-JSON output from the compact one-line form to the 7-line block below. |

There are no `--name` / `--description` flags. The root Director's name and description are hardcoded (`name="Director"`, `description="Root Director for this fleet"`).

Creates a new fleet with a DB-assigned integer identifier. **Must be run inside a tmux session** — outside tmux the command exits 1 with `Error: cafleet fleet create must be run inside a tmux session` and writes nothing to the DB. It creates the fleet, its root Director (and placement), and the built-in Administrator atomically (all-or-nothing) — see [data-model.md](./data-model.md) for the Administrator's distinguishing `agent_card_json.cafleet.kind` flag.

**Non-JSON output (default)** — one compact line carrying the new `fleet_id`, the root Director's `agent_id`, and the built-in Administrator's `agent_id`:

```
<fleet_id> director=<director_agent_id> admin=<administrator_agent_id>
```

**Non-JSON output (`--full`)** — line 1 is `fleet_id`, line 2 is the root Director's `agent_id`:

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

Both the root Director and the built-in Administrator are protected from `agent deregister` — see [Error Messages](#error-messages).

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

| Flag | Required | Notes |
|---|---|---|
| `--fleet-id` | yes | The fleet to show |
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

| Flag | Required | Notes |
|---|---|---|
| `--fleet-id` | yes | The fleet to delete |

Soft-deletes a fleet in one transaction: it stamps the fleet as deleted, deregisters every agent that was active at the moment of deletion (root Director included), and removes their placement rows. Tasks are untouched — the message history remains queryable. Output:

```
Deleted fleet <fleet_id>. Deregistered N agents.
```

`N` counts every agent that was active at the moment of deletion (root Director included). On re-run against an already-deleted fleet, the command prints `Deleted fleet <fleet_id>. Deregistered 0 agents.` and exits 0 — the command is idempotent.

There is no `--force` flag. Calling `fleet delete` on an unknown `fleet_id` exits 1 with `Error: fleet 'X' not found.`.

Agent tmux panes spawned by `cafleet agent spawn` are **not** automatically closed by `fleet delete`. For a clean teardown, call `cafleet agent deregister` per agent first (which sends the backend exit keystroke to the pane). If a pane refuses to close (e.g. blocked on a confirmation prompt), rerun `cafleet agent deregister` with `--force`, which kill-panes the target, sweeps the placement, and rebalances the layout.

## `cafleet doctor` — Placement Diagnostics {#cafleet-doctor}

Prints the calling pane's tmux session/window/pane identifiers (plus `$TMUX_PANE`) for operators diagnosing placement issues without reaching for raw tmux commands.

| Flag | Required | Notes |
|---|---|---|
| `--json` | no | Top-level `--json`, written ahead of the subcommand name (same pattern as every other CLI command). |

Environment requirements:

- `TMUX` env var must be set — the command rejects otherwise with `Error: cafleet tmux-pane commands must be run inside a tmux session` (the same message used by the `pane` / `agent spawn` tmux guard).
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

`cafleet server` does NOT accept `--fleet-id`. Supplying it exits 2 with `No such option: --fleet-id`, matching the `setup` / `fleet create` / `doctor` pattern.

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

# --fleet-id is not accepted: exits 2 with "No such option: --fleet-id"
cafleet server --fleet-id 1
```

## `cafleet agent` — Agent Registry + Lifecycle

The `agent` group is the single home for the agent registry and pane-bound lifecycle: `register | list | show | deregister | spawn`. A "member" is just an agent with a placement, so the registry CRUD and the spawn/teardown of pane-bound agents both live here. All five subcommands require the per-subcommand `--fleet-id`. The default-vs-`--full` output projection shared by `agent list` and `agent show` is documented in [`--full` semantics](#full-semantics) and is not restated per subcommand. `agent spawn` and `agent deregister` interact with tmux and must be run inside a tmux session.

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

### `agent list`

| Flag | Required | Notes |
|---|---|---|
| `--full` | no | See [`--full` semantics](#full-semantics). |
| `--activity` | no | Aggregate per-agent activity timestamps from the `tasks` table and render `last_sent`, `last_recv`, `last_ack`, and `idle` columns; broadcast summary rows are excluded from `last_ack`. |

No identity flag — the listing is scoped by the per-subcommand `--fleet-id` alone.
Default text output is one `<agent_id> <name> <status>` line per active agent,
blank-line separated, with a placement/pane column for placed agents; an empty
fleet prints `No agents found.`:

```
2 Director active

3 Administrator active

4 demo-member active
```

#### `agent list --activity` output {#agent-list-activity-output}

```
cafleet agent list --fleet-id 1 --activity
3 members:
  agent_id        name      status  last_sent  last_recv  last_ack   idle
  --------------  --------  ------  ---------  ---------  ---------  -----
  4               alice     active  -          12:20:00   12:20:00   14m
  5               bob       active  12:30:11   12:33:02   12:33:02   2m
  6               carol     active  12:34:56   12:34:50   12:34:50   6s
```

`last_sent` is the agent's most recent outgoing message; `last_recv` is its most recent delivery; `last_ack` is the most recent delivery it acknowledged (broadcast summaries excluded); `idle` is wall-time since the latest of `last_sent` / `last_recv`. An absent cell renders as a single ASCII `-`.

### `agent show`

| Flag | Required | Notes |
|---|---|---|
| `--agent-id` | yes | The acting agent (requester; fleet-membership gate). |
| `--id` | yes | The target agent to show. |
| `--full` | no | See [`--full` semantics](#full-semantics). |

Default text output is the same one-line `<agent_id> <name> <status>` row as
`agent list`; the default `--json` projection additionally carries
`coding_agent` when the target has a placement (see
[`--full` semantics](#full-semantics)). A target id that does not exist exits 1
with `Error: Agent <id> not found`.

### `agent deregister`

| Flag | Required | Notes |
|---|---|---|
| `--agent-id` | yes | The agent to deregister (target). |
| `--force` / `-f` | no | When the target has a pane, skip the graceful close wait: immediately kill-pane the target, then deregister, then rebalance layout. Exit 0 even if the pane was already gone. |

Tears down the target's pane (when one exists) and soft-deletes the agent. One unified error model — every failure path exits 1.

- **No pane** (registry-only agent, or pending placement) — registry soft-delete. If nothing was deregistered, exit 1 with `Error: agent <agent_id> not found or already deregistered.`. Success text: `Agent deregistered successfully.`.
- **Has a pane (default path)** — send the backend exit keystroke, then poll for the pane to disappear (every 500 ms, up to a 15.0 s timeout). Pane gone → deregister, header `Agent deregistered successfully.`. A typical coding-agent exit completes in 1–3 s; operators who need faster escalation pass `--force`. On timeout, the pane buffer tail (last 80 lines) is captured and printed on stderr with a recovery hint, and the command exits **1**.
- **Has a pane (`--force`)** — kill-pane immediately, then deregister; header `Agent deregistered (--force).`.

The only boundary is fleet isolation: an `--agent-id` that does not belong to `--fleet-id` resolves to `None` and exits 1 with `Error: Agent <agent-id> not found`. There is no caller-auth check. Targeting the root Director is rejected **early — before any tmux pane mutation** — with `Error: cannot deregister the root Director; use 'cafleet fleet delete' instead` (exit 1); this prevents `agent deregister --agent-id <root-director-id>` from injecting the exit keystroke into (or killing) the Director's own pane. The Administrator is rejected with `Error: Administrator cannot be deregistered` (exit 1). Use `cafleet fleet delete` to tear down the whole fleet.

JSON output: `{agent_id, pane_status}`, where `pane_status` is `(pending — no pane)`, `<pane_id> (closed)`, `<pane_id> (killed)`, or `<pane_id> (timeout)`.

#### Timeout output shape

```
Error: pane %7 did not close within 15.0s after the exit keystroke.
--- pane %7 tail (last 80 lines) ---
<captured terminal buffer>
---
Recovery: inspect with `cafleet pane capture`, answer any prompt with `cafleet pane input`, then re-run `cafleet agent deregister`. Or re-run with `--force` to skip the wait and kill the pane.
```

#### Exit codes

| Exit | When |
|---|---|
| `0` | Success — default-path pane-gone confirmed, `--force` pane killed, or no-pane deregister. |
| `1` | Any failure: missing fleet, unknown agent-id (including cross-fleet), nothing deregistered, the root-Director or Administrator guard, a tmux failure sending the exit keystroke (pre-poll), a tmux failure while waiting for the pane to disappear, or the 15.0 s default-path timeout. |

### `agent spawn`

The one genuinely distinct lifecycle op: register an agent **and** spawn its coding-agent pane.

| Flag | Required | Notes |
|---|---|---|
| `--agent-id` | yes | Director's agent ID. |
| `--name` | yes | Display name of the new agent — see [Known asymmetries](../concepts/coding-agents.md#known-asymmetries-intentional-non-goals) for pane-title behavior. |
| `--description` | yes | One-sentence purpose. |
| `--coding-agent` | no | One of `claude`, `codex`, or `opencode`; an ordinary member defaults to `claude` and a `--role monitor` member inherits the spawning Director's backend (an explicit value always wins). Backend resolution and the per-backend spawn argv live in [Coding agents](../concepts/coding-agents.md). Exits 1 with `Error: binary <name> not found on PATH` when the chosen binary is not on `PATH`. |
| `--model` | no | Model forwarded to the backend binary's `--model` flag; omitted by default — see [Model selection](../concepts/coding-agents.md#model-selection). |
| `--role` | no | One of `member` (default) or `monitor`. `monitor` spawns the fleet's dedicated **monitoring member** (sets `agent_card_json.cafleet.kind == "monitoring-member"`, skips `monitor_config` enrollment, and inherits the Director's backend when `--coding-agent` is omitted); an ordinary `member` is enrolled as a watched agent. The LLM is still chosen by `--model` (the Director passes `--model haiku`). A second `--role monitor` spawn in the same fleet is rejected — see [Error Messages](#error-messages). See [Monitoring](../concepts/monitoring.md#the-monitoring-member). |
| `--full` | no | Documented flag: switches the non-JSON output to the 6-line block below. |
| `--prompt-file` | no | Absolute path to a UTF-8 file used as the spawn prompt; mutually exclusive with the positional prompt. |
| *(positional, after `--`)* | no | Prompt text for the spawned coding-agent process, delivered **verbatim**. All three backends receive the same prompt; the prompt template is backend-neutral. Mutually exclusive with `--prompt-file`. |

#### Spawn command per backend

The per-backend spawn argv and auto-approval flags live in the Backend-resolution table on [Coding agents](../concepts/coding-agents.md). In all three modes the member's Bash tool is enabled and routine permission prompts auto-resolve — see [Bash routing](../concepts/bash-routing.md) for the fallback protocol.

#### Spawn-prompt input modes

`cafleet agent spawn` accepts the spawn prompt in three mutually exclusive shapes, **all delivered verbatim** (there is no brace `{placeholder}` substitution):

| Inputs | Resulting spawn prompt |
|---|---|
| Neither `--prompt-file` nor positional prompt | The built-in default prompt template, verbatim. |
| Positional prompt only | The positional argument(s) joined by spaces, verbatim. |
| `--prompt-file PATH` only | The file contents, byte-for-byte. Surrounding whitespace and trailing newlines are preserved. |
| Both a positional prompt and `--prompt-file` | Error (exit 2) — see [Error Messages](#error-messages). |

Identity reaches the spawned agent as **environment variables** injected into its pane — `CAFLEET_FLEET_ID`, `CAFLEET_AGENT_ID` (the spawned agent's own id), and `CAFLEET_DIRECTOR_AGENT_ID` (its Director) — alongside the forwarded `CAFLEET_DATABASE_URL`. Only `CAFLEET_FLEET_ID` auto-defaults `--fleet-id`; the agent reads `$CAFLEET_AGENT_ID` / `$CAFLEET_DIRECTOR_AGENT_ID` and passes them explicitly (see [Coding agents](../concepts/coding-agents.md)). For `--prompt-file`, relative paths, missing files, unreadable files, invalid UTF-8, and empty (zero-byte or whitespace-only) files all produce non-zero-exit errors — see the [Error Messages](#error-messages) table for the full surface. Inline prompts beyond a few KB exceed tmux's argv ceiling (`tmux command failed: command too long` rolls back the registration) — use `--prompt-file` for long prompts.

#### Focus behavior

The spawn always invokes `tmux split-window` with `-d` so the Director's pane and active window keep focus — the new pane is created in the Director's window but is not made active.

#### Output format

Text (default) — one compact line; `pane` renders `(pending)` when the pane
id has not been patched onto the placement yet:

```
<agent_id> <name> backend=<coding_agent> pane=<pane_id>
```

Text (`--full`) — 6-line block:

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

## `cafleet message` — Message Broker

All six subcommands require the per-subcommand `--fleet-id` and name the requester with `--agent-id`.
The task envelope schema is canonical in
[Message envelope](./message-envelope.md); body truncation and the
`--full` flag are canonical in
[Message Body Truncation](#message-body-truncation) — neither is restated
per subcommand. The compact text render is `[<id> | from:<from> | <ts>]` on
line 1 (plus `kind` / `origin` segments only when present) and the body on
line 2.

Text output is the subcommand's acknowledgement line — `Message sent.`
(`send`), `Message acknowledged.` (`ack`), `Task canceled.` (`cancel`) —
followed by the compact rendered envelope; `message show` prints the envelope
alone. `poll` and `broadcast` have their own output shapes, noted below.

### `message send`

| Flag | Required | Notes |
|---|---|---|
| `--agent-id` | yes | Sender (requester). |
| `--to` | yes | Recipient agent id. |
| `--text` | yes | Message body. |
| `--full` | no | See [Message Body Truncation](#message-body-truncation). |

### `message broadcast`

| Flag | Required | Notes |
|---|---|---|
| `--agent-id` | yes | Broadcaster (requester). |
| `--text` | yes | Message body. |
| `--full` | no | See [`--full` semantics](#full-semantics). |

Default text output is the one-line summary
`broadcast id=<task_id> recipients=<N> delivered=<k>`, where `N` is the real
recipient count and `k` is the count of best-effort inline previews that
landed; the two diverge when any preview fails to deliver. In `--json` the
result object carries both `recipients` and `delivered`.

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
| `--full` | no | See [Message Body Truncation](#message-body-truncation). |

### `message cancel`

| Flag | Required | Notes |
|---|---|---|
| `--agent-id` | yes | Sender retracting the message (sender-only). |
| `--task-id` | yes | Task to cancel. |
| `--full` | no | See [Message Body Truncation](#message-body-truncation). |

### `message show`

| Flag | Required | Notes |
|---|---|---|
| `--agent-id` | yes | The acting agent (fleet-membership gate). |
| `--task-id` | yes | Task to fetch. |
| `--full` | no | See [Message Body Truncation](#message-body-truncation). |

## `cafleet pane` — Pane Interaction

The `cafleet pane` subgroup keystrokes a pane-bound agent and must be run inside a tmux session. Every subcommand targets its agent by `--agent-id` (the **target**, scoped to the per-subcommand `--fleet-id`) and delivers keystrokes into that agent's tmux pane: `capture | input | exec | wake`. They share the resolution, key-delivery, and exit-code rules below; each subcommand's own section documents only its unique flags, key sequence, validation, and output.

### Pane targeting and key delivery

#### Agent resolution

1. Load the active in-fleet target. A cross-fleet, unknown, or inactive (deregistered) `--agent-id` all resolve to "not found" and exit 1 with `Error: Agent <agent-id> not found`. There is no caller-auth check beyond fleet membership.
2. If the agent has no placement row, exit 1 with ``Error: agent <agent-id> has no placement row; it was not spawned via `cafleet agent spawn`.``.
3. If the placement's pane id is `None` (pending placement), exit 1 — each subcommand uses its own "nothing to …" wording (see its section).

The only boundary is fleet isolation: any **active** in-fleet agent **with a placement row** (the root Director included) is a valid `--agent-id`.

#### Literal key delivery

Each key sequence is delivered literally — shell meta (`$VAR`, backticks, `$(...)`), key names (`Enter`, `C-c`, `Esc`), backslash-escapes, and multi-byte characters all arrive as plain characters. The CLI runs each `send-keys` with `shell=False`, so no shell ever evaluates the text.

#### Common exit codes

| Exit | When |
|---|---|
| `0` | Dispatch success. |
| `1` | tmux unavailable / `TMUX` env var missing; agent not found (including cross-fleet `--agent-id`); missing placement row; pending placement; `tmux send-keys` subprocess failure. |
| `2` | Per-subcommand argument/validation errors (see each subcommand). |

### `pane capture`

| Flag | Required | Notes |
|---|---|---|
| `--agent-id` | yes | Target agent's ID |
| `--lines` | no | Number of trailing lines to capture (default: **20**). The single spelling — there is no `--tail` alias. |
| `--ansi` / `--no-ansi` | no | Default `--no-ansi`: ANSI escape sequences are stripped and carriage-return redraw fragments cleaned up. Pass `--ansi` to disable post-processing and emit the raw tmux capture. |

JSON: `{agent_id, pane_id, lines, content}`; text emits the content with no trailing newline.

### `pane input`

Forwards a restricted keystroke to an agent's tmux pane. Two input modes, both AskUserQuestion-only — `--freetext` prepends the digit `4` (the "Type something" gate). For shell dispatch use [`pane exec`](#pane-exec) instead.

Exactly one of the two flags must be supplied.

| Flag | Required | Notes |
|---|---|---|
| `--agent-id` | yes | Target agent's ID |
| `--choice` | one-of | Integer `1`, `2`, or `3`. Sends the matching digit key to the pane (no Enter). Values outside 1–3 are rejected (exit 2). |
| `--freetext` | one-of | Free-text string to type into the "Type something" field. Sends `4`, then the literal text via `tmux send-keys -l`, then `Enter`. AskUserQuestion-only. Rejected if the first non-whitespace character is `!` (use `pane exec` for shell dispatch). |

Exactly one of `--choice` / `--freetext` must appear (zero or both exits 2 — see [Error Messages](#error-messages)).

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
| Zero or both of `--choice` / `--freetext` | Rejected (exit 2). |
| `--choice 0` / `--choice 4` / `--choice a` | Rejected — values outside 1–3 (exit 2). |
| `--freetext ""` (empty) | Allowed — sends `4` + empty literal + `Enter` (submits an empty answer; AskUserQuestion's own UI decides whether to accept it). |
| `--freetext "   "` (whitespace-only) | Allowed — `lstrip()` empties the string before the `startswith("!")` check, so the bang-prefix guard does not fire. |
| `--freetext` whose first non-whitespace character is `!` | Rejected (exit 2) — use `pane exec` for shell dispatch. |
| `--freetext` containing `\n` or `\r` | Rejected (exit 2) — single-action contract, one prompt submission per call. |

Error strings: see [Error Messages](#error-messages).

#### Output format

Text:

```
Sent choice 1 to agent Claude-B (%7).
Sent free text to agent Claude-B (%7).
```

JSON (`cafleet --json ... pane input ...`):

```json
{
  "agent_id": <id>,
  "pane_id": "%7",
  "action": "choice",
  "value": "1"
}
```

```json
{
  "agent_id": <id>,
  "pane_id": "%7",
  "action": "freetext",
  "value": "<user text as-sent>"
}
```

#### Director-side usage pattern

The canonical three-beat workflow (`pane capture` → AskUserQuestion → `pane input`) lives in the claude overlay (`skills/cafleet/reference/coding-agent/claude.md`); the backend-neutral relay flow is in `skills/cafleet/reference/director.md` § "Answering a member's relayed question". This page documents only the CLI surface.

### `pane exec`

Director-only shell-dispatch primitive. Keystrokes `! <command>` + `Enter` into an agent's pane so the coding agent's `!` shortcut runs the command natively (bypassing the member's Bash tool permission system). All three backends (`claude`, `codex`, and `opencode`) honor the leading-`!` shortcut on their input line, so `pane exec` works against any backend without modification. The fallback path for the bash-via-Director protocol — see [Bash routing](../concepts/bash-routing.md).

```bash
cafleet pane exec --fleet-id <fleet-id> \
  --agent-id <agent-id> "git log -1 --oneline"
```

| Flag / argument | Required | Notes |
|---|---|---|
| `--agent-id` | yes | Target agent's ID |
| *(positional `COMMAND`)* | yes | Single shell command. Leading and trailing whitespace are stripped before dispatch into the pane (the JSON `command` field and the text echo both reflect the trimmed form). Otherwise pipes, `&&`, `;`, `$(...)`, and backticks are not special-cased — the command is forwarded opaquely. |

#### Key sequence sent to the pane

| Invocation | tmux calls issued in order |
|---|---|
| `pane exec "X"` | `tmux send-keys -t <pane> -l "! X"` → `tmux send-keys -t <pane> Enter` |

#### Validation rules

| Input | Result |
|---|---|
| Missing positional `COMMAND` | Rejected (exit 2). |
| `command` empty after `.strip()` (`""` or whitespace-only) | Rejected (exit 2). |
| `command` containing `\n` or `\r` | Rejected (exit 2). |

Error strings: see [Error Messages](#error-messages).

(tmux-unavailable and binary-not-found errors are common — see [Pane targeting and key delivery](#pane-targeting-and-key-delivery).)

#### Output format

Text:

```
Sent bash command 'git log -1 --oneline' to agent Claude-B (%7).
```

JSON (`cafleet --json ... pane exec ...`):

```json
{
  "agent_id": <id>,
  "pane_id": "%7",
  "command": "<command as-sent>"
}
```

Three keys: `agent_id`, `pane_id`, `command`.

### `pane wake`

One command with two mutually-exclusive modes for waking a pane-bound agent: `--poll-only` re-pokes the agent's inbox; `--message` persists an ACKable broker task **and** fires an `Esc`-safeguarded inline preview into the target's pane. Exactly one of `--poll-only` / `--message` is required; supplying both, or `--message` without `--from` and `--text`, is a usage error (exit 2). There is no `--quiet` flag.

| Flag | Required | Notes |
|---|---|---|
| `--agent-id` | yes | The **target** agent (typically the Director for `--message`). |
| `--poll-only` | one-of | Inject the `Esc`-safeguarded inbox-poll keystroke into the target's pane (no sender, no text). |
| `--message` | one-of | Persist an ACKable task and fire the inline preview. Requires `--from` and `--text`. |
| `--from` | with `--message` | The **sender** (typically the monitoring member). Persisted as the task's `from_agent_id` so the Director sees who woke it. |
| `--text` | with `--message` | The re-engage summary (un-ACKed inbox items, stalled members). Persisted as the task body and keystroked as the inline preview. Empty / whitespace-only is rejected (exit 2). |

```bash
# Re-poll a pane that missed the broker's automatic on-delivery notification:
cafleet pane wake --fleet-id <fleet-id> --agent-id <agent-id> --poll-only

# Re-engage an idle Director with an ACKable task + hardened preview:
cafleet pane wake --fleet-id <fleet-id> --agent-id <director-agent-id> \
  --message --from <monitoring-member-id> --text "<re-engage summary>"
```

#### `--poll-only` behavior

Keystrokes `Esc` → `cafleet message poll --fleet-id <fleet-id> --agent-id <agent-id>` → `Enter` into the target's pane so the agent drains its inbox via a normal poll; the leading `Esc` is the permission-prompt safeguard (see [tmux push](../concepts/tmux-push.md)). This is the manual re-poke for a pane that missed the broker's automatic on-delivery notification. The action is wholly determined by the mode flag — there is no operator-controlled keystroke body, which is why `pane wake` sits in `permissions.allow` while `pane exec` stays in `permissions.ask`. A `tmux send-keys` non-delivery exits 1 with `Error: send failed: tmux send-keys did not deliver the poll-trigger keystroke to pane <pane>.`. JSON: `{agent_id, pane_id}`; text: `Woke agent <name> (<pane_id>) — poll keystroke dispatched.`.

#### `--message` behavior

Functionally equivalent to a monitoring-member `cafleet message send --to <director-id>` (both ride the same hardened send path). Resolves the target (fleet-isolation only — no caller-auth check), then calls the broker send path, which (1) persists a `unicast` / `input_required` task — the ACKable inbox item the Director's facilitation loop consumes — and (2) best-effort fires the `Esc`-safeguarded inline preview into the target's pane. A target with no live pane is tolerated: the task still persists and the keystroke best-effort no-ops, identical to `message send` semantics.

Text:

```
Woke <name> (<pane_id>) — task <task_id> queued, Esc-safeguarded preview dispatched.
```

A target with no placement pane prints the `no pane; task queued` variant. A target **with** a pane whose best-effort preview did not land (tmux binary missing, self-send, or a send failure) prints `Woke <name> (<pane_id>) — task <task_id> queued; inline preview not delivered.` — the task still persists in all three cases. JSON (`cafleet --json ... pane wake --message ...`):

```json
{
  "agent_id": <id>,
  "pane_id": "%7",
  "task_id": <task_id>,
  "notification_sent": true
}
```

#### Exit codes

| Exit | When |
|---|---|
| `0` | `--poll-only` keystroke dispatched, or `--message` task persisted (preview dispatched or best-effort no-op). |
| `1` | Target not found (cross-fleet / unknown / inactive `--agent-id`), an in-fleet target with no placement row, a pending placement, a `--poll-only` non-delivery, or the `--message` sender (`--from`) not active in the fleet — see [Error Messages](#error-messages). |
| `2` | Both/neither of `--poll-only` / `--message`; `--message` missing `--from` / `--text`; empty / whitespace-only `--text`. |

## `cafleet monitor` — Supervision Scheduler {#cafleet-monitor}

The `cafleet monitor` subgroup is the per-fleet scheduler that wakes the monitoring member whenever a watched agent is due. All three subcommands require the per-subcommand `--fleet-id`. `start` runs the loop in-process (the fleet's dedicated **monitoring member** launches it as a **background task** in its own pane and owns its lifetime); `status` and `config` view and edit the schedule. The conceptual model is canonical on the [Monitoring](../concepts/monitoring.md) concepts page; this page documents the CLI surface.

There is no `monitor stop` command and no detached process: stop the loop by stopping the monitoring member's background task (or deleting the monitoring member), and the loop also self-terminates when the fleet is torn down. Launching/stopping the loop is **CLI-only** by nature; the schedule-view and schedule-edit surfaces are at WebUI/CLI parity ([WebUI API](./webui-api.md)).

### `monitor start`

| Flag | Required | Notes |
|---|---|---|
| `--tick` | no | Scan-tick cadence in seconds (`click.IntRange(min=1)`, default **5**). Stored in `monitor_runtime.tick_seconds` so `status` can report it. The tick is the floor on per-agent interval precision — see [Monitoring](../concepts/monitoring.md#cadence-and-tick-precision). |

Runs the `scan → wake monitor when any watched agent is due → heartbeat → sleep` loop **in-process** via `run_monitor_loop` — the fleet's monitoring member launches it as a background task in its own pane (the loop blocks the task). On startup it runs the tmux precondition guard (the same `TMUX`-env check the `pane` / `agent spawn` commands use), then atomically claims the single-instance `monitor_runtime` row, installs `SIGTERM`/`SIGINT` handlers (a clean stop clears the row), and loops until signalled or the fleet is torn down (`monitor_tick` returns `STOP` once the fleet is soft-deleted). There is no detached subprocess, no PID file, and no log file — the loop writes to the launching task's own stdout.

Each tick scans the watched set and, when any agent is due, wakes the monitoring member once with a single-line wake nudge naming each freshly-due agent and the Director, then advances each due agent's cadence so it is not re-flagged on the next tick. The loop **never** keystrokes a watched pane. The watched-set intervals, the wake-nudge contract, and the `Esc`-safeguard placement are canonical on [Monitoring](../concepts/monitoring.md). Each due agent is logged to stdout as `<iso-ts> due agent <id> (<name>) -> wake monitor`, so the launching background task's output shows live heartbeat activity.

If the fleet has **no** monitoring member when `start` runs (`broker.find_monitoring_member(fleet_id) is None`), the command prints a warning to stderr (`Warning: fleet <id> has no monitoring member; the monitor heartbeat will wake no agent. Spawn one first with 'cafleet agent spawn --role monitor'.`) and then runs the loop anyway (warn-but-run). In the canonical flow the warning never fires — the monitoring member is spawned at `agent spawn`, before it launches `monitor start` in its own pane.

Exit codes: `0` clean exit (signalled, or the fleet was torn down); `1` already running (`monitor already running for fleet N`), unknown or soft-deleted fleet, or tmux unreachable; `2` click usage errors (e.g. `--tick 0`).

### `monitor status`

No flags beyond the per-subcommand `--fleet-id`. Reports runtime liveness derived from the DB heartbeat (true even when the process died silently) plus the watched-agent schedule table.

Text output:

```
monitor: running (pid 4821, last tick 2s ago, tick 5s, started 2026-06-13T04:50:00+00:00)
  agent_id  name         role      interval  last_ping  enabled  pending
  --------  -----------  --------  --------  ---------  -------  -------
  2         Director      director  180s      8s ago     yes      1
  5         alice         member    720s      -          yes      0
```

The **watched** agents appear — the root Director (`role: director`, derived from `fleets.director_agent_id`) and every ordinary member (`role: member`). The monitoring member is **not** enrolled, so it never shows in this table (it is the watcher, located by kind). `last_ping` renders as a human age (`8s ago`, or `-` when never pinged) as optional context on each watched agent's cadence — the monitoring member learns which agents are freshly due from the wake nudge's named list, not from these ages. When no monitor is running the first line reads `monitor: stopped`; the schedule table still renders. JSON output keeps `last_ping_at` (ISO or null) and adds a derived `last_ping_age_seconds` (int or null) per agent:

```json
{
  "runtime": {"running": true, "pid": 4821, "tick_seconds": 5,
              "last_tick_at": "2026-06-13T04:51:02+00:00", "last_tick_age_seconds": 2,
              "started_at": "2026-06-13T04:50:00+00:00"},
  "agents": [
    {"agent_id": 2, "name": "Director", "role": "director", "interval_seconds": 180,
     "last_ping_at": "2026-06-13T04:50:54+00:00", "last_ping_age_seconds": 8,
     "enabled": true, "pending_count": 1},
    {"agent_id": 5, "name": "alice", "role": "member", "interval_seconds": 720,
     "last_ping_at": null, "last_ping_age_seconds": null, "enabled": true, "pending_count": 0}
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

With no edit flag, prints the agent's current config. With `--interval` / `--enable` / `--disable`, applies the update and prints the new config. The command is generic — it edits any enrolled agent, the root Director or an ordinary member. Exits 1 if the agent is not in the fleet or not enrolled (the monitoring member, the Administrator, and card-only agents are never enrolled, so `--agent-id <monitoring-member-id>` reports not-enrolled). `--enable` and `--disable` together exit 2.

Text output:

```
agent 5: interval 720s, enabled, last_ping 2026-06-13T04:51:00
```

`last_ping` renders the timestamp, or ASCII `-` when never pinged. JSON output: `{"agent_id": 5, "interval_seconds": 720, "last_ping_at": "<iso8601>|null", "enabled": true}`.

## Error Messages

| Situation | Error Message |
|---|---|
| Missing `--fleet-id` on a fleet-scoped subcommand | `Error: Missing option '--fleet-id'.` (exit 2) |
| Missing `--agent-id` | `Error: Missing option '--agent-id'.` (exit 2) |
| `fleet create` run outside a tmux session | `Error: cafleet fleet create must be run inside a tmux session` (exit 1; no DB writes) |
| `fleet delete` on unknown fleet_id | `Error: fleet 'X' not found.` (exit 1) |
| `agent register` into a soft-deleted fleet | `Error: fleet X is deleted` (exit 1) |
| `agent deregister` against the root Director's `agent_id` | `Error: cannot deregister the root Director; use 'cafleet fleet delete' instead` (exit 1) |
| `agent deregister` against the Administrator's `agent_id` | `Error: Administrator cannot be deregistered` (exit 1) |
| `agent deregister` that deregisters nothing | `Error: agent <id> not found or already deregistered.` (exit 1) |
| `agent deregister` default path when the pane does not close within 15.0 s | `Error: pane <pane> did not close within 15.0s after the exit keystroke.` (exit 1; pane tail printed on stderr) |
| `agent show` / `agent deregister` / `message send` / `message poll` / `message ack` / `message cancel` / `message show` with an `--agent-id` that is not a member of `--fleet-id` | `Error: agent <id> is not a member of fleet <fleet-id>.` (exit 1) — the fleet-membership gate runs before any read/write operation. Also fires for unknown `--agent-id` (the gate cannot tell "unknown" from "in a different fleet" apart and treats both as not-a-member). |
| `pane input` with zero or both of `--choice` / `--freetext` | `Error: --choice and --freetext are mutually exclusive; supply exactly one.` (exit 2) |
| `pane input --choice` outside `1..3` | Values outside 1–3 are rejected (exit 2) |
| `pane input --freetext` whose first non-whitespace character is `!` | `Error: --freetext may not start with '!' — that triggers the coding agent's shell-execution shortcut. Use 'cafleet pane exec' for shell dispatch instead.` (exit 2) |
| `pane input --freetext` with `\n` or `\r` | `Error: free text may not contain newlines.` (exit 2) |
| `pane input` on an agent with pending placement | `Error: agent <id> has no pane yet (pending placement) — nothing to input.` (exit 1) |
| `pane exec` with missing positional `COMMAND` | `Error: Missing argument 'COMMAND'.` (exit 2) |
| `pane exec ""` (empty / whitespace-only) | `Error: command may not be empty.` (exit 2) |
| `pane exec` with `\n` or `\r` | `Error: command may not contain newlines.` (exit 2) |
| `pane exec` on an agent with pending placement | `Error: agent <id> has no pane yet (pending placement) — nothing to exec.` (exit 1) |
| `pane wake --poll-only` on an agent with pending placement | `Error: agent <id> has no pane yet (pending placement) — nothing to wake.` (exit 1) |
| `pane wake --poll-only` when `tmux send-keys` fails | `Error: send failed: tmux send-keys did not deliver the poll-trigger keystroke to pane <pane>.` (exit 1) |
| `pane wake` with neither or both of `--poll-only` / `--message` | usage error (exit 2) |
| `pane wake --message` with a cross-fleet / unknown / inactive `--agent-id` | `Error: Agent <agent-id> not found` (exit 1) |
| `pane wake --message` on an in-fleet `--agent-id` with no placement row | ``Error: agent <agent-id> has no placement row; it was not spawned via `cafleet agent spawn`.`` (exit 1) |
| `pane wake --message` with empty / whitespace-only `--text` | `Error: text may not be empty.` (exit 2) |
| `pane wake --message` whose `--from` (sender) is not active in the fleet | `Error: <sender ValueError from the broker send path>` (exit 1) |
| `agent spawn` with both `--prompt-file` and a positional prompt argument | `Error: --prompt-file and the positional prompt argument are mutually exclusive.` (exit 2) |
| `agent spawn --prompt-file` with a relative path | ``Error: --prompt-file requires an absolute path (got '<input>'). Resolve relative paths against your BASE first — see the `cafleet-base-dir` skill.`` (exit 2) |
| `agent spawn --prompt-file` to a non-existent path or non-regular file (e.g. directory) | `Error: --prompt-file <path>: file does not exist or is not a regular file.` (exit 1) |
| `agent spawn --prompt-file` to an unreadable file | `Error: --prompt-file <path>: file is not readable.` (exit 1) |
| `agent spawn --prompt-file` to a file containing invalid UTF-8 | `Error: --prompt-file <path>: file is not valid UTF-8.` (exit 1) |
| `agent spawn --prompt-file` to a zero-byte or whitespace-only file | `Error: --prompt-file <path>: file is empty.` (exit 1) |
| `agent spawn --coding-agent opencode --model` with a value violating the `<provider-id>/<model-id>` format | `Error: --model for the opencode backend must be '<provider-id>/<model-id>' (got '<value>').` (exit 2; fires before any agent registration or tmux side effect) |
| `agent spawn --role monitor` when the fleet already has an active monitoring member | `Error: fleet <id> already has an active monitoring member (agent <existing-id>); only one is allowed.` (exit 1; enforced in `register_agent`) |
| `agent spawn --role monitor` with `--coding-agent` omitted and the spawning Director not found in the fleet | `Error: cannot resolve the monitor's coding agent: Director <director-id> not found in fleet <fleet-id>. Re-run with an explicit --coding-agent.` (exit 1; nothing spawned) |
| `agent spawn --role monitor` with `--coding-agent` omitted and the spawning Director has no placement row | `Error: cannot resolve the monitor's coding agent: Director <director-id> has no placement row recording its backend. Re-run with an explicit --coding-agent.` (exit 1; nothing spawned) |
| `monitor start` for a fleet that already has a live monitor | `Error: monitor already running for fleet <id>` (exit 1) |
| `monitor start` / `monitor status` against an unknown or soft-deleted fleet | `Error: fleet <id> not found` (exit 1) |
| `monitor config` with both `--enable` and `--disable` | `Error: --enable and --disable are mutually exclusive.` (exit 2) |
| `monitor config` against an agent not in the fleet or not enrolled | `Error: agent <id> is not enrolled in monitoring for fleet <fleet-id>.` (exit 1) |
