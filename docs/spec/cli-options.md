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
| `setup` | Create the database schema + install the skills (bare group invocation) | no | none | [setup](#cafleet-setup) |
| `setup db` | Migrate the database schema only | no | none | [setup db](#setup-db) |
| `setup skill` | Install the skills + record the installed version | no | none | [setup skill](#setup-skill) |
| `doctor` | Print the resolved multiplexer backend + the calling pane's identifiers + the skills-install report | no | none | [doctor](#cafleet-doctor) |
| `server` | Start the admin WebUI server | no | none | [server](#cafleet-server) |
| `fleet create` | Create a fleet with its root Director | no | none | [fleet create](#fleet-create) |
| `fleet list` | List non-deleted fleets | no | none | [fleet list](#fleet-list) |
| `fleet show` | Show one fleet (soft-deleted included) | yes | none | [fleet show](#fleet-show) |
| `fleet delete` | Soft-delete a fleet and deregister its members | yes | none | [fleet delete](#fleet-delete) |
| `message send` | Send a unicast message | yes | `--from-member-id` (sender) + `--to-member-id` (recipient) | [message send](#message-send) |
| `message broadcast` | Broadcast a message to all fleet members | yes | `--from-member-id` (sender) | [message broadcast](#message-broadcast) |
| `message poll` | Fetch un-acked incoming messages | yes | `--member-id` | [message poll](#message-poll) |
| `message ack` | Acknowledge a received message | yes | `--member-id` | [message ack](#message-ack) |
| `message cancel` | Retract an un-acked sent message | yes | `--member-id` | [message cancel](#message-cancel) |
| `message show` | Show one message | yes | `--member-id` | [message show](#message-show) |
| `member create` | Register a member and spawn its coding-agent pane | yes | none (Director auto-resolved) | [member create](#member-create) |
| `member delete` | Tear down a member's pane (when one exists) and deregister it | yes | `--member-id` | [member delete](#member-delete) |
| `member show` | Show one member's detail | yes | `--member-id` | [member show](#member-show) |
| `member list` | List every active registry entry of the fleet | yes | none | [member list](#member-list) |
| `member capture` | Capture the tail of a member's pane | yes | `--member-id` | [member capture](#member-capture) |
| `member exec` | Dispatch a shell command into a member's pane | yes | `--member-id` | [member exec](#member-exec) |
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

## Global Options

Placed **before** the subcommand:

| Flag | Required | Notes |
|---|---|---|
| `--version` | no | Print `cafleet <version>` and exit 0. Bypasses the `--fleet-id` requirement. |

### `--full` semantics (cross-subcommand escape hatch) {#full-semantics}

`--full` is the global "give me every field cafleet has, untruncated,
unfiltered" escape hatch — a documented flag on every subcommand that accepts
it:

| Subcommand | Default behavior | `--full` behavior |
|---|---|---|
| `message {send,poll,ack,cancel,show}` | `text` truncated to `CAFLEET_MAX_TEXT_LEN` codepoints + `…`; compact rendered envelope. | Untruncated `text`; the full typed-column message dict in `--json`, the verbose labeled block in text mode (see [Message envelope](./message-envelope.md#text-mode)). |
| `message broadcast` | One-line summary (`broadcast id=<id> recipients=<N> delivered=<k>`). | The single `broadcast_summary` message rendered as the full verbose envelope. Never per-recipient envelopes or a `recipient_ids` list. |
| `member show` | Compact one-line row `<member_id> <name> <status>`. | Labeled block with `kind`, `skills`, and the placement sub-block. Text mode only — JSON is the unprojected broker dict regardless. |
| `member create` | One compact line: `<member_id> <name> backend=<coding_agent> pane=<pane_id>`. | The 6-line `Member registered and spawned.` block. |

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

Subcommands accepting `--json`:

| Group | Subcommands |
|---|---|
| `message` | `send`, `broadcast`, `poll`, `ack`, `cancel`, `show` |
| `member` | `create`, `delete`, `show`, `list`, `capture`, `exec`, `ping` |
| `monitor` | `status`, `config` |
| `fleet` | `create`, `list`, `show` |
| (root) | `doctor` |

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
requester on `message poll` / `ack` / `cancel` / `show`, the target on
`member delete` / `show` / `capture` / `exec` / `ping`, and the enrolled
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
- **`member exec` is excluded** so it stays under `permissions.ask` — its
  positional command body is operator-controlled.

```
Bash(cafleet message poll --fleet-id *)
Bash(cafleet member create --fleet-id *)
```

Apply the patterns to your user-level `~/.claude/settings.json` manually; the
repo does not ship a committed permissions block.

## Message Body Truncation

The five subcommands that emit a user-supplied delivery body —
`cafleet message {send,poll,ack,cancel,show}` — truncate the `text` body to
the first `CAFLEET_MAX_TEXT_LEN` Unicode codepoints plus a single `…`
(U+2026) suffix by default, uniformly in text and `--json` output. Length is
measured in Python `str` codepoints, never bytes. Pass `--full` (a documented
per-subcommand option) to disable truncation; it composes orthogonally with
`--json`.

| Variable | Settings field | Default | Notes |
|---|---|---|---|
| `CAFLEET_MAX_TEXT_LEN` | `max_text_len` | `200` | Also bounds the broker's inline-preview truncation. `member.description` truncation uses a separate hard-coded 60-codepoint limit. |

This applies to CLI emit sites only — FastAPI `/api/*` responses
([webui-api.md](./webui-api.md)) and `member capture` content are untouched.
A `--quiet` flag on `cafleet message send`, `cafleet message ack`, and
`cafleet member ping` suppresses the normal output and prints only the bare
`message_id` (the target member id for `ping`), for shell capture.

## `cafleet setup` — Onboarding and Schema Management {#cafleet-setup}

`cafleet setup` is a Click group with `invoke_without_command=True`: bare
`cafleet setup` runs the full onboarding sequence (the recommended end-user
path — see [Quickstart](../quickstart.md#install)), while `setup db` and
`setup skill` run one half each. No command in the group accepts `--fleet-id`.

### `setup` (bare invocation)

Takes no options and runs two independent halves, in order:

1. **db half** — initializes or migrates the registry database to the bundled
   Alembic head revision (idempotent).
2. **skills half** — downloads the `cafleet-skills-v<version>.zip` asset
   matching the installed CLI version from the corresponding GitHub Release,
   extracts the three skill directories into every detected coding-agent home,
   and records one `skill_installs` row per home.

The halves fail independently (`db half failed: <msg>` / `skills half failed:
<msg>`); if anything failed the command exits 1 with the failed halves joined
by `' and '`. Bare `setup` accepts no `--agent` — per-agent targeting lives on
[`setup skill`](#setup-skill).

### `setup db` {#setup-db}

Takes no options. Runs the db half only. Output, by prior DB state:

```
Created <db_file> and applied migrations to head (<head>).   # fresh DB
Upgraded from <old_rev> to <head>.                           # behind head
Already at head (<head>); nothing to do.                     # at head
```

It refuses two states, exiting 1:

```
Error: DB has existing tables but no alembic_version. Run `alembic stamp head` manually if you are sure the schema matches.
Error: DB schema is at revision <rev> which is unknown to this version of cafleet. Refusing to downgrade automatically.
```

`setup db` never records `skill_installs` rows (schema only).

### `setup skill` {#setup-skill}

| Flag | Required | Notes |
|---|---|---|
| `--agent` | no | One of `claude`, `codex`, or `opencode`; repeatable, duplicates deduped. Scopes the install to exactly the named agents (a named agent's home is created if missing). Omitted → auto-detect every agent whose home directory exists. |

Pre-flight: the `skill_installs` table must exist, else exit 1 with
`Error: the database schema is missing or outdated; run 'cafleet setup' or
'cafleet setup db' first` — `setup skill` does not auto-create the schema.
Per-home success output:

```
<agent>: installed cafleet, cafleet-design-doc, cafleet-research (v<version>) -> <skills dir>
```

An install failure aborts the loop; rows recorded before the failure remain.

## Stale-skills guard {#stale-skills-guard}

Every fleet-scoped command group — `fleet`, `member`, `message`, and
`monitor` — validates the recorded skills installs before any subcommand body
runs:

1. Missing DB file, missing `skill_installs` table, or zero rows → exit 1 with
   `Error: no skills install is recorded; run 'cafleet setup' first`.
2. Any recorded `cafleet_version` differing from the runtime CLI version
   (string inequality — a downgrade also triggers) → exit 1 with
   `Error: stale skills detected (<agent>=<recorded>[, ...]; CLI <runtime>);
   run 'cafleet setup skill' to reinstall`, stale agents in ascending order.
3. Otherwise the command proceeds silently.

Homes with no recorded row are not checked. Exempt surfaces: the `setup` group
(must remain runnable to repair), `doctor` (reports instead of blocking), and
`server`. Group-level help (`cafleet fleet --help`) prints before the callback
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
atomically — see [data-model.md](./data-model.md). Default
output is one compact line carrying the two ids:

```
<fleet_id> director=<director_member_id>
```

`--json` returns the fleet dict with a nested `director` (including its
`placement`).

### `fleet list`

| Flag | Required | Notes |
|---|---|---|
| `--json` | no | Output as JSON |

Lists all non-soft-deleted fleets. Text output renders `FLEET_ID`, `DIRECTOR`,
`NAME`, `MEMBERS`, `CREATED_AT` columns; each row exposes `director_member_id`
so the Director's id can be recovered after `fleet create` output scrolls
away.

### `fleet show`

| Flag | Required | Notes |
|---|---|---|
| `--fleet-id` | yes | The fleet to show |
| `--json` | no | Output as JSON |

Exits 1 with `Error: fleet 'X' not found.` if the row does not exist.
Intentionally returns soft-deleted rows (audit info stays reachable), adding a
`deleted_at:` line in text mode; `--json` always includes `deleted_at` (null
when active).

### `fleet delete`

| Flag | Required | Notes |
|---|---|---|
| `--fleet-id` | yes | The fleet to delete |

Soft-deletes the fleet in one transaction: stamps `deleted_at`, deregisters
every active member (root Director included), and removes their placement rows;
messages are untouched. Prints `Deleted fleet <fleet_id>. Deregistered N members.`
and is idempotent (`Deregistered 0 members.` on re-run). Unknown `fleet_id`
exits 1 with `Error: fleet 'X' not found.`. Member panes are **not** closed —
run `cafleet member delete` per member first for a clean teardown.

## `cafleet doctor` — Placement Diagnostics {#cafleet-doctor}

Resolves the active multiplexer backend via `resolve_multiplexer()`
([Multiplexer backends](multiplexer-backends.md#backend-selection)), prints the
resolved backend and the calling pane's session/window/pane identifiers, then
the skills-install report (the runtime CLI version and every recorded
`skill_installs` row, each marked `ok` or `STALE`). `doctor` is exempt from
the [stale-skills guard](#stale-skills-guard) — a stale or missing install is
reported, not fatal.

| Flag | Required | Notes |
|---|---|---|
| `--json` | no | Output as JSON — trailing per-subcommand flag (see [JSON output](#json-output)). |

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

All six subcommands require `--fleet-id`, name the acting member
(`--from-member-id` on `send` / `broadcast`, `--member-id` on `poll` / `ack` /
`cancel` / `show`), and run behind the
[stale-skills guard](#stale-skills-guard).
The envelope schema is canonical in
[Message envelope](./message-envelope.md); truncation and `--full` are
canonical [above](#message-body-truncation). Text output is the subcommand's
acknowledgement line (`Message sent.` / `Message acknowledged.` /
`Message canceled.`) followed by the compact rendered envelope; `message show`
prints the envelope alone. Behavior detail:
[`send_message`](../api/broker.md#cafleet.broker.send_message),
[`broadcast_message`](../api/broker.md#cafleet.broker.broadcast_message),
[`poll_messages`](../api/broker.md#cafleet.broker.poll_messages),
[`ack_message`](../api/broker.md#cafleet.broker.ack_message),
[`cancel_message`](../api/broker.md#cafleet.broker.cancel_message),
[`get_message`](../api/broker.md#cafleet.broker.get_message).

### `message send`

| Flag | Required | Notes |
|---|---|---|
| `--from-member-id` | yes | Sender. |
| `--to-member-id` | yes | Recipient member id. |
| `--text` | no | Inline message body. Exactly one of `--text` / `--text-file`. |
| `--text-file` | no | Path to a UTF-8 file whose contents are the body (`-` = stdin); use it for bodies that would exceed the shell's `ARG_MAX`. |
| `--full` | no | See [Message Body Truncation](#message-body-truncation). |

### `message broadcast`

| Flag | Required | Notes |
|---|---|---|
| `--from-member-id` | yes | Broadcaster (sender). |
| `--text` / `--text-file` | one of | Message body, as on `message send`. |
| `--full` | no | See [`--full` semantics](#full-semantics). |

Default text output is `broadcast id=<message_id> recipients=<N> delivered=<k>`,
where `k` counts the best-effort inline previews that landed. `--json` carries
both `recipients` and `delivered`.

### `message poll`

| Flag | Required | Notes |
|---|---|---|
| `--member-id` | yes | Recipient whose inbox is fetched. |
| `--full` | no | See [Message Body Truncation](#message-body-truncation). |

Returns only un-acked (`input_required`) deliveries addressed to the member;
an empty inbox prints `No messages found.`.

### `message ack`

| Flag | Required | Notes |
|---|---|---|
| `--member-id` | yes | Recipient acknowledging the message (recipient-only). |
| `--message-id` | yes | Message to acknowledge. |
| `--full` | no | See [Message Body Truncation](#message-body-truncation). |

### `message cancel`

| Flag | Required | Notes |
|---|---|---|
| `--member-id` | yes | Sender retracting the message (sender-only). |
| `--message-id` | yes | Message to cancel. |
| `--full` | no | See [Message Body Truncation](#message-body-truncation). |

### `message show`

| Flag | Required | Notes |
|---|---|---|
| `--member-id` | yes | The acting member (fleet-membership gate). |
| `--message-id` | yes | Message to fetch. |
| `--full` | no | See [Message Body Truncation](#message-body-truncation). |

## `cafleet member` — Member Lifecycle + Pane Interaction {#cafleet-member}

The `cafleet member` subgroup owns the member lifecycle: `create` registers a
member **and** spawns its coding-agent pane; `delete` tears it down; `capture`
/ `exec` / `ping` inspect or keystroke an existing member's pane;
`show` and `list` are registry reads (no multiplexer requirement). All run
behind the [stale-skills guard](#stale-skills-guard). Behavior detail:
[`register_member`](../api/broker.md#cafleet.broker.register_member),
[`deregister_member`](../api/broker.md#cafleet.broker.deregister_member),
[`get_member`](../api/broker.md#cafleet.broker.get_member),
[`list_members`](../api/broker.md#cafleet.broker.list_members).

### Member targeting and key delivery

Resolution rules shared by the `--member-id` verbs:

1. A cross-fleet, unknown, or inactive `--member-id` resolves to "not found"
   (exit 1, `Error: Member <member-id> not found`). Any active in-fleet member
   (the root Director included) is a valid target; there is no caller-auth
   check beyond fleet membership.
2. No placement row → exit 1 (see [Error Messages](#error-messages)) — except
   `member show` and `member delete`, which tolerate a missing placement.
3. A pending placement (`pane_id` is `None`) → `capture` / `exec` / `ping`
   exit 1; `delete` tolerates it.

Key sequences are delivered **literally** (`send-keys` with `shell=False`) —
shell meta, key names, and multi-byte characters all arrive as plain
characters. Common exit codes: `0` dispatch success; `1` multiplexer
unavailable, member not found, missing placement, pending placement, or a
`send-keys` failure; `2` per-subcommand argument/validation errors.

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
| `--model` | no | Model forwarded to the backend binary's own `--model` flag — see [Model selection](../concepts/coding-agents.md#model-selection). |
| `--role` | no | `member` (default) or `monitor`. `monitor` spawns the fleet's single dedicated monitoring member; a second spawn is rejected. See [Monitoring](../concepts/monitoring.md#the-monitoring-member). |
| `--full` | no | Switches the non-JSON output to the 6-line labeled block. |
| `--text` | no | Inline spawn prompt (backend-neutral template). Exactly one of `--text` / `--text-file`. |
| `--text-file` | no | Path to a UTF-8 file whose contents are the spawn prompt (`-` = stdin). Inline prompts beyond a few KB exceed the multiplexer argv ceiling — use `--text-file` for long prompts. |

#### Spawn command per backend

The per-backend spawn argv and auto-approval flags live on the backend
reference pages: [Claude members](../reference/coding-agents/claude.md),
[Codex members](../reference/coding-agents/codex.md),
[Opencode members](../reference/coding-agents/opencode.md).

#### Spawn-prompt substitution

`cafleet member create` runs `str.format` over the resolved prompt body,
substituting exactly four placeholders: `{fleet_id}`, `{member_id}` (the
member's own newly-allocated id), `{director_member_id}`, and `{coding_agent}`.
Identity reaches the spawned member as literals rendered into its prompt; the
only environment variable forwarded into the pane is `CAFLEET_DATABASE_URL`.
A literal brace must be doubled (`{{` / `}}`); an unknown placeholder or
malformed brace expression exits 2 and rolls back the just-registered member
(see [Error Messages](#error-messages)).

The spawn always creates the pane without stealing focus (tmux
`split-window -d`): the Director's pane and active window stay active. Default
output is the compact line `<member_id> <name> backend=<coding_agent>
pane=<pane_id>` (`pane` renders `(pending)` until the pane id is patched onto
the placement); `--json` returns the member dict with its `placement`.

### `member delete` {#member-delete}

| Flag | Required | Notes |
|---|---|---|
| `--member-id` | yes | The member to delete (target). |

Tears down the target's pane (when one exists) and soft-deletes the member.
Targeting the root Director is blocked (see
[Error Messages](#error-messages)). A placementless or pending-placement
delete is a pure registry soft-delete and succeeds outside a multiplexer.
The pane path kills the pane immediately (tolerating an already-gone pane),
then soft-deletes; exit 0. Success output is a `Member deleted.`
header plus `member_id:` / `pane_id:` lines with pane status `<pane_id> (killed)`;
JSON is `{member_id, pane_status}`.

### `member show` {#member-show}

| Flag | Required | Notes |
|---|---|---|
| `--member-id` | yes | Any active in-fleet registry entry — placed or placementless (root Director included). |
| `--full` | no | Text mode only — see [`--full` semantics](#full-semantics). |

Registry read — no multiplexer requirement. Default text is the compact
one-line row `<member_id> <name> <status>`; `--full` renders the labeled block
with `kind`, `skills`, and the placement sub-block (`placement:   none` when
placementless; `None` fields render `-`). `--json` returns the broker
`get_member` dict unchanged regardless of `--full`. `kind` is one of
`director`, `monitor`, or `member`.

### `member list` {#member-list}

No flags beyond `--fleet-id` and the shared trailing [`--json`](#json-output)
flag; no identity flag. Lists every **active** registry entry of the fleet —
the root Director, the monitoring member, ordinary members, and placementless
rows. An empty roster prints `0 members.`.

Text output renders one row per member with `member_id`, `name`, `kind`
(`director` / `monitor` / `member`), `backend`, `pane_id`, and `idle` columns.
A placementless row renders `-` in its placement cells (`backend`, `pane_id`);
a placed row whose pane id is not yet patched renders `(pending)` in
`pane_id`. `idle` is the wall-time since the member's most recent message
activity — the latest of `last_sent` (most recent outgoing message) and
`last_recv` (most recent delivery), broadcast summaries excluded — humanized
as `Ns` / `Nm` / `Nh`, `-` when the member has no message activity.

`--json` returns one dict per row with `member_id`, `name`, `kind`,
`placement` (the placement sub-dict, `null` for a placementless row), and the
activity fields `last_sent` / `last_recv` / `last_ack` (ISO timestamps or
`null`; `last_ack` is the most recent acknowledged delivery) plus `idle`
(integer seconds or `null`). Per-member detail such as `description` and
`registered_at` lives on [`member show`](#member-show).

### `member capture` {#member-capture}

| Flag | Required | Notes |
|---|---|---|
| `--member-id` | yes | Target member's ID |
| `--lines` | no | Number of trailing lines to capture (default: **20**). |
| `--ansi` / `--no-ansi` | no | Default `--no-ansi` strips ANSI escapes and cleans carriage-return redraws; `--ansi` emits the raw capture. |

JSON: `{member_id, pane_id, lines, content}`; text emits the content
with no trailing newline.

### `member exec` {#member-exec}

Director-only shell-dispatch primitive. Keystrokes `! <command>` + `Enter`
into a member's pane so the coding agent's `!` shortcut runs the command
natively — honored by all three backends. This is the dispatch half of the
bash-via-Director fallback protocol, canonical in the cafleet skill's
`reference/exec-routing.md`.

| Flag / argument | Required | Notes |
|---|---|---|
| `--member-id` | yes | Target member's ID |
| *(positional `COMMAND`)* | yes | Single shell command; leading/trailing whitespace stripped before dispatch. Pipes, `&&`, `;`, `$(...)`, and backticks are forwarded opaquely. Empty (after strip) or newline-containing commands exit 2. |

Text output: `Sent bash command '<command>' to member <name> (<pane_id>).`;
JSON: `{member_id, pane_id, command}`.

### `member ping` {#member-ping}

Re-pokes a member's inbox: keystrokes `Esc` → `cafleet message poll
--fleet-id <fleet-id> --member-id <member-id>` → `Enter` into the target's pane
(the leading `Esc` is the permission-prompt safeguard — see
[Push notifications](multiplexer-backends.md#esc-safeguard)). The manual
re-poke for a pane that missed the broker's automatic on-delivery
notification; the action is wholly fixed by the command — no
operator-controlled body — which is why `member ping` sits in
`permissions.allow` while `member exec` stays in `permissions.ask`.

| Flag | Required | Notes |
|---|---|---|
| `--member-id` | yes | The target member. |
| `--quiet` | no | Print only the bare member id, for shell capture. |

Text: `Pinged member <name> (<pane_id>) — poll keystroke dispatched.`; JSON:
`{member_id, pane_id}`. A keystroke non-delivery exits 1.

## `cafleet monitor` — Supervision Scheduler {#cafleet-monitor}

The per-fleet scheduler that wakes the monitoring member whenever a watched
member is due. All three subcommands require `--fleet-id` and run behind the
[stale-skills guard](#stale-skills-guard). The conceptual model is canonical
on the [Monitoring](../concepts/monitoring.md) concepts page; there is no
`monitor stop` — stop the monitoring member's background task. Behavior
detail:
[`list_monitor_targets`](../api/broker.md#cafleet.broker.list_monitor_targets),
[`record_pings`](../api/broker.md#cafleet.broker.record_pings),
[`get_monitor_config`](../api/broker.md#cafleet.broker.get_monitor_config),
[`update_monitor_config`](../api/broker.md#cafleet.broker.update_monitor_config),
[`monitor_runtime_payload`](../api/broker.md#cafleet.broker.monitor_runtime_payload).

### `monitor start`

| Flag | Required | Notes |
|---|---|---|
| `--tick` | no | Scan-tick cadence in seconds (`click.IntRange(min=1)`, default **5**). The tick is the floor on interval precision — see [Monitoring](../concepts/monitoring.md#cadence-and-tick-precision). |

Runs the loop **in-process** (the monitoring member launches it as a
background task in its own pane; the loop blocks the task and writes to its
stdout — one `<iso-ts> due member <id> (<name>) [<reasons>] -> wake monitor`
line per due member). On startup it runs the multiplexer precondition guard,
atomically claims the single-instance `monitor_runtime` row, and installs
`SIGTERM`/`SIGINT` handlers (a clean stop clears the row). If the fleet has no
monitoring member, it warns on stderr and runs anyway. Exit `0` on clean exit;
`1` already running / unknown fleet / multiplexer unreachable; `2` usage
errors.

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

The monitoring member is not enrolled and never shows in the table. When no
monitor is running the first line reads `monitor: stopped`. JSON keeps
`last_ping_at` (ISO or null) and adds derived `*_age_seconds` fields.

### `monitor config`

| Flag | Required | Notes |
|---|---|---|
| `--member-id` | yes | The enrolled, in-fleet member whose schedule is shown or edited. |
| `--interval` | no | New ping interval in seconds (`click.IntRange(min=1)`). |
| `--enable` / `--disable` | no | Enable or disable monitoring for the member. Mutually exclusive. |

With no edit flag, prints the current config; with an edit flag, applies and
prints the new config. Text:
`member 5: interval 720s, enabled, last_ping 2026-06-13T04:51:00` (`-` when
never pinged). Exits 1 for a not-in-fleet or not-enrolled member.

## Error Messages

| Situation | Error Message |
|---|---|
| Any `fleet` / `member` / `message` / `monitor` command with no skills install recorded (missing DB file, missing `skill_installs` table, or zero rows) | `Error: no skills install is recorded; run 'cafleet setup' first` (exit 1; see [Stale-skills guard](#stale-skills-guard)) |
| Any `fleet` / `member` / `message` / `monitor` command with a recorded `skill_installs` version differing from the runtime CLI version | `Error: stale skills detected (<agent>=<recorded>[, ...]; CLI <runtime>); run 'cafleet setup skill' to reinstall` (exit 1; see [Stale-skills guard](#stale-skills-guard)) |
| `setup skill` when the `skill_installs` table is missing | `Error: the database schema is missing or outdated; run 'cafleet setup' or 'cafleet setup db' first` (exit 1) |
| Missing `--fleet-id` on a fleet-scoped subcommand | `Error: --fleet-id <int> is required for this subcommand. Create a fleet with 'cafleet fleet create' and pass its id.` (exit 1) |
| Missing `--member-id` | `Error: Missing option '--member-id'.` (exit 2) |
| `fleet create` with no `--name` | `Error: Missing option '--name'.` (exit 2) |
| `fleet create` with `--coding-agent` omitted | `Error: Missing option '--coding-agent'. Choose from:` followed by `claude,` / `codex,` / `opencode`, one per tab-indented line — recorded verbatim under [`fleet create`](#fleet-create) (exit 2) |
| `fleet create` run outside a supported multiplexer | `Error: cafleet fleet create must be run inside a tmux or herdr session` (exit 1; no DB writes) |
| `fleet delete` on unknown fleet_id | `Error: fleet 'X' not found.` (exit 1) |
| `member create` against an unknown `--fleet-id` | `Error: Fleet '<fleet-id>' not found.` (exit 2; Director auto-discovery runs first thing) |
| `member create` into a soft-deleted fleet | `Error: fleet X is deleted` (exit 1) |
| `member create` when the fleet row has no `director_member_id` recorded (mid-bootstrap corruption) | `Error: fleet <fleet-id> has no root Director recorded; re-create the fleet with 'cafleet fleet create'.` (exit 1) |
| `member create` (with a placement) when the fleet's root Director is not an active member | `Error: fleet <fleet-id>'s root Director (member <id>) is not active.` (exit 1; the `register_member` invariant guard) |
| `member delete` against the root Director's id | `Error: cannot deregister the root Director; use 'cafleet fleet delete' instead` (exit 1) |
| `message send` / `message broadcast` / `message poll` / `message ack` / `message cancel` / `message show` with an acting member id (`--member-id` / `--from-member-id`) that is not in `--fleet-id` | `Error: member <member-id> is not in fleet <fleet-id>.` (exit 1) — the fleet-membership gate runs before any read/write operation, and also fires for an unknown id. |
| `member exec` with missing positional `COMMAND` | `Error: Missing argument 'COMMAND'.` (exit 2) |
| `member exec ""` (empty / whitespace-only) | `Error: command may not be empty.` (exit 2) |
| `member exec` with `\n` or `\r` | `Error: command may not contain newlines.` (exit 2) |
| `member capture` / `member exec` / `member ping` on a member with pending placement | `Error: member <id> has no pane yet (pending placement) — nothing to <capture|exec|ping>.` (exit 1) |
| `member ping` when the keystroke fails | `Error: send failed: tmux send-keys did not deliver the poll-trigger keystroke to pane <pane>.` (exit 1) |
| `member show` / `member capture` / `member exec` / `member ping` with a cross-fleet / unknown / inactive target member id | `Error: Member <member-id> not found` (exit 1) |
| `member capture` / `member exec` / `member ping` on an in-fleet target with no placement row | ``Error: member <member-id> has no placement row; it was not spawned via `cafleet member create`.`` (exit 1) |
| Any `--text` / `--text-file` command (`message send`, `message broadcast`, `member create`) with neither flag | `Error: Provide exactly one of --text or --text-file.` (exit 2) |
| Any `--text` / `--text-file` command with both flags | `Error: --text and --text-file are mutually exclusive.` (exit 2) |
| `--text` empty or whitespace-only | `Error: text may not be empty.` (exit 2) |
| `--text-file <path>` to an empty (zero-byte or whitespace-only) file | `Error: --text-file <path>: file is empty.` (exit 1) |
| `--text-file -` with empty (or whitespace-only) stdin | `Error: --text-file -: stdin is empty.` (exit 1) |
| `--text-file <path>` to a non-existent path or non-regular file | `Error: --text-file <path>: file does not exist or is not a regular file.` (exit 1) |
| `--text-file <path>` to an unreadable file | `Error: --text-file <path>: file is not readable.` (exit 1) |
| `--text-file <path>` to a file containing invalid UTF-8 | `Error: --text-file <path>: file is not valid UTF-8.` (exit 1) |
| `member create --coding-agent opencode --model` with a value violating the `<provider-id>/<model-id>` format | `Error: --model for the opencode backend must be '<provider-id>/<model-id>' (got '<value>').` (exit 2; fires before any side effect) |
| `member create` with an unknown `{placeholder}` in the prompt | `Error: Unknown placeholder '<name>' in custom prompt. Supported placeholders: {fleet_id}, {member_id}, {director_member_id}, {coding_agent}. Double literal braces ({{, }}) to keep them as text.` (exit 2; the just-registered member is rolled back) |
| `member create` with a malformed brace expression in the prompt | `Error: Malformed custom prompt: <detail>. Double literal braces ({{, }}) to keep them as text.` (exit 2; the just-registered member is rolled back) |
| `member create --role monitor` when the fleet already has an active monitoring member | `Error: fleet <id> already has an active monitoring member (member <existing-id>); only one is allowed.` (exit 1) |
| `member create` with `--coding-agent` omitted and the spawning Director not found in the fleet | `Error: cannot resolve the member's coding agent: Director <director-id> not found in fleet <fleet-id>. Re-run with an explicit --coding-agent.` (exit 1; nothing spawned) |
| `member create` with `--coding-agent` omitted and the spawning Director has no placement row | `Error: cannot resolve the member's coding agent: Director <director-id> has no placement row recording its backend. Re-run with an explicit --coding-agent.` (exit 1; nothing spawned) |
| `monitor start` for a fleet that already has a live monitor | `Error: monitor already running for fleet <id>` (exit 1) |
| `monitor start` / `monitor status` against an unknown or soft-deleted fleet | `Error: fleet <id> not found` (exit 1) |
| `monitor config` with both `--enable` and `--disable` | `Error: --enable and --disable are mutually exclusive.` (exit 2) |
| `monitor config` against a member not in the fleet or not enrolled | `Error: member <id> is not enrolled in monitoring for fleet <fleet-id>.` (exit 1) |
