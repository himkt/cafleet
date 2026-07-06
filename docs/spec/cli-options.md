---
icon: lucide/square-terminal
---

# CLI options

How the unified CAFleet CLI (`cafleet`) accepts configuration parameters.

## Subcommand summary

One row per subcommand. "Identity flag" is the per-subcommand option naming the acting agent (requester or Director/sender, spelled `--agent-id`) or the target member (spelled `--member-id`), with the polarity fixed per command (see [Agent ID](#agent-id)). In the `--fleet-id` column, **yes** means the flag is a required per-subcommand option (placed after the subcommand name); **no** means the subcommand rejects `--fleet-id` with `No such option`.

| Subcommand | Purpose | `--fleet-id` | Identity flag | Section |
|---|---|---|---|---|
| `setup` | Create the database schema + install the skills (bare group invocation) | no | none | [setup](#cafleet-setup) |
| `setup db` | Migrate the database schema only | no | none | [setup db](#setup-db) |
| `setup skill` | Install the skills + record the installed version | no | none | [setup skill](#setup-skill) |
| `doctor` | Print the resolved multiplexer backend + the calling pane's identifiers + the skills-install report | no | none | [doctor](#cafleet-doctor) |
| `server` | Start the admin WebUI server | no | none | [server](#cafleet-server) |
| `fleet create` | Create a fleet with its root Director and Administrator | no | none | [fleet create](#fleet-create) |
| `fleet list` | List non-deleted fleets | no | none | [fleet list](#fleet-list) |
| `fleet show` | Show one fleet (soft-deleted included) | yes | none | [fleet show](#fleet-show) |
| `fleet delete` | Soft-delete a fleet and deregister its agents | yes | none | [fleet delete](#fleet-delete) |
| `message send` | Send a unicast message | yes | `--agent-id` (requester) | [message send](#message-send) |
| `message broadcast` | Broadcast a message to all fleet agents | yes | `--agent-id` (requester) | [message broadcast](#message-broadcast) |
| `message poll` | Fetch un-acked incoming messages | yes | `--agent-id` (requester) | [message poll](#message-poll) |
| `message ack` | Acknowledge a received message | yes | `--agent-id` (requester) | [message ack](#message-ack) |
| `message cancel` | Retract an un-acked sent message | yes | `--agent-id` (requester) | [message cancel](#message-cancel) |
| `message show` | Show one task | yes | `--agent-id` (requester) | [message show](#message-show) |
| `member create` | Register a member and spawn its coding-agent pane | yes | `--agent-id` (Director) | [member create](#member-create) |
| `member delete` | Tear down a member's pane (when one exists) and deregister it | yes | `--member-id` (target) | [member delete](#member-delete) |
| `member show` | Show one agent's detail | yes | `--member-id` (target) | [member show](#member-show) |
| `member list` | List the fleet's members (every active agent with `--all`) | yes | none | [member list](#member-list) |
| `member capture` | Capture the tail of a member's pane | yes | `--member-id` (target) | [member capture](#member-capture) |
| `member exec` | Dispatch a shell command into a member's pane | yes | `--member-id` (target) | [member exec](#member-exec) |
| `member ping` | Inject an inbox-poll keystroke into a member's pane | yes | `--member-id` (target) | [member ping](#member-ping) |
| `member nudge` | Deliver an ACKable task + inline preview to a member | yes | `--agent-id` (sender) + `--member-id` (target) | [member nudge](#member-nudge) |
| `monitor start` | Run the per-fleet scheduler loop in-process (launch as a background task) | yes | none | [monitor start](#monitor-start) |
| `monitor status` | Show monitor liveness and the per-agent schedule | yes | none | [monitor status](#monitor-status) |
| `monitor config` | Show or edit an agent's monitor schedule | yes | `--agent-id` | [monitor config](#monitor-config) |

## Option Source Matrix

Each parameter has exactly one input source:

| Parameter | Source |
|---|---|
| Fleet ID | `--fleet-id <int>` per-subcommand option (placed after the subcommand name) |
| Database URL | `CAFLEET_DATABASE_URL` env var (optional) — see [config](../api/config.md) for its default and the absolute-path requirement. |
| Multiplexer backend | `CAFLEET_MULTIPLEXER` env var (optional) — an explicit override naming a supported backend (`tmux` or `herdr`); unset ⇒ auto-detect from `HERDR_ENV` / `TMUX`. See [Multiplexer backends](../concepts/multiplexer-backends.md). |
| Agent ID | `--agent-id <int>` subcommand option |
| Member ID | `--member-id <int>` subcommand option (the target member on `member *` lifecycle verbs) |
| JSON output | `--json` global flag |

> **`--fleet-id` is a literal CLI flag** — see [Fleet ID](#fleet-id) for why agents pass it literally, and how `permissions.allow` matching depends on the canonical flag order.

Create a fleet first if you don't have one:

```bash
cafleet fleet create --label "my-project"
# → prints the fleet_id
```

Then pass the printed id as `--fleet-id <id>` on every fleet-scoped command.

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
| `member show` | The compact one-line row `<agent_id> <name> <status>`. | Labeled block: `agent_id`, `name`, `description` (truncated to 60 codepoints), `status`, `kind`, `skills`, and the placement sub-block (see [member show](#member-show)). `--full` affects **text mode only** — JSON returns the broker agent dict unchanged regardless of `--full`. |
| `member create` | One compact line: `<agent_id> <name> backend=<coding_agent> pane=<pane_id>`. | The 6-line `Member registered and spawned.` block — see [member create](#member-create). |

## Fleet ID (`--fleet-id`) {#fleet-id}

`--fleet-id` is a **per-subcommand option** (not a top-level option). It names the fleet the command acts on and is placed immediately **after** the subcommand name (the canonical position), ahead of the other flags — e.g. `cafleet message poll --fleet-id <id> --agent-id <id>`. It is typed `int`; a non-integer fails with Click's standard `Error: Invalid value for '--fleet-id': '<x>' is not a valid integer.` (exit 2). Every `member *`, `message *`, and `monitor *` command, plus `fleet show` and `fleet delete`, carries it; `setup`, `doctor`, `server`, `fleet create`, and `fleet list` do **not** accept it and reject it with `No such option: --fleet-id`. Which subcommand takes it is in the [Subcommand summary](#subcommand-summary).

`--fleet-id` is a required option with no environment default. A spawned member reads its fleet id from the `FLEET ID:` line the CLI rendered into its spawn prompt (see [member create](#member-create)) and passes it as a literal flag on every command.

Agents still pass `--fleet-id` as a literal flag because Claude Code's `permissions.allow` matches Bash invocations as literal command strings: a literal `--fleet-id <int>` keeps the invocation a fixed string an allow pattern can match, while a shell-expanded variable (`$FLEET_ID`) breaks the match and forces per-invocation permission prompts that interrupt agent work. Substitute the literal integer ids printed by `cafleet fleet create` and `cafleet member create` — never shell variables to hold them. Matching also depends on the canonical flag order (`--fleet-id` first, immediately after the subcommand name); a different order does not match — see [`permissions.allow` coverage](#permissionsallow-coverage).

## Agent ID (`--agent-id`) {#agent-id}

`--agent-id` is a **per-subcommand option** (not a global option). It is typed `int`; a non-integer fails with Click's standard `Error: Invalid value for '--agent-id': '<x>' is not a valid integer.` (exit 2). The same `type=int` applies to every id option — `--to`, `--member-id`, and `--task-id` — so each rejects a non-integer the same way. Ids are short by construction (DB-assigned integers, typically 1–4 digits), so they are pasted in full; there is no prefix resolution.

**Polarity is per-command.** `--agent-id` names the **calling agent (requester)** on every `message *` command, the **acting Director** on `member create`, and the **sender** on `member nudge`. Every other `member *` lifecycle verb targets its member with the separate `--member-id` option. Which subcommand takes which identity flag is in the [Subcommand summary](#subcommand-summary).

## Member ID (`--member-id`) {#member-id}

`--member-id` is the per-subcommand option naming the **target member** on `member delete`, `member show`, `member capture`, `member exec`, `member ping`, and `member nudge`. It is typed `int` and rejects a non-integer with Click's standard invalid-integer error (exit 2). The target must be an active agent of `--fleet-id`; a placement row is required by the pane-touching verbs but **not** by `member show` or `member delete` — see [Member targeting and key delivery](#member-targeting-and-key-delivery).

## `permissions.allow` coverage

The allow set is generated mechanically, one `Bash(...)` pattern per allow-listed subcommand, by this rule:

- **One pattern per subcommand**, each matching the canonical `--fleet-id`-first flag order (see [Fleet ID](#fleet-id)) — `Bash(cafleet <grp> <cmd> --fleet-id *)`. A different flag order does not match and prompts.
- **`member exec` is excluded** so it stays under `permissions.ask` — its positional command body is operator-controlled.
- **Each subcommand an agent runs with `--json` needs a companion pattern**, because `--json` is a top-level option that precedes the subcommand name and breaks the prefix: `Bash(cafleet --json <grp> <cmd> --fleet-id *)`.

Three representative patterns:

```
Bash(cafleet message poll --fleet-id *)
Bash(cafleet member create --fleet-id *)
Bash(cafleet --json message poll --fleet-id *)
```

Expanding the rule over every allow-listed subcommand (the fleet-scoped leaf subcommands minus `member exec`, plus a `--json` companion per JSON-invoked subcommand) yields the full set mechanically. Apply the patterns to your user-level `~/.claude/settings.json` manually; the repo does **not** ship a committed `.claude/settings.json` permissions block.

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

The `--quiet` flag is available on `cafleet message send`, `cafleet message ack`, and `cafleet member ping`: it suppresses the normal output and prints only the bare `task_id` (the target member id for `ping`), for shell capture.

Length is measured in Python `str` codepoints, never bytes — multibyte characters are never split.

```bash
cafleet message poll --fleet-id <fleet-id> --agent-id <my-agent-id>          # default: text truncated to 200 cp + "…"
cafleet message poll --fleet-id <fleet-id> --agent-id <my-agent-id> --full   # full body
```

This applies to CLI emit sites only. FastAPI `/api/*` responses (see [webui-api.md](./webui-api.md)) are unchanged — the WebUI is human-facing and renders full bodies. `agent.description`, `skills[].description`, `agent_card_json` sub-fields, and `member capture` content are also untouched.

## `cafleet setup` — Onboarding and Schema Management {#cafleet-setup}

`cafleet setup` is a Click **group** with `invoke_without_command=True`: bare
`cafleet setup` runs the full onboarding sequence — the recommended end-user
path (see [Install](../get-started/install.md)) — while `cafleet setup db` and
`cafleet setup skill` run one half each. The group callback no-ops unless
`ctx.invoked_subcommand is None`, so the subcommands never trigger the
bare-setup sequence. The `setup` group is the single schema-management entry
point.

No command in the `setup` group accepts `--fleet-id`. Supplying it exits 2
with `No such option: --fleet-id`, matching the `fleet create` / `fleet list` /
`server` / `doctor` pattern.

### `setup` (bare invocation)

Bare `cafleet setup` takes no options and runs two independent halves, in
order:

1. **db half** — initializes or migrates the registry database to the bundled
   Alembic head revision (idempotent; a DB already at head is a no-op); see
   [Storage](../concepts/storage.md).
2. **skills half** — reads the installed `cafleet` CLI version, downloads the
   matching `cafleet-skills-v<version>.zip` asset from the corresponding
   GitHub Release of `himkt/cafleet`, extracts the three skill directories
   (`cafleet`, `cafleet-design-doc`, `cafleet-research`) into **every
   detected** coding-agent home (each home whose parent directory exists),
   replacing any existing copy, and records one `skill_installs` row per home
   after that home's install succeeds (see
   [data-model.md](./data-model.md)). Zero detected homes fail the half with
   the same no-homes-detected error as `setup skill`.

The halves fail independently — each catches its own error and prints
`db half failed: <msg>` or `skills half failed: <msg>` — and if anything
failed the command exits 1 with `<failed halves joined by ' and '> half failed`
(db listed first, matching the run order; e.g. `db and skills half failed`).
If the db half failed, the skills half fails its schema pre-flight and both
halves are reported failed.

Bare `setup` accepts **no** `--agent` — supplying it exits 2 with Click's
standard `No such option` error. Per-agent targeting lives on
[`setup skill`](#setup-skill).

Skills-half failures surface as runtime error messages — no release for the
installed version, a missing or malformed skills asset, the GitHub API
unreachable, an unwritable target, or zero detected agent homes.

### `setup db` {#setup-db}

Takes no options. Runs the db half only: forces a sync SQLite URL from the
configured database URL, creates the DB file's parent directory, and applies
the bundled Alembic migrations up to the head revision (idempotent). Output,
by prior DB state:

```
Created <db_file> and applied migrations to head (<head>).   # fresh DB
Upgraded from <old_rev> to <head>.                           # behind head
Already at head (<head>); nothing to do.                     # at head
```

It refuses two states, exiting 1 — a DB with existing tables but no
`alembic_version` table, and a DB whose recorded revision is unknown to the
bundled script directory:

```
Error: DB has existing tables but no alembic_version. Run `alembic stamp head` manually if you are sure the schema matches.
Error: DB schema is at revision <rev> which is unknown to this version of cafleet. Refusing to downgrade automatically.
```

`setup db` never touches `skill_installs` **rows** — it creates the table (as
part of migration `0006`) but records nothing.

### `setup skill` {#setup-skill}

| Flag | Required | Notes |
|---|---|---|
| `--agent` | no | One of `claude`, `codex`, or `opencode`; repeatable (`multiple=True`), duplicate values deduped silently. Scopes the skills targets to exactly the named agents — an explicitly named agent's home/skills tree is created if missing. Omitted → auto-detect every agent whose home directory exists (`~/.claude`, `~/.codex`, `~/.config/opencode`). An unknown value fails Click's choice check (exit 2). |

Pre-flight: the `skill_installs` table must exist, else the command exits 1
with:

```
Error: the database schema is missing or outdated; run 'cafleet setup' or 'cafleet setup db' first
```

`setup skill` does **not** auto-create the schema. After the pre-flight it
resolves the targets (`--agent` values, else auto-detect), downloads and
installs exactly as the bare skills half, and after each home's install
succeeds upserts that home's `skill_installs` row with the runtime CLI
version. Per-home success output:

```
<agent>: installed cafleet, cafleet-design-doc, cafleet-research (v<version>) -> <skills dir>
```

An install failure aborts the loop; rows recorded for homes completed before
the failure remain.

## Stale-skills guard {#stale-skills-guard}

Every fleet-scoped command group — `fleet`, `member`, `message`, and
`monitor` — validates the recorded skills installs at the top of its group
callback, before any subcommand body runs:

1. If the DB file, the `skill_installs` table, or all rows are missing, the
   command exits 1 with:

    ```
    Error: no skills install is recorded; run 'cafleet setup' first
    ```

2. If any recorded `cafleet_version` differs from the runtime CLI version
   (simple string inequality — a downgrade also triggers), the command exits 1
   with the stale agents listed in ascending `coding_agent` order:

    ```
    Error: stale skills detected (<agent>=<recorded>[, ...]; CLI <runtime>); run 'cafleet setup skill' to reinstall
    ```

    e.g. `stale skills detected (claude=0.5.0, codex=0.5.0; CLI 0.6.0); run 'cafleet setup skill' to reinstall`.

3. Otherwise the command proceeds silently.

Homes with no recorded row (an agent that was never installed) are not
checked. Exempt surfaces: the `setup` group (must remain runnable to repair),
`doctor` (reports instead of blocking — see
[doctor](#cafleet-doctor)), and `server` (the WebUI is human-facing and not
fleet-scoped).

**Help interaction (contract).** Group-level help (`cafleet fleet --help`) is
parsed eagerly during the group's own context and prints help before the
callback runs, so it always works — even under a missing or stale install.
Subcommand help (`cafleet fleet create --help`) runs the group callback first,
so under a missing/stale install the guard **errors instead of printing
help**.

## `cafleet fleet` — Fleet Management

The `cafleet fleet` subgroup manages fleets. These commands write directly to SQLite — the broker server does not need to be running. `fleet show` and `fleet delete` take the required `--fleet-id`; `fleet create` and `fleet list` do not. Every `fleet` command runs behind the [stale-skills guard](#stale-skills-guard).

### `fleet create`

| Flag | Required | Notes |
|---|---|---|
| `--label` | no | Free-form text label for the fleet |
| `--coding-agent` | no | One of `claude` (default), `codex`, or `opencode`, recorded as the root Director's placement `coding_agent` — see [Coding agents](../concepts/coding-agents.md). |
| `--json` | no | Output as JSON |
| `--full` | no | Documented flag: switches the non-JSON output from the compact one-line form to the 7-line block below. |

There are no `--name` / `--description` flags. The root Director's name and description are hardcoded (`name="Director"`, `description="Root Director for this fleet"`).

Creates a new fleet with a DB-assigned integer identifier. **Must be run inside a tmux or herdr session** — outside a supported multiplexer the command exits 1 with `Error: cafleet fleet create must be run inside a tmux or herdr session` and writes nothing to the DB. It creates the fleet, its root Director (and placement), and the built-in Administrator atomically (all-or-nothing) — see [data-model.md](./data-model.md) for the Administrator's distinguishing `agent_card_json.cafleet.kind` flag.

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
pane:             <mux_session>:<mux_window_id>:<mux_pane_id>
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
      "backend": "tmux",
      "mux_session": "main",
      "mux_window_id": "@3",
      "mux_pane_id": "%0",
      "coding_agent": "claude",
      "created_at": "2026-04-15T10:00:00+00:00"
    }
  }
}
```

`placement.director_agent_id` is `null` because the root Director has no parent. `placement.coding_agent` is the value of `--coding-agent` (default `"claude"`); operators running the codex CLI in the calling pane should pass `--coding-agent codex` so the placement metadata is accurate. cafleet does not spawn the root Director's coding-agent process and cannot auto-detect what is running in the calling pane.

Both the root Director and the built-in Administrator are protected from `member delete` — see [Error Messages](#error-messages).

### `fleet list`

| Flag | Required | Notes |
|---|---|---|
| `--json` | no | Output as JSON |

Lists all **non-soft-deleted** fleets with their `director_agent_id`, label, created_at, and active agent count. Soft-deleted fleets (`fleets.deleted_at IS NOT NULL`) are hidden.

Each row exposes the fleet's root `director_agent_id` so the Director's ID can be recovered from a list after `fleet create` output scrolls away. The `--json` output carries it as a `director_agent_id` field (integer). Text output renders five columns: `FLEET_ID`, `DIRECTOR`, `LABEL`, and `AGENTS` are each left-padded to widths 40 / 40 / 20 / 8 (one space between columns), followed by an unpadded trailing `CREATED_AT`. The wide left-padding means the real gaps between columns are far larger than a compact sample can depict; the column order is `FLEET_ID`, `DIRECTOR` (immediately after `FLEET_ID`), `LABEL`, `AGENTS`, `CREATED_AT`. Nullable `DIRECTOR` / `LABEL` cells fall back to empty strings.

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

Member tmux panes spawned by `cafleet member create` are **not** automatically closed by `fleet delete`. For a clean teardown, call `cafleet member delete` per member first (which sends the backend exit keystroke to the pane). If a pane refuses to close (e.g. blocked on a confirmation prompt), rerun `cafleet member delete` with `--force`, which kill-panes the target, sweeps the placement, and rebalances the layout.

## `cafleet doctor` — Placement Diagnostics {#cafleet-doctor}

Resolves the active multiplexer backend via `resolve_multiplexer()`, then prints
the resolved backend and the calling pane's session/window/pane identifiers for
operators diagnosing placement issues without reaching for raw multiplexer
commands, followed by the skills-install report: the runtime CLI version and
every recorded `skill_installs` row. `doctor` is exempt from the
[stale-skills guard](#stale-skills-guard) — it reports staleness instead of
blocking on it.

| Flag | Required | Notes |
|---|---|---|
| `--json` | no | Top-level `--json`, written ahead of the subcommand name (same pattern as every other CLI command). |

Environment requirements:

- A supported multiplexer must be detected: `resolve_multiplexer()` succeeds when `CAFLEET_MULTIPLEXER` is set to a supported backend, or exactly one of `HERDR_ENV` / `TMUX` is present (see [Multiplexer backends](../concepts/multiplexer-backends.md)). An ambiguous or empty environment fails loudly.
- The resolved backend's `ensure_available()` + `context_discovery()` must succeed (the binary is on `PATH` and the calling pane is discoverable).

Text output — the `multiplexer:` block carries the resolved `backend`, the three
pane identifiers, and the backend's presence env var (`TMUX` for tmux,
`HERDR_ENV` for herdr) with its value:

```
multiplexer:
  backend:       tmux
  session:       main
  window_id:     @3
  pane_id:       %0
  presence:      TMUX=/tmp/tmux-501/default,12345,0
skills:
  cli_version: 0.6.0
  claude:      0.6.0 (2026-07-04T00:12:09.123456+00:00) ok
  codex:       0.5.0 (2026-06-20T10:00:00.987654+00:00) STALE
```

One line per recorded `skill_installs` row: the recorded version, the stored
`installed_at` string printed **verbatim** (microsecond precision, exactly as
stored), and `ok` when the recorded version equals the runtime CLI version,
`STALE` otherwise. When no rows exist (or the table is missing), the `skills:`
block is instead:

```
skills:
  (no skills install recorded; run 'cafleet setup')
```

JSON output — a `"multiplexer"` object with `backend` plus `session` /
`window_id` / `pane_id` and the presence env var (`presence_var` /
`presence_value`), sibling to `"skills"` whose `installs` is an empty list when
no rows exist:

```json
{
  "multiplexer": {
    "backend": "tmux",
    "session": "main",
    "window_id": "@3",
    "pane_id": "%0",
    "presence_var": "TMUX",
    "presence_value": "/tmp/tmux-501/default,12345,0"
  },
  "skills": {
    "cli_version": "0.6.0",
    "installs": [
      {"coding_agent": "claude", "cafleet_version": "0.6.0", "installed_at": "2026-07-04T00:12:09.123456+00:00", "current": true},
      {"coding_agent": "codex", "cafleet_version": "0.5.0", "installed_at": "2026-06-20T10:00:00.987654+00:00", "current": false}
    ]
  }
}
```

Exit codes:

| Exit | When |
|---|---|
| `0` | Success — the multiplexer fields and the skills report printed. A stale or missing skills install does **not** fail `doctor`. |
| `1` | Any multiplexer or environment failure: no supported multiplexer detected (or an ambiguous `HERDR_ENV` + `TMUX`), the backend binary not on `PATH`, the pane not discoverable, or a multiplexer subprocess failure. |

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

## `cafleet message` — Message Broker

All six subcommands require the per-subcommand `--fleet-id` and name the requester with `--agent-id`, and run behind the [stale-skills guard](#stale-skills-guard).
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
| `--text` | no | Inline message body. Mutually exclusive with `--text-file`; exactly one of the two is required. |
| `--text-file` | no | Path to a UTF-8 file whose contents are the message body — absolute, or relative to CWD; `-` reads the whole body from stdin. Mutually exclusive with `--text`; exactly one of the two is required. Use this for long or multi-line bodies that would exceed the shell's `ARG_MAX`. |
| `--full` | no | See [Message Body Truncation](#message-body-truncation). |

### `message broadcast`

| Flag | Required | Notes |
|---|---|---|
| `--agent-id` | yes | Broadcaster (requester). |
| `--text` | no | Inline message body. Mutually exclusive with `--text-file`; exactly one of the two is required. |
| `--text-file` | no | Path to a UTF-8 file whose contents are the message body — absolute, or relative to CWD; `-` reads the whole body from stdin. Mutually exclusive with `--text`; exactly one of the two is required. Use this for long or multi-line bodies that would exceed the shell's `ARG_MAX`. |
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

## `cafleet member` — Member Lifecycle + Pane Interaction {#cafleet-member}

The `cafleet member` subgroup owns the agent lifecycle: `create | delete | show | list | capture | exec | ping | nudge`. `member create` registers a member **and** spawns its coding-agent pane; `member delete` tears it down; `capture` / `exec` / `ping` / `nudge` inspect or keystroke an existing member's pane; `show` and `list` are registry reads. The tmux-session requirement is scoped to the pane-touching verbs — `create`, `capture`, `exec`, `ping`, `nudge`, and `member delete`'s pane-teardown paths; `show`, `list`, and a placementless or pending-placement `delete` run without tmux. `create` names the acting Director with `--agent-id`; every other lifecycle verb targets its member by `--member-id` (`nudge` additionally names its sender with `--agent-id`). The `--member-id` verbs share the resolution, key-delivery, and exit-code rules below; each subcommand's own section documents only its unique flags, key sequence, validation, and output. Every `member` command runs behind the [stale-skills guard](#stale-skills-guard).

### Member targeting and key delivery

#### Member resolution

1. Load the active in-fleet target. A cross-fleet, unknown, or inactive (deregistered) `--member-id` all resolve to "not found" and exit 1 with `Error: Agent <member-id> not found`. There is no caller-auth check beyond fleet membership.
2. If the agent has no placement row, exit 1 with ``Error: agent <member-id> has no placement row; it was not spawned via `cafleet member create`.`` — except `member show` and `member delete`, which tolerate a missing placement (`show` renders `placement:   none`; `delete` performs a registry soft-delete — see their sections).
3. If the placement's pane id is `None` (pending placement), `capture` / `exec` / `ping` exit 1 with `Error: member <member-id> has no pane yet (pending placement) — nothing to <capture|exec|ping>.`; `delete` and `nudge` tolerate a pending placement (see their sections).

The only boundary is fleet isolation: any **active** in-fleet agent (the root Director included) is a valid `--member-id`; the pane-touching verbs additionally require a placement row (rule 2), while `member show` and `member delete` accept a placementless target.

#### Literal key delivery

Each key sequence is delivered literally — shell meta (`$VAR`, backticks, `$(...)`), key names (`Enter`, `C-c`, `Esc`), backslash-escapes, and multi-byte characters all arrive as plain characters. The CLI runs each `send-keys` with `shell=False`, so no shell ever evaluates the text.

#### Common exit codes

| Exit | When |
|---|---|
| `0` | Dispatch success. |
| `1` | tmux unavailable / `TMUX` env var missing; agent not found (including cross-fleet `--member-id`); missing placement row; pending placement; `tmux send-keys` subprocess failure. |
| `2` | Per-subcommand argument/validation errors (see each subcommand). |

### `member create` {#member-create}

Register a member agent **and** spawn its coding-agent pane.

| Flag | Required | Notes |
|---|---|---|
| `--agent-id` | yes | The acting Director's agent ID. |
| `--name` | yes | Display name of the new member — see [Known asymmetries](../concepts/coding-agents.md#known-asymmetries-intentional-non-goals) for pane-title behavior. |
| `--description` | yes | One-sentence purpose. |
| `--coding-agent` | no | One of `claude`, `codex`, or `opencode`; an ordinary member defaults to `claude` and a `--role monitor` member inherits the spawning Director's backend (an explicit value always wins). Backend resolution and the per-backend spawn argv live in [Coding agents](../concepts/coding-agents.md). Exits 1 with `Error: binary <name> not found on PATH` when the chosen binary is not on `PATH`. |
| `--model` | no | Model forwarded to the backend binary's `--model` flag; omitted by default — see [Model selection](../concepts/coding-agents.md#model-selection). |
| `--role` | no | One of `member` (default) or `monitor`. `monitor` spawns the fleet's dedicated **monitoring member** (sets `agent_card_json.cafleet.kind == "monitoring-member"`, skips `monitor_config` enrollment, and inherits the Director's backend when `--coding-agent` is omitted); an ordinary `member` is enrolled as a watched agent. The LLM is still chosen by `--model` (the Director passes `--model haiku`). A second `--role monitor` spawn in the same fleet is rejected — see [Error Messages](#error-messages). See [Monitoring](../concepts/monitoring.md#the-monitoring-member). |
| `--full` | no | Documented flag: switches the non-JSON output to the 6-line block below. |
| `--text` | no | Inline spawn prompt text. All three backends receive the same prompt; the prompt template is backend-neutral. Mutually exclusive with `--text-file`; exactly one of the two is required. |
| `--text-file` | no | Path to a UTF-8 file whose contents are the spawn prompt — absolute, or relative to CWD; `-` reads the whole prompt from stdin. Mutually exclusive with `--text`; exactly one of the two is required. |

#### Spawn command per backend

The per-backend spawn argv and auto-approval flags live in the Backend-resolution table on [Coding agents](../concepts/coding-agents.md). In all three modes the member's Bash tool is enabled and routine permission prompts auto-resolve — see [Bash routing](../concepts/bash-routing.md) for the fallback protocol.

#### Spawn-prompt input modes

`cafleet member create` requires the spawn prompt via exactly one of `--text` / `--text-file`:

| Inputs | Resulting spawn prompt body |
|---|---|
| `--text "<prompt>"` only | The inline argument. |
| `--text-file PATH` only | The file contents, decoded as UTF-8 byte-for-byte (no newline translation). A relative PATH resolves against CWD. |
| `--text-file -` only | The whole prompt read from stdin, decoded as UTF-8. |
| Both `--text` and `--text-file` | Error (exit 2) — mutually exclusive; see [Error Messages](#error-messages). |
| Neither `--text` nor `--text-file` | Error (exit 2) — exactly one is required; see [Error Messages](#error-messages). |

For `--text-file`, missing files, unreadable files, invalid UTF-8, and empty (zero-byte or whitespace-only) files all produce non-zero-exit errors — see the [Error Messages](#error-messages) table for the full surface. Inline `--text` prompts beyond a few KB exceed tmux's argv ceiling (`tmux command failed: command too long` rolls back the registration) — use `--text-file` for long prompts.

#### Spawn-prompt substitution

`cafleet member create` runs `str.format` over the resolved prompt body, substituting exactly four placeholders:

| Placeholder | Rendered value |
|---|---|
| `{fleet_id}` | The `--fleet-id` value. |
| `{agent_id}` | The member's **own** newly-allocated agent id. |
| `{director_agent_id}` | The `--agent-id` value (the acting Director). |
| `{coding_agent}` | The resolved backend name (`claude` / `codex` / `opencode`). |

Identity reaches the spawned member as literals rendered into its prompt — the only environment variable forwarded into the pane is `CAFLEET_DATABASE_URL`. A literal brace in prompt text must be **doubled** (`{{` / `}}`) to survive `.format()`. An unknown placeholder or a malformed brace expression is a usage error (exit 2) — see [Error Messages](#error-messages); because substitution needs the freshly-allocated `{agent_id}`, it runs after registration, and the just-registered agent is rolled back before the error surfaces.

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
where `placement` carries `director_agent_id`, `backend`, `mux_session`,
`mux_window_id`, `mux_pane_id`, `coding_agent`, and `created_at` (the same
shape as `fleet create`'s `director.placement`).

### `member delete` {#member-delete}

| Flag | Required | Notes |
|---|---|---|
| `--member-id` | yes | The member to delete (target). |
| `--force` / `-f` | no | When the target has a pane, skip the graceful close wait: immediately kill-pane the target, then deregister. Exit 0 even if the pane was already gone. |

Tears down the target's pane (when one exists) and soft-deletes the agent. The tmux guard fires only on the pane-teardown paths (live `pane_id`) — a placementless or pending-placement delete is a pure registry operation and succeeds outside tmux.

- **Root Director guard (early — before any tmux pane mutation):** targeting the root Director exits 1 with `Error: cannot deregister the root Director; use 'cafleet fleet delete' instead`.
- **Administrator guard:** targeting the built-in Administrator exits 1 with `Error: Administrator cannot be deregistered` (enforced in the broker deregister path).
- **No placement row** — registry soft-delete; header `Member deleted.`, `pane_status` `(no placement)`.
- **Pending placement (no pane yet)** — plain registry soft-delete; header `Member deleted.`, `pane_status` `(pending — no pane)`.
- **Has a pane (default path)** — send the backend exit keystroke, then poll for the pane to disappear (every 500 ms, up to a 15.0 s timeout). Pane gone → deregister, header `Member deleted.`. A typical coding-agent exit completes in 1–3 s; operators who need faster escalation pass `--force`. On timeout, the pane buffer tail (last 80 lines) is captured and printed on stderr with a recovery hint, and the command exits **2**.
- **Has a pane (`--force`)** — kill-pane immediately, then deregister; header `Member deleted (--force).`.

Success text output is the header line followed by `agent_id:` / `pane_id:` lines. JSON output: `{agent_id, pane_status}`, where `pane_status` is `(no placement)`, `(pending — no pane)`, `<pane_id> (closed)`, `<pane_id> (killed)`, or `<pane_id> (timeout)`.

#### Timeout output shape

```
Error: pane %7 did not close within 15.0s after /exit.
--- pane %7 tail (last 80 lines) ---
<captured terminal buffer>
---
Recovery: inspect with `cafleet member capture`, then re-run `cafleet member delete`. Or re-run with `--force` to skip the wait and kill the pane.
```

#### Exit codes

| Exit | When |
|---|---|
| `0` | Success — default-path pane-gone confirmed, `--force` pane killed, pending-placement deregister, or no-placement registry soft-delete. |
| `1` | Missing fleet, unknown member-id (including cross-fleet), the root-Director guard, the Administrator guard, a tmux failure sending the exit keystroke or waiting for the pane, a kill-pane failure, or a deregister failure. |
| `2` | The 15.0 s default-path timeout (pane tail printed on stderr). |

### `member show` {#member-show}

| Flag | Required | Notes |
|---|---|---|
| `--member-id` | yes | The target agent. Any **active in-fleet** agent is valid — placed or placementless (root Director and Administrator included). |
| `--full` | no | Documented flag; affects **text mode only** — see [`--full` semantics](#full-semantics). |

Registry read — no tmux requirement (like `member list`) and no requester gate. A cross-fleet, unknown, or inactive target exits 1 with `Error: Agent <member-id> not found`.

**Default text** is the compact one-line row:

```
<agent_id> <name> <status>
```

**`--full` text** is a labeled block. `description` keeps the 60-codepoint truncation; `skills` renders as a compact JSON array or `-` when empty; the placement sub-block appears only when a placement row exists, `placement:   none` otherwise; a `None` field inside the placement (the root Director's `director_agent_id`, a pending `pane_id`) renders `-`:

```
  agent_id:    3
  name:        Administrator
  description: <description, 60 cp + …>
  status:      active
  kind:        administrator
  skills:      -
  placement:   none
```

```
  agent_id:    2
  name:        Director
  description: Root Director for this fleet
  status:      active
  kind:        director
  skills:      -
  placement:
    director_agent_id: -
    backend:           claude
    session:           main
    window_id:         @3
    pane_id:           %0
    created_at:        2026-04-15T10:00:00+00:00
```

**`--json`** returns the broker `get_agent` dict unchanged (`agent_id`, `name`, `description`, `status`, `registered_at`, `kind`, `skills`, `placement` or `null`) regardless of `--full` — consistent with the `member` group's unprojected-JSON convention.

`kind` takes one of four values: `director` (the fleet's root Director), `administrator` (the built-in Administrator), `monitor` (the monitoring member), or `member` (everything else).

### `member list` {#member-list}

| Flag | Required | Notes |
|---|---|---|
| `--activity` | no | Aggregate per-member activity timestamps from the `tasks` table and render `last_sent`, `last_recv`, `last_ack`, and `idle` columns; broadcast summary rows are excluded from `last_ack`. Mutually exclusive with `--all`. |
| `--all` | no | List every **active agent** of the fleet, not just members: root Director, Administrator, monitoring member, ordinary members, and any placementless rows. Mutually exclusive with `--activity` — combining them exits 2 with `Error: --all and --activity are mutually exclusive.`. |

No identity flag — the listing is scoped by the per-subcommand `--fleet-id` alone. Lists every **member** of the fleet (agents with a placement row); the root Director is excluded. An empty roster prints `0 members.`. Default text output is a placement table:

```
2 members:
  agent_id        name      status  backend  session  window_id  pane_id  created_at
  --------------  --------  ------  -------  -------  ---------  -------  --------------------
  4               alice     active  claude   main     @3         %7       2026-06-11T09:00:00+00:00
  5               bob       active  codex    main     @3         %8       2026-06-11T09:00:05+00:00
```

`pane_id` renders `(pending)` for a pending placement. `--json` returns the member rows unprojected.

#### `member list --all` output {#member-list-all-output}

`--all` lists every **active agent** of the fleet. The header becomes `N agents:`; the table gains a `kind` column (the same four values as [member show](#member-show)) and renders `-` in every placement column (`backend`, `session`, `window_id`, `pane_id`, `created_at`) for placementless rows:

```
4 agents:
  agent_id  name           status  kind           backend  session  window_id  pane_id  created_at
  --------  -------------  ------  -------------  -------  -------  ---------  -------  --------------------
  2         Director       active  director       claude   main     @3         %0       2026-04-15T10:00:00+00:00
  3         Administrator  active  administrator  -        -        -          -        -
  4         monitor        active  monitor        claude   main     @3         %5       2026-04-15T10:01:00+00:00
  5         alice          active  member         claude   main     @3         %7       2026-04-15T10:02:00+00:00
```

`--json --all` returns the rows unprojected — the member-row shape (`agent_id`, `name`, `description`, `status`, `registered_at`, `placement`) plus `kind`, with `placement: null` for placementless rows. The default (no `--all`) output is unchanged.

#### `member list --activity` output {#member-list-activity-output}

```
cafleet member list --fleet-id 1 --activity
3 members:
  agent_id        name      status  last_sent  last_recv  last_ack   idle
  --------------  --------  ------  ---------  ---------  ---------  -----
  4               alice     active  -          12:20:00   12:20:00   14m
  5               bob       active  12:30:11   12:33:02   12:33:02   2m
  6               carol     active  12:34:56   12:34:50   12:34:50   6s
```

`last_sent` is the member's most recent outgoing message; `last_recv` is its most recent delivery; `last_ack` is the most recent delivery it acknowledged (broadcast summaries excluded); `idle` is wall-time since the latest of `last_sent` / `last_recv`. An absent cell renders as a single ASCII `-`.

### `member capture` {#member-capture}

| Flag | Required | Notes |
|---|---|---|
| `--member-id` | yes | Target member's ID |
| `--lines` / `--tail` | no | Number of trailing lines to capture (default: **20**). `--tail` is an alias for `--lines`. |
| `--ansi` / `--no-ansi` | no | Default `--no-ansi`: ANSI escape sequences are stripped and carriage-return redraw fragments cleaned up. Pass `--ansi` to disable post-processing and emit the raw tmux capture. |

JSON: `{member_agent_id, pane_id, lines, content}`; text emits the content with no trailing newline.

### `member exec` {#member-exec}

Director-only shell-dispatch primitive. Keystrokes `! <command>` + `Enter` into a member's pane so the coding agent's `!` shortcut runs the command natively (bypassing the member's Bash tool permission system). All three backends (`claude`, `codex`, and `opencode`) honor the leading-`!` shortcut on their input line, so `member exec` works against any backend without modification. The fallback path for the bash-via-Director protocol — see [Bash routing](../concepts/bash-routing.md).

```bash
cafleet member exec --fleet-id <fleet-id> \
  --member-id <member-id> "git log -1 --oneline"
```

| Flag / argument | Required | Notes |
|---|---|---|
| `--member-id` | yes | Target member's ID |
| *(positional `COMMAND`)* | yes | Single shell command. Leading and trailing whitespace are stripped before dispatch into the pane (the JSON `command` field and the text echo both reflect the trimmed form). Otherwise pipes, `&&`, `;`, `$(...)`, and backticks are not special-cased — the command is forwarded opaquely. |

#### Key sequence sent to the pane

| Invocation | tmux calls issued in order |
|---|---|
| `member exec "X"` | `tmux send-keys -t <pane> -l "! X"` → `tmux send-keys -t <pane> Enter` |

#### Validation rules

| Input | Result |
|---|---|
| Missing positional `COMMAND` | Rejected (exit 2). |
| `command` containing `\n` or `\r` | Rejected (exit 2). |
| `command` empty after `.strip()` (`""` or whitespace-only) | Rejected (exit 2). |

Error strings: see [Error Messages](#error-messages).

(tmux-unavailable and binary-not-found errors are common — see [Member targeting and key delivery](#member-targeting-and-key-delivery).)

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

### `member ping` {#member-ping}

Re-pokes a member's inbox. Keystrokes `Esc` → `cafleet message poll --fleet-id <fleet-id> --agent-id <member-id>` → `Enter` into the target's pane so the member drains its inbox via a normal poll; the leading `Esc` is the permission-prompt safeguard (see [tmux push](../concepts/tmux-push.md)). This is the manual re-poke for a pane that missed the broker's automatic on-delivery notification. The action is wholly determined by the command — there is no operator-controlled keystroke body, which is why `member ping` sits in `permissions.allow` while `member exec` stays in `permissions.ask`.

| Flag | Required | Notes |
|---|---|---|
| `--member-id` | yes | The target member. |
| `--quiet` | no | Suppress the normal `Pinged …` line and print only the bare member id, for shell capture. |

```bash
cafleet member ping --fleet-id <fleet-id> --member-id <member-id>
```

A `tmux send-keys` non-delivery exits 1 with `Error: send failed: tmux send-keys did not deliver the poll-trigger keystroke to pane <pane>.`. JSON: `{member_agent_id, pane_id}`; text: `Pinged member <name> (<pane_id>) — poll keystroke dispatched.`.

### `member nudge` {#member-nudge}

Re-engages a member (typically the Director) by persisting an ACKable broker task **and** firing an `Esc`-safeguarded inline preview into the target's pane. Functionally equivalent to a monitoring-member `cafleet message send --to <director-id>` (both ride the same hardened send path): it resolves the target (fleet-isolation only — no caller-auth check), then calls the broker send path, which (1) persists a `unicast` / `input_required` task — the ACKable inbox item the Director's facilitation loop consumes — and (2) best-effort fires the `Esc`-safeguarded inline preview into the target's pane. A target with no live pane is tolerated: the task still persists and the keystroke best-effort no-ops, identical to `message send` semantics.

| Flag | Required | Notes |
|---|---|---|
| `--agent-id` | yes | The **sender** (typically the monitoring member). Persisted as the task's `from_agent_id` so the Director sees who nudged it. |
| `--member-id` | yes | The **target** member (typically the Director). |
| `--text` | no | Inline re-engage summary (un-ACKed inbox items, stalled members). Persisted as the task body and keystroked as the inline preview. Mutually exclusive with `--text-file`; exactly one of the two is required. |
| `--text-file` | no | Path to a UTF-8 file whose contents are the body — absolute, or relative to CWD; `-` reads the whole body from stdin. Mutually exclusive with `--text`; exactly one of the two is required. Use this for long or multi-line bodies that would exceed the shell's `ARG_MAX`. |

```bash
cafleet member nudge --fleet-id <fleet-id> \
  --agent-id <monitoring-member-id> --member-id <director-agent-id> \
  --text "<re-engage summary>"
```

Text:

```
Nudged <name> (<pane_id>) — task <task_id> queued, Esc-safeguarded preview dispatched.
```

A target with a pending placement prints `Nudged <name> — no pane; task <task_id> queued.`. A target **with** a pane whose best-effort preview did not land (tmux binary missing, self-send, or a send failure) prints `Nudged <name> (<pane_id>) — task <task_id> queued; inline preview not delivered.` — the task still persists in all three cases. JSON (`cafleet --json ... member nudge ...`):

```json
{
  "member_agent_id": <id>,
  "pane_id": "%7",
  "task_id": <task_id>,
  "notification_sent": true
}
```

#### Exit codes

| Exit | When |
|---|---|
| `0` | Task persisted (preview dispatched or best-effort no-op). |
| `1` | Target not found (cross-fleet / unknown / inactive `--member-id`), an in-fleet target with no placement row, or the sender (`--agent-id`) rejected by the broker send path — see [Error Messages](#error-messages). |
| `2` | Both/neither of `--text` / `--text-file`; empty / whitespace-only inline `--text`. |

## `cafleet monitor` — Supervision Scheduler {#cafleet-monitor}

The `cafleet monitor` subgroup is the per-fleet scheduler that wakes the monitoring member whenever a watched agent is due. All three subcommands require the per-subcommand `--fleet-id` and run behind the [stale-skills guard](#stale-skills-guard). `start` runs the loop in-process (the fleet's dedicated **monitoring member** launches it as a **background task** in its own pane and owns its lifetime); `status` and `config` view and edit the schedule. The conceptual model is canonical on the [Monitoring](../concepts/monitoring.md) concepts page; this page documents the CLI surface.

There is no `monitor stop` command and no detached process: stop the loop by stopping the monitoring member's background task (or deleting the monitoring member), and the loop also self-terminates when the fleet is torn down. Launching/stopping the loop is **CLI-only** by nature; the schedule-view and schedule-edit surfaces are at WebUI/CLI parity ([WebUI API](./webui-api.md)).

### `monitor start`

| Flag | Required | Notes |
|---|---|---|
| `--tick` | no | Scan-tick cadence in seconds (`click.IntRange(min=1)`, default **5**). Stored in `monitor_runtime.tick_seconds` so `status` can report it. The tick is the floor on per-agent interval precision — see [Monitoring](../concepts/monitoring.md#cadence-and-tick-precision). |

Runs the `scan → wake monitor when any watched agent is due → heartbeat → sleep` loop **in-process** via `run_monitor_loop` — the fleet's monitoring member launches it as a background task in its own pane (the loop blocks the task). On startup it runs the tmux precondition guard (the same `TMUX`-env check the `member *` commands use), then atomically claims the single-instance `monitor_runtime` row, installs `SIGTERM`/`SIGINT` handlers (a clean stop clears the row), and loops until signalled or the fleet is torn down (`monitor_tick` returns `STOP` once the fleet is soft-deleted). There is no detached subprocess, no PID file, and no log file — the loop writes to the launching task's own stdout.

Each tick scans the watched set and, when any agent is due, wakes the monitoring member once with a single-line wake nudge naming each freshly-due agent and the Director, then advances each due agent's cadence so it is not re-flagged on the next tick. The loop **never** keystrokes a watched pane. The watched-set intervals, the wake-nudge contract, and the `Esc`-safeguard placement are canonical on [Monitoring](../concepts/monitoring.md). Each due agent is logged to stdout as `<iso-ts> due agent <id> (<name>) -> wake monitor`, so the launching background task's output shows live heartbeat activity.

If the fleet has **no** monitoring member when `start` runs (`broker.find_monitoring_member(fleet_id) is None`), the command prints a warning to stderr (`Warning: fleet <id> has no monitoring member; the monitor heartbeat will wake no agent. Spawn one first with 'cafleet member create --role monitor'.`) and then runs the loop anyway (warn-but-run). In the canonical flow the warning never fires — the monitoring member is spawned at `member create`, before it launches `monitor start` in its own pane.

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

With no edit flag, prints the agent's current config. With `--interval` / `--enable` / `--disable`, applies the update and prints the new config. The command is generic — it edits any enrolled agent, the root Director or an ordinary member. Exits 1 if the agent is not in the fleet or not enrolled (the monitoring member, the Administrator, and placementless agents are never enrolled, so `--agent-id <monitoring-member-id>` reports not-enrolled). `--enable` and `--disable` together exit 2.

Text output:

```
agent 5: interval 720s, enabled, last_ping 2026-06-13T04:51:00
```

`last_ping` renders the timestamp, or ASCII `-` when never pinged. JSON output: `{"agent_id": 5, "interval_seconds": 720, "last_ping_at": "<iso8601>|null", "enabled": true}`.

## Error Messages

| Situation | Error Message |
|---|---|
| Any `fleet` / `member` / `message` / `monitor` command with no skills install recorded (missing DB file, missing `skill_installs` table, or zero rows) | `Error: no skills install is recorded; run 'cafleet setup' first` (exit 1; see [Stale-skills guard](#stale-skills-guard)) |
| Any `fleet` / `member` / `message` / `monitor` command with a recorded `skill_installs` version differing from the runtime CLI version | `Error: stale skills detected (<agent>=<recorded>[, ...]; CLI <runtime>); run 'cafleet setup skill' to reinstall` (exit 1; see [Stale-skills guard](#stale-skills-guard)) |
| `setup skill` when the `skill_installs` table is missing | `Error: the database schema is missing or outdated; run 'cafleet setup' or 'cafleet setup db' first` (exit 1) |
| Missing `--fleet-id` on a fleet-scoped subcommand | `Error: --fleet-id <int> is required for this subcommand. Create a fleet with 'cafleet fleet create' and pass its id.` (exit 1) |
| Missing `--agent-id` | `Error: Missing option '--agent-id'.` (exit 2) |
| `fleet create` run outside a supported multiplexer | `Error: cafleet fleet create must be run inside a tmux or herdr session` (exit 1; no DB writes) |
| `fleet delete` on unknown fleet_id | `Error: fleet 'X' not found.` (exit 1) |
| `member create` into a soft-deleted fleet | `Error: fleet X is deleted` (exit 1) |
| `member delete` against the root Director's id | `Error: cannot deregister the root Director; use 'cafleet fleet delete' instead` (exit 1) |
| `member delete` against the Administrator's `agent_id` | `Error: Administrator cannot be deregistered` (exit 1) |
| `member delete` default path when the pane does not close within 15.0 s | `Error: pane <pane> did not close within 15.0s after /exit.` (exit 2; pane tail printed on stderr) |
| `member list` with both `--all` and `--activity` | `Error: --all and --activity are mutually exclusive.` (exit 2) |
| `message send` / `message poll` / `message ack` / `message cancel` / `message show` with an `--agent-id` that is not a member of `--fleet-id` | `Error: agent <id> is not a member of fleet <fleet-id>.` (exit 1) — the fleet-membership gate runs before any read/write operation. Also fires for unknown `--agent-id` (the gate cannot tell "unknown" from "in a different fleet" apart and treats both as not-a-member). |
| `member exec` with missing positional `COMMAND` | `Error: Missing argument 'COMMAND'.` (exit 2) |
| `member exec ""` (empty / whitespace-only) | `Error: command may not be empty.` (exit 2) |
| `member exec` with `\n` or `\r` | `Error: command may not contain newlines.` (exit 2) |
| `member capture` / `member exec` / `member ping` on a member with pending placement | `Error: member <id> has no pane yet (pending placement) — nothing to <capture|exec|ping>.` (exit 1) |
| `member ping` when `tmux send-keys` fails | `Error: send failed: tmux send-keys did not deliver the poll-trigger keystroke to pane <pane>.` (exit 1) |
| `member show` / `member capture` / `member exec` / `member ping` / `member nudge` with a cross-fleet / unknown / inactive `--member-id` | `Error: Agent <member-id> not found` (exit 1) |
| `member capture` / `member exec` / `member ping` / `member nudge` on an in-fleet `--member-id` with no placement row | ``Error: agent <member-id> has no placement row; it was not spawned via `cafleet member create`.`` (exit 1) |
| `member nudge` whose `--agent-id` (sender) is rejected by the broker send path | `Error: <sender ValueError from the broker send path>` (exit 1) |
| Any `--text` / `--text-file` command (`message send`, `message broadcast`, `member nudge`, `member create`) with neither flag | `Error: Provide exactly one of --text or --text-file.` (exit 2) |
| Any `--text` / `--text-file` command with both flags | `Error: --text and --text-file are mutually exclusive.` (exit 2) |
| `--text` empty or whitespace-only | `Error: text may not be empty.` (exit 2) |
| `--text-file <path>` to an empty (zero-byte or whitespace-only) file | `Error: --text-file <path>: file is empty.` (exit 1) |
| `--text-file -` with empty (or whitespace-only) stdin | `Error: --text-file -: stdin is empty.` (exit 1) |
| `--text-file <path>` to a non-existent path or non-regular file (e.g. a directory) | `Error: --text-file <path>: file does not exist or is not a regular file.` (exit 1) |
| `--text-file <path>` to an unreadable file | `Error: --text-file <path>: file is not readable.` (exit 1) |
| `--text-file <path>` to a file containing invalid UTF-8 | `Error: --text-file <path>: file is not valid UTF-8.` (exit 1) |
| `member create --coding-agent opencode --model` with a value violating the `<provider-id>/<model-id>` format | `Error: --model for the opencode backend must be '<provider-id>/<model-id>' (got '<value>').` (exit 2; fires before any agent registration or tmux side effect) |
| `member create` with an unknown `{placeholder}` in the prompt | `Error: Unknown placeholder '<name>' in custom prompt. Supported placeholders: {fleet_id}, {agent_id}, {director_agent_id}, {coding_agent}. Double literal braces ({{, }}) to keep them as text.` (exit 2; the just-registered agent is rolled back) |
| `member create` with a malformed brace expression in the prompt | `Error: Malformed custom prompt: <detail>. Double literal braces ({{, }}) to keep them as text.` (exit 2; the just-registered agent is rolled back) |
| `member create --role monitor` when the fleet already has an active monitoring member | `Error: fleet <id> already has an active monitoring member (agent <existing-id>); only one is allowed.` (exit 1; enforced in `register_agent`) |
| `member create --role monitor` with `--coding-agent` omitted and the spawning Director not found in the fleet | `Error: cannot resolve the monitor's coding agent: Director <director-id> not found in fleet <fleet-id>. Re-run with an explicit --coding-agent.` (exit 1; nothing spawned) |
| `member create --role monitor` with `--coding-agent` omitted and the spawning Director has no placement row | `Error: cannot resolve the monitor's coding agent: Director <director-id> has no placement row recording its backend. Re-run with an explicit --coding-agent.` (exit 1; nothing spawned) |
| `monitor start` for a fleet that already has a live monitor | `Error: monitor already running for fleet <id>` (exit 1) |
| `monitor start` / `monitor status` against an unknown or soft-deleted fleet | `Error: fleet <id> not found` (exit 1) |
| `monitor config` with both `--enable` and `--disable` | `Error: --enable and --disable are mutually exclusive.` (exit 2) |
| `monitor config` against an agent not in the fleet or not enrolled | `Error: agent <id> is not enrolled in monitoring for fleet <fleet-id>.` (exit 1) |
