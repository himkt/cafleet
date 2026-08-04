# CLI options

How the unified CAFleet CLI (`cafleet`) accepts configuration parameters. This
page catalogs the arguments, conventions, and error strings.

## Subcommand summary

One row per subcommand. The **subject** — the id a command acts on — is a
required positional argument (see [Positional subject
ids](#positional-subject-ids)); ids that describe a relationship rather than
the subject stay as flags. "Id flags" are `member create`'s `--fleet-id` (the
fleet the new member joins) and the two-party pair `--from-member-id`
(sender) + `--to-member-id` (recipient) on the message commands.

| Subcommand | Purpose | Positional subject | Id flags | Section |
|---|---|---|---|---|
| `setup` | Migrate the database schema + install the coding-agent assets (skills and presets) | — | — | [setup](#cafleet-setup) |
| `doctor` | Print the resolved multiplexer backend + the calling pane's identifiers + the assets-install report | — | — | [doctor](#cafleet-doctor) |
| `server` | Start the admin WebUI server | — | — | [server](#cafleet-server) |
| `monitor` | Run the per-fleet scheduler loop in-process (launch as a background task) | `FLEET_ID` | — | [monitor](#cafleet-monitor) |
| `fleet create` | Create a fleet with its root Director | — | — | [fleet create](#fleet-create) |
| `fleet list` | List non-deleted fleets | — | — | [fleet list](#fleet-list) |
| `fleet show` | Show one fleet (soft-deleted included) | `FLEET_ID` | — | [fleet show](#fleet-show) |
| `fleet delete` | Soft-delete a fleet and deregister its members | `FLEET_ID` | — | [fleet delete](#fleet-delete) |
| `message send` | Send a unicast message | — | `--from-member-id` + `--to-member-id` | [message send](#message-send) |
| `message broadcast` | Broadcast a message to all fleet members | — | `--from-member-id` | [message broadcast](#message-broadcast) |
| `message poll` | Fetch un-acked incoming messages | `MEMBER_ID` | — | [message poll](#message-poll) |
| `message ack` | Acknowledge a received message | `MESSAGE_ID` | — | [message ack](#message-ack) |
| `message show` | Show one message | `MESSAGE_ID` | — | [message show](#message-show) |
| `member create` | Register a member and spawn its coding-agent pane | — | `--fleet-id` (Director auto-resolved) | [member create](#member-create) |
| `member delete` | Tear down a member's pane (when one exists) and deregister it | `MEMBER_ID` | — | [member delete](#member-delete) |
| `member show` | Show one member's detail | `MEMBER_ID` | — | [member show](#member-show) |
| `member list` | List every active registry entry of the fleet | `FLEET_ID` | — | [member list](#member-list) |
| `member prompt` | Keystroke a prompt (or, with `--shell`, a shell command) into a member's pane | `MEMBER_ID` | — | [member prompt](#member-prompt) |
| `member ping` | Inject an inbox-poll keystroke into a member's pane | `MEMBER_ID` | — | [member ping](#member-ping) |
| `member capture` | Capture the tail of a member's pane | `MEMBER_ID` | — | [member capture](#member-capture) |

## Option Source Matrix

Each parameter has exactly one input source:

| Parameter | Source |
|---|---|
| Subject id (fleet / member / message) | The required positional argument (placed after the subcommand name) |
| Database URL | `CAFLEET_DATABASE_URL` env var (optional) — defaults to `sqlite:///` + `~/.local/share/cafleet/cafleet_v6.db` (home expanded at startup); a user-supplied value must be an absolute-path `sqlite:///` URL. |
| Multiplexer backend | `CAFLEET_MULTIPLEXER` env var (optional) — unset ⇒ auto-detect. See [Multiplexer backends](multiplexer-backends.md#backend-selection). |
| New member's fleet | `--fleet-id <int>` on `member create` |
| Sender / recipient member IDs | `--from-member-id <int>` / `--to-member-id <int>` on two-party subcommands |
| JSON output | `--json` per-subcommand option (trailing canonical position — placed after all other arguments) |

## Environment variables

Every `CAFLEET_`-prefixed variable cafleet reads:

| Environment variable | Settings field | Default | Controls | Overridden by |
|---|---|---|---|---|
| `CAFLEET_DATABASE_URL` | `database_url` | `sqlite:///` + `~/.local/share/cafleet/cafleet_v6.db` | The registry database location; an absolute path is required | — |
| `CAFLEET_MULTIPLEXER` | `multiplexer` | unset ⇒ auto-detect | The multiplexer backend, per [Backend selection](multiplexer-backends.md#backend-selection) | — |
| `CAFLEET_MAX_TEXT_LEN` | `max_text_len` | `200` | Text-mode body truncation on `message {send,poll,ack,show}`, and the broker's inline-preview truncation; `--json` output is never truncated | — |
| `CAFLEET_BROKER_HOST` | `broker_host` | `127.0.0.1` | The `cafleet server` bind address | `--host` |
| `CAFLEET_BROKER_PORT` | `broker_port` | `8000` | The `cafleet server` bind port | `--port` |
| `CAFLEET_MONITOR_WAKE_INTERVAL` | `monitor_wake_interval` | `600` | The `cafleet monitor` Director wake interval in seconds; `0` disables the wake while the loop keeps heartbeating. A non-integer value fails loudly | `--interval` |

A flag wins over its environment variable, and the environment variable wins
over the hardcoded default.

## Global Options

`--version`, placed **before** the subcommand, prints `cafleet <version>` and
exits 0, short-circuiting before subcommand dispatch.

## Output shapes {#output-shapes}

One row per subcommand. Text output is the human/pane form — message bodies
truncated per [Message Body Truncation](#message-body-truncation); `--json`
is the complete, untruncated machine form.

| Subcommand | Text output | JSON payload |
|---|---|---|
| `fleet create` | The compact line `<fleet_id> director=<director_member_id>` | The fleet dict with a nested `director` (including its `placement`) |
| `fleet list` | `FLEET_ID`, `DIRECTOR`, `NAME`, `MEMBERS`, `CREATED_AT` columns, one row per fleet | Output as JSON |
| `fleet show` | The fleet row, adding a `deleted_at:` line | Always includes `deleted_at`, null when active |
| `fleet delete` | `Deleted fleet <fleet_id>. Deregistered N members.` | `{"deregistered_count": <n>}` |
| `message send` | `Message sent.` plus the compact rendered envelope | `{"message": <typed-column envelope>, "notification_sent": <bool>}`, untruncated |
| `message broadcast` | `broadcast id=<message_id> recipients=<N> delivered=<k>` | `[{"message": <summary envelope>, "recipients": <N>, "delivered": <k>}]`, untruncated |
| `message poll` | The compact rendered envelopes; `No messages found.` on an empty inbox | The typed-column envelopes, untruncated |
| `message ack` | `Message acknowledged.` plus the compact rendered envelope | `{"message": <typed-column envelope>}`, untruncated |
| `message show` | The compact rendered envelope alone | `{"message": <typed-column envelope>}`, untruncated |
| `member create` | The compact line `<member_id> <name> backend=<coding_agent> pane=<pane_id>` | The member dict with its `placement` |
| `member delete` | A `Member deleted.` header plus `member_id:` / `pane_id:` lines, pane status `<pane_id> (killed)` | `{member_id, pane_status}` |
| `member show` | The compact one-line row `<member_id> <name> <status>` | The broker `get_member` dict — the detailed view (`kind`, `skills`, `placement`) |
| `member list` | One row per member; `0 members.` on an empty roster | One dict per row |
| `member prompt` | `Sent prompt '<text>' to member <name> (<pane_id>).`, or `Sent shell prompt '<text>' …` with `--shell` | `{member_id, pane_id, text, shell}` |
| `member ping` | `Pinged member <name> (<pane_id>) — poll keystroke dispatched.`, or the pending-placement skip line — see [member ping](#member-ping) | `{member_id, pane_id, skipped}` — `skipped` present on both success paths |
| `member capture` | The captured content alone | `{member_id, pane_id, lines, content, captured_at, content_sha256}` in that key order |
| `doctor` | The `multiplexer:` and `assets:` blocks | Output as JSON |

`setup`, `server`, and `monitor` are absent by design — they stream
progress or run a loop rather than emitting a one-shot payload — so the
[Subcommand summary](#subcommand-summary) legitimately carries three more rows
than this table.

## JSON output (`--json`) {#json-output}

`--json` is the single output switch — a shared per-subcommand flag, placed
after the subcommand name, canonically **trailing**, after all other
arguments:

```bash
cafleet message poll 7 --json
cafleet member show 7 --json
```

It switches the output to compact single-line JSON; non-ASCII (like the `…`
truncation suffix in an inline preview) is emitted as UTF-8, not escaped.
JSON is always the complete, untruncated machine form — full envelopes, full
message bodies; text output is always truncated per
[Message Body Truncation](#message-body-truncation). The trailing position
keeps JSON invocations inside the existing per-subcommand allow patterns (see
[`permissions.allow` coverage](#permissionsallow-coverage)).

Subcommands accepting `--json`, one row per subcommand:

| Subcommand | Group |
|---|---|
| `doctor` | (root) |
| `fleet create` | `fleet` |
| `fleet list` | `fleet` |
| `fleet show` | `fleet` |
| `fleet delete` | `fleet` |
| `message send` | `message` |
| `message broadcast` | `message` |
| `message poll` | `message` |
| `message ack` | `message` |
| `message show` | `message` |
| `member create` | `member` |
| `member delete` | `member` |
| `member show` | `member` |
| `member list` | `member` |
| `member prompt` | `member` |
| `member ping` | `member` |
| `member capture` | `member` |

All other subcommands reject `--json` with the parser's unknown-argument
error (exit 2) — including the root group itself, so a
pre-subcommand `cafleet --json <grp> <cmd>` does not parse.

## Positional subject ids {#positional-subject-ids}

The id a command acts on rides as a required positional integer immediately
after the subcommand name: `FLEET_ID` on `fleet show` / `fleet delete` /
`member list` / `monitor`, `MEMBER_ID` on `member show` / `delete` / `prompt`
/ `ping` / `capture` and `message poll`, `MESSAGE_ID` on `message ack` /
`show`. Everything else is derived from the subject row: a member id is
globally unique, so the member row names its fleet; a message row names its
recipient and fleet. A missing subject is the parser's native
missing-required-argument error, a non-integer its native invalid-value
error (both exit 2).

Ids are DB-assigned integers, typically 1–4 digits, pasted in full — there is
no prefix resolution and no environment default: a spawned member reads its
ids from the literal `FLEET ID:` / `YOUR MEMBER ID:` / `DIRECTOR MEMBER ID:`
lines rendered into its spawn prompt and passes them as literal arguments on
every command. Members pass the literal integer — never a shell variable —
because Claude Code's `permissions.allow` matches Bash invocations as literal
command strings (see
[`permissions.allow` coverage](#permissionsallow-coverage)).

## Sender and recipient (`--from-member-id`, `--to-member-id`) {#from-to-member-id}

Two-party commands name both parties as flags — the roles need labels:
`--from-member-id` is the sender (`message send`, `message broadcast`) and
`--to-member-id` is the recipient (`message send`). Both are typed `int`
(a non-integer is the parser's invalid-value error, exit 2). The fleet is
derived from the sender row; a sender/recipient pair from different fleets
fails — see [Error Messages](#error-messages). A pane-touching target must be
an active member with a placement row — see
[Member targeting and key delivery](#member-targeting-and-key-delivery).

## `permissions.allow` coverage

The allow set is generated mechanically, one `Bash(...)` pattern per
allow-listed subcommand:

- **One pattern per subcommand**, matching the subcommand prefix —
  `Bash(cafleet <grp> <cmd> *)`. The positional subject id and trailing flags
  such as [`--json`](#json-output) are covered by the same pattern.
- **`member prompt` is excluded** so it stays under `permissions.ask` — its
  positional text body is operator-controlled, in both the plain and the
  `--shell` form.

```
Bash(cafleet message poll *)
Bash(cafleet member create *)
Bash(cafleet member capture *)
```

Apply the patterns to your user-level `~/.claude/settings.json` manually; the
repo does not ship a committed permissions block.

## Message Body Truncation

The four subcommands that emit a user-supplied delivery body —
`cafleet message {send,poll,ack,show}` — truncate the `text` body in **text
output** to the first `CAFLEET_MAX_TEXT_LEN` Unicode codepoints plus a single
`…` (U+2026) suffix. Length is measured in Unicode codepoints, never bytes.
There is no untruncated text form; [`--json`](#json-output) is the complete,
untruncated machine form.

The limit is `CAFLEET_MAX_TEXT_LEN` — see
[Environment variables](#environment-variables).

This applies to CLI text emit sites only — the WebUI `/api/*` responses
([webui-api.md](./webui-api.md)) and `member capture` content are untouched.

## Text bodies (positional `TEXT` / `--file`) {#text-body-input}

`message send`, `message broadcast`, and `member create` take their body as a
positional argument (`TEXT`; named `PROMPT` on `member create`) with `--file
PATH` as the alternative. Exactly one of the positional and `--file` must be
supplied — supplying neither or both is the parser's native
argument-group error (exit 2). `--file -` reads the body from stdin; use
`--file` for bodies that would exceed the shell's `ARG_MAX` or the
multiplexer argv ceiling. The body is used verbatim (no stripping); an empty
or whitespace-only body is rejected uniformly across inline / file / stdin —
the error strings are in [Error Messages](#error-messages).

`member prompt` also takes its text as a positional `TEXT`, but has no
`--file` alternative — its body is a one-line keystroke by contract.

## `cafleet setup` — Onboarding and Schema Management {#cafleet-setup}

`cafleet setup` is a plain command — the single onboarding and
schema-management entry point (the recommended end-user path — see
[Quickstart](../quickstart.md#install)). Command help: `Migrate the database
schema and install the coding-agent assets (skills and presets).` It takes no
positional arguments — `cafleet setup <word>` fails with the parser's
unexpected-argument error.

The one flag is `--skip AGENT`: optional and repeatable, a choice over
`claude` / `codex` / `opencode`, with duplicates deduplicated
and an unknown value failing with the parser's invalid-value error
(exit 2). Its help text is
`Skip the named agent's assets install (repeatable).`

The command runs two halves, in order:

1. **db half** — initializes or migrates the registry database to the head of
   the migration chain embedded in the binary (idempotent). Each refusal
   message below becomes the db-half failure `<msg>`; `<M>` / `<N>` are the
   integer migration versions:

    | Prior DB state | Outcome | Output / refusal message |
    |---|---|---|
    | No DB file | Created and migrated to head | `Created <db_file> and applied migrations to head (<N>).` |
    | Behind head | Upgraded | `Upgraded from <M> to <N>.` |
    | At head | No-op | `Already at head (<N>); nothing to do.` |
    | Tables present but no `refinery_schema_history` | Refused | `DB has existing tables but no refinery_schema_history. Refusing to migrate an unversioned database.` |
    | Version unknown to this CLI version | Refused | `DB schema is at version <M> which is unknown to this version of cafleet. Refusing to downgrade automatically.` |

2. **assets half** — targets are the fixed list `claude`, `codex`, `opencode`
   (in that order) minus the skipped agents. Installs, from the data embedded
   in the binary at build time with no network access, per target the three
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
| assets | A skills install fails for a target | `failed to install skills into <skills_dir>: <error>` | Aborts the loop; rows recorded before the failure remain |
| assets | A preset install fails for a target | `failed to install preset into <target>: <error>` | Aborts the loop; rows recorded before the failure remain |
| assets | All three agents skipped | `assets half skipped (all agents skipped)` | Not-run; cannot contribute a failure |

The assets-half pre-flight fires only after a db-half failure or an externally
broken schema, since the db half always runs first within the same command; it
is kept as defense.

### Schema-only invocation {#schema-only}

The documented invocation for "bring the DB to head without touching assets"
(the contributor and CI path):

```bash
cafleet setup --skip claude --skip codex --skip opencode
```

It is deterministic (independent of which agent homes exist) and exits 0 when
the db half succeeds. The schema-only invocation never records
`asset_installs` rows.

### Assets half {#assets-half}

Each agent's preset, where one exists, is a static file embedded in the
binary next to the skills:

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

Every fleet-scoped surface — the `fleet`, `member`, and `message` groups plus
the `monitor` command — validates the recorded assets installs before any
subcommand body runs:

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
| `server` | It serves the WebUI rather than running a fleet-scoped command | Starts normally |

`--help` renders at parse time and exits before any command body runs, so
neither group-level help (`cafleet fleet --help`) nor subcommand help
(`cafleet fleet create --help`) triggers the guard — both always print help,
even under a missing or stale install.

## `cafleet fleet` — Fleet Management

Fleet lifecycle; writes directly to SQLite — no server required.

### `fleet create`

| Flag | Required | Notes |
|---|---|---|
| `--name` | yes | Human-readable name for the fleet |
| `--coding-agent` | yes | One of `claude`, `codex`, or `opencode`, recorded as the root Director's placement `coding_agent` — the operator declares the backend the Director is actually running on; see [Coding agents](../concepts/coding-agents.md). |
| `--json` | no | Output as JSON |

Omitting `--coding-agent` exits 2 with the parser's native
missing-required-argument error naming `--coding-agent`.

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

`cafleet fleet show FLEET_ID [--json]` — the positional `FLEET_ID` names the
fleet to show.

Exits 1 with `Error: fleet 'X' not found.` if the row does not exist.
Intentionally returns soft-deleted rows, so audit info stays reachable.

### `fleet delete`

`cafleet fleet delete FLEET_ID [--json]` — the positional `FLEET_ID` names
the fleet to delete.

Soft-deletes the fleet in one transaction: stamps `deleted_at`, deregisters
every active member (root Director included), and removes their placement rows;
messages are untouched. It is idempotent (`Deregistered 0 members.` on
re-run). Unknown `FLEET_ID`
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

Starts the admin WebUI app under the built-in HTTP server (single process, no
auto-reload, no worker or log-level flags). CLI commands do not require this
server.

| Flag | Default | Notes |
|---|---|---|
| `--host` | `settings.broker_host` (default `127.0.0.1`) | Bind address. Overrides `CAFLEET_BROKER_HOST` when both are set. |
| `--port` | `settings.broker_port` (default `8000`) | Bind port. Overrides `CAFLEET_BROKER_PORT` when both are set. |

Flag wins over env var; env var wins over the hardcoded default. The WebUI
assets are embedded in the binary at build time, so the served SPA always
matches the binary; port-in-use errors propagate unwrapped.

## `cafleet message` — Message Broker

`message poll` takes the positional `MEMBER_ID` (the requester); `message
ack` / `show` take the positional `MESSAGE_ID`; `send` / `broadcast` name the
parties with `--from-member-id` / `--to-member-id`. The broker derives the
fleet from the subject row, and every subcommand runs behind the
[stale-assets guard](#stale-assets-guard).
The envelope schema is canonical in
[Message envelope](./message-envelope.md); truncation and `--json` are
canonical [above](#message-body-truncation); per-subcommand output shapes are
in [Output shapes](#output-shapes).

### `message send`

`cafleet message send --from-member-id ID --to-member-id ID (TEXT | --file
PATH) [--json]`

| Argument | Required | Notes |
|---|---|---|
| `--from-member-id` | yes | Sender. The fleet is derived from the sender row. |
| `--to-member-id` | yes | Recipient member id; must be in the sender's fleet. |
| positional `TEXT` | one of | Inline message body. Exactly one of `TEXT` / `--file`. |
| `--file PATH` | one of | Path to a UTF-8 file whose contents are the body (`-` = stdin); use it for bodies that would exceed the shell's `ARG_MAX`. |

### `message broadcast`

`cafleet message broadcast --from-member-id ID (TEXT | --file PATH) [--json]`

| Argument | Required | Notes |
|---|---|---|
| `--from-member-id` | yes | Broadcaster (sender). The fleet is derived from the sender row. |
| positional `TEXT` / `--file PATH` | one of | Message body, as on `message send`. |

`delivered=<k>` counts the best-effort inline previews that landed.

### `message poll`

`cafleet message poll MEMBER_ID [--json]` — the positional `MEMBER_ID` is the
recipient whose inbox is fetched. An unknown or inactive member exits 1 with
`Error: Member <member-id> not found`.

Returns only un-acked (`input_required`) deliveries addressed to the member.

### `message ack`

`cafleet message ack MESSAGE_ID [--json]` — the positional `MESSAGE_ID` names
the message to acknowledge. The recipient and fleet are derived from the
message row; existence and `input_required` state are the only guards.

### `message show`

`cafleet message show MESSAGE_ID [--json]` — the positional `MESSAGE_ID`
names the message to fetch; existence is the only guard.

## `cafleet member` — Member Lifecycle + Pane Interaction {#cafleet-member}

The `cafleet member` subgroup owns the member lifecycle: `create` registers a
member **and** spawns its coding-agent pane; `delete` tears it down; `prompt`
/ `ping` keystroke an existing member's pane; `capture` reads it;
`show` and `list` are registry reads (no multiplexer requirement). All run
behind the [stale-assets guard](#stale-assets-guard).

### Member targeting and key delivery

Resolution shared by the pane verbs (`capture` / `prompt` / `ping`) and the
registry verbs, by target state:

| Target state | `member capture` / `member prompt` | `member ping` | `show` | `delete` |
|---|---|---|---|---|
| Active, placed with a `pane_id` | Dispatches | Dispatches | Shows the member | Kills the pane, then soft-deletes |
| Active, placement pending (`pane_id` is `None`) | Exit 1 | Skips the keystroke; exit 0 — see [member ping](#member-ping) | Shows the member | Tolerated — a plain registry soft-delete |
| Active, no placement row | Exit 1 | Exit 1 | Tolerated | Tolerated — a plain registry soft-delete |
| Unknown or inactive | Exit 1, `Error: Member <member-id> not found` | The same error | The same error | The same error |

Any active member (the root Director included) is a valid target;
there is no caller-auth check. Key sequences are
delivered **literally** (`send-keys` with `shell=False`) — shell meta, key
names, and multi-byte characters all arrive as plain characters.

| Exit | Meaning |
|---|---|
| `0` | Dispatch success |
| `1` | Multiplexer unavailable |
| `1` | Member not found |
| `1` | Missing placement |
| `1` | Pending placement (`member capture` / `member prompt` only — `member ping` skips and exits 0) |
| `1` | A `send-keys` failure |
| `2` | Per-subcommand argument or validation errors |

### `member create` {#member-create}

Register a member **and** spawn its coding-agent pane. It takes no identity
flag: the acting Director is auto-resolved from `fleets.director_member_id`
first thing, before registration (the resolved id also feeds the member's
backend inheritance and the spawn-prompt substitution). A fleet has exactly
one root Director by construction, so no override flag exists.

| Argument | Required | Notes |
|---|---|---|
| `--fleet-id` | yes | The fleet the new member joins — a relationship flag; the subject of the command is the member being created. |
| `--name` | yes | Display name — see [Known asymmetries](../concepts/coding-agents.md#known-asymmetries-intentional-non-goals) for pane-title behavior. |
| `--description` | yes | One-sentence purpose. |
| `--coding-agent` | no | One of `claude`, `codex`, or `opencode`; when omitted, the member — every role — inherits the spawning Director's placement backend. Exits 1 with `Error: binary <name> not found on PATH` when the binary is missing. |
| `--model` | no | Model forwarded to the backend binary's own `--model` flag. The opencode backend additionally requires `<provider-id>/<model-id>`; per-backend formats and create-time validation are in [Model selection](coding-agent-backends.md#model-selection). |
| `--effort` | no | Reasoning-effort level forwarded to the backend binary, validated per backend before any side effect. Accepted levels, forwarding forms, and rejection strings are in [Reasoning effort](coding-agent-backends.md#reasoning-effort). |
| positional `PROMPT` | one of | Inline spawn prompt (backend-neutral template). Exactly one of `PROMPT` / `--file`. |
| `--file PATH` | one of | Path to a UTF-8 file whose contents are the spawn prompt (`-` = stdin). Inline prompts beyond a few KB exceed the multiplexer argv ceiling — use `--file` for long prompts. |
| `--json` | no | Output as JSON |

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

`cafleet member delete MEMBER_ID [--json]` — the positional `MEMBER_ID` names
the member to delete.

Tears down the target's pane (when one exists) and soft-deletes the member.
Targeting the root Director is blocked (see
[Error Messages](#error-messages)). A placementless or pending-placement
delete is a pure registry soft-delete and succeeds outside a multiplexer.
The pane path kills the pane immediately (tolerating an already-gone pane),
then soft-deletes; exit 0. Pane status renders `(no placement)` for a
placementless target and `(pending — no pane)` for a pending placement.

### `member show` {#member-show}

`cafleet member show MEMBER_ID [--json]` — the positional `MEMBER_ID` names
any active registry entry — placed or placementless (root Director included).

Registry read — no multiplexer requirement. Text is the compact one-line row;
the detailed view — `kind` (`director` or `member`), `skills`, and the
placement sub-dict — is the `--json` payload (see
[Output shapes](#output-shapes)).

### `member list` {#member-list}

`cafleet member list FLEET_ID [--json]` — the positional `FLEET_ID` names the
fleet whose roster is listed; no identity flag. Lists every **active**
registry entry of the fleet —
the root Director, ordinary members, and placementless rows. An empty roster
prints `0 members.`.

| Field | Text column | Text rendering when absent | JSON key | JSON type |
|---|---|---|---|---|
| `member_id` | yes | — | `member_id` | integer |
| `name` | yes | — | `name` | string |
| `kind` | yes (`director` / `member`) | — | `kind` | string |
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

| Argument | Required | Notes |
|---|---|---|
| positional `MEMBER_ID` | yes | Target member's ID (first positional) |
| positional `TEXT` | yes | Single line of text (second positional); leading/trailing whitespace stripped before dispatch. Newline-containing or empty-after-strip text exits 2. No `--file` alternative — the body is a one-line keystroke by contract. |
| `--shell` | no | Boolean flag, default off. Dispatch `! TEXT` (shell form) instead of `TEXT` (plain form). |
| `--json` | no | Output as JSON |

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

`cafleet member ping MEMBER_ID [--json]` — the positional `MEMBER_ID` names
the target member.

Re-pokes a member's inbox: keystrokes `Esc` → `cafleet message poll
<member-id> — then resume your work if
something was still running.` → `Enter` into the target's pane
(the leading `Esc` is the permission-prompt safeguard — see
[Push notifications](multiplexer-backends.md#esc-safeguard)). The manual
re-poke for a pane that missed the broker's automatic on-delivery
notification; the action is wholly fixed by the command — no
operator-controlled body — which is why `member ping` sits in
`permissions.allow` while `member prompt` stays in `permissions.ask`.

A pending placement (a placement row whose `pane_id` is not yet patched) takes
the **skip path**: no keystroke is sent and the command succeeds — the pending
member's inbox is intact and it polls it on spawn, so there is nothing a ping
would add. Exit code 0 on both success paths in every mode; the `skipped`
JSON key is present on **both** paths (stable schema).

| Mode | Normal success | Pending-placement skip |
|---|---|---|
| text | `Pinged member <name> (<pane_id>) — poll keystroke dispatched.` | `Member <name> has no pane yet (pending placement) — ping skipped; it will poll its inbox on spawn.` |
| `--json` | `{"member_id": <id>, "pane_id": "<pane_id>", "skipped": false}` | `{"member_id": <id>, "pane_id": null, "skipped": true}` |

A keystroke non-delivery, an unknown member, and a missing placement row all
still exit 1.

### `member capture` {#member-capture}

`cafleet member capture MEMBER_ID [--lines N] [--ansi] [--json]` — the
positional `MEMBER_ID` names the target member.

| Argument | Required | Notes |
|---|---|---|
| `--lines` | no | Number of trailing lines to capture (default: **20**). |
| `--ansi` | no | Preserve ANSI escapes in the raw capture. The default strips ANSI escapes and cleans carriage-return redraws. |

Output shapes are in [Output shapes](#output-shapes); target resolution is
shared with the `member` keystroke verbs — see
[Member targeting and key delivery](#member-targeting-and-key-delivery). A
pending placement is a hard error (see [Error Messages](#error-messages)).

JSON stamps `captured_at` from the local UTC
clock at the capture read boundary and computes
`content_sha256 = sha256(content.encode("utf-8"))` from the exact emitted
`content`. The default mode hashes the ANSI-stripped,
carriage-return-defragmented
string; `--ansi` hashes the ANSI-preserving string. No normalization occurs
after the selected mode, and capture content is never stored in SQLite.

## `cafleet monitor` — Supervision Scheduler {#cafleet-monitor}

`cafleet monitor FLEET_ID [--tick N] [--interval N]` — a single top-level
command with the positional `FLEET_ID` subject. It runs
behind the [stale-assets guard](#stale-assets-guard). The conceptual model is
canonical on the [Monitoring](../concepts/monitoring.md) concepts page; there
is no stop subcommand — the Director stops the background task hosting the
loop, and a still-running loop self-terminates on its next tick after
`fleet delete`.

| Flag | Required | Notes |
|---|---|---|
| `--tick` | no | The scan-tick cadence in seconds (an integer ≥ 1, default **5**). The tick is the floor on interval precision — see [Monitoring](../concepts/monitoring.md#cadence-and-tick-precision). |
| `--interval` | no | The Director wake interval in seconds (an integer ≥ 0); `0` disables the wake while the loop keeps heartbeating. When omitted, falls back to `CAFLEET_MONITOR_WAKE_INTERVAL` (default **600**). |

Runs the loop **in-process** (the Director launches it as a background task
in its own pane; the loop blocks the task and writes to its stdout — one
`<iso-ts> tick -> wake director <director-member-id> (<N> members)` line per
delivered wake). On startup it runs the multiplexer precondition guard,
atomically claims the single-instance `monitor_runtime` row, installs
`SIGTERM`/`SIGINT` handlers (a clean stop clears the row), and — immediately
after the successful claim, before the first tick — prints the startup line
the Director confirms before its first `member create`:

```
monitor loop started (fleet <fleet_id>, tick <tick>s, pid <pid>)
```

| Exit | Meaning |
|---|---|
| `0` | Clean exit |
| `1` | A monitor is already running for the fleet |
| `1` | Unknown fleet |
| `1` | Multiplexer unreachable |
| `2` | Usage errors |

## Error Messages

| Command | Situation | Error message | Exit | Notes |
|---|---|---|---|---|
| (any fleet-scoped command) | No assets install recorded (missing DB file, missing `asset_installs` table, or zero rows) | `Error: no assets install is recorded; run 'cafleet setup' first` | 1 | See [Stale-assets guard](#stale-assets-guard) |
| (any fleet-scoped command) | A recorded `asset_installs` version differs from the runtime CLI version | `Error: stale assets detected (<agent>=<recorded>[, ...]; CLI <runtime>); run 'cafleet setup' to reinstall` | 1 | See [Stale-assets guard](#stale-assets-guard) |
| `setup` | The `asset_installs` table is missing as the assets half starts | `the database schema is missing or outdated; run 'cafleet setup' first` | 1 | An assets-half failure, after a db-half failure or an externally broken schema |
| (any subject-taking command) | Missing positional subject id | The parser's native missing-required-argument error naming the positional | 2 | — |
| (any id argument) | A non-integer id | The parser's native invalid-value error | 2 | — |
| `fleet create` | No `--name` | `Error: Missing option '--name'.` | 2 | — |
| `fleet create` | `--coding-agent` omitted | `Error: Missing option '--coding-agent'. Choose from:` followed by `claude,` / `codex,` / `opencode`, one per tab-indented line | 2 | Recorded verbatim under [`fleet create`](#fleet-create) |
| `fleet create` | Run outside a supported multiplexer | `Error: cafleet fleet create must be run inside a tmux or herdr session` | 1 | No DB writes |
| `fleet show` / `fleet delete` | Unknown `FLEET_ID` | `Error: fleet 'X' not found.` | 1 | — |
| `member create` | Unknown `--fleet-id` | `Error: Fleet '<fleet-id>' not found.` | 2 | Director auto-discovery runs first thing |
| `member create` | Into a soft-deleted fleet | `Error: fleet X is deleted` | 1 | — |
| `member create` | The fleet row has no `director_member_id` recorded | `Error: fleet <fleet-id> has no root Director recorded; re-create the fleet with 'cafleet fleet create'.` | 1 | Mid-bootstrap corruption |
| `member create` | With a placement, when the fleet's root Director is not an active member | `Error: fleet <fleet-id>'s root Director (member <id>) is not active.` | 1 | The `register_member` invariant guard |
| `member delete` | Against the root Director's id | `Error: cannot deregister the root Director; use 'cafleet fleet delete' instead` | 1 | — |
| `message poll` | An unknown or inactive `MEMBER_ID` | `Error: Member <member-id> not found` | 1 | — |
| `message ack` / `message show` | An unknown `MESSAGE_ID` | `Error: Message <message-id> not found` | 1 | — |
| `message ack` | A message not in the `input_required` state | `Error: Cannot ACK message in state <state>` | 1 | — |
| `message send` / `message broadcast` | The sender is unknown or inactive | `Error: Sender member not found or not active: <from-member-id>` | 1 | — |
| `message send` | The recipient is unknown or inactive | `Error: Destination member not found: <to-member-id>` | 1 | — |
| `message send` | Sender and recipient in different fleets | `Error: members <from-member-id> and <to-member-id> are not in the same fleet.` | 1 | — |
| `member prompt` | Missing positional `TEXT` | `Error: Missing argument 'TEXT'.` | 2 | — |
| `member prompt` | `\n` or `\r` in the text | `Error: text may not contain newlines.` | 2 | Checked first, against the original text — a `"\n"`-only input raises this, not the empty-text error |
| `member prompt` | Empty / whitespace-only text | `Error: text may not be empty.` | 2 | — |
| `member capture` / `member prompt` | The member has a pending placement | <code>Error: member &lt;id&gt; has no pane yet (pending placement) — nothing to &lt;capture&#124;prompt&gt;.</code> | 1 | `member ping` instead skips and exits 0 — see [member ping](#member-ping) |
| `member ping` | The keystroke fails | `Error: send failed: tmux send-keys did not deliver the poll-trigger keystroke to pane <pane>.` | 1 | — |
| `member show` / `prompt` / `ping` / `capture` | An unknown or inactive target member id | `Error: Member <member-id> not found` | 1 | — |
| `member prompt` / `ping` / `capture` | A target with no placement row | ``Error: member <member-id> has no placement row; it was not spawned via `cafleet member create`.`` | 1 | — |
| `message send` / `message broadcast` / `member create` | Neither the positional body nor `--file`, or both | The parser's native argument-group error | 2 | — |
| `message send` / `message broadcast` / `member create` | Positional body empty or whitespace-only | `Error: text may not be empty.` | 2 | — |
| `message send` / `message broadcast` / `member create` | `--file <path>` to an empty (zero-byte or whitespace-only) file | `Error: --file <path>: file is empty.` | 1 | — |
| `message send` / `message broadcast` / `member create` | `--file -` with empty or whitespace-only stdin | `Error: --file -: stdin is empty.` | 1 | — |
| `message send` / `message broadcast` / `member create` | `--file <path>` to a non-existent path or non-regular file | `Error: --file <path>: file does not exist or is not a regular file.` | 1 | — |
| `message send` / `message broadcast` / `member create` | `--file <path>` to an unreadable file | `Error: --file <path>: file is not readable.` | 1 | — |
| `message send` / `message broadcast` / `member create` | `--file <path>` to a file containing invalid UTF-8 | `Error: --file <path>: file is not valid UTF-8.` | 1 | — |
| `member create` | `--coding-agent opencode --model` violating the `<provider-id>/<model-id>` format | `Error: --model for the opencode backend must be '<provider-id>/<model-id>' (got '<value>').` | 2 | Fires before any side effect |
| `member create` | `--effort` with a level unknown to the claude backend | `Error: --effort for the claude backend must be one of low, medium, high, xhigh, max (got '<value>').` | 2 | Fires before any side effect |
| `member create` | `--coding-agent codex --effort` with an unknown level | `Error: --effort for the codex backend must be one of minimal, low, medium, high, xhigh (got '<value>').` | 2 | Fires before any side effect |
| `member create` | `--coding-agent opencode --effort` with any value | `Error: opencode does not support reasoning effort.` | 2 | Fires before any side effect |
| `member create` | An unknown `{placeholder}` in the prompt | `Error: Unknown placeholder '<name>' in custom prompt. Supported placeholders: {fleet_id}, {member_id}, {director_member_id}, {coding_agent}. Double literal braces ({{, }}) to keep them as text.` | 2 | The just-registered member is rolled back |
| `member create` | A malformed brace expression in the prompt | `Error: Malformed custom prompt: <detail>. Double literal braces ({{, }}) to keep them as text.` | 2 | The just-registered member is rolled back |
| `member create` | `--coding-agent` omitted and the spawning Director not found in the fleet | `Error: cannot resolve the member's coding agent: Director <director-id> not found in fleet <fleet-id>. Re-run with an explicit --coding-agent.` | 1 | Nothing spawned |
| `member create` | `--coding-agent` omitted and the spawning Director has no placement row | `Error: cannot resolve the member's coding agent: Director <director-id> has no placement row recording its backend. Re-run with an explicit --coding-agent.` | 1 | Nothing spawned |
| `monitor` | The fleet already has a live monitor | `Error: monitor already running for fleet <id>` | 1 | — |
| `monitor` | An unknown or soft-deleted fleet | `Error: fleet <id> not found` | 1 | — |
