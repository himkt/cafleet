# CAFleet — Reimplementation Specification

This is the self-contained interface and observable-behavior specification for the `cafleet` CLI, broker, independent coding-agent sessions and embedded assets. Exact authored errors, JSON fields/order and command layouts govern; incidental host-language quoting and operating-system error suffixes preserve intent. Complete module contracts appear in [§6](#6-module-specifications).

## 1. Overview & goals

CAFleet multiplexes coding-agent sessions in tmux or herdr and coordinates them with durable point-to-point SQLite messages. The host owns task continuation and native execution results; the Director owns delegation, finite receiving, evidence inspection, authorization and cleanup. The shipped product consists of the CLI, embedded skills/presets and documentation site.

Reproduce the documented command/option/exit surface, configuration defaults, current schema, enum values, output shapes, lifecycle, finite receive, notification, locking, readiness and native dispatch semantics. The current format identity defines compatibility; required data corruption surfaces at its read boundary.

## 2. Architecture stance

The core is synchronous. A finite receive invocation owns its bounded wait and notification retries. Separate processes coordinate SQLite changes through transactions and complete pane submissions through process-shared file locks. Host-native background tools own task/session handles and collect terminal results.

A Director retains live receiving while replies remain outstanding or explicitly hands unfinished work to the user. Consumed deliveries are ACKed immediately; deferred actions remain in host-owned state. Timeout triggers evidence checks rather than a completion/failure inference.

## 3. Module layout

| Concern | Responsibility |
|---|---|
| Configuration | Selected database, text limit and multiplexer override |
| Persistence | Connection factory, format/shape diagnosis and embedded migrations |
| Broker | Typed fleet/member/message/asset records and synchronous transactions |
| Output | Ordered wire presenters, truncation and capture formatting |
| Multiplexer | Discovery, pane creation/capture/input/kill and layout compensation |
| Pane input | Stable OS locks, styled readiness and checked literal submission |
| Notification / receiving | Synchronous preview attempt and caller-owned finite retries |
| Coding-agent backends | Registry, availability, model/effort validation and spawn argv |
| CLI | Parse/guard/compose these contracts and emit results |

## 4. Architecture & module dependency graph

```text
CLI -> receiving / notification -> broker -> persistence
       pane input -> styled readiness
                  -> multiplexer -> bounded process runner
CLI -> coding-agent backends -> availability probes
CLI -> ordered output presenters
```

The broker persists/queries typed data. It performs no pane I/O. Multiplexer utilities use injected process runners; bounded pipe draining and owned-child cleanup remain independent of native command submission. Pane ownership transfers only after successful split, with explicit rollback guards.

## 5. Shared domain model

### 5.1 Timestamps

Serialize timestamps as fixed-width UTC `YYYY-MM-DDTHH:MM:SS.ffffff+00:00` text with six fractional digits. Compare canonical timestamps lexicographically for SQL ordering/maxima. Parse only for age/idle arithmetic; externally supplied parsing accepts missing fractional parts and UTC offset spellings. Durations/deadlines use monotonic time.

### 5.2 Core entities

| Entity | Required shape |
|---|---|
| Fleet | ID, optional name, creation/deletion timestamp and root Director reference |
| Member | ID, fleet, name, description, active/deregistered status and registration/deregistration timestamps |
| Placement | Member, multiplexer session/window, optional pending pane, mux backend, agent backend and creation time |
| Message | Sender, integer recipient, exact text, creation and consumption state/timestamp |
| Asset install | Backend/config-path identity, installed CLI version and timestamp |

The full column and index declarations are in §8. Records decode required columns with typed errors.

### 5.3 Enums

`MemberStatus` is `active` / `deregistered`; `MessageStatus` is `input_required` / `completed`. Unknown required enum strings fail. Supported backend tokens come from their registries. Readiness is `idle`, `working`, `awaiting_user`, `draft` or `unknown`.

### 5.4 Member kind discriminator

Derive `director` exactly when `fleets.director_member_id == members.member_id`, otherwise `member`. A live fleet requires a valid active Director and placement.

### 5.5 Recipient and pending placement

Every message has an integer `to_member_id` identifying its inbox owner. Nullable `mux_pane_id` represents pending member creation and documented diagnostics; required concrete pane actions fail contextually until it is patched.

### 5.6 Typed records and wire presenters

Broker records carry typed values independently of CLI formatting. Public message records are ordered `message_id, from_member_id, to_member_id, created_at, status_state, status_timestamp, text`. Full JSON remains untruncated; internal notification state stays in storage.

## 6. Module specifications

### 6.1 Persistence & Schema

The fresh baseline creates exactly the application tables declared in the schema. Compatibility diagnosis verifies current application identity, ledger version, every required table’s column shape, and required indexes/foreign keys. Known versions use their recorded migration shape; newer versions are diagnosed before any head-shape comparison. Unrelated auxiliary tables, indexes and triggers are compatible while those required definitions remain intact.

Connections use SQLite foreign-key enforcement and a normal five-second busy timeout. A bounded operation caps that wait to its remaining budget. Resolve a relative selected SQLite file path against the invoking working directory and canonicalize the database file before lock-key directory construction.

Classify database identity, ledger version and required table/column shapes for the recorded supported version before every write and before either setup half installs assets. Empty zero-identity files initialize with the baseline atomically. Current-family known older versions migrate; an ahead ledger version rejects both setup write paths before database mutation, asset installation or asset-install recording. Independent half-error reporting applies only after the compatibility gate. Other populated/identified or malformed files fail without content writes:

`database format is incompatible at 'PATH'; choose an empty database path and run 'cafleet setup'.`

Preserve database open/read causes and incompatible file bytes/sidecars. Doctor reports classification while completing other read-only diagnostics. Setup and invocation guards use this same classifier.

### 6.2 Broker

#### Fleets

Bootstrap validates the calling multiplexer context and backend token, then inserts fleet, Director and placement and backfills the Director reference in one transaction. The current session remains the Director; no pane split or process launch occurs.

List live fleets by descending creation timestamp/ID. Show includes soft-deleted fleets. Delete locks/revalidates active placements, soft-deletes the fleet and deregisters its members/removes placements in one transaction, preserving messages. Repeat deletion returns zero newly deregistered members.

#### Members

Registration requires a live fleet and active valid root Director. Insert the member and pending placement atomically. Creation owns compensation for later prompt/split/patch failures. Root Director individual deletion fails. Member deletion locks its concrete pane, kills it and deregisters it; a placementless/pending member requires no concrete pane lock.

Roster reads active entries, derive kind from the fleet reference and include nullable activity. Sent/received/ACK maxima determine `idle` seconds, truncated to an integer; null is the correct state without activity. Required Director/placement/enum corruption fails rather than producing placeholder values.

#### Messaging

Send requires active sender/recipient in the same live fleet and persists exact text. Every individual fan-out send commits independently; keep already committed IDs when another recipient fails. Poll/wait read incoming unread messages by descending status timestamp/message ID. Show requires existence. ACK requires an unread existing row, stamps completion and removes notification eligibility. Consumption is independent of assignment completion.

Pending preview selection chooses the oldest eligible row by `(created_at, message_id)` for an active ordinary recipient. Revalidate fleet, member, placement, message status and null notification stamp while holding pane input ownership. Stamp success conditionally only after complete submission.

#### Broker errors

Preserve the validation boundary and explicit cause. Missing member/message, inactive fleet/Director, cross-fleet pair and already-ACKed rows fail with application errors. Cross-fleet send uses `members {from} and {to} are not in the same fleet.`; a missing destination uses `Destination member not found: {to_id}`. Unknown subjects use `Member ID not found` / `Message ID not found`; unknown fleet uses `fleet 'ID' not found.`

### 6.3 CLI

#### Subcommand summary

| Command | Purpose | Subject | JSON |
|---|---|---|---|
| `setup` | Initialize/migrate the current database and install assets | — | no |
| `doctor` | Diagnose multiplexer, database, agent assets and member permissions | — | yes |
| `fleet create` | Register the calling Director atomically | — | yes |
| `fleet list` / `fleet show` | Read fleet registry | `FLEET_ID` for show | yes |
| `fleet delete` | Deregister a fleet and its members | `FLEET_ID` | yes |
| `fleet capture` | Capture the active roster once | `FLEET_ID` | yes |
| `member create` | Register and spawn an independent agent | `--fleet-id` relationship | yes |
| `member delete` | Kill its owned pane and deregister | `MEMBER_ID` | yes |
| `member show` / `member list` | Read detail / active roster | `MEMBER_ID` / `FLEET_ID` | yes |
| `member prompt` / `member exec` | Submit authorized native input | `MEMBER_ID` | yes |
| `member ping` | Submit a fixed inbox-poll prompt when idle | `MEMBER_ID` | yes |
| `member capture` | Read a pane tail | `MEMBER_ID` | yes |
| `message send` | Persist a point-to-point message | Sender/recipient flags | yes |
| `message poll` / `message wait` | Read incoming deliveries / receive finitely | `MEMBER_ID` | yes |
| `message ack` / `message show` | Consume / read one message | `MESSAGE_ID` | yes |

#### Environment variables

| Variable | Field | Default | Meaning |
|---|---|---|---|
| `CAFLEET_DATABASE_URL` | `database_url` | `sqlite:///` + `~/.local/share/cafleet/cafleet_v8.db` | Selected SQLite file path |
| `CAFLEET_MULTIPLEXER` | `multiplexer` | Auto-detect | Explicit supported multiplexer override |
| `CAFLEET_MAX_TEXT_LEN` | `max_text_len` | `200` | Text-body and pane-preview codepoint limit |

The default expands HOME once. An explicit URL has the form `sqlite:///<path>`; resolve a relative file path against the invoking working directory before canonicalizing the database file for shared pane-lock identity. A literal tilde in an override is not expanded. Count values accept nonnegative integers and invalid values fail with `<NAME> must be a non-negative integer (got '<value>')`.

#### Global Options

`cafleet --version` prints `cafleet <version>` and exits 0 before dispatch.

#### Output shapes {#output-shapes}

| Command | Text | JSON |
|---|---|---|
| `fleet create` | `<fleet-id> director=<member-id>` | Ordered `fleet_id, name, created_at, director`, with Director identity/description/registration/placement |
| `fleet list` | `FLEET_ID DIRECTOR NAME MEMBERS CREATED_AT` columns | Fleet records |
| `fleet show` | Fleet row with `deleted_at:` | Fleet including nullable `deleted_at` |
| `fleet delete` | `Deleted fleet ID. Deregistered N members.` | `{deregistered_count}` |
| `message send` | `Message sent.` then envelope | `{message, notification_sent}` |
| `message poll` | Envelopes, or `No messages found.` | Full record array |
| `message ack` | `Message acknowledged.` then envelope | `{message}` |
| `message show` | Envelope | `{message}` |
| `message wait` | Availability/timeout line and any messages | `{member_id, outcome, messages, notification_errors}` |
| `member create` | `ID NAME backend=AGENT pane=PANE` | Member with placement |
| `member delete` | `Member deleted.` with member/pane lines | `{member_id, pane_status}` |
| `member show` | `ID NAME STATUS` | Detail including kind and placement |
| `member list` | Roster rows or `0 members.` | Roster array |
| `member prompt` | `Sent prompt TEXT_REPR to member NAME (PANE).` | `{member_id, pane_id, text}` |
| `member exec` | `Dispatched native command to member ID (PANE). Inspect the pane for its result.` | `{member_id, pane_id, coding_agent, command, dispatched}` |
| `member ping` | Dispatch or skip line | `{member_id, pane_id, skipped, reason}` |
| `member capture` | Exact selected content | `{member_id, pane_id, lines, content, captured_at, content_sha256}` |
| `fleet capture` | One annotated section per entry | Ordered capture-entry array |
| `doctor` | Complete diagnosis and issue footer | Diagnosis object |

##### Member list output {#member-list-output}

Roster records include `member_id`, `name`, `kind`, `placement`, `last_sent`, `last_recv`, `last_ack` and `idle`. `kind` is `director` when the fleet reference matches, otherwise `member`. Missing activity is null; placement may be null, or its pane ID pending. Text uses `-` for absent values and `(pending)` for a pending pane.

#### JSON output (`--json`) {#json-output}

Use a trailing per-subcommand `--json`. It emits compact single-line JSON with UTF-8 and complete untruncated message text. Application errors remain stderr text; parse errors exit 2. Commands marked no reject the flag, including the root group.

#### Positional subject ids {#positional-subject-ids}

Place `FLEET_ID`, `MEMBER_ID` or `MESSAGE_ID` immediately after the subcommand. The subject row supplies related fleet/recipient state. IDs are complete DB-assigned integers, without prefix resolution or environment defaults. A missing or malformed subject uses the parser's native error, exit 2.

Spawned members read literal `FLEET ID:`, `YOUR MEMBER ID:`, `DIRECTOR MEMBER ID:` and `CODING AGENT:` lines. Pass these literal IDs on each invocation.

#### Sender and recipient (`--from-member-id`, `--to-member-id`) {#from-to-member-id}

Send requires integer sender and recipient flags. Both must be active in the same live fleet. The sender row determines the fleet; IDs are coordination identifiers within the local trust model.

#### `permissions.allow` coverage

Use prefix rules such as `Bash(cafleet message *)` to cover ordinary broker operations, including `cafleet message wait ID --timeout 300 --json`. Director capture is covered by `Bash(cafleet fleet capture *)` or a broader authorized prefix. Literal IDs and trailing JSON flags preserve prefix matching.

Keep `Bash(cafleet member prompt *)` and `Bash(cafleet member exec *)` under `permissions.ask`, because their bodies carry operator-controlled input. Ask rules outrank allow rules. The permission profiles describe each role's needs.

#### Message Body Truncation

CLI text and inline preview bodies use `CAFLEET_MAX_TEXT_LEN` Unicode codepoints followed by `…` when truncated. Full JSON and pane captures preserve their complete content. Message envelope owns the compact rendering.

#### Text bodies (positional `TEXT` / `--file`) {#text-body-input}

Send and member creation require exactly one positional body or `--file PATH`; `--file -` reads UTF-8 stdin. Preserve exact nonblank body text. Missing/both sources or a blank positional body exit 2; blank file/stdin sources and file-read errors exit 1 with the authored diagnostics below. File read/UTF-8 failures retain the actual cause. Compact role-by-path spawn prompts remain necessary because resolved prompts enter backend/multiplexer argv.

`member prompt` takes one positional line. Native exec has the separate command contract.

#### `cafleet setup` — Onboarding and Schema Management {#cafleet-setup}

`cafleet setup [--coding-agent AGENT...]` validates the selected database format before either half writes. Initialize an empty zero-identity file with the embedded baseline in one transaction; migrate known current-family versions. An ahead ledger version or incompatible shape rejects both setup write paths before database mutation, asset installation or asset-install recording. Independent half-error reporting applies after this compatibility gate. A format failure prevents asset installation and content writes. Compatible setup installs embedded assets offline and records versions. Backend selection is multi-value/repeatable, deduplicated and defaults to the supported backend registry.

The database half prints `Created PATH and applied migrations to head (N).`, `Upgraded from M to N.` or `Already at head (N); nothing to do.` according to state. Report actual errors as `db half failed: CAUSE` or `assets half failed: CAUSE`; failed halves exit 1. Assets require a compatible usable current schema.

##### Config-dir resolution {#config-dir-resolution}

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
stale-assets guard fronting every fleet-scoped
command, `member create` included, resolves all three identity paths. One
exception to strict lazy failure: `doctor` catches per-agent resolution
errors and renders them as issues instead of aborting (see
doctor).

**Recorded-path identity.** Every surface that keys on "the agent's resolved
path" — the `asset_installs` rows, the
stale-assets guard, and `doctor`'s setup column — uses
one canonical path per agent, the resolved base directory, stored absolute
exactly as resolved (no canonicalization beyond the absolute-path
validation):

| Agent | Recorded / status-keyed path |
|---|---|
| claude | The resolved claude config dir (`$CLAUDE_CONFIG_DIR` or `~/.claude`) |
| codex | The resolved codex home (`$CODEX_HOME` or `~/.codex`) |
| opencode | The resolved preset base (`$OPENCODE_CONFIG_DIR` or `~/.opencode`) — the only opencode root that can vary; the skills base is fixed and carries no identity |


##### Assets half {#assets-half}

Each agent's preset, where one exists, is a static file embedded in the
binary next to the skills; install targets resolve per
Config-dir resolution:

| Agent | Bundled preset | Install target |
|---|---|---|
| claude | — (skills only) | — |
| codex | `presets/codex/cafleet.rules` | `<codex base>/rules/cafleet.rules` (default `~/.codex/rules/cafleet.rules`) |
| opencode | `presets/opencode/cafleet.md` | `<preset base>/agents/cafleet.md` (default `~/.opencode/agents/cafleet.md`) |

Each backend replaces the two embedded skills sequentially, removes a leftover
`cafleet-research` entry, then replaces its preset where present. Existing targets
are deleted before their replacements are written. Unrelated skills remain
untouched. A symlink target is unlinked as an entry;
its referent is not deleted. Skills-operation errors retain `failed to install
skills into <skills_dir>: <error>` and preset-operation errors retain `failed
to install preset into <target>: <error>`.

The command prints each success line after that operation completes, then
records the installed version after all operations succeed. The preset line
appears only for codex and opencode:

```
<agent>: installed cafleet, cafleet-design-doc (v<version>) -> <skills dir>
<agent>: installed preset (v<version>) -> <target>
```


#### Shared diagnosis and connection reuse {#diagnosis-reuse}

Use the invocation’s shared classifier and connection for guard/setup work. Doctor completes read-only sections even when the database is unavailable. Compatibility includes identity, ledger version and expected table/column shapes.

#### Schema-version guard {#schema-version-guard}

The fresh baseline creates exactly the application tables declared in the schema. Compatibility diagnosis verifies current application identity, ledger version, every required table’s column shape, and required indexes/foreign keys. Known versions use their recorded migration shape; newer versions are diagnosed before any head-shape comparison. Unrelated auxiliary tables, indexes and triggers are compatible while those required definitions remain intact.

Every fleet/member/message command validates current format before writes and asset checks. Identity is `application_id=1128351288`; malformed or incompatible files fail with `database format is incompatible at 'PATH'; choose an empty database path and run 'cafleet setup'.` Preserve file bytes and existing sidecars. Empty state reports `no cafleet database; run 'cafleet setup'`. Behind state reports `database schema is outdated (schema M, head N); run 'cafleet setup'`; ahead reports `database schema M is newer than this cafleet (head N); upgrade cafleet`. Database-open/read errors preserve their cause. Setup initializes/repairs compatible known versions; doctor reports rather than blocks its other diagnostics.

#### Stale-assets guard {#stale-assets-guard}

Every fleet-scoped surface — the `fleet`, `member`, and `message` groups — validates the recorded assets installs after the
schema-version guard passes and before any
subcommand body runs. The guard resolves each agent's identity path per
Config-dir resolution and checks only the row at
that path. Apply the following precedence:

| Recorded install state | Result | Exit |
|---|---|---|
| A config-path variable fails validation | The pinned validation error | 1 |
| No agent has a row at its currently-resolved path in a compatible at-head database | `Error: no assets install is recorded at the resolved paths; run 'cafleet setup' to install` | 1 |
| A row at a resolved path has `cafleet_version` ≠ the runtime CLI version (string inequality — either direction) | `Error: stale assets detected (<agent>=<recorded>[, ...]; CLI <runtime>); run 'cafleet setup' to reinstall`, stale agents in ascending order | 1 |
| Every row at a resolved path matches | The command proceeds silently | 0 |

Staleness checks use version records at currently-resolved paths; agents without
a matching row contribute nothing to staleness. Plain `cafleet setup` installs
agents at their resolved paths. Setup and doctor are exempt:

| Exempt surface | Why exempt | Behavior under a stale/missing install |
|---|---|---|
| `setup` | It must remain runnable to repair the install | Runs normally — it is the repair path |
| `doctor` | It reports instead of blocking | Renders each agent's setup state in the coding-agents section |

`--help` renders at parse time and exits before any command body runs, so
neither group-level help (`cafleet fleet --help`) nor subcommand help
(`cafleet fleet create --help`) triggers either guard — both always print
help, even under a missing database or a missing or stale install.


#### `cafleet fleet` — Fleet Management

##### `fleet create`

`cafleet fleet create --name NAME --coding-agent AGENT [--json]`

Require the selected backend token and a valid tmux/herdr context. Outside that context fail with `cafleet fleet create must be run inside a tmux or herdr session`. Insert fleet, current Director member and placement in one transaction and backfill `director_member_id` before commit. The Director has `name="Director"` and `description="Root Director for this fleet"`. Creation splits no pane and launches no process. An ordinary member can be created immediately.

##### `fleet list`

List non-deleted fleets by descending creation timestamp/fleet ID. Rows retain the Director reference so its ID remains discoverable.

##### `fleet show`

`cafleet fleet show FLEET_ID [--json]` includes soft-deleted rows. Unknown ID: `fleet 'X' not found.`, exit 1.

##### `fleet delete`

`cafleet fleet delete FLEET_ID [--json]` locks active concrete placements in sorted key order, revalidates the placement set, then stamps deletion and deregisters active members/removes placements in one transaction. It retains messages and is idempotent. Close owned member panes individually first.

One operation-wide five-second deadline bounds lock acquisition, with 50-millisecond retries and set-change restart. Contention leaves state unchanged and exits 1 with `fleet ID pane input remained busy for 5 seconds at member MEMBER_ID; fleet was not deleted.`

##### `fleet capture`

`cafleet fleet capture FLEET_ID [--lines N] [--ansi] [--json]` checks fleet existence and soft-deletion before resolving the multiplexer. An absent or deleted fleet exits 1 with `Error: fleet ID not found`. Capture reads the current active roster once, Director first then placement-owning members by ascending ID. The default depth is 20. Per-pane failures, including a pending Director placement, annotate entries and leave the rest available. It writes no broker state.

Text sections begin `=== ID (NAME; kind=KIND; coding_agent=AGENT; pane=PANE; captured_at=TS) ===`, separated by a blank line. A failed/pending pane omits capture time and carries its diagnostic.

JSON entries are ordered `member_id, name, kind, coding_agent, pane_id, lines, content, captured_at, content_sha256, error`. Kind is director/member. On error, content/time/hash are null; pane_id is null only without a concrete pane. Successful hash/timestamp semantics match member capture.

#### `cafleet doctor` — Environment Diagnosis {#cafleet-doctor}

A full-environment diagnosis that renders **all** sections even when the
multiplexer is unavailable or the database is missing or stale — no early
abort. Diagnosis order: multiplexer, database, coding agents, member
permissions. `doctor` is
exempt from the schema-version guard and the
stale-assets guard — a missing or outdated database
and a stale or missing install are reported, not fatal.

The only flag is the optional `--json`, a trailing
per-subcommand flag.

##### Text layout

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
✓ member permissions
  claude: no setting blocks the member broker commands
1 issue found
```

##### Multiplexer section

`✓` with the five detail lines (`backend`, `session`, `window_id`,
`pane_id`, `presence`). On any multiplexer or environment failure (no
supported multiplexer, ambiguous environment, binary not on `PATH`, pane not
discoverable): `✗ multiplexer` with the resolver's error message as the
single detail line, and the report continues. One issue.

##### Database section

One detail line describes the database state (`<M>` recorded version,
`<N>` embedded head). An empty database has `application_id=0` and no
non-internal tables. Validate application identity `1128351288` and the
migration ledger first, then read the recorded version. For supported
versions, validate required table/column, index and foreign-key shapes
against that recorded version's migrations. Older supported versions
therefore remain eligible for in-place migration. A newer version reports
ahead without comparing its unknown shape to the current head.
A missing required table (including `asset_installs`) or malformed shape in
a supported version is incompatible even when its ledger version equals
head. Compatible auxiliary tables, indexes and triggers remain accepted.

| State | Glyph | Detail line | Issue |
|---|---|---|---|
| Empty database, or no DB file | `✗` | `no database — run: cafleet setup` | yes |
| Incompatible identity, required shape or ledger | `✗` | `database format is incompatible at 'PATH'; choose an empty database path and run 'cafleet setup'.` | yes |
| Compatible identity/shape and recorded `<M>` = `<N>` | `✓` | `schema <N> (head)` | no |
| Compatible identity/shape and recorded `<M>` < `<N>` | `✗` | `schema <M>, head is <N> — run: cafleet setup` | yes |
| Valid application identity/ledger and recorded `<M>` > `<N>` | `✗` | `schema <M> is newer than this CLI (head <N>) — upgrade cafleet` | yes |

Connection and SQL inspection failures render `✗` with their actual cause as
the detail line (one issue). Doctor completes the coding-agents section for
every database result. It reads recorded installs only for a compatible
at-head database. Every other database state renders resolvable agents as
`–`, with no recorded versions or superseded footnotes, and contributes one
database issue to exit status 1. A dropped `asset_installs` table therefore
produces the incompatible-format diagnosis before any install-row lookup.

##### Coding agents section

A light box-drawing framed table (`┌ ─ ┬ ┐ │ ├ ┼ ┤ └ ┴ ┘`), header separator
only, no per-row rules. Column alignment uses **display width**, never byte
length. One row per agent in the fixed order `claude`, `codex`, `opencode`.

| Column | Content |
|---|---|
| `coding agent` | The agent name. |
| `path` | The resolved identity path (Config-dir resolution), with `~` abbreviation when under `$HOME`. On a resolution error: the raw invalid variable value. |
| `source` | The winning origin: `$<VAR>` (e.g. `$CLAUDE_CONFIG_DIR`) or `default`. |
| `setup` | The setup state below, keyed on the **resolved path only**. |

| State | Cell | Issue |
|---|---|---|
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

##### Member permissions section {#member-permissions}

Reports every Claude Code setting that would stop a claude member from running
its broker commands, so the failure is visible before a spawn. The section
runs for `claude` when the claude assets are installed, and reads each of
these files that exists:

| File | Location |
|---|---|
| User settings | `<claude config dir>/settings.json`, per Config-dir resolution |
| Project settings | `<cwd>/.claude/settings.json` and `<cwd>/.claude/settings.local.json` |
| Managed settings | `/Library/Application Support/ClaudeCode/managed-settings.json` on macOS, `/etc/claude-code/managed-settings.json` on Linux |

Each rule is tested against these probe commands:

```
cafleet message send --from-member-id 1 --to-member-id 2 x
cafleet message poll 1
cafleet message wait 1 --timeout 300 --json
cafleet message ack 1
cafleet message show 1
```

A rule matches a probe when it is the bare `Bash`, or `Bash(<pattern>)` whose
pattern matches the probe. `*` matches any text; a trailing ` *` or the legacy
`:*` also matches the bare command.

| Finding | Condition |
|---|---|
| Blocking rule | A `permissions.deny` or `permissions.ask` rule in any file matches a probe. |
| Managed rules only | The managed file sets `allowManagedPermissionRulesOnly` to `true` and its own `permissions.allow` does not match every probe. |
| Unreadable file | A settings file exists but does not parse as JSON. A missing file is expected and is not a finding. |

With no finding the section is `✓`; otherwise it is `✗` with one detail line
per finding, and each finding is one issue:

```
✓ member permissions
  claude: no setting blocks the member broker commands

✗ member permissions
  claude: ~/.claude/settings.json permissions.deny "Bash(cafleet *)" matches "cafleet message send --from-member-id 1 --to-member-id 2 x"
  claude: <file> allowManagedPermissionRulesOnly is true and its permissions.allow does not match "<command>"
  claude: <file> is not valid JSON
```

The three `✗` lines are, in order, a blocking rule (naming the file, the
list, the rule, and the probe it matches), the managed-rules-only finding
(naming the first unmatched probe), and an unreadable file.

When the claude assets are not installed the row reads
`claude: – not installed` and never counts as an issue.

The section cannot see a blocking `PreToolUse` hook, or a rule added after a
member is spawned — see
Spawn-time allow rules.

##### Footer and exit code

Last line: `no issues found`, `1 issue found`, or `<N> issues found` (proper
pluralization). Exit code: 0 when no issues, 1 otherwise — the `–` state and
footnotes never count. Every failure is a rendered issue; no failure exits
before output.

##### `--json`

Mirrors the sections with `ok` booleans, unabbreviated absolute paths, and
the issue count. `source` holds the winning env-var **name** (no `$`) or the
literal `"default"`; `state` is `"ok" | "stale" | "not_installed" | "error"`
(`"error"` is the per-agent resolution-error state; `"not_installed"` never
contributes to `issues`). Every agent row carries the same keys. `error` is
the validation message for `"error"`, and otherwise null, without an `Error: ` prefix. Section `error` fields likewise hold
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
    "schema_version": 1,
    "head_version": 1,
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
  "member_permissions": {
    "ok": true,
    "findings": []
  },
  "issues": 1
}
```

`member_permissions` sits between `coding_agents` and `issues`. Each element
of `findings` carries the keys `coding_agent`, `file`, `list`, `rule`, and
`command`, in that order:

| Finding | `list` | `rule` | `command` |
|---|---|---|---|
| Blocking rule | `"deny"` or `"ask"` | The matching rule | The probe it matches |
| Managed rules only | `"allowManagedPermissionRulesOnly"` | `null` | The first unmatched probe |
| Unreadable file | `null` | `null` | `null` |

Each finding adds one to `issues`.

On a multiplexer failure the `multiplexer` object is `{"ok": false,
"backend": null, "session": null, "window_id": null, "pane_id": null,
"presence_var": null, "presence_value": null, "error": "<message>"}`. On an
agent resolution error the row is `{"coding_agent": "...", "path": null,
"source": "<VAR>", "recorded_version": null, "installed_at": null, "state":
"error", "error": "<VAR> must be an absolute path (got '<value>')"}` — the
raw invalid value appears only inside `error`; `path` stays `null` because
no path resolved. `schema_version` contains the recorded version after identity
and ledger validation, plus shape validation for supported versions. A newer
version retains its recorded version without a head-shape comparison;
incompatible or missing state uses `null`. For every non-`ok` database report, recorded
installs are not read: resolved agents use `"not_installed"`, path errors
remain `"error"`, recorded fields are null and `superseded` is empty.
A missing `asset_installs` table is an incompatible database state.
Exit-code semantics are identical to text mode.


#### `cafleet message` — Message Broker

##### `message send`

`cafleet message send --from-member-id ID --to-member-id ID (TEXT | --file PATH) [--json]`

Persist one point-to-point delivery, then attempt at most one oldest eligible ordinary-member preview. A committed/persisted send remains successful with exit 0; held/unavailable notifications preserve the stdout record and `notification_sent=false`. The Director receives by broker reads; self-sends have settled notification state.

`notification_sent` refers to the newly committed message; delivery of an older preview leaves it false. Push notifications specifies selection and under-lock revalidation.

###### Delivery outcome {#message-send-delivery}

On the first attempted unknown/draft capture, stderr is `Message NEW_ID was persisted. member ID is STATE; preview PREVIEW_ID was not sent.` Other post-commit notification/stamp failures also name the persisted ID without suppressing successful stdout. Storage failure before commit exits 1 with its cause. Consume an existing committed ID once; notification failure supplies no reason to create another delivery.

##### `message poll`

`cafleet message poll MEMBER_ID [--json]` returns only incoming `input_required` records, descending by status timestamp/message ID. Unknown or inactive member: `Member ID not found`, exit 1.

##### `message wait`

`cafleet message wait MEMBER_ID [--timeout SECONDS] [--json]`

Timeout is an integer in `1..=1800`, default `300`. Validate an active subject and live fleet. Existing unread deliveries return immediately; otherwise check at one-second monotonic intervals. The invocation owns finite receiving and never ACKs messages.

Between inbox checks, retry at most one ordinary-member preview per interval with an in-memory round-robin cursor. Exclude the Director, self-notifications, ACKed and already-notified rows. Recheck the inbox before starting another attempt. Shared input locks and under-lock revalidation keep multiple waiters safe.

Start each receive check or notification attempt before the monotonic receive deadline. An attempt already in flight retains its original five-second total budget for revalidation, capture, typing, settle and stamp. Cap subprocess/database busy waits to its remaining budget; return by the receive deadline plus at most one in-flight five-second attempt/cleanup allowance. Stop between stages when the attempt budget expires and report uncertainty if bytes may have arrived.

The ordered JSON result is:

```json
{"member_id":41,"outcome":"timeout","messages":[],"notification_errors":[]}
```

`outcome` is `messages` with nonempty full poll records, or `timeout` with an empty array. Each diagnostic is `{member_id, message_id, delivery, error}`, where delivery is `not_sent` or `uncertain`. Keep only the latest error per member in memory.

Unknown/draft diagnostics are `member ID is STATE; preview MESSAGE_ID was not sent.` Working, approval, pending placement and lock contention are ordinary holds. Reconcile diagnostics with each successful receive snapshot and the typed observations of its owned notification attempt. Remove entries when these observations show ACK, member departure or successful notification by any invocation; a later error replaces the previous one. After receive expiry, return the last successful snapshot reconciled with the completed attempt's observations. Subsequent external changes become visible to the next invocation; final reconciliation starts no additional I/O or tail budget.

Text is `Messages available for member ID.` plus normal messages, or `Timed out waiting for member ID.`; diagnostics go to stderr. Both outcomes exit 0. Deleted subject/fleet or corrupt/unreadable state exits 1 with its cause. Ctrl+C terminates the finite process normally.

The Director ACKs every consumed delivery before the next wait, including questions or deferred approvals; unfinished actions remain in host-owned task state. Use the default for a host-native background task, retain its live task/session handle and collect the terminal result. A foreground-only mechanism uses an explicit timeout of at most 30 seconds. On timeout inspect roster, panes and artifacts, then receive again while delegated work remains. Keep active receive ownership until the reply or explicit user handoff.

##### `message ack`

`cafleet message ack MESSAGE_ID [--json]` transitions an existing unread message to completed, updating its status timestamp. The recipient/fleet derive from that row. ACK signifies consumption.

##### `message show`

`cafleet message show MESSAGE_ID [--json]` returns one full record. Existence is the read guard.

#### `cafleet member` — Member Lifecycle + Pane Interaction {#cafleet-member}

##### Member targeting and key delivery

Require an active member and placement for pane operations; concrete pane IDs remain opaque. Pending placement is valid during creation and yields contextual prompt/exec/capture errors, or a ping skip. Missing placement fails pane actions; list/show retain diagnostic nulls. Exec and ping reject the root Director before pane I/O; individual member deletion protects it.

##### `member create` {#member-create}

Register a member **and** spawn its coding-agent pane. It takes no identity
flag: the acting Director is auto-resolved from `fleets.director_member_id`
first thing, before registration (the resolved id also feeds the member's
backend inheritance and the spawn-prompt substitution). A fleet has exactly
one root Director by construction, so no override flag exists.

| Argument | Required | Notes |
|---|---|---|
| `--fleet-id` | yes | The fleet the new member joins — a relationship flag; the subject of the command is the member being created. |
| `--name` | yes | Display name — see Known asymmetries for pane-title behavior. |
| `--description` | yes | One-sentence purpose. |
| `--coding-agent` | no | One of `claude`, `codex`, or `opencode`; when omitted, the member — every role — inherits the spawning Director's placement backend. Exits 1 with `Error: binary <name> not found on PATH` when the binary is missing. The opencode backend additionally requires its agent preset at the resolved preset path (via `OPENCODE_CONFIG_DIR` — see Config-dir resolution); a missing preset or an invalid variable exits 1 per Error Messages. |
| `--model` | no | Model forwarded to the backend binary's own `--model` flag. The opencode backend additionally requires `<provider-id>/<model-id>`; per-backend formats and create-time validation are in Model selection. |
| `--effort` | no | Reasoning-effort level forwarded to the backend binary, validated per backend before any side effect. Accepted levels, forwarding forms, and rejection strings are in Reasoning effort. |
| positional `PROMPT` | one of | Inline spawn prompt (backend-neutral template). Exactly one of `PROMPT` / `--file`. |
| `--file PATH` | one of | Path to a UTF-8 file whose contents are the spawn prompt (`-` = stdin). Follow the shared body-input and transport contract. |
| `--json` | no | Output as JSON |



###### Spawn command per backend

The per-backend spawn argv and auto-approval flags live in
Coding-agent backends.

###### Spawn-prompt substitution

`member create` renders its resolved spawn prompt body
with the same formatter, substituting exactly four placeholders:

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
malformed brace expression exits 2. Member creation attempts deregistration;
fleet creation attempts bootstrap transaction rollback. Cleanup failures follow the primary error
(see Error Messages).

The spawn always creates the pane without stealing focus (tmux
`split-window -d`): the Director's pane and active window stay active. In the
default output, `pane` renders `(pending)` until the pane id is patched onto
the placement.

###### Creation failure compensation {#creation-failure-compensation}

Member creation registers a pending placement, renders the prompt, creates a
pane, and patches its id. The CLI owns a registration guard after successful
registration and acquires a pane guard only when `split_window` succeeds.
The backend owns the pane before that return; see
pane ownership.

| Failure point | Compensation order | DB result when compensation succeeds |
|---|---|---|
| Member registration | Broker rolls back registration; no pane guard exists | No new member or placement |
| Member placeholder expansion | CLI deregisters; no pane was created | Member becomes deregistered and placement is removed |
| Member Herdr run after split returned an id | Backend tries pane kill, returns the error, then CLI deregisters | Member deregistered and placement removed |
| Member placement update error or missing row | After handling the failed SQL, CLI kills pane, then deregisters | Member deregistered and placement removed |

Continue remaining compensation even if an earlier cleanup fails. A backend
`PaneCleanup::Attempted` result is never killed again by the CLI. A failed
split with no confirmed id reports that the pane id is unknown and cleanup
is unconfirmed, including for member creation; it does not infer which other
pane should be closed. Creation rollback uses `kill_pane(id, true)`.

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
member placement is confirmed , disarm all creation
guards before the existing `emit` call. Successful text/JSON output keeps its
shape, ordering, and nulls. No new stdout-failure exit or diagnostic contract
is introduced.



##### `member delete` {#member-delete}

Lock the concrete placement before kill/deregistration. A member without a concrete pane needs no lock. Retry contention every 50 milliseconds against one five-second deadline; expiry exits 1 with `member ID's pane input remained busy for 5 seconds; member was not deleted.` and leaves state unchanged. Preserve pane/registry errors and layout cleanup ownership.

Revalidate pending or absent placement and deregister atomically. For a concrete pane, retain input ownership while closing it outside a SQLite write transaction, then revalidate and finalize the registry. After successful close, an expired budget permits uncontended finalization and permits no further lock wait; actual database failures report that the pane was closed and preserve the database cause. Optional layout work follows mandatory finalization.

##### `member show` {#member-show}

`cafleet member show MEMBER_ID [--json]` — the positional `MEMBER_ID` names
any active registry entry — placed or placementless (root Director included).

Registry read — no multiplexer requirement. Text is the compact one-line row;
the detailed view — `kind` (`director` / `member`), and the
placement sub-dict — is the `--json` payload (see
Output shapes).


##### `member list` {#member-list}

`cafleet member list FLEET_ID [--json]` — the positional `FLEET_ID` names the
fleet whose roster is listed; no identity flag. Lists every **active**
registry entry of the fleet —
the root Director, ordinary members, and placementless rows. An empty roster
prints `0 members.`.

Field projection and absent values are in Member list output.

Rows and activity follow the data-model query contract.
Text idle values render as `Ns` / `Nm` / `Nh`.
Per-member detail such as `description` and `registered_at` lives on
`member show`.


##### `member prompt` {#member-prompt}

`cafleet member prompt MEMBER_ID TEXT [--json]` validates a nonblank line without CR/LF or C0/DEL controls, otherwise exits 2 with `text must be one non-empty line without control characters.`

Acquire the shared input lock, revalidate activity/placement and require idle. Submit literal text, settle and Enter. Exit 0 means submission only, with `{member_id, pane_id, text}`.

A held lock uses `member ID's pane is receiving another input; retry after it is idle.` Readiness rejection is `member ID is STATE; prompt was not dispatched.` Pre-write error: `prompt was not dispatched to member ID: CAUSE`. Partial uncertainty: `prompt delivery to member ID is uncertain: CAUSE. Inspect the pane before retrying.` These exit 1 without a success record.

##### `member exec` {#member-exec}

`cafleet member exec MEMBER_ID COMMAND [--json]` or `cafleet member exec MEMBER_ID --file PATH [--json]`; `--file -` reads stdin. Exactly one source is required. File/stdin is UTF-8 and strips at most one terminal LF or CRLF. Preserve inline bytes exactly.

Require a nonblank line without CR/LF, NUL or other C0/DEL controls, including tabs. Invalid input exits 2 before pane I/O with `command must be one non-empty line without control characters.` A multiline task supplies a one-line explicit script invocation.

Resolve an active ordinary member and concrete placement, reject the root Director, lock input, reload and require idle. Type native shell input `! ` followed by the actual command once, settle and Enter. The backend owns working directory, environment, permissions, output and exit rendering.

Successful exit 0 establishes complete submission/dispatch only. Ordered JSON is:

```json
{"member_id":42,"pane_id":"w1:p2","coding_agent":"codex","command":"printf example","dispatched":true}
```

Text: `Dispatched native command to member ID (PANE). Inspect the pane for its result.`

| Condition | Error body (exit 1) |
|---|---|
| Root Director | `cannot exec in the Director's own pane` |
| Pending placement | `member ID has no pane yet (pending placement) — nothing to exec.` |
| Input lock held | `member ID's pane is receiving another input; retry after it is idle.` |
| Non-idle/unknown | `member ID is STATE; native command was not dispatched.` |
| Capture/transport before typing | `native command was not dispatched to member ID: CAUSE` |
| Typing may have begun | `native command delivery to member ID is uncertain: CAUSE. Inspect the pane before retrying.` |

The first text-writing call can partially type even if it fails. Failure after Enter may mean execution began. Inspect native pane output before any newly authorized retry. The command writes no execution ledger, helper script or wrapper and returns only dispatch metadata. The Director authorizes native input under its host rules; visibility establishes no second member-tool permission check.

After native completion and idle evidence, send an explicit prompt to use the output and continue when the host has not resumed the member.

##### `member ping` {#member-ping}

`cafleet member ping MEMBER_ID [--json]` is an explicit Director action. A Director target exits 1 with `cannot ping the Director's own pane` before capture or input. Lock/revalidate the target, require idle, then submit exactly `cafleet message poll ID --json` with settle/Enter.

Successful JSON is `{member_id, pane_id, skipped:false, reason:null}`; text is `Pinged member NAME (PANE) — poll keystroke dispatched.`

Pending placement, lock contention and readiness yield exit 0 with `skipped:true`. Reasons are `pending_placement`, `busy`, `working`, `awaiting_user`, `draft`, `unknown`; pane_id is null only for pending placement. Pending text: `Member NAME has no pane yet (pending placement) — ping skipped; it will poll its inbox on spawn.` Other skip text: `Member NAME (PANE) is REASON — ping skipped.`

Capture/transport failures exit 1 with their cause; possible partial input is `poll-trigger delivery to member ID is uncertain: CAUSE. Inspect the pane before retrying.` Finite receiving creates no ping.

##### `member capture` {#member-capture}

`cafleet member capture MEMBER_ID [--lines N] [--ansi] [--json]` — the
positional `MEMBER_ID` names the target member.

- `--lines`: Number of trailing lines to capture (default: **20**).
- `--ansi`: Preserve ANSI escapes in the raw capture. The default strips ANSI escapes and cleans carriage-return redraws.

Output shapes are in Output shapes; target resolution is
shared with the `member` keystroke verbs — see
Member targeting and key delivery. A
pending placement is a hard error (see Error Messages).

JSON stamps `captured_at` from the local UTC
clock at the capture read boundary and computes
`content_sha256 = sha256(content.encode("utf-8"))` from the exact emitted
`content`. The default mode hashes the ANSI-stripped,
carriage-return-defragmented
string; `--ansi` hashes the ANSI-preserving string. No normalization occurs
after the selected mode, and capture content is never stored in SQLite.

The shared `CaptureSnapshot::from_raw(raw, ansi, now)` supplies this
content, timestamp, and lowercase SHA256 hex for both member capture and
fleet capture. It hashes the final selected content's UTF-8 bytes, including
Unicode and empty captures. Existing line validation/windowing and timestamp
format stay unchanged. Text member capture prints content without adding a
newline; JSON retains its existing fields and ordering.


#### Error Messages

Application errors print as stderr text even with `--json`; the flag selects successful output. Input errors use the documented usage/application category. Dispatch, readiness, compatibility and deletion-lock errors are specified at their action owners above.

| Command | Situation | Error message | Exit | Notes |
|---|---|---|---|---|
| (any surface resolving a config dir) | A set config-path variable holds a non-absolute value (empty string and relative paths included) | Config-path validation | 1 | See Config-dir resolution; `doctor` renders it as a per-agent issue instead |
| (any non-setup command) | The recorded schema version is behind the embedded head | Schema-version guard | 1 | See Schema-version guard |
| (any non-setup command) | No schema ledger and no app tables (missing or empty DB file) | Schema-version guard | 1 | See Schema-version guard |
| (any non-setup command) | No schema ledger but app tables present | Schema-version guard | 1 | See Schema-version guard |
| (any non-setup command) | The recorded schema version is ahead of the embedded head | Schema-version guard | 1 | See Schema-version guard |
| (any fleet-scoped command) | No agent has an `asset_installs` row at its currently-resolved path | Stale-assets guard | 1 | See Stale-assets guard |
| (any fleet-scoped command) | An `asset_installs` row at a resolved path differs from the runtime CLI version | Stale-assets guard | 1 | See Stale-assets guard |
| `member create` | `--coding-agent opencode` with no agent preset at the resolved preset path | `Error: opencode agent preset not found at <preset>; run 'cafleet setup --coding-agent opencode' first` | 1 | `<preset>` is the resolved `<preset base>/agents/cafleet.md` — see Config-dir resolution |
| `setup` | No database remains after a failed db half, or `asset_installs` is removed after the assets-half format check | `the database schema is missing or outdated; run 'cafleet setup' first` | 1 | An assets-half failure |
| (any subject-taking command) | Missing positional subject id | The parser's native missing-required-argument error naming the positional | 2 | — |
| (any id argument) | A non-integer id | The parser's native invalid-value error | 2 | — |
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
| `message send` | The sender is unknown or inactive | `Error: Sender member not found or not active: <from-member-id>` | 1 | — |
| `message send` | The recipient is unknown or inactive | `Error: Destination member not found: <to-member-id>` | 1 | — |
| `message send` | Sender and recipient in different fleets | `Error: members <from-member-id> and <to-member-id> are not in the same fleet.` | 1 | — |
| `member exec` | The target is the fleet's root Director | `Error: cannot exec in the Director's own pane` | 1 | — |
| `member capture` / `member prompt` / `member exec` | The member has a pending placement | <code>Error: member &lt;id&gt; has no pane yet (pending placement) — nothing to &lt;capture&#124;prompt&#124;exec&gt;.</code> | 1 | `member ping` instead skips and exits 0 — see member ping |
| `member show` / `prompt` / `exec` / `ping` / `capture` | An unknown or inactive target member id | `Error: Member <member-id> not found` | 1 | — |
| `member prompt` / `exec` / `ping` / `capture` | A target with no placement row | ``Error: member <member-id> has no placement row; it was not spawned via `cafleet member create`.`` | 1 | — |
| `message send` / `member create` | Neither the positional body nor `--file`, or both | The parser's native argument-group error | 2 | — |
| `message send` / `member create` | Positional body empty or whitespace-only | `Error: text may not be empty.` | 2 | — |
| `message send` / `member create` | `--file <path>` to an empty (zero-byte or whitespace-only) file | `Error: --file <path>: file is empty.` | 1 | — |
| `message send` / `member create` | `--file -` with empty or whitespace-only stdin | `Error: --file -: stdin is empty.` | 1 | — |
| `message send` / `member create` | `--file <path>` to a non-existent path or non-regular file | `Error: --file <path>: file does not exist or is not a regular file.` | 1 | — |
| `message send` / `member create` | `--file <path>` to an unreadable file | `Error: --file <path>: file is not readable.` | 1 | — |
| `message send` / `member create` | `--file <path>` to a file containing invalid UTF-8 | `Error: --file <path>: file is not valid UTF-8.` | 1 | — |
| `member create` | `--coding-agent opencode --model` violating the `<provider-id>/<model-id>` format | Backend model error | 2 | Fires before any side effect |
| `member create` | `--effort` with a level unknown to the claude backend | Backend effort errors | 2 | Fires before any side effect |
| `member create` | `--coding-agent codex --effort` with an unknown level | Backend effort errors | 2 | Fires before any side effect |
| `member create` | `--coding-agent opencode --effort` with any value | Backend effort errors | 2 | Fires before any side effect |
| `member create` | An unknown `{placeholder}` in the prompt | `Error: Unknown placeholder '<name>' in custom prompt. Supported placeholders: {fleet_id}, {member_id}, {director_member_id}, {coding_agent}. Double literal braces ({{, }}) to keep them as text.` | 2 | `member create` attempts deregistration; cleanup failures follow the primary error |
| `member create` | A malformed brace expression in the prompt | `Error: Malformed custom prompt: <detail>. Double literal braces ({{, }}) to keep them as text.` | 2 | `member create` attempts deregistration; cleanup failures follow the primary error |
| `member create` | `--coding-agent` omitted and the spawning Director not found in the fleet | `Error: cannot resolve the member's coding agent: Director <director-id> not found in fleet <fleet-id>. Re-run with an explicit --coding-agent.` | 1 | Nothing spawned |
| `member create` | `--coding-agent` omitted and the spawning Director has no placement row | `Error: cannot resolve the member's coding agent: Director <director-id> has no placement row recording its backend. Re-run with an explicit --coding-agent.` | 1 | Nothing spawned |

### 6.4 Output & Formatting


A message is a durable point-to-point delivery. The recipient owns its inbox and explicitly acknowledges consumption.

#### Persisted shape

The messages table stores typed routing/status columns, exact body text and internal `notified_at` state. Required values decode with typed errors.

The ordered public record is:

```json
{
  "message_id": 42,
  "from_member_id": 7,
  "to_member_id": 3,
  "created_at": "2026-05-05T05:42:11.123456+00:00",
  "status_state": "input_required",
  "status_timestamp": "2026-05-05T05:42:11.123456+00:00",
  "text": "build OK"
}
```

`to_member_id` is always an integer. ACK changes `status_state` to `completed` and stamps `status_timestamp`; completion means the delivery was consumed, independently of the assignment's progress.

#### Rendered shape

Output shapes owns each command's wrapper. Poll returns an array of full records; show/ACK wrap one record as `message`; send adds `notification_sent`. Wait returns full poll records inside its finite receive result.

##### Text-mode projection

| Field | Text rendering |
|---|---|
| `message_id` | Bracketed ID on the first line |
| `from_member_id` | `from:` segment |
| `status_timestamp` | Timestamp on the first line |
| `text` | Body on the next line when nonempty |
| Recipient, creation and status fields | Available in full JSON |

```text
[42 | from:7 | 2026-05-05T05:42:11.123456+00:00]
build OK
```

Text bodies use `CAFLEET_MAX_TEXT_LEN` codepoints plus `…`; JSON preserves the complete exact text. Pane previews apply their separate control-rendering contract.


#### Capture snapshot

A capture timestamps the read boundary in local UTC and hashes the final selected content’s UTF-8 bytes as lowercase SHA-256 hex. Plain mode strips ANSI and defragments carriage-return redraws; ANSI mode preserves styling. Text prints selected content without adding a newline. JSON keeps content, timestamp and hash in order. Capture content is ephemeral.


#### `strip_ansi` regex and CR-defrag

The CSI regex is exactly `\x1b\[[0-?]*[ -/]*[@-~]`: ESC, `[`, any run of
parameter bytes (`0x30`–`0x3F`), any run of intermediate bytes (`0x20`–`0x2F`),
then one final byte (`0x40`–`0x7E`). This matches **CSI sequences only** — OSC,
DCS, and single-character escapes are deliberately **not** stripped. After
stripping CSI matches, split on `\n` and, per line, keep only the substring after
the **last** `\r` (CR-redraw defrag: a TUI redraw `prefix\rNEW` keeps only
`NEW`), then re-join with `\n`. An empty/falsy input returns unchanged.


#### Formatter helper semantics

Ordinary absent member/fleet cells use ASCII `-`; a fleet-capture absent pane uses `—`; doctor’s recommendation uses `–`. ISO-to-HMS takes the first eight characters after `T`; absent/non-string/no-T values render `-`. Idle renders null as `-`, seconds below 60, floor minutes below 3600 and floor hours afterward. Required fields fail if absent; nullable activity and pending placement are documented valid cases.

### 6.5 Multiplexer (tmux + herdr)

#### Backend matrix {#backend-matrix}

| Behavior | tmux | herdr |
|---|---|---|
| Pane ID example | `%7` | `w1:p1` |
| Access | CLI subprocess | CLI subprocess |
| Spawn working directory | Splitting pane inheritance | Explicit `--cwd` |
| Capture for readiness | `capture-pane -e` | `pane read --format ansi` |
| Input | Literal text, settle, Enter | Literal text, settle, Enter |
| Delete layout | Native auto-fit | Best-effort rebalance scoped to the closed pane’s tab |

#### Backend selection {#backend-selection}

Every call site resolves its backend through one shared resolver rather than
hardcoding one. Resolution
precedence:

1. **Explicit override.** If `CAFLEET_MULTIPLEXER` is set, it must name a
   supported backend (`tmux` or `herdr`); an unknown value fails loudly.
2. **Auto-detect from the environment.** `HERDR_ENV` truthy signals a herdr
   session; `TMUX` set signals a tmux session.
3. **Ambiguity is a hard error.** Both `HERDR_ENV` and `TMUX` set → error
   (set `CAFLEET_MULTIPLEXER` to disambiguate). Neither set → error (run cafleet
   inside a tmux or herdr session, or set `CAFLEET_MULTIPLEXER`). Exactly one
   present → that backend.

The outcomes of that order:

| `CAFLEET_MULTIPLEXER` | `HERDR_ENV` | `TMUX` | Result |
|---|---|---|---|
| `tmux` or `herdr` | any | any | That backend |
| An unknown value | any | any | Fails loudly |
| unset | truthy | set | Error — set `CAFLEET_MULTIPLEXER` to disambiguate |
| unset | truthy | unset | herdr |
| unset | not truthy | set | tmux |
| unset | not truthy | unset | Error — run cafleet inside a tmux or herdr session, or set `CAFLEET_MULTIPLEXER` |

Auto-detect (an unset `CAFLEET_MULTIPLEXER`) is the default. `cafleet doctor`
reports the resolved backend and its
identifiers (see CLI options).


#### Error taxonomy

Backend failures share a base `MultiplexerError`, with the per-backend
subclasses in the backend matrix. CLI boundaries catch
`MultiplexerError`, so both backends' failures are handled uniformly while each
backend keeps its own message text.


#### Pane ownership during creation {#pane-creation-ownership}

`split_window` owns a newly created pane until it returns successfully. As
soon as herdr extracts the pane id from the split response, it arms a pane
guard. If the subsequent `pane run` fails, that guard calls
`kill_pane(id, true)` before returning the original run error. A failed close
retains both the pane id and the run error and adds the cleanup failure.
Layout equalization remains best-effort and does not fail an otherwise
successful spawn.

Creation errors carry internal cleanup metadata:

| Metadata | Meaning and caller action |
|---|---|
| `PaneCleanup::Attempted { pane_id, error: None }` | The backend already closed the known pane; the CLI performs the remaining DB compensation without another kill. |
| `PaneCleanup::Attempted { pane_id, error: Some(_) }` | The backend tried to close the known pane and failed; retain that diagnostic, perform remaining DB compensation, and do not retry the pane kill. |
| `PaneCleanup::Unknown` | A failed or malformed split response left no confirmed pane id; report that pane compensation is unconfirmed, and do not guess an id or close another pane. |

On success, ownership transfers immediately to the CLI's creation guard. Guard `finish`/`rollback` explicitly
disarms ownership; `Drop` is a last defense for unhandled ownership, so an
explicit cleanup attempt cannot cause a second kill. CLI registration and
transaction compensation follow the
creation failure order.
Creation compensation closes only the pane owned by that operation.


#### Subprocess output and deadlines

The shared subprocess runner captures stdout and stderr without truncation.
With a timeout, it drains both pipes concurrently from spawn using nonblocking
I/O. Each loop checks the monotonic deadline and the direct child's exit status,
then reads at most 64 KiB per stream; polling waits at most 20 ms or the remaining
deadline, whichever is shorter. Interrupted operations retry against the same
deadline. Completion requires both the child's exit and EOF on both streams.
Success returns stdout as lossy UTF-8; a nonzero exit reports stderr as lossy
UTF-8. Calls without a timeout retain the unbounded output-collection path.

At the deadline, the runner kills and reaps the direct child, closes both read
pipes, and reports a timeout. This also applies when the direct child has exited
but a descendant still holds a pipe open. Descendant termination is outside this
contract. FD setup, read, poll, and child-status failures also release the pipes
and kill/reap the child; cleanup failures accompany the primary error instead of
replacing it. The deadline bounds observation and the start of cleanup; an
unresponsive operating system can delay cleanup itself.


#### Pane spawn working directory {#pane-spawn-cwd}

A member pane spawned by `cafleet member create` starts in the invoking
process's working directory (the Director's pane cwd); each backend realizes
this differently, per the backend matrix.

On herdr, `<dir>` is the invoking process's current working directory. herdr's own
inheritance is not relied upon because herdr spawns `/bin/sh` instead of the
passwd login shell when `SHELL` is unset
(herdr discussion #1517).
An unresolvable cwd fails the spawn loudly with `HerdrError`; there is no
fallback directory.


#### Delete-time pane layout {#delete-time-pane-layout}

Closing a member pane leaves the two backends asymmetric on layout reflow, per
the backend matrix.

herdr's `kill_pane` reads the target pane's tab (`herdr pane get`) before the
close, runs `herdr pane close`, then rebalances. The scoping comes from the
layout read itself: the killed pane is gone, so the rebalance picks a pane
still open in that tab (`herdr pane list`) and anchors the geometry read on it
(`herdr pane layout --pane <surviving>`), which returns that tab's layout
regardless of which tab or pane holds focus. When no pane remains in the tab,
there is nothing to rebalance and the step is skipped. With ≥ 2 members
remaining, the member column is re-equalized to equal heights (the same
invariant the create path enforces); after the last member is deleted, the
Director pane is explicitly restored to full tab width when the layout read
shows a residual right split; a single remaining member needs no resize. The
rebalance silently skips on unexpected layout shapes. Any `HerdrError` during
the rebalance is swallowed: a layout failure never fails `member delete` — the
pane is closed and the member deregistered regardless.


#### Pane input lock {#pane-input-lock}

Every notification, prompt, ping and native command uses one process-shared OS file lock. Acquire the guard before placement revalidation and styled capture; hold it through focus normalization, literal text, settle, Enter and the notification stamp. Close the non-cloned owned handle on every path.

| Property | Contract |
|---|---|
| Directory | `<canonical-database-file>.pane-locks/` adjacent to the database |
| Key | SHA-256 of UTF-8 `backend + NUL + mux_session + NUL + mux_pane_id` |
| File | `<key>.lock`, read/write create without truncation, owner-only creation permissions |
| Acquisition | Nonblocking exclusive `std::fs::File::try_lock`; preserve unexpected errors with path/member context |
| Identity | Keep lockfiles stable while processes may use the database |
| Contention | Hold notifications, skip ping, reject prompt/exec |

Duplicate placements for one pane share the same lock key. Process exit releases ownership. Read-only capture needs no input lock.

Member deletion locks its concrete placement before killing/deregistering. Fleet deletion acquires active placement keys in sorted order and revalidates the placement set in its transaction. Retry contention every 50 milliseconds against one operation-wide five-second monotonic deadline. A changed set releases all guards and restarts within the same deadline. On timeout release all acquired guards and leave registry/panes unchanged.

Errors are `member ID's pane input remained busy for 5 seconds; member was not deleted.` and `fleet ID pane input remained busy for 5 seconds at member MEMBER_ID; fleet was not deleted.`

#### Pane-state classification {#pane-state-classification}

Use the recorded coding-agent backend and a fresh styled capture after locking. tmux captures with `-e`; herdr reads with `--format ansi`. Parse SGR spans in the recognized composer before stripping ANSI for presentation.

| Readiness | Evidence | Action |
|---|---|---|
| `idle` | Affirmative empty composer or verified host-styled empty placeholder | Submit |
| `working` | Active turn, streaming, tool/shell execution or interrupt cue | Hold/reject |
| `awaiting_user` | Question, confirmation or approval UI | Hold/reject |
| `draft` | Unsent user text in the composer | Preserve and hold/reject |
| `unknown` | Missing, ambiguous, truncated or unrecognized evidence | Inspect and hold/reject |

Approval and work evidence outrank empty-composer cues. A dim/grey host suggestion or changing placeholder is empty only in its recognized composer decoration style. The same visible text in user-input style is a draft; grey text elsewhere establishes no readiness. Missing or ambiguous style evidence yields unknown. Claude member argv disables prompt suggestions, while the recognizer still handles operator-enabled suggestions.

#### Prompt dispatch (`send_prompt`) {#prompt-dispatch}

Validate a nonblank literal line without CR/LF or C0/DEL control characters. After locking and idle revalidation, type the payload once, wait the backend settle interval and submit Enter. Native shell input starts with `! ` followed by the validated actual command. Composer controls begin the submitted input.

Each submission has one five-second transport budget. Bound capture and every write/settle/stamp by the remaining budget. A pre-write error reports no dispatch; an error from the first text-writing call or later is uncertain, because bytes may already have arrived. A failure after Enter can mean the command ran. Inspect the pane before authorizing any retry.

The lock excludes CAFleet writers; the user's keyboard and host UI can still change after a capture. Successful dispatch establishes complete submission only.

#### Push notifications {#push-notifications}

The Director receives through broker poll/wait reads; sends to the Director persist without injected text. A self-send is already settled. For an ordinary member, a synchronous send attempts at most one oldest pending unacknowledged preview ordered by `(created_at, message_id)`.

Acquire the shared input lock, then revalidate activity, fleet, concrete placement, message status and `notified_at`. An idle empty composer accepts one literal line:

```text
[cafleet msg MESSAGE_ID from SENDER_ID TIMESTAMP] TEXT
```

Stored text remains exact. Truncate the preview body by `CAFLEET_MAX_TEXT_LEN`, fold CRLF/LF/CR to `⏎`, render tabs as `␉`, and escape other C0/DEL controls as visible hexadecimal escapes. The fixed prefix distinguishes message text from native composer controls.

Stamp `notified_at` only after complete submission and only while the message remains pending. ACK removes preview eligibility. If a process exits after submission but before stamping, a preview may repeat; members use its ID and current inbox/ACK evidence to establish actionable work.

##### Held delivery {#held-delivery}

Working, approval, pending placement and lock contention are ordinary holds. A finite message wait or a later send retries eligible ordinary-member previews. Its in-memory round-robin cursor attempts at most one member per interval.

A first attempted draft or unknown composer yields an immediate diagnostic naming the member and preview ID. Capture and relay retained user drafts before a user decision or authorized replacement. Transport and stamp failures after persistence remain diagnostics alongside a successful send result. No notification outcome erases a committed delivery.

##### Delivery outcomes

`notification_sent` refers to the newly committed message. Delivering an older preview leaves it false. The send contract owns exact output and errors; finite receiving owns diagnostic revalidation and bounded retries.



#### Transport interface and capture windowing


`Multiplexer.split_window(*, reference: MultiplexerContext, env, command) -> str`
takes the full reference context rather than a bare window id: tmux splits a
*window* and uses `reference.window_id`; herdr splits a *pane* and uses
`reference.pane_id`. The `member create` orchestrator (§6.3) hold the Director's `MultiplexerContext` and pass it directly. The
backend owns a created pane until successful return; the CLI takes ownership
then. Failed creation carries `PaneCleanup::Attempted` or
`PaneCleanup::Unknown` metadata when applicable, per §6.3 *Creation ownership
and compensation*. A missing/invalid id is an unconfirmed cleanup, not a
license to infer another target.


#### Capture windowing

Both backends reject `lines <= 0` with
`capture_pane: lines must be positive, got <lines>` and over-fetch by a fixed
1000 lines using their exact argv below. Split raw output on `"\n"` only,
preserving `\r` for the CLI's later CR-defrag. Drop the trailing run of
visually blank lines: test each line for whitespace-only content after per-line
CSI stripping, retaining the original bytes of kept lines. This recognizes
TUI-painted empty rows without altering retained ANSI sequences.

Return the last `lines` remaining lines joined by `"\n"`, with no trailing
newline; an all-blank buffer becomes the empty string. Preserve interior blank
lines. Enforce the last-N window client-side even when the backend returns more
than requested, so the window shows the drawn bottom rather than blank cursor
space. A blank tail deeper than the requested window is why capture over-fetches.


#### `TmuxMultiplexer` method surface

- **`name`** — the registry key literal `"tmux"`.
- **`ensure_available()`** — fail-fast. Raises if `tmux` is not on `PATH` →
  `tmux binary not found on PATH`; or if `TMUX` is unset/empty → `cafleet
  member commands must be run inside a tmux session`.
- **`context_discovery() -> MultiplexerContext`** — resolves the **calling
  shell's** pane via `$TMUX_PANE` (not the active window). Read `TMUX_PANE`;
  missing/empty → `TMUX_PANE is not set; not running inside a tmux pane`. Invoke
  `tmux display-message -p -t <TMUX_PANE> "#{session_name}|#{window_id}|#{pane_id}"`,
  strip, split on `|` into **exactly 3** parts (max-split 2); wrong count →
  `unexpected tmux display-message output: <quoted-output>`. Return the context.
- **`split_window(*, reference, env, command) -> str`** — spawns a new
  **detached** pane and returns its id, splitting `reference.window_id`. Base
  argv `tmux split-window -t <reference.window_id> -P -F "#{pane_id}" -d` (the
  `-d` detach is unconditional; `-P -F "#{pane_id}"` prints the new pane id); for
  each `(k, v)` in `env` append `-e <k>=<v>`; append the `command` argv elements.
  Run, take the printed pane id, then call `select_layout(reference.window_id)`
  (default layout `main-vertical`, **swallowing** any error from it), and return
  the pane id. `select_layout` runs `tmux select-layout -t <reference.window_id>
  <layout>` and is internal to the tmux backend (not on the interface).
- **`capture_pane(*, target_pane_id, lines=20) -> str`** — fail-fast. Validate
  lines, run `tmux capture-pane -e -p -t <target_pane_id> -S -<lines + 1000>`,
  then apply the shared Capture windowing contract above.
- **`list_pane_ids() -> set`** — fail-fast. `tmux list-panes -a -F "#{pane_id}"`
  with `timeout=5`s; split on whitespace; return the pane-id set.
- **`kill_pane(*, target_pane_id, ignore_missing=False)`** — fail-fast. `tmux
  kill-pane -t <target_pane_id>` through the pane-gone-tolerant runner.


#### `HerdrMultiplexer` method surface

Uses the herdr **CLI exclusively** (subprocess, argv list, no shell), mirroring
`TmuxMultiplexer`'s dispatcher. Pane ids are opaque strings (`w1:p1`), never
parsed. A `_run()` dispatcher maps binary-not-found / timeout / non-zero exit to
`HerdrError`, with a `not_found`-tolerant helper for `ignore_missing` teardown.
Each method's herdr realization:

- **`name`** — the registry key literal `"herdr"`.
- **`ensure_available()`** — fail-fast (`HerdrError`): the `herdr` binary is
  missing from `PATH`, or `HERDR_ENV` is unset.
- **`context_discovery() -> MultiplexerContext`** — `HERDR_ENV` present + a
  **single** `herdr pane current` call whose `result.pane` object already carries
  `workspace_id` / `tab_id` / `pane_id` (no follow-up `pane get`). Returns the
  context (`session ← workspace_id`, `window_id ← tab_id`, `pane_id`).

- **`split_window(*, reference, env, command) -> str`** — layout-aware (emulates
  tmux `select-layout main-vertical`): fetches the invoking process's working
  directory once per call via `os.getcwd()` (an `OSError` from the fetch maps to
  `HerdrError("cannot resolve the working directory for pane spawn: …")`; no
  fallback directory), then reads `herdr pane list` and computes the right column
  as the panes in the Director's tab (`tab_id == reference.window_id`) minus the
  Director's own `pane_id`. If empty → `herdr pane split <reference.pane_id>
  --direction right --no-focus --cwd <cwd> [--env K=V …]` (first member); else
  `herdr pane split <max(column)> --direction down --no-focus --cwd <cwd>
  [--env K=V …]` followed by the column-equalization step below,
  `_equalize_tab_column(reference.pane_id)` — anchored on the Director's own
  pane, so the rebalance is independent of which tab or pane holds focus.
  `<cwd>` is the fetched working directory, passed verbatim (absolute path,
  argv list, no quoting). Then `herdr pane run <new_id> "<shlex.join(command)>"`.
  The argv `command` is rendered to a single properly-quoted string with
  `shlex.join` before the `pane run` because `pane run` submits one text line into
  the pane's shell (a genuine semantic difference from the tmux exec-argv path —
  otherwise an argument containing spaces would be re-split). Immediately
  after extracting `<new_id>` from the split response, arm the backend pane
  guard. A later run failure tries `kill_pane(new_id, true)` and returns the
  run error with id and `PaneCleanup::Attempted` metadata; close failure adds
  its diagnostic without replacing the run error. Return success only after
  transferring pane ownership to the caller. A split failure before obtaining
  an id returns unknown/unconfirmed compensation and never guesses a pane.
- **`_equalize_tab_column(anchor_pane_id)`** — herdr has no single reflow command,
  so after appending a member `split_window` rebalances the right column to equal
  heights arithmetically. It reads the tab geometry of the tab containing
  `anchor_pane_id` (`herdr pane layout --pane <anchor_pane_id>` → `result.layout`
  with `panes[].rect{x,y,width,height}` and `splits[].{direction,rect,ratio}`);
  the read is anchored on a pane the backend already holds, so there is no
  `herdr pane current` call and no tab-id comparison. The right column is every
  pane whose `rect.x` is not the minimum x (the Director column); its `down`
  splits form a right-leaning chain where split *k* (top→bottom) separates
  member *k* from the members below.
  Equal heights ⇔ split *k* has ratio `1/(N-k)` (top → `1/N`, …, bottom pair →
  `1/2`). Because `herdr pane resize --amount` is a signed delta on a split's
  ratio, each split is driven to target by one resize: `delta = 1/(N-k) - ratio`
  (rounded to 4 dp; skipped if `|delta| < 1e-3`); `delta > 0` → `herdr pane resize
  --pane <member k> --direction down --amount <delta>`, else `--pane <member k+1>
  --direction up --amount <|delta|>`. Best-effort: any `HerdrError` is swallowed
  so a resize failure never fails a spawn. tmux is unaffected (still
  `select-layout main-vertical`).
- **`kill_pane(*, target_pane_id, ignore_missing=False)`** — three phases.
  (1) Pre-close tab read: `_pane_tab_id` runs `herdr pane get <pane_id>` and
  returns `result.pane.tab_id`; **any** failure — a `HerdrError` (including
  `pane_not_found` for a pane already gone) or a missing envelope field —
  yields `None` and never blocks the close. (2) `herdr pane close <pane_id>`
  through the `not_found`-tolerant runner; a close error not tolerated under
  `ignore_missing` propagates unchanged and no rebalance runs.
  (3) `_rebalance_after_close(target_tab_id)` — best-effort: any `HerdrError`
  is swallowed so a layout failure never fails a delete (the pane is already
  closed). Skips when `target_tab_id` is `None`. Otherwise it resolves an anchor
  for the layout read — the killed pane is gone, so `_surviving_pane_in_tab`
  runs `herdr pane list` and takes the first pane whose `tab_id` equals
  `target_tab_id` (a missing envelope field raises `HerdrError`); `None` — no
  pane left in that tab — skips the rebalance. It then reads that tab's layout
  via `_read_tab_layout(anchor_pane_id)` (`herdr pane layout --pane <anchor>`,
  which by construction returns the anchor pane's tab; a missing envelope field
  raises `HerdrError`), returns on an empty pane list, and computes the member
  column as the panes whose `rect.x` is not the minimum x, sorted by `y`. Column
  case table: size ≥ 2 → `_equalize_column` (the `_equalize_tab_column`
  arithmetic above — `1/(N-k)` split targets, one signed resize per off-target
  split, the `len(down_splits) != n - 1` malformed-chain skip); size 1 → no
  resize (heights are trivially equal and the right split's ratio is unaffected
  by a down-close); size 0 → `_restore_director_full_width`: with exactly one
  pane and exactly one residual `right` split, emit one corrective `herdr pane
  resize --pane <director> --direction right --amount <round(1.0 - ratio, 4)>`
  (skipped when the delta `< 1e-3`); an empty `splits` list is already
  structurally full-width (nothing emitted), and any other residue (multiple
  splits, a non-`right` split, ≥ 2 panes) is skipped. The create path shares
  `_read_tab_layout` / `_equalize_column` with `_equalize_tab_column`;
  tmux `kill_pane` stays a bare `kill-pane` (native auto-fit).
- **`list_pane_ids() -> set`** — `herdr pane list` → the set of pane ids.
- **`capture_pane(*, target_pane_id, lines=20) -> str`** — fail-fast. Validate
  lines, run `herdr pane read <id> --format ansi --source recent-unwrapped --lines <lines + 1000>`,
  then apply the shared Capture windowing contract above, including its
  client-side last-N limit when the daemon returns excess rows.


#### `MultiplexerContext` (frozen value type)

Immutable, three non-nullable string fields, no defaults, constructed only by
`context_discovery`: `session` (tmux session name), `window_id` (e.g. `@N`),
`pane_id` (e.g. `%N`).


#### Literal input and process errors

Use argv subprocesses without a shell for multiplexer utilities. Type literal payload with tmux `send-keys -l` or herdr `pane send-text`, then the configured settle and a separate Enter. Every notification/prompt/ping/exec uses the shared owned guard and operation deadline; automatic interruption keys are absent.

Preserve binary-not-found, nonzero stderr and timeout context in the backend error type. Pane-gone tolerance applies only to an explicit ignore-missing teardown and a recognized pane-not-found cause; other errors propagate. Layout rebalance remains best effort after confirmed close.

### 6.6 Finite receiving and pane input

The receive task is the synchronous caller-owned process. Its cursor and latest-per-member diagnostics live only in memory. Check the subject/inbox and reconcile diagnostics against each successful snapshot before another attempt. Retry one eligible ordinary member per one-second interval. Start a receive check or attempt only before the monotonic receive deadline; after expiry return the last successful snapshot reconciled with its owned attempt's typed result. A later invocation observes subsequent external changes.

Each notification or explicit input operation has one five-second budget covering capture, literal input, settle, Enter and conditional notification stamping. Bound database busy waits and process calls by remaining budget. A receive returns by its own deadline plus one in-flight attempt/cleanup allowance. Distinguish pre-write failure from possible partial submission at the first writing call.

The shared OS guard uses `File::try_lock`. Store stable, owner-created lockfiles under `<canonical-database-file>.pane-locks/`, keyed by SHA-256 of backend, NUL, session, NUL, pane ID. Acquire before reload/capture; retain ownership through final Enter/stamp. Close on every path. Keep lockfiles in place. Placement deletion locks the same keys with sorted acquisition, revalidation and one five-second deadline.

### 6.7 Coding-agent backends

The backend registry may use a name-to-backend map or a backend enum; either representation must satisfy the same registry contract.

**Scope:** the `CodingAgent` interface and `claude`/`codex`/`opencode` backends
that determine which binary to launch and how to build its spawn `argv`, plus
the shipped per-agent preset files (§ preset sections below) and the
`CODING_AGENTS` registry.

#### Interface

A `CodingAgent` is a stateless backend object selected per member at spawn time.
Each exposes two read-only properties and three methods:

- **`name`** — the backend's registry key, a stable lowercase string (`"claude"`
  / `"codex"` / `"opencode"`). This MUST equal both the persisted
  `placement.coding_agent` value used to look the backend up and the key under
  which it is registered.
- **`binary_name`** — the executable resolved against `PATH` (`"claude"` /
  `"codex"` / `"opencode"`).
- **`ensure_available()`** — raises if any spawn precondition is unmet: the
  binary resolves on `PATH` and (for backends with bundled presets) the preset
  file exists on disk. A shared helper resolves `binary_name` against `PATH`
  and, on a miss, raises `binary {binary_name} not found on PATH`.
  Preconditions read the host environment through a **spawn-probe seam**
  (`SpawnProbe`): binary lookup plus `env_var(name) -> optional string`. The
  system implementation reads the process environment; the test fake carries
  a settable env map defaulting to empty, so precondition tests inject
  environments without mutating the process.
- **`validate_model(model)`** — `model` is optional; raises a value-error if
  malformed for this backend; a `None` model is always valid. **Exit-code note:**
  `member create` translates this value-error to a **usage error (exit 2)** with
  the backend's message (§6.3). This is distinct from the broker/messaging
  value-errors of §7.2, which the CLI wraps to **exit 1** — do **not** route a
  `validate_model` failure through the generic value-error→exit-1 path.
- **`validate_effort(effort)`** — `effort` is optional; raises a value-error if
  the level is not acceptable to this backend; a `None` effort (flag omitted)
  is always valid. Backends without a reasoning-effort control reject every
  non-None value. Same exit-code note as `validate_model`: `member create`
  translates the value-error to a **usage error (exit 2)** with the backend's
  message (§6.3).
- **`build_spawn_argv(prompt, display_name, model, effort)`** —
  returns the full argv vector (binary + flags + prompt) for the multiplexer's
  window-split.

**Ordering invariant:** the consumer (`member create`) MUST call them in the
order **`validate_model` → `validate_effort` → `ensure_available` →
`build_spawn_argv`**, so a malformed model or effort fails before any
precondition check or registration side effect.

**No-model byte-identity:** when `model` is `None`, `build_spawn_argv` emits
**no** `--model` tokens at all — the argv is identical to the no-model form.
Never emit an empty `--model ""`. When `effort` is `None`, no effort tokens
are emitted; the argv is byte-identical to the no-effort form. A non-`None`
invalid level never reaches argv construction — `validate_effort` rejects it
first (exit 2, before any side effect).

#### Registry resolution

A single module-level registry maps backend name → backend singleton, eagerly
constructed, with keys matching each supported backend’s name.
Resolution is a direct lookup keyed by the persisted `placement.coding_agent`
value — no fuzzy matching, no default fallback; an unknown name has no entry.
Each entry's key equals that backend's own `name` property.

#### Per-backend `build_spawn_argv` (exact, token-by-token)

**claude** — `validate_model` pass-through (accepts any string; the binary
validates). `validate_effort` enum check over the module-level
`EFFORT_LEVELS = ("low", "medium", "high", "xhigh", "max")`; an unknown level
raises the claude effort value-error (see § Contract error strings).
`ensure_available` PATH check on `claude` only. claude is the
**only** backend that honors `display_name` (via `--name`).

```
["claude", "--permission-mode", "dontAsk", "--prompt-suggestions", "false", "--allowedTools", <rule>...,
 "--name", <display_name>]
  (+ ["--model", <model>]  if model is not None)
  (+ ["--effort", <effort>]  if effort is not None)
  (+ <prompt>)                                       # bare trailing positional
```

The `--allowedTools` values are session allow rules for the member's broker
commands, one argv token per rule, so a claude member does not depend on any
`Bash(cafleet ...)` rule in the user's `settings.json`:

| Member | Rules, in order |
|---|---|
| Every claude member | `Bash(cafleet message *)` |

`--allowedTools` takes several values, so it sits before `--name`, which ends
its value list and keeps the prompt positional. A `deny` or `ask` rule at any
settings level still blocks the command, and the managed setting
`allowManagedPermissionRulesOnly` makes Claude Code ignore `--allowedTools`;
`doctor` reports both (§6.3 *`doctor`*). A blocking `PreToolUse` hook is
outside what `doctor` can evaluate.

**codex** — `validate_model` pass-through. `validate_effort` enum check over
the module-level `EFFORT_LEVELS = ("minimal", "low", "medium", "high",
"xhigh")`; an unknown level raises the codex effort value-error (see
§ Contract error strings). `ensure_available` PATH check on
`codex` only. `display_name` is silently ignored.

```
["codex", "--ask-for-approval", "never", "--sandbox", "workspace-write"]
  (+ ["--model", <model>]  if model is not None)
  (+ ["--config=model_reasoning_effort=<effort>"]  if effort is not None)  # ONE token
  (+ <prompt>)                                       # bare trailing positional
```

**opencode** — `validate_model`: `None` is valid; otherwise split `model` on the
**first** `/` into `<provider-id>` and `<model-id>`, both halves MUST be
non-empty, else value-error `--model for the opencode backend must be
'<provider-id>/<model-id>' (got '{model}').`. (`"openai/gpt-4"` accepted;
`"a/b/c"` accepted as provider `a` / model `b/c`; `"a/"`, `"/b"`, `"abc"`
rejected.) `validate_effort` rejects **every** non-None value with the
opencode effort value-error (see § Contract error strings) — the backend has
no reasoning-effort control, so `build_spawn_argv` never receives a non-None
`effort` and never emits effort tokens (it asserts `effort is None`).
`ensure_available` PATH check on `opencode` **first**, then verify
the preset file exists at the resolved preset path `<preset
base>/agents/cafleet.md` — `<preset base>` resolved through the probe's
`env_var` lookup per §6.3 *Config-dir resolution* (`OPENCODE_CONFIG_DIR`,
default `~/.opencode`); an invalid variable surfaces the pinned validation
error before the existence check (see § opencode
preset). `display_name` is ignored; the prompt is
passed as a `--prompt <prompt>` flag pair (two tokens), unlike claude/codex's
bare positional.

```
["opencode", "--agent", "cafleet"]
  (+ ["--model", <model>]  if model is not None)
  (+ ["--prompt", <prompt>])                         # prompt via flag — TWO tokens
```

#### codex rules file

The codex auto-approval rules for `cafleet` commands are a static file embedded
in the binary (source `presets/codex/cafleet.rules`) and installed to
`<codex home>/rules/cafleet.rules` — `<codex home>` resolved per §6.3
*Config-dir resolution* (`CODEX_HOME`, default `~/.codex`) — by the assets
half of `setup`
(§6.3), overwriting any existing target. The file is not a spawn precondition —
codex's `ensure_available` is PATH-check-only — and codex loads every `*.rules`
file under its rules directory, applying the strictest matching decision, so
operator customizations live in a separate rules file in that directory. Exact
contents (verbatim):

```text
prefix_rule(pattern = ["cafleet"], decision = "allow")

prefix_rule(
    pattern = ["cafleet", "member", "prompt"],
    decision = "prompt",
    justification = "cafleet member prompt keystrokes arbitrary text into a member pane",
)

prefix_rule(
    pattern = ["cafleet", "member", "exec"],
    decision = "prompt",
    justification = "cafleet member exec runs an arbitrary shell command in a member pane",
)
```

Codex applies the strictest matching decision, so `member prompt` and
`member exec` keep requiring approval while every other subcommand matches
only the broad allow.

#### opencode preset

The `cafleet` agent definition is a static file embedded in the binary
(source `presets/opencode/cafleet.md`) and installed to
`<preset base>/agents/cafleet.md` — `<preset base>` resolved per §6.3
*Config-dir resolution* (`OPENCODE_CONFIG_DIR`, default `~/.opencode`) — by
the assets half of `setup`
(§6.3), overwriting any existing target. **Two opencode base directories serve
two distinct purposes** and are not interchangeable: the agent preset lives
under the resolved preset base (default `~/.opencode/`), which is opencode's
`--agent cafleet` discovery path (`agents/` is in `OPENCODE_CONFIG_DIR`'s
documented search list, so a relocated preset remains discoverable);
`setup`'s skills install (§6.3) targets the fixed `~/.config/opencode/`,
cafleet's
own skills-install target for the opencode agent.

The preset is a spawn precondition (the spawn argv references `--agent
cafleet`): opencode's `ensure_available` verifies the file exists at the
resolved install target and raises `opencode agent preset not found at
{preset}; run 'cafleet setup --coding-agent opencode' first` when it does
not — the remedy names the selector as the targeted repair; plain
`cafleet setup` also installs it, as it covers all three agents.

#### Exact preset file contents (verbatim)

The checked-in `presets/opencode/cafleet.md` is the content contract: a
`---`-delimited **JSON** (not YAML) frontmatter block (2-space indent,
non-ASCII preserved, top-level key order `description`, `mode`, `permission`),
a blank line, then the markdown body. `bash`/`read`/`edit` are glob→decision
maps (`"allow"`/`"deny"`); the other seven `permission` fields are scalar
`"deny"`. Reproduce this file faithfully (the body contains literal backticks
around command names and `.env`):

````markdown
---
{
  "description": "CAFleet-spawned member with a deny-by-default bash allowlist derived from the operator's Claude Code permission set.",
  "mode": "primary",
  "permission": {
    "bash": {
      "*": "deny",
      "git add *": "allow",
      "git commit *": "allow",
      "git diff *": "allow",
      "git grep *": "allow",
      "git log *": "allow",
      "git ls-tree *": "allow",
      "git ls-files *": "allow",
      "git branch *": "allow",
      "git status": "allow",
      "grep *": "allow",
      "ls": "allow",
      "ls *": "allow",
      "stat *": "allow",
      "tree": "allow",
      "tree *": "allow",
      "mise //cafleet:test": "allow",
      "mise //cafleet:test *": "allow",
      "mise //cafleet:lint": "allow",
      "mise //cafleet:format": "allow",
      "mise //cafleet:typecheck": "allow",
      "mise //cafleet:build": "allow",
      "wc *": "allow",
      "cafleet *": "allow",
      "cafleet member prompt *": "deny",
      "cafleet member exec *": "deny",
    },
    "read": {
      "*": "allow",
      "**/.env": "deny",
      "**/.env.*": "deny"
    },
    "edit": {
      "*": "allow",
      "**/.env": "deny",
      "**/.env.*": "deny"
    },
    "external_directory": "deny",
    "webfetch": "deny",
    "websearch": "deny",
    "repo_clone": "deny",
    "question": "deny",
    "plan_enter": "deny",
    "plan_exit": "deny"
  }
}
---

# CAFleet member agent

You are a CAFleet member spawned by the Director. The frontmatter is the literal tool-permission map: Bash defaults to deny and allows its listed patterns, including `cafleet *`, the listed Git commands, inspection utilities and CAFleet mise tasks. Read/edit permit their listed paths with `.env` exclusions; external-directory access and the other named tools retain their declared denials. A denied command receives no permission prompt.

Your role and the Director's assignment determine which permitted tools you may use for this task. Broad tool patterns do not grant Director-only actions or Git write authority to an ordinary member. Follow the CAFleet member protocol and route a genuinely needed denied command through prompt-routing after reconsidering it. Read the Director's spawn instructions for your assigned scope.

Perform assignments with the selected provider/model and permitted workspace tools. Poll after ready, before reports and on every wake; ACK consumed messages and handle queued work before becoming idle. The Director owns prompt/native command dispatch and inspects native output before explicit continuation when needed.
````

The body has three paragraphs after the heading, separated by a blank line, with
no internal hard line breaks in either paragraph. The file ends with exactly
one trailing newline.

#### Contract error strings

- PATH miss: `binary {binary_name} not found on PATH`
- opencode model format: `--model for the opencode backend must be
  '<provider-id>/<model-id>' (got '{model}').`
- claude effort level: `--effort for the claude backend must be one of low,
  medium, high, xhigh, max (got '{effort}').`
- codex effort level: `--effort for the codex backend must be one of minimal,
  low, medium, high, xhigh (got '{effort}').`
- opencode effort: `opencode does not support reasoning effort.`
- missing opencode preset: `opencode agent preset not found at {preset}; run
  'cafleet setup --coding-agent opencode' first`
- invalid config-path variable: `{var} must be an absolute path (got
  '{value}')` (§6.3 *Config-dir resolution*)

#### Host evidence and native-input authorization

**claude**: A separate evaluator checks the session condition against the transcript (Claude Code 2.1.296, checked 2026-10-22). Native background work defers evaluation and returns a host result; external panes supply evidence when the Director reads broker/pane state. An unmet condition can immediately start another turn without native background work; consecutive tool-free stops reach the configured block cap, eight by default. Retain a finite receive task handle while external work remains.

Native composer controls act at the beginning of submitted input. A local probe merged control text into a concurrent preview, so serialize complete input and inspect capture evidence. Native shell probes displayed command/stdout/stderr followed by a model turn; the trigger and any second permission check were not established. The Director authorizes each routed command under its host rules.

**codex**: The objective belongs to the thread, with model-callable lifecycle tools and optional token-budget accounting (Codex CLI 0.162.1, checked 2026-10-22). A two-turn tool-bearing probe observed continuation and model-reported completion. Preserve the user's objective, activation authority and budget; retain live tool/session handles and collect explicit broker/pane evidence.

A native shell probe displayed the actual command, exit code, duration and output. Installed protocol metadata describes unsandboxed dispatch while public CLI guidance describes current approval/sandbox settings; the TUI's effective boundary and protocol path were not established. Treat native input as a Director-authorized action rather than assume a second member-tool permission check.

**opencode**: OpenCode members perform supplied assignments with their selected provider/model, workspace reads/edits, permitted tools and broker communication. The Director coordinates next steps and routed native shell commands. The TUI documents leading `!` input and conversation-visible output; an actual OpenCode native-input permission probe was outside the investigation. Preserve the preset's member-tool permissions and the Director's command authorization.



### 6.8 Configuration

Settings retain the selected SQLite URL, nonnegative text limit and optional multiplexer override. The process reads only the explicitly named environment variables in §6.3. Spawned member panes inherit `CAFLEET_DATABASE_URL`; identity is rendered into prompt literals.

## 7. Cross-cutting concerns

### 7.1 Configuration & environment

Expand HOME only for the default `sqlite:///` URL ending in `cafleet_v8.db`. Explicit `sqlite:///<path>` overrides preserve their path text; relative file paths resolve against the invoking working directory before canonical database-file identity supplies shared pane locking. Validate supplied values and required state at their boundary. Config-directory relocation is lazy per selected backend except guards that deliberately inspect every supported asset identity.

### 7.2 Error handling & exit-code policy

Usage/parser/model/effort/line-validation errors exit 2. Application, database and transport errors exit 1. Successful finite timeout and ping skips exit 0. Top-level application errors use `Error: MESSAGE` on stderr. Preserve primary causes alongside explicit cleanup failures. Missing required state or unknown enum/column values propagate instead of substituting meaningless defaults.

### 7.3 Output / JSON / truncation

JSON uses explicit ordered presenters and unescaped UTF-8. Preserve full message bodies and capture bytes selected for presentation. Text truncates only message bodies by Unicode codepoint limit; exact deliberate error and layout strings remain contractual.

### 7.4 Logging & stdout discipline

Successful command records stay on stdout. Post-commit notification diagnostics and creation compensation failures go to stderr. A committed message retains successful output even when its preview cannot be delivered; an uncertain native command prints no success record.

### 7.5 Time discipline

Use UTC for durable timestamps, monotonic clocks for budgets and deadlines. Cap every bounded process/database operation to remaining time. Every guard/owned child releases resources on ordinary errors and process exit.

### 7.6 Packaging & distribution

Ship one `cafleet` binary per release target. Archives are `cafleet-v<version>-<target>.tar.gz` containing the binary; the release tag is the bare package version. The compile-time version owns `--version`, asset stamps and stale checks. Embed migrations, skills and presets offline. Rust builds/release packaging require no frontend output. The documentation site retains its independent Node/pnpm tooling.

### 7.7 Agent-operation contract — one-shot command isolation

Each CAFleet process is the only command in its shell-tool invocation. Run sequences as separate calls. Leading `NAME=value` directly before the executable is valid. Long bodies use `--file PATH`. On permission failures, first verify isolation, then route a still-required denied command to the Director under its host rules. Preserve existing committed message IDs instead of creating duplicates.

## 8. Database schema

The baseline is version 1 in the current distinct format family. The fresh baseline creates exactly the application tables declared in the schema. Compatibility diagnosis verifies current application identity, ledger version, every required table’s column shape, and required indexes/foreign keys. Known versions use their recorded migration shape; newer versions are diagnosed before any head-shape comparison. Unrelated auxiliary tables, indexes and triggers are compatible while those required definitions remain intact. Exact DDL:

```sql
PRAGMA application_id = 1128351288;

CREATE TABLE members (
    member_id INTEGER NOT NULL PRIMARY KEY AUTOINCREMENT,
    fleet_id INTEGER NOT NULL REFERENCES fleets (fleet_id) ON DELETE RESTRICT,
    name TEXT NOT NULL,
    description TEXT NOT NULL,
    status TEXT NOT NULL,
    registered_at TEXT NOT NULL,
    deregistered_at TEXT
);
CREATE INDEX idx_members_fleet_status ON members (fleet_id, status);

CREATE TABLE fleets (
    fleet_id INTEGER NOT NULL PRIMARY KEY AUTOINCREMENT,
    name TEXT,
    created_at TEXT NOT NULL,
    deleted_at TEXT,
    director_member_id INTEGER REFERENCES members (member_id) ON DELETE RESTRICT
);

CREATE TABLE asset_installs (
    coding_agent TEXT NOT NULL,
    path TEXT NOT NULL,
    cafleet_version TEXT NOT NULL,
    installed_at TEXT NOT NULL,
    PRIMARY KEY (coding_agent, path)
);

CREATE TABLE member_placements (
    member_id INTEGER NOT NULL PRIMARY KEY REFERENCES members (member_id) ON DELETE CASCADE,
    mux_session TEXT NOT NULL,
    mux_window_id TEXT NOT NULL,
    mux_pane_id TEXT,
    backend TEXT NOT NULL DEFAULT 'tmux',
    coding_agent TEXT NOT NULL,
    created_at TEXT NOT NULL
);

CREATE TABLE messages (
    message_id INTEGER NOT NULL PRIMARY KEY AUTOINCREMENT,
    from_member_id INTEGER NOT NULL REFERENCES members (member_id) ON DELETE RESTRICT,
    to_member_id INTEGER NOT NULL REFERENCES members (member_id) ON DELETE RESTRICT,
    created_at TEXT NOT NULL,
    status_state TEXT NOT NULL,
    status_timestamp TEXT NOT NULL,
    text TEXT NOT NULL,
    notified_at TEXT
);
CREATE INDEX idx_messages_to_member_status_ts ON messages (to_member_id, status_timestamp);
CREATE INDEX idx_messages_from_member_status_ts ON messages (from_member_id, status_timestamp);
```

## 9. Testing strategy

Exercise current schema/identity and incompatible-file preservation; atomic registration and creation compensation; point-to-point/ACK/pending notification behavior; finite receive deadlines, round-robin retries and diagnostic clearing; process-shared exclusion and process-death release; styled empty/decorated/draft/approval/unknown composer fixtures; literal native command visibility and partial uncertainty; explicit continuation; backend/multiplexer smoke coverage; links, embedding and release builds. Preserve general timed pipe draining and owned-child cleanup coverage.

Run repository validation through full-path mise tasks. Report meaningful current test results and production/documentation size reductions excluding generated, vendored, lockfile and design/research evidence.

## 10. CLI command checklist

The complete current command registry, subjects, options and outputs are specified in §6.3. Check each registered command’s parse validation, guards, text/JSON shape and exit category against that owner.

## 11. Decisions & clarifications

| Decision | Owner |
|---|---|
| Host continuation and Director receiving | §1–2 and §6.3 |
| Typed records, recipient ownership and timestamps | §5 and §8 |
| Registration/compensation and explicit pane actions | §6.2–3 |
| Full JSON, truncation and capture integrity | §6.4 |
| Stable input exclusion and readiness | §6.5–6 |
| Backend argv, presets and evidence boundaries | §6.7 |
| Configuration, errors and release packaging | §7 |
