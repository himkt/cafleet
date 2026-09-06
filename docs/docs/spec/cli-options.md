# CLI options

How the unified CAFleet CLI (`cafleet`) accepts configuration parameters. This
page catalogs the arguments, conventions, and error strings.

The marked argument blocks for monitor loop/scan and member capture come
from clap. Maintainers run `mise //cafleet:docs-generate` to update these
and the bounded schema blocks in SPEC, then `mise //cafleet:docs-check` to
check for drift without writing documents. Other contracts remain hand-written.

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
| `doctor` | Print the three-section environment diagnosis (multiplexer, database, coding agents) | — | — | [doctor](#cafleet-doctor) |
| `server` | Start the admin WebUI server | — | — | [server](#cafleet-server) |
| `monitor` | Run the per-fleet scheduler loop in-process as a long-lived execution owned by the monitor member | `FLEET_ID` | — | [monitor](#cafleet-monitor) |
| `monitor scan` | Capture the Director's pane and every active member's pane once | `FLEET_ID` | — | [monitor scan](#cafleet-monitor-scan) |
| `fleet create` | Create a fleet with its root Director and monitor member | — | — | [fleet create](#fleet-create) |
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
| `CAFLEET_MAX_TEXT_LEN` | `max_text_len` | `200` | Text-mode body truncation on `message {send,poll,ack,show}`, and the broker's inline-preview truncation | — |
| `CAFLEET_BROKER_HOST` | `broker_host` | `127.0.0.1` | The `cafleet server` bind address | `--host` |
| `CAFLEET_BROKER_PORT` | `broker_port` | `8000` | The `cafleet server` bind port | `--port` |
| `CAFLEET_MONITOR_WAKE_INTERVAL` | `monitor_wake_interval` | `600` | The `cafleet monitor` wake interval in seconds; `0` disables the wake while the loop keeps heartbeating. A non-integer value fails loudly | `--interval` |

A flag wins over its environment variable, and the environment variable wins
over the hardcoded default.

## Global Options

`--version`, placed **before** the subcommand, prints `cafleet <version>` and
exits 0, short-circuiting before subcommand dispatch.

## Output shapes {#output-shapes}

One row per subcommand. Text output is the human/pane form — message bodies
truncated per [Message Body Truncation](#message-body-truncation); the
machine form is [`--json`](#json-output).

| Subcommand | Text output | JSON payload |
|---|---|---|
| `fleet create` | The compact line `<fleet_id> director=<director_member_id> monitor=<monitor_member_id>` | The fleet dict with nested `director` and `monitor` objects (each including its `placement`) |
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
| `monitor scan` | One `===`-header section per roster entry, separated by one blank line — see [monitor scan](#cafleet-monitor-scan) | A top-level array, one object per entry in the pinned key order |
| `doctor` | The three-section diagnosis report (multiplexer, database, coding agents) plus the issue-count footer | Output as JSON |

`setup`, `server`, and the `monitor` loop form stream progress or run a loop
rather than emitting a one-shot payload, so they carry no output-shape row.
The `message send` row describes its exit-0 shapes; when an attempted pane
notification fails after persistence, the command instead exits 1 through
stderr — see
[Notification outcome and partial failure](#message-send-partial-failure).

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
| `monitor scan` | `monitor` |

All other subcommands reject `--json` with the parser's unknown-argument
error (exit 2) — including the root group itself, so a
pre-subcommand `cafleet --json <grp> <cmd>` does not parse.

## Positional subject ids {#positional-subject-ids}

The id a command acts on rides as a required positional integer immediately
after the subcommand name: `FLEET_ID` on `fleet show` / `fleet delete` /
`member list` / both `monitor` forms, `MEMBER_ID` on `member show` / `delete` / `prompt`
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
  such as [`--json`](#json-output) are covered by the same pattern. Both
  `monitor` forms ride the single `Bash(cafleet monitor *)` pattern —
  `cafleet monitor scan` needs no pattern of its own.
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
There is no untruncated text form — the full body is available via
[`--json`](#json-output).

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
schema-management entry point: the recommended end-user path (see
[Quickstart](../quickstart.md#install)) and the migrations-apply path for
contributors and CI, idempotent and safe to re-run. Command help: `Migrate
the database schema and install the coding-agent assets (skills and
presets).` It takes no
positional arguments — a bare `cafleet setup <word>` fails with the parser's
unexpected-argument error, while a word following the flag (`cafleet setup
--coding-agent claude <word>`) is greedily consumed as another flag value and
fails with the parser's invalid-value error unless it names an agent (both
exit 2).

The one flag is `--coding-agent AGENT...`: optional, multi-value
(space-delimited), and repeatable — `--coding-agent claude codex` and
`--coding-agent claude --coding-agent codex` are valid and equivalent. It is
a choice over `claude` / `codex` / `opencode`, with duplicates deduplicated
and an unknown value failing with the parser's invalid-value error
(exit 2). Its help text is
`Install the named agent's assets (space-delimited, repeatable; default: all agents).`

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
    | Pending migration with duplicate active monitors | Refused; DB rows, schema history, and panes preserved | `active monitor duplicates prevent migration: fleet <id>: members <ids>; ...` |

   The unversioned and newer-schema refusals retain precedence. Before applying
   pending migrations, check existing members using the active-monitor index
   predicate; a new DB without members skips the diagnostic. Conflicting fleet
   ids and member ids are ascending. Pending migrations use one grouped
   transaction, so a failure rolls back every pending schema/history change.
   The index also rejects a duplicate introduced after the diagnostic. If a
   recheck after migration failure finds duplicates, use the same diagnostic;
   otherwise retain the original migration error. The DB half never manipulates
   panes or chooses which monitor survives. Recovery uses the prior compatible
   binary and may require restoring its assets first; see
   [duplicate-monitor recovery](../concepts/storage.md#duplicate-monitor-recovery).

2. **assets half** — installs, from the data embedded in the binary at build
   time with no network access, each selected agent's two skill directories
   plus its bundled preset where one exists (creating the agent's directories
   as needed) at the directories resolved per
   [Config-dir resolution](#config-dir-resolution), and upserts one
   `asset_installs` row per installed agent keyed on `(coding_agent, path)`
   (see [Assets half](#assets-half)). The selection:

    | Invocation | Assets-half behavior |
    |---|---|
    | `--coding-agent` given (one or more values) | Install exactly the named agents, in the fixed order `claude`, `codex`, `opencode`, each at its resolved paths; upsert the `(agent, resolved identity path)` row after that agent's skills and preset (where one exists) install successfully. |
    | No flag | Install all three agents, in the fixed order — identical to `--coding-agent claude codex opencode`. |

Validation applies uniformly wherever the assets half resolves an agent's
identity path — a targeted agent in the selector form, and every agent in
the no-flag form, which resolves all three identity paths: a config-path
validation failure fails the assets half with the pinned
[validation error](#config-dir-resolution) as `<msg>`.

The halves fail independently (`db half failed: <msg>` / `assets half failed:
<msg>`); if any half that ran failed, the command exits 1 with the failed
halves joined by `' and '`.

| Half | Trigger | Message | Effect |
|---|---|---|---|
| db | Any refusal state in the table above | The refusal message, as `db half failed: <msg>` | Exit 1 |
| assets | The `asset_installs` table is missing as the half starts | `the database schema is missing or outdated; run 'cafleet setup' first` | Exit 1 |
| assets | A resolved agent's config-path variable fails validation | `<VAR> must be an absolute path (got '<value>')` | Exit 1 |
| assets | A skills install fails for a target | `failed to install skills into <skills_dir>: <error>` | Aborts the loop; rows recorded before the failure remain |
| assets | A preset install fails for a target | `failed to install preset into <target>: <error>` | Aborts the loop; rows recorded before the failure remain |

The assets-half pre-flight can fire only after a db-half failure or an
externally broken schema — the db half always runs first within the same
command. A DB-half failure does not automatically fail or skip the assets
half: if `asset_installs` remains usable, assets can be updated and recorded
even when duplicate monitors prevent the schema upgrade.

### Config-dir resolution {#config-dir-resolution}

Every surface that installs to or checks a backend's user-level config
directory resolves it through the backend's native config-location
environment variable, falling back to the default when the variable is unset:

| Backend | Variable | Base when set | Base when unset | Skills dir | Preset target |
|---|---|---|---|---|---|
| claude | `CLAUDE_CONFIG_DIR` | `$CLAUDE_CONFIG_DIR` | `~/.claude` | `<base>/skills` | — |
| codex | `CODEX_HOME` | `$CODEX_HOME` | `~/.codex` | `<base>/skills` | `<base>/rules/cafleet.rules` |
| opencode (skills) | — (fixed discovery path) | — | `~/.config/opencode` | `<base>/skills` | — |
| opencode (preset) | `OPENCODE_CONFIG_DIR` | `$OPENCODE_CONFIG_DIR` | `~/.opencode` | — | `<base>/agents/cafleet.md` |

opencode splits by purpose: `agents/` is in `OPENCODE_CONFIG_DIR`'s
documented search list, so the preset may relocate and remain a valid
`--agent cafleet` discovery path; skills are not in that list — opencode
discovers them only at fixed paths — so the skills install ignores the
variable.

**Validation.** A set variable must hold an absolute path. Any other value —
the empty string, a relative path, a literal unexpanded `~/…` — fails at
resolution time with exit 1:

```
Error: <VAR> must be an absolute path (got '<value>')
```

Validation is lazy: a variable is read and validated only when a site
actually resolves that backend's directory. `cafleet setup --coding-agent
claude` with an invalid `CODEX_HOME` succeeds because the selector resolves
only the targeted agent's directory; plain `cafleet setup` resolves all
three identity paths, so an invalid variable fails its assets half. The
spawn preconditions themselves read
none of the three variables for claude and codex (PATH-check-only;
opencode's resolves the preset base) — but the
[stale-assets guard](#stale-assets-guard) fronting every fleet-scoped
command, `member create` included, resolves all three identity paths. One
exception to strict lazy failure: `doctor` catches per-agent resolution
errors and renders them as issues instead of aborting (see
[doctor](#cafleet-doctor)).

**Recorded-path identity.** Every surface that keys on "the agent's resolved
path" — the `asset_installs` rows, the
[stale-assets guard](#stale-assets-guard), and `doctor`'s setup column — uses
one canonical path per agent, the resolved base directory, stored absolute
exactly as resolved (no canonicalization beyond the absolute-path
validation):

| Agent | Recorded / status-keyed path |
|---|---|
| claude | The resolved claude config dir (`$CLAUDE_CONFIG_DIR` or `~/.claude`) |
| codex | The resolved codex home (`$CODEX_HOME` or `~/.codex`) |
| opencode | The resolved preset base (`$OPENCODE_CONFIG_DIR` or `~/.opencode`) — the only opencode root that can vary; the skills base is fixed and carries no identity |

### Assets half {#assets-half}

Each agent's preset, where one exists, is a static file embedded in the
binary next to the skills; install targets resolve per
[Config-dir resolution](#config-dir-resolution):

| Agent | Bundled preset | Install target |
|---|---|---|
| claude | — (skills only) | — |
| codex | `presets/codex/cafleet.rules` | `<codex base>/rules/cafleet.rules` (default `~/.codex/rules/cafleet.rules`) |
| opencode | `presets/opencode/cafleet.md` | `<preset base>/agents/cafleet.md` (default `~/.opencode/agents/cafleet.md`) |

Each backend installs the two embedded skills, its preset where present, and
removes a leftover `cafleet-research` entry as one recoverable plan. Unrelated
skills remain untouched. A symlink target is moved or unlinked as an entry;
its referent is not deleted. Skills-operation errors retain `failed to install
skills into <skills_dir>: <error>` and preset-operation errors retain `failed
to install preset into <target>: <error>`.

After durable success, the command prints the existing lines (the preset line
appears only for codex and opencode):

```
<agent>: installed cafleet, cafleet-design-doc (v<version>) -> <skills dir>
<agent>: installed preset (v<version>) -> <target>
```

### Assets install recovery {#assets-install-recovery}

Setup first writes and validates all replacements in hidden staging entries
beside their targets. Each backup also lives beside its target, so a preset
on another filesystem does not require a cross-filesystem rename. Physical
target locks serialize concurrent setup calls, including configured paths
that alias the same skills tree. These are OS advisory locks released on
process exit, not PID-file checks; recorded identity paths remain unchanged.

Persistent lock sidecars point to the owning journal, so another configured
identity sharing a physical skills tree can find interrupted work. Keep these
files: Finished sidecars without a journal are normal. Recovery checks the
recorded physical database path against the current connection and asks for
the original database configuration on a mismatch; it never opens another DB
automatically. During reinstall, an old Finished sidecar may temporarily refer
to a new untouched Prepared journal. Setup verifies the exact old row and all
old entries, then updates every sidecar to the current Active transaction
before beginning rollback. Other transaction or target mismatches stop and
retain evidence. This normalization is itself restartable.

Before exchanging entries, setup writes a durable
`<identity>/.cafleet-install-journal.json` with transaction/phase, paths, old
entry presence, the previous database row or null, and the new manifest.
Updates use temporary-file write, file sync, rename and directory sync before
destructive changes. Old entries move to backups; stages move to targets.
The obsolete research skill moves to a backup without a replacement.
Only after all exchanges does setup commit the new `asset_installs` row and
mark the journal committed. It then removes backups/stages and the journal last.

| Failure or interruption | Result and next setup |
|---|---|
| Stage write or validation | Current files and row remain unchanged; unused stages can be cleaned. |
| Before durable journal commit, even after SQLite commit | Restore old entries and the exact previous row, including its timestamp or absence. Retry interrupted recovery from the journal. |
| After durable journal commit | Installation succeeded. A cleanup failure is reported separately; retain the journal and next time verify the new manifest/row and resume cleanup only. |
| Corrupt journal or failed restoration | Preserve journal and remaining backups, report incomplete installation, and stop. |

Do not delete journals or backups to clear an error. Guard and doctor report
`incomplete assets install at <path>; run 'cafleet setup' to recover` while
recovery evidence remains unfinished, including committed cleanup pending.
Version equality alone does not establish a healthy install. Doctor includes
this as an assets issue (`state: "incomplete"` in JSON) even when no asset row
can be read, and continues the other diagnostic sections.

An install failure aborts the backend loop; successful earlier backends remain
installed and recorded. A committed cleanup warning does not make the assets
half fail. The DB and assets halves retain their independent failure handling.
Its stderr line is `warning: assets installed at <path>; cleanup pending:
<cause>; run 'cafleet setup' to recover`. A later cleanup-only run prints the
journal's version, not the version of a newer binary running the recovery.
The procedure recovers coordinated changes across filesystems and SQLite; it
is not one atomic commit. Power-loss guarantees depend on filesystem support
for sync and rename. The optional developer reference [installer test boundaries](https://github.com/himkt/cafleet/blob/main/docs/docs/contributing.md)
explains what isolated fixtures establish; it is not required for offline setup or recovery.

## Shared diagnosis and connection reuse {#diagnosis-reuse}

Typed diagnosis and an invocation's SQLite connection are shared between
guards and command work. The output and failure rules below remain unchanged.

| Internal schema state | Meaning |
|---|---|
| `Missing` | No recorded schema version (ledger absent or empty) and no application tables, including a newly opened empty file. |
| `Unversioned` | Application tables exist without a recorded schema version (ledger absent or empty). |
| `Behind` / `Head` / `Ahead` | Recorded schema version is below / equal to / above embedded head. |
| `Unreachable` | Opening or inspecting the database failed; retain the original cause. |

Schema and per-agent/path `AssetState` facts carry the diagnostic state. Assets retain their path source, current matching/stale or
missing install, path-resolution error, incomplete recovery, and informational
superseded records. Incomplete recovery uses the existing JSON keys with
`state: "incomplete"` and its recovery diagnostic in `error`.
The guard and doctor keep their respective text, JSON, issue counts, and exit
codes; internal state names do not become new wire fields.

Ordinary commands must apply schema → assets → command-body order on the
same connection. Asset path validation must not run ahead of a failing schema
guard. Doctor still reports multiplexer, database, and coding agents even
after connection/schema failure; recorded assets are read only at head with
the table present, as specified in its [database section](#database-section).

Setup reuses a successfully opened connection and diagnoses again after DB
creation/migration. DB failure still proceeds to the independent assets half;
an older database with `asset_installs` can accept assets, while a missing
table retains the preflight error. If the first connection attempt failed,
the assets half may try again. Targeted setup still resolves only the selected
agents. Keep the existing refusal messages and combined half-failure result.
HTTP connections remain scoped to individual blocking handlers.

Fleet creation retains access to the connection's owner so a broker failure
closes the actual database handle before CLI pane compensation. A failed
explicit rollback does not prevent that close. Success keeps the same
connection; cleanup does not reopen a closed connection just for reporting.
See [creation failure compensation](#creation-failure-compensation).

## Schema-version guard {#schema-version-guard}

Every non-setup command — the `fleet`, `member`, and `message` groups, the
`monitor` command (both forms), and `server` — classifies the database
schema against the head of the migration chain embedded in the binary before
its command body runs, and before the
[stale-assets guard](#stale-assets-guard). `setup` (it must remain runnable
to repair the database) and `doctor` (it reports instead of blocking) are
exempt. `<M>` is the recorded version, `<N>` the embedded head:

| Database state | Result | Exit |
|---|---|---|
| Recorded version == head | The command proceeds silently | 0 |
| Recorded version < head | `Error: database schema is outdated (schema <M>, head <N>); run 'cafleet setup'` | 1 |
| No ledger, no app tables (missing or empty DB file) | `Error: no cafleet database; run 'cafleet setup'` | 1 |
| No ledger, app tables present | `Error: database has tables but no schema history — not a cafleet database?` | 1 |
| Recorded version > head | `Error: database schema <M> is newer than this cafleet (head <N>); upgrade cafleet` | 1 |

Opening the database creates an empty DB file when one is missing, so the
guard detects "missing" post-hoc as the no-ledger/no-tables state — the same
classification `doctor`'s database section reports as `no database`.
Connection-level failures (an unreadable file, a bad URL scheme) keep their
existing `failed to open database at '<path>': <e>` / scheme errors — those
are environment errors, not schema states. With this guard in front, the
stale-assets guard runs only against an at-head schema, so no missing or
outdated schema state can surface a raw SQLite error from a guarded command.

## Stale-assets guard {#stale-assets-guard}

Every fleet-scoped surface — the `fleet`, `member`, and `message` groups plus
the `monitor` command — validates the recorded assets installs after the
[schema-version guard](#schema-version-guard) passes and before any
subcommand body runs. The guard resolves each agent's identity path per
[Config-dir resolution](#config-dir-resolution) and checks only the row at
that path plus recovery evidence. Apply the following precedence:

| Recorded install state | Result | Exit |
|---|---|---|
| A config-path variable fails validation | The pinned [validation error](#config-dir-resolution) | 1 |
| Recovery evidence remains unfinished, even with a matching version or absent row | `Error: incomplete assets install at <path>; run 'cafleet setup' to recover` | 1 |
| No agent has a row at its currently-resolved path (zero rows, or — on a hand-tampered at-head database — a dropped `asset_installs` table) | `Error: no assets install is recorded at the resolved paths; run 'cafleet setup' to install` | 1 |
| A row at a resolved path has `cafleet_version` ≠ the runtime CLI version (string inequality — either direction) | `Error: stale assets detected (<agent>=<recorded>[, ...]; CLI <runtime>); run 'cafleet setup' to reinstall`, stale agents in ascending order | 1 |
| Every row at a resolved path matches | The command proceeds silently | 0 |

After recovery checks, agents with no row at their currently-resolved path
are not checked for staleness (they
contribute nothing to staleness), and superseded rows at other paths are
ignored everywhere in the guard. Plain `cafleet setup` remains the correct
remedy for these errors: it recovers or installs agents at their resolved
paths. Three surfaces are exempt:

| Exempt surface | Why exempt | Behavior under a stale/missing install |
|---|---|---|
| `setup` | It must remain runnable to repair the install | Runs normally — it is the repair path |
| `doctor` | It reports instead of blocking | Renders each agent's setup state in the coding-agents section |
| `server` | It serves the WebUI rather than running a fleet-scoped command | Starts normally (the [schema-version guard](#schema-version-guard) still covers it) |

`--help` renders at parse time and exits before any command body runs, so
neither group-level help (`cafleet fleet --help`) nor subcommand help
(`cafleet fleet create --help`) triggers either guard — both always print
help, even under a missing database or a missing or stale install.

## `cafleet fleet` — Fleet Management

Fleet lifecycle; writes directly to SQLite — no server required.

### `fleet create`

| Flag | Required | Notes |
|---|---|---|
| `--name` | yes | Human-readable name for the fleet |
| `--coding-agent` | yes | One of `claude`, `codex`, or `opencode`, recorded as the root Director's placement `coding_agent` — the operator declares the backend the Director is actually running on; see [Coding agents](../concepts/coding-agents.md). The monitor member inherits this backend by construction — there is no `--monitor-coding-agent`. |
| `--monitor-file PATH` | yes | UTF-8 file whose contents are the monitor member's spawn prompt (`-` = stdin). Same body semantics as `member create --file` (empty / non-UTF-8 rejection); there is no inline positional form. |
| `--monitor-model MODEL` | no | Model forwarded to the monitor's backend binary, validated by the `--coding-agent` backend exactly as `member create --model`. When omitted the backend spawns on its own default model. |
| `--json` | no | Output as JSON |

Omitting a required flag exits 2 with the parser's native
missing-required-argument error naming the flag.

**Must be run inside a tmux or herdr session** — outside one it exits 1 with
`Error: cafleet fleet create must be run inside a tmux or herdr session` and
writes nothing. One invocation creates the fleet, its root Director, and its
monitor member in a single DB transaction, with owned-pane compensation
for a failed bootstrap. The
Director and monitor identities are hardcoded (Director:
`name="Director"`, `description="Root Director for this fleet"`; monitor:
`name="monitor"`, `description="Monitor member for this fleet"`); there are
no name / description / effort flags for either. Output shapes are in
[Output shapes](#output-shapes).

The command runs a single-transaction ladder — see
[data-model.md](./data-model.md) for the transaction description:

1. Multiplexer preconditions, before any write.
2. Resolve the monitor prompt body from `--monitor-file` (file, or stdin
   via `-`).
3. Backend checks before any write: backend lookup,
   `--monitor-model` validation, and the binary-on-`PATH` availability
   check (`Error: binary <name> not found on PATH`, exit 1).
4. In one SQLite transaction: insert the fleet row, the Director member +
   placement, backfill `fleets.director_member_id`, and insert the monitor
   member row with the monitor card marker.
5. Substitute the four identity placeholders into the prompt body
   (same substitution and error strings as
   [`member create`](#member-create)).
6. Spawn the monitor pane (detached split, `CAFLEET_DATABASE_URL` as the
   only forwarded environment variable). On successful `split_window`,
   immediately transfer pane ownership to the CLI guard and return the id
   from the callback, without intervening fallible work.
7. Insert the monitor placement row (same session/window context as the
   Director), then commit. Disarm all creation guards before calling the
   existing text/JSON output path.

A bootstrap failure attempts rollback of the whole DB transaction. For a
Herdr run failure inside the callback, the backend tries to kill the known
pane before the callback returns an error and the broker rolls back. For
placement insert or commit failure after a successful callback, the broker
closes its transaction first; then the CLI kills its owned pane. Successful
rollback leaves no fleet, Director, monitor, or placement rows. A failed
cleanup is reported with the primary error; the command does not claim a
complete rollback or unconditional retryability when cleanup is uncertain.
The [creation failure contract](#creation-failure-compensation) specifies all
orders and diagnostics. Error strings are in [Error Messages](#error-messages).

Once its pane boots, the monitor member sends `ready`, launches the
`cafleet monitor` wake loop, and sends `monitor live` (see
[Monitoring](../concepts/monitoring.md)).
`member create --role monitor` remains the mid-run recovery path for
re-spawning a dead monitor.

### `fleet list`

The only flag is the optional shared [`--json`](#json-output).

Lists all non-soft-deleted fleets in `created_at DESC, fleet_id DESC` order
(higher id first when timestamps tie). Each row exposes `director_member_id` so
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

## `cafleet doctor` — Environment Diagnosis {#cafleet-doctor}

A full-environment diagnosis that renders **all** sections even when the
multiplexer is unavailable or the database is missing or stale — no early
abort. Diagnosis order: multiplexer, database, coding agents. `doctor` is
exempt from the [schema-version guard](#schema-version-guard) and the
[stale-assets guard](#stale-assets-guard) — a missing or outdated database
and a stale or missing install are reported, not fatal.

The only flag is the optional [`--json`](#json-output), a trailing
per-subcommand flag.

### Text layout

The first output line of the whole report is `cafleet <version>`. Each
section is led by a single-width verdict glyph (`✓` U+2713 / `✗` U+2717)
plus the section name; detail lines are indented two spaces beneath. A
worked example (at-head schema, one stale agent, one superseded record):

```
cafleet 0.22.0
✓ multiplexer
  backend:   tmux
  session:   main
  window_id: @3
  pane_id:   %0
  presence:  TMUX=/tmp/tmux-501/default,12345,0
✓ database
  schema 12 (head)
✗ coding agents
  ┌──────────────┬──────────────┬────────────────────┬───────────────────────────────────────────────┐
  │ coding agent │ path         │ source             │ setup                                         │
  ├──────────────┼──────────────┼────────────────────┼───────────────────────────────────────────────┤
  │ claude       │ ~/cfg/claude │ $CLAUDE_CONFIG_DIR │ ✓ 0.22.0                                      │
  │ codex        │ ~/.codex     │ default            │ ✗ 0.21.0 → cafleet setup --coding-agent codex │
  │ opencode     │ ~/.opencode  │ default            │ – cafleet setup --coding-agent opencode       │
  └──────────────┴──────────────┴────────────────────┴───────────────────────────────────────────────┘
  note: codex was previously set up at ~/.codex-old
1 issue found
```

### Multiplexer section

`✓` with the five detail lines (`backend`, `session`, `window_id`,
`pane_id`, `presence`). On any multiplexer or environment failure (no
supported multiplexer, ambiguous environment, binary not on `PATH`, pane not
discoverable): `✗ multiplexer` with the resolver's error message as the
single detail line, and the report continues. One issue.

### Database section

One detail line; the five states (`<M>` recorded version, `<N>` embedded
head):

| State | Glyph | Detail line | Issue |
|---|---|---|---|
| Ledger present, `<M>` = `<N>` | `✓` | `schema <N> (head)` | no |
| Ledger present, `<M>` < `<N>` | `✗` | `schema <M>, head is <N> — run: cafleet setup` | yes |
| Ledger present, `<M>` > `<N>` | `✗` | `schema <M> is newer than this CLI (head <N>) — upgrade cafleet` | yes |
| Ledger absent, foreign tables present | `✗` | `database has tables but no schema history — not a cafleet database?` | yes |
| Ledger absent, no tables (or no DB file) | `✗` | `no database — run: cafleet setup` | yes |

A connection failure (unreadable path) renders `✗` with the connection error
as the detail line (one issue). A `✗` database never suppresses the
coding-agents section, but the recorded rows are read only when the database
report is `✓` (at head) AND the `asset_installs` table exists. Whenever the
rows are not read — any non-head state, or an at-head ledger with a
hand-dropped table — the section renders with no recorded-install data:
every resolvable agent without incomplete recovery shows the `–` state;
incomplete recovery still shows an assets issue. No superseded footnotes render,
and — in the non-head states — doctor exits 1 for the database issue;
either way, never a raw SQLite error from `asset_installs`.

### Coding agents section

A light box-drawing framed table (`┌ ─ ┬ ┐ │ ├ ┼ ┤ └ ┴ ┘`), header separator
only, no per-row rules. Column alignment uses **display width**, never byte
length. One row per agent in the fixed order `claude`, `codex`, `opencode`.

| Column | Content |
|---|---|
| `coding agent` | The agent name. |
| `path` | The resolved identity path ([Config-dir resolution](#config-dir-resolution)), with `~` abbreviation when under `$HOME`. On a resolution error: the raw invalid variable value. |
| `source` | The winning origin: `$<VAR>` (e.g. `$CLAUDE_CONFIG_DIR`) or `default`. |
| `setup` | The setup state below, keyed on the **resolved path only**. |

| State | Cell | Issue |
|---|---|---|
| Recovery evidence remains unfinished at the resolved install (takes precedence over the row/version states below) | `✗ incomplete assets install at <path>; run 'cafleet setup' to recover` | yes |
| A row exists at the resolved path, version = CLI version (string equality) | `✓ <version>` | no |
| A row exists at the resolved path, version ≠ CLI version (string inequality, either direction — never semver comparison) | `✗ <recorded-version> → cafleet setup --coding-agent <agent>` | yes |
| No row exists at the resolved path (regardless of rows elsewhere) | `– cafleet setup --coding-agent <agent>` (`–` U+2013 EN DASH) | never |
| The agent's config-path variable fails validation (caught per-agent, not fatal) | `✗ <VAR> is not an absolute path` | yes |

Records at other paths only feed informational footnote lines under the
table, one per superseded row, ordered ascending `(coding_agent, path)`,
`~`-abbreviated:

```
note: <agent> was previously set up at <path>
```

Footnotes are informational — they never count as issues.

### Footer and exit code

Last line: `no issues found`, `1 issue found`, or `<N> issues found` (proper
pluralization). Exit code: 0 when no issues, 1 otherwise — the `–` state and
footnotes never count. Every failure is a rendered issue; no failure exits
before output.

### `--json`

Mirrors the sections with `ok` booleans, unabbreviated absolute paths, and
the issue count. `source` holds the winning env-var **name** (no `$`) or the
literal `"default"`; `state` is `"ok" | "stale" | "not_installed" | "error" | "incomplete"`
(`"error"` is the per-agent resolution-error state; `"not_installed"` never
contributes to `issues`). Every agent row carries the same keys. `error` is
the validation message for `"error"`, the recovery diagnostic for
`"incomplete"` (one issue), and otherwise null, without an `Error: ` prefix. Section `error` fields likewise hold
the detail text when `ok` is false, else `null`.

```json
{
  "multiplexer": {
    "ok": true,
    "backend": "tmux",
    "session": "main",
    "window_id": "@3",
    "pane_id": "%0",
    "presence_var": "TMUX",
    "presence_value": "/tmp/tmux-501/default,12345,0",
    "error": null
  },
  "database": {
    "ok": true,
    "schema_version": 12,
    "head_version": 12,
    "error": null
  },
  "coding_agents": {
    "ok": false,
    "cli_version": "0.22.0",
    "agents": [
      {"coding_agent": "claude", "path": "/Users/x/cfg/claude", "source": "CLAUDE_CONFIG_DIR", "recorded_version": "0.22.0", "installed_at": "2026-08-12T00:00:00.000000+00:00", "state": "ok", "error": null},
      {"coding_agent": "codex", "path": "/Users/x/.codex", "source": "default", "recorded_version": "0.21.0", "installed_at": "2026-08-01T00:00:00.000000+00:00", "state": "stale", "error": null},
      {"coding_agent": "opencode", "path": "/Users/x/.opencode", "source": "default", "recorded_version": null, "installed_at": null, "state": "not_installed", "error": null}
    ],
    "superseded": [
      {"coding_agent": "codex", "path": "/Users/x/.codex-old", "recorded_version": "0.20.0", "installed_at": "2026-07-01T00:00:00.000000+00:00"}
    ]
  },
  "issues": 1
}
```

On a multiplexer failure the `multiplexer` object is `{"ok": false,
"backend": null, "session": null, "window_id": null, "pane_id": null,
"presence_var": null, "presence_value": null, "error": "<message>"}`. On an
agent resolution error the row is `{"coding_agent": "...", "path": null,
"source": "<VAR>", "recorded_version": null, "installed_at": null, "state":
"error", "error": "<VAR> must be an absolute path (got '<value>')"}` — the
raw invalid value appears only inside `error`; `path` stays `null` because
no path resolved. `schema_version` is `null` when the ledger is absent.
When the recorded rows are not read (a non-`ok` database report, or a
missing `asset_installs` table), resolved agents use `"not_installed"`
unless recovery evidence requires `"incomplete"`; path errors remain
`"error"`. Recorded fields are null and `superseded` is empty. Exit-code semantics are identical to text mode.

## `cafleet server` — Admin WebUI Server {#cafleet-server}

Starts the admin WebUI app under the built-in HTTP server (single process, no
auto-reload, no worker or log-level flags). CLI commands do not require this
server.

| Flag | Default | Notes |
|---|---|---|
| `--host` | `settings.broker_host` (default `127.0.0.1`) | Bind address. Overrides `CAFLEET_BROKER_HOST` when both are set. |
| `--port` | `settings.broker_port` (default `8000`) | Bind port. Overrides `CAFLEET_BROKER_PORT` when both are set. |

Flag wins over env var; env var wins over the hardcoded default. The
[schema-version guard](#schema-version-guard) runs before the server starts,
so a missing or outdated database fails with its `cafleet setup` guidance
instead of a runtime SQLite error on the first request. The WebUI
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

#### Notification outcome and partial failure {#message-send-partial-failure}

`message send` persists the message row first, then attempts one inline-preview
pane notification — unless the send is a self-send or the recipient's placement
has no pane id, which are intentional skips (see
[Push notifications](multiplexer-backends.md#push-notifications)). The
notification is attempted at most once; no layer retries it. The persisted row
is never deleted, rolled back, or duplicated on a notification failure — it
stays `input_required` and recoverable through the normal `poll`/`ack` path.

| Mode | Exit | stdout | stderr |
|---|---|---|---|
| Text, attempted notification succeeds | 0 | `Message sent.` plus the compact rendered envelope | Empty |
| `--json`, attempted notification succeeds | 0 | The untruncated `{"message": …, "notification_sent": true}` JSON | Empty |
| Text, self-send or no-pane skip | 0 | The success output, with no warning added | Empty |
| `--json`, self-send or no-pane skip | 0 | The success JSON with `notification_sent: false` | Empty |
| Text, attempted notification fails | 1 | Empty | `Error: ` plus the exact partial-failure message below |
| `--json`, attempted notification fails | 1 | Empty | The same text error as non-JSON mode |

An attempted notification failure exits 1 with:

```text
Error: Message <message-id> was persisted, but pane notification failed: <raw backend error>. Do not resend this message. Recover the recipient pane, then run 'cafleet member ping <recipient-id>' or have the recipient run 'cafleet message poll <recipient-id>'.
```

`<raw backend error>` is the multiplexer's error detail inserted verbatim; it
may contain the backend command, its payload argv, and a newline-delimited
stderr detail (see
[Multiplexer backends](multiplexer-backends.md#inline-preview-errors)). The
formatter adds no separate copy of the sent message body. The `--json` failure
behavior follows the existing global error contract: `--json` selects
successful command output only and never produces a JSON error envelope.

The recovery contract is no-resend:

1. Treat `<message-id>` as authoritative proof that persistence succeeded.
2. Repair or re-engage the recipient pane.
3. Run `cafleet member ping <recipient-id>` as its own shell-tool invocation,
   or have the recipient run `cafleet message poll <recipient-id>` as its own
   shell-tool invocation.
4. Consume and ACK the existing row normally. Do not issue a second
   `message send` for the same content.

### `message broadcast`

`cafleet message broadcast --from-member-id ID (TEXT | --file PATH) [--json]`

| Argument | Required | Notes |
|---|---|---|
| `--from-member-id` | yes | Broadcaster (sender). The fleet is derived from the sender row. |
| positional `TEXT` / `--file PATH` | one of | Message body, as on `message send`. |

`delivered=<k>` counts the inline previews that landed. A failed preview only
lowers `delivered` — broadcast keeps its single summary envelope, its
`recipients`/`delivered` counts, and exit 0, with no per-recipient failure
detail.

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
| `--coding-agent` | no | One of `claude`, `codex`, or `opencode`; when omitted, the member — every role — inherits the spawning Director's placement backend. Exits 1 with `Error: binary <name> not found on PATH` when the binary is missing. The opencode backend additionally requires its agent preset at the resolved preset path (via `OPENCODE_CONFIG_DIR` — see [Config-dir resolution](#config-dir-resolution)); a missing preset or an invalid variable exits 1 per [Error Messages](#error-messages). |
| `--model` | no | Model forwarded to the backend binary's own `--model` flag. The opencode backend additionally requires `<provider-id>/<model-id>`; per-backend formats and create-time validation are in [Model selection](coding-agent-backends.md#model-selection). |
| `--effort` | no | Reasoning-effort level forwarded to the backend binary, validated per backend before any side effect. Accepted levels, forwarding forms, and rejection strings are in [Reasoning effort](coding-agent-backends.md#reasoning-effort). |
| `--role` | no | The sole accepted value is `monitor` — registers the member as the fleet's monitor member (see [Monitoring](../concepts/monitoring.md)); any other value is the parser's invalid-value error (exit 2). The bootstrap monitor is spawned by [`fleet create`](#fleet-create); this flag is the mid-run recovery path for re-spawning a dead monitor. A fleet holds at most one active monitor member, and an ordinary `member create` requires one — both guards are in [Error Messages](#error-messages). |
| positional `PROMPT` | one of | Inline spawn prompt (backend-neutral template). Exactly one of `PROMPT` / `--file`. |
| `--file PATH` | one of | Path to a UTF-8 file whose contents are the spawn prompt (`-` = stdin). Inline prompts beyond a few KB exceed the multiplexer argv ceiling — use `--file` for long prompts. |
| `--json` | no | Output as JSON |

#### Concurrent monitor registration

The CLI's one-per-fleet check remains before the monitor-first check and before
registration or pane creation, preserving the existing validation order.
Registration also opens an `IMMEDIATE` DB transaction and checks the active
monitor slot inside it. The partial unique index enforces the same predicate
for all writers, including direct broker calls and status/card/fleet updates.
Only a conflict with that constraint maps to the typed
`ActiveMonitorExists { fleet_id, member_id }` error; the CLI renders the
existing `Error: fleet <fleet-id> already has an active monitor member
(member <member-id>)`, exit 1, without the `register failed:` prefix.
Other SQL failures keep their existing classification. The losing registration
adds no member or placement and never creates a pane. Ordinary broker
registration retains its existing behavior; requiring an active monitor
before creating an ordinary pane remains the CLI's monitor-first policy.

#### Spawn command per backend

The per-backend spawn argv and auto-approval flags live in
[Coding-agent backends](coding-agent-backends.md#spawn-argv).

#### Spawn-prompt substitution

`cafleet member create` uses the Rust spawn-placeholder mini-formatter on the resolved prompt body,
substituting exactly four placeholders:

| Placeholder | Substituted value | How the spawned member sees it |
|---|---|---|
| `{fleet_id}` | The member's fleet id | `FLEET ID: <fleet_id>` |
| `{member_id}` | The member's own newly-allocated id | `YOUR MEMBER ID: <member_id>` |
| `{director_member_id}` | The fleet's root Director id | `DIRECTOR MEMBER ID: <director_member_id>` |
| `{coding_agent}` | The resolved backend name (`claude`, `codex`, or `opencode`) | `CODING AGENT: <coding_agent>` |

Identity reaches the spawned member as literals rendered into its prompt; the
only environment variable forwarded into the pane is `CAFLEET_DATABASE_URL`.
The formatter accepts only the four exact names above and doubled literal
braces (`{{` / `}}`), not Python format specifications, conversions, or
attribute/index access. An unknown placeholder or
malformed brace expression exits 2 and attempts to deregister the just-registered
member; any cleanup failure is reported after the primary error
(see [Error Messages](#error-messages)).

The spawn always creates the pane without stealing focus (tmux
`split-window -d`): the Director's pane and active window stay active. In the
default output, `pane` renders `(pending)` until the pane id is patched onto
the placement.

#### Shared spawn preparation and deadlines {#creation-deadlines}

Shared lifecycle preparation groups the existing arguments into
`MemberCreateOptions` and `FleetCreateOptions`; flags, defaults, output, and
validation precedence stay unchanged. Fleet and member preconditions remain
separate. Shared preparation resolves the prompt, validates the backend, and
prepares cwd, the forwarded environment, and the identity-independent argv
before the fleet transaction (or member registration). Only identity-dependent
prompt/argv rendering and pane spawn remain after ids are allocated.

Fleet creation still commits the fleet, Director, monitor, and placement in
one transaction. Its spawn callback gets one **30-second monotonic deadline**;
each Herdr list/split/run/layout/resize or tmux split/layout subprocess uses
the time remaining on that deadline, never a fresh 30 seconds. The shared
member spawn path uses the same budget. Known-pane compensation has a separate
**5-second budget**, so timeout recovery can run after the spawn budget is
exhausted. See [backend deadlines](multiplexer-backends.md#spawn-deadlines).

The existing SQLite busy timeout remains 5 seconds. The spawn deadline does
not promise that DB lock waits, rollback, process creation, or OS termination
will finish within 30 seconds. Preserve the
[compensation order](#creation-failure-compensation), including closing the
fleet invocation's actual DB handle before CLI pane cleanup after broker
failure. An id-known Herdr run timeout triggers backend kill before rollback;
a split timeout without an id reports unknown cleanup and never guesses a
pane. Keep the primary timeout and append any cleanup errors.

#### Creation failure compensation {#creation-failure-compensation}

Member creation registers a pending placement, renders the prompt, creates a
pane, and patches its id. The CLI owns a registration guard after successful
registration and acquires a pane guard only when `split_window` succeeds.
The backend owns the pane before that return; see
[pane ownership](multiplexer-backends.md#pane-creation-ownership).

| Failure point | Compensation order | DB result when compensation succeeds |
|---|---|---|
| Member registration | Broker rolls back registration; no pane guard exists | No new member or placement |
| Member placeholder expansion | CLI deregisters; no pane was created | Member becomes deregistered and placement is removed |
| Member Herdr run after split returned an id | Backend tries pane kill, returns the error, then CLI deregisters | Member deregistered and placement removed |
| Member placement update error or missing row | After handling the failed SQL, CLI kills pane, then deregisters | Member deregistered and placement removed |
| Fleet callback before pane creation | Broker rolls back bootstrap; no pane guard exists | No added fleet, Director, monitor, or placement |
| Fleet callback Herdr run failure/timeout with known id | Backend tries pane kill, callback returns error, then broker rolls back bootstrap | No added bootstrap rows |
| Fleet placement insert/commit after callback success | CLI holds the transferred guard; broker rolls back and closes its transaction, then CLI kills pane | No added bootstrap rows |
| Fleet callback fails/times out before obtaining an id | Backend reports unknown pane compensation, then broker rolls back; no guessed pane kill | No added bootstrap rows; pane state unconfirmed |

Continue remaining compensation even if an earlier cleanup fails. A backend
`PaneCleanup::Attempted` result is never killed again by the CLI. A failed
split with no confirmed id reports that the pane id is unknown and cleanup
is unconfirmed, including for member creation; it does not infer which other
pane should be closed. Creation rollback uses `kill_pane(id, true)` rather
than `send_exit`.

Keep the primary cause and its exit category. Member split failures retain
the primary reason `tmux split-window failed: <error>`; placement errors retain
`placement update failed: <error>` or `placement row vanished before pane-id
patch`. Placeholder failures retain their usage error and exit 2. Append
applicable cleanup diagnostics after the primary error, on stderr:

- `cleanup failed for pane <id>: <detail>`
- `cleanup failed for member <id>: <detail>`
- `cleanup failed for fleet <id> transaction: <detail>`

Report transaction rollback failures explicitly. Existing rollback-success
suffixes may describe confirmed compensation; they must not claim success or
complete rollback when cleanup failed or pane compensation is unconfirmed.
Guards disarm through explicit `finish`/`rollback`, preventing repeated kill
or deregistration; `Drop` handles only remaining unhandled ownership. Once a
member placement is confirmed or fleet commit succeeds, disarm all creation
guards before the existing `emit` call. Successful text/JSON output keeps its
shape, ordering, and nulls. No new stdout-failure exit or diagnostic contract
is introduced. Normal `member delete` behavior and persisted-message
notification failures remain unchanged.

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

Rows are ordered by `member_id ASC`. `last_sent` is the maximum creation time
of every message sent by the member, including broadcast summaries;
`last_recv` is the maximum creation time of owned unicast deliveries;
`last_ack` is the maximum status timestamp of owned completed unicast
deliveries. `idle` uses the greatest non-null string among all three, parsed
with the existing lenient reader against one `now` for the list. All null or
an unparseable selected value yields null; no older timestamp fallback is
used. Text remains humanized as `Ns` / `Nm` / `Nh`.
A zero clamp applies to the final whole-second idle result; it does not
change stored future timestamps or parsing. See the [activity contract](data-model.md#query-and-activity-contracts).
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

The `--shell` flag controls only the payload prefix; both forms use the same
Esc safeguard and failure semantics:

| Form | Keystroke sequence | Follow-up |
|---|---|---|
| Plain (no `--shell`) | `Esc` → settle → literal `TEXT` → `Enter` | None |
| `--shell` | `Esc` → settle → literal `! TEXT` → `Enter` | `cafleet member ping` required |

In both forms the leading `Esc` (as in `member ping` and inline previews) keeps
the dispatch from blindly confirming a pending permission prompt. In the plain
form the trailing `Enter` submits a real user turn and opens the member's turn
directly. The `--shell` form's bang output only stages in the pane — the ping
advances the member's turn to consume it.

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
notification, owned by the Director and the monitor member (whose fixed-ping
exception is the one automatic use — see
[Monitoring](../concepts/monitoring.md)); the action is wholly fixed by the
command — no operator-controlled body — which is why `member ping` sits in
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

<!-- BEGIN GENERATED cli-member-capture -->
| Argument | Type | Values per occurrence | Action | Parser default | Required |
|---|---|---|---|---|---|
| `MEMBER_ID` | `i64` | 1 | `Set` | — | yes |
| `--lines` | `i64` | 1 | `Set` | `20` | no |
| `--ansi` | `bool` | 0 | `SetTrue` | `false` | no |
| `--json` | `bool` | 0 | `SetTrue` | `false` | no |

Parser defaults only; runtime environment fallbacks and value constraints remain in the prose.
<!-- END GENERATED cli-member-capture -->

- `--lines`: Number of trailing lines to capture (default: **20**).
- `--ansi`: Preserve ANSI escapes in the raw capture. The default strips ANSI escapes and cleans carriage-return redraws.

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

The shared `CaptureSnapshot::from_raw(raw, ansi, now)` supplies this
content, timestamp, and lowercase SHA256 hex for both member capture and
monitor scan. It hashes the final selected content's UTF-8 bytes, including
Unicode and empty captures. Existing line validation/windowing and timestamp
format stay unchanged. Text member capture prints content without adding a
newline; JSON retains its existing fields and ordering.

## `cafleet monitor` — Supervision Scheduler {#cafleet-monitor}

`cafleet monitor` is a two-form command. The bare positional form runs the
supervision loop; the `scan` subcommand is a one-shot batch capture. Both
forms run behind the [stale-assets guard](#stale-assets-guard).

| Form | Behavior |
|---|---|
| `cafleet monitor FLEET_ID [--tick N] [--interval N]` | The in-process scheduler loop. |
| `cafleet monitor scan FLEET_ID [--lines N] [--ansi] [--json]` | Capture the Director's pane + every active member's pane once, print, exit. No loop, no `monitor_runtime` claim, no DB writes. |

### The loop form

`cafleet monitor FLEET_ID [--tick N] [--interval N]` takes the positional
`FLEET_ID` subject. The conceptual model is
canonical on the [Monitoring](../concepts/monitoring.md) concepts page; there
is no stop subcommand — deleting the monitor member kills the pane hosting
the loop, and a still-running loop self-terminates on its next tick after
`fleet delete`.

<!-- BEGIN GENERATED cli-monitor -->
| Argument | Type | Values per occurrence | Action | Parser default | Required |
|---|---|---|---|---|---|
| `FLEET_ID` | `i64` | 1 | `Set` | — | yes |
| `--tick` | `i64` | 1 | `Set` | `5` | no |
| `--interval` | `i64` | 1 | `Set` | — | no |

Parser defaults only; runtime environment fallbacks and value constraints remain in the prose.
Required arguments apply to the loop form; selecting a subcommand negates them.
<!-- END GENERATED cli-monitor -->

- `--tick`: The scan-tick cadence in seconds (an integer ≥ 1, default **5**). The tick is the floor on interval precision — see [Monitoring](../concepts/monitoring.md#cadence-and-tick-precision).
- `--interval`: The wake interval in seconds (an integer ≥ 0); `0` disables the wake while the loop keeps heartbeating. When omitted, falls back to `CAFLEET_MONITOR_WAKE_INTERVAL` (default **600**).

The startup-resolved interval (`--interval` > `CAFLEET_MONITOR_WAKE_INTERVAL`
> 600) is stamped into the fleet's `monitor_runtime` row at each start and
re-read on every tick, so a
[`PATCH /api/monitor`](webui-api.md#patch-api-monitor)
edit changes the running loop's cadence within one tick.

Runs the loop **in-process** and blocks. The monitor member hosts it as a
long-lived execution using its backend's launch primitive; the loop writes to its stdout — one
`<iso-ts> tick -> wake monitor <monitor-member-id> (<N> members)` line per
delivered wake). On startup it runs the multiplexer precondition guard,
atomically claims the single-instance `monitor_runtime` row, installs
`SIGTERM`/`SIGINT` handlers (a clean stop clears the row), and — immediately
after the successful claim, before the first tick — prints the startup line
the monitor member confirms before sending `monitor live` to the Director:

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

#### Monitor resource cleanup {#monitor-resource-cleanup}

`MonitorLease` ownership begins immediately after a successful runtime
claim. Each successful SIGTERM/SIGINT registration retains its own handle.
Registration failure (including the second handler), startup write or flush
failure, tick failure, normal stop, and owner displacement all release the
registered handles and attempt the ownership-checked runtime clear. Startup
still writes the exact line above after handler installation and before the
first tick; a flush failure is an error rather than a successful startup.

`clear_monitor_runtime(fleet_id, pid)` clears only that owner, preserving a
replacement PID and the existing wake ledger. If work and clear both fail,
retain the primary error/exit category and append the clear diagnostic. If
only clear fails, return that error. SIGKILL or a crash cannot run cleanup;
existing stale-owner reclaim remains the recovery path. Signal tests use
per-call injected registration handles and a stop flag, without changing
process-global signal handlers.

### `cafleet monitor scan` {#cafleet-monitor-scan}

`cafleet monitor scan FLEET_ID [--lines N] [--ansi] [--json]` — a one-shot
batch capture of the whole fleet: the Director's pane plus every active
member's pane, captured back-to-back and printed in a single invocation.
No loop runs, no `monitor_runtime` row is claimed, the command performs no
DB writes, and capture content is never stored in SQLite.

<!-- BEGIN GENERATED cli-monitor-scan -->
| Argument | Type | Values per occurrence | Action | Parser default | Required |
|---|---|---|---|---|---|
| `FLEET_ID` | `i64` | 1 | `Set` | — | yes |
| `--lines` | `i64` | 1 | `Set` | `20` | no |
| `--ansi` | `bool` | 0 | `SetTrue` | `false` | no |
| `--json` | `bool` | 0 | `SetTrue` | `false` | no |

Parser defaults only; runtime environment fallbacks and value constraints remain in the prose.
<!-- END GENERATED cli-monitor-scan -->

- `--lines`: Trailing lines captured per pane (an integer ≥ 1, default **20**).
- `--ansi`: Preserve ANSI escapes in every captured content. The default strips ANSI escapes, as in [`member capture`](#member-capture).
- `--json`: Emit the JSON array instead of the text sections.

**Roster.** The scan requires a live fleet (a soft-deleted or unknown fleet
errors — see [Error Messages](#error-messages)) and a resolvable
multiplexer — the same guards as the loop form. The roster is the Director's
row first, then every other active member owning a placement row, ascending
by `member_id`. A member with no placement row (not spawned via
`cafleet member create`) is excluded, mirroring the wake roster's join; a
placement row with a `NULL` pane (pending placement) stays in the roster as
an annotated entry. A fleet with no members scans the Director's pane only.

**Per-entry capture.** For each roster entry, in order:

| Condition | Entry outcome |
|---|---|
| The pane id is `NULL` (pending placement) | Annotated entry: `pane not available (pending placement)`. |
| The pane capture errors (dead pane, backend failure — including the Director's own pane) | Annotated entry: `capture failed: <error>`. |
| The capture succeeds | Content (ANSI-stripped unless `--ansi`), `captured_at` stamped from local UTC at that entry's read boundary, `content_sha256` over the emitted content. |

The scan always completes: an annotated entry never aborts the remaining
captures, and a scan whose every entry is annotated still exits 0.

**Text mode** — one section per roster entry, separated by one blank line.
`<name>` is the raw DB value (stdout is not a keystroke path, so no
sanitization). `kind` is `director` or `member`, making the Director row
self-identifying beyond its first position.

```text
=== <member-id> (<name>; kind=<kind>; coding_agent=<coding_agent>; pane=<pane-id>; captured_at=<ts>) ===
<content>
```

An annotated entry (the pane token is `—` when no pane exists; a failed
capture keeps its real pane id):

```text
=== <member-id> (<name>; kind=<kind>; coding_agent=<coding_agent>; pane=—) ===
pane not available (pending placement)
```

**JSON mode** — a top-level array, same order, one object per entry
mirroring [`member capture`](#member-capture)'s keys plus `name` / `kind` /
`coding_agent` / `error`, in this pinned key order:

```json
{
  "member_id": 4,
  "name": "drafter",
  "kind": "member",
  "coding_agent": "claude",
  "pane_id": "%2",
  "lines": 20,
  "content": "…",
  "captured_at": "2026-08-04T11:20:00.000000+00:00",
  "content_sha256": "…",
  "error": null
}
```

The shared capture path retains typed scan results until the output
branch. Only the text presenter builds the section headings; JSON presentation
does not construct or discard them. Both modes preserve the current roster
order, raw member names, line count, error annotations, and success timestamps.

On an annotated entry `content`, `captured_at`, and `content_sha256` are
`null`; `error` carries the exact annotation string from text mode;
`pane_id` is `null` for a pending placement and the real pane id for a
failed capture. `lines` always echoes the requested depth.

| Exit | Meaning |
|---|---|
| `0` | Completed scan (annotated entries included) |
| `1` | Unknown or soft-deleted fleet |
| `1` | Multiplexer unreachable |
| `2` | Usage errors |

## Error Messages

| Command | Situation | Error message | Exit | Notes |
|---|---|---|---|---|
| (any surface resolving a config dir) | A set config-path variable holds a non-absolute value (empty string and relative paths included) | `Error: <VAR> must be an absolute path (got '<value>')` | 1 | See [Config-dir resolution](#config-dir-resolution); `doctor` renders it as a per-agent issue instead |
| (any non-setup command) | The recorded schema version is behind the embedded head | `Error: database schema is outdated (schema <M>, head <N>); run 'cafleet setup'` | 1 | See [Schema-version guard](#schema-version-guard) |
| (any non-setup command) | No schema ledger and no app tables (missing or empty DB file) | `Error: no cafleet database; run 'cafleet setup'` | 1 | See [Schema-version guard](#schema-version-guard) |
| (any non-setup command) | No schema ledger but app tables present | `Error: database has tables but no schema history — not a cafleet database?` | 1 | See [Schema-version guard](#schema-version-guard) |
| (any non-setup command) | The recorded schema version is ahead of the embedded head | `Error: database schema <M> is newer than this cafleet (head <N>); upgrade cafleet` | 1 | See [Schema-version guard](#schema-version-guard) |
| (any fleet-scoped command) | No agent has an `asset_installs` row at its currently-resolved path | `Error: no assets install is recorded at the resolved paths; run 'cafleet setup' to install` | 1 | See [Stale-assets guard](#stale-assets-guard) |
| (any fleet-scoped command) | An `asset_installs` row at a resolved path differs from the runtime CLI version | `Error: stale assets detected (<agent>=<recorded>[, ...]; CLI <runtime>); run 'cafleet setup' to reinstall` | 1 | See [Stale-assets guard](#stale-assets-guard) |
| `member create` | `--coding-agent opencode` with no agent preset at the resolved preset path | `Error: opencode agent preset not found at <preset>; run 'cafleet setup --coding-agent opencode' first` | 1 | `<preset>` is the resolved `<preset base>/agents/cafleet.md` — see [Config-dir resolution](#config-dir-resolution) |
| `setup` | Existing duplicate active monitors prevent a pending migration | `active monitor duplicates prevent migration: fleet <id>: members <ids>; ...` | 1 | DB-half failure; see [duplicate-monitor recovery](../concepts/storage.md#duplicate-monitor-recovery) |
| `setup` | The `asset_installs` table is missing as the assets half starts | `the database schema is missing or outdated; run 'cafleet setup' first` | 1 | An assets-half failure, after a db-half failure or an externally broken schema |
| (any subject-taking command) | Missing positional subject id | The parser's native missing-required-argument error naming the positional | 2 | — |
| (any id argument) | A non-integer id | The parser's native invalid-value error | 2 | — |
| `fleet create` | `--name`, `--coding-agent`, or `--monitor-file` omitted | The parser's native missing-required-argument error naming the flag | 2 | — |
| `fleet create` | Run outside a supported multiplexer | `Error: cafleet fleet create must be run inside a tmux or herdr session` | 1 | No DB writes |
| `fleet create` | `--monitor-file` resolution failure (empty, missing, unreadable, or non-UTF-8 file, or the empty-stdin variant via `-`) | The `--file` rejection strings with the flag label `--monitor-file` (e.g. `Error: --monitor-file <path>: file is empty.`) | 1 | The message names the flag the user typed; nothing written |
| `fleet create` | Invalid `--monitor-model` for the `--coding-agent` backend | The `member create` `--model` validation string, verbatim | 2 | Nothing written — see [Model selection](coding-agent-backends.md#model-selection) |
| `fleet create` | The `--coding-agent` binary missing from `PATH` | `Error: binary <name> not found on PATH` | 1 | Nothing written |
| `fleet create` | Split fails during the monitor pane spawn | `Error: tmux split-window failed: <detail>` followed by the applicable rollback result/cleanup diagnostics | 1 | The primary reason is retained; rollback-success suffix requires confirmed compensation — see [creation failure compensation](#creation-failure-compensation) |
| `fleet create` | Placement insert or commit fails after a successful pane spawn | The underlying error followed by applicable cleanup diagnostics | 1 | Broker attempts rollback and closes the transaction before the CLI attempts pane kill; successful rollback leaves no bootstrap rows |
| `fleet show` / `fleet delete` | Unknown `FLEET_ID` | `Error: fleet 'X' not found.` | 1 | — |
| `member create` | Unknown `--fleet-id` | `Error: Fleet '<fleet-id>' not found.` | 2 | Director auto-discovery runs first thing |
| `member create` | Into a soft-deleted fleet | `Error: fleet X is deleted` | 1 | — |
| `member create` | The fleet row has no `director_member_id` recorded | `Error: fleet <fleet-id> has no root Director recorded; re-create the fleet with 'cafleet fleet create'.` | 1 | Mid-bootstrap corruption |
| `member create` | With a placement, when the fleet's root Director is not an active member | `Error: fleet <fleet-id>'s root Director (member <id>) is not active.` | 1 | The `register_member` invariant guard |
| `member create` | `--role monitor` when the fleet already has an active monitor member | `Error: fleet <fleet-id> already has an active monitor member (member <member-id>)` | 1 | The early check precedes the monitor-first guard; an in-transaction conflict returns the same error with no losing member, placement, or pane |
| `member create` | Without `--role` when the fleet has no active monitor member | `Error: fleet <fleet-id> has no active monitor member; spawn one with --role monitor first` | 1 | The monitor-first placement guard, before any registration or pane effect — satisfied by the `fleet create` bootstrap; it fires only after a monitor death with no re-spawn |
| `member delete` | Against the root Director's id | `Error: cannot deregister the root Director; use 'cafleet fleet delete' instead` | 1 | — |
| `message poll` | An unknown or inactive `MEMBER_ID` | `Error: Member <member-id> not found` | 1 | — |
| `message ack` / `message show` | An unknown `MESSAGE_ID` | `Error: Message <message-id> not found` | 1 | — |
| `message ack` | A message not in the `input_required` state | `Error: Cannot ACK message in state <state>` | 1 | — |
| `message send` / `message broadcast` | The sender is unknown or inactive | `Error: Sender member not found or not active: <from-member-id>` | 1 | — |
| `message send` | The recipient is unknown or inactive | `Error: Destination member not found: <to-member-id>` | 1 | — |
| `message send` | Sender and recipient in different fleets | `Error: members <from-member-id> and <to-member-id> are not in the same fleet.` | 1 | — |
| `message send` | The row was persisted but the attempted pane notification failed | `Error: Message <message-id> was persisted, but pane notification failed: <raw backend error>. Do not resend this message. Recover the recipient pane, then run 'cafleet member ping <recipient-id>' or have the recipient run 'cafleet message poll <recipient-id>'.` | 1 | The row stays `input_required`; self-send and no-pane skips stay exit 0 — see [message send](#message-send-partial-failure) |
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
| `member create` / `fleet create` | An unknown `{placeholder}` in the prompt | `Error: Unknown placeholder '<name>' in custom prompt. Supported placeholders: {fleet_id}, {member_id}, {director_member_id}, {coding_agent}. Double literal braces ({{, }}) to keep them as text.` | 2 | `member create` attempts deregistration; `fleet create` attempts bootstrap transaction rollback; cleanup failures follow the primary error |
| `member create` / `fleet create` | A malformed brace expression in the prompt | `Error: Malformed custom prompt: <detail>. Double literal braces ({{, }}) to keep them as text.` | 2 | `member create` attempts deregistration; `fleet create` attempts bootstrap transaction rollback; cleanup failures follow the primary error |
| `member create` | `--coding-agent` omitted and the spawning Director not found in the fleet | `Error: cannot resolve the member's coding agent: Director <director-id> not found in fleet <fleet-id>. Re-run with an explicit --coding-agent.` | 1 | Nothing spawned |
| `member create` | `--coding-agent` omitted and the spawning Director has no placement row | `Error: cannot resolve the member's coding agent: Director <director-id> has no placement row recording its backend. Re-run with an explicit --coding-agent.` | 1 | Nothing spawned |
| `monitor` (loop form) | The fleet already has a live monitor | `Error: monitor already running for fleet <id>` | 1 | — |
| `monitor` / `monitor scan` | An unknown or soft-deleted fleet | `Error: fleet <id> not found` | 1 | — |
