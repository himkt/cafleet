---
icon: lucide/square-terminal
---

# CLI options

How the unified CAFleet CLI (`cafleet`) accepts configuration parameters. This
page catalogs the flags, conventions, and error strings; each subcommand's
behavior detail lives on the docstring of its broker function in the
[API reference](../api/broker.md).

## Subcommand summary

One row per subcommand. "Identity flags" are the per-subcommand options naming
members: `--member-id` names **the member in question** on single-member
commands, and two-party commands name both parties as `--from-member-id`
(sender) + `--to-member-id` (recipient/target). In the `--fleet-id` column,
**yes** means the flag is a required per-subcommand option; **no** means the
subcommand rejects it with `No such option`.

| Subcommand | Purpose | `--fleet-id` | Identity flag | Section |
|---|---|---|---|---|
| `setup` | Migrate the database schema + install the coding-agent assets (skills and presets) | no | none | [setup](#cafleet-setup) |
| `doctor` | Print the resolved multiplexer backend + the calling pane's identifiers + the assets-install report | no | none | [doctor](#cafleet-doctor) |
| `server` | Start the admin WebUI server | no | none | [server](#cafleet-server) |
| `fleet create` | Create a fleet with its root Director | no | none | [fleet create](#fleet-create) |
| `fleet list` | List non-deleted fleets | no | none | [fleet list](#fleet-list) |
| `fleet show` | Show one fleet (soft-deleted included) | yes | none | [fleet show](#fleet-show) |
| `fleet delete` | Soft-delete a fleet and deregister its members | yes | none | [fleet delete](#fleet-delete) |
| `message send` | Send a unicast message | yes | `--from-member-id` (sender) + `--to-member-id` (recipient) | [message send](#message-send) |
| `message broadcast` | Broadcast a message to all fleet members | yes | `--from-member-id` (sender) | [message broadcast](#message-broadcast) |
| `message poll` | Fetch un-acked incoming messages | yes | `--member-id` | [message poll](#message-poll) |
| `message ack` | Acknowledge a received message | yes | `--member-id` | [message ack](#message-ack) |
| `message show` | Show one message | yes | `--member-id` | [message show](#message-show) |
| `member create` | Register a member and spawn its coding-agent pane | yes | none (Director auto-resolved) | [member create](#member-create) |
| `member delete` | Tear down a member's pane (when one exists) and deregister it | yes | `--member-id` | [member delete](#member-delete) |
| `member show` | Show one member's detail | yes | `--member-id` | [member show](#member-show) |
| `member list` | List every active registry entry of the fleet | yes | none | [member list](#member-list) |
| `member capture` | Capture the tail of a member's pane | yes | `--member-id` | [member capture](#member-capture) |
| `member prompt` | Keystroke a prompt (or, with `--shell`, a shell command) into a member's pane | yes | `--member-id` | [member prompt](#member-prompt) |
| `member ping` | Inject an inbox-poll keystroke into a member's pane | yes | `--member-id` | [member ping](#member-ping) |
| `monitor start` | Run the per-fleet scheduler loop in-process (launch as a background task) | yes | none | [monitor start](#monitor-start) |
| `monitor status` | Show monitor liveness and the per-member schedule | yes | none | [monitor status](#monitor-status) |
| `monitor config` | Show or edit a member's monitor schedule | yes | `--member-id` | [monitor config](#monitor-config) |

## Option Source Matrix

Each parameter has exactly one input source:

| Parameter | Source |
|---|---|
| Fleet ID | `--fleet-id <int>` per-subcommand option (placed after the subcommand name) |
| Database URL | `CAFLEET_DATABASE_URL` env var (optional) — see [config](../api/config.md) for its default and the absolute-path requirement. |
| Multiplexer backend | `CAFLEET_MULTIPLEXER` env var (optional) — unset ⇒ auto-detect. See [Multiplexer backends](multiplexer-backends.md#backend-selection). |
| Member ID | `--member-id <int>` subcommand option (the member in question) |
| Sender / recipient member IDs | `--from-member-id <int>` / `--to-member-id <int>` on two-party subcommands |
| JSON output | `--json` per-subcommand option (trailing canonical position — placed after all other flags) |

## Environment variables

Every `CAFLEET_`-prefixed variable cafleet reads:

| Environment variable | Settings field | Default | Controls | Overridden by |
|---|---|---|---|---|
| `CAFLEET_DATABASE_URL` | `database_url` | see [config](../api/config.md) | The registry database location; an absolute path is required | — |
| `CAFLEET_MULTIPLEXER` | `multiplexer` | unset ⇒ auto-detect | The multiplexer backend, per [Backend selection](multiplexer-backends.md#backend-selection) | — |
| `CAFLEET_MAX_TEXT_LEN` | `max_text_len` | `200` | Body truncation on `message {send,poll,ack,show}`, and the broker's inline-preview truncation | `--full` |
| `CAFLEET_BROKER_HOST` | `broker_host` | `127.0.0.1` | The `cafleet server` bind address | `--host` |
| `CAFLEET_BROKER_PORT` | `broker_port` | `8000` | The `cafleet server` bind port | `--port` |

A flag wins over its environment variable, and the environment variable wins
over the hardcoded default. `member.description` truncation uses a separate
hard-coded 60-codepoint limit.

## Global Options

`--version`, placed **before** the subcommand, prints `cafleet <version>` and
exits 0, bypassing the `--fleet-id` requirement.

## Output shapes {#output-shapes}

One row per subcommand. `--full` is the global "give me every field cafleet
has, untruncated, unfiltered" escape hatch; an em-dash marks a subcommand that
does not accept it.

| Subcommand | Default text output | `--full` text output | JSON payload |
|---|---|---|---|
| `fleet create` | The compact line `<fleet_id> director=<director_member_id>` | The labeled block | The fleet dict with a nested `director` (including its `placement`) |
| `fleet list` | `FLEET_ID`, `DIRECTOR`, `NAME`, `MEMBERS`, `CREATED_AT` columns, one row per fleet | — | Output as JSON |
| `fleet show` | The fleet row, adding a `deleted_at:` line | — | Always includes `deleted_at`, null when active |
| `fleet delete` | `Deleted fleet <fleet_id>. Deregistered N members.` | — | — |
| `message send` | `Message sent.` plus the compact rendered envelope | Untruncated `text`; the verbose labeled block | The rendered envelope, or the full typed-column message dict with `--full` |
| `message broadcast` | `broadcast id=<message_id> recipients=<N> delivered=<k>` | The single `broadcast_summary` message as the full verbose envelope, never per-recipient envelopes or a `recipient_ids` list | Carries both `recipients` and `delivered` |
| `message poll` | The compact rendered envelopes; `No messages found.` on an empty inbox | Untruncated `text`; the verbose labeled block | The rendered envelopes |
| `message ack` | `Message acknowledged.` plus the compact rendered envelope | Untruncated `text`; the verbose labeled block | The rendered envelope |
| `message show` | The compact rendered envelope alone | Untruncated `text`; the verbose labeled block | The rendered envelope |
| `member create` | The compact line `<member_id> <name> backend=<coding_agent> pane=<pane_id>` | The 6-line `Member registered and spawned.` block | The member dict with its `placement` |
| `member delete` | A `Member deleted.` header plus `member_id:` / `pane_id:` lines, pane status `<pane_id> (killed)` | — | `{member_id, pane_status}` |
| `member show` | The compact one-line row `<member_id> <name> <status>` | The labeled block with `kind`, `skills`, and the placement sub-block | The broker `get_member` dict, unchanged regardless of `--full` |
| `member list` | One row per member; `0 members.` on an empty roster | — | One dict per row |
| `member capture` | The captured content, with no trailing newline | — | `{member_id, pane_id, lines, content}` |
| `member prompt` | `Sent prompt '<text>' to member <name> (<pane_id>).`, or `Sent shell prompt '<text>' …` with `--shell` | — | `{member_id, pane_id, text, shell}` |
| `member ping` | `Pinged member <name> (<pane_id>) — poll keystroke dispatched.` | — | `{member_id, pane_id}` |
| `monitor status` | `monitor: running (…)` or `monitor: stopped`, plus the schedule table | — | Keeps `last_ping_at` and adds derived `*_age_seconds` fields |
| `monitor config` | `member 5: interval 720s, enabled, last_ping <ts>` | — | Output as JSON |
| `doctor` | The `multiplexer:` and `assets:` blocks | — | Output as JSON |

`setup`, `server`, and `monitor start` are absent by design — they stream
progress or run a loop rather than emitting a one-shot payload — so the
[Subcommand summary](#subcommand-summary) legitimately carries three more rows
than this table.

### `--quiet` semantics {#quiet-semantics}

`--quiet` suppresses the normal output and prints one bare value, for shell
capture. It is a text-only shortcut, ignored in the JSON branch.

| Subcommand | Quiet output |
|---|---|
| `message send` | The bare `message_id` |
| `message ack` | The bare `message_id` |
| `member ping` | The bare target member id |

## JSON output (`--json`) {#json-output}

`--json` is a shared per-subcommand flag, placed after the subcommand name —
canonically **trailing**, after all other flags:

```bash
cafleet message poll --fleet-id 2 --member-id 7 --json
cafleet message poll --fleet-id 2 --member-id 7 --full --json
```

It switches the output to compact single-line JSON; non-ASCII (like the `…`
truncation suffix) is emitted as UTF-8, not escaped. `--json` and `--full`
are independent and composable — truncation is applied to the result before
the json-vs-text fork. `--quiet` is a text-only shortcut, ignored in the JSON
branch. The trailing position keeps JSON invocations inside the existing
per-subcommand allow patterns (see
[`permissions.allow` coverage](#permissionsallow-coverage)).

Subcommands accepting `--json`, one row per subcommand:

| Subcommand | Group |
|---|---|
| `doctor` | (root) |
| `fleet create` | `fleet` |
| `fleet list` | `fleet` |
| `fleet show` | `fleet` |
| `message send` | `message` |
| `message broadcast` | `message` |
| `message poll` | `message` |
| `message ack` | `message` |
| `message show` | `message` |
| `member create` | `member` |
| `member delete` | `member` |
| `member show` | `member` |
| `member list` | `member` |
| `member capture` | `member` |
| `member prompt` | `member` |
| `member ping` | `member` |
| `monitor status` | `monitor` |
| `monitor config` | `monitor` |

All other subcommands reject `--json` with Click's standard
`No such option` error (exit 2) — including the root group itself, so a
pre-subcommand `cafleet --json <grp> <cmd>` does not parse.

## Fleet ID (`--fleet-id`) {#fleet-id}

`--fleet-id` is a **per-subcommand option**, typed `int`, placed immediately
after the subcommand name (the canonical position). It is required with no
environment default: a spawned member reads its fleet id from the `FLEET ID:`
line rendered into its spawn prompt and passes it as a literal flag on every
command. Members pass the literal integer — never a shell variable — because
Claude Code's `permissions.allow` matches Bash invocations as literal command
strings, and matching also depends on the canonical flag order (see
[`permissions.allow` coverage](#permissionsallow-coverage)).

## Member ID (`--member-id`) {#member-id}

`--member-id` is a per-subcommand option typed `int` (as are
`--from-member-id`, `--to-member-id`, and `--message-id`; each rejects a
non-integer with Click's standard invalid-integer error, exit 2). Ids are
DB-assigned integers, typically 1–4 digits, pasted in full — there is no
prefix resolution. `--member-id` always names **the member in question**: the
requester on `message poll` / `ack` / `show`, the target on
`member delete` / `show` / `capture` / `prompt` / `ping`, and the enrolled
member on `monitor config`.

## Sender and recipient (`--from-member-id`, `--to-member-id`) {#from-to-member-id}

Two-party commands name both parties: `--from-member-id` is the sender
(`message send`, `message broadcast`) and `--to-member-id` is
the recipient (`message send`). A pane-touching target
must be an active member of `--fleet-id` with a placement row — see
[Member targeting and key delivery](#member-targeting-and-key-delivery).

## `permissions.allow` coverage

The allow set is generated mechanically, one `Bash(...)` pattern per
allow-listed subcommand:

- **One pattern per subcommand**, matching the canonical `--fleet-id`-first
  flag order — `Bash(cafleet <grp> <cmd> --fleet-id *)`. A different flag
  order does not match and prompts. Trailing flags such as
  [`--json`](#json-output) are covered by the same pattern.
- **`member prompt` is excluded** so it stays under `permissions.ask` — its
  positional text body is operator-controlled, in both the plain and the
  `--shell` form.

```
Bash(cafleet message poll --fleet-id *)
Bash(cafleet member create --fleet-id *)
```

Apply the patterns to your user-level `~/.claude/settings.json` manually; the
repo does not ship a committed permissions block.

## Message Body Truncation

The four subcommands that emit a user-supplied delivery body —
`cafleet message {send,poll,ack,show}` — truncate the `text` body to
the first `CAFLEET_MAX_TEXT_LEN` Unicode codepoints plus a single `…`
(U+2026) suffix by default, uniformly in text and `--json` output. Length is
measured in Python `str` codepoints, never bytes. Pass `--full` (a documented
per-subcommand option) to disable truncation; it composes orthogonally with
`--json`.

The limit is `CAFLEET_MAX_TEXT_LEN` — see
[Environment variables](#environment-variables).

This applies to CLI emit sites only — FastAPI `/api/*` responses
([webui-api.md](./webui-api.md)) and `member capture` content are untouched.

## `cafleet setup` — Onboarding and Schema Management {#cafleet-setup}

`cafleet setup` is a plain command — the single onboarding and
schema-management entry point (the recommended end-user path — see
[Quickstart](../quickstart.md#install)). Command help: `Migrate the database
schema and install the coding-agent assets (skills and presets).` It takes no
positional arguments — `cafleet setup <word>` fails with Click's standard
`Got unexpected extra argument (<word>)` error — and does not accept
`--fleet-id`.

The one flag is `--skip AGENT`: optional and repeatable, typed
`click.Choice(["claude", "codex", "opencode"])`, with duplicates deduplicated
and an unknown value failing with Click's standard invalid-choice error
(exit 2). Its help text is
`Skip the named agent's assets install (repeatable).`

The command runs two halves, in order:

1. **db half** — initializes or migrates the registry database to the bundled
   Alembic head revision (idempotent). Each refusal message below becomes the
   db-half failure `<msg>`:

    | Prior DB state | Outcome | Output / refusal message |
    |---|---|---|
    | No DB file | Created and migrated to head | `Created <db_file> and applied migrations to head (<head>).` |
    | Behind head | Upgraded | `Upgraded from <old_rev> to <head>.` |
    | At head | No-op | `Already at head (<head>); nothing to do.` |
    | Tables present but no `alembic_version` | Refused | ``DB has existing tables but no alembic_version. Run `alembic stamp head` manually if you are sure the schema matches.`` |
    | Revision unknown to this CLI version | Refused | `DB schema is at revision <rev> which is unknown to this version of cafleet. Refusing to downgrade automatically.` |

2. **assets half** — targets are the fixed list `claude`, `codex`, `opencode`
   (in that order) minus the skipped agents. Downloads the
   `cafleet-assets-v<version>.zip` asset matching the installed CLI version
   from the corresponding GitHub Release, then per target installs the three
   skill directories plus the agent's bundled preset where one exists
   (creating the agent's directories as needed), and records one
   `asset_installs` row per target (see [Assets half](#assets-half)). When
   all three agents are skipped, the half is skipped entirely: the command
   echoes `assets half skipped (all agents skipped)` and the half counts as
   not-run.

The halves fail independently (`db half failed: <msg>` / `assets half failed:
<msg>`); if any half that ran failed, the command exits 1 with the failed
halves joined by `' and '`.

| Half | Trigger | Message | Effect |
|---|---|---|---|
| db | Either refusal state in the table above | The refusal message, as `db half failed: <msg>` | Exit 1 |
| assets | The `asset_installs` table is missing as the half starts | `the database schema is missing or outdated; run 'cafleet setup' first` | Exit 1 |
| assets | No GitHub Release for the installed CLI version | `no release found for version <version>` | Exit 1 |
| assets | The release exists but the asset is absent | `asset cafleet-assets-v<version>.zip not found in release <version>` | Exit 1 |
| assets | A skills install fails for a target | `failed to install skills into <skills_dir>: <error>` | Aborts the loop; rows recorded before the failure remain |
| assets | A preset install fails for a target | `failed to install preset into <target>: <error>` | Aborts the loop; rows recorded before the failure remain |
| assets | All three agents skipped | `assets half skipped (all agents skipped)` | Not-run; cannot contribute a failure |

The assets-half pre-flight fires only after a db-half failure or an externally
broken schema, since the db half always runs first within the same command; it
is kept as defense.

### Schema-only invocation {#schema-only}

The documented invocation for "bring the DB to head without touching assets"
(the contributor and CI path — e.g. the prerequisite for
`mise //cafleet:makemigration`):

```bash
cafleet setup --skip claude --skip codex --skip opencode
```

It is deterministic (independent of which agent homes exist), exits 0 when
the db half succeeds, and never contacts GitHub — so it works on unreleased
dev versions. The schema-only invocation never records `asset_installs` rows.

### Assets half {#assets-half}

Each agent's preset, where one exists, is a static file shipped in the release
archive next to the skills:

| Agent | Bundled preset | Install target |
|---|---|---|
| claude | — (skills only) | — |
| codex | `presets/codex/cafleet.rules` | `~/.codex/rules/cafleet.rules` |
| opencode | `presets/opencode/cafleet.md` | `~/.opencode/agents/cafleet.md` |

Per target:

1. The three skill directories are delete-and-reinstalled into the agent's
   skills dir; a failure aborts with `failed to install skills into
   <skills_dir>: <error>`.
2. For agents with a bundled preset (codex, opencode), the preset is installed
   to its target, overwriting whatever exists there — a regular file, a
   directory, or a symlink. A filesystem error aborts with `failed to install
   preset into <target>: <error>`.
3. The agent's `asset_installs` row is upserted only after both its skills and
   its preset install successfully — the row attests skills + preset — then
   the command echoes (the preset line appears for codex and opencode targets
   only):

```
<agent>: installed cafleet, cafleet-design-doc, cafleet-research (v<version>) -> <skills dir>
<agent>: installed preset (v<version>) -> <target>
```

An install failure aborts the loop; rows recorded before the failure remain.

## Stale-assets guard {#stale-assets-guard}

Every fleet-scoped command group — `fleet`, `member`, `message`, and
`monitor` — validates the recorded assets installs before any subcommand body
runs:

| Recorded install state | Result | Exit |
|---|---|---|
| No DB file, no `asset_installs` table, or zero rows | `Error: no assets install is recorded; run 'cafleet setup' first` | 1 |
| A recorded `cafleet_version` differs from the runtime CLI version (string inequality — a downgrade also triggers) | `Error: stale assets detected (<agent>=<recorded>[, ...]; CLI <runtime>); run 'cafleet setup' to reinstall`, stale agents in ascending order | 1 |
| All recorded versions match | The command proceeds silently | 0 |

Agents with no recorded row are not checked. Three surfaces are exempt:

| Exempt surface | Why exempt | Behavior under a stale/missing install |
|---|---|---|
| `setup` | It must remain runnable to repair the install | Runs normally — it is the repair path |
| `doctor` | It reports instead of blocking | Prints each recorded row marked `ok` or `STALE` |
| `server` | It serves the WebUI rather than running a fleet-scoped command, and the guard wraps only the four fleet-scoped groups | Starts normally |

Group-level help (`cafleet fleet --help`) prints before the callback
runs and always works; subcommand help runs the group callback first, so under
a missing/stale install the guard errors instead of printing help.

## `cafleet fleet` — Fleet Management

Fleet lifecycle; writes directly to SQLite — no server required. Behavior
detail: [`create_fleet`](../api/broker.md#cafleet.broker.create_fleet),
[`list_fleets`](../api/broker.md#cafleet.broker.list_fleets),
[`get_fleet`](../api/broker.md#cafleet.broker.get_fleet),
[`delete_fleet`](../api/broker.md#cafleet.broker.delete_fleet).

### `fleet create`

| Flag | Required | Notes |
|---|---|---|
| `--name` | yes | Human-readable name for the fleet |
| `--coding-agent` | yes | One of `claude`, `codex`, or `opencode`, recorded as the root Director's placement `coding_agent` — the operator declares the backend the Director is actually running on; see [Coding agents](../concepts/coding-agents.md). |
| `--json` | no | Output as JSON |
| `--full` | no | Switches the non-JSON output from the compact one-line form to a labeled block. |

Omitting `--coding-agent` exits 2 with Click's missing-option error for a
required `Choice` option, printed after the auto-generated usage block:

```
Error: Missing option '--coding-agent'. Choose from:
	claude,
	codex,
	opencode
```

**Must be run inside a tmux or herdr session** — outside one it exits 1 with
`Error: cafleet fleet create must be run inside a tmux or herdr session` and
writes nothing. It creates the fleet and its root Director (hardcoded
`name="Director"`; there is no `--description` flag)
atomically — see [data-model.md](./data-model.md). Output shapes are in
[Output shapes](#output-shapes).

### `fleet list`

The only flag is the optional shared [`--json`](#json-output).

Lists all non-soft-deleted fleets. Each row exposes `director_member_id` so
the Director's id can be recovered after `fleet create` output scrolls away.

### `fleet show`

| Flag | Required | Notes |
|---|---|---|
| `--fleet-id` | yes | The fleet to show |
| `--json` | no | Output as JSON |

Exits 1 with `Error: fleet 'X' not found.` if the row does not exist.
Intentionally returns soft-deleted rows, so audit info stays reachable.

### `fleet delete`

The only flag is `--fleet-id` (required), naming the fleet to delete.

Soft-deletes the fleet in one transaction: stamps `deleted_at`, deregisters
every active member (root Director included), and removes their placement rows;
messages are untouched. It is idempotent (`Deregistered 0 members.` on
re-run). Unknown `fleet_id`
exits 1 with `Error: fleet 'X' not found.`. Member panes are **not** closed —
run `cafleet member delete` per member first for a clean teardown.

## `cafleet doctor` — Placement Diagnostics {#cafleet-doctor}

Resolves the active multiplexer backend via `resolve_multiplexer()`
([Multiplexer backends](multiplexer-backends.md#backend-selection)), prints the
resolved backend and the calling pane's session/window/pane identifiers, then
the assets-install report (the runtime CLI version and every recorded
`asset_installs` row, each marked `ok` or `STALE`). `doctor` is exempt from
the [stale-assets guard](#stale-assets-guard) — a stale or missing install is
reported, not fatal.

The only flag is the optional [`--json`](#json-output), a trailing
per-subcommand flag.

```
multiplexer:
  backend:       tmux
  session:       main
  window_id:     @3
  pane_id:       %0
  presence:      TMUX=/tmp/tmux-501/default,12345,0
assets:
  cli_version: 0.6.0
  claude:      0.6.0 (2026-07-04T00:12:09.123456+00:00) ok
  codex:       0.5.0 (2026-06-20T10:00:00.987654+00:00) STALE
```

Exit 1 on any multiplexer or environment failure (no supported multiplexer
detected, ambiguous environment, binary not on `PATH`, pane not discoverable).

## `cafleet server` — Admin WebUI Server {#cafleet-server}

Starts the admin WebUI FastAPI app via uvicorn (uvicorn defaults — no reload,
workers, or log-level flags; users who need them invoke uvicorn directly,
which is what `mise //cafleet:dev` does). CLI commands do not require this
server. Does not accept `--fleet-id`.

| Flag | Default | Notes |
|---|---|---|
| `--host` | `settings.broker_host` (default `127.0.0.1`) | Bind address. Overrides `CAFLEET_BROKER_HOST` when both are set. |
| `--port` | `settings.broker_port` (default `8000`) | Bind port. Overrides `CAFLEET_BROKER_PORT` when both are set. |

Flag wins over env var; env var wins over the hardcoded default. If the
bundled WebUI dist directory does not exist, the app warns on stderr
(`warning: admin WebUI is not built. / will return 404. Run 'mise
//admin:build'.`); port-in-use errors propagate unwrapped from uvicorn.

## `cafleet message` — Message Broker

All five subcommands require `--fleet-id`, name the acting member
(`--from-member-id` on `send` / `broadcast`, `--member-id` on `poll` / `ack` /
`show`), and run behind the
[stale-assets guard](#stale-assets-guard).
The envelope schema is canonical in
[Message envelope](./message-envelope.md); truncation and `--full` are
canonical [above](#message-body-truncation); per-subcommand output shapes are
in [Output shapes](#output-shapes). Behavior detail:
[`send_message`](../api/broker.md#cafleet.broker.send_message),
[`broadcast_message`](../api/broker.md#cafleet.broker.broadcast_message),
[`poll_messages`](../api/broker.md#cafleet.broker.poll_messages),
[`ack_message`](../api/broker.md#cafleet.broker.ack_message),
[`get_message`](../api/broker.md#cafleet.broker.get_message).

### `message send`

| Flag | Required | Notes |
|---|---|---|
| `--from-member-id` | yes | Sender. |
| `--to-member-id` | yes | Recipient member id. |
| `--text` | no | Inline message body. Exactly one of `--text` / `--text-file`. |
| `--text-file` | no | Path to a UTF-8 file whose contents are the body (`-` = stdin); use it for bodies that would exceed the shell's `ARG_MAX`. |
| `--full` | no | See [Message Body Truncation](#message-body-truncation). |
| `--quiet` | no | Print only the bare `message_id`, for shell capture — see [`--quiet` semantics](#quiet-semantics). |

### `message broadcast`

| Flag | Required | Notes |
|---|---|---|
| `--from-member-id` | yes | Broadcaster (sender). |
| `--text` / `--text-file` | one of | Message body, as on `message send`. |
| `--full` | no | See [Output shapes](#output-shapes). |

`delivered=<k>` counts the best-effort inline previews that landed.

### `message poll`

| Flag | Required | Notes |
|---|---|---|
| `--member-id` | yes | Recipient whose inbox is fetched. |
| `--full` | no | See [Message Body Truncation](#message-body-truncation). |

Returns only un-acked (`input_required`) deliveries addressed to the member.

### `message ack`

| Flag | Required | Notes |
|---|---|---|
| `--member-id` | yes | Recipient acknowledging the message (recipient-only). |
| `--message-id` | yes | Message to acknowledge. |
| `--full` | no | See [Message Body Truncation](#message-body-truncation). |
| `--quiet` | no | Print only the bare `message_id`, for shell capture — see [`--quiet` semantics](#quiet-semantics). |

### `message show`

| Flag | Required | Notes |
|---|---|---|
| `--member-id` | yes | The acting member (fleet-membership gate). |
| `--message-id` | yes | Message to fetch. |
| `--full` | no | See [Message Body Truncation](#message-body-truncation). |

## `cafleet member` — Member Lifecycle + Pane Interaction {#cafleet-member}

The `cafleet member` subgroup owns the member lifecycle: `create` registers a
member **and** spawns its coding-agent pane; `delete` tears it down; `capture`
/ `prompt` / `ping` inspect or keystroke an existing member's pane;
`show` and `list` are registry reads (no multiplexer requirement). All run
behind the [stale-assets guard](#stale-assets-guard). Behavior detail:
[`register_member`](../api/broker.md#cafleet.broker.register_member),
[`deregister_member`](../api/broker.md#cafleet.broker.deregister_member),
[`get_member`](../api/broker.md#cafleet.broker.get_member),
[`list_members`](../api/broker.md#cafleet.broker.list_members).

### Member targeting and key delivery

Resolution shared by the `--member-id` verbs, by target state:

| Target state | `capture` / `prompt` / `ping` | `show` | `delete` |
|---|---|---|---|
| Active, placed with a `pane_id` | Dispatches | Shows the member | Kills the pane, then soft-deletes |
| Active, placement pending (`pane_id` is `None`) | Exit 1 | Shows the member | Tolerated — a plain registry soft-delete |
| Active, no placement row | Exit 1 | Tolerated | Tolerated — a plain registry soft-delete |
| Cross-fleet, unknown, or inactive | Exit 1, `Error: Member <member-id> not found` | The same error | The same error |

Any active in-fleet member (the root Director included) is a valid target;
there is no caller-auth check beyond fleet membership. Key sequences are
delivered **literally** (`send-keys` with `shell=False`) — shell meta, key
names, and multi-byte characters all arrive as plain characters.

| Exit | Meaning |
|---|---|
| `0` | Dispatch success |
| `1` | Multiplexer unavailable |
| `1` | Member not found |
| `1` | Missing placement |
| `1` | Pending placement |
| `1` | A `send-keys` failure |
| `2` | Per-subcommand argument or validation errors |

### `member create` {#member-create}

Register a member **and** spawn its coding-agent pane. It takes no identity
flag: the acting Director is auto-resolved from `fleets.director_member_id`
first thing, before registration (the resolved id also feeds the member's
backend inheritance and the spawn-prompt substitution). A fleet has exactly
one root Director by construction, so no override flag exists.

| Flag | Required | Notes |
|---|---|---|
| `--name` | yes | Display name — see [Known asymmetries](../concepts/coding-agents.md#known-asymmetries-intentional-non-goals) for pane-title behavior. |
| `--description` | yes | One-sentence purpose. |
| `--coding-agent` | no | One of `claude`, `codex`, or `opencode`; when omitted, the member — every role — inherits the spawning Director's placement backend. Exits 1 with `Error: binary <name> not found on PATH` when the binary is missing. |
| `--model` | no | Model forwarded to the backend binary's own `--model` flag. The opencode backend additionally requires `<provider-id>/<model-id>`; per-backend formats and create-time validation are in [Model selection](coding-agent-backends.md#model-selection). |
| `--effort` | no | Reasoning-effort level forwarded to the backend binary, validated per backend before any side effect. Accepted levels, forwarding forms, and rejection strings are in [Reasoning effort](coding-agent-backends.md#reasoning-effort). |
| `--role` | no | `member` (default) or `monitor`. `monitor` spawns the fleet's single dedicated monitoring member; a second spawn is rejected. See [Monitoring](../concepts/monitoring.md#the-monitoring-member). |
| `--full` | no | Switches the non-JSON output to the 6-line labeled block. |
| `--text` | no | Inline spawn prompt (backend-neutral template). Exactly one of `--text` / `--text-file`. |
| `--text-file` | no | Path to a UTF-8 file whose contents are the spawn prompt (`-` = stdin). Inline prompts beyond a few KB exceed the multiplexer argv ceiling — use `--text-file` for long prompts. |

#### Spawn command per backend

The per-backend spawn argv and auto-approval flags live in
[Coding-agent backends](coding-agent-backends.md#spawn-argv).

#### Spawn-prompt substitution

`cafleet member create` runs `str.format` over the resolved prompt body,
substituting exactly four placeholders:

| Placeholder | Substituted value | How the spawned member sees it |
|---|---|---|
| `{fleet_id}` | The member's fleet id | `FLEET ID: <fleet_id>` |
| `{member_id}` | The member's own newly-allocated id | `YOUR MEMBER ID: <member_id>` |
| `{director_member_id}` | The fleet's root Director id | `DIRECTOR MEMBER ID: <director_member_id>` |
| `{coding_agent}` | The resolved backend name (`claude`, `codex`, or `opencode`) | `CODING AGENT: <coding_agent>` |

Identity reaches the spawned member as literals rendered into its prompt; the
only environment variable forwarded into the pane is `CAFLEET_DATABASE_URL`.
A literal brace must be doubled (`{{` / `}}`); an unknown placeholder or
malformed brace expression exits 2 and rolls back the just-registered member
(see [Error Messages](#error-messages)).

The spawn always creates the pane without stealing focus (tmux
`split-window -d`): the Director's pane and active window stay active. In the
default output, `pane` renders `(pending)` until the pane id is patched onto
the placement.

### `member delete` {#member-delete}

The only flag is `--member-id` (required), naming the member to delete.

Tears down the target's pane (when one exists) and soft-deletes the member.
Targeting the root Director is blocked (see
[Error Messages](#error-messages)). A placementless or pending-placement
delete is a pure registry soft-delete and succeeds outside a multiplexer.
The pane path kills the pane immediately (tolerating an already-gone pane),
then soft-deletes; exit 0. Pane status renders `(no placement)` for a
placementless target and `(pending — no pane)` for a pending placement.

### `member show` {#member-show}

| Flag | Required | Notes |
|---|---|---|
| `--member-id` | yes | Any active in-fleet registry entry — placed or placementless (root Director included). |
| `--full` | no | Text mode only — see [Output shapes](#output-shapes). |

Registry read — no multiplexer requirement. In the `--full` block, the
placement sub-block renders `placement:   none` when the member is
placementless, and `None` fields render `-`. `kind` is one of `director`,
`monitor`, or `member`.

### `member list` {#member-list}

No flags beyond `--fleet-id` and the shared trailing [`--json`](#json-output)
flag; no identity flag. Lists every **active** registry entry of the fleet —
the root Director, the monitoring member, ordinary members, and placementless
rows. An empty roster prints `0 members.`.

| Field | Text column | Text rendering when absent | JSON key | JSON type |
|---|---|---|---|---|
| `member_id` | yes | — | `member_id` | integer |
| `name` | yes | — | `name` | string |
| `kind` | yes (`director` / `monitor` / `member`) | — | `kind` | string |
| `backend` | yes | `-` for a placementless row | — (inside `placement`) | — |
| `pane_id` | yes | `-` placementless; `(pending)` before the pane id is patched | — (inside `placement`) | — |
| `idle` | yes | `-` when the member has no message activity | `idle` | integer seconds or `null` |
| `placement` | — | — | `placement` | the placement sub-dict, `null` for a placementless row |
| `last_sent` | — | — | `last_sent` | ISO timestamp or `null` |
| `last_recv` | — | — | `last_recv` | ISO timestamp or `null` |
| `last_ack` | — | — | `last_ack` | ISO timestamp or `null` |

`idle` is the wall-time since the member's most recent message activity — the
latest of `last_sent` (most recent outgoing message) and `last_recv` (most
recent delivery), broadcast summaries excluded — humanized as `Ns` / `Nm` /
`Nh` in text mode. `last_ack` is the most recent acknowledged delivery.
Per-member detail such as `description` and `registered_at` lives on
[`member show`](#member-show).

### `member capture` {#member-capture}

| Flag | Required | Notes |
|---|---|---|
| `--member-id` | yes | Target member's ID |
| `--lines` | no | Number of trailing lines to capture (default: **20**). |
| `--ansi` / `--no-ansi` | no | Default `--no-ansi` strips ANSI escapes and cleans carriage-return redraws; `--ansi` emits the raw capture. |

Output shapes are in [Output shapes](#output-shapes).

### `member prompt` {#member-prompt}

Director-only keystroke primitive with two forms. The plain form keystrokes
`TEXT` into a member's pane as a submitted user turn — for text that only
takes effect when it arrives as a direct user turn (slash commands, skill
invocations, and other magic commands a broker message body cannot trigger).
The `--shell` form keystrokes `! TEXT` so the coding agent's `!` shortcut runs
the command natively — honored by all three backends; it is the dispatch half
of the cafleet skill's bash-via-Director fallback protocol. Broker messaging
remains the canonical
coordination channel; the plain form is not a substitute for `message send`.

| Flag / argument | Required | Notes |
|---|---|---|
| `--member-id` | yes | Target member's ID |
| `--shell` | no | Boolean flag, default off. Dispatch `! TEXT` (shell form) instead of `TEXT` (plain form). |
| *(positional `TEXT`)* | yes | Single line of text; leading/trailing whitespace stripped before dispatch. Newline-containing or empty-after-strip text exits 2. |

Shell metacharacters — pipes, `&&`, `;`, `$(...)`, and backticks — are
forwarded opaquely. The newline check runs first, against the original text.

The `--shell` flag controls both the payload prefix and the Esc safeguard:

| Form | Keystroke sequence | Follow-up |
|---|---|---|
| Plain (no `--shell`) | `Esc` → settle → literal `TEXT` → `Enter` | None |
| `--shell` | literal `! TEXT` → `Enter` | `cafleet member ping` required |

In the plain form the trailing `Enter` submits a real user turn, and the
leading `Esc` (as in `member ping` and inline previews) keeps it from blindly
confirming a pending permission prompt; the submitted turn opens the member's
turn directly. The `--shell` form leads with no `Esc` because an `Esc` before
`! <cmd>` would mis-fire (see
[Esc safeguard](multiplexer-backends.md#esc-safeguard)), and its bang output
only stages in the pane — the ping advances the member's turn to consume it.

The flag performs no content inspection: plain-form `TEXT` beginning with `!`
is delivered verbatim without the shell mechanics.

Output shapes are in [Output shapes](#output-shapes).

### `member ping` {#member-ping}

Re-pokes a member's inbox: keystrokes `Esc` → `cafleet message poll
--fleet-id <fleet-id> --member-id <member-id>` → `Enter` into the target's pane
(the leading `Esc` is the permission-prompt safeguard — see
[Push notifications](multiplexer-backends.md#esc-safeguard)). The manual
re-poke for a pane that missed the broker's automatic on-delivery
notification; the action is wholly fixed by the command — no
operator-controlled body — which is why `member ping` sits in
`permissions.allow` while `member prompt` stays in `permissions.ask`.

| Flag | Required | Notes |
|---|---|---|
| `--member-id` | yes | The target member. |
| `--quiet` | no | Print only the bare member id, for shell capture. |

A keystroke non-delivery exits 1.

## `cafleet monitor` — Supervision Scheduler {#cafleet-monitor}

The per-fleet scheduler that wakes the monitoring member whenever a watched
member is due. All three subcommands require `--fleet-id` and run behind the
[stale-assets guard](#stale-assets-guard). The conceptual model is canonical
on the [Monitoring](../concepts/monitoring.md) concepts page; there is no
`monitor stop` — the loop terminates with the monitoring member's pane
(`member delete`), and a still-running loop self-terminates on its next tick
after `fleet delete`. Behavior detail:
[`list_monitor_targets`](../api/broker.md#cafleet.broker.list_monitor_targets),
[`record_pings`](../api/broker.md#cafleet.broker.record_pings),
[`get_monitor_config`](../api/broker.md#cafleet.broker.get_monitor_config),
[`update_monitor_config`](../api/broker.md#cafleet.broker.update_monitor_config),
[`monitor_runtime_payload`](../api/broker.md#cafleet.broker.monitor_runtime_payload).

### `monitor start`

The one flag is `--tick` (optional): the scan-tick cadence in seconds
(`click.IntRange(min=1)`, default **5**). The tick is the floor on interval
precision — see
[Monitoring](../concepts/monitoring.md#cadence-and-tick-precision).

Runs the loop **in-process** (the monitoring member launches it as a
background task in its own pane; the loop blocks the task and writes to its
stdout — one `<iso-ts> due member <id> (<name>) [<reasons>] -> wake monitor`
line per due member). On startup it runs the multiplexer precondition guard,
atomically claims the single-instance `monitor_runtime` row, and installs
`SIGTERM`/`SIGINT` handlers (a clean stop clears the row). If the fleet has no
monitoring member, it warns on stderr and runs anyway.

| Exit | Meaning |
|---|---|
| `0` | Clean exit |
| `1` | A monitor is already running for the fleet |
| `1` | Unknown fleet |
| `1` | Multiplexer unreachable |
| `2` | Usage errors |

### `monitor status`

No flags beyond `--fleet-id` and the shared [`--json`](#json-output) flag.
Reports runtime liveness derived from the DB
heartbeat (true even when the process died silently) plus the watched-member
schedule table:

```
monitor: running (pid 4821, last tick 2s ago, tick 5s, started 2026-06-13T04:50:00+00:00)
  member_id  name         role      interval  last_ping  enabled  pending
  ---------  -----------  --------  --------  ---------  -------  -------
  2          Director      director  180s      8s ago     yes      1
  5          alice         member    720s      -          yes      0
```

The monitoring member is not enrolled and never shows in the table.

### `monitor config`

| Flag | Required | Notes |
|---|---|---|
| `--member-id` | yes | The enrolled, in-fleet member whose schedule is shown or edited. |
| `--interval` | no | New ping interval in seconds (`click.IntRange(min=1)`). |
| `--enable` / `--disable` | no | Enable or disable monitoring for the member. Mutually exclusive. |

With no edit flag, prints the current config; with an edit flag, applies and
prints the new config. `last_ping` renders `-` when the member has never been
pinged. Exits 1 for a not-in-fleet or not-enrolled member.

## Error Messages

| Command | Situation | Error message | Exit | Notes |
|---|---|---|---|---|
| (any fleet-scoped command) | No assets install recorded (missing DB file, missing `asset_installs` table, or zero rows) | `Error: no assets install is recorded; run 'cafleet setup' first` | 1 | See [Stale-assets guard](#stale-assets-guard) |
| (any fleet-scoped command) | A recorded `asset_installs` version differs from the runtime CLI version | `Error: stale assets detected (<agent>=<recorded>[, ...]; CLI <runtime>); run 'cafleet setup' to reinstall` | 1 | See [Stale-assets guard](#stale-assets-guard) |
| `setup` | The `asset_installs` table is missing as the assets half starts | `the database schema is missing or outdated; run 'cafleet setup' first` | 1 | An assets-half failure, after a db-half failure or an externally broken schema |
| (any fleet-scoped command) | Missing `--fleet-id` | `Error: --fleet-id <int> is required for this subcommand. Create a fleet with 'cafleet fleet create' and pass its id.` | 1 | — |
| (any `--member-id` command) | Missing `--member-id` | `Error: Missing option '--member-id'.` | 2 | — |
| `fleet create` | No `--name` | `Error: Missing option '--name'.` | 2 | — |
| `fleet create` | `--coding-agent` omitted | `Error: Missing option '--coding-agent'. Choose from:` followed by `claude,` / `codex,` / `opencode`, one per tab-indented line | 2 | Recorded verbatim under [`fleet create`](#fleet-create) |
| `fleet create` | Run outside a supported multiplexer | `Error: cafleet fleet create must be run inside a tmux or herdr session` | 1 | No DB writes |
| `fleet delete` | Unknown fleet_id | `Error: fleet 'X' not found.` | 1 | — |
| `member create` | Unknown `--fleet-id` | `Error: Fleet '<fleet-id>' not found.` | 2 | Director auto-discovery runs first thing |
| `member create` | Into a soft-deleted fleet | `Error: fleet X is deleted` | 1 | — |
| `member create` | The fleet row has no `director_member_id` recorded | `Error: fleet <fleet-id> has no root Director recorded; re-create the fleet with 'cafleet fleet create'.` | 1 | Mid-bootstrap corruption |
| `member create` | With a placement, when the fleet's root Director is not an active member | `Error: fleet <fleet-id>'s root Director (member <id>) is not active.` | 1 | The `register_member` invariant guard |
| `member delete` | Against the root Director's id | `Error: cannot deregister the root Director; use 'cafleet fleet delete' instead` | 1 | — |
| `message *` | An acting member id (`--member-id` / `--from-member-id`) not in `--fleet-id` | `Error: member <member-id> is not in fleet <fleet-id>.` | 1 | The fleet-membership gate runs before any read/write operation, and also fires for an unknown id |
| `member prompt` | Missing positional `TEXT` | `Error: Missing argument 'TEXT'.` | 2 | — |
| `member prompt` | `\n` or `\r` in the text | `Error: text may not contain newlines.` | 2 | Checked first, against the original text — a `"\n"`-only input raises this, not the empty-text error |
| `member prompt` | Empty / whitespace-only text | `Error: text may not be empty.` | 2 | — |
| `member capture` / `prompt` / `ping` | The member has a pending placement | <code>Error: member &lt;id&gt; has no pane yet (pending placement) — nothing to &lt;capture&#124;prompt&#124;ping&gt;.</code> | 1 | — |
| `member ping` | The keystroke fails | `Error: send failed: tmux send-keys did not deliver the poll-trigger keystroke to pane <pane>.` | 1 | — |
| `member show` / `capture` / `prompt` / `ping` | A cross-fleet, unknown, or inactive target member id | `Error: Member <member-id> not found` | 1 | — |
| `member capture` / `prompt` / `ping` | An in-fleet target with no placement row | ``Error: member <member-id> has no placement row; it was not spawned via `cafleet member create`.`` | 1 | — |
| `message send` / `message broadcast` / `member create` | Neither `--text` nor `--text-file` | `Error: Provide exactly one of --text or --text-file.` | 2 | — |
| `message send` / `message broadcast` / `member create` | Both flags | `Error: --text and --text-file are mutually exclusive.` | 2 | — |
| `message send` / `message broadcast` / `member create` | `--text` empty or whitespace-only | `Error: text may not be empty.` | 2 | — |
| `message send` / `message broadcast` / `member create` | `--text-file <path>` to an empty (zero-byte or whitespace-only) file | `Error: --text-file <path>: file is empty.` | 1 | — |
| `message send` / `message broadcast` / `member create` | `--text-file -` with empty or whitespace-only stdin | `Error: --text-file -: stdin is empty.` | 1 | — |
| `message send` / `message broadcast` / `member create` | `--text-file <path>` to a non-existent path or non-regular file | `Error: --text-file <path>: file does not exist or is not a regular file.` | 1 | — |
| `message send` / `message broadcast` / `member create` | `--text-file <path>` to an unreadable file | `Error: --text-file <path>: file is not readable.` | 1 | — |
| `message send` / `message broadcast` / `member create` | `--text-file <path>` to a file containing invalid UTF-8 | `Error: --text-file <path>: file is not valid UTF-8.` | 1 | — |
| `member create` | `--coding-agent opencode --model` violating the `<provider-id>/<model-id>` format | `Error: --model for the opencode backend must be '<provider-id>/<model-id>' (got '<value>').` | 2 | Fires before any side effect |
| `member create` | `--effort` with a level unknown to the claude backend | `Error: --effort for the claude backend must be one of low, medium, high, xhigh, max (got '<value>').` | 2 | Fires before any side effect |
| `member create` | `--coding-agent codex --effort` with an unknown level | `Error: --effort for the codex backend must be one of minimal, low, medium, high, xhigh (got '<value>').` | 2 | Fires before any side effect |
| `member create` | `--coding-agent opencode --effort` with any value | `Error: opencode does not support reasoning effort.` | 2 | Fires before any side effect |
| `member create` | An unknown `{placeholder}` in the prompt | `Error: Unknown placeholder '<name>' in custom prompt. Supported placeholders: {fleet_id}, {member_id}, {director_member_id}, {coding_agent}. Double literal braces ({{, }}) to keep them as text.` | 2 | The just-registered member is rolled back |
| `member create` | A malformed brace expression in the prompt | `Error: Malformed custom prompt: <detail>. Double literal braces ({{, }}) to keep them as text.` | 2 | The just-registered member is rolled back |
| `member create` | `--role monitor` when the fleet already has an active monitoring member | `Error: fleet <id> already has an active monitoring member (member <existing-id>); only one is allowed.` | 1 | — |
| `member create` | `--coding-agent` omitted and the spawning Director not found in the fleet | `Error: cannot resolve the member's coding agent: Director <director-id> not found in fleet <fleet-id>. Re-run with an explicit --coding-agent.` | 1 | Nothing spawned |
| `member create` | `--coding-agent` omitted and the spawning Director has no placement row | `Error: cannot resolve the member's coding agent: Director <director-id> has no placement row recording its backend. Re-run with an explicit --coding-agent.` | 1 | Nothing spawned |
| `monitor start` | The fleet already has a live monitor | `Error: monitor already running for fleet <id>` | 1 | — |
| `monitor start` / `monitor status` | An unknown or soft-deleted fleet | `Error: fleet <id> not found` | 1 | — |
| `monitor config` | Both `--enable` and `--disable` | `Error: --enable and --disable are mutually exclusive.` | 2 | — |
| `monitor config` | A member not in the fleet or not enrolled | `Error: member <id> is not enrolled in monitoring for fleet <fleet-id>.` | 1 | — |
