# CAFleet — Reimplementation Specification

This is the self-contained interface and observable-behavior specification for the `cafleet` CLI, broker, independent coding-agent sessions and embedded assets. Exact authored errors, JSON fields/order and command layouts govern; incidental host-language quoting and operating-system error suffixes preserve intent. Complete module contracts appear in §6 Module specifications.

## 1. Overview & goals

CAFleet multiplexes coding-agent sessions in tmux or herdr and coordinates them with durable point-to-point SQLite messages. The host owns task continuation and native execution results; the Director owns delegation, wake handling, evidence inspection, authorization and cleanup. The shipped product consists of the CLI, embedded skills/presets and documentation site.

Reproduce the documented command/option/exit surface, configuration defaults, current schema, enum values, output shapes, lifecycle, finite receive, notification, locking, termination deferral and native dispatch semantics. The current format identity defines compatibility; required data corruption surfaces at its read boundary.

## 2. Architecture stance

The core is synchronous. A send previews pending ordinary-member messages within its budget; finite receive owns bounded waits and notification retries. A persistent read-only listener emits pending inbox records through JSONL without pane operations. Separate processes coordinate SQLite changes through transactions and complete pane submissions through process-shared file locks. Host-native tools own receiver handles and collect native results.

A Director arms native reception before relying on replies and retains one receiver while authorized work remains, preserving supplied goal controls and user decision gates. Consumed deliveries are explicitly ACKed; deferred actions and native handles remain in host-owned state. Every received batch/wake triggers broker and artifact evidence checks. Automatic Director reception sends no Escape, text, Enter or queued composer input.

## 3. Module layout

| Concern | Responsibility |
|---|---|
| Configuration | Selected database, text limit and multiplexer override |
| Persistence | Connection factory, format/shape diagnosis and embedded migrations |
| Broker | Typed fleet/member/message/asset records and synchronous transactions |
| Output | Ordered wire presenters, truncation and capture formatting |
| Multiplexer | Discovery, pane creation/capture/input/kill and layout compensation |
| Pane input | Stable OS locks, runner-state gating and durable receipt settlement; explicitly unwrapped input retains Escape-first literal submission |
| Member runtime | Owned PTY transport, typed observations, durable event/inbox transactions and identity-pinned recovery |
| Notification / receiving | Ordinary-member draining delivery, caller-owned finite retries and read-only persistent inbox streams |
| Coding-agent backends | Registry, availability, model/effort validation and spawn argv |
| CLI | Parse/guard/compose these contracts and emit results |

## 4. Architecture & module dependency graph

```text
CLI -> receiving / notification -> broker -> persistence
       pane input -> multiplexer -> bounded process runner
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

`MemberStatus` is `active` / `deregistered`; `MessageStatus` is `input_required` / `completed`. Unknown required enum strings fail. Supported backend tokens come from their registries.

### 5.4 Member kind discriminator

Derive `director` exactly when `fleets.director_member_id == members.member_id`, otherwise `member`. A live fleet requires a valid active Director and placement.

### 5.5 Recipient and pending placement

Every message has an integer `to_member_id` identifying its inbox owner. Nullable `mux_pane_id` represents pending member creation and documented diagnostics; required concrete pane actions fail contextually until it is patched.

### 5.6 Typed records and wire presenters

Broker records carry typed values independently of CLI formatting. Public message records are ordered `message_id, from_member_id, to_member_id, created_at, status_state, status_timestamp, text`. Full JSON remains untruncated; internal notification state stays in storage.

## 6. Module specifications

### 6.1 Persistence & Schema

Schema compatibility and the application identity are specified in §8 Database schema.

Connections use SQLite foreign-key enforcement and a normal five-second busy timeout. A bounded operation caps that wait to its remaining budget. Resolve a relative selected SQLite file path against the invoking working directory and canonicalize the database file before lock-key directory construction.

Validate database identity, ledger version and required table/column shapes for the recorded supported version before command writes and before setup installs assets. An invocation's validated connection serves its internal send, ACK and notification stamp operations; standalone public broker write functions retain their own format guard. Schedule schema upgrades between active command invocations. Empty zero-identity files initialize with the baseline atomically. Current-family known older versions migrate; an ahead ledger version fails setup before database mutation, asset installation or asset-install recording. Other populated/identified or malformed files fail without content writes:

`database format is incompatible at 'PATH'; choose an empty database path and run 'cafleet setup'.`

Preserve database open/read causes and incompatible file bytes/sidecars. Doctor reports classification while completing other read-only diagnostics. Setup and invocation guards use this same classifier.

### 6.2 Broker

#### Fleets

Bootstrap validates the calling multiplexer context and backend token, then inserts fleet, Director and placement and backfills the Director reference in one transaction. The current session remains the Director; no pane split or process launch occurs.

List live fleets by descending creation timestamp/ID. Show includes soft-deleted fleets. Delete locks/revalidates active placements, soft-deletes the fleet and deregisters its members/removes placements in one transaction, preserving messages. Repeat deletion returns zero newly deregistered members.

#### Members

Registration requires a live fleet and active valid root Director. Insert the member and pending placement atomically. Creation owns compensation for later prompt/split/patch failures. Root Director individual deletion fails. Member deletion locks its concrete pane, kills it and deregisters it; a placementless/pending member requires no concrete pane lock.

Roster reads active members of live fleets, derive kind from the fleet reference and include nullable activity. Activity is the maximum sent, received and ACK timestamp; `idle` is the elapsed seconds since the latest known activity, truncated to an integer, and null is the correct state without activity. Detailed views retain registration, description and placement data. Placement mutations acquire the same pane input locks as input writers. Required Director/placement/enum corruption fails rather than producing placeholder values.

#### Messaging

Send requires active sender/recipient in the same live fleet and persists exact text. Every individual fan-out send commits independently; keep already committed IDs when another recipient fails. Poll/wait read incoming unread messages by descending status timestamp/message ID. Show requires existence. ACK requires an unread existing row, stamps completion and removes notification eligibility. Consumption is independent of assignment completion.

Pending preview selection chooses the oldest eligible row by `(created_at, message_id)` for an active ordinary recipient with a concrete pane; root Directors and self-sends are ineligible. Revalidate ordinary-member kind, fleet, placement, message status and null notification stamp while holding pane input ownership. Stamp success conditionally only after complete submission; delivery leaves `status_state` unchanged until explicit ACK. Listening selects every pending record independently of notification fields, including stamped pending self-sends, without changing broker state.

#### Broker errors

Preserve the validation boundary and explicit cause. Missing member/message, inactive fleet/Director, cross-fleet pair and already-ACKed rows fail with application errors; §6.3 CLI *Error Messages* lists their exact strings.

### 6.3 CLI

#### Subcommand summary

| Command | Purpose | Subject | JSON |
|---|---|---|---|
| `setup` | Initialize/migrate the current database and install assets | — | no |
| `doctor` | Diagnose multiplexer, database, agent assets and member runtimes | — | yes |
| `fleet create` | Register the calling Director atomically | — | yes |
| `fleet list` / `fleet show` | Read fleet registry | `FLEET_ID` for show | yes |
| `fleet delete` | End a fleet's ordinary members, then deregister the fleet | `FLEET_ID` | yes |
| `member create` | Register and spawn an independent agent | `--fleet-id` relationship | yes |
| `member delete` | Kill its owned pane and deregister | `MEMBER_ID` | yes |
| `member show` / `member list` | Read detail / active roster | `MEMBER_ID` / `FLEET_ID` | yes |
| `member prompt` / `member exec` | Submit authorized native input | `MEMBER_ID` | yes |
| `member press` | Request explicit ordered keys | `MEMBER_ID` | yes |
| `member capture` | Read a pane tail | `MEMBER_ID` | yes |
| `member recover` | Reconcile the recorded run without restarting an agent | `MEMBER_ID` | yes |
| `event show` | Read a durable runtime event and current state | `EVENT_ID` | yes |
| `input show` / `input reconcile` | Read / settle a durable input receipt | `INPUT_ID` | yes |
| `permission inspect` / `permission respond` | Inspect / answer the current native permission request | `INTERACTION_ID` | yes |
| `message send` | Persist a point-to-point message and preview it for an ordinary recipient | Sender/recipient flags | yes |
| `message poll` / `message wait` | Read incoming deliveries / block until deliveries arrive | `MEMBER_ID` | yes |
| `message listen` | Stream pending inbox records through stdout | `MEMBER_ID` | JSONL only |
| `message ack` / `message show` | Consume / read one message | `MESSAGE_ID` | yes |

#### Environment variables

| Variable | Field | Default | Meaning |
|---|---|---|---|
| `CAFLEET_DATABASE_URL` | `database_url` | `sqlite:///` + `~/.local/share/cafleet/cafleet_v8.db` | Selected SQLite file path |
| `CAFLEET_MULTIPLEXER` | `multiplexer` | Auto-detect | Explicit supported multiplexer override |
| `CAFLEET_MAX_TEXT_LEN` | `max_text_len` | `200` | CLI text-output body codepoint limit |

The default expands HOME once. An explicit URL has the form `sqlite:///<path>`; resolve a relative file path against the invoking working directory before canonicalizing the database file for shared pane-lock identity. A literal tilde in an override is not expanded. Count values accept nonnegative integers and invalid values fail with `<NAME> must be a non-negative integer (got '<value>')`.

#### Global Options

`cafleet --version` prints `cafleet <version>` and exits 0 before dispatch.

#### Help text

`--help` describes every visible command, subcommand and option with a one-line description. Command group descriptions state the purpose given in the subcommand summary. Help derives choice lists from the same registries that validate the values:

| Option | Help shows | Source |
|---|---|---|
| `member create --permission-mode MODE` | Each backend's accepted modes and its omitted default, in backend registry order, as `BACKEND: MODE, MODE (default MODE)` joined by `; ` | The backend policy catalog (§6.9 Launch policy and capability registry) |
| `permission respond --decision`, `--decision-source`, `--director-mode` | Their possible values | The stored enum values of the permission response storage (§6.9 Permission response storage) |
| `input reconcile --disposition`, `member create --event-evidence` | Their possible values | Their command enums |

An invalid value for an option with possible values exits 2 with a usage error that lists those values. `member create` resolves its backend at run time, so a `--permission-mode` value outside that backend's catalog keeps its run-time usage error `--permission-mode for BACKEND must be one of CHOICES (got 'VALUE')`, also with exit 2. Help reads `claude: default, manual, dontAsk, acceptEdits, auto, bypassPermissions, plan (default auto); codex: on-request, never, approve-for-me (default approve-for-me); opencode: cafleet, cafleet-ask (default cafleet)`. For codex the error reads `--permission-mode for codex must be one of on-request, never, approve-for-me (got 'VALUE')`, and for opencode `--permission-mode for opencode must be one of cafleet, cafleet-ask (got 'VALUE')`.

#### Output shapes

| Command | Text | JSON |
|---|---|---|
| `fleet create` | `<fleet-id> director=<member-id>` | Ordered `fleet_id, name, created_at, director`, with Director identity/description/registration/placement |
| `fleet list` | `FLEET_ID DIRECTOR NAME MEMBERS CREATED_AT` columns | Fleet records |
| `fleet show` | Fleet row with `deleted_at:` | Fleet including nullable `deleted_at` |
| `fleet delete` | `Deleted fleet ID. Deregistered N members.` plus one line per ordinary member | `{fleet_id, deregistered_count, members}` |
| `message send` | `Message sent.` then envelope, delivery line and any input line | `{message, notification_sent, input_id, delivery}` |
| `message poll` | Envelopes, or `No messages found.` | Full record array |
| `message ack` | `Message acknowledged.` then envelope | `{message}` |
| `message show` | Envelope | `{message}` |
| `message wait` | Availability/timeout line and any messages | `{member_id, outcome, messages, notification_errors}` |
| `message listen` | — | Tagged `ready`, `message`, `closed` JSONL records; no `--json` flag |
| `member create` | `ID NAME backend=AGENT pane=PANE` plus runtime line | Member with placement and runtime |
| `member delete` | `Member deleted.` with member/pane lines | `{member_id, pane_status}` |
| `member show` | `ID NAME STATUS` plus wrapped runtime line | Detail including kind, placement and runtime |
| `member recover` | Recovery counts/state and cleanup lines | Ordered recovery result |
| `event show` | Event, runtime and optional interaction lines | Event and current state |
| `input show` / `input reconcile` | Input receipt and optional decision lines | `{input}` |
| `permission inspect` | Request identity/capability lines | `{request}` |
| `permission respond` | Answer submission status/stage line | `{response}` |
| `member list` | Roster rows or `0 members.` | Roster array |
| `member prompt` | `Sent prompt TEXT_REPR to member NAME (PANE).` or the queued line, plus delivery and input lines | `{member_id, pane_id, text, input_id, delivery}` |
| `member exec` | `Dispatched native command to member ID (PANE). Inspect the pane for its result.` or the queued line, plus delivery and input lines | `{member_id, pane_id, coding_agent, command, dispatched, input_id, delivery}` |
| `member press` | `Acknowledged keys KEYS_JSON for member ID (PANE).` | Ordered `member_id, pane_id, keys, status` |
| `member capture` | Exact selected content | `{member_id, pane_id, lines, content, captured_at, content_sha256}` |
| `doctor` | One `STATUS NAME: DETAIL` line per check | `{checks}` |

##### Member list output

Roster records include `member_id`, `name`, `kind`, `placement`, `last_sent`, `last_recv`, `last_ack` and `idle`. `kind` is `director` when the fleet reference matches, otherwise `member`. Missing activity is null; placement may be null, or its pane ID pending. Text uses `-` for absent values and `(pending)` for a pending pane.

#### JSON output (`--json`)

Place `--json` among the subcommand options, before a `--` separator that introduces literal key arguments. It emits compact single-line JSON with UTF-8 and complete untruncated message text. Application errors remain stderr text, with the structured permission-response exception (§6.3 Permission commands); parse errors exit 2. Commands marked no reject the flag, including the root group.

#### Positional subject ids

Place the command’s subject ID immediately after its subcommand; event, input and permission subjects use their positive durable event, input and interaction IDs. The subject row supplies related fleet/recipient state. IDs are complete DB-assigned integers, without prefix resolution or environment defaults. A missing or malformed subject uses the parser's native error, exit 2.

Spawned members read literal `FLEET ID:`, `YOUR MEMBER ID:`, `DIRECTOR MEMBER ID:` and `CODING AGENT:` lines. Pass these literal IDs on each invocation.

#### Sender and recipient (`--from-member-id`, `--to-member-id`)

Send requires integer sender and recipient flags. Both must be active in the same live fleet. The sender row determines the fleet; IDs are coordination identifiers within the local trust model.

#### `permissions.allow` coverage

Use prefix rules such as `Bash(cafleet message *)` to cover broker operations, including `cafleet message listen ID --timeout 300` and finite `cafleet message wait`. Verify native Monitor authorization separately: a Bash-rule check establishes no Monitor permission. Literal IDs and trailing JSON flags preserve prefix matching.

Keep commands that send explicit native pane input under `permissions.ask`, including the `Bash(cafleet member prompt *)`, `Bash(cafleet member exec *)` and `Bash(cafleet member press *)` prefixes. Ask rules outrank allow rules. Authorize `cafleet permission respond` under the Director host’s effective policy after its request-specific decision; mode/source flags bypass no ask, deny or managed rule. Ordinary member role authority covers broker reception and evidence routing, while the Director owns permission responses. The recommended permission profiles (§6.7 Claude permission posture) describe each role's needs.

#### Message Body Truncation

CLI text bodies use `CAFLEET_MAX_TEXT_LEN` Unicode codepoints followed by `…` when truncated. Full JSON and pane captures preserve their complete content. Pane notification lines follow their own inline-body limit (§6.5 Push notifications). §6.4 Rendered shape owns the compact rendering.

#### Text bodies (positional `TEXT` / `--file`)

Send and member creation require exactly one positional body or `--file PATH`; `--file -` reads UTF-8 stdin. Preserve exact nonblank body text. Missing/both sources or a blank positional body exit 2; blank file/stdin sources and file-read errors exit 1 with the authored diagnostics below. File read/UTF-8 failures retain the actual cause. Compact role-by-path spawn prompts remain necessary because resolved prompts enter backend/multiplexer argv.

`member prompt` takes one positional line. Native exec has the separate command contract (§6.3 `member exec`).

#### `cafleet setup` — Onboarding and Schema Management

`cafleet setup [--coding-agent AGENT...]` prepares the database first, then installs assets. It validates the selected database format before any write: an empty or missing file is initialized with the embedded baseline in one transaction, a known older current-family version migrates to head, and an ahead ledger version or incompatible shape fails before database mutation, asset installation or asset-install recording. Compatible setup then installs embedded assets offline and records versions. Backend selection is multi-value/repeatable, deduplicated and defaults to the supported backend registry; selected backends install in registry order.

After migrating, setup prints `Applied migrations to head ({head}) at {path}.`; a database already at head prints `Already at head ({head}).`. Asset installation stops at the first backend whose config-dir resolution or installation fails: backends earlier in registry order keep their installation and record, and setup exits 1 with that backend's cause.

##### Config-dir resolution

Every surface that installs to or checks a backend's user-level config
directory resolves it through the backend's native config-location
environment variable, falling back to the default when the variable is unset:

| Backend | Variable | Base when set | Base when unset | Skills dir | Preset target |
|---|---|---|---|---|---|
| claude | `CLAUDE_CONFIG_DIR` | `$CLAUDE_CONFIG_DIR` | `~/.claude` | `<base>/skills` | — |
| codex | `CODEX_HOME` | `$CODEX_HOME` | `~/.codex` | `<base>/skills` | `<base>/rules/cafleet.rules` |
| opencode (skills) | — (fixed discovery path) | — | `~/.config/opencode` | `<base>/skills` | — |
| opencode (preset) | `OPENCODE_CONFIG_DIR` | `$OPENCODE_CONFIG_DIR` | `~/.config/opencode` | — | `<base>/agents/cafleet.md`, `<base>/agents/cafleet-ask.md` |

opencode discovers skills only at fixed paths but loads agent definitions from
`OPENCODE_CONFIG_DIR` as well, so only the presets follow the variable.

**Validation.** A set variable must hold an absolute path. Any other value —
the empty string, a relative path, a literal unexpanded `~/…` — fails at
resolution time with exit 1:

```
Error: <VAR> must be an absolute path (got '<value>')
```

Validation is lazy: a variable is read and validated only when a site
actually resolves that backend's directory. `cafleet setup --coding-agent
claude` with an invalid `CODEX_HOME` succeeds because the selector resolves
only the targeted agent's directory; plain `cafleet setup` resolves each
backend's directory in registry order, so an invalid variable stops
installation at that backend. `member create` resolves only the selected
backend's directory during its availability check and forwards that
validated value into the member environment (§6.3 Member environment).
`doctor` reports a per-agent resolution error as that agent's
`<agent>.configuration` check instead of aborting (§6.3 `cafleet doctor` — Environment Diagnosis).

**Recorded-path identity.** Every surface that keys on "the agent's resolved
path" — the `asset_installs` rows and `doctor`'s installation checks — uses
one canonical path per agent, the resolved base directory, stored absolute
exactly as resolved (no canonicalization beyond the absolute-path
validation):

| Agent | Recorded / status-keyed path |
|---|---|
| claude | The resolved claude config dir (`$CLAUDE_CONFIG_DIR` or `~/.claude`) |
| codex | The resolved codex home (`$CODEX_HOME` or `~/.codex`) |
| opencode | The resolved preset base (`$OPENCODE_CONFIG_DIR` or `~/.config/opencode`) — the only opencode root that can vary; the skills base is fixed and carries no identity |


##### Asset installation

Each agent's presets, where they exist, are static files embedded in the
binary next to the skills; install targets resolve per
§6.3 Config-dir resolution. Setup installs every embedded
preset of the agent, in this order:

| Agent | Bundled preset | Install target |
|---|---|---|
| claude | — (skills only) | — |
| codex | `presets/codex/cafleet.rules` | `<codex base>/rules/cafleet.rules` (default `~/.codex/rules/cafleet.rules`) |
| opencode | `presets/opencode/cafleet.md` | `<preset base>/agents/cafleet.md` (default `~/.config/opencode/agents/cafleet.md`) |
| opencode | `presets/opencode/cafleet-ask.md` | `<preset base>/agents/cafleet-ask.md` (default `~/.config/opencode/agents/cafleet-ask.md`) |

Each backend replaces every embedded skill sequentially, then replaces each of its presets in order. Existing targets
are deleted before their replacements are written. Unrelated skills remain
untouched. A symlink target is unlinked as an entry;
its referent is not deleted. Skills-operation errors retain `failed to install
skills into <skills_dir>: <error>` and preset-operation errors retain `failed
to install preset into <target>: <error>`. A successful install records one
`asset_installs` row for the agent, covering all of its presets; a failure on
any preset fails setup and records no installation.

The command prints each success line after that operation completes, then
records the installed version after all operations succeed: the skills line,
then one preset line per installed preset target in install order. Claude
prints only the skills line:

```
<agent>: installed cafleet, cafleet-design-doc (v<version>) -> <skills dir>
<agent>: installed preset (v<version>) -> <target>
```

`cafleet setup --coding-agent opencode` prints:

```
opencode: installed cafleet, cafleet-design-doc (v<version>) -> <skills dir>
opencode: installed preset (v<version>) -> <preset base>/agents/cafleet.md
opencode: installed preset (v<version>) -> <preset base>/agents/cafleet-ask.md
```


#### Shared diagnosis and connection reuse

Use the invocation’s shared classifier and connection for guard/setup work. Doctor completes its read-only checks even when the database is unavailable. Compatibility includes identity, ledger version and expected table/column shapes.

#### Schema-version guard

Every fleet/member/message command validates the current format (§8 Database schema) before writes. Malformed or incompatible files fail with `database format is incompatible at 'PATH'; choose an empty database path and run 'cafleet setup'.` Preserve file bytes and existing sidecars. Empty state reports `no cafleet database; run 'cafleet setup'`. Behind state reports `database schema is outdated (schema M, head N); run 'cafleet setup'`; ahead reports `database schema M is newer than this cafleet (head N); upgrade cafleet`. Database-open/read errors preserve their cause. Setup initializes/repairs compatible known versions; doctor reports rather than blocks its other diagnostics.

`--help` renders at parse time and exits before any command body runs, so
neither group-level help (`cafleet fleet --help`) nor subcommand help
(`cafleet fleet create --help`) triggers the guard — both always print
help, even under a missing database.

#### `cafleet fleet` — Fleet Management

##### `fleet create`

`cafleet fleet create --name NAME --coding-agent AGENT [--json]`

Require the selected backend token and a valid tmux/herdr context. Outside that context fail with `cafleet fleet create must be run inside a tmux or herdr session`. Insert fleet, current Director member and placement in one transaction and backfill `director_member_id` before commit. The Director has `name="Director"` and `description="Root Director for this fleet"`. Creation splits no pane and launches no process. An ordinary member can be created immediately.

The calling pane becomes the Director's concrete placement. Run `fleet create` from the coding-agent session declared by `--coding-agent`, then arm native message reception (§6.3 `message listen`) before depending on member replies. Root-Director messages remain in the durable inbox for native or manual reads; automatic pane previews target ordinary members.

##### `fleet list`

List non-deleted fleets by descending creation timestamp/fleet ID. Rows retain the Director reference so its ID remains discoverable.

##### `fleet show`

`cafleet fleet show FLEET_ID [--json]` includes soft-deleted rows. Unknown ID: `fleet 'X' not found.`, exit 1.

##### `fleet delete`

`cafleet fleet delete FLEET_ID [--json]` ends every active ordinary member that owns a concrete pane, then deregisters the fleet. It retains messages and is idempotent; an unknown ID fails with `fleet 'X' not found.`, exit 1. One operation-wide five-second deadline bounds the whole command.

1. **Lock.** Lock every active concrete placement in sorted key order and revalidate the placement set, retrying contention every 50 milliseconds and restarting on a set change. Contention until the deadline leaves every pane and registry row unchanged and exits 1 with `fleet ID pane input remained busy for 5 seconds at member MEMBER_ID; fleet was not deleted.`
2. **End members.** Under the held locks, end each active ordinary member with a concrete pane, in lock-key order, through the `member delete` procedure: runner shutdown and recovery for a wrapped member, pane close, that member's deregistration, then optional layout restoration. A member without a runtime has its pane closed as well. The root Director's pane stays open.
3. **Deregister the fleet.** One transaction stamps deletion, deregisters the remaining active members (the root Director and members with a pending or absent placement) and removes their placements.

The first member that cannot be ended stops the command, and the fleet is not deleted. Members ended before it stay deregistered with closed panes; the failing member and every later member keep their registry rows and panes, so `member list` still shows them. The command exits 1 with `fleet ID was not deleted: member MEMBER_ID: CAUSE. Members already deleted: IDS.`, where `CAUSE` is the member deletion cause and `IDS` is a comma-separated id list or `none`. A placement-set change after cleanup began exits 1 with `fleet ID placement or runtime ownership changed during cleanup; fleet was not deleted.` Rerunning `fleet delete` resumes with the members that remain.

On success the text output is `Deleted fleet ID. Deregistered N members.` followed by one `  member MEMBER_ID: PANE_STATUS` line per ordinary member in ascending id. `N` counts every member the command deregistered, root Director included; `PANE_STATUS` takes the `member delete` values `PANE (killed)`, `(pending — no pane)` and `(no placement)`. JSON is ordered `fleet_id, deregistered_count, members`, with one `{member_id, pane_status}` entry per ordinary member in the same order.

#### `cafleet doctor` — Environment Diagnosis

A full-environment diagnosis that reports **every** check even when the
multiplexer is unavailable or the database is missing or incompatible — no
early abort. `doctor` is exempt from the
schema-version guard (§6.3 Schema-version guard): a missing, outdated or
incompatible database is reported, not fatal. The only flag is the optional
`--json` (§6.3 JSON output (`--json`)).

##### Check rows

Each check has a stable `name`, a `status` of `ok`, `info` or `error`, and a
one-line `detail`. Checks run in this order:

| Check name | `ok` | `info` | `error` |
|---|---|---|---|
| `multiplexer` | `<backend> session=<session> window=<window_id> pane=<pane_id>` | — | The multiplexer resolution, availability or context-discovery error |
| `database` | `compatible schema <N>` at head | — | The schema-version guard error for an empty, behind, ahead or incompatible database, or the open/inspection cause |
| `<agent>.configuration` | The resolved config path (§ Config-dir resolution) | — | The config-path validation error, or `HOME is not set` |
| `<agent>.executable` | `<binary> is on PATH` | `<binary> is optional and absent from PATH` | — |
| `opencode.preset[.<mode>]` | § OpenCode preset checks | § OpenCode preset checks | § OpenCode preset checks |
| `<agent>.installation` | `recorded <version> at <installed_at>` when the version at the resolved path equals the CLI version | `recorded <version> at <installed_at>; run 'cafleet setup --coding-agent <agent>'` for another version; `no installation recorded at <path>; run 'cafleet setup --coding-agent <agent>'`; `unavailable: configuration failed: <cause>`; `unavailable: <database error>` for a database not at head | `installation query failed: <cause>` |
| `member_runtime.<run_id>` | — | § Member runtime checks | § Member runtime checks |

The three per-agent rows repeat for each backend in registry order (`claude`,
`codex`, `opencode`); the OpenCode preset checks follow `opencode.executable`.
A recorded version is compared by string equality, and a version mismatch is
`info`. Installation records are read only at the resolved path of a database
at head; a row recorded at another path is not the resolved installation.

Text mode prints one `{status} {name}: {detail}` line per check, with each LF
in the detail escaped as `\n` and each CR as `\r`. `--json` prints
`{"checks":[{"name":…,"status":…,"detail":…}]}` with the checks in the same
order. When any check has status `error`, doctor exits 1 with
`Error: doctor found errors` after printing every check; otherwise it exits 0.

###### OpenCode preset checks

`doctor` has no selected launch policy, so it checks the preset of every
OpenCode launch-policy choice (§ Launch policy and capability registry), in
catalog order. The default choice keeps the check name `opencode.preset`;
every other choice appends `.<mode>`:

| Check name | File |
|---|---|
| `opencode.preset` | `<preset base>/agents/cafleet.md` |
| `opencode.preset.cafleet-ask` | `<preset base>/agents/cafleet-ask.md` |

Each check applies the same statuses to its own file:

| State | Status | Detail |
|---|---|---|
| The file is readable | `ok` | The preset path |
| The file is missing or unreadable | `error` | `<path>: <cause>; run 'cafleet setup --coding-agent opencode'` |
| The opencode configuration failed | `info` | `unavailable: configuration failed: <cause>` |
| The `opencode` executable is absent from `PATH` | `info` | `unavailable: executable is absent` |

Setup installs every preset, so a missing `cafleet-ask` preset is an `error`:
the installation predates that preset or was altered.

An OpenCode installation recorded only at another base is not the resolved installation. With `OPENCODE_CONFIG_DIR` unset, doctor
reports it through the checks at the resolved base:

| Check name | Status | Detail |
|---|---|---|
| `opencode.configuration` | `ok` | The resolved base, `~/.config/opencode` |
| `opencode.installation` | `info` | `no installation recorded at <home>/.config/opencode; run 'cafleet setup --coding-agent opencode'` |
| `opencode.preset`, `opencode.preset.cafleet-ask` | `error` | `<home>/.config/opencode/agents/<mode>.md: <cause>; run 'cafleet setup --coding-agent opencode'` |

Rerunning setup installs both presets at the resolved base and leaves the
files at the other base in place.

##### Member runtime checks

With a compatible at-head database, doctor diagnoses every recorded member run in ascending `run_id` order without mutating registry or runtime state. Each run produces one entry named `member_runtime.<run_id>`:

| Outcome | Status | Detail |
|---|---|---|
| The read-only diagnosis completes | `info` | The run's diagnostics joined by `; `, such as a leftover launch descriptor, the owner-lock observation and the health-probe result |
| The diagnosis fails | `error` | The diagnosis cause; it counts as an issue |

One five-second budget covers all runs, and each run's diagnosis is bounded by the smaller of 100 ms and the remaining budget. When the budget is spent, doctor adds one `member_runtime` entry with status `info` and detail `runtime diagnosis budget exhausted; inspect remaining runs explicitly`, and skips the remaining runs. A database that is not at head produces no runtime entries. The recovery contract (§ Storage pressure and durability boundary) owns the probes and their recovery actions.

#### `cafleet message` — Message Broker

##### `message send`

`cafleet message send --from-member-id ID --to-member-id ID (TEXT | --file PATH) [--json]`

Resolve the complete text body before starting the five-second persistence/notification budget, including when `--file -` waits for stdin. Persist one point-to-point delivery, then evaluate delivery for it. A self-send (equal sender and recipient IDs) persists only in the broker and is evaluated first, even for an ordinary member with a healthy wrapped pane. Other root-Director messages use native or manual broker reads with no pane-delivery diagnostic and no multiplexer requirement. Otherwise drain an ordinary recipient's pending messages oldest-first into its concrete pane within the remaining budget, the new message included. A committed/persisted send remains successful with exit 0; queued, held or unavailable ordinary-member notifications preserve the stdout record and `notification_sent=false`.

`notification_sent` is `true` exactly when this invocation submitted the newly committed message, including when it was drained after an older one. When another invocation delivered the new message, `notification_sent` stays `false` and stderr carries no diagnostic; the recipient still received the line. Messages left pending when the budget no longer covers another submission stay pending with no diagnostic; the next send to that recipient or a `message wait` (§6.3) rotation retries them. A preview the recipient's runner accepted into its queue is retried by the runner itself (§6.9 Runner-owned deferred delivery). §6.5 Push notifications specifies the line, the drain and under-lock revalidation.

JSON is ordered `message,notification_sent,input_id,delivery`. `input_id` is the nullable durable input ID of the new message's preview receipt. `delivery` is the shared delivery projection (§6.9 Delivery reporting) for the newly sent message, even when the same invocation also retries an older pending message:

| Recipient | `delivery` | `input_id` |
|---|---|---|
| The sender itself | `{"status":"not_applicable","method":null,"reason":"self_send"}` | `null` |
| Another sender's root Director | `{"status":"not_applicable","method":null,"reason":"director_stdout"}` | `null` |
| An ordinary member | The projection of this invocation's evaluation of the new message | The receipt's ID when allocated |

Text output adds `Delivery: status=STATUS method=METHOD reason=REASON` after the envelope, using `-` for a missing method or reason, then `Input: INPUT_ID` when an ID is allocated. A queued result states that the runner will retry the preview.

###### Delivery outcome

A notification failure prints `Message NEW_ID was persisted. ` followed by its diagnostic on stderr, without suppressing successful stdout. Every per-attempt failure uses this diagnostic:

```text
message MESSAGE_ID was not notified to member MEMBER_ID: CAUSE; the next send to member MEMBER_ID or the next message wait retries it.
```

| Failure | `CAUSE` |
|---|---|
| The pane input lock stays busy through the lock wait (§6.5 Pane input lock) | `pane input remained busy` |
| Lock acquisition error | `pane input: <lock error>` |
| Multiplexer resolution or availability error | The multiplexer error text |
| Budget cannot cover literal input, settle and Enter before the first line | `insufficient pane input budget for literal input, settle and Enter` |
| Transport error before or during typing | The transport error text |
| Database busy or deadline exceeded during the under-lock snapshot or notification stamp | The storage error text; names the current candidate, or the candidate from placement revalidation when selection has not completed |
| `message wait` attempt fails with database busy or deadline exceeded before selecting a candidate | The storage error text; names the selected rotation entry |

A lock wait that ends after another sender delivered every pending message reports nothing. Other failures, including corrupt, incompatible or unreadable storage and a termination-span start failure, retain their causes after the persisted prefix. Storage failure before commit exits 1 with its cause. Consume an existing committed ID once; notification failure supplies no reason to create another delivery.

##### `message poll`

`cafleet message poll MEMBER_ID [--json]` returns only incoming `input_required` records, descending by status timestamp/message ID. Unknown or inactive member: `Member ID not found`, exit 1.

##### `message wait`

`cafleet message wait MEMBER_ID [--timeout SECONDS] [--json]`

Timeout is an integer in `1..=1800`, default `300`. Validate an active subject and live fleet. Existing unread deliveries return immediately; otherwise check at one-second monotonic intervals. The invocation is a blocking receive for scripts and manual use and never ACKs messages.

Between inbox checks, retry pending deliveries for at most one ordinary member per interval with an in-memory round-robin cursor. Select concrete ordinary-member placements in the fleet, excluding its root Director, the wait's own subject, self-notifications, ACKed and already-notified rows. Each attempt drains that member's pending messages under the delivery procedure (§6.5 Push notifications) without consulting pane state. One invocation attempts each pending message at most once; a failed message is retried by a later send or wait invocation, while later messages for the same member remain eligible. Recheck the inbox before starting another attempt. Shared input locks and under-lock revalidation keep multiple waiters safe.

Start each receive check or notification attempt before the monotonic receive deadline. An attempt already in flight retains its original five-second budget for the lock wait, revalidation, Escape, typing, settle and Enter. A busy pane lock makes the attempt wait up to 1.5 seconds; a lock still busy then records the not-notified diagnostic (§6.3 Delivery outcome) in `notification_errors`. Cap subprocess/database busy waits to its remaining budget and start input only with time reserved for settle and Enter. Mandatory database finalization uses the later of the original deadline and one second after finalization begins, keeping pane transport within its original budget. A receive returns by its deadline plus at most one six-second in-flight attempt/finalization allowance. A snapshot blocked until receive expiry returns `timeout` with retained notification diagnostics; earlier transient contention can retry before expiry. Corruption and other fatal storage errors retain their causes.

The ordered JSON result is:

```json
{"member_id":41,"outcome":"timeout","messages":[],"notification_errors":[]}
```

`outcome` is `messages` with nonempty full poll records, or `timeout` with an empty array. Each diagnostic has ordered keys `{member_id, message_id, error}`. Keep one diagnostic per not-notified message, ordered by message ID.

Pending placement and a placement change under the lock are ordinary holds that keep entries unchanged. A drain that notifies a recorded message removes that message's entry. Reconcile diagnostics with each successful receive snapshot and the typed observations of its owned notification attempt. Remove entries when these observations show that message ACKed, its recipient gone or the message notified by any invocation. After receive expiry, return the last successful snapshot reconciled with the completed attempt's observations. Subsequent external changes become visible to the next invocation; final reconciliation starts no additional I/O or tail budget.

Text is `Messages available for member ID.` plus normal messages, or `Timed out waiting for member ID.`; diagnostics go to stderr. Both outcomes exit 0. Deleted subject/fleet or corrupt/unreadable state exits 1 with its cause and retains earlier notification diagnostics on stderr, including when JSON output was requested. Ctrl+C outside a delivery attempt terminates the process normally; inside an attempt it takes effect when the attempt's termination-deferral span (§6.5 Termination during pane input) ends.

A script or operator uses `message wait` for finite manual reception and ordinary-member retry recovery, then polls and ACKs consumed messages. Native Director reception retains the existing listener process between events.

##### `message listen`

`cafleet message listen MEMBER_ID [--timeout SECONDS]`

Stream any active member's pending inbox through UTF-8 JSONL. Derive and pin its fleet at startup, using the configured database and the ordinary schema-version guard. Require a live root Director with a concrete registered placement, without resolving a multiplexer or checking its executable/physical pane. The subject's own placement may be absent. Unknown/inactive startup subjects use `Member ID not found`, exit 1.

| Surface | Contract |
|---|---|
| Lease | Optional integer `1..=1800`; absence runs until closure, cancellation or failure |
| Expiry | One monotonic lease from startup; activity leaves expiry unchanged |
| Output | Compact tagged JSON object plus LF, flushed per record; JSON strings escape controls/newlines |
| Startup | Validate/pin identity, then emit and flush `ready` before replay |
| Idle | Check immediately, then wait one second after a snapshot emits zero new IDs, shortened by the lease |
| Completion | Flush `closed`, then exit 0 for expiry or normal subject/fleet closure |
| Failure | Diagnostic on stderr and exit 1; parser usage errors retain exit 2 |

The command always emits JSONL; text mode and `--json` are absent. Stdout contains protocol records only:

```json
{"event":"ready","member_id":1,"fleet_id":1}
{"event":"message","message":{"message_id":12,"from_member_id":2,"to_member_id":1,"created_at":"2026-10-25T00:50:00.000000+00:00","status_state":"input_required","status_timestamp":"2026-10-25T00:50:00.000000+00:00","text":"complete (doc)\nFull original body"}}
{"event":"closed","member_id":1,"reason":"timeout"}
```

The inner message is exactly the full message record (§6.4 Persisted shape), preserving original text beyond CLI/pane limits. `closed.reason` is `timeout`, `member_inactive` or `fleet_deleted`; fleet deletion takes precedence when both lifecycle closures are observed. Check normal closures before revalidating a continuing live fleet. Missing persisted rows, changed fleet identity, invalid stored enums/root relationships or missing root placement are fatal errors.

Each short read transaction validates identity and selects every `input_required` record for the pinned subject ordered by `(created_at ASC, message_id ASC)`. Commit before output or sleep. A later commit with an earlier timestamp emits when first observed. A process-local ID set suppresses already-emitted pending IDs; insert only after the complete record and LF flush successfully. Retain only IDs still pending in successful snapshots; a failed snapshot leaves the set unchanged. A snapshot containing only previously emitted pending IDs waits one second, while newly emitted records permit immediate recheck.

Listening leaves consumption and `notified_at` unchanged and performs no pane retry, lock or dispatch. Stamped pending self-sends remain stream-eligible. Concurrent listeners are independent read-only observers with no consumer lock; restart replays all pending IDs. ACK-before-snapshot removes a message, while ACK racing with buffered output may leave a stale pending envelope. Reconcile known/replayed IDs with `poll --json` or `show --json` before acting, record deferred actions and explicitly ACK consumption. Completed records need no second action or ACK.

Each snapshot has a fresh database deadline at most five seconds away, shortened by the remaining lease. Scoped busy waiting and an execution progress handler every 1,000 VM instructions bound lock/SQL work. Check that deadline before/after prepare and execution, before/after decoding each row and before commit. Expiry discards partial snapshots; interrupted SQL maps to deadline exceeded only when that deadline expired. Restore busy handling and remove the listener-owned progress handler on every path, preserving cleanup failures alongside the original cause. These cooperative boundaries include row collection without claiming hard real-time preemption of one allocation or SQL operation.

Output owns one serialized record at a time and applies backpressure before selecting more messages. Nonblocking filesystem/poll output preserves/restores descriptor flags on normal return, resumes partial writes from their byte offset and checks every write/flush. Its five-second deadline is shortened by the lease. Broken pipe, write-zero, flush failure or output deadline exhaustion terminates visibly with exit 1, even after partial records. Normal lease expiry between records uses a separate bounded final write for `closed`. Complete records survive a later error; an incomplete final line remains unconsumed and recovers through broker replay. Only the current snapshot, one record and pending emitted IDs occupy listener memory.

Database contention retries stay inside the snapshot budget; exhaustion or another fatal error terminates the stream. The listener retains its connection/PID across healthy events and ordinary signal behavior outside pane termination deferral. Host cancellation may end it without `closed`. Native owners retain one handle, reconcile durable state after failure and establish old-process termination before replacement. Installed backend instructions own concrete native tool recipes and their evidence limits.

##### `message ack`

`cafleet message ack MESSAGE_ID [--json]` transitions an existing unread message to completed, updating its status timestamp. The recipient/fleet derive from that row. ACK signifies consumption.

##### `message show`

`cafleet message show MESSAGE_ID [--json]` returns one full record. Existence is the read guard.

#### `cafleet member` — Member Lifecycle + Pane Interaction

##### Member targeting and key delivery

Require an active member and placement for pane operations; concrete pane IDs remain opaque. Pending placement is valid during creation and yields contextual prompt/exec/capture errors. Missing placement fails pane actions; list/show retain diagnostic nulls. Prompt and exec reject the root Director before pane I/O; individual member deletion protects it.

##### `member create`

Register a member **and** spawn its coding-agent pane. It takes no identity
flag: the acting Director is auto-resolved from `fleets.director_member_id`
first thing, before registration (the resolved id also feeds the member's
backend inheritance and the spawn-prompt substitution). A fleet has exactly
one root Director by construction, so no override flag exists.

| Argument | Required | Notes |
|---|---|---|
| `--fleet-id` | yes | The fleet the new member joins — a relationship flag; the subject of the command is the member being created. |
| `--name` | yes | Display name; pane-title behavior per backend is in §6.7 Backend capabilities. |
| `--description` | yes | One-sentence purpose. |
| `--coding-agent` | no | A backend registry token; when omitted, the member inherits the spawning Director's placement backend. Availability failures exit 1 (below). |
| `--model` | no | Model forwarded to the backend binary's own `--model` flag. The opencode backend additionally requires `<provider-id>/<model-id>`; per-backend formats and create-time validation are in §6.7 Model selection. |
| `--permission-mode MODE` | no | Launch-only selection from the resolved backend’s policy catalog (§6.9 Launch policy and capability registry); omission selects the backend's automatic approval mode for claude (`auto`) and codex (`approve-for-me`) and the `cafleet` preset for opencode. Pass it only for another mode, such as claude `dontAsk`, codex `on-request` or `never`, or opencode `cafleet-ask`. |
| `--event-evidence POLICY` | no | `metadata` by default, or explicit `redacted` evidence under the privacy contract (§6.9 Persistence, notification and read interfaces). |
| `--event-redaction-file PATH` | for redacted | Absolute readable UTF-8 redaction configuration; invalid combinations/configuration fail with usage exit 2 before registration. |
| `--effort` | no | Reasoning-effort level forwarded to the backend binary, validated per backend before any side effect. Accepted levels, forwarding forms, and rejection strings are in §6.7 Reasoning effort. |
| positional `PROMPT` | one of | Inline spawn prompt (backend-neutral template). Exactly one of `PROMPT` / `--file`. |
| `--file PATH` | one of | Path to a UTF-8 file whose contents are the spawn prompt (`-` = stdin). Follow the shared body-input and transport contract (§6.3 Text bodies (positional `TEXT` / `--file`)). |
| `--json` | no | Output as JSON |

A missing backend binary exits 1 with `Error: binary <name> not found on PATH`. The opencode backend additionally requires the selected launch policy's agent preset `<preset base>/agents/<mode>.md` at the resolved preset path (§ Config-dir resolution); a missing or unreadable selected preset or an invalid config-location variable exits 1 per §6.3 Error Messages.

###### Spawn command per backend

The per-backend child argv and omitted permission defaults live in
§6.7 Per-backend `build_spawn_argv` (exact, token-by-token).

###### Member environment

The native child runs in the multiplexer pane's environment plus the spawn
environment, which the runner applies to the child as overrides. The spawn
environment holds `CAFLEET_DATABASE_URL`, followed by the selected backend's
config-location variable with the creator's exact value when the creator's
environment sets it:

| Backend | Forwarded config-location variable |
|---|---|
| claude | `CLAUDE_CONFIG_DIR` |
| codex | `CODEX_HOME` |
| opencode | `OPENCODE_CONFIG_DIR` |

The member therefore reads the configuration that `member create` validated
and `setup` installed into (§6.3 Config-dir resolution).
Only the selected backend's variable is forwarded. An unset variable stays
unset, so the member uses the native default, which equals the CAFleet default
base. An invalid value fails `member create` before registration with the
pinned validation error. The launch descriptor and launch evidence record the
spawn environment; config locations are paths, not secrets.

###### Spawn-prompt substitution

`member create` renders its resolved spawn prompt body
with the same formatter, substituting exactly four placeholders:

| Placeholder | Substituted value | How the spawned member sees it |
|---|---|---|
| `{fleet_id}` | The member's fleet id | `FLEET ID: <fleet_id>` |
| `{member_id}` | The member's own newly-allocated id | `YOUR MEMBER ID: <member_id>` |
| `{director_member_id}` | The fleet's root Director id | `DIRECTOR MEMBER ID: <director_member_id>` |
| `{coding_agent}` | The resolved backend name (`claude`, `codex`, or `opencode`) | `CODING AGENT: <coding_agent>` |

Identity reaches the spawned member as literals rendered into its prompt, not
through the environment. The formatter accepts only the four exact names above and doubled literal
braces (`{{` / `}}`), not Python format specifications, conversions, or
attribute/index access. An unknown placeholder or
malformed brace expression exits 2 before registration
(see §6.3 Error Messages).

The spawn always creates the pane without stealing focus (tmux
`split-window -d`): the Director's pane and active window stay active. In the
default output, `pane` renders `(pending)` until the pane id is patched onto
the placement.

###### Creation failure compensation

Member creation validates policy, evidence configuration and assignment, and records the installed native version through the native version probe (§6.9 Native version probe), then registers a pending placement and run. A failed probe adds the stderr line `Warning: native version unavailable for BACKEND: CAUSE; coverage unknown.` and creation continues with unknown coverage. A probed version selects the bundled profile with the greatest minimum version at or below it, with no version-mismatch warning, and a known version below the backend's first boundary exits 1 before member/run registration or pane creation with `unsupported native version for BACKEND: VERSION; minimum supported version is MIN_VERSION` (§6.9 Profile selection by minimum version). The multiplexer starts a runner from a private descriptor; a `prepared` handshake precedes placement publication and `start`, and creation returns only after child spawn produces `started` within one handshake deadline. §6.9 Runner boundary and creation transaction owns descriptor cleanup and uncertain-launch compensation. Spawn-prompt placeholders are validated before registration, so a placeholder error exits 2 with nothing registered. The backend owns a newly split pane until the split returns; see §6.5 Pane ownership during creation.

| Failure point | Compensation order | DB result when compensation succeeds |
|---|---|---|
| Pending member/run registration | The registration transaction rolls back | No new member, placement or run |
| Runtime artifact preparation or prompt rendering after registration | Remove the pending descriptor, then mark the run `failed` and deregister the member in one transaction | Member deregistered, placement removed, run `failed` |
| Runner pane split | The backend closes a pane it already identified; then the same descriptor/run/member rollback | Member deregistered, placement removed, run `failed` |
| After the split: handshake, placement publication, `start` or the started-state check | Shut down the owned runner and child when the runner already owns the run, close the created pane, then the same descriptor/run/member rollback | Member deregistered, placement removed, run `failed` |

Continue remaining compensation even if an earlier cleanup fails. A pane the
backend already tried to close is never closed again by the creation rollback.
A failed split with no confirmed id reports `pane ID unknown; pane cleanup
unconfirmed`; it does not infer which other pane should be closed.

Keep the primary cause and its exit category, and append each cleanup
diagnostic after it on its own stderr line. A backend close failure appends
`cleanup failed for pane <id>: <detail>`. An unconfirmed owned-child shutdown
appends its shutdown diagnostics, or `owned child cleanup remains unconfirmed`
when it reports none. A failed descriptor removal, run update or deregistration
appends its own cause, and a recorded run with remaining diagnostics keeps
`cleanup_state=incomplete`. A panic during launch reports
`native launch unwound; performing owned compensation` after the same
compensation, and an interrupted pending creation reports
`pending creation was interrupted`. Once the runner reports `started` under
the matching identity, creation removes the descriptor and emits its result;
placement publication alone keeps the rollback armed. Existing output keys
retain their order and nullability; runtime fields append under the
runtime presentation contract (§6.3 Runtime presentation).



##### `member delete`

For wrapped members, request owned runner shutdown under the recorded identity before pane/registry cleanup, retaining explicit unconfirmed-child diagnostics and recoverable runtime records. Attempt remaining event replay before deregistration only within the remaining operation deadline. Lock the concrete placement before kill/deregistration. A member without a concrete pane needs no lock. Retry contention every 50 milliseconds against one five-second deadline; expiry exits 1 with `member ID's pane input remained busy for 5 seconds; member was not deleted.` and leaves state unchanged. Preserve pane/registry errors and layout cleanup ownership.

Revalidate pending or absent placement and deregister atomically. For a concrete pane, retain input ownership while closing it outside a SQLite write transaction, then revalidate and finalize the registry. After successful close, mandatory registry finalization uses the later of the original deadline and one second after finalization begins, allowing short reader/writer contention to clear; actual database failures report that the pane was closed and preserve the database cause. Optional layout work follows mandatory finalization within the original deadline. A failed or expired restoration emits a stderr diagnostic while preserving successful deregistration.

##### `member show`

`cafleet member show MEMBER_ID [--json]` — the positional `MEMBER_ID` names
any active registry entry — placed or placementless (root Director included).

Registry read — no multiplexer requirement. Text is the compact one-line row;
the detailed view — including `kind` (`director` / `member`, §5.4 Member kind discriminator) and the
placement sub-dict and appended runtime summary — is the `--json` payload (see
§6.3 Runtime presentation). A wrapped result appends the runtime text line; an unwrapped result retains its existing row.


##### `member list`

`cafleet member list FLEET_ID [--json]` — the positional `FLEET_ID` names the
fleet whose roster is listed; no identity flag. Lists every **active**
registry entry of the fleet —
the root Director, ordinary members, and placementless rows. An empty roster
prints `0 members.`.

Field projection and absent values are in §6.3 Member list output.

Rows and activity follow §6.2 Members.
Text idle values render as `Ns` / `Nm` / `Nh`.
Per-member detail such as `description` and `registered_at` lives on
§6.3 `member show`.


##### `member prompt`

`cafleet member prompt MEMBER_ID TEXT [--json]` validates a nonblank line without CR/LF or C0/DEL controls, otherwise exits 2 with `text must be one non-empty line without control characters.`

A root Director target exits 1 with `cannot prompt the Director's own pane` before any pane I/O, because Escape would interrupt the Director's own running turn. For a wrapped member, validate activity/placement and submit the request to its runner, which owns the pane input lock for the actual stages (§6.5 Pane input lock) and selects profile delivery, delivery fallback or queueing under the delivery arbitration contract (§6.9 Delivery method and interaction priority). Explicitly unwrapped members acquire the shared input lock within its bounded wait, revalidate activity/placement and receive Escape, settle, literal text, settle and Enter; that Escape-first path can interrupt work or dismiss a prompt. Exit 0 means submission or queue acceptance only, with ordered `{member_id, pane_id, text, input_id, delivery}`; unwrapped input has a null ID and method `unwrapped`.

A submitted prompt prints `Sent prompt TEXT_REPR to member NAME (PANE).`; a queued prompt prints `Queued prompt for member ID (PANE).` Both add the delivery and input lines (§6.3 Runtime presentation).

A lock still busy when the wait ends uses `member ID's pane input remained busy; prompt was not dispatched.` Pre-write error, including an Escape failure or a held runner reply: `prompt was not dispatched to member ID: CAUSE`. Partial uncertainty: `prompt delivery to member ID is uncertain: CAUSE. Inspect the pane before retrying.` These exit 1 without a success record.

##### `member exec`

`cafleet member exec MEMBER_ID COMMAND [--json]` or `cafleet member exec MEMBER_ID --file PATH [--json]`; `--file -` reads stdin. Exactly one source is required. File/stdin is UTF-8 and strips at most one terminal LF or CRLF. Preserve inline bytes exactly.

Require a nonblank line without CR/LF, NUL or other C0/DEL controls, including tabs. Invalid input exits 2 before pane I/O with `command must be one non-empty line without control characters.` A multiline task supplies a one-line explicit script invocation.

Resolve an active ordinary member and concrete placement and reject the root Director. Wrapped input goes to the runner, which owns the pane input lock for the actual stages (§6.5 Pane input lock) and submits literal `! ` plus the command through the native-exec submission plan, delivery fallback or its queue (§6.9 Delivery method and interaction priority). Explicitly unwrapped input locks within its bounded wait, reloads and retains Escape, settle, literal native shell input, settle and Enter. The backend owns working directory, environment, permissions, output and exit rendering.

Successful exit 0 establishes complete submission/dispatch or queue acceptance only. Ordered JSON is:

```json
{"member_id":42,"pane_id":"w1:p2","coding_agent":"codex","command":"printf example","dispatched":true,"input_id":17,"delivery":{"status":"submitted","method":"profile","reason":null}}
```

`dispatched` is `true` for `submitted` and `false` for `queued`. Text: `Dispatched native command to member ID (PANE). Inspect the pane for its result.` for a submission, or `Queued native command for member ID (PANE).` for queue acceptance, each followed by the delivery and input lines (§6.3 Runtime presentation).

| Condition | Error body (exit 1) |
|---|---|
| Root Director | `cannot exec in the Director's own pane` |
| Pending placement | `member ID has no pane yet (pending placement) — nothing to exec.` |
| Input lock still busy when the wait ends | `member ID's pane input remained busy; native command was not dispatched.` |
| Escape or transport failure before typing | `native command was not dispatched to member ID: CAUSE` |
| Typing may have begun | `native command delivery to member ID is uncertain: CAUSE. Inspect the pane before retrying.` |

The first text-writing call can partially type even if it fails. Failure after Enter may mean execution began. Inspect native pane output before any newly authorized retry. A wrapped operation durably records its operation key, payload digest and submission state; the receipt contains no command text. The result describes dispatch, independently of native execution. The Director authorizes native input under its host rules; visibility establishes no second member-tool permission check.

After native completion, inspect the pane and send an explicit prompt to use the output and continue when the host has not resumed the member.

##### `member press`

`cafleet member press MEMBER_ID KEY... [--json]` requests only the specified ordered keys. Require a positive signed 64-bit member ID and one through 64 keys, each at most 32 UTF-8 bytes, using the common key grammar and backend mappings (§6.5 Explicit key dispatch). Preserve literal character case and render named keys canonically. Validate the complete vector before any backend probe or writing invocation.

Use the ordinary `--` option separator before literal punctuation that could be read as an option; put `--json` before that separator. For example, `cafleet member press 42 -- '-' '+' ';'` requests three keys. It supplies no Escape prelude, Enter suffix, prompt text or shell-command submission.

Resolve an active ordinary member and its concrete placement, preserving the Director-target restriction and the shared input lock, selected-context validation and original budget (§6.5 Explicit key dispatch). Wrapped members receive the request through their outer pane as manual native input (§6.9 Explicit manual key input). Success means backend acknowledgment of this ordered request; actual PTY bytes, native approval and completion require their own observation.

Exit 0 prints compact ordered JSON when `--json` is selected:

```json
{"member_id":42,"pane_id":"%5","keys":["Ctrl+a",";"],"status":"acknowledged"}
```

Otherwise print `Acknowledged keys KEYS_JSON for member ID (PANE).`, where `KEYS_JSON` is the compact JSON array of canonical keys. Both modes end with one newline. The result carries no native-byte count, delivered-prefix count or permission receipt.

Missing/extra positional arguments and a nonpositive/malformed member ID use Clap's native usage diagnostics. Direct key validation uses these usage-error bodies, with one-based `N` and JSON-quoted invalid input:

| Condition | Error body (exit 2) |
|---|---|
| Key count outside the domain | `press requires between 1 and 64 keys` |
| A key exceeds its UTF-8 bound | `press key at position N exceeds 32 UTF-8 bytes` |
| A key is outside the common grammar | `invalid press key at position N: JSON_QUOTED_TOKEN` |

Operational failures exit 1 with empty stdout, including under `--json`. Retain the original typed cause and any cleanup diagnostics, then append the effect diagnostic below on its own stderr line; the executable prefixes the primary first line with `Error: `.

| Observed boundary | Effect diagnostic |
|---|---|
| Before any writing process starts, including a failed metadata probe | `keys were not dispatched to member ID.` |
| A writing process starts and fails, times out or returns an unreadable acknowledgment | `key delivery to member ID is uncertain; any prefix may have been handled. Inspect the pane before an explicit retry.` |

| Local refusal | Primary error body |
|---|---|
| Root Director target | `cannot press keys in the Director's own pane` |
| Input lock remains busy | `member ID's pane input remained busy` |
| Revalidated target changes | `member ID placement changed before key dispatch` |
| Selected adapter differs from placement | `press backend does not match the recorded placement` |
| Selected live context differs from placement | `press target context does not match the recorded placement` |

Missing member/placement and native process errors retain their existing primary diagnostics. A pending pane uses `member ID has no pane yet (pending placement) — nothing to press.` Binary-not-found, not-started and prelaunch expiration have zero writing invocations. Every started failure, including a pane-gone response, retains possible partial delivery; perform no automatic retry. Request-bound permission decisions retain the separate `permission respond` contract and host authorization.

##### `member capture`

`cafleet member capture MEMBER_ID [--lines N] [--ansi] [--json]` — the
positional `MEMBER_ID` names the target member.

- `--lines`: Number of trailing lines to capture (default: **20**).
- `--ansi`: Preserve ANSI escapes in the raw capture. The default strips ANSI escapes and cleans carriage-return redraws.

Output shapes are in §6.3 Output shapes; target resolution is
shared with the `member` keystroke verbs — see
§6.3 Member targeting and key delivery. A
pending placement is a hard error (see §6.3 Error Messages).

JSON stamps `captured_at` from the local UTC clock at the capture read
boundary and sets `content_sha256` to the lowercase SHA-256 hex of the exact
emitted `content`'s UTF-8 bytes, including Unicode and empty captures. The
default mode hashes the ANSI-stripped, carriage-return-defragmented string;
`--ansi` hashes the ANSI-preserving string. No normalization occurs after the
selected mode, and capture content is never stored in SQLite. Text member
capture prints content without adding a newline.


#### Runtime presentation

Member create preserves ordered `member_id,name,registered_at,placement` and appends `runtime`; member show preserves ordered `member_id,name,description,status,registered_at,kind,placement` and appends `runtime`. An explicitly unwrapped member has `runtime: null`. The runtime object has ordered keys `run_id,resolved_mode,status,reachability,owner_epoch,last_health_at,coverage,current_interaction_id,unresolved_input_ids,owner_kind,owner_pid,owner_start_identity,runner_pid,runner_start_identity,child_pid,child_start_identity,effective_policy`, preserving stored nullability. `effective_policy` is the stored effective policy record (§6.9 Effective policy record) or null. `unresolved_input_ids` lists the run's inputs in `in_progress`, `accepted` or `uncertain`, plus `ready` inputs carrying deferred metadata, in input-ID order; an accepted input needs no reconciliation and stays listed until a settlement records its disposition, a deferred ready input stays listed while the runner owns it in its queue, and `in_progress` and `uncertain` inputs need Director reconciliation. These are registry reads and make no live-health claim.

Wrapped create/show preserves its existing text row and appends `Runtime: run=RUN_ID mode=MODE status=STATUS coverage=COVERAGE review=STATE`, where `STATE` is the effective policy's `automatic_review` value, `-` when absent. `event show` uses the same runtime object and line. Prompt/exec/send JSON appends `delivery` after `input_id` (§6.9 Delivery reporting). Their text output appends `Delivery: status=STATUS method=METHOD reason=REASON`, using `-` for a missing method or reason, then `Input: INPUT_ID` when a durable ID exists. The existing submitted-success lines print only for actual submissions; a fallback submission always prints `method=fallback` with reason `submission_plan_unavailable` or `idle_timeout`, and a queued result names `interaction_open`, `interaction_candidate`, `composer_busy` or `earlier_input` and states that the runner will retry. A queued result also points to `cafleet input show INPUT_ID --json` for the eventual result. An uncertain error retains its existing exit category, names the receipt's recorded `uncertain_cause` and the selected method, as in `exec delivery to member 110 is uncertain: manual_input (method=profile)`, and includes known receipt identity as `input_id=INPUT_ID`; run/interaction context accompanies held/rejected input.

#### Runtime event and input commands

`cafleet event show EVENT_ID [--json]` returns ordered `event,notification_state,message_id,archive_reason,run,interaction`. The event is the complete versioned runtime payload (§6.9 Persistence, notification and read interfaces), run is the current runtime summary, and nullable interaction contains the stored interaction fields in the interaction field order below. Text is `Event: EVENT_ID run=RUN_ID seq=EVENT_SEQ kind=KIND notification=STATE message=MESSAGE_ID` followed by the runtime summary line and `Interaction: INTERACTION_ID classification=CLASS closed_at=TIMESTAMP close_reason=REASON` when linked. Unavailable optional text values render `-`.

`cafleet input show INPUT_ID [--json]` and `cafleet input reconcile INPUT_ID --disposition DISPOSITION --evidence-ref REFERENCE [--json]` return `{input}` with the durable row ordered `input_id,run_id,operation_key,kind,payload_digest,state,created_at,updated_at,message_id,disposition,evidence_ref,permission_response,delivery,deferred_at,defer_reason,payload_digest_version,uncertain_cause`. The nullable audit object and additional text line follow the permission contract (§6.3 Permission-response audit output). `delivery` is the ordered `status,method,reason` projection of the stored receipt (§6.9 Delivery reporting), or null for a permission response; `deferred_at` and `defer_reason` are null together unless the runner queued the input. `uncertain_cause` is the cause the runner recorded when the input became `uncertain` (§6.9 Uncertain cause), retained through settlement, or null when no cause was recorded. Text is `Input: INPUT_ID run=RUN_ID kind=KIND state=STATE disposition=DISPOSITION message=MESSAGE_ID`, with `-` for absent optional fields.

Reconciliation is Director-owned. CLI disposition tokens are `submitted`, `not-submitted` and `abandoned-uncertain`, persisted with underscores as defined by the receipt contract (§6.9 Input arbitration and the original native answer). Require a nonempty evidence reference, synchronize with the current owner before CAS settlement and preserve uncertainty while an active writer cannot be ruled out. A settled ordinary receipt with the same disposition returns its original row and evidence reference before live work; a conflicting disposition fails unchanged. Settlement sends no input and establishes no native command result. One original five-second budget covers the pane wait, current-owner synchronization, supplied-connection SQL and socket exchange. If the writer is unreachable, use the receipt-only exclusive ownership contract (§6.9 Input arbitration and the original native answer), preserving the fixed placement state, any original pane guard, the original identity-matching run lock and lifecycle history; unavailable proof retains uncertainty. Ordinary member recovery keeps its completed-terminal no-work behavior.

All event/input IDs are positive database-assigned integers; malformed IDs/options fail with usage exit 2. Missing rows and invalid stored state fail explicitly. Interaction fields are ordered `interaction_id,run_id,episode_seq,classification,evidence_digest,opened_at,evidence_json,closed_at,close_reason`.

##### `member recover`

`cafleet member recover MEMBER_ID [--json]` is Director-owned and reconciles a recorded run of an active or deregistered member. A member without a run fails explicitly before mutation. §6.9 Storage pressure and durability boundary owns identity, ownership, replay, archival and its bounded pass.

The exceptional form is `cafleet member recover MEMBER_ID --owner-terminated --expected-run-id RUN_ID --expected-owner-epoch EPOCH --evidence-ref REFERENCE [--json]`. Require all four options together, positive snapshot IDs/epoch and a nonempty operator evidence reference; incomplete or invalid input exits 2. A changed owner rejects the attestation, and positive live evidence preserves the live owner.

Ordered JSON keys are `member_id,run_id,outcome,replayed_count,inbox_count,archived_count,status,cleanup_diagnostics`. Outcome is `recovered`, `recovering`, `live_or_unreachable` or `operator_reconciliation_required`; diagnostics retain cleanup/attestation provenance. Counters measure work newly committed by this invocation under the runtime recovery count domains below. A matching completed terminal return validates supplied current run/epoch expectations and preserves existing history with zero counts. Text is `Recovery: member=MEMBER_ID run=RUN_ID outcome=OUTCOME status=STATUS replayed=COUNT inbox=COUNT archived=COUNT`, followed by each bounded diagnostic as `Cleanup: DIAGNOSTIC` in array order. A string diagnostic renders its contained text; a structured diagnostic renders compact JSON. Incomplete or unreachable outcomes remain distinct from confirmed recovery.

#### Permission commands

`cafleet permission inspect INTERACTION_ID --event-id EVENT_ID --from-member-id DIRECTOR_ID [--json]` reads the current request through its pinned runner. Require positive IDs, then an existing interaction (an unknown `INTERACTION_ID` is reported as not found before any event, run or database-identity check), then an active ordinary target, pinned active Director and live fleet; verify the event/message association and interaction ownership under the existing pane lock. One original five-second deadline, clipped to any inherited SQL bound, covers locking, current observation/manual progress, database work and the control exchange. Inspection sends zero programmatic child bytes and creates no receipt, audit or claim. Unknown answer capability is an inspection result; stale ownership or an unavailable owner is an operational error.

The inspection JSON is `{request}`. Its object has ordered keys:

```text
event_id,message_id,member_id,fleet_id,director_member_id,run_id,owner_epoch,interaction_id,episode_seq,classification,visibility,backend,native_version,profile_version,answer_profile_version,answer_rule_id,answer_capability,capability_reason,supported_decisions,request_token,output_seq,evidence,missing_evidence
```

IDs/epoch/episode are positive integers; `output_seq` is an unsigned integer. `message_id` is the verified inbox message ID. `classification` is the current interactive class or null, and `visibility` is `visible`, `obscured`, `not_visible` or `unknown`. Native/detector/answer-profile identifiers are nullable when unavailable. `answer_profile_version` is shared by the currently supported decision plans. `answer_rule_id` is their common rule when unique, including the actual rule for one supported decision; differing supported rules yield null. Respond derives its chosen nonnullable rule internally for the immutable audit. `answer_capability` is `tested` or `unknown`; `capability_reason` is null when the current request is answerable and otherwise a bounded explanation. `supported_decisions` contains the currently tested subset of `allow,deny`, in that order; a blocked or unsupported request returns an empty array and null token.

`request_token` is a nullable random lowercase 64-hex value issued by the current runner, bound to the request and invalidated under the response contract (§6.9 Request-bound permission responses). `evidence` is the complete selected current post-policy object. `missing_evidence` lists unavailable public facts in order from `operation,arguments,cwd,environment,targets,effects,choice_scope`. Treat operation as available only when its post-policy value is nonblank, unredacted and untruncated; metadata-only, `[redacted]` or truncated evidence leaves it missing. The current detector supplies no independent structured arguments, cwd, environment, targets or effects, so those facts remain missing rather than being parsed from operation text. Include `choice_scope` whenever the public post-policy evidence has no explicit scope fact, including scope-free evidence accompanied by a valid token and a proved private `single_request` plan. The Director obtains necessary missing facts through authorized current capture/routed evidence before deciding. Token identity and private plan eligibility are independent of public evidence completeness and authorization. The token binds the full immutable decision-to-plan set, including each profile, rule and stage identity; a null common-rule field supplies no authority to select a replacement plan.

Inspection text is:

```text
Permission: interaction=INTERACTION_ID event=EVENT_ID member=MEMBER_ID run=RUN_ID epoch=EPOCH visibility=VISIBILITY answers=DECISIONS
Request: token=TOKEN output_seq=SEQUENCE profile=ANSWER_PROFILE rule=ANSWER_RULE
```

Join supported decisions with commas and render unavailable values or an empty decision set as `-`. Append `Capability: REASON` when nonnull and `Missing evidence: FIELDS` when the array is nonempty; join fields with commas. Text includes no raw screen content.

`respond` requires this complete request snapshot and decision provenance:

```text
cafleet permission respond INTERACTION_ID --event-id EVENT_ID --from-member-id DIRECTOR_ID --expected-run-id RUN_ID --expected-owner-epoch EPOCH --request-token TOKEN --decision allow|deny --decision-source director-auto|user --director-mode auto|manual|unknown --mode-evidence-ref REFERENCE --decision-evidence-ref REFERENCE --operation-key KEY [--json]
```

Require positive IDs/epoch, the token's exact hex form, a nonblank operation key and the bounded evidence references (§6.9 Permission response storage). Source `director-auto` requires mode `auto`; source `user` records the user's decision in any mode. Enum/shape/source-mode errors exit 2 before lock or byte effects. These fields are attributed claims: resolve Director mode from current host evidence independently of member launch policy and obtain user permission in other/unknown modes. Native and managed rules still authorize running the command.

Validate a retained operation key against its recorded database/run, response identity and canonical digest before any current-request eligibility check, pane lock or live-owner probe. A matching retained receipt returns its stored result after token revocation, interaction closure, deregistration or owner exit, sending zero bytes; conflicting reuse fails. Validate historical audit relationships against recorded ownership while preserving closed/inactive history. Only an absent key or a positively recorded ready zero-byte attempt enters current live-request validation.

The response JSON is `{response}`, with ordered keys:

```text
status,input_id,member_id,run_id,owner_epoch,interaction_id,event_id,decision,decision_source,stage,output_seq,reason
```

`status` is `accepted`, `held`, `rejected` or `uncertain`; `input_id` is nullable until a durable row exists. Decision/source retain their command enums. `stage` is `zero_bytes`, `partial`, `submitted` or `unknown`; accepted has submitted stage, held/rejected has zero-byte stage, and uncertainty reports the observed partial/submitted stage or unknown. `output_seq` is a nullable unsigned integer when no current sequence can be established. `reason` is a nullable bounded explanation. A matching settled/submitted receipt returns accepted without typing; another settled disposition returns rejected without typing and explains that the key is consumed.

Historical response projection uses only the durable row and immutable audit. Return the original input identity, decision/source and audit `expected_owner_epoch` as `owner_epoch`, with `output_seq=null`; current owner/screen evidence supplies no replacement value. It performs no lock, live probe, control exchange or database mutation.

| Durable receipt | Retry status and stage |
|---|---|
| `accepted`, or `settled/submitted` | `accepted/submitted`. |
| `in_progress` or `uncertain` | `uncertain/unknown`; retain original transmission uncertainty and claim. |
| `settled/not_submitted` or `settled/abandoned_uncertain` | `rejected/zero_bytes` with a consumed-key reason; preserve the original disposition and claim. |
| `ready` | The separate exact-metadata live-validation path; retain its original epoch/token binding. |

An uncertain reply, live or retained, reports the receipt's recorded `uncertain_cause` (§6.9 Uncertain cause) as `reason`; a receipt without a recorded cause, such as an `in_progress` row left by runner loss, reports `prior transmission remains uncertain`. For a consumed key, `zero_bytes` describes this retry only and supplies no zero-byte evidence about the original attempt. A prior abandoned-uncertain claim remains possibly transmitted. This projection reads retained history even after current owner/interaction eligibility is lost.


Response text is:

```text
Permission response: interaction=INTERACTION_ID run=RUN_ID decision=DECISION source=SOURCE status=STATUS input=INPUT_ID stage=STAGE output_seq=SEQUENCE
```

Render null values as `-` and append `Reason: REASON` when nonnull. Accepted means answer-key submission, independently of native approval and execution.

Exit 0 means accepted submission or its matching prior submitted receipt. A structured held/rejected/uncertain reply prints the response in the selected text/JSON form and exits 1, with stderr `Error: permission response STATUS for interaction INTERACTION_ID: REASON`; append ` (input_id=INPUT_ID)` when known. This structured-response stdout is the exception to ordinary application errors being stderr-only. Pre-reply operational failures retain their ordinary error category and include known run/interaction/input context; usage failures exit 2.

A new attempt against a blocked current request fails before any receipt, audit, claim or byte. It exits 1 with the primary stderr line `Error: runner control: permission request is blocked: REASON`, followed by the known run/interaction/operation-key context. Respond checks the blocker at admission, after its admission drain, and again before each answer stage. `REASON` is the same bounded explanation that `permission inspect` returns as `capability_reason` for that state, so a blocked inspection shows a null token, an empty decision set and the reason a response would report:

| Reason | Blocking state |
|---|---|
| `permission input is fenced` | Programmatic input is fenced for shutdown, ownership loss or recovery. |
| `manual input accounting failed` | The runner could not account for actual manual input. |
| `observation degraded: CODE` | The observer recorded a gap or degradation with diagnostic `CODE`. |
| `observation transitions pending` | Observed transitions await persistence. |
| `observation evidence pending` | An interaction evidence refresh awaits persistence. |
| `observation update pending` | A retained transition, draft or evidence update has not completed. |
| `observation incomplete` | Child output was read but its observation has not completed. |
| `observation health unavailable` | The observer reported no input health. |
| `terminal input interrupted` | A terminal read was interrupted during the drain. |
| `outer terminal closed` | The outer terminal reached end of input. |
| `native output closed` | The native child's output reached end of file. |
| `terminal output still arriving` | The drain's bounded rounds ended before human and child input were quiet. |
| `terminal reply pending` | An outer-terminal reply prefix awaits completion within its 100 ms classification bound. |
| `permission request is not visible` | The request is obscured, closed or not observed as visible. |
| `permission request is not the current interaction` | Input eligibility is not held for the current permission interaction. |

A retained operation key is resolved before the admission drain, so its stored result returns without typing even when the current request is blocked.

The CLI takes semantic decisions. For a new attempt, the pinned runner validates the current stable permission request, token, profile, manual input and output state, atomically reserves its audit/claim/receipt, and sends only a tested single-request native choice. Same-key retries return the prior receipt; a conflicting decision/digest or another key bypassing the episode claim fails. Unknown capability preserves manual native input. `member prompt`, `member exec`, message previews, receipt settlement and recovery remain separate operations.

##### Permission-response audit output

`input show` and `input reconcile` append nullable `permission_response` after `evidence_ref`. For a response kind it contains ordered `input_id,event_id,interaction_id,expected_owner_epoch,request_token,decision,director_member_id,director_mode,decision_source,mode_evidence_ref,decision_evidence_ref,answer_profile_version,answer_rule_id,decided_at`; every field is required and follows its SQL type/nullability. Other input kinds return null.

Preserve the existing input text line and append `Permission decision: interaction=INTERACTION_ID event=EVENT_ID decision=DECISION source=SOURCE mode=MODE director=DIRECTOR_ID epoch=EPOCH` for a response audit. Settlement records disposition and sends zero bytes; it neither grants a native answer nor releases a possibly transmitted episode claim.

#### Error Messages

Application errors print as stderr text even with `--json`; the flag selects successful output. The permission-response exception (§6.3 Permission commands) also prints a structured held/rejected/uncertain response to stdout before exit 1 and its stderr diagnostic. Input errors use the documented usage/application category. Dispatch, input-lock, compatibility and deletion-lock errors are specified at their action owners above.

| Command | Situation | Error message | Exit | Notes |
|---|---|---|---|---|
| (any surface resolving a config dir) | A set config-path variable holds a non-absolute value (empty string and relative paths included) | §6.3 Config-dir resolution validation | 1 | `doctor` reports it as the agent's configuration check instead |
| (any non-setup command) | A missing, empty, behind, ahead or incompatible database | §6.3 Schema-version guard | 1 | — |
| `member create` | `--coding-agent opencode` with the selected agent preset missing or unreadable at the resolved preset path | `Error: cannot read opencode agent preset at <preset>: <cause>; run 'cafleet setup --coding-agent opencode' first` | 1 | `<preset>` is the resolved `<preset base>/agents/<mode>.md` for the resolved launch policy, and `<cause>` is the read error; fires before registration or multiplexer effects |
| (any subject-taking command) | Missing positional subject id | The parser's native missing-required-argument error naming the positional | 2 | — |
| (any id argument) | A non-integer id | The parser's native invalid-value error | 2 | — |
| `member create` | Unknown `--fleet-id` | `Error: Fleet '<fleet-id>' not found.` | 2 | Director auto-discovery runs first thing |
| `member create` | Into a soft-deleted fleet | `Error: fleet X is deleted` | 1 | — |
| `member create` | The fleet row has no `director_member_id` recorded | `Error: fleet <fleet-id> has no root Director recorded; re-create the fleet with 'cafleet fleet create'.` | 1 | Mid-bootstrap corruption |
| `member create` | The fleet's root Director is not an active member | `Error: fleet <fleet-id>'s root Director (member <id>) is not active.` | 1 | The live-fleet guard |
| `member create` | The fleet's root Director has no concrete placement | `Error: fleet <fleet-id>'s root Director (member <id>) has no concrete placement.` | 1 | The live-fleet guard |
| `member delete` | Against the root Director's id | `Error: cannot deregister the root Director; use 'cafleet fleet delete' instead` | 1 | — |
| `message ack` / `message show` | An unknown `MESSAGE_ID` | `Error: Message <message-id> not found` | 1 | — |
| `message ack` | A message not in the `input_required` state | `Error: Cannot ACK message in state <state>` | 1 | — |
| `message send` | The sender is unknown or inactive | `Error: Sender member not found or not active: <from-member-id>` | 1 | — |
| `message send` | The recipient is unknown or inactive | `Error: Destination member not found: <to-member-id>` | 1 | — |
| `message send` | Sender and recipient in different fleets | `Error: members <from-member-id> and <to-member-id> are not in the same fleet.` | 1 | — |
| `member capture` / `member prompt` / `member exec` | The member has a pending placement | <code>Error: member &lt;id&gt; has no pane yet (pending placement) — nothing to &lt;capture&#124;prompt&#124;exec&gt;.</code> | 1 | — |
| `member show` / `prompt` / `exec` / `capture` | An unknown or inactive target member id | `Error: Member <member-id> not found` | 1 | — |
| `member prompt` / `exec` / `capture` | A target with no placement row | ``Error: member <member-id> has no placement row; it was not spawned via `cafleet member create`.`` | 1 | — |
| `message send` / `member create` | Neither the positional body nor `--file`, or both | The parser's native argument-group error | 2 | — |
| `message send` / `member create` | Positional body empty or whitespace-only | `Error: text may not be empty.` | 2 | — |
| `message send` / `member create` | `--file <path>` to an empty (zero-byte or whitespace-only) file | `Error: --file <path>: file is empty.` | 1 | — |
| `message send` / `member create` | `--file -` with empty or whitespace-only stdin | `Error: --file -: stdin is empty.` | 1 | — |
| `message send` / `member create` | `--file <path>` to a non-existent path or non-regular file | `Error: --file <path>: file does not exist or is not a regular file.` | 1 | — |
| `message send` / `member create` | `--file <path>` to an unreadable file | `Error: --file <path>: file is not readable.` | 1 | — |
| `message send` / `member create` | `--file <path>` to a file containing invalid UTF-8 | `Error: --file <path>: file is not valid UTF-8.` | 1 | — |
| `member create` | A `--model` or `--effort` value the selected backend rejects | §6.7 *Model selection* and *Reasoning effort* | 2 | Fires before any side effect |
| `member create` | An unknown `{placeholder}` in the prompt | `Error: Unknown placeholder '<name>' in custom prompt. Supported placeholders: {fleet_id}, {member_id}, {director_member_id}, {coding_agent}. Double literal braces ({{, }}) to keep them as text.` | 2 | Before registration |
| `member create` | A malformed brace expression in the prompt | `Error: Malformed custom prompt: <detail>. Double literal braces ({{, }}) to keep them as text.` | 2 | Before registration |
| `member create` | `--coding-agent` omitted and the spawning Director not found in the fleet | `Error: Director (member <director-id>) not found.` | 1 | Nothing spawned |
| `member create` | `--coding-agent` omitted and the spawning Director has no placement row | `Error: Director (member <director-id>) has no placement; supply --coding-agent.` | 1 | Nothing spawned |
| `permission inspect` | An unknown `INTERACTION_ID` | `Error: Interaction <interaction-id> not found` | 1 | Reported before any event, run or database-identity check |
| `permission inspect` | The inspected run records a database identity other than the selected database's | `Error: invalid stored value for member_runs.database_identity_json: selected database identity mismatch` | 1 | Followed by the known interaction context |

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

Output shapes owns each command's wrapper. Poll returns an array of full records; show/ACK wrap one record as `message`; send adds `notification_sent`. Wait returns full poll records inside its finite receive result. Listen emits that same full record inside a tagged `message` envelope, with separate `ready`/`closed` lifecycle records and explicit ACK consumption.

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

Text bodies use `CAFLEET_MAX_TEXT_LEN` codepoints plus `…`; JSON preserves the complete exact text. A pane notification line folds the full body under its own line contract, independent of `CAFLEET_MAX_TEXT_LEN`.

#### Pane preview forms

The wrapped and explicitly unwrapped pane preview lines, their body folding, inline limit and long-body `message show` pointer are specified in §6.5 Multiplexer *Push notifications*.

#### Runtime observation messages

A runtime notification is an ordinary inbox record whose `text` is the complete versioned event JSON. The pinned observed member is its sender, and the Director is its recipient; verify the durable event/message association through `event show` before acting. Its typed source distinguishes runner observation from model-authored speech. Version-2 permission-opening events carry a broker-built review instruction and inspection argv, validated against the stored event/Director association. When the evidence's `operation_rows` reports an operation shown over several displayed rows, the instruction adds a sentence asking the Director to confirm the command against the excerpt, because a row break may be a wrap or a line break. ACK consumes the message independently of the attributed decision, answer submission, event visibility and actual continuation. §6.9 Persistence, notification and read interfaces owns evidence policy, atomic association and archived recovery.

#### Capture snapshot

A capture timestamps the read boundary in local UTC and hashes the final selected content’s UTF-8 bytes as lowercase SHA-256 hex. Plain mode strips ANSI and defragments carriage-return redraws; ANSI mode preserves styling. Text prints selected content without adding a newline. JSON keeps content, timestamp and hash in order. Capture content is ephemeral.


#### `strip_ansi` regex and CR-defrag

The CSI regex is exactly `\x1b\[[0-?]*[ -/]*[@-~]`: ESC, `[`, any run of
parameter bytes (`0x30`–`0x3F`), any run of intermediate bytes (`0x20`–`0x2F`),
then one final byte (`0x40`–`0x7E`). This matches **CSI sequences only** — OSC,
DCS, and single-character escapes are deliberately **not** stripped. After
stripping CSI matches, split on `\n` and, per line, trim the trailing run of
`\r` — a carriage return with nothing written after it, such as the CR of a CRLF
row terminator, overwrites nothing — then keep only the substring after the
**last** remaining `\r` (CR-redraw defrag: a TUI redraw `prefix\rNEW` keeps only
`NEW`, and a CRLF-terminated `row\r\n` keeps `row`), then re-join with `\n`. An
empty/falsy input returns unchanged.


#### Formatter helper semantics

Ordinary absent member/fleet cells use ASCII `-`. ISO-to-HMS takes the first eight characters after `T`; absent/non-string/no-T values render `-`. Idle renders null as `-`, seconds below 60, floor minutes below 3600 and floor hours afterward. Required fields fail if absent; nullable activity and pending placement are documented valid cases.

### 6.5 Multiplexer (tmux + herdr)

CAFleet hosts coding-agent sessions in tmux or herdr panes. Pane IDs are opaque strings passed verbatim. The adapters preserve pane ownership, layout, literal input and bounded subprocess behavior.

#### Backend matrix

| Behavior | tmux | herdr |
|---|---|---|
| Pane ID example | `%7` | `w1:p1` |
| Access | CLI subprocess | CLI subprocess |
| Spawn working directory | Splitting pane inheritance | Explicit `--cwd` |
| Capture for `member capture` | `capture-pane -p -e` | `pane read --format ansi` |
| Unwrapped input | Optional Escape lead, literal text, settle, Enter | Optional Escape lead, literal text, settle, Enter |
| Wrapped input | Pinned PTY runner control socket | Pinned PTY runner control socket |
| Delete layout | Native auto-fit | Best-effort rebalance scoped to the closed pane’s tab |

#### Backend selection

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
identifiers (§6.3 `cafleet doctor` — Environment Diagnosis).


#### Error taxonomy

Both backends report failures through one multiplexer error type carrying the
backend's own message text, so CLI boundaries handle both uniformly. A creation
error additionally records its pane-cleanup outcome (below), and a key-dispatch
error records whether a writing process had started.


#### Pane ownership during creation

The split owns a newly created pane until it returns successfully. As soon as
herdr extracts the pane id from the split response, it takes ownership of that
pane. If the subsequent `pane run` fails, herdr closes the pane, tolerating an
already-missing pane, before returning the original run error. A failed close
retains both the pane id and the run error and adds the cleanup failure.
Layout equalization remains best-effort and does not fail an otherwise
successful spawn.

A creation error carries one of these pane-cleanup outcomes:

| Outcome | Meaning and caller action |
|---|---|
| Closed | The backend already closed the known pane; the caller performs the remaining database compensation without another close |
| Close failed | The backend tried to close the known pane and failed; the error appends `cleanup failed for pane <id>: <detail>`, and the caller performs the remaining compensation without retrying the close |
| Unknown | A failed or malformed split response left no confirmed pane id; the error appends `pane ID unknown; pane cleanup unconfirmed`, and the caller closes no pane |

On success, ownership transfers immediately to member creation, whose
compensation follows the
creation failure order (§6.3 Creation failure compensation)
and closes only the pane owned by that operation.


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


#### Pane spawn working directory

A member pane spawned by `cafleet member create` starts in the invoking
process's working directory (the Director's pane cwd); each backend realizes
this differently, per the §6.5 Backend matrix.

On herdr, `--cwd` receives the invoking process's current working directory.
herdr's own inheritance is not relied upon because herdr spawns `/bin/sh`
instead of the passwd login shell when `SHELL` is unset
([herdr discussion #1517](https://github.com/ogulcancelik/herdr/discussions/1517)).
An unresolvable cwd fails the spawn with `cannot resolve the working directory
for pane spawn: <cause>`; there is no fallback directory.


#### Delete-time pane layout

Closing a member pane leaves the two backends asymmetric on layout reflow, per
the §6.5 Backend matrix.

herdr's pane close reads the target pane's tab (`herdr pane get`) before the
close, runs `herdr pane close`, then rebalances (§ herdr column equalization). The scoping comes from the
layout read itself: the killed pane is gone, so the rebalance picks a pane
still open in that tab (`herdr pane list`) and anchors the geometry read on it
(`herdr pane layout --pane <surviving>`), which returns that tab's layout
regardless of which tab or pane holds focus. When no pane remains in the tab,
there is nothing to rebalance and the step is skipped. With ≥ 2 members
remaining, the member column is re-equalized to equal heights (the same
invariant the create path enforces); after the last member is deleted, the
Director pane is explicitly restored to full tab width when the layout read
shows a residual right split; a single remaining member needs no resize. The
rebalance uses the original operation deadline after registry finalization.
A failed or expired restoration emits a stderr diagnostic; `member delete`
retains its successful close and deregistration result.


#### Pane input lock

Every operation that writes pane input uses one process-shared OS file lock. Acquire the guard before placement revalidation. For wrapped prompt/exec/preview submissions the runner owns the lock: wrapped clients validate and submit without holding it while awaiting the runner, and the runner acquires it nonblockingly from its event loop under the attempt deadline, revalidates the concrete placement and retains it across an active staged write through durable receipt/stamp settlement (§6.9 Runner-owned deferred delivery). A merely queued request holds no lock. Explicitly unwrapped submissions hold it through the Escape lead, literal text, settle, Enter and the notification stamp of the last drained message. Explicit key requests hold it through their outer-pane dispatch and bounded cleanup. Close the non-cloned owned handle on every path.

| Property | Contract |
|---|---|
| Directory | `<canonical-database-file>.pane-locks/` adjacent to the database |
| Key | SHA-256 of UTF-8 `backend + NUL + mux_session + NUL + mux_pane_id` |
| File | `<key>.lock`, read/write create without truncation, owner-only creation permissions |
| Acquisition | Exclusive nonblocking OS file lock, retried every 50 milliseconds while another writer holds it; preserve unexpected errors with path/member context |
| Identity | Keep lockfiles stable while processes may use the database; unlinking a held file would let another process lock a different inode for the same pane |
| Contention | Unwrapped prompt/exec/preview submission reserves 3.5 seconds, leaving at most 1.5 seconds to wait. The runner retries a busy lock every 100 milliseconds within its five-second attempt deadline. Explicit keys use their dispatch reservation (§6.5 Explicit key dispatch); a lock still busy at its wait bound fails that writer |

The unwrapped 3.5-second input reservation covers a 0.5-second revalidation margin, the 0.5-second Escape settle, a 0.5-second minimum literal budget, the one-second submit settle and one second for Enter. A writer that acquires the lock within its wait therefore still submits; a lock released just before the wait ends still leads to a successful submission. No database statement runs while a writer waits for the lock.

| Writer | Lock still busy when the wait ends |
|---|---|
| `message send` / `message wait` delivery | `message MESSAGE_ID was not notified to member MEMBER_ID: pane input remained busy; the next send to member MEMBER_ID or the next message wait retries it.` |
| `member prompt` | Exit 1: `member ID's pane input remained busy; prompt was not dispatched.` |
| `member exec` | Exit 1: `member ID's pane input remained busy; native command was not dispatched.` |
| `member press` | Exit 1 with the primary and zero-dispatch diagnostics (§6.3 `member press`) |

When a delivery's lock wait ends, one short read selects the oldest pending message this invocation has not attempted for the recipient. When none remains, the attempt reports nothing. Otherwise the lock-busy diagnostic names that message.

Duplicate placements for one pane share the same lock key. Process exit releases ownership. Read-only capture needs no input lock.

Member deletion locks its concrete placement before killing/deregistering. Fleet deletion acquires active placement keys in sorted order and revalidates the placement set before ending its members and again in its final transaction. Retry contention every 50 milliseconds against one operation-wide five-second monotonic deadline. A changed set releases all guards and restarts within the same deadline. On timeout release all acquired guards and leave registry/panes unchanged.

Errors are `member ID's pane input remained busy for 5 seconds; member was not deleted.` and `fleet ID pane input remained busy for 5 seconds at member MEMBER_ID; fleet was not deleted.`

#### Termination during pane input

Interrupting a host turn can also end the turn's in-flight shell tool, including a running `cafleet` command. The CLI therefore blocks `SIGINT`, `SIGTERM` and `SIGHUP` across every pane-input and lifecycle span; a signal received meanwhile takes its default action when the span ends. Spans nest and restore the outer signal mask.

| Command | Span |
|---|---|
| `message send`, each `message wait` attempt | From lock acquisition through the last `notified_at` stamp |
| `member prompt`, `member exec` | Unwrapped: from lock acquisition through Enter; wrapped: from the runner request through its reply |
| `member press` | Begin before acquiring the lock; retain it through the writing process and bounded cleanup, releasing the lock before restoring signals |
| `member create` | From the pane spawn through the registry commit or its compensation |
| `member delete`, `fleet delete` | From lock acquisition through deregistration and layout cleanup |

Every span is bounded by its existing deadline, so a deferred signal waits at most one span. `message wait` outside an attempt keeps default signal behavior. Multiplexer client processes started inside a span inherit the blocked set and stay bounded by their timeouts. The spawned member session is started by the multiplexer server and does not inherit it.

`SIGKILL` cannot be deferred. A kill inside a span leaves:

| Killed command | State left | Discovery and recovery |
|---|---|---|
| `message send` / `message wait` between first keystroke and stamp | `notified_at` stays NULL; the line may or may not have reached the pane | The next send or `message wait` retries it; a duplicate line is possible |
| `member prompt` / `member exec` after typing began | Partial or complete input | `member capture`; re-dispatch only after inspecting the pane |
| `member press` after its writing process starts | Any prefix, including empty or complete, may have been handled | Inspect the pane before an explicitly authorized new action |
| `member create` | A pane without a registry row, or a row whose placement stays pending | `member list` and `member capture`; `member delete` the row, and close an unregistered pane with the multiplexer's own kill command |
| `member delete` / `fleet delete` | A closed pane whose row remains, or the reverse | `member list`; rerun the delete |

#### Explicit key dispatch

`member press` maps the complete validated ordered vector from the CLI contract (§6.3 `member press`) to one writing invocation: `tmux send-keys -t PANE KEY...` or `herdr pane send-keys PANE KEY...`. The typed registry owns the common grammar and mappings below. Single printable characters preserve case; named keys and modifiers match case-insensitively, and control letters normalize to lowercase.

| Common key | tmux argument | herdr argument |
|---|---|---|
| Single printable ASCII character, U+0021–U+007E | `0xXX` for its ASCII codepoint | Same character, with `+` as `plus`, `-` as `minus`, and `;` as `semicolon` |
| `Space` | `Space` | `space` |
| `Escape` | `Escape` | `esc` |
| `Enter` | `Enter` | `enter` |
| `Tab` | `Tab` | `tab` |
| `Backspace` | `BSpace` | `backspace` |
| `Up`, `Down`, `Left`, `Right` | Canonical capitalized name | Lowercase name |
| `F1` through `F12` | `F1` through `F12` | `f1` through `f12` |
| `Ctrl+a` through `Ctrl+z` | `C-a` through `C-z` | `ctrl+a` through `ctrl+z` |
| `Shift+Tab` | `BTab` | `shift+tab` |

Accept only these productions, with the CLI's count/UTF-8 bounds and complete-vector validation. Tmux hexadecimal character arguments keep punctuation out of its command parser. The dedicated key adapter sends only requested keys, using shell-free argv and no prompt phase or settling sleep.

Acquire the same canonical pane-input lock used by deletion and validate its retained descriptor/path binding. Compare the configured/environment adapter with the recorded backend. Within that selected live context, require the current session/workspace to match the placement, then verify its complete target tuple: tmux reads `#{pane_id}|#{window_id}|#{session_name}` for the target and splits only the first two separators; herdr reads its pane-list JSON and requires exactly one matching workspace/tab/pane. Re-read the active member after metadata, compare its fixed fleet/kind/full placement and validate the held lock immediately before writing. A missing, ambiguous, malformed or changed target fails before the writing invocation.

The placement records no historical socket/server-generation identity. Context validation establishes its complete tuple within the selected current server; it provides no cross-server fallback or proof of historical server/build equivalence. Keep that same adapter/environment across every probe and the write.

Capture one inherited-or-new five-second absolute deadline before invocation database validation. The full availability/current-context/target-metadata/write chain uses absolute `run_until` execution with that supplied bound; relative or untimed helper execution supplies no press path. Retain one second for metadata/dispatch/cleanup when computing the lock-wait deadline, rather than the prompt reservation. Every later check uses the remaining original budget, and insufficient launch/cleanup reserve starts no new writing process.

Within the original deadline, reserve 250 milliseconds for owned process cleanup and end collection/launch at that earlier bound. On an actual error, close read pipes, kill only the still-owned unreaped direct child and poll `try_wait` with sleeps at most 2 milliseconds while the original cleanup deadline has time remaining, checking that deadline after every interruption. Explicit cleanup and unwind fallback use the same original deadline and perform no blocking wait. An explicit cleanup attempt prevents a second Drop retry cycle; unconfirmed termination/reap remains an explicit diagnostic: `cleanup unconfirmed for child PID: original command deadline expired before reap`. Preserve actual kill/status errors and the primary process error; unwind emits cleanup diagnostics to stderr. Existing relative/untimed execution retains its separate behavior. Synchronous calls have no hard scheduler/syscall return-time guarantee.

Both writing adapters acknowledge only successful process completion with empty/whitespace stdout. Unexpected non-whitespace stdout on success is unreadable acknowledgment with uncertain effects; herdr metadata retains its command-specific JSON parsing. Binary-not-found/not-started means zero writing invocations; every started failure, including pane-gone stderr, timeout or unreadable output, retains possible prefix delivery and receives no automatic retry. Backend keyboard modes or key interception can affect actual PTY bytes even after acknowledgment. Use the CLI result/error contract (§6.3 `member press`) and observe actual native effects separately.

#### Prompt dispatch

The sequences below specify the explicitly unwrapped multiplexer path. Runner-managed members use a pinned, healthy control socket and their runner's owned child PTY under the runtime input contract (§6.9 Input arbitration and the original native answer). The runner submits through the selected profile's measured plan without Escape into a verified empty composer. It queues ordinary input with zero bytes while a detected interaction is a candidate or open, or while a composer with an executable plan is not verified empty; the latter falls back with `idle_timeout` after 3 minutes. Without an executable plan it uses Escape-first delivery fallback (§6.9 Delivery method and interaction priority). Uncertain reachability preserves manual terminal use while holding generated input.

Validate a nonblank literal line without CR/LF or C0/DEL control characters before starting any process. No input path captures the pane or consults its state. Every submission selects a lead:

| Lead | Input sequence | Used by |
|---|---|---|
| Escape | Escape, 0.5-second settle, literal line, settle, Enter | `member prompt`, `member exec` and the first notification of a drain |
| Direct | Literal line, settle, Enter | Each further notification drained in the same lock hold |

Escape is meant to interrupt a working turn or close an open prompt so the line reaches the composer. A further drained line uses the direct lead because a second Escape would interrupt the turn the first delivered line just started; the host queues or appends it.

| Input | tmux argv | herdr argv |
|---|---|---|
| Escape | `tmux send-keys -t PANE Escape` | `herdr pane send-keys PANE escape` |
| Literal | `tmux send-keys -t PANE -l -- TEXT` | `herdr pane send-text PANE TEXT` |
| Enter | `tmux send-keys -t PANE Enter` | `herdr pane send-keys PANE enter` |

Native shell input starts with `! ` followed by the validated actual command. Composer controls begin the submitted input. Tmux uses the `--` option terminator before its literal payload to preserve leading hyphens. Herdr receives the complete payload as one TEXT argument immediately after the pane ID.

Opencode `mini` receives pushed notifications and direct prompts as submitted user turns. Native `! ` input executes a shell command. Poll returns the full persisted message, and ACK consumes its ID.

Deletion and prompt/exec operations start their five-second budgets after the invocation's schema-version guard. Resolve native command bodies before starting their input budget. Each submission has one five-second transport budget:

1. Reserve the one-second settle delay and one second for Enter, plus the 0.5-second Escape settle for the Escape lead. When no time remains beyond the reservation, fail with `insufficient pane input budget for literal input, settle and Enter` and start no input.
2. For the Escape lead, send Escape bounded by one second or the unreserved time, whichever is shorter, then wait the Escape settle. Any Escape failure (launch, missing pane, nonzero exit or timeout) reports no dispatch, because no text was typed.
3. Recompute the literal budget as the remaining time minus the settle delay and Enter. When Escape consumed the slack, fail with the same insufficient-budget error and type no text. Otherwise type the literal line within that budget, wait the settle delay and submit Enter.

A confirmed launch failure or recognized missing pane reports no dispatch. A started text write with an ambiguous result, or any failure after successful literal input, is uncertain. A failure after Enter can mean the command ran. Inspect the pane before authorizing any retry.

The lock excludes CAFleet writers; the user's keyboard and host UI can change at any time. Successful dispatch establishes complete submission only.

#### Push notifications

Active ordinary recipients with concrete panes receive previews through their recorded transport. The root Director receives through native `message listen` (§6.3) or manual broker reads, with zero automatic pane input. Self-sends persist without pane input.

The canonical wrapped preview, which profile delivery submits unchanged, is this exact line:

```text
[cafleet msg MESSAGE_ID from SENDER_ID TIMESTAMP] BODY [cafleet] Resume your work if something was still running.
```

Wrapped delivery fallback (§6.9 Delivery method and interaction priority) and explicitly unwrapped previews use the full line:

```text
[cafleet msg MESSAGE_ID from SENDER_ID TIMESTAMP] BODY [cafleet] Resume your work if something was still running. [cafleet] If this delivery's Escape dismissed a pending prompt in your pane, that rejection did not come from the user; re-issue the tool call if you still need it.
```

Stored text remains exact. Both forms fold CRLF/LF/CR to `⏎`, tabs to `␉`, and other C0/DEL controls to visible hexadecimal escapes (`\xNN`). A folded body of at most 4000 Unicode scalar values appears inline; a longer body becomes `Body is N characters; run cafleet message show MESSAGE_ID --json to read it.`, where N counts stored text. This limit is independent of `CAFLEET_MAX_TEXT_LEN`. Every profile-delivered line ends with the resume clause; every fallback or unwrapped line, whose submission leads with Escape, carries both clauses. The runner renders the wrapped form from the selected delivery method before its first byte (§6.9 Delivery reporting).

One synchronous drain owns the shared pane lock and five-second attempt budget. Commit short read snapshots before socket or multiplexer I/O.

1. Read recipient/message eligibility, excluding the root Director before preview selection or pane locking. An inactive/ineligible recipient, fleet mismatch or no pending row ends the attempt; pending placement holds it.
2. For wrapped delivery, query durable key `preview:<run_id>:<message_id>` before live input eligibility. Conflicting reuse fails; in-progress or uncertain holds for inspection, a queued ready receipt returns its queued result, and a settled key returns its existing disposition without replay. An accepted receipt permits settlement without typing. Capture one expiration from the original deadline for every path that will submit, queue, settle or retire, including retained accepted/submitted history.
3. For unwrapped delivery, acquire the pane input lock, then revalidate activity, member kind, fleet, concrete placement, message status and notification stamp. Wrapped delivery performs the same revalidation without the lock, and the runner acquires and revalidates it for the actual stages. Changed placement holds delivery.
4. Resolve the recorded transport. A wrapped new input attempt requires a fresh pinned identity/epoch and socket health; retained accepted history proceeds to settlement. An unwrapped placement resolves the multiplexer and checks availability.
5. Select the oldest unattempted pending message by `(created_at,message_id)` and revalidate the recipient/placement. Each invocation attempts a selected message at most once. Query the wrapped receipt for each later candidate before current input eligibility; unwrapped delivery retains the same owned lock.
6. A wrapped new attempt sends the canonical base preview to the runner, which reserves the receipt, then submits it immediately, queues it with zero bytes or holds it (§6.9 Delivery method and interaction priority). An immediate submission commits its durable in-progress row and selected method before child bytes, then records accepted or uncertain submission. Reuse the captured expiration for submission and any later retirement.
7. Settle wrapped accepted delivery in one transaction that stamps `notified_at` or verifies ACK and marks the receipt settled/submitted. The serialized service verifies that same durable settled/submitted receipt and excludes every active pending writer before removing an unleased cache entry with no available attempt permit. Retirement uses the remaining original expiration and sends zero native bytes. Failure after possible input preserves the actual receipt state and notification stamp for reconciliation.
8. For unwrapped delivery, reserve the lead/settle/Enter time plus the minimum literal budget before input. Use Escape for the first line and direct input for later drained lines, then stamp `notified_at` after full submission; an intervening ACK counts as notified.
9. Continue at step 5 while the budget and input state permit. Each new wrapped attempt is evaluated afresh by the runner; a request queued behind an earlier queued input receives the same queued result.

A retirement failure after settlement preserves the committed sent prefix and `notification_sent=true`. Report `message MESSAGE_ID was notified to member MEMBER_ID; input INPUT_ID is settled/submitted; receipt retirement failed: CAUSE`, retaining known run/input/key context and the original cause. This warning uses the existing attempt error shape; durable settlement remains authoritative and grants no submission replay.

The lock owner retains the lock through final submission/settlement. Wrapped delivery follows the durable input contract (§6.9 Input arbitration and the original native answer); only absent receipts or certified ready/zero-byte state permit a new eligible attempt. Explicitly unwrapped delivery remains at-least-once: a failed stamp leaves the message pending, so later send/wait may repeat partial or complete input. Poll and ACK identify consumption independently of either transport.

The notification diagnostic (§6.3 Delivery outcome) applies to failed input attempts and database contention, retaining actual causes and the committed message. Mandatory finalization uses the later of the original deadline and one second after finalization begins, while the transport remains within its original deadline. Other fatal storage errors retain their cause; a send preserves its committed result. A held state remains pending for external retry and reports no successful notification; a queued preview is retried by the runner.

Delivery leaves `status_state` unchanged. An inline or pointer preview remains `input_required` until its recipient consumes and ACKs the message ID.

##### Held delivery

A wrapped preview arriving while a detected interaction is a candidate or open, or behind an earlier queued input, is queued by the runner with zero bytes and a durable ready receipt; the runner retries it after the interaction closes (§6.9 Runner-owned deferred delivery). With an executable plan, a composer that is not verified empty (a draft or an unclassifiable composer) queues it under `composer_busy` until the composer empties, falling back with `idle_timeout` after 3 minutes. A missing operation plan or missing profile coverage selects delivery fallback rather than a hold. Wrapped delivery remains held, with no queue acceptance or write, for a degraded/recovering/lost observer, unavailable ownership or storage, or an uncertain receipt/reachability. Queued and held delivery leave `notified_at` unset, create no new permission event and grant no native answer.

Pending placement and a placement change under the lock are holds. Lock contention waits for the lock within the attempt budget. A message remains pending while the lock stays busy, the pane is not yet placed, the budget runs out or an attempt fails. The next send to that ordinary recipient, or a `message wait` (§6.3) rotation, retries it. Rotation selects concrete ordinary-member placements, excluding the root Director and the wait's own subject, and attempts at most one member per interval. One invocation attempts each pending message at most once; a later invocation retries a failed one. Successful snapshots can establish ACK, departure or notification by another invocation.

Transport and stamp failures after persistence remain diagnostics alongside a successful send result. No notification outcome erases a committed delivery.

##### Accepted trade-offs

The following trade-offs apply to explicitly unwrapped input and to wrapped delivery fallback, both of which lead with Escape. What a single Escape does depends on the backend, and a backend may need more than one Escape to interrupt a running session. Delivery is correct whatever Escape does: when Escape does not interrupt, the line is still typed into the composer and the host submits or queues it. Both trailing clauses are conditional, so they stay accurate either way.

- Where Escape interrupts an ordinary member's working turn, the resume clause tells the recipient to continue.
- Where Escape dismisses an ordinary member's question, confirmation or approval prompt, the host can report a user rejection before the input arrives; Claude Code reports `The user doesn't want to proceed with this tool use.` The preview's dismissed-prompt clause marks that rejection as delivery-caused, and the member re-issues the required tool call or re-asks the question. `member prompt` and `member exec` input carries no clause; the Director checks the pane for an open prompt before that dispatch.
- Escape does not reliably clear typed composer text on every host; unsent text in an ordinary member's pane can be submitted together with the delivered line.
- A failed or interrupted notification is retried, so a recipient can see the same `[cafleet msg ID …]` line more than once; it polls and ACKs by ID.
- Two sends in quick succession from different senders each lead with Escape; the second can interrupt the turn the first delivery started.
- Interrupting a host turn can also end its in-flight shell tool; the CLI defers termination signals across its spans (§6.5 Termination during pane input).
- A wrapped runner suppresses fallback for detected and candidate interactions only. On a run with unknown or partial detection coverage, fallback can dismiss an undetected permission request or question; a changed native UI can trigger fallback while its unsupported permission answer remains unavailable. Semantic permission answers never use fallback.
- Opencode mini captures can expose only the status bar. Inspect retained session records for submitted input and native shell output when capture omits them; use broker poll and ACK to establish message consumption.

##### Delivery outcomes

`notification_sent` is `true` exactly when this invocation delivered the newly committed message. Delivering only older messages, or another invocation delivering the new message, leaves it `false` without a diagnostic. §6.3 `message send` owns exact output and errors; §6.3 `message wait` owns diagnostic revalidation and bounded retries.

#### Multiplexer context

A multiplexer context is a plain value of three non-empty strings: `session`
(the tmux session name or herdr workspace ID), `window_id` (the tmux window or
herdr tab, e.g. `@N`) and `pane_id` (e.g. `%N`). Context discovery produces it
for the calling pane. The split takes the Director's full context rather than a
bare window ID: tmux splits the reference window, herdr splits relative to the
reference pane. A missing or invalid created-pane ID is an unconfirmed cleanup,
not a license to infer another target (§ Pane ownership during creation).

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


#### Explicit key adapter surface

A validated key vector produces its canonical names plus backend-specific arguments. Each backend exposes two key operations: target validation against the recorded placement, and key sending, which accepts only a validated vector and performs the single bounded writing invocation (§6.5 Explicit key dispatch). The press entry keeps the original supplied connection and deadline and holds its validated pane lock through process cleanup.

Every press process, including the availability, current-context and target probes, runs against one absolute deadline. The process runner keeps that bound through spawn, collection and cleanup, and a nested runner clips it by any earlier retained bound. A timed run error keeps its original cause and its ordered cleanup diagnostics. Relative and untimed process execution keep their independent behavior.

#### Backend operations

Both backends run their CLI as an argv subprocess without a shell. Pane IDs are
passed verbatim. Prompt and key operations use the argv in § Prompt dispatch and
§ Explicit key dispatch.

| Operation | tmux argv | herdr argv | Failure handling |
|---|---|---|---|
| Availability | `tmux` on `PATH` and a nonempty `TMUX` | `herdr` on `PATH` and a nonempty `HERDR_ENV` | `<backend> binary not found on PATH`; `cafleet member commands must be run inside a <backend> session` |
| Context discovery | <code>tmux display-message -p -t &lt;TMUX_PANE&gt; "#{session_name}&#124;#{window_id}&#124;#{pane_id}"</code>, split on the first two <code>&#124;</code> | `herdr pane current`; `result.pane` supplies `workspace_id`, `tab_id`, `pane_id` | tmux: `TMUX_PANE is not set; not running inside a tmux pane` or `unexpected tmux display-message output: <quoted>`; herdr: `herdr <context> missing '<key>' field` |
| Split | `tmux split-window -t <window_id> -P -F "#{pane_id}" -d [-e K=V]... <command>...`, then `tmux select-layout -t <window_id> main-vertical` | <code>herdr pane split &lt;pane&gt; --direction right&#124;down --no-focus --cwd &lt;cwd&gt; [--env K=V]...</code>, then `herdr pane run <new pane> "<shell-quoted command>"` | An empty or failed split is an unknown pane-cleanup outcome; tmux ignores a layout error; herdr closes the new pane when `pane run` fails |
| Capture | `tmux capture-pane -p -e -t <pane> -S -<lines + 1000>` | `herdr pane read <pane> --format ansi --source recent-unwrapped --lines <lines + 1000>` | `capture_pane: lines must be positive, got <lines>` before any process |
| Close | `tmux kill-pane -t <pane>` | `herdr pane get <pane>`, `herdr pane close <pane>`, then the delete-time rebalance | An explicit ignore-missing teardown tolerates only a recognized pane-gone cause (tmux stderr `can't find pane` / `no such pane`, herdr error code `pane_not_found`) |

The herdr split targets the Director's pane rightward for the first member;
otherwise it splits the highest-sorting pane ID in the Director's tab (other
than the Director) downward and then equalizes the member column. It passes
the invoking process's working directory verbatim as `--cwd`; an unresolvable
working directory fails with `cannot resolve the working directory for pane
spawn: <cause>` (§ Pane spawn working directory). `pane run` submits one text
line into the pane's shell, so the argv command is rendered as a single
POSIX-shell-quoted string. Every herdr JSON reply is parsed as a `result`
envelope; non-JSON output fails with `herdr returned non-JSON output: <quoted>`
and a reply without a `result` object with `herdr output missing 'result'
object: <quoted>`.

Process failures preserve their context in the multiplexer error:
`<backend> binary not found: <detail>`, `<backend> command was not started:
<detail>: <argv>`, `<backend> command timed out after <N>s: <argv>` (or
`<backend> command timed out at the pane input deadline: <argv>` for a
deadline-bounded call) and `<backend> command failed: <argv>` followed by a
`stderr: <trimmed stderr>` line. Other errors propagate.

#### herdr column equalization

herdr has no single reflow command, so it rebalances the member column
arithmetically, emulating tmux `main-vertical`. A layout read
`herdr pane layout --pane <anchor>` returns `result.layout` with
`panes[].rect{x,y,width,height}` and `splits[].{direction,rect,ratio}` for the
anchor pane's tab, so the read needs no tab-ID comparison. The member column is
every pane whose `rect.x` is not the minimum x (the Director column), sorted by
`y`; its `down` splits at the column's x form a right-leaning chain where split
*k* (top→bottom) separates member *k* from the members below.

Equal heights ⇔ split *k* has ratio `1/(N-k)` (top → `1/N`, …, bottom pair →
`1/2`). `herdr pane resize --amount` is a signed delta on a split's ratio, so
each split is driven to target by one resize: `delta = 1/(N-k) - ratio`
(rounded to 4 decimal places; skipped if `|delta| < 1e-3`); `delta > 0` →
`herdr pane resize --pane <member k> --direction down --amount <delta>`, else
`--pane <member k+1> --direction up --amount <|delta|>`. A chain whose `down`
split count is not `N - 1` is left untouched.

| Trigger | Anchor | Column size and action |
|---|---|---|
| A split appended a member to an existing column | The Director's own pane | Size ≥ 2: equalize |
| A member pane closed | The first pane in `herdr pane list` whose `tab_id` equals the closed pane's pre-close tab; no surviving pane skips the rebalance | Size ≥ 2: equalize. Size 1: no resize. Size 0: with exactly one pane and exactly one residual `right` split, `herdr pane resize --pane <director> --direction right --amount <1 - ratio, rounded to 4 places>` unless the delta is below `1e-3`; any other residue is left untouched |

The pre-close tab read is `herdr pane get <pane>`; any failure there skips the
later rebalance and never blocks the close. A close error that the teardown
does not tolerate propagates unchanged and no rebalance runs. Every rebalance is
best effort: its failure never fails a spawn or a delete.

### 6.6 Receiving and pane input

The receive task is the synchronous caller-owned process. Its round-robin cursor, attempted-message set and per-message diagnostics live only in memory. Check the subject/inbox and reconcile diagnostics against each successful snapshot before another attempt. Retry one eligible ordinary member other than the subject per one-second interval, excluding the root Director. A member remains eligible while it has a pending message this invocation has not attempted. Each failed attempt records its message ID in the attempted set and retains one diagnostic keyed by that ID, emitted in ascending message-ID order. Successful snapshots remove diagnostics for ACKed messages, departed recipients or messages notified by any invocation; the owned drain also removes entries for messages it notifies. Database busy or deadline exceeded before candidate selection records the same not-notified diagnostic for the selected rotation entry and adds that ID to the attempted set. A fatal error retains every notification diagnostic with its cause. Start a receive check or attempt only before the monotonic receive deadline; after expiry return the last successful snapshot reconciled with its owned attempt's typed result. A later invocation observes subsequent external changes. Submission budgets and pane input locking follow §6.5 Multiplexer.

The persistent listener owns one connection and process-local pending-ID set. Each short validated snapshot ends before stdout/backpressure or sleep; its scoped SQL progress handler and row-boundary checks enforce a fresh bounded deadline. Output preserves exact full records with checked nonblocking partial writes and flushes, restoring descriptor flags on normal return. Listening performs no preview retry, pane lock, multiplexer resolution or termination-deferral span. The full JSONL, replay, closure and deadline contract is in §6.3, message listen. Native adapters retain one handle, reconcile replay and establish process termination before recovery/renewal; goal activation/pause/budget and pending decisions retain host authority.

### 6.7 Coding-agent backends

Every member pane runs a coding-agent binary selected from the backend registry and recorded per member in `member_placements.coding_agent`. The registry may use a name-to-backend map or a backend enum; either representation satisfies the same registry contract. This section specifies the backend interface, each backend's spawn argv, model and effort validation, permission posture, bundled presets and host evidence.

#### Role requirements

| Role | Required backend capability |
|---|---|
| Director | Native goal lifecycle controls, plus background execution while supervision continues |
| Member | A compatible PTY lifecycle with an explicit detection-coverage declaration; launch, detection and composer capability are evaluated separately |

Evaluate each role independently. A backend's native automatic review is its omitted member policy where the backend offers one; interactive and fixed allow/deny policies remain explicit choices under §6.9 Launch policy and capability registry. Preserve native guards, managed policy, explicit denies, sandbox boundaries and the host's activation, pause, completion and budget authority. A backend with unknown detector/composer coverage permits manual terminal work and delivers ordinary generated input through delivery fallback. Native receive support is evaluated independently: a Director meeting both requirements may use manual `poll` or finite `wait` when its native adapter is unavailable, and background execution alone establishes no unsolicited wake or per-record reception guarantee.

The bundled backend policy catalog owns launch-mode choices and defaults, exact argv mapping, version eligibility and detection/composer profiles. The omitted native argv is specified below; an explicit native mode changes only its permission argument and preserves the remaining argv and literal assignment.

#### Interface

A coding-agent backend is a stateless value selected per member at spawn time.
Each exposes two read-only properties and four operations:

- **`name`** — the backend's registry key, a stable lowercase string (`"claude"`
  / `"codex"` / `"opencode"`). This MUST equal both the persisted
  `placement.coding_agent` value used to look the backend up and the key under
  which it is registered.
- **`binary_name`** — the executable name resolved against `PATH` (`"claude"` /
  `"codex"` / `"opencode"`).
- **`ensure_available()`** — fails if any spawn precondition is unmet, checked
  in this order: the backend's config directory resolves per §6.3 CLI
  *Config-dir resolution* (an invalid config-location variable fails first),
  the binary resolves on `PATH`, and (for backends with bundled presets) the
  selected preset file is readable. A shared helper resolves `binary_name` to
  an absolute executable path per §6.9 Member PTY runtime *Native executable
  resolution* and, on a miss, fails with `binary {binary_name} not found on PATH`.
  Preconditions read the host environment through a **spawn probe**: binary
  lookup, the home directory and `env_var(name) -> optional string`. The
  system probe reads the process environment; a test probe carries a settable
  environment that defaults to empty, so precondition tests inject
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
  window-split. Its leading token is `binary_name`; `member create` launches
  the absolute path from § Native executable resolution as `argv[0]`, the same
  path the version probe ran, and keeps every later token unchanged.

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

The blocks below are the child argv with omitted permission selection. The
leading token names the backend binary; the launched `argv[0]` is the absolute
path chosen by §6.9 *Native executable resolution*, the same path the version
probe ran. The multiplexer starts the CAFleet PTY runner, which preserves the
literal child argv, cwd and required environment. Every backend honors the
leading-`!` shell shortcut that `member exec` dispatches through, and every
member can run its broker commands from the moment it is spawned: claude
through its spawn-time allow rules, codex through the rules file and opencode
through its agent preset.

**claude** — `validate_model` passes any string through. `validate_effort`
accepts the levels in § Reasoning effort. `ensure_available` resolves the
claude config directory and checks `claude` on `PATH`. claude is the **only**
backend that honors `display_name` (via `--name`).

```
["claude", "--permission-mode", "auto", "--prompt-suggestions", "false", "--allowedTools", <rule>...,
 "--name", <display_name>]                           # explicit selection replaces "auto" with <mode>
  (+ ["--model", <model>]  if model is not None)
  (+ ["--effort", <effort>]  if effort is not None)
  (+ <prompt>)                                       # bare trailing positional
```

The `--allowedTools` values are the claude member's **spawn-time allow
rules**, one argv token per rule, so the member does not depend on any
`Bash(cafleet ...)` rule in the user's `settings.json`. Every claude member
receives exactly `Bash(cafleet message *)`, which covers its broker commands
including finite `message wait` and `message listen`; its work commands stay
under the host-configured read/edit/command rules. `--allowedTools` takes
several values, so it sits before `--name`, which ends its value list and
keeps the prompt positional. The rules are fixed at spawn: a later loss of
allow rules in the user's settings does not affect a running member. The
rules leave these cases to the host:

- A `deny` or `ask` rule at any settings level, including managed policy,
  still blocks the command.
- The managed setting `allowManagedPermissionRulesOnly` makes Claude Code
  ignore `--allowedTools`.
- A blocking `PreToolUse` hook stops a tool call before permission rules are
  evaluated.
- Settings can change after spawn, so overdue member readiness or replies are
  inspected directly.
- Work commands (`mise`, `git commit`, package managers) stay under the
  user's own rules; a denied one is routed through `member exec`.

**codex** — `validate_model` passes any string through. `validate_effort`
accepts the levels in § Reasoning effort. `ensure_available` resolves the
codex home and checks `codex` on `PATH`. `display_name` is silently ignored.

```
["codex", "--approve-for-me"]
  (+ ["--model", <model>]  if model is not None)
  (+ ["--config=model_reasoning_effort=<effort>"]  if effort is not None)  # ONE token
  (+ <prompt>)                                       # bare trailing positional
```

An explicit `on-request` or `never` replaces `"--approve-for-me"` with
`"--ask-for-approval", <mode>, "--sandbox", "workspace-write"`;
`"--approve-for-me"` selects the workspace-write sandbox itself and Codex
0.163.0 rejects it together with `--sandbox`.

**opencode** — `validate_model`: `None` is valid; otherwise split `model` on the
**first** `/` into `<provider-id>` and `<model-id>`; both halves MUST be
non-empty (§ Model selection). (`"openai/gpt-4"` accepted; `"a/b/c"` accepted
as provider `a` / model `b/c`; `"a/"`, `"/b"`, `"abc"` rejected.)
`validate_effort` rejects **every** non-None value (§ Reasoning effort) — the
backend has no reasoning-effort control, so `build_spawn_argv` never receives
a non-None `effort` and never emits effort tokens. `member create` resolves the
launch policy before `ensure_available`, which first resolves the preset base
per §6.3 *Config-dir resolution* (`OPENCODE_CONFIG_DIR`, default
`~/.config/opencode`), so an invalid variable fails before the PATH check, then
checks `opencode` on `PATH`, then reads the selected preset file
`<preset base>/agents/<mode>.md` for the resolved mode (§ opencode preset).
The unselected preset is not consulted; only opencode has a required preset.
`display_name` is ignored; the prompt is passed as a `--prompt <prompt>` flag
pair (two tokens), unlike claude/codex's bare positional.

```
["opencode", "mini", "--standalone", "--agent", <mode>]   # <mode> is cafleet or cafleet-ask
  (+ ["--model", <model>]  if model is not None)
  (+ ["--prompt", <prompt>])                         # prompt via flag — TWO tokens
```

The pane runs `opencode mini --standalone`, the long-lived minimal interactive
interface with a private server per pane. The prompt travels through `--prompt`
because `mini` takes no positional message. The `--agent` value is the only
argv difference between the two launch policies; availability checks validate
PATH and the selected preset without detecting a version.
Spawn, preset loading, broker input and native shell input are validated against
opencode 2.1.4; opencode 2 is the minimum supported version.

#### Model selection

`member create --model <m>` inserts `--model <m>` immediately before the
prompt tokens. Omitted, no model tokens are emitted and the binary uses its
configured default.

| Backend | Accepted value format | Create-time validation |
|---|---|---|
| `claude` | Passes through verbatim | None; the binary rejects unknown models, so newly released models need no CAFleet release |
| `codex` | Passes through verbatim | None; the binary rejects unknown models |
| `opencode` | `<provider-id>/<model-id>`, split on the **first** `/` into two non-empty segments; model ids may contain further slashes | Exit 2 with `Error: --model for the opencode backend must be '<provider-id>/<model-id>' (got '<value>').` |

#### Reasoning effort

`member create --effort <level>` is validated against the backend's accepted
set before registration or multiplexer effects and emitted immediately after
the model tokens, before the prompt. Omitted, no effort tokens are emitted.

| Backend | Accepted levels | Forwarded as | Rejected with (exit 2) |
|---|---|---|---|
| `claude` | `low`, `medium`, `high`, `xhigh`, `max` | `--effort <level>` | `Error: --effort for the claude backend must be one of low, medium, high, xhigh, max (got '<value>').` |
| `codex` | `minimal`, `low`, `medium`, `high`, `xhigh` | The single token `--config=model_reasoning_effort=<level>` | `Error: --effort for the codex backend must be one of minimal, low, medium, high, xhigh (got '<value>').` |
| `opencode` | None; the backend exposes no reasoning-effort control | — | `Error: opencode does not support reasoning effort.` |

#### Backend capabilities

| Backend | OS-level sandbox | Sets the pane title | Shell-command posture | Preset or config prerequisite |
|---|---|---|---|---|
| `claude` | None | Yes, via `--name <member-name>` | Runs its spawn-time allow rules and every command matching `permissions.allow`; auto mode's classifier reviews the rest, and a refused command routes to the Director | A Claude Code permission profile; a permission posture, not a spawn dependency |
| `codex` | Kernel-enforced workspace-write sandbox | No; `member list` `pane_id` locates the pane | Runs commands its workspace/host rules permit; automatic review decides escalations, and a refused required command routes to the Director | `<codex home>/rules/cafleet.rules`, the `config.toml` settings below and a trusted working directory; a permission posture, not a spawn dependency |
| `opencode` | None | No; `member list` `pane_id` locates the pane | `cafleet`: deny-by-default allowlist; `cafleet-ask`: the same allowlist, with every other command opening a native permission form | The selected policy's preset `<preset base>/agents/<mode>.md`; a spawn precondition |

The omitted claude and codex policies are each backend's native automatic
approval mode: an automatic reviewer decides each operation that no native rule
settles, and a refusal returns to the member, which routes a necessary refused
command to the Director. The runner reports observed automatic refusals and
unavailable automatic review as automatic review events (§6.9 Member PTY
runtime). An explicit interactive policy may display native approval prompts.

#### Claude permission posture

Under the omitted `--permission-mode auto`, a member runs its spawn-time allow
rules and every tool call matching `permissions.allow` without review; Claude
Code's auto mode classifier reviews each other call, and a call it refuses
returns to the member as a refusal, which the member routes to the Director
like any denial. Native rules, hooks and managed policy remain authoritative:
a `deny` or `ask` rule still applies, and auto mode must be available for the
account and model. Where it is unavailable, the member can show native
permission forms, and the runner reports `automatic_review_unavailable` when
its profile recognizes the notice. Claude Code 2.1.302 in auto mode can show
its `Teach auto mode about your environment?` dialog once per configuration
directory; the runner holds it as a `question` interaction (§6.9 *Bundled
profiles and native screen fixtures*).

Explicit native modes retain their own permission behavior. Under explicit
`dontAsk`, a member runs its spawn-time allow rules, Claude Code's built-in
read-only commands, file reads inside its working directory and every tool
call matching `permissions.allow` without a prompt. Every other call — a shell
command or a file edit, including one matching `permissions.ask` — is denied
without a prompt. A write to one of Claude Code's protected paths, such as a
project's `.git` or `.claude` directory, is denied whatever the allow list
holds.

The recommended Claude Code permission profiles carry these entries:

| Entry | Profile | Needed by | Why |
|---|---|---|---|
| `Bash(cafleet *)` | Other modes | The Director | One pattern pre-approves every subcommand the Director runs |
| `Skill(cafleet)`, `Skill(cafleet-design-doc)` | Both | The Director and every member | Each loads the installed skills through the Skill tool |
| `Read(~/.claude/skills/**)` | Both | The Director and every member | Role and reference pages of an installed skill sit outside the working directory |
| `Edit`, `Write` | Both | Every member that writes files | Under `auto` a matching entry pre-approves the edit without classifier review; under explicit `dontAsk` an unmatched edit is denied. No entry covers a protected path |
| `Bash(cafleet member prompt *)`, `Bash(cafleet member exec *)`, `Bash(cafleet member press *)` under `ask` | Both | The Director | The operator confirms each explicit native pane-input dispatch; a member's own attempt reaches the `ask` rule |

The auto mode profile carries no `Bash(cafleet ...)` allow entry: the
classifier reviews each Director command that no rule matches, auto mode
approves the Director's file edits inside the working directory, and members
receive their broker commands as spawn-time allow rules. An ask rule outranks
every allow rule, so the pane-input entries hold beside `Bash(cafleet *)`.
`Edit` and `Write` apply to every session that reads the settings file; a path
rule such as `Edit(~/work/**)` confines the approval. When `CLAUDE_CONFIG_DIR`
relocates the config directory, the `Read` entry names that directory's
`skills/**`.

#### Codex sandbox and configuration

The workspace-write sandbox confines writes to the workspace under a
kernel-enforced sandbox; codex is the only backend with one. The omitted
`--approve-for-me` selects that sandbox itself and routes approval requests
through Codex's automatic review. An explicit choice adds
`--sandbox workspace-write` to its `--ask-for-approval` value: `never` disables
interactive approval prompts and `on-request` shows them. The explicit choices
inherit the user's Codex configuration, including an `approvals_reviewer`
setting that can answer escalations without a native form.

A codex spawn requires these `config.toml` settings:

| Setting | Required value | Why |
|---|---|---|
| `network_access` | `true` | The multiplexer socket counts as network access |
| `writable_roots` | Includes the CAFleet database directory | The member writes the broker database |
| `trust_level` | `"trusted"` under `[projects."<absolute workspace path>"]` | The working directory must be trusted before spawning |

#### codex rules file

The codex auto-approval rules for `cafleet` commands are a static file embedded
in the binary (source `presets/codex/cafleet.rules`) and installed to
`<codex home>/rules/cafleet.rules` — `<codex home>` resolved per §6.3
*Config-dir resolution* (`CODEX_HOME`, default `~/.codex`) — by `setup`'s
asset installation (§6.3), overwriting any existing target. The file is not a spawn precondition —
codex's `ensure_available` checks only the codex home and `PATH` — and codex loads every `*.rules`
file under its rules directory, applying the strictest matching decision, so
operator customizations live in a separate rules file in that directory. The
checked-in `presets/codex/cafleet.rules` is the content contract: one broad
`prefix_rule(pattern = ["cafleet"], decision = "allow")`, followed by one
`decision = "prompt"` rule with a justification for each Director-owned
command prefix — `member prompt`, `member exec`, `member press`,
`permission respond`, `input reconcile` and `member recover`.

Codex applies the strictest decision when more than one rule matches
(`forbidden` > `prompt` > `allow`), so the Director-owned explicit pane-input,
native-answer, input-reconciliation and run-recovery commands keep operator
approval. Commands without a more specific rule match the broad allow for
every fleet, since every ID rides past the matched prefix as a positional or
trailing argument.

A rules directory maintained without the installed file needs at least these
rules: an `allow` rule for the `["cafleet", "message"]` prefix, covering the
broker commands members run themselves (the same commands a claude member
receives as spawn-time allow rules), and `prompt` rules for the
`member prompt`, `member exec` and `member press` prefixes, which keep explicit
native pane input under operator approval. That minimum set suffices for
members, not for a Director session: codex has no classifier to review what
the rules leave unmatched, so every other `cafleet` command a Director runs
follows that session's own approval policy and sandbox. The installed file is a
superset of the minimum set.

#### opencode preset

Each opencode launch policy names an agent definition of the same name. Both
are static files embedded in the binary and installed, in this order, by
`setup`'s asset installation (§6.3), overwriting any existing target — `<preset
base>` resolved per §6.3 *Config-dir resolution* (`OPENCODE_CONFIG_DIR`,
default `~/.config/opencode`):

| Launch policy | Spawn argument | Source | Install target |
|---|---|---|---|
| `cafleet` (default) | `--agent cafleet` | `presets/opencode/cafleet.md` | `<preset base>/agents/cafleet.md` |
| `cafleet-ask` | `--agent cafleet-ask` | `presets/opencode/cafleet-ask.md` | `<preset base>/agents/cafleet-ask.md` |

The preset base is opencode's `--agent <mode>` discovery path: opencode 2.1.4
loads agent definitions from `~/.config/opencode` and from
`OPENCODE_CONFIG_DIR`, so a relocated preset remains discoverable, and
definitions with the same name can merge across those sources. `member create`
forwards a set `OPENCODE_CONFIG_DIR` into the member environment (§6.3
*Member environment*), so the member's opencode loads the preset the
precondition read.

The selected policy's preset is a spawn precondition (§ Contract error
strings): `ensure_available` reads it at the resolved install target for the
resolved mode. The remedy names the selector as the targeted repair; plain
`cafleet setup` also installs it.

Each preset's `shell` ruleset places its `"*"` base before an explicit allowlist translated from the operator's Claude Code `permissions.allow` set (`cafleet *`, its declared Git commands, file-inspection utilities and the project's cargo-backed mise tasks). Opencode selects the last matching rule: later denies for the Director-owned commands — explicit pane input (`cafleet member prompt *`, `cafleet member exec *`, `cafleet member press *`), request-bound native answers (`cafleet permission respond *`), input reconciliation (`cafleet input reconcile *`) and run recovery (`cafleet member recover *`) — win over `cafleet *`. A member's denied command receives no permission prompt, while permitted broker commands run normally. Tool permission and assignment authority remain separate; the assigned role and Director instructions determine which allowed operations a member may perform.

The `cafleet-ask` preset differs from the `cafleet` preset only in its
`description`, its shell base and the first body paragraph; every allowlist
entry, every Director-only deny, their order and every non-shell permission are
identical:

| Preset | Shell base | An unmatched shell command |
|---|---|---|
| `cafleet` | `"*": "deny"` | Is denied without a permission form |
| `cafleet-ask` | `"*": "ask"` | Opens a native permission form, which the member runtime reports to the Director |

Non-shell permissions never ask under either preset, so only a shell command
opens a form and the `question` tool raises no competing question form.

Under `cafleet-ask`, a shell permission form is a runtime decision: the member
runtime detects it, notifies the Director with an `interaction_opened` event,
and the Director answers it through `cafleet permission respond`; the member
continues from the result OpenCode returns. The OpenCode 2.1.4 profile's answer
plans and continuation behavior are in §6.9 *Bundled profiles and native screen
fixtures*. A command that wraps in the form and a reason the user wants to give
are answered manually on the native surface. Under `cafleet`, a permission popup in an opencode pane is a
regression escape: the Director captures the pane and escalates, and the
allowlist is extended only by operator decision.

Both postures are allowlists with no OS-level sandbox. Standalone
un-enumerated commands fall to the `"*"` base. Apply these limits when
choosing tools and authorizing commands:

| Surface | Example | Usage limit |
|---|---|---|
| MCP-contributed tools | — | Their participation in preset permission evaluation is unverified on opencode 2.1.4. Keep loaded configurations free of MCP servers. |
| Compound shell commands | <code>git log --stat; curl … &#124; sh</code> | Permission matching for compound lines is unverified on opencode 2.1.4. Submit one command per invocation and authorize each actual command. |
| Interpreter or hook execution via allowed tooling | `mise //cafleet:test *` executes workspace-writable test code; `git commit *` runs `.git/hooks` | Authorize the code and hooks that an allowed command can execute. |

cafleet ships no MCP stanzas. Operators MUST keep every loaded opencode
configuration free of MCP servers. Use the `codex` backend for kernel-enforced
isolation.

#### Preset file format

The checked-in `presets/opencode/cafleet.md` and
`presets/opencode/cafleet-ask.md` are the content contract. Each is a
`---`-delimited **JSON** (not YAML) frontmatter block (2-space indent,
non-ASCII preserved, top-level key order `description`, `mode`, `permission`),
a blank line, then the markdown body. `shell`/`read`/`edit` are glob→decision
maps; the other seven `permission` fields are scalar `"deny"`. The
`cafleet-ask` frontmatter equals the `cafleet` preset's except `description`
and the shell `"*"` value, and its body differs only in the first paragraph.

In each preset, the body has three paragraphs after the heading, separated by a
blank line, with no internal hard line breaks in any paragraph. Each file ends
with exactly one trailing newline.

#### Contract error strings

Model and effort rejections are in § Model selection and § Reasoning effort.
The availability errors are:

- PATH miss: `binary {binary_name} not found on PATH`
- missing or unreadable selected opencode preset: `cannot read opencode agent
  preset at {preset}: {cause}; run 'cafleet setup --coding-agent opencode'
  first`
- invalid config-path variable: `{var} must be an absolute path (got
  '{value}')` (§6.3 *Config-dir resolution*)

#### Host evidence and native-input authorization

**claude**: A separate evaluator checks the session condition against the transcript; native background work defers evaluation and returns a host result. Use one native Monitor task for Director inbox records, retaining its handle across separate events and renewing after observed source-task exit. The installed CAFleet skill's [Claude native recipe](https://github.com/himkt/cafleet/blob/main/skills/cafleet/reference/coding-agents.md#claude-native-director-reception) owns discovery, invocation, renewal and evidence limits. Preserve supplied `/goal` state and pending user decisions; manual finite reception remains available when Monitor is unavailable or denied.

Native shell probes displayed command/stdout/stderr followed by a model turn; the trigger and any second permission check were not established. The Director authorizes each routed command under its host rules.

CAFleet input cancels no Claude tool approval or `AskUserQuestion` in the root Director's pane, which receives no automatic pane input, or a detected one in a runner-managed member's pane, where input queues while the interaction is open (§6.9 Delivery method and interaction priority). Escape-first input can cancel an open prompt the runner did not detect: an explicitly unwrapped member's previews, `member prompt` and `member exec` input, and a runner-managed member's delivery fallback. Claude Code then reports a user rejection before the input arrives; the member treats a rejection followed by a `[cafleet msg …]` line ending with the Escape-dismissal clause as delivery-caused and re-asks the question or re-issues the call. A profile-delivered preview ends with the resume clause only and sends no Escape, so a rejection before it stands. A rejection followed by Director prompt or native-command input follows that input, and the Director checks the pane for an open prompt before such dispatch.

**codex**: The objective belongs to the thread, with model-callable lifecycle tools and optional token-budget accounting. Retain one native execution session handle for Director inbox reception and explicitly read it while delegated work remains, independently of whether a host goal exists. The installed CAFleet skill's [Codex native recipe](https://github.com/himkt/cafleet/blob/main/skills/cafleet/reference/coding-agents.md#codex-native-director-reception) owns tool calls, bounded reads, leases and recovery. Preserve supplied goal activation, pause, completion and budget authority. A background process alone supplies no unsolicited per-line wake after a model turn ends; fully idle Codex reconciles its inbox on the next user/native activation.

A native shell probe displayed the actual command, exit code, duration and output. Installed protocol metadata describes unsandboxed dispatch while public CLI guidance describes current approval/sandbox settings; the TUI's effective boundary and protocol path were not established. Treat native input as a Director-authorized action rather than assume a second member-tool permission check.

**opencode**: OpenCode members perform supplied assignments with their selected provider/model, workspace reads/edits, permitted tools and broker communication. The Director coordinates next steps and routed native shell commands, and owns explicit pane input dispatch. Leading-`!` input runs a native shell command in `mini`. The native path's effective permission boundary and any second member-tool permission check remain unestablished; preserve the preset's member-tool permissions and the Director's command authorization.

### 6.8 Configuration

Settings retain the selected SQLite URL, nonnegative text limit and optional multiplexer override. The process reads only the explicitly named environment variables in §6.3. Spawned members receive `CAFLEET_DATABASE_URL` and, when the creator sets it, the selected backend's config-location variable (§ *Member environment*); identity is rendered into prompt literals.

### 6.9 Member PTY runtime

#### Launch policy and capability registry

`cafleet member create --permission-mode <MODE>` selects a launch policy. Resolve the coding-agent backend first, validate `MODE` against that backend's typed launch-policy catalog, then allocate the member or pane. An omitted value selects the backend default below; an empty, unsupported or unavailable explicit value fails with usage exit `2`, naming the backend and supported choices. Selection is launch-only and is independent of Director policy; it introduces no fleet inheritance, environment default or live policy mutation.

| Backend | Omitted selection | Explicit selection mapping |
|---|---|---|
| Claude | `auto` | Use the native permission-mode value through `--permission-mode`; retain broker-tool allowance, prompt suggestions, name and model/effort boundaries. |
| Codex | `approve-for-me` | Map `on-request` and `never` to `--ask-for-approval <mode> --sandbox workspace-write` and `approve-for-me` to the single `--approve-for-me` flag, which selects the workspace-write sandbox itself. |
| OpenCode | `cafleet` preset | Map `cafleet` and `cafleet-ask` to `--agent <mode>`, selecting the agent preset of that name; explicit `cafleet` records `mode_source=explicit`. |

The omitted Claude and Codex selections are each backend's native automatic approval mode: an automatic reviewer decides the operations that no native rule settles, and the runner reports its observable refusals and unavailability as automatic review events (§ Automatic review events). Every other choice applies only through an explicit `--permission-mode`. An omitted selection is not validated against the version catalog, so a member whose probed version is newer than every profile boundary still launches with the default; a native version that rejects `auto` or `--approve-for-me` exits immediately and produces `runner_exited` as described below.

The bundled backend policy catalog is the single source of truth for each backend's launch-policy catalog, default, exact argv mapping, native-version eligibility and detector/composer/answer profiles. Help and usage errors report choices from that catalog. Claude accepts native `--permission-mode` values verified from installed help and parser evidence, including documented aliases, recorded by its version-qualified adapter. Claude Code 2.1.302 accepts canonical `default` and its `manual` alias; help displays `manual`, while the [official permission-mode reference](https://code.claude.com/docs/en/permission-modes) documents both. Preserve the explicitly requested spelling in argv and launch intent. Codex accepts the native `--ask-for-approval` values `on-request` and `never` and the `approve-for-me` choice, read from the installed `codex --help` and recorded the same way; the `approve-for-me` token names its native `--approve-for-me` flag, which routes approval requests through automatic review. The explicit `on-request` and `never` choices inherit the user's Codex configuration, including an `approvals_reviewer` setting that can answer escalations without a native form. OpenCode accepts `cafleet` and `cafleet-ask` for every native version, because both select an agent preset rather than a native permission flag; every other explicit token fails with usage exit `2`. The `cafleet` preset resolves every shell check to `allow` or `deny`; the `cafleet-ask` preset asks for every shell command outside its allowlist and its Director-only denies, so OpenCode opens a native permission form.

Creation success establishes that the child was spawned, not that its native policy was accepted. A fast native exit after spawning, including policy rejection, produces `runner_exited` with the actual code/signal and a Director inbox event; the runner preserves the chosen mode rather than retrying with another one. An error from the spawn operation itself still fails creation and follows compensation. There is no post-spawn liveness window.

Record `requested_mode` as nullable text, `resolved_mode` as required text, `mode_source` as `default` or `explicit`, and the exact non-secret permission/sandbox argv in the run record. These fields state launch intent:

| Selection | `requested_mode` | `resolved_mode` | `mode_source` | `permission_argv_json` |
|---|---|---|---|---|
| Omitted Claude | `null` | `auto` | `default` | `["--permission-mode","auto"]` |
| Omitted Codex | `null` | `approve-for-me` | `default` | `["--approve-for-me"]` |
| Explicit Codex `never` | `never` | `never` | `explicit` | `["--ask-for-approval","never","--sandbox","workspace-write"]` |

`effective_policy_json` remains null unless a tested native observation supplies it (§ Effective policy record); an installed version, argv or selected profile alone proves no effective authorization decision.

Keep launch, detection, empty-composer observation, prompt submission, native-exec submission and request-bound answer capabilities separate. Answer profiles declare support for each semantic decision independently. Profiles declare a minimum native version, rule-set version and capability-specific fixture provenance; bundled tested declarations require native captures within the profile interval and measured single-request answer stages for the capability claimed. Synthetic fixtures exercise rule and capability validation while unverified components remain unknown or unsupported. The probed version selects the profile with the greatest minimum version at or below it (§ Profile selection by minimum version); a known version below the backend's first boundary fails creation. A failed probe or a backend without bundled profiles yields `coverage=unknown` and an initial coverage notification; it still permits manual terminal operation, records the selected mode and delivers ordinary programmatic input through delivery fallback. Automatic/fixed policies remain useful even where screen detection is unknown. The `cafleet` OpenCode preset's returned denials continue through member evidence routing; returned tool responses are observable only when the native terminal exposes them.

##### Native executable resolution

`member create` resolves the backend's binary to one absolute executable path during the backend availability check, using the creator process's `PATH`. It searches the absolute `PATH` entries in order, skipping empty and relative entries, and selects the first `<entry>/<binary>` that is a regular file with an execute permission bit after following symbolic links. The selected path is used as found, without symbolic-link resolution, so a link or wrapper keeps its own name. When no entry matches, creation fails before registration with exit 1 and `Error: binary <name> not found on PATH`.

The version probe below runs that exact path, and the same path is `argv[0]` of the runner's literal child argv. The probed binary and the launched binary are therefore the same path on every multiplexer; the pane's own `PATH` and shell configuration play no part in selecting the native executable.

##### Native version probe

`member create` records the installed native version of the resolved backend. After launch-policy validation and the backend availability check, and before registration, it runs the resolved executable (§ Native executable resolution) with exactly the argument `--version`, so `claude`, `codex` and `opencode` each report their own version. The probe has null stdin and captures at most 4096 bytes each of stdout and stderr. It ends within two seconds, and those two seconds belong to the command's existing ten-second creation budget, which starts before the probe. A probe still running at its bound is killed.

The probe yields a version only when it exits with status `0` and the first stdout line has exactly one whitespace-separated token that, after stripping one leading `v`, parses as canonical SemVer `major.minor.patch` with optional prerelease and build metadata. That stripped token is recorded as the run's `native_version`. An incomplete version such as `2.1` is not a version token and is never normalized into one:

| First stdout line | Recorded `native_version` |
|---|---|
| `2.1.302 (Claude Code)` | `2.1.302` |
| `codex-cli 0.163.0` | `0.163.0` |
| `opencode v2.1.4` | `2.1.4` |
| `claude 2.1.310-beta.1+build.7` | `2.1.310-beta.1+build.7` |
| `v2.1.4 2.1.5` | Probe failure: several version tokens |
| `tool 2.1` | Probe failure: no version token |

The probed version then selects the run's bundled profile (§ Profile selection by minimum version).

A probe that cannot start, times out, exits nonzero, prints non-UTF-8 output or has zero or several version tokens records `native_version` null, `profile_version` null and `coverage=unknown`. Creation continues. The command writes one stderr line, `Warning: native version unavailable for BACKEND: CAUSE; coverage unknown.`, and leaves its stdout text and JSON unchanged. Ordinary programmatic input to that run uses delivery fallback (§ Delivery method and interaction priority). The probed version identifies the installed binary only; it establishes no effective native policy.

##### Profile selection by minimum version

Each profile declares one SemVer `min_version`, and a backend's profiles form an ordered stack of compatibility boundaries. Multiplexer, platform and permission mode introduce no additional selection axis. For a successfully parsed native version `v`:

```text
eligible = profiles where profile.backend == backend and profile.min_version <= v
selected = eligible profile with greatest min_version
```

| Native version and catalog state | Creation behavior | `profile_version` and `coverage` |
|---|---|---|
| At a boundary or between boundaries | Launch with the selected profile | Its identifier; `tested` when the profile declares detection, otherwise `unknown` |
| Newer than every boundary | Launch with the newest eligible profile | The same; there is no upper cap and no version-mismatch warning |
| Below the backend's first boundary | Exit 1 before member/run registration or pane creation | — |
| Probe failure | Continue launch with the probe warning | `null`; `unknown`, and ordinary input uses delivery fallback |
| Backend with no profile entries | Continue launch | `null`; `unknown`, and ordinary input uses delivery fallback |

A known version below the first boundary fails with `Error: unsupported native version for BACKEND: VERSION; minimum supported version is MIN_VERSION`. For example, boundaries `2.1.302` and `2.1.310` select the first profile for `2.1.308` and `2.1.310-beta.1` and the second for `2.1.310` and `2.2.0`, while `2.1.301` is unsupported. A prerelease can itself be a boundary when a measured prerelease requires a new profile.

One shared SemVer parser and precedence comparison serves profile validation, native-version admission, fixture validation and selection. Numeric components compare numerically, prerelease identifiers by SemVer precedence, and a release ranks above its prereleases. Build metadata contributes no precedence. Selection is independent of catalog order.

Detection, annotations, empty-composer observation, prompt submission, native-exec submission and request-bound permission answers all resolve through the selected profile, and their coverage declarations stay independent: selecting a profile or observing an empty composer alone supplies no executable plan or answer capability. The run retains the actual probed `native_version` and selected `profile_version`. A composer plan and an answer capability retain the actual run version, the selected profile identifier and the cited fixture identities, while each fixture record keeps its original capture version; input validation compares these identities with the run rather than requiring the run version to equal a capture version or boundary.

The runner pins its selected profile for its lifetime. Fresh creation and observer initialization verify that deterministic selection agrees with the recorded profile and coverage. Historical run, event and audit reads preserve their recorded identities; recovery reports an unavailable or mismatched historical profile explicitly rather than attaching another profile. A catalog update affects newly launched runners only.

The rules match screen patterns, so a changed native UI fails safe for detection: an unmatched form yields the `unknown` or `unknown_interaction` observation and offers no answer plan, and an unverified composer selects delivery fallback for ordinary input. An answer plan writes only after its request rule, choices and gates match the fresh observation.

Native evidence is the provenance of a profile interval. Catalog validation requires every cited fixture's backend to match its profile and its capture version to lie at or above the profile's `min_version` and, when the backend has a next boundary, below it. Each subsequent native release within the interval reuses the declaration without another exact-version capture. A new profile is added only when an observed native change requires different rules or plans, with `min_version` set to the first native version requiring the change.

##### Bundled profiles and native screen fixtures

The bundled catalog is embedded in the CAFleet binary together with the native screen fixtures that its declarations cite. Each profile's capabilities rest on accepted native captures taken on a native version within its interval. A bundled profile entry has required `backend`, `min_version`, `profile_version`, `rules` and `capabilities`; each fixture entry keeps its own `native_version` as the actual capture version. Each existing profile's `min_version` is the native version its captures were taken on, and adding operation plans to an existing entry changes its contents without introducing a new boundary.

Each automatic-mode profile replaces its predecessor as its version's bundled profile and keeps every predecessor rule and answer plan unchanged; earlier runs keep their recorded `profile_version` text. Codex 0.163.0 shows no startup notice or opt-in dialog for `approve-for-me`, so its profile declares no startup rule. Claude Code 2.1.302 shows its `Teach auto mode about your environment?` dialog at most once per native configuration, described below, and the profile declares the question rule `claude-auto-startup` for it.

Profile `claude-2.1.302-auto-1`, with `min_version` `2.1.302`, applies to Claude Code under every Claude launch policy:

| Capability | Status | Declaration |
|---|---|---|
| Detection | `tested` | Rule `claude-bash-permission`: the native `default` Bash permission form (`Bash command` heading, approval text, proceed question, focused choice and the `Yes`/`No` single-request choices) in the alternate screen, with a plain or boxed operation, unwrapped or wrapped, within 24 rows from its heading to its `Esc to cancel · Tab to amend` line. Rules `claude-bash-permission-auto-tip`, `claude-auto-ask-permission` and the question rule `claude-auto-startup`: described below |
| Allow answer | `tested` | One stage, byte `31` (digit `1`), selecting `Yes` as `allow-once`, for each permission request rule |
| Deny answer | `tested` | One stage, byte `34` (digit `4`) for `claude-bash-permission` and `claude-bash-permission-auto-tip`, and byte `32` (digit `2`) for `claude-auto-ask-permission`, selecting `No` as `deny-once` |
| Empty-composer observation | `tested` | The idle prompt line with the terminal cursor immediately after the prompt prefix; a verified non-interaction boundary |
| Annotations | `tested` | `claude-auto-refusal`, `claude-auto-refusal-notice`, `claude-auto-unavailable` and `claude-auto-mode-indicator`, described below |
| Prompt submission | `tested` | Empty prefix and the measured native submit key, as separate literal and submit stages (§ Measured submission plans and settle intervals) |
| Native-exec submission | `tested` | Prefix `2120` (`! `) and the measured native submit key, as separate literal and submit stages with independent native-shell provenance |

Every answer selects its choice by its number, so it does not depend on which choice is focused. The broadened choices 2 and 3 have no answer plan. The empty-composer observation reads the composer as empty only when the cursor sits at the end of the prompt prefix; a draft whose cursor was moved back to the prefix reads as empty. Every Claude version at or above `2.1.302` selects this profile until a later boundary is added (§ Profile selection by minimum version).

Detection rests on native captures of the form unwrapped at 120 columns, wrapped plain at 52 columns, and boxed at 89 and 120 columns; both answers rest on the 120-column and the 52-column plain captures. A narrow pane or a long command makes Claude wrap the form: the tip, the description and the choice labels continue on further rows, and the operation between the dashed rules spans several rows. The operation rows take one of two native shapes:

| Shape | Row layout | Where Claude breaks the operation | Native evidence |
|---|---|---|---|
| Plain | One-space margin, then the operation text | At the row's last usable column, one column before the pane's right edge, even inside a token | Captures at 52 columns |
| Boxed | One-space margin, then the `│ ` gutter and the operation text | At a space | Captures at 89 columns (two rows) and 120 columns (one row) |

The shape follows the operation's length rather than the pane width. In the captures, operations of 57 and 61 characters render plain, and an 86-character operation renders boxed, on one row at 120 columns and over two rows at 89 columns.

Claude positions every row explicitly, so the terminal records no soft wrap, and a displayed row break cannot be told from a line break in the command itself. The rule therefore reports the operation as its displayed rows: it removes exactly the one-space margin from each operation row, keeps the boxed `│ ` gutter (also on a single boxed row), and joins the rows with line breaks. No row is joined to another, whichever column it ends at. The profile declares this margin removal in its operation extraction. The evidence records the number of displayed rows as `operation_rows`, and an operation over more than one row extends the Director's review instruction (§6.9 Request-bound permission responses).

The request identity follows the displayed rows. A resize that rewraps a multi-row operation, or moves it between one row and several, replaces its episode; a resize that leaves every operation row unchanged keeps it.

Claude Code 2.1.302 in `auto` shows a native permission form only through an explicit `permissions.ask` rule. That form differs from the `default` form: it carries a `│ Ask rule … overrides auto mode for this command.` row and a `/permissions to let auto mode decide` row, has no `This command requires approval` line, and offers `2. No` as its deny choice. Rule `claude-auto-ask-permission` (priority 100, alternate screen, cursor inside) spans from `^ Bash command$` to `^ Esc to cancel · Tab to amend$` within 24 rows and requires these line matchers besides the shared operation matcher and its one-space `row_margin`:

- `^ │ Ask rule \S`
- `^ /permissions to let auto mode decide$`
- `^ Do you want to proceed\?\s*$`
- `^ ❯ [1-9]\. ` for the focus, `^ [❯ ] 1\. (?P<label>Yes)$` and `^ [❯ ] 2\. (?P<label>No)$` for the choices

In the captures, allow `1` ran the command, and deny `2` printed `Interrupted · What should Claude do instead?`.

Under a non-automatic mode such as an explicit `default`, Claude Code 2.1.302 can show another Bash form for a workspace file write. It replaces the `This command requires approval` line with a `Tip: auto mode handles these prompts for you — choose "switch to auto mode" below` row and a description row, and offers four choices. Rule `claude-bash-permission-auto-tip` (priority 100, alternate screen, cursor inside) spans from `^ Bash command$` to `^ Esc to cancel · Tab to amend$` within 24 rows and requires the shared operation matcher with its one-space `row_margin` and these line matchers:

- `^ Do you want to proceed\?$` and `^ ❯ [1-9]\. ` for the focus
- `^ [❯ ] 1\. (?P<label>Yes)$` and `^ [❯ ] 4\. (?P<label>No)$` for the single-request choices
- `^ [❯ ] 2\. Yes, and always allow access to \S.* from this project$` and `^ [❯ ] 3\. Yes, and switch to auto mode · auto mode handles these prompts for you$` for the broadened choices, which have no answer plan

Allow `1` ran the command in the captures, and deny `4` printed `Interrupted · What should Claude do instead?`. The form was captured at 120 columns only, so a wrapped or boxed rendering of it is not detected.

In `auto`, Claude Code 2.1.302 can show `Teach auto mode about your environment?` after a turn. The dialog first offers `1. Yes`, `2. Not now` and `3. Don't show again`; `1. Yes` opens a wizard with `How you use Claude here`, `Also scan shell history`, `Also scan your other repos` and `Continue`. The dialog consumes typed input, and once answered or cancelled it does not reappear for that configuration. Rule `claude-auto-startup` (priority 100, alternate screen, cursor inside) matches both stages as one `question` interaction:

- start `^ {2,3}Teach auto mode about your environment\?$`
- end `^ {2,3}(?:Enter to confirm|←/→ to change · Enter to continue) · Esc to cancel$`, within 14 rows
- line matchers for the heading, the footer and the focus `^ {2,3}❯ \S`

The rule declares no operation and no choices. Offer and wizard therefore share one fingerprint, and the wizard continues the offer's episode, which closes when the idle composer returns. The rule has no answer plan; the Director asks the user to answer the dialog on the member's native surface. To avoid it, answer the dialog once in an interactive session of the pinned version with the member's configuration directory; `3. Don't show again` persists.

Claude renders an automatic refusal in one of two native forms, depending on the user's Verbose output setting, and the profile claims both. Every Claude annotation rule (priority 40, alternate screen, cursor any) declares the same frame guard: require `^❯\u{a0}` (the live composer prefix) and reject `^  Showing detailed transcript · ctrl\+o to toggle` (the transcript viewer's footer).

| Rule | Region | Captured screen |
|---|---|---|
| `claude-auto-refusal` | Start `^● [A-Z][A-Za-z]*\(`, end `^  ⎿ \u{a0}Denied by auto mode classifier ∙ \[`, at most 8 rows | The verbose-view transcript block `● Bash(<command>)` followed by `  ⎿  Denied by auto mode classifier ∙ [<category>] ∙ see https://code.claude.com/docs/s/claude-code-auto-mode`, kept in the live transcript |
| `claude-auto-refusal-notice` | Start `^ +[a-z][a-z_]* denied by auto mode · \[[^\]]+\] · /permissions`, end the composer border `^─{3,}(?: \S.* ─)?$`, at most 2 rows | The default-view status-row notice `<tool> denied by auto mode · [<category>] · /permissions` above the composer border, shown for about 8–10 seconds |
| `claude-auto-unavailable` | Start `^ +auto mode disabled by settings\b`, end the composer border, at most 2 rows | The transient notice `auto mode disabled by settings`, induced by `permissions.disableAutoMode`; the footer then shows `⏸ manual mode on` |
| `claude-auto-mode-indicator` | Start and end `^  ⏵⏵ auto mode on \(shift\+tab to cycle\)`, one row | The footer badge on every live `auto` frame; its `mode` capture records `observed_mode` `auto mode` |

`claude-auto-refusal` matches its region with one region-scoped matcher that captures the refused tool, the operation between the parentheses and the category, so the operation evidence is the refused command. Its first row follows `● Bash(` and carries no margin, so the rule declares no `row_margin`, and a wrapped operation reports its continuation rows with their two-space margin. `claude-auto-refusal-notice` captures `<tool> denied by auto mode · [<category>]` as its operation, because the default view never shows the refused command. `claude-auto-unavailable` captures its notice with a null operation. The native result prefix carries a U+00A0 after `⎿`, while a model quote of the same words carries two ASCII spaces, so quotes do not match.

Profile `codex-0.163.0-approve-for-me-1`, with `min_version` `0.163.0`, applies to Codex CLI under every Codex launch policy; the request and answer rules come from captures under `--ask-for-approval on-request --sandbox workspace-write`:

| Capability | Status | Declaration |
|---|---|---|
| Detection | `tested` | The native command-approval form in the alternate screen, from its `Would you like to run the following command?` heading to its `Press enter to confirm or esc to cancel` line within 24 rows: the `Environment:` and `Reason:` lines, the single-line `$ ` command, the focused `›` choice and the `1. Yes, proceed (y)` / `3. No, and tell Codex what to do differently (esc)` single-request choices, with the cursor inside the form |
| Allow answer | `tested` | One stage, byte `79` (`y`), selecting `Yes, proceed` as `allow-once` |
| Deny answer | `tested` | One stage, byte `33` (digit `3`), selecting `No, and tell Codex what to do differently` as `deny-once` |
| Empty-composer observation | `tested` | The idle `› ` composer line with the terminal cursor immediately after the prefix, its placeholder text shown; a verified non-interaction boundary |
| Annotations | `tested` | `codex-auto-review-refusal` and `codex-auto-review-indicator`, described below |
| Prompt submission | `tested` | Empty prefix and the measured native submit key, as separate literal and submit stages (§ Measured submission plans and settle intervals) |
| Native-exec submission | `tested` | Prefix `2120` (`! `) and the measured native submit key, as separate literal and submit stages with independent native-shell provenance |

The rules anchor on the live form's heading and closing lines rather than on absolute rows, and the reason line is model-authored text that the rule requires but excludes from the request identity. The deny answer uses the choice's digit because that choice displays only `esc` as its shortcut, and CAFleet never sends a lone Escape to a held form. Choice 2, which approves a command prefix persistently, has no answer plan. The native captures showed the decision on the native surface: after allow, Codex printed `✔ You approved codex to run <command> this time` and ran the command; after deny, it printed `✗ You canceled the request to run <command>` and `■ Conversation interrupted`, and ran nothing. These lines are decision evidence for the Director's continuation check, not bundled rules. A Codex configuration that routes approvals to its automatic reviewer (`approvals_reviewer`) answers escalations without any native form, so detection observes no request. The detection covers a single-line command; a command that wraps in a narrow pane yields no detection, so no `interaction_opened` event is produced, ordinary programmatic input is not suppressed and may reach the form through delivery fallback, and the request is answered manually on the native surface.

Codex 0.163.0 in `approve-for-me` routes every escalation to its automatic reviewer and shows no native permission form, so the profile adds no request rule. Both Codex annotation rules (priority 40, alternate screen, cursor any) declare the same frame guard: require `^(?:› Ask Codex to do anything|  \? for shortcuts)` (the live composer or its hint line) and reject `^  k keep & next · esc dismiss & close` (the f2 warnings overlay).

- `codex-auto-review-refusal` spans from `^✗ Request denied for codex to run \S` to the next blank row `^\s*$` within 6 rows. Its region matcher `\A✗ Request denied for codex to run (?P<operation>[^\n]*\S(?:\n  [^\n]*\S)*)\s*\n\s*\z` captures the refused command, and its `row_margin` is two spaces. The line stays in the transcript at column 0, followed by `• Failed (exit 1) <command>`; model quotes render indented or `• `-prefixed, never at column 0, so they do not match. Codex wraps a long command inside a token with a two-space continuation margin, and the fingerprint strips that margin before removing the row break, so a resize keeps the fingerprint.
- `codex-auto-review-indicator` matches the one row `^  Permissions: +Workspace \(Approve for me\)\s*$` in the `/status` block. Its `mode` capture records `observed_mode` `Workspace (Approve for me)`.

The captures showed an allowed escalation only as an ordinary `• Ran …` transcript line, and the reviewer at work as `• Reviewing approval request (Ns • esc to interrupt)`; both are verified non-interaction frames with the composer visible, so neither needs a rule.

Profile `opencode-2.1.4-cafleet-ask-1`, with `min_version` `2.1.4`, applies to OpenCode `mini` under either OpenCode launch policy; only `cafleet-ask` shows permission forms:

| Capability | Status | Declaration |
|---|---|---|
| Detection | `tested` | Rule `opencode-shell-permission`: the shell permission form on the primary screen, from its `△ Permission required` heading to its `⇆ select  enter confirm  esc reject` control line within 24 rows, with the single-line `$ ` operation, the focused choice and the `Allow once` and `Reject` choices |
| Allow answer | `tested` | One gated stage: byte `0d` (Enter) while `Allow once` is focused, as `allow-once` |
| Deny answer | `tested` | Three gated stages: `68` (`h`) while `Allow once` is focused, `0d` while `Reject` is focused, then `0d` on the empty rejection-reason dialog, as `deny-once` |
| Empty-composer observation | `tested` | The idle `┃` composer line with the terminal cursor immediately after the prefix; a verified non-interaction boundary |
| Prompt submission | `tested` | Empty prefix and the measured native submit key, as separate literal and submit stages (§ Measured submission plans and settle intervals) |
| Native-exec submission | `tested` | Prefix `2120` (`! `) and the measured native submit key, as separate literal and submit stages with independent native-shell provenance |

The form offers no focus-independent key: `h`/`l` cycle the focus, Enter selects the focused option and Escape rejects. Focus starts on `Allow once`, and `h` from there wraps to the last option, `Reject`, in both the three-option (`Allow once` / `Always allow` / `Reject`) and the two-option layout. The form marks the focused option by cell background only, so the `focus` matcher uses `focus_style: "background"` (§ Focused-choice resolution). In the captures the focused label sits on rgb(250,178,131) and the other labels on the row background rgb(30,30,30), but the rule compares backgrounds rather than these colours. `Always allow`, which approves persistently, has no choice entry or answer plan, so a focus on it satisfies no gate.

The choices share the control line's row. `mini` hides the cursor while a form or dialog shows, and its recorded position varies, including the pending-wrap position on the row below the form. Both OpenCode permission rules therefore match with cursor `any` and rely on their heading and control-line anchors.

| Stage | Bytes | Gate | Native effect |
|---|---|---|---|
| Allow 1 | `0d` | Focus `allow-once` | Selects `Allow once`; OpenCode runs the command once |
| Deny 1 | `68` | Focus `allow-once` | Moves the focus to `Reject` |
| Deny 2 | `0d` | Focus `deny-once` | Selects `Reject`; OpenCode opens the rejection-reason dialog and keeps the request pending |
| Deny 3 | `0d` | Screen `opencode-reject-reason` | Submits an empty reason; OpenCode rejects the request without running it |

The continuation rule `opencode-reject-reason` (§ Continuation screens) continues `opencode-shell-permission` after `deny-once`. It matches the `△ Reject permission` dialog with its `Tell OpenCode what to do differently` prompt and the `enter confirm  esc cancel` control line. It recognises an empty reason by the textarea span on the control-line row equalling the placeholder `Tell OpenCode what to do differently` exactly; typed text replaces the placeholder and fails the gate. CAFleet never types a reason and never sends Escape to the dialog.

The native captures showed this decision evidence:

| Answer | Native surface | Continuation |
|---|---|---|
| Allow | The hint area briefly shows `Waiting for permission event…`, then the form disappears without transcript text, the command runs and the member prints its report, a `<agent> · <model> · <duration>` line and the idle composer | The member continues on its own |
| Deny | After the final Enter the hint area briefly shows `Waiting for permission event…`, then the dialog disappears and the transcript shows `Step interrupted`, a `<agent> · <model> · <duration>` line and the idle composer; nothing runs | OpenCode does not continue after a rejection; the Director's next message reaches the idle composer through the prompt submission plan and continues the member |

These lines are decision evidence for the Director's continuation check, not bundled rules. Because an answered form or dialog leaves no transcript text, the false-positive guard rests on the live-region anchors. The fixtures are native 36x120 captures with only row 35 masked; the visible default public model name and context-usage counter are neither account, plan nor path text. The OpenCode limits apply as follows:

- A command that wraps in the form yields no detection, so the request is answered manually.
- Non-shell permissions never ask under the `cafleet-ask` preset.
- A typed reason identical to the placeholder cannot be told from an empty textarea, but such typing is human input, which fails the deny stage-3 gate inside a response.

A native screen fixture records one captured terminal screen with exactly these fields, of which only `cell_backgrounds` is optional:

| Field | Requirement |
|---|---|
| `schema_version` | `1` |
| `fixture_id` | Equals the catalog fixture `id` |
| `provenance` | `native` |
| `backend`, `native_version` | Equal the catalog fixture entry |
| `rows`, `cols` | Positive measured geometry, equal to the catalog fixture entry |
| `alternate_screen`, `screen_seq`, `cursor_row`, `cursor_col` | Measured screen state; `cursor_row` is below `rows`, and `cursor_col` is at most `cols`, where `cursor_col` equal to `cols` records the terminal's pending-wrap position after the last column |
| `lines` | Exactly `rows` captured row texts, in viewport order |
| `masked_rows` | Unique zero-based row indices, each `{"row":N,"reason":"reviewed_privacy"}`; the corresponding `lines` entry is the empty string |
| `cell_backgrounds` | Optional runs `{"row":N,"start_col":C,"end_col":E,"bg":COLOUR}`, one per maximal run of cells in an unmasked row that share one non-default background, `end_col` exclusive; `COLOUR` is `{"kind":"indexed","index":I}` or `{"kind":"rgb","r":R,"g":G,"b":B}`. An absent key means every cell has the default background. |

The capture-to-fixture conversion derives `cell_backgrounds` from the capture grid's per-cell `bg`, and fixture replay sets each run's background before writing its row text, so a fixture reproduces the focus styling a `focus_style` matcher reads.

Masking applies only to rows removed by the reviewed capture privacy policy, such as home paths and account, model or plan text. Every other row keeps its captured text exactly, including trailing spaces. The detection region of every rule citing the fixture lies entirely in unmasked rows, and a masked fixture keeps `provenance` `native`.

The bundled catalog fails to load unless each bundled fixture satisfies all of these:

- Its embedded bytes hash to the catalog entry's `sha256`.
- It parses with exactly the fields above.
- It declares `native` provenance.
- It matches the entry's backend, version and geometry.
- Every row declared in `masked_rows` is in range and empty.
- Every `cell_backgrounds` run lies inside the geometry and outside masked rows, has `start_col` below `end_col`, overlaps no other run and carries a valid non-default colour.

A synthetic fixture, an undeclared alteration or a digest mismatch therefore rejects the bundled catalog. Synthetic fixtures remain available to explicitly selected test catalogs.

A rule's cursor condition treats a pending-wrap cursor as standing on the last cell of its row, both in fixtures and on live screens. A region spanning the full width therefore counts a cursor at column `cols` on one of its rows as inside the region.

##### Executable native plans

Keep detection, empty-composer observation, prompt submission, native-exec submission and each semantic answer decision independently eligible for the run's selected profile. Observing an empty composer supplies input-state evidence; executable submission additionally requires its measured literal/submit plan. Existing observation definitions without a submission plan retain their observation coverage and yield no executable composer capability. Production capabilities require accepted native evidence within the profile interval for the claimed operation; synthetic plans run only in explicitly selected test services.

NativeCapabilities reports empty-composer observation through `composer`, executable prompt submission through `prompt_submission`, and executable native-shell submission through `native_exec_submission`. Each operation field is `tested` exactly when the run's selected profile has a validated executable definition for that operation; otherwise it is `unknown`, and ordinary input for that operation uses delivery fallback. A catalog selected explicitly by test services describes its fixture coverage; production declarations additionally require accepted native evidence. Observation coverage and support for another operation leave each operation field independent.

An answer definition retains its validated single-request `choice_id`, `focus_matcher_id` and ordered `stages`, in addition to its decision/request/rule/provenance fields. Each stage has `bytes_hex`, `required_rule_id` and `required_choice_id`, where the required rule and choice equal the answer's request and selected single-request choice, and at most one optional gate, checked on the fresh observation immediately before the runner writes that stage:

| Gate | Holds when |
|---|---|
| `required_focus_choice_id` | The plan's focus matcher resolves this choice id as focused on the request form |
| `required_screen_rule_id` | This `permission_continuation` rule matches as the continuation of the bound request (§ Continuation screens) |

A stage without a gate is focus-independent; the bundled Claude and Codex stages are ungated. The catalog fails to load unless:

- every focus gate names a choice of the request rule;
- every screen gate names a `permission_continuation` rule whose `continues_rule_id` is the answer's request rule, and the stage immediately before it is gated on that rule's `continues_choice_id`;
- a plan with any gated stage gates every stage, and its final stage is gated either on the answer's own `choice_id` or on a continuation rule whose `continues_choice_id` is that choice;
- a plan with a focus gate has a focus matcher that declares `focus_style`, and every choice label it compares is captured by a line-scoped matcher;
- a `permission_continuation` rule declares no operation, choices or answers and cites a native fixture.

###### Focused-choice resolution

A focus matcher with `focus_style: "background"` resolves the focused choice from cell styling, without depending on the theme:

1. Locate each choice label's cells through its line's cell boundaries.
2. Take the choice row background from the choice line's cell at the live region's left edge.
3. Resolve the focused choice as the one choice whose label cells all carry a single background that differs from the row background, provided no other choice label carries a differing background.

A focus on an option without a choice entry, on several options or on none resolves to no choice, and a gate that does not hold is never written. The focus text alone does not change when a styling-only focus moves. The focus value compared in the observed request, the answer match and the write-time binding is therefore the resolved focused choice id (or none) together with the matcher's capture, so a styling-only focus change starts a new request generation.

The composer object at `capabilities.composer` has required `rule_ids,fixture_ids` and `submission`; every bundled profile declares `submission`, and an explicitly selected test catalog may omit it to exercise missing-plan fallback. `submission` has exactly the required operation definitions `prompt,exec`, each an object with exactly `prefix_hex,submit_hex,fixture_ids`. Prompt has an empty prefix; exec has prefix `2120`, the UTF-8 bytes of `! `; `submit_hex` is the measured native submit key, never an assumed Enter mapping. Prompt and exec carry independent provenance. Every plan's fixture references resolve to the profile's backend and to capture versions within its interval, and establish the complete composer/work boundary for that operation. Unknown fields and incomplete or incompatible provenance fail validation.

| Plan property | Inclusive limit or required representation |
|---|---|
| Answer stages | 1–16 ordered stages. |
| Escape | No stage contains the byte `1b`, for every backend. |
| Answer bytes | 1–256 decoded bytes per stage; at most 4096 decoded bytes per plan. |
| Composer submit key | 1–32 decoded bytes. |
| Composer prefix | Zero bytes for prompt; exactly two bytes for exec as specified above. |
| Encoded native ordinary submission | At most 65536 bytes for prefix, unchanged literal UTF-8 and submit key together; the complete control JSON must independently satisfy its frame limit. |
| Hex byte definitions | Even-length lowercase hexadecimal, with nonempty bytes wherever the domain above requires them. |

Validate the complete shape, reference set, hex form and byte/stage bounds before decoding and constructing an executable plan. Keep selected native bytes and live semantic identity in memory, outside ordinary errors, debug output and persistence. An ordinary submission writes its literal stage and its submit-key stage separately (§ Measured submission plans and settle intervals); a positive short write stops remaining bytes and records partial uncertainty. Before any positive ordinary write, WouldBlock or Interrupted yields and requires fresh input/output/state validation before another attempt. Answer stages follow their independently measured order and revalidate request, choice, focus and the stage's gate before each stage; a positive short stage stops the remaining plan with uncertainty.

##### Measured submission plans and settle intervals

Measurement establishes each operation plan. For each bundled profile, capture native empty-composer, typed-literal/pre-submit and post-submit behavior for prompt and native shell input on a native version within its interval. Record the native version, multiplexer, screen geometry, policy, exact stage bytes and times, the observed native result and privacy-reviewed fixture digests. An idle screen capture alone is insufficient evidence that either operation executes.

The settle interval is 500 milliseconds on the monotonic clock, matching the pane-input settle delay. Every profile-driven submission runs these separate runner stages:

1. Revalidate the run, active placement, observer and interaction state, then durably reserve the input before any child byte.
2. Write the operation prefix plus the unchanged literal UTF-8 as the literal stage.
3. Continue servicing child output, outer-terminal input and control traffic while a 500-millisecond monotonic timer runs from completion of the literal stage.
4. Revalidate identity, interaction state and absence of intervening manual input (terminal replies excluded, § Terminal reply classification), then write the separately measured submit-key stage.
5. Record accepted submission only after the entire submit-key stage is written.

The runner stores the literal and submit-key stages separately in memory. Ordinary output redraw caused by the literal is expected, so the literal's own nonempty composer leaves the pending submit stage valid. A candidate or new interaction, manual input, an ownership or placement change or observer failure stops the remaining stages; terminal replies forwarded during the settle leave the submit stage valid.

A positive short write, a failure after any child byte, or a timer-stage cancellation after a positive write produces `uncertain` with the input ID and last stage; an Escape byte counts as a side effect. The receipt records its `uncertain_cause` (§ Uncertain cause). A zero-byte transient write may retry only after fresh validation, and an uncertain operation is reconciled before any new attempt. The inclusive 65536-byte encoded input limit and the control-frame limit apply unchanged.

#### Runner boundary and creation transaction

The internal, hidden `cafleet __member-run --descriptor <ABSOLUTE_PATH>` command accepts only a descriptor allocated by `member create`; the public launch interface remains `member create`. Its dispatch validates storage but bypasses ordinary command deadlines for its long-lived event loop.

```text
tmux/herdr pane <--> CAFleet runner <--> child PTY <--> native agent CLI
                         |
                         +--> terminal state --> detector --> event writer
                         +<-> local control socket                 |
                                                                 v
                                                      SQLite event + inbox message
                                                                 |
                                                      Director stdout receiver
```

The runner owns the child PTY and reconstructs terminal state independently of the multiplexer. Terminal modes, signals and process handling preserve native input and owned-child cleanup.

Store runtime artifacts in `.cafleet-runs/<database_namespace>/<run_id>/` beside the canonical selected database. The migration creates a singleton `runtime_database_identity` row with a random 128-bit `instance_id`; the database identity is the canonical path, filesystem device/inode and this stored instance ID. Compute `database_namespace` as the first 26 base32 characters of its SHA-256 digest. A private namespace manifest stores the complete identity; a prefix collision with a different manifest fails explicitly rather than sharing artifacts. Distinct databases under one parent therefore remain separate even with equal run IDs.

Directories use `0700`, descriptor/spool files use `0600`, and a control socket is accessible only to its owner. Create each run directory exclusively and reconcile an existing directory before reuse. Reject symlinks, ownership mismatches, malformed descriptors and socket paths exceeding the platform limit against the complete resulting path before pane creation. The descriptor contains schema version, complete database identity, fleet/member/Director/run identities, ownership epoch, a random launch nonce, literal child argv, absolute cwd, required environment overrides, evidence policy and initial terminal dimensions. Preserve the existing inherited native configuration; avoid copying the entire environment or creating per-member native profiles.

Replacing a database file creates a different filesystem identity and namespace even when restored bytes retain the instance ID. Runners validate the selected path's file/instance identity before input and storage mutations; replacement fences the old run from the new database, holds input and initiates owned-child cleanup. Retain the old namespace for explicit reconciliation; a new creator never adopts its socket or outbox. A copied database at another canonical path also receives a separate namespace.

The descriptor and control socket are local coordination mechanisms, not an authorization boundary against the same operating-system user. Require matching identities and the nonce to reject stale endpoints. The creator's guarded cleanup deletes the descriptor on every outcome: success, handshake failure, timeout, rollback and uncertain post-`start` timeout. Failed run records retain only the non-secret `permission_argv_json`, never the assignment-bearing prompt element or raw environment. Report a failed unlink explicitly. After a creator crash, `member recover` removes the descriptor only after identity/ownership validation; read-only `doctor` reports the leftover and its recovery action. The runner removes its loaded descriptor after preparation when safe, independently of the creator's idempotent cleanup.

During verified ordinary recovery, retain the original ownership lock and pin a leftover launch descriptor by its opened file identity and bounded content. Validate the recorded namespace/directory, private regular file, immutable database/fleet/member/Director/run identity and nonce. A positive historical descriptor epoch at or below the captured current epoch remains valid across takeover and retry; native launch loading still requires current creator ownership and the exact current epoch. Expected absence is idempotent while the pathname remains absent.

Before unlink, revalidate the fixed current run/placement, original lock/directory, named file type/permissions/device/inode/content and original deadline. Remove the pinned descriptor before reporting complete cleanup. Preserve a replacement or symlink and return an explicit identity error; report unlink failure explicitly and retain incomplete, retryable recovery. Keep assignment and environment content out of diagnostics. Exceptional database-only reconciliation leaves surviving unverified descriptors untouched.

Prepare connection setup waits only before any request write for an absent endpoint, a same-user socket awaiting mode 0600, or a transient bound-but-not-listening socket. All retries share the original handshake deadline. Wrong type, owner or symlink, including a dangling symlink, fails explicitly; current-run health keeps strict endpoint checks. A possible prepare write, malformed reply or lost reply returns its explicit error or uncertainty without reconnecting.

Creation follows this ordered protocol:

1. Validate assignment input/placeholders, backend policy eligibility, runtime paths and backend/multiplexer prerequisites. Preserve one assignment substitution and one literal prompt argv element.
2. Register the pending member and allocate its run row in one transaction. Write the private descriptor and create the pane with the absolute current CAFleet executable and the internal runner argv.
3. The runner validates the descriptor, acquires the precreated identity-pinned ownership lock, records its process identity, opens the PTY, initializes terminal state and event machinery, and binds its control endpoint. It reports `prepared` without spawning the native CLI.
4. The creator connects, validates the nonce/run identity, and publishes the concrete pane placement with `status=prepared`. Only then send the control `start` frame. The runner revalidates placement, spawns the child and records `running` before reporting `started` with child PID/start identity.
5. Return creation success only after `started`, meaning child spawn succeeded without a native-policy/liveness guarantee; include the run identity and detection coverage with the member result. A child that immediately exits is reported by `runner_exited`, including its native status and Director notification. Use a ten-second monotonic handshake budget and preserve existing rollback/cleanup diagnostics if any phase fails.

Until `start`, the runner exits if its creator disconnects or the handshake expires. Failure after child spawn kills and reaps that owned child, closes the known pane, deregisters the pending member and retains a failed run record. A failed cleanup retains the original failure plus explicit orphan/pane diagnostics. A timeout after `start` is an uncertain launch: inspect the existing run and stop its owned child during compensation; creating a second child is not a retry mechanism. Registration and pane ownership remain guarded until the handshake is complete.

#### Terminal transport and lifecycle

The runner relays terminal input and child output as bytes, with the outer terminal in raw mode under a restoration guard. Preserve interactive control sequences, UTF-8 fragments, bracketed paste and alternate-screen transitions. Size the PTY from the outer terminal, propagate resize to both PTY and parser, and preserve the native child's cwd, intended environment and exit status. Keep child bytes and runner diagnostics on separate logical channels; diagnostics must not become child input or detector evidence.

Forward outer-terminal input so that a burst spanning several reads reaches the child together. One read of the raw-mode outer terminal returns at most the line discipline's 4096-byte buffer, and a longer terminal reply that reaches the child a tick late can be parsed as keys:

| Concern | Contract |
|---|---|
| Read | Each tick repeats the non-blocking outer read until it would block, reaches EOF or fills the 65536-byte input queue |
| Write | The tick then writes the whole queue to the PTY master in one write; unaccepted bytes stay queued, in order, for the next tick |
| EOF | Bytes read before EOF are forwarded before outer EOF is reported |
| Pacing | A tick that moved input bytes, by reading outer bytes or writing a positive count to the PTY master, is followed immediately by the next tick; every other tick, including one whose queued input the master did not accept, keeps the loop's idle sleep |
| Framing | None: forwarded bytes pass unchanged and in order, with no holding or timeout. Manual-input accounting classifies the same bytes separately (§ Terminal reply classification) |

Only bytes classified as manual input advance the manual-input generation; a complete terminal reply is forwarded but leaves the generation unchanged (§ Terminal reply classification).

The PTY-reader path updates terminal state before publishing the same bytes to display and passes immutable observations to the classifier. Track cursor/cell state, primary/alternate screens, OSC title and a monotonic output sequence. Keep scrollback out of live-prompt matching. Terminal queries and replies continue through the terminal transport, and parser callbacks must not synthesize a second reply to a query already handled by the outer terminal.

Separate terminal transport, classification and event storage. Database waits, Director receiver loss and event-spool retries must not stop draining child output or accepting a human's native answer. Use bounded byte buffers and ordinary terminal backpressure for a slow outer display; a broken outer terminal triggers owned-child cleanup. Classification may coalesce intermediate working-screen snapshots, but it processes every potential interaction transition from the ordered parser stream before such coalescing. This is an observation contract for output actually rendered/read, not a guarantee to capture a dialog opened and closed between observable states.

| Condition | Required lifecycle behavior |
|---|---|
| Native child exits | Record its actual code/signal immediately, then apply the bounded drain/cleanup protocol below and close the episode with `process_exited`. Restore the outer terminal independently of storage completion. |
| Runner receives termination or outer EOF/HUP | Stop programmatic input, terminate the owned process group, allow two seconds for cleanup, then kill verified remaining owned children and reap. Record the real termination cause. |
| Runner fails while child is live | Mark observation degraded, stop programmatic input and attempt the same owned-child cleanup; return failure rather than a fabricated native success. |
| Runner is killed without cleanup | Closing the PTY may cause native hangup, but descendants may survive. Mark reachability suspect until exclusive recovery establishes loss; inspect verified PID/start identity before signaling. |
| Member/fleet deletion | Acquire existing pane locks in their established order, revalidate placement/run, request runner shutdown and finish pane/registry cleanup under existing semantics. Report any unconfirmed child cleanup explicitly. |
| Restart or replacement | Create a fresh member/run after explicit reconciliation. Keep earlier episodes/events; a process generation never inherits an earlier pending request or causes automatic command re-execution. |

Use one shutdown clock starting at normal native-child exit, runner failure or termination request. Stop programmatic input immediately. During the first two seconds, drain available PTY bytes and terminate verified surviving descendants in the owned process group: send TERM immediately and KILL any verified survivors at the two-second boundary. Close the master at that boundary even if a descendant retains the slave. Nonblocking outer-display flushing ends by the three-second boundary; discard only undelivered display-tail bytes, record their count and retain a truncation diagnostic. Event-worker flush/SQLite attempts end by the four-second boundary; retain complete uncommitted outbox frames for explicit recovery and record any unsaved in-memory gap. All waits share these absolute deadlines.

Run every runner SQLite attempt under a bounded database budget using the remaining absolute work/shutdown deadline; it replaces the connection's default five-second busy wait with bounded waiting and restores the prior handler afterward. Cancel long-running statements within the same deadline and restore connection hooks after each attempt. Shutdown attempts share the four-second boundary, including transaction completion and retry delays; they receive no additional grace budget.

Restore terminal settings and release ownership after stopping/joining the writer, regardless of EOF, display or database completion. Retain actual child code/signal separately from `cleanup_state=complete|incomplete` and bounded diagnostic reasons for truncated output, pending events or unconfirmed descendants. When cleanup/restoration is complete, return the native status (`128 + signal` for a signal exit). With incomplete cleanup, preserve a native nonzero return; use runner exit `1` if the native status was zero or unavailable, and report its distinct recorded native status. Hard-kill or restoration failure remains an explicit host-recovery condition.

Maintain liveness counters only in process and expose them through `health`; persist `last_health_at` on lifecycle transitions and explicit input/recovery checks, rather than writing a per-second heartbeat. An old recorded timestamp is historical evidence, not a staleness threshold. `member show` remains a registry read, returns the recorded fields and requires neither a socket nor a multiplexer. `doctor` reports registry metadata, pending content reconciliation and the results of nonblocking identity-pinned lock and bounded socket probes. Outbox content inspection belongs to the current lock/epoch owner; diagnosis leaves registry records, events and reserved sequences unchanged. Input dispatch requires a fresh matching health/ownership check; its normal bounded transaction can record `reachability=suspect` on failure or `reachable` on success. A failed probe holds input but closes no interaction, changes no terminal status and allocates no event sequence.

The creator precreates `owner.lock`, records its device/inode and runtime-directory identity, and records the current owner PID/start identity. Every ownership transfer advances `owner_epoch` with a CAS, including creator-to-runner transfer after the runner verifies/acquires that inode; the runner's `prepared` reply reports its new epoch for subsequent control frames. Recoverers likewise record their identity and advance the epoch. The runner holds the ownership lock through every writer/input thread and bounded shutdown. Validate the open descriptor's identity against both registry and current pathname; reopening a replacement lock is not acquisition of the original lock. A held matching lock returns `live_or_unreachable`; a deleted directory/lock or changed inode requires the distinct missing-path recovery branch below. An open unlinked inode may still have a live owner.

Normal recovery acquires the identity-matching lock before a CAS epoch takeover. Every event mutation validates complete database identity, owner kind and epoch; input additionally validates its ownership/lock identities immediately before child writes. A delayed live runner retaining the original lock prevents normal takeover. The missing-path branch also requires verified original-owner termination before fencing; missing files alone establish no exclusive ownership or safe input. Every takeover records the recovering process identity and advances the epoch, rejecting stale writes/input.

Only the current lock/epoch owner allocates event sequences. Confirmed loss and its events are produced by the explicit recovery procedure below after exclusive takeover, ordered after complete outbox replay. Concurrent reconcilers contend for the same lock and CAS rather than allocating competing loss events. PID/start identity assists child reconciliation; it never substitutes for ownership. There is no unattended restart or approval/command replay.

##### Terminal reply classification

The runner classifies every positive outer-terminal read for manual-input accounting only. Each read byte is queued for the PTY immediately, unchanged and in order; classification holds, reorders, drops and synthesizes no byte and retains only parse state and a pending byte count. `human_generation` is the cumulative count of bytes resolved as manual input. The staged settle check, drain proofs, permission-response authority and `member press` accounting all use that one count, so a terminal reply affects none of them. Tick pacing still counts every byte read.

Every byte resolves exactly once: as manual input (counted), as part of a complete terminal reply (not counted) or as pending inside a possible reply prefix. A sequence is a terminal reply only when it is complete and matches one allowlist row. All bytes are 7-bit; `ESC` is `0x1b`, `CSI` is `ESC [`, `ST` is `ESC \` and `BEL` is `0x07`, and spaces in the byte forms are for readability only. Two productions are shared by every row:

- `Pd`: one or more ASCII digits (`0x30`–`0x39`).
- `L(min,max)`: between `min` and `max` `Pd` values joined by single `;` bytes, with no empty parameter. `L(1,3)` matches `8`, `8;24` and `8;24;80`, and rejects the empty string, `8;` and `;24`.

| Reply | Byte forms | Notes |
|---|---|---|
| Cursor position report | `CSI L(2,2) R`, `CSI ? L(2,3) R` | Collides with modified F3; accepted as a reply |
| Device status | `CSI 0 n`, `CSI 3 n`, `CSI ? L(1,4) n` | The `?` form covers private status reports such as the color-scheme report `CSI ? 997 ; 1 n` |
| Device attributes | `CSI ? L(1,32) c`, `CSI > L(1,4) c` | Primary and secondary; the tertiary reply is a DCS |
| Mode report (DECRPM) | `CSI ? L(2,2) $ y`, `CSI L(2,2) $ y` | — |
| Window report | `CSI L(1,3) t` | Single-parameter state reports (`CSI 1 t`, `CSI 2 t`) and size reports (`CSI 8 ; 24 ; 80 t`) |
| Kitty keyboard flags | `CSI ? L(1,1) u` | `CSI L(1,3) u` without `?` is a key event and stays manual |
| Focus | `CSI I`, `CSI O` | No parameters or intermediates |
| OSC reply | `ESC ]` body `BEL`, or `ESC ]` body `ST` | Any body of at most 4096 bytes |
| DCS reply | `ESC P` body `ST` | XTVERSION, XTGETTCAP, DECRQSS and tertiary device-attribute replies |
| APC reply | `ESC _` body `ST` | Kitty graphics-protocol query responses, with the DCS termination rules |

Every other input counts as manual input:

| Input | Classification |
|---|---|
| Printable text, UTF-8, C0 controls (Enter, Tab, Ctrl-keys), DEL | Manual immediately |
| Any CSI outside the allowlist: arrow/function keys, `CSI L(1,2) ~`, kitty key events `CSI L(1,3) u` | Manual on its final byte |
| Mouse reports: SGR `CSI < … M` / `CSI < … m`, X10 `CSI M Cb Cx Cy` | Manual, because they are human pointer input even when no composer text changes |
| Bracketed paste `CSI 200 ~` … `CSI 201 ~`, including every byte between the markers | Manual, even when the pasted body contains allowlisted forms |
| SS3 `ESC O x` and Alt-modified `ESC x` for any `x` other than `[`, `]`, `P`, `_` | Manual once the second byte arrives |
| SOS `ESC X` and PM `ESC ^` strings | Manual through the Alt-modified row; terminals send no SOS/PM replies |
| 8-bit C1 bytes (`0x9b`, `0x9d`, `0x90`, `0x9f`) | Manual, as ordinary bytes in UTF-8 mode |
| A CSI whose parameter and intermediate bytes exceed 64 | Manual at the byte that exceeds the limit |
| An OSC, DCS or APC body that exceeds 4096 bytes | Manual at the byte that exceeds the limit, so a larger reply such as an OSC 52 clipboard read counts as manual |

The classifier is in exactly one state: ground, escape (after `ESC`), CSI, string body (OSC, DCS or APC), string escape (after `ESC` inside a body) or paste.

| State | Valid continuation bytes | Completes on |
|---|---|---|
| Escape | `[` → CSI; `]`, `P`, `_` → string body | — |
| CSI | Parameter bytes `0x30`–`0x3F`, then intermediate bytes `0x20`–`0x2F` | A final byte `0x40`–`0x7E`; the complete sequence is a reply when it matches an allowlist row, otherwise manual |
| String body | `0x20`–`0x7E` and bytes `≥ 0x80`; `BEL` for OSC only; `ESC` → string escape | `BEL` (OSC only) |
| String escape | `\` | `\` (ST) |

Any byte that is not a valid continuation for the current state resolves every pending byte as manual. That byte is then classified again from ground: an `ESC` starts a new sequence, and any other byte counts as manual.

| Input | Resolution |
|---|---|
| `ESC [ 1 ; ESC [ I` | `ESC [ 1 ;` is 4 manual bytes; `ESC [ I` is a focus reply |
| `ESC ESC [ I` (Alt-Escape or a stray Escape before a reply) | The first `ESC` is 1 manual byte; `ESC [ I` is a focus reply |
| A C0 control (including CAN `0x18` and SUB `0x1a`), DEL or a byte `≥ 0x80` inside a pending CSI | The pending bytes and that byte count as manual |
| `BEL` inside a DCS or APC body | The pending bytes and the `BEL` count as manual |
| `ESC` inside a string body followed by any byte other than `\` | The pending bytes up to and including that `ESC` count as manual, and the following byte is classified again from ground |
| An `ESC x` key where `x` is not a valid escape continuation | Both bytes count as manual |

The paste state follows a manual `CSI 200 ~`: every byte counts as manual, and `CSI 201 ~` counts as manual and returns to ground. A sequence split across reads stays pending across reads, within one tick or across ticks.

A pending prefix resolves as manual input once 100 milliseconds pass since its last byte:

- The runner resolves expired prefixes at the start of each tick's outer read and at the start of every drain round, before reading, and advances `human_generation` by the resolved manual count.
- While a prefix is pending, the transport is not quiescent: a drain returns `Pending` with blocker `terminal reply pending`, an earlier drain proof fails validation with `input transport is no longer quiescent`, and no programmatic stage, permission answer or deferred write proceeds.
- Outer EOF resolves a pending prefix as manual before EOF is reported.
- A lone Escape key is pending for at most 100 milliseconds and then counts as manual, so a staged delivery whose settle sees that key stops with `manual_input` once the bound resolves it.
- The bound runs inside the active attempt's five-second deadline and adds no deadline of its own.
- Each new byte of a prefix restarts the bound; the 64-byte CSI and 4096-byte string body limits cap how long a continuously fed prefix stays pending.

Human input whose bytes exactly equal an allowlisted reply and arrive within the bound counts as a reply. The realistic member of this accepted collision class is a modified F3 key (`CSI 1 ; Pmod R`), which matches the cursor-position-report form. The other members need an exact multi-key sequence typed within 100 milliseconds: Alt-`[` then `I`/`O` (a focus report), Alt-`]` … Ctrl-G (an OSC reply), and Alt-Shift-P or Alt-`_` … Alt-`\` (a DCS or APC reply). A colliding key still reaches the child unchanged; it only fails to interrupt a staged delivery or revoke automatic authority.

#### Classification and episode identity

Each detector consumes a reconstructed screen, title, cursor/input-region evidence, output sequence and known backend version. Return a typed observation with profile/rule identifiers, matched region, extracted visible operation/choices, a normalized semantic fingerprint and a confidence/evidence designation. Compile rules at startup and fail on invalid rule definitions. Bundle versioned profiles in CAFleet; profiles are bundled locally rather than loaded from remote rules or herdr manifests.

| Classification | Evidence needed | Meaning |
|---|---|---|
| `permission_request` | A backend-specific live permission heading/operation and its choice/input region | An observed request awaiting a native decision; title text alone is insufficient. |
| `permission_denied` | A tested permission-specific returned-result region | A displayed refusal, not an arbitrary nonzero exit and not proof the user refused. |
| `permission_continuation` | A tested screen the native agent shows for a pending request after one of its choices is selected, such as a rejection-reason dialog | A continuation of the active permission episode; it never opens an episode (§ Continuation screens). |
| `question` | A tested general input/question form | Human input is required without establishing a tool-permission request. |
| `trust` / `update` | A tested startup trust or update form | A distinct native interaction, preserving its own consequences. |
| `unknown_interaction` | A live interactive form without enough evidence for a more precise type | Queue ordinary programmatic input with zero bytes and request inspection. |
| `working` / `composer_empty` / `unknown` | The corresponding tested activity/composer rule, or lack of one | Input eligibility is evaluated separately; absence of permission text is not an empty composer. |

The exact persisted episode-class domain is `permission_request`, `question`, `trust`, `update`, `unknown_interaction`; `trust` and `update` are distinct values. `permission_denied` is a returned-result observation recorded only through `permission_denied_observed`, with a nullable episode link. `permission_continuation`, `working`, `composer_empty` and `unknown` create no interaction row. Each frame has exactly one of these primary observations; transcript results and notices of automatic review are annotations evaluated beside it (§ Annotation detection).

A title such as `Action Required`, a quoted approval transcript, a transcript viewer and an old permission line cannot independently create `permission_request`. Profiles must bind controls to the same live region, distinguish general questions/startup forms, and remain explicit about wrapping, layout changes and competing rules. Conflicting strongest classifications yield `unknown_interaction` and preserve the screen. A run with no selected profile, or whose profile declares no detection, advertises unknown coverage; missing matches never imply that all requests were observed.

An episode is a CAFleet observation identity, `(run_id, episode_seq)`, not a native request ID. Allocate `episode_seq` monotonically within a run when a stable live interaction first appears. Use two compatible observations at least 100 ms apart before opening; recheck after 100 ms even if no bytes arrive. Gate programmatic input immediately on the first candidate, before this notification debounce.

The fingerprint contains backend/profile, interaction class, visible operation and stable choice labels. Exclude spinner/counter/cursor decoration. The visible operation includes its displayed row breaks, so a resize that rewraps the operation changes the fingerprint and replaces the episode. Redraws and resize matching the fingerprint refresh the stored latest interaction evidence without another opening notification, but only when the post-policy evidence differs in a field other than `screen_seq` and the `evidence_digest` derived from it; a redraw that changes only the output sequence schedules no refresh and leaves the stored evidence and any issued request token unchanged. A different operation/choice/class candidate holds input immediately and needs two compatible observations at least 100 ms apart before replacing the active episode; retain the old episode until then, close it with `superseded`, and open the new episode in order. An incompatible candidate restarts stability, while return to the original fingerprint cancels the pending replacement. After a verified native non-interaction boundary, an identical request gets a new episode. If two native requests are visually indistinguishable with no observable boundary, preserve one episode and report that identity limitation; never claim one-to-one native request coverage.

| Episode transition | Event and interpretation |
|---|---|
| Stable candidate appears | `interaction_opened`; record its classification and observed evidence. |
| Partial redraw or temporary overlay | Keep the episode open, gate input and update local visibility. A two-second uncertain interval emits one `interaction_obscured` event. |
| Verified non-interaction boundary stable for 250 ms | `interaction_closed` with `not_visible`; record the observation rather than inferring a decision. |
| Different interaction replaces the active form | Require two compatible observations at least 100 ms apart, then close with `superseded` and open the next episode in order. |
| Child exits or exclusive recovery confirms observer loss | Close with `process_exited` or `monitor_lost`; a suspect/unreachable observer leaves the episode open and approval outcome remains unknown. |
| Returned permission refusal appears | Require two compatible observations at least 100 ms apart and emit `permission_denied_observed` once per stable result. Link an episode only through native correlation evidence. |
| Continuation screen of the active permission episode | Keep the episode open and `Visible` with ordinary input queued for the interaction. Emit no event, start no obscured timer and keep the stored request evidence. |

##### Continuation screens

A `permission_continuation` rule declares `continues_rule_id`, the permission rule it continues, and `continues_choice_id`, the choice whose selection opens it. It shows no operation, so it cannot identify a request by itself; it is bound to one by episode continuity and succession:

| Concern | Contract |
|---|---|
| Episode | While a permission episode of its `continues_rule_id` is active, a continuation observation keeps that episode open as the transition table states. Without such an active episode it is an `unknown` screen. |
| Request binding | Inside a response, a continuation qualifies for the next stage only as the first fresh quiescent observation after the complete write of the stage gated on its `continues_choice_id`, with the episode, manual-input generation and runner ownership unchanged and no observation gap or degradation in between. The runner then rebinds the in-flight selection to the continuation's rule id and match fingerprint within the same episode. |
| Inspection | `permission inspect` on a continuation offers no request token or answer capability, so a continuation reached by human input is completed manually. |

A returned refusal resets its deduplication identity only after a profile-verified noninteraction boundary lasting 250 ms. Temporary uncertainty resets pending stability while retaining the previous deduplication identity; a generic working screen establishes a boundary only when its profile verifies the result region's disappearance.

Equivalent redraw evidence uses one bounded coalesced worker slot, with a bounded local snapshot during an in-flight attempt. Apply the evidence policy before queuing, bind the refresh to the current owner/run/open episode and output sequence, and coalesce only newer evidence for that same active episode. Persist it in an ownership-checked transaction under an absolute deadline; equal sequence with identical evidence is idempotent, while regressing, conflicting, closed-episode or stale-episode updates fail explicitly.

Order each refresh after its opening and previously accepted semantic transitions, and before later transitions. A transient event-sequence barrier prevents delayed evidence from overwriting a later closure or visibility transition; preserve a newer coalesced revision when an earlier attempt completes. Refresh only the interaction's latest evidence/digest, preserving original opening event/message bytes and allocating no event, episode, sequence or inbox notification. Failed refreshes retain bounded pending evidence and expose degraded/gap state while manual terminal I/O continues; recovery preserves the last committed evidence and reports the gap rather than reconstructing lost redraws.

Only a fingerprint-matching redraw refreshes an interaction's stored latest evidence. A closing transition keeps that stored evidence of the request; the closing frame's post-policy evidence appears only in the `interaction_closed` event payload.

Keep decision outcome (`approved`, `denied`, `cancelled`) and operation outcome separate from episode visibility. Screen disappearance, Enter, Escape, a timeout, a message ACK and resumed animation establish none of those outcomes by themselves. In this release, machine decision outcome remains `unknown` unless explicitly documented native evidence establishes it; human/Director reports remain attributed reports, not authenticated native events.

##### Annotation detection

An annotation is a transcript result or notice rather than a live form. The detector evaluates annotation rules on every frame alongside the primary observation and keeps them out of its priority and conflict resolution: it returns the primary observation exactly as without annotations, plus the list of annotation blocks, each with its rule id, matched region, post-policy evidence and in-memory fingerprint. Annotations create no `member_interactions` row and leave input eligibility, episode visibility and the primary observation unchanged. They are evaluated only while semantic classification runs, so a degraded runner's observation gap covers them too.

| Annotation kind | Meaning | Effect |
|---|---|---|
| `automatic_refusal` | The automatic reviewer refused an operation and returned the refusal to the model | `automatic_refusal_observed` (§ Automatic refusal event) |
| `automatic_review_unavailable` | Automatic review is unavailable or degraded, for example unavailable for the account or model or fallen back to prompting | `automatic_review_unavailable` (§ Unavailable automatic review event) and `automatic_review=unavailable` in the effective policy record |
| `automatic_review_available` | A positive automatic-mode indicator, such as a mode badge | `automatic_review=available` in the effective policy record; no event |

Observation kinds parse and render each annotation kind by those exact strings. Profiles declare annotation rules in the required capability group `annotations`, which is either `null` or `{ "rule_ids": [...], "fixture_ids": [...] }` and follows the same provenance rules as every other capability group: nonempty rule ids and known fixtures whose capture versions lie within the profile interval. Every bundled profile declares the key. The catalog fails to compile unless these hold:

- The `annotations` group accepts only annotation kinds, and the `detection`, `composer` and continuation groups reject them; an annotation rule outside the `annotations` group is invalid.
- An annotation rule has empty `extract.choices`, null `extract.native_request_id` and `verified_noninteraction` `false`. `automatic_refusal` requires an `operation` capture, and the other two kinds require a null one.
- An annotation rule declares `frame`, `{ "require_patterns": [...], "reject_patterns": [...] }`, with at least one compilable require pattern and zero or more compilable reject patterns. Non-annotation rules reject the field.
- Each automatic-mode fixture produces exactly its expected primary observation and its expected annotation set, and no annotation fixture produces a `permission_denied` primary observation.

Annotation rules use the region contract of the other rules, with these differences and guards:

| Concern | Contract |
|---|---|
| Screen | Each rule declares the screen its captures show. Scrollback is excluded, as for every rule. |
| Blocks | The start and end patterns delimit one refusal or notice block, and the matchers require the native result decoration, so model text quoting the same words does not match. Every qualifying region yields a block, in row order and non-overlapping, so two refusals visible together are two blocks; other rules still require exactly one qualifying region. |
| Live-transcript frame guard | Each `frame.require_patterns` entry must match a live-screen line of the same frame and no `frame.reject_patterns` entry may match one. Require patterns name lines present only in the live TUI, such as its composer or footer line; reject patterns name the transcript viewer's header, and a frame failing the guard yields no blocks for that rule. |
| Fingerprint | Backend, profile, rule id and the displayed operation or notice text, normalized by first stripping the operation extract's `row_margin` from each continuation row, then removing every row break and collapsing each whitespace run to one space, so a resize that rewraps a block, even inside a token, keeps its fingerprint. The event's `operation` evidence still reports the displayed rows, continuation margins included. |

The annotation tracker keeps a set of fingerprint states per annotation rule:

1. A fingerprint present in a frame's blocks is emitted after two compatible observations at least 100 ms apart; several fingerprints are tracked and emitted independently.
2. An emitted fingerprint is not emitted again until its state resets.
3. A state resets only after the fingerprint has been absent for 250 ms of continuous frames whose primary observation is a verified noninteraction and whose frame guard holds. A frame showing a permission form, another interaction, an overlay, an unknown screen, a guard failure or a redraw does not advance the reset, so a block hidden by any of them keeps its state and emits nothing when it reappears. A working screen advances the reset only when the profile classifies it as a verified noninteraction; Claude Code 2.1.302 and Codex 0.163.0 keep their idle composer visible while working (Codex also while its reviewer runs), so their working frames do.
4. `automatic_review_unavailable` is emitted at most once per run per rule id unless an `automatic_review_available` annotation was observed after it.
5. `automatic_review_available` emits no event; it only updates the effective policy record.

The `permission_denied` reset uses the verified noninteraction boundary alone because a primary refusal frame and an idle-composer frame exclude each other. An annotation stays visible above the idle composer, so its reset also requires the annotation's absence. An automatic refusal is a separate kind from `permission_denied_observed` because the automatic reviewer decided it and the Director procedure differs; the two never match the same block, because one is a primary rule and the other an annotation.

The event is an alert, not a one-to-one count of native refusals. These limits apply:

| Backend | Limit |
|---|---|
| Every backend | A second refusal of the same normalized operation emits nothing while the first is visible or before an idle frame shows the first gone. A refusal rendered and removed within 100 ms is not observed. |
| Claude Code 2.1.302 | In the default view the notice names no command, so refusals of the same tool and category collapse into one event, and a notice that expires while a form hides it is never observed. The default-view notice was captured at 120 columns only; a narrower pane can truncate its ` · /permissions` suffix, and such a notice is not observed. |
| Claude Code 2.1.302 | The native-quote guard rests on the U+00A0 that the native result prefix carries. The first launch on a brand-new configuration renders on the primary screen, so no rule matches during that session. |
| Claude Code 2.1.302 | `claude-auto-refusal` strips no row margin, so a command token wider than the pane, wrapped inside the token, changes its fingerprint after a resize and can emit once more. |
| Claude Code 2.1.302 | Only the captured unavailable notice `auto mode disabled by settings` is observed. The binary also carries `auto mode is unavailable for your plan`, `auto mode unavailable while fast mode is on`, `auto mode unavailable for this model` and `auto mode is unavailable right now`, which no capture shows and no rule claims. |
| Codex 0.163.0 | Commands longer than about 70 characters display truncated, so refusals sharing the displayed prefix share a fingerprint. The reviewer's reason appears only in the f2 warnings overlay and the ctrl+t view. |
| Codex 0.163.0 | After three consecutive automatic denials Codex interrupts the turn (`■ Conversation interrupted`); the reason, `Automatic approval review rejected too many approval requests for this turn …`, appears only in the f2 warnings overlay. The Director receives one `automatic_refusal_observed` per denial, and the member stops without reporting. |
| Codex 0.163.0 | No unavailable-review screen appears on the live screen, so no rule claims `automatic_review_unavailable`, and the positive indicator appears only on demand in the `/status` block. |

The member's routed refusal evidence remains the record in each case.

#### Persistence, notification and read interfaces

The `cafleet_v8.db` format preserves its application identity and existing rows through contiguous embedded migrations. V4 establishes runtime records, V5 adds the permission-response audit/claim relationships specified below, V6 adds the automatic review event kinds, V7 adds ordinary-delivery receipt metadata, payload digest versions and the deferred-result event kind, and V8 adds the uncertain cause of input receipts (§ V6 storage, § V7 storage, § V8 storage). The singleton `runtime_database_identity(singleton_id INTEGER PRIMARY KEY CHECK(singleton_id=1), instance_id TEXT NOT NULL)` has a random 128-bit hex instance ID initialized once during migration. Runtime records have these fields:

| Table | Required fields and constraints |
|---|---|
| `member_runs` | `run_id INTEGER PRIMARY KEY AUTOINCREMENT`; member/fleet/Director foreign keys; backend, launch nonce, canonical runtime path, requested/resolved mode, mode source, permission argv JSON, native version, profile version, coverage, status and lifecycle/health timestamps; nullable runner/child PID plus verified start identity and exit code/signal. At most one nonterminal run per member. |
| `member_interactions` | `interaction_id INTEGER PRIMARY KEY AUTOINCREMENT`, `run_id` foreign key, `episode_seq`, classification, post-policy evidence digest, opened timestamp, nullable closed timestamp/reason, and latest bounded evidence JSON. Unique `(run_id, episode_seq)` and at most one unclosed interaction per run. |
| `member_runtime_events` | `event_id INTEGER PRIMARY KEY AUTOINCREMENT`, `run_id` foreign key, positive `event_seq`, nullable interaction foreign key, `kind`, observed/committed timestamps, versioned payload JSON, notification state and nullable unique `message_id` foreign key. Unique `(run_id, event_seq)` supports idempotent retries. |
| `member_input_operations` | `input_id INTEGER PRIMARY KEY AUTOINCREMENT`, run foreign key, unique `(run_id, operation_key)`, kind, payload digest, nullable message foreign key, durable receipt/settlement state, payload digest version, ordinary delivery method/reason, deferred metadata, uncertain cause and timestamps. Retain settled rows as replay-rejection records for the run's lifetime. |
| `member_permission_responses` | Immutable required decision/provenance fields keyed by the input ID, with event/interaction/Director foreign keys. |
| `member_permission_response_claims` | Interaction primary key and unique input reference, retaining one possibly transmitted answer per episode. |

Use the enum values specified in this document as SQL CHECK and typed-conversion contracts. `member_runs.status` is `starting`, `prepared`, `running`, `degraded`, `stopping`, `recovering`, `exited`, `failed` or `lost`; terminal states are `exited`, `failed` and `lost`. Nullable fields represent genuinely unavailable evidence or phases not reached. A missing required identity, invalid enum, malformed payload or cross-run interaction reference fails the write/read explicitly. Member history is soft-deregistered; retain these foreign-key-linked records under existing database ownership/retention behavior.

The table below fixes names/types beyond the primary-key fields above. Use `INTEGER` for identities, counters, PIDs and exit values; `TEXT` for enum/string/digest/JSON/timestamp values. Timestamps use UTC RFC 3339 and ordering uses the run/event sequences rather than wall-clock uniqueness.

| Record | Exact fields |
|---|---|
| Run identity/policy | Required `member_id`, `fleet_id`, `director_member_id`, `backend`, `database_identity_json`, `launch_nonce`, `runtime_path`, `resolved_mode`, `mode_source`, `permission_argv_json`, `event_evidence`, `coverage`, `status`, `created_at`; nullable `requested_mode`, `native_version`, `profile_version`, `effective_policy_json`. |
| Run lifecycle/ownership | Required `owner_epoch INTEGER` starts at 1, `owner_kind TEXT` is `creator`, `runner` or `reconciler`, `owner_pid INTEGER`, `owner_start_identity TEXT`, `reachability TEXT` is `reachable` or `suspect`, `cleanup_state TEXT` is `complete` or `incomplete`, `event_seq_reserved_through INTEGER` starts at zero, and `observation_gap_count INTEGER` starts at zero. Nullable `runtime_dir_identity_json`, `run_lock_identity_json`, `runner_pid`, `runner_start_identity`, `child_pid`, `child_start_identity`, `started_at`, `last_health_at`, `ended_at`, `exit_code`, `exit_signal`, `cleanup_diagnostics_json`, `recovery_origin_status`; the latter preserves the original lifecycle status while recovering. |
| Interaction | Required `run_id`, `episode_seq`, `classification`, `evidence_digest`, `opened_at`, `evidence_json`; nullable `closed_at`, `close_reason`. `classification` is an interactive class from the classification table; both closure fields are null together or present together. |
| Runtime event | Required `run_id`, `event_seq`, `kind`, `observed_at`, `committed_at`, `payload_json`, `notification_state`; nullable `interaction_id`, `message_id`, `archive_reason`. `notification_state=inbox` requires a message and null archive reason; `archived` requires no message and a closure reason. |
| Input operation | Required `run_id`, `operation_key TEXT`, `kind TEXT` (`preview`, `prompt`, `exec`, `permission_response`), `payload_digest TEXT`, `payload_digest_version INTEGER` (`1` or `2`; `2` only for preview), `state TEXT` (`ready`, `in_progress`, `accepted`, `uncertain`, `settled`), `created_at`, `updated_at`; nullable `message_id`, `disposition TEXT`, `evidence_ref TEXT`, `delivery_method TEXT` (`profile` or `fallback`), `delivery_reason TEXT`, `deferred_at TEXT`, `defer_reason TEXT`, `uncertain_cause TEXT`. `ready` certifies a zero-byte attempt, and a ready row with deferred metadata is a runner-queued ordinary input; `deferred_at` and `defer_reason` are present together and only for ordinary input kinds; `uncertain_cause` is a value from § Uncertain cause and is present only in `uncertain` or `settled` state; settled disposition is `submitted`, `not_submitted` or `abandoned_uncertain`, and preview kind requires its original message ID. Permission responses additionally require the audit and episode-claim relationships specified below. |

Store process-start identities as platform-specific tokens in `TEXT`. JSON fields validate as objects except `permission_argv_json` and `cleanup_diagnostics_json`, which are arrays. New event payloads carry `schema_version=2`; readers validate v1 and v2 explicitly and return existing records and their associated message bytes unchanged on replay. `event_evidence` is `metadata` or `redacted`. `cleanup_state` begins `incomplete` and becomes complete only after successful shutdown cleanup, so a live/crashed run makes no prior completion claim.

`coverage` is `tested` or `unknown`; tested means the pinned profile passed its declared release gates, not universal prompt detection. `launch_nonce` is a random 32-byte value encoded as hex and stored with the run so later same-user control clients can present it; it prevents accidental reuse of stale endpoints and is not a privileged authorization credential. Validate member/fleet/Director ownership against broker records within each mutation transaction; inbox inserts additionally require their active/live status, while archived recovery validates historical ownership. Use partial unique indexes for the active-run and open-interaction invariants.

Runtime event kinds are `coverage_unknown`, `interaction_opened`, `interaction_obscured`, `interaction_closed`, `permission_denied_observed`, `automatic_refusal_observed`, `automatic_review_unavailable`, `input_delivery_result`, `runner_degraded`, `runner_lost` and `runner_exited`. The event payload has this versioned shape:

```json
{
  "schema_version": 2,
  "event_id": 481,
  "run_id": 37,
  "event_seq": 4,
  "member_id": 92,
  "fleet_id": 23,
  "director_member_id": 91,
  "kind": "interaction_opened",
  "interaction_id": 108,
  "classification": "permission_request",
  "observed_at": "2026-10-26T02:30:00Z",
  "source": "runner_screen",
  "native_request_id": null,
  "decision_outcome": "unknown",
  "evidence": {
    "backend": "codex",
    "native_version": "recorded-at-launch",
    "profile_version": "bundled-profile-version",
    "rule_id": "native-tool-permission",
    "screen_seq": 120,
    "truncated": false,
    "evidence_policy": "metadata",
    "operation": null,
    "operation_rows": null,
    "choices": null,
    "native_request_id": null,
    "excerpt": null,
    "evidence_digest": "sha256-of-post-policy-evidence"
  },
  "director_action": {
    "kind": "review_permission_request",
    "instruction": "Verify this event and the current request. Evaluate the operation and its risk. In the Director session's effective auto mode, decide allow or deny; otherwise obtain the user's permission. Submit the selected answer through the request-bound CAFleet response command, or give that complete command to the user. Inspect the native result and continuation afterward.",
    "inspect_argv": ["cafleet", "permission", "inspect", "108", "--event-id", "481", "--from-member-id", "91", "--json"]
  }
}
```

Version-2 event keys are ordered `schema_version,event_id,run_id,event_seq,member_id,fleet_id,director_member_id,kind,interaction_id,classification,observed_at,source,native_request_id,decision_outcome,evidence`, followed by `close_reason` for closures or `cause` for lifecycle events, then, for `input_delivery_result` only, ordered `input_id,message_id,delivery,stage`, then required `director_action`. The action object has ordered `kind,instruction,inspect_argv`, and its `kind` is `review_permission_request`, `review_automatic_refusal` or `review_automatic_review_unavailable`; other new events use null action. Both automatic review event kinds fit this version-2 shape: `evidence.backend` together with the kind identifies the reviewer, and the effective policy lives on the run rather than in the payload. Version-1 reads preserve their original fields and order. An older binary never reads the new kinds, because it refuses the V6 or V7 database as ahead of its embedded chain.

The evidence object, in events and in stored interaction evidence, has ordered keys `backend,native_version,profile_version,rule_id,screen_seq,truncated,evidence_policy,operation,operation_rows,choices,native_request_id,excerpt,evidence_digest`. `operation_rows` is the positive number of displayed rows the operation was read from; it is null exactly when `operation` is null, including under metadata evidence.

Run-level events use null interaction/classification, `source=runner_lifecycle` and a typed health/exit cause in the payload. Missing command, cwd, environment or native request identity stays null; screen parsing never fills it with invented execution data. Limit a UTF-8 excerpt to 8 KiB and the complete payload to 16 KiB, record truncation, escape control characters and omit raw terminal recording. Treat screen text as untrusted evidence, never as a command or instruction to approve.

Launch-only `--event-evidence <metadata|redacted>` defaults to `metadata`. Metadata mode persists only identities, classification, rule/profile/version, sequence, timestamps and health/closure causes; every screen-derived operation, choice, excerpt and native identifier is null. Semantic fingerprints used for live episode comparison remain in memory and appear in neither logs, outbox nor database. `evidence_digest` is SHA-256 of the canonical post-policy evidence object, so metadata mode exposes no hash of hidden screen text.

Redacted mode requires `--event-redaction-file <ABSOLUTE_PATH>` naming readable UTF-8 JSON `{ "schema_version": 1, "patterns": ["regex"], "sensitive_rules": ["rule-id"] }`. Require a nonempty pattern array, known rule IDs and compilable expressions. Missing/malformed configuration, an unsupported policy or a redaction file supplied with metadata mode fails before registration with usage exit `2`; there is no fallback to raw evidence. Compile the file once at launch and keep the compiled policy in memory; events store the policy name, not raw patterns or the file contents.

In redacted mode, apply the same configured patterns to every normalized screen-derived field before any persistence, including operations, choices, native IDs and latest interaction evidence. Replace matching fields in full with `[redacted]`; a profile-marked sensitive form or a rule named by `sensitive_rules` uses metadata-only evidence for the entire form. Apply UTF-8 bounds and escaping after policy transformation, then compute `evidence_digest`; persist no pre-policy screen hash or semantic fingerprint. Unmatched text is allowed only by this explicit operator choice, which does not promise automatic secret discovery.

Persist a transition, its event and a normal `input_required` inbox message to the pinned Director in one immediate transaction. The message's text is the full event JSON and its sender is the observed member, with the typed `source` distinguishing runtime observation from model-authored speech. The transaction shares the broker’s existing fleet/active-member checks. Insert the message and event, fill the final ID-bearing payload and validate the complete association before commit; no intermediate message body is externally visible. A retry of the same `(run_id, event_seq)` returns the existing event/message IDs and verifies identical payload identity; conflicting reuse fails.

Before persistence, the outbox stores an observed transition keyed by `(run_id, event_seq)` and nullable `episode_seq`, with kind, original observation time and immutable evidence. The transaction allocates or resolves interaction/event/message IDs and produces the final public payload. Idempotence compares these original transition fields; generated IDs and commit timestamps are not inputs to that comparison.

The runner uses the broker library directly and never depends on its child invoking `cafleet message send`. A persisted notification remains pending through Director absence or listener restart. Live events use `notification_state=inbox`; active ownership and the sender/recipient fleet checks remain mandatory. After closure, ordinary observation stops. Exclusive recovery may replay historical frames as `notification_state=archived`, `message_id=null`, with `archive_reason=member_inactive|fleet_deleted|director_inactive`; it retains the event/interaction history without inserting a message from an inactive sender or claiming Director notification.

`event show` returns the complete durable event with its notification/archive state and current interaction/run state, and `input show` returns a durable programmatic-input receipt (§ CLI presentation contract). Runtime events use the ordinary broker message envelope and ACK semantics.

##### Automatic review events

Both events follow the runtime-event contract above: an atomic event plus an `input_required` Director inbox message, the outbox, idempotent replay, archived recovery and the evidence policy. Neither creates an interaction row or changes input eligibility.

##### Automatic refusal event

| Field | Value |
|---|---|
| `kind` | `automatic_refusal_observed` |
| `interaction_id`, `classification` | `null` |
| `source` | `runner_screen` |
| `native_request_id`, `decision_outcome` | `null`, `"unknown"` |
| `evidence` | The ordered evidence object; `rule_id` names the refusal rule, and `operation`, `operation_rows` and `excerpt` follow the run's evidence policy (null under `metadata`) |
| `director_action.kind` | `review_automatic_refusal` |
| `director_action.inspect_argv` | `["cafleet","event","show","<EVENT_ID>","--json"]`, built from the stored event ID |

The `director_action.instruction` is: "Verify this event. The member's automatic reviewer refused the operation and no native request is waiting. Dispatch evidence comes only from the member's routed request with its exact command, cwd, environment and refusal; this event and screen captures corroborate it. With no routed request, dispatch nothing and ask the member whether the refused operation is still needed when it next reports or goes idle. For a routed request, decide in the Director session's effective auto mode whether to authorize a separate execution through the routed-command path or to refuse; otherwise obtain the user's permission first. Inspect the actual result and the member's continuation afterward." When the evidence has `operation_rows` greater than 1, the instruction appends the same displayed-rows sentence as the permission-request instruction (§ Request-bound permission responses).

The event has no linked interaction, so `permission inspect` and `permission respond` naming it fail their event/interaction association check before any byte.

##### Unavailable automatic review event

| Field | Value |
|---|---|
| `kind` | `automatic_review_unavailable` |
| `interaction_id`, `classification` | `null` |
| `source` | `runner_screen` |
| `native_request_id`, `decision_outcome` | `null`, `"unknown"` |
| `evidence` | The ordered evidence object; `rule_id` names the notice rule, `operation` and `operation_rows` are null, and `excerpt` follows the evidence policy |
| `director_action.kind` | `review_automatic_review_unavailable` |
| `director_action.inspect_argv` | `["cafleet","event","show","<EVENT_ID>","--json"]` |

The `director_action.instruction` is: "Verify this event. Automatic review is unavailable for this member, so native permission forms may appear and each needs a decision, and refusals may stop its work. Supervise the member as interactive, and tell the user when the selected automatic mode cannot apply." The member keeps running under the mode the native host actually applies; CAFleet never restarts it in another mode.

##### Input delivery result event

The runner emits one `input_delivery_result` event to the run's Director for a deferred input's first submitted, not-submitted or uncertain result. It follows the runtime-event contract above: an atomic event plus an `input_required` Director inbox message, the outbox, idempotent replay and archived recovery. It creates no interaction row and carries no literal payload.

| Field | Value |
|---|---|
| `kind` | `input_delivery_result` |
| `interaction_id`, `classification` | `null` |
| `source` | `runner_lifecycle` |
| `native_request_id`, `decision_outcome` | `null`, `"unknown"` |
| `evidence` | The run's ordered evidence object with null screen-derived fields |
| `cause` | `{"kind":UNCERTAIN_CAUSE}`, present only for an uncertain result whose receipt records an `uncertain_cause` (§ Uncertain cause), and absent otherwise. Every publisher derives it from the durable receipt, and readers require it to match that receipt |
| `input_id` | The deferred input's ID |
| `message_id` | The preview's message ID, or null for explicit prompt/exec input |
| `delivery` | The ordered `status,method,reason` projection of the stored receipt (§ Delivery reporting) |
| `stage` | `zero_bytes`, `partial`, `submitted` or `unknown` |
| `director_action` | `null` |

The terminal receipt, with any preview notification stamp, is the durable source of the event: the live runner publishes it after the receipt commits, and recovery publishes any durable first result that has no event yet. Publication is deduplicated by run and input identity, orderly shutdown publishes every pending result before `runner_exited`, and event listen/poll delivers it without injecting anything into the Director's pane. Immediate submissions publish no event; their command output and queryable receipt supply the result.

##### Effective policy record

The runner writes `member_runs.effective_policy_json` only from a tested annotation observation:

```json
{
  "schema_version": 1,
  "automatic_review": "unavailable",
  "observed_mode": null,
  "rule_id": "claude-auto-unavailable",
  "event_id": 512,
  "observed_at": "2026-10-29T02:30:00Z"
}
```

| Key | Type | Contract |
|---|---|---|
| `schema_version` | integer | `1` |
| `automatic_review` | text | `available` or `unavailable` |
| `observed_mode` | text or null | The native mode name the screen displays, after the evidence policy: null under `metadata`; under `redacted`, the displayed name with the configured patterns applied, or null when the screen names none |
| `rule_id` | text | The observing rule |
| `event_id` | integer or null | The emitted `automatic_review_unavailable` event; null for an `automatic_review_available` update |
| `observed_at` | text | UTC RFC 3339 observation time |

Only a change of `automatic_review` or `observed_mode` writes the record; repeated identical observations write nothing. Readers validate the shape and fail on a malformed record. A null record means no tested observation has been made; it is never filled from argv or the selected mode.

| Write | Transaction | Failure behavior |
|---|---|---|
| With `automatic_review_unavailable` | Part of the event's outbox frame and its immediate event transaction, so live commit and recovery replay apply it idempotently with the event | The event-transaction behavior |
| `automatic_review_available` update | Submitted to the event worker in observation order and committed through the ownership-checked transaction (owner epoch, database identity and historical ownership), additionally requiring `member_runs.status` `running` or `degraded` | A stale owner fails with `stale ownership token` and any other status fails as a `member_runs.status` invariant violation; the worker records a local diagnostic, drops that write without retry and continues |

The available update is not spooled: a runner lost before it commits loses only that observation, and the next indicator observation records it again. A superseded runner therefore never overwrites the record.

##### V6 storage

V6 widens the `member_runtime_events.kind` CHECK to the event kinds listed above and adds `member_runs.profile_match` (§8 *V6 automatic review migration*). SQLite cannot alter a CHECK in place, and the migration transaction runs with foreign keys enabled, so V6 rebuilds `member_runtime_events`, `member_permission_responses` and `member_permission_response_claims` in dependency order. It preserves every row, the event-ID sequence and every foreign key; the rebuilt tables keep their names, columns, other constraints and indexes. Every run recorded before V6 selected its profile by exact version, so V6 sets `profile_match='exact'` on each run with a `profile_version` and leaves the others null.

##### V7 storage

V7 drops `member_runs.profile_match` with its column-local CHECK, adds the ordinary-delivery and digest-version columns to `member_input_operations`, and widens the runtime-event kind CHECK with `input_delivery_result` (§8 *V7 minimum-version delivery migration*). It preserves the application and instance identities, every row and sequence, every foreign key and the audit relationships, rebuilding the event, permission-response and claim tables in the V6 dependency order. Existing input rows receive `payload_digest_version=1` with unchanged digest bytes and null delivery/deferred metadata, because those facts were not recorded.

##### V8 storage

V8 adds the nullable `member_input_operations.uncertain_cause` column in place, with a column-local CHECK restricting it to the § Uncertain cause values and to `uncertain` or `settled` rows (§8 *V8 uncertain cause migration*). It needs no table rebuild and preserves the application and instance identities, every row, ID, sequence, foreign key and audit relationship. Existing input rows receive a null cause, because the cause was not recorded.

#### CLI presentation contract

Member create/show append a `runtime` object (null for an explicitly unwrapped member) and a wrapped text row appends a `Runtime:` line; `event show`, `input show`/`reconcile`, `member recover` and wrapped prompt/exec have their own runtime outputs. §6.3 CLI *Runtime presentation*, *Runtime event and input commands* and *`member recover`* fix their exact keys, order, nullability and text layouts. The registry-only runtime fields let the Director bind a recovery attestation to the inspected owner snapshot without probing a live socket, and printed submission or settlement establishes no native execution result.

#### Storage pressure and durability boundary

An event worker owns SQLite and a private append-only per-run outbox. The parser/control loop submits immutable transitions through a bounded channel of 64 records. The worker fsyncs length-delimited, checksummed, redacted records before attempting SQLite. Retain committed frames while any complete frame remains uncommitted; truncate and fsync the file only after independently validating every complete frame's committed event/message association. Under exclusive ownership, diagnose and remove an incomplete final tail at the last valid boundary before further append; preserve complete corrupt bytes and fail explicitly. Replay valid complete records with their original run/sequence, and compact once the remaining frames commit so pressure can clear.

The current owner reserves event-sequence ranges of 256 by advancing `event_seq_reserved_through` in a bounded ownership-checked transaction before using them. The classifier consumes the reserved range in memory; if the next range is unavailable it enters the defined observation-gap state while terminal I/O continues. Unused reserved values are valid gaps. Recovery allocates new sequences above this durable high-water mark as well as all committed/complete-spool sequences, so missing artifacts cannot cause reuse of a previously issued sequence.

The Director-owned explicit `cafleet member recover <MEMBER_ID> --json` command operates on the recorded run of an active or deregistered member, never starts an agent, and returns recovery counts, final lifecycle/notification state and cleanup diagnostics. A five-second invocation deadline bounds each recovery pass. Start one cleanup clock at entry to the common recovery coordinator, before snapshot, locking, process/health evidence and replay; its existing two/three/four-second boundaries share that origin and are clipped to the supplied original deadline, including a deletion caller's earlier deadline. Incomplete replay retains the spool and `recovering` state for the next pass. `doctor` diagnoses recovery needs without mutating them. Member/fleet deletion invokes the same recovery component after successful runner shutdown and before deregistration when its remaining deadline permits; otherwise retain the namespace for a later explicit pass.

Every recovery counter measures durable work newly committed by this invocation.

| Counter | Commit domain |
|---|---|
| `replayed_count` | Distinct original `(run_id,event_seq)` transitions from this invocation's input spool whose event transaction first commits in this invocation. Already committed originals and identical duplicate frames contribute zero additional count. |
| `inbox_count` | Newly committed runtime event/message associations, including replayed originals and newly synthesized recovery transitions. Each association counts once after commit. |
| `archived_count` | Newly committed runtime events with archived disposition, including replayed originals and newly synthesized recovery transitions. Each event counts once after commit. |

A duplicate lookup, skipped frame, reserved sequence or failed/rolled-back transaction contributes zero. An incomplete report retains its already committed work; a later invocation counts only its additional commits. A transition synthesized during this invocation contributes to the inbox/archive domain rather than its replay count; if an interrupted invocation fsynced it without committing, its successful replay from the next invocation's input spool contributes to that next replay count. The count domains are independent: two unique uncommitted inputs, an identical duplicate, an already committed prefix and two newly synthesized inbox transitions yield `replayed_count=2,inbox_count=4,archived_count=0`. Database-only missing-path recovery has zero replay count while its newly committed closure/loss events contribute to inbox/archive counts.

Before returning an existing completed terminal result, validate supplied expected run/epoch against the currently inspected registry. A matching no-work return preserves existing status, diagnostics and attestation provenance, reports zero counts and advances no ownership; it observes terminal history even if the recorded current reconciler remains alive. A successful exceptional takeover advances the epoch, so repeating its original pre-takeover expected epoch is stale; any later owner change also invalidates an earlier expected epoch.

Recovery follows this sequence:

1. Validate complete database/run identity, acquire the existing pane lock when a concrete placement remains, then open/verify the recorded runtime directory and lock inode without creating a replacement. A matching held lock returns `live_or_unreachable`. A creator-owned run still in startup also requires creator/pinned-process reconciliation even if its precreated lock is free. A missing path, changed inode or unregistered startup owner follows the special branch below; it is not a free-lock result.
2. After acquiring the matching ownership lock, CAS the run's owner epoch to the next integer, record the recovering PID/start identity, set `owner_kind=reconciler`, preserve `recovery_origin_status` on the first pass and set `status=recovering`. A later pass takes a fresh epoch and retains the same original status. This fences prior event producers and input clients.
3. Under that exclusive ownership, validate the entire complete frame sequence before replay: checksums and sequence identities match, and `event_seq` is nondecreasing in physical order. Reconcile identical same-sequence originals idempotently; conflicting same-sequence originals or any physical sequence regression are complete corruption, which blocks the pass and preserves original spool bytes. Replay valid originals in their existing ascending order, updating interactions before later closures. An incomplete final suffix retains its separate tail diagnostic and exclusive-owner handling. Choose inbox or archived insertion from current member/fleet/Director status within each transaction.
4. After all complete original frames are reconciled, reserve new sequences above the durable reserved high-water mark, committed events and every valid complete outbox frame. Derive the terminal outcome from the original status and any replayed terminal lifecycle record. If neither establishes an exit/failure/loss, append a pending interaction's `monitor_lost` closure and then `runner_lost` through the same fsynced outbox/idempotent transaction path. Otherwise finish the recorded terminal outcome without inventing a loss; a replayed child exit supersedes the stale pre-crash running status. Repeated passes detect existing closure/loss records instead of generating them again.
5. Persist the final `lost`, `exited` or `failed` status, ended/cleanup evidence and notification disposition, then truncate/fsync the outbox after validating all complete frames are committed and release ownership. Reconcile verified descendants under the bounded cleanup rules; unknown ownership remains a diagnostic. Retain archived events permanently under database retention and delete the outbox only when all complete frames are durably represented.

Only the lock owner reads/compacts/writes the outbox, including normal event-worker operation. An external pass cannot race the live writer.

For a missing directory/lock or identity mismatch, a recorded owner still alive under its PID/start identity returns `live_or_unreachable`; unknown process identity returns `operator_reconciliation_required`. Preserve suspect state and the open interaction in both cases. An owner may hold an unlinked original lock, so creating/acquiring a new pathname supplies no death proof. Startup without a registered runner requires reconciling the creator, pinned pane and any started process before treating ownership as stopped.

For unreadable owner identity, expose the exceptional Director-owned `cafleet member recover <MEMBER_ID> --owner-terminated --expected-run-id <RUN_ID> --expected-owner-epoch <EPOCH> --evidence-ref <REFERENCE> --json` path. Require all four options together, with positive IDs/epoch and a nonempty reference to the operator's recorded attestation that the inspected owner and startup processes have terminated; incomplete options fail with usage exit `2`. Without that attestation, unreadable identity continues to return `operator_reconciliation_required`.

The Director obtains the run ID, owner epoch and PID/start identity from the inspected registry record and binds the operator evidence to that snapshot. Revalidate database/run identity, epoch and owner tuple after obtaining the SQLite write lock; a changed run or owner rejects the stale attestation rather than rebinding it. A matching live-process identity, held identity-matching lock or matching live health reply rejects the termination claim even with the flag. For an unchanged owner with unreadable identity and no positive live evidence, record `owner_termination_attestation` in `cleanup_diagnostics_json` with the original run/epoch, owner PID/start token, evidence reference, UTC timestamp and `source=operator_via_director`, then execute the database-only branch in the same CAS transaction. Preserve that provenance separately from machine-verified termination. This attestation enables bookkeeping closure of the identified owner only; it authorizes no child creation, input, approval answer or replay.

Once the recorded owner's termination is machine-verified or accepted through that explicitly attributed attestation, perform database-only reconciliation in one bounded immediate transaction, rereading the fixed database/run identity, placement, epoch, owner tuple and supplied attestation after obtaining the SQLite write lock. A completed terminal result uses the expectation-checked no-work return above; a concurrent live reconciler on unfinished recovery causes deferral. For unfinished recovery, advance the epoch and record the recovering process identity, retain separately attributed provenance, increment the gap count once and add `runtime_path_missing` to cleanup diagnostics.

Derive the terminal outcome from durable terminal records or the preserved original terminal status, retaining the exact recorded exit code/signal and existing event/message bytes. When a terminal outcome is established, finish that outcome with no new lifecycle event. Otherwise reserve sequences above the durable high-water mark, close any persisted interaction with `monitor_lost` before `runner_lost`, and finish as `lost`. In the same transaction, settle every deferred `ready` receipt as `not_submitted` with reason `runner_lost`, then publish each durable first deferred result that has no `input_delivery_result` event yet on sequences reserved above the high-water mark, deduplicated by run and input (§ Input delivery result event). Choose inbox or archived insertion from current lifecycle state within the same transaction. This branch performs no outbox replay and leaves surviving mismatched artifacts untouched; missing observations remain an explicit gap. Commit the terminal outcome and all new bookkeeping/records atomically. A failed transaction leaves the original state for retry.

Cap the outbox at 16 MiB per run. Retry database contention with bounded exponential delays from 100 ms to two seconds, each clipped to the same absolute database/work/shutdown budget rather than added to the connection's default timeout. Keep terminal transport and socket health service independent. A full channel/outbox or filesystem failure changes the runner to `degraded`, holds all programmatic input, emits one local health diagnostic and retains the failed transition plus an explicit observation-gap counter. Stop further semantic classification rather than silently discarding events, while continuing manual terminal I/O; resume with a fresh screen and an explicit gap event after the writer recovers. `runner_degraded` carries the gap interval/count, and events potentially missed during that gap remain unverified.

Durability begins at a successful SQLite commit; a spooled record is recoverable pending delivery, not a delivered notification. A hard crash before spool fsync may lose an observation; runner-loss reconciliation identifies that coverage gap. If both database and filesystem writes fail, use the control health response and local diagnostic, and preserve the actual error. Neither endless memory growth nor a claim of successful notification is a recovery strategy.

#### Input arbitration and the original native answer

For a runner-managed member, every CAFleet-generated preview, `member prompt` and `member exec` goes through the pinned runner control socket rather than multiplexer pane input. The client validates and submits without retaining the outer pane input lock while awaiting the runner. The runner owns that lock for wrapped writes: it acquires it nonblockingly from its event loop under the attempt deadline, revalidates the concrete placement, and retains it across an active staged write, so explicit press, deletion and other programmatic writers serialize correctly. It releases the lock while a request is merely queued. The runner serializes the final check and child writes with its own terminal-state loop. The legacy unwrapped path retains its client-owned lock.

Control frames are length-prefixed JSON, capped at 64 KiB, with protocol version, run/nonce/owner epoch, operation key, kind and literal payload. Encode the serialized UTF-8 JSON body length as a four-byte unsigned big-endian prefix; the inclusive 65536-byte body limit excludes that prefix, and an oversized declared length fails immediately after the header is read. Requests and replies require the top-level integer `protocol_version: 1`; missing or unsupported versions and truncated frames fail explicitly. The runner supports `health`, handshake `prepare`/`start`, `receipt`, `submit_prompt`, `submit_exec`, `permission_inspect`, `permission_respond` and `shutdown`. `receipt` has `query`, `retire` and synchronized Director reconciliation actions. Submission replies distinguish `accepted`, `queued`, `held`, `rejected` and `uncertain`, carrying a nullable input ID, output-sequence evidence, byte/submission stage and the delivery projection; the ID is null for a zero-byte attempt with no durable row, and `queued` preserves zero-byte stage evidence. Conflicting reuse of an operation key with another payload digest fails; store no literal prompt/command in the receipt table.

##### Input control frames and deadlines

New input, permission and receipt requests have required top-level `protocol_version,kind,identity,expires_at_mono_ns`. `protocol_version` is the integer `1`. `identity` has exactly `database_identity_json,fleet_id,member_id,director_member_id,run_id,owner_epoch,launch_nonce`, binding the complete selected database and recorded owner tuple. For submissions, validate recorded database/run/nonce and historical associations before durable key lookup; apply current epoch/owner and active eligibility checks only to a new or live ready attempt. IDs/epoch are positive signed 64-bit integers; sequences and byte counts are unsigned 64-bit integers. `expires_at_mono_ns` is a positive unsigned 64-bit CLOCK_MONOTONIC nanosecond expiration. Additional required request fields depend on `kind`:

| Kind | Additional exact fields |
|---|---|
| `submit_prompt` | `operation_key,input_kind,message_id,literal`; input_kind is preview or prompt. |
| `submit_exec` | `operation_key,input_kind,message_id,literal`; input_kind is exec. |
| `permission_inspect` | `interaction_id,event_id,director_member_id`. |
| `permission_respond` | `interaction_id,event_id,director_member_id,expected_run_id,expected_owner_epoch,request_token,decision,decision_source,director_mode,mode_evidence_ref,decision_evidence_ref,operation_key`. |
| `receipt`, action `query` | `action,operation_key`. |
| `receipt`, action `retire` | `action,input_id`. |
| `receipt`, action `reconcile` | `action,input_id,expected_state,disposition,evidence_ref`. |

Preview requires a positive message ID; prompt/exec require null `message_id`. `literal` is the unchanged message/prompt/command text; the exec plan supplies its own `! ` prefix. Require nonblank operation keys, validate the ordinary single-line/control-character contract and enforce matching outer kind/input_kind. Permission IDs/expected epoch must agree with identity and the stored association. Reconcile is the private action used by the Director-owned input coordinator; its enum values match the durable state/disposition domains. Reject unknown, duplicate, missing or wrongly typed fields before effects. Existing health/prepare/start/shutdown frame compatibility stays unchanged.

Replies retain exactly `protocol_version` plus one of `result` or textual `error`. Input results have exactly `kind` plus the following field: submission/`submission`, inspection/`request`, permission_response/`response`, or receipt/`receipt`. Submission contains exactly `status,input_id,stage,output_seq,reason,delivery`, where `delivery` is the ordered `status,method,reason` projection (§ Delivery reporting); permission response and inspection carry the complete ordered public objects fixed by the CLI contract. A receipt object has exactly `status,input_id,submission`: missing has null ID/submission; present has a positive ID and submission object; retired or settled has a positive ID and null submission. Result kind must match the requested operation. Clients validate and reload durable input/audit rows from their supplied database connection; socket receipt metadata supplies no replacement durable identity.

The original invocation captures one expiration and carries it through framing, transit, queues, SQL and every native stage. Convert using paired clock samples `Instant_before,CLOCK_MONOTONIC,Instant_after`, validating sample order and checked arithmetic. Sender expiration adds the remaining original deadline measured at Instant_after to the earlier monotonic sample. Receiver deadline adds remaining expiration measured at the later monotonic sample to Instant_before. Scheduling delay can shorten this budget and grants no extension; use no wall clock. Clock failure, overflow or expiration fails before new reservation/native bytes while preserving prior committed or possibly transmitted state.

At the service, also cap new-operation admission to five seconds from the first observed frame-header byte and any earlier shutdown deadline. Delayed framing and queueing consume that same cap. Client exchange, lock acquisition, supplied-connection SQL scopes and stage advancement retain their earlier original/inherited deadline. The frame body limit remains 65536 serialized UTF-8 bytes, excluding its four-byte prefix; validate declared size before allocation and the complete serialized size before sending.

##### Bounded input draining

Before a programmatic input attempt, drain actual human and child input through the existing manual queue, terminal parser, ordered observer and outer display. A drain returns either `Pending` or `Quiescent` with the actual cumulative `human_generation`, the count of bytes classified as manual input (§ Terminal reply classification), and terminal `output_seq`. Quiescence describes transport and observation health; the input coordinator separately validates composer/request eligibility, ownership, reservation and the original deadline before a write.

Each call executes at most eight rounds under the original deadline. A round attempts at most one human read, one manual-queue PTY write, one child read, one display-queue write and one display flush, in that order. Skip an operation only for unavailable capacity or remaining quota, an empty queue or previously observed stream `EOF`. Let B be the positive configured `bytes_per_tick`; each actual read buffer is bounded by the minimum of 16384 bytes, the remaining stream quota and available queue capacity.

| Work per drain call | Inclusive bound |
|---|---|
| Human reads | Eight calls and B positive bytes. |
| Child reads | Eight calls and B positive bytes, including previously deferred child bytes. |
| Manual PTY writes | Eight calls and B positive bytes. |
| Display writes | Eight calls and B positive bytes. |
| Display flushes | Eight calls. |
| Final observation-health samples | One sample, taken only for a candidate completed quiet round. |
| Rounds | Eight; unresolved work returns `Pending`. |

`Quiescent` requires both actual reads to return `WouldBlock` in the same completed round, an empty manual queue, consumed deferred child bytes, successful parser/observer processing and positive free display capacity. The final health sample must be present, nondegraded, have `pending_count=0` and `pending_evidence=false`. An absent health adapter supplies no proof; a healthy `Hold(Unknown)` state may establish physical quiescence while remaining ineligible for programmatic input. `Quiescent` also requires no pending terminal-reply prefix: each round first resolves expired prefixes, and a round that would otherwise certify quiescence while a prefix is pending returns `Pending` with blocker `terminal reply pending` and installs no proof. The eighth round may certify these actual facts; quota exhaustion, skipped reads, `Interrupted`, `EOF`, outstanding manual bytes, a pending reply prefix or display pressure supply `Pending` instead. A continuously readable source terminates at the round or byte bound. Deferred child bytes precede subsequent reads and consume the same child quota.

Classify every actual positive human read immediately and count its manual bytes with checked cumulative unsigned 64-bit arithmetic, in both ordinary progress and input draining. Retain those counts and literal queued bytes when later PTY, child, display, parser or observer work fails. The service delivers the observed before/after count delta to manual-input invalidation on both successful and failed progress before propagating any error; a simultaneous notification failure accompanies the primary error. Actual nonrecoverable errors propagate, and a zero-length write retains the existing no-progress error. `Interrupted` requires a later fresh read pair; `EOF` records the stream state and leaves shutdown handling to the normal service path.

Clear stored drain proof at entry, then check the original deadline before I/O and before each endpoint/callback. Recheck after completed work, retaining the actual bytes/counts from successful completed I/O before reporting expiry. In-memory bookkeeping and observation of a completed read may finish after expiry; an observed expiration permits no later endpoint call or proof installation. Synchronous I/O and callbacks have no preemption guarantee, so finite work bounds and clock checks supply no hard syscall or callback latency cap. Every call retains the original deadline and performs no internal sleep or deadline renewal.

Retain every completed child read for display and preserve the exact prefix processed by the terminal parser and ordered observer. If a positive child chunk fails to complete that observation, retain an observation-incomplete fence for the `Transport` lifetime. Subsequent drains continue bounded manual and display progress and return `Pending` without proof, including when later health is healthy or the caller supplies a new deadline. Ordinary progress, resize and display flushing retain the fence. Fully completed observation remains complete when only a later deadline or display operation fails. Partly processed chunks are neither replayed nor treated as observed suffixes; the runtime exposes no repair/reset operation for this fence.

Only a successful final check installs an opaque, single-use proof bound to its transport instance, drain turn and actual health/generation/sequence facts. The visible result fields describe evidence and grant no write authority. Taking the stored proof consumes it once, and validating it while a terminal-reply prefix is pending fails with `input transport is no longer quiescent`. Ordinary progress, resize, prefetch, display mutation/discard and a new drain invalidate it before their validation or I/O, including later error paths; pure generation/snapshot reads preserve it. A new transport begins with generation 0 and no proof. Programmatic writing consumes actual coordinator-owned authority rather than a caller-constructed result.

##### Durable input receipts and settlement

Preview keys are exactly `preview:<run_id>:<message_id>` and are independent of CLI invocation or connection. Every submission first checks the durable operation row, including retired dispositions. Before a later send/wait retries an unstamped message, it queries the receipt. Durable `accepted` permits stamping without typing; `in_progress` or `uncertain` holds for inspection, a queued `ready` row returns its queued result, and a settled key returns its disposition without replay. Only absence of a row under the same verified run/database identity, or a recorded `ready` zero-byte state without runner queue ownership, permits reevaluating input eligibility. A queued request commits its `ready` receipt with deferred metadata (§ Runner-owned deferred delivery); a held/rejected zero-byte attempt creates no side-effect receipt; if a row was already reserved, move it to `ready` only after positively recording that zero bytes were written.

An ordinary `receipt query` reads the durable operation for its fixed run/key and reports its durable status together with the last real retained stage and output sequence. `missing` requires an absent durable row and absent matching cache entry. A durable row remains `present` after cache retirement; a cache entry without its durable reservation is an identity error. Validate the retained immutable input identity before using its stage.

| Durable receipt | Status | Stage without a cache entry |
| --- | --- | --- |
| `ready` with deferred metadata | `queued` | `zero_bytes`. |
| `ready` without deferred metadata | `held` | `zero_bytes`. |
| `in_progress` or `uncertain` | `uncertain` | `unknown`. |
| `accepted` or `settled/submitted` | `accepted` | `submitted`. |
| `settled/not_submitted` or `settled/abandoned_uncertain` | `rejected` | `unknown`. |

With a matching retained entry, return its actual stage and output sequence alongside that durable status; otherwise the output sequence is null. Query preserves pending writers, clients, permits, receipts and native bytes. Its results describe observations: new write authority and zero-byte retirement proof still require their separate validated guards. A settled `not_submitted` or `abandoned_uncertain` disposition alone supplies no observed byte stage; its absent-cache result remains `unknown` and leaves the original transmission uncertainty intact.

Before the first child byte, commit `in_progress` to `member_input_operations` under the current ownership epoch and the bounded database budget. If that commit fails, write no child bytes. After submission, persist `accepted` or `uncertain` before promising that durable outcome; failure to persist after a possible write leaves the durable row in progress and returns uncertainty. The live runner may reconcile its retained stage evidence into the same row later; a crashed runner's in-progress row is never an automatic replay instruction.

Retire only the in-memory receipt after validating durable settlement, the current full run/owner and original run lock, exclusion of every active writer, and absence of a leased guard or available attempt permit for that input. Cache absence is idempotent only after the same durable and ownership checks. For a preview, atomically stamp `notified_at` (or verify the message is already ACKed) and set its input row to `settled/submitted`; a later `receipt retire` verifies that transaction before freeing memory. For prompt/exec, the durable accepted input row itself is the queryable submission record: the creator or runner can settle it as submitted and retire the cache after verifying that stored row. Printed stdout is merely a presentation of its input ID and supplies no durable acknowledgement. Retain settled operation keys/digests/dispositions for the run's database lifetime, so a delayed request still returns the prior disposition and never types again.

Bound unretired side-effect receipts, including queued deferred inputs, to 4096, rejecting additional new side-effect attempts while that unresolved set is full with an explicit zero-byte capacity error; settled receipts leave the bounded set. Keep uncertain receipts until the Director records an evidence-based disposition through `cafleet input reconcile <INPUT_ID> --disposition <submitted|not-submitted|abandoned-uncertain> --evidence-ref <REFERENCE> --json`. This Director-owned operation only records disposition, settles the key and permits cache retirement; it sends no bytes, grants no native permission and schedules no replay. `input show` exposes that durable record. Even a settled not-submitted key is consumed; a later deliberate action uses a new operation key after reconciliation.

Reconciliation pins the original input row in its supplied-connection snapshot: `input_id`, `run_id`, `operation_key`, `kind`, `payload_digest`, `message_id`, `created_at`, the complete immutable permission-response audit where present, and expected state remain fixed through live preference and exclusive settlement. For a recorded concrete placement, acquire its genuine pane/input lock and retain that same guard through current-owner synchronization and any receipt-only exclusive ownership proof. Synchronize with the pinned runner's receipt handler so an active byte-writing operation cannot be settled concurrently; an unreachable owner requires the exclusive proof below. Compare-and-swap the original expected operation state in the settlement transaction; a changed row requires a separate invocation. Settlement records the observed input disposition, not the native command's execution outcome.

For an ordinary or permission receipt already settled with the requested disposition, reconciliation returns the unchanged validated durable row and its original `evidence_ref` before live locks, health probes or exchange. A conflicting disposition fails with the row unchanged. This historical return uses the original deadline clipped to any inherited SQL bound and grants no new input authority. For a live nonsettled receipt, validate the full fixed identity and original ownership lock inside the same supplied-connection immediate transaction as the expected-state settlement CAS, after the SQL wait. After a new reconciliation, require the matching `settled` acknowledgment and a validated durable reread, then exactly one `retire` exchange with a matching input ID under the same pane guard and original expiration. A post-commit failure retains the known input context and original cause while the durable settlement remains authoritative. A changed expected state requires a separately reconciled attempt.

Receipt-only reconciliation for ordinary and permission inputs accepts the fixed placement state from that original snapshot. A present placement requires its matching, continuously held pane/input guard. An absent placement requires an absent guard, valid historical member/fleet/Director relationships and a deregistered member or deleted fleet; an active member in a nondeleted fleet requires its recorded placement. Revalidate the exact placement or its exact absence, complete fixed run/input identity and historical relationships inside the supplied-connection immediate transaction after preceding waits. A changed placement, identity or expected input state invalidates the attempt.

Both placement states require the recorded identity-matching original run lock and positive exclusion of the previous owner and relevant startup processes under the original deadline clipped to any inherited SQL bound. A held original lock, matching live process/health, unresolved termination or competing live reconciler preserves uncertainty. Missing or replaced runtime paths supply no original-lock proof. Placement absence, a failed live call and prior exceptional bookkeeping each grant no takeover authority; retain the original live failure when typed exclusion cannot be established. Valid absent-placement history proceeds directly to the same exclusive proof.

Its receipt-only CAS advances only the ownership tuple: increment `owner_epoch`, set `owner_kind=reconciler` and record the current reconciler PID/start identity. Commit that CAS, expected-state settlement and any matching claim removal authorized by a genuine zero-byte proof atomically in one supplied-connection immediate transaction; any failure rolls all those mutations back. Preserve lifecycle status, recovery origin, exit fields, cleanup state/diagnostics, events, reservations, spool and immutable input/audit history. Retain the exclusive guard through commit and release it on every outcome. This path performs no observation recovery, child operation or native input; ordinary member recovery retains its completed-terminal no-work behavior.

A durable ready row is positive zero-byte evidence only under actual writer exclusion and exact associations. Owner death and an attributed not-submitted disposition alone supply no such proof; recovered in-progress, accepted and uncertain records retain their transmission uncertainty. Proven-zero settlement and matching claim release commit atomically under the same proof lifetime. A ready response from a different epoch/token requires a fresh inspection and separate deliberate key; its immutable metadata is never rebound.

The live input guard distinguishes native-write authority, receipt-only writer exclusion and current-plan ready rearm. A production zero-byte proof borrows its still-held guard, binds the complete immutable input/audit and current ownership, and carries a non-Send marker; its constructors and fields remain private. Receipt-only exclusion authorizes bookkeeping without a write permit or current visible prompt. With a retained entry, proof requires its actual zero-byte stage and cumulative count zero; without one, only a validated durable ready row supplies that fact under genuine writer exclusion. A projected retry stage or a settled not-submitted label supplies no replacement observation.

Rearm additionally requires the unchanged old token/epoch/metadata and the actual current request, selected answer plan, manual generation and healthy complete observation drain. Revalidate owner/audit/claim under the bounded immediate transaction, CAS ready to in-progress once, then require fresh normal pre-stage checks before any native byte. Retain the input ID, original audit, decision timestamp, key and digest. Preserve its matching claim, or restore that exact pair only when a released ready row has no competing claim. The exclusion-only proof grants no rearm authority. Each explicit ready invocation captures one expiration; an in-progress stage continuation retains its original attempt expiration.

Live permission query, reconciliation and settled retirement use the existing serialized receipt operations and original run lock. Query preserves the last real retained stage and any pending writer/client. Reconciliation pins the original full input/audit snapshot, excludes active writers and leases, and commits a proven-zero not-submitted settlement plus matching claim deletion in one supplied-connection immediate transaction. Actual retained zero-byte in-progress evidence may establish that synchronized settlement; possible-byte or absent-cache uncertainty retains its claim. Submitted and abandoned-uncertain dispositions retain claims. A mutation failure rolls back settlement and claim removal together. Retirement removes only settled cache after validation; durable audit, key and claim history remain. The client retains its genuine pane guard and original inherited deadline through query, settlement, durable verification and one matching retire; already-settled same-disposition history returns unchanged before live work. An unavailable permission owner uses the same fixed-snapshot receipt-only exclusive guard. Only exact durable ready history supplies its zero-byte proof after positive prior-owner/startup exclusion and ownership CAS; disposition and matching claim removal share that transaction. Possibly transmitted history retains its claim. The native startup probe reports unknown, so unavailable exclusion preserves the original live failure.


A process loss, ownership change or unavailable receipt permits no automatic replay of an attempted submission. Keep notification unstamped, retain its durable uncertainty and reconcile actual output before a separate deliberate action. Explicit prompt/exec use generated keys and printed durable input IDs with the same rule. Test cross-invocation accepted-then-failed-stamp recovery, queued-then-drained delivery after interaction closure, disconnected partial writes, receipt retirement followed by delayed replay, and a lost explicit-command stdout result.

Native-exec success means submission, not execution success. Queued and held messages keep `notified_at` unset, and only confirmed submission or the existing ACK settlement stamps them. Queueing supplies no new permission event and never closes the native request.

Screen observation cannot create an atomic native request API: unread child output may race with a write, an unknown CLI can redraw unexpectedly, and a person can type concurrently. The runner therefore guarantees zero ordinary-input bytes for detected and candidate interactions and refuses uncertain states; tested composer profiles bound the remaining race for profile delivery, rather than asserting universal prompt preservation. Release evidence must include this race and preserve uncertainty instead of authorizing a second submission.

Human keystrokes from the outer terminal pass to the child as native input, including the human's intentional approval, denial or cancellation. A Director may submit a request-bound allow or deny through the permission-response path below after applying its session decision policy; ordinary preview/prompt/exec input stays queued with zero bytes during the request. Keep attributed decisions, key submission, native outcomes and actual continuation separately observable. Unknown answer capability or uncertain request identity preserves manual original-surface operation. Only explicitly unwrapped members use direct multiplexer input, with its narrower semantics (§6.5 Prompt dispatch).

##### Delivery method and interaction priority

These ordered rules select the delivery of every wrapped preview, `member prompt` and `member exec`:

1. Validate target/run identity, reachability, writer ownership, storage and receipt history. A failed invariant or an uncertain prior attempt is an explicit operational failure, never a reason to send fallback bytes. Unknown detection coverage alone is valid capability absence, not an observer-health failure.
2. A candidate or open detected interaction queues ordinary input with zero bytes. An obscured interaction remains preserved, and `permission_request`, `question`, `trust`, `update` and `unknown_interaction` are all blocking interactions.
3. A verified empty composer with the operation's executable plan uses profile delivery without Escape (§ Measured submission plans and settle intervals). This includes a working member whose composer row is empty: the native CLI queues the submitted input behind its current turn without interrupting it.
4. With an executable plan and a composer that is not verified empty (a draft is present or the observer cannot classify the composer), queue the input with zero bytes under defer reason `composer_busy`. The runner drains it by profile delivery as soon as it observes a verified empty composer. A queued head that has waited 3 minutes only on a composer that is not verified empty is delivered by fallback with reason `idle_timeout`. The bound starts when the head first waits solely on that composer and restarts whenever an interaction blocks it; time spent behind an interaction or earlier input never counts. A detector that stops matching a newer native UI therefore delays delivery rather than stranding it.
5. Otherwise use delivery fallback: Escape, settle 500 ms, literal prefix and text, settle 500 ms, Enter (`0d`). This covers missing operation plans, missing profile coverage and failed version probes.

| Selection | Bytes | `delivery.method` | Selection reason |
|---|---|---|---|
| Queued behind an interaction, a busy composer or earlier input | Zero | `null` until a later attempt selects one | `interaction_open`, `interaction_candidate`, `composer_busy` or `earlier_input` |
| Profile delivery | Literal stage, 500 ms settle, measured submit key | `profile` | `null` |
| Fallback, no executable plan or profile | Escape, settle, literal stage, settle, `0d` | `fallback` | `submission_plan_unavailable` |
| Fallback after 3 minutes queued behind a composer that is not verified empty | Escape, settle, literal stage, settle, `0d` | `fallback` | `idle_timeout` |

The Director interrupts a working member explicitly with `cafleet member press MEMBER_ID Escape`; ordinary delivery never sends Escape to a member whose profile has an executable plan before the 3-minute bound.

Unknown-profile and failed-probe runs use `submission_plan_unavailable`. Fallback executes through the owned child PTY and the same input coordinator and receipts, never through a second multiplexer input writer. It preserves the caller's literal and the exec `! ` prefix. Revalidate the higher-priority interaction rule before Escape and before every later stage: a newly detected interaction before the first byte requeues the input, and one after a positive write stops the remaining stages with uncertainty.

Fallback may interrupt work, mix text with a draft, and dismiss an undetected permission request or question on an unknown-coverage run (§6.5 Accepted trade-offs). The guarantee is suppression for detected and candidate interactions, not detection of every native form. Semantic permission answers continue to require their independently tested request-bound plans and never use fallback. Human keystrokes and request-bound permission responses keep their separate paths and can resolve the interaction that blocks ordinary input; closing that interaction grants no permission and supplies no native execution result.

##### Runner-owned deferred delivery

The runner owns a FIFO of ordinary input requests, including previews. It accepts a blocked request into the queue only after allocating its durable input ID and committing a `ready` receipt with `deferred_at` and `defer_reason` under its operation key, digest and target identity. The literal or control payload stays in runner memory; receipt tables, result events and diagnostics retain metadata only. The 4096 unresolved-input limit bounds queued plus active work, and a full queue returns an explicit zero-byte capacity error. A repeated operation key with a matching digest returns its queued or recorded result; a different digest fails.

Only one ordinary request writes at a time, in enqueue order, and a request queued behind another receives the same explicit queued result. Queue processing is scheduled from the runner event loop:

| Trigger | Runner action |
|---|---|
| `interaction_closed` after its event transaction commits | Drain pending output and revalidate the current state; a superseding interaction keeps the queue blocked, and closure due to exit or monitor loss dispatches nothing |
| A candidate clears | Retry the queued head after rechecking the current interaction state |
| The active input completes | Retry the queued head after rechecking the current interaction state |
| A temporary zero-byte obstacle during an active attempt | Retry on a 100-millisecond timer within the same five-second attempt deadline |

No external `message send`, `message wait` or Director wake is required to resume the accepted queue. Immediately before writing a queued preview, recheck that its message remains unconsumed and its target active; an ACK or target removal cancels its zero-byte receipt as not submitted and advances the queue rather than typing an already consumed preview.

A queued request outlives the originating CLI connection and command deadline. Each actual drain attempt receives the five-second ordinary-input budget, while waiting for an interaction consumes none of it. A client timeout after queue acceptance reports the known input ID and directs the caller to inspect the receipt, rather than generating a new operation key. The 100-millisecond timer retries lock acquisition, storage readiness and observer health even without child output or broker commands, preserving the input ID, FIFO position and `ready` receipt without renewing the deadline. A reappearing candidate or open interaction pauses dispatch and ends the zero-byte attempt; the next closure or clearance starts another bounded attempt.

| Deferred-attempt condition | Receipt and queue outcome | Result publication |
|---|---|---|
| Busy pane lock or temporary storage/observer unavailability before the deadline, with zero bytes | Retain the queued head and retry on the 100 ms timer | Input inspection remains `queued` with its original defer reason |
| Five-second deadline expires with zero bytes | Settle `not_submitted`, clear the unused selected method, and advance after the terminal transaction commits | `input_delivery_result` with reason `pane_input_busy`, `storage_unavailable`, `observer_unavailable` or `delivery_budget_exhausted`, according to the last obstacle |
| Target/run/placement identity fails revalidation or a queued preview is consumed, with zero bytes | Settle `not_submitted` immediately and advance after commit | The result with reason `target_inactive`, `target_changed` or `message_consumed` and diagnostic context |
| Any attempt may have written a child byte | Record `uncertain`, stop automatic ordinary dispatch and retain its method and selection reason | The uncertain result; existing receipt reconciliation is required before advancing |

If storage prevents the terminal transaction, discard the zero-byte payload and retain a metadata-only pending finalization at the FIFO head. Retry that transaction on the 100 ms timer without sending bytes or starting another input; commit the receipt when storage recovers, then advance, and its result event follows from that durable receipt. While publication is unavailable, write a runner-stderr diagnostic naming the input ID and persistence failure; the Director receives the durable result after recovery. Ownership loss transfers finalization to the existing fenced recovery path, and the old writer neither updates nor dispatches again. An accepted request therefore becomes queued, submitted, not submitted or uncertain; `held` remains a result for work the runner did not accept into its queue.

Orderly shutdown cancels queued zero-byte requests as `settled/not_submitted` with reason `runner_stopped`. After runner loss, recovery may settle a deferred `ready` receipt as not submitted with reason `runner_lost` once ownership fencing establishes that its in-memory payload and writer are gone. Explicit payloads are never restored from disk; a later deliberate submission is a new request. Existing `in_progress`/`uncertain` reconciliation applies to any attempt that may have written bytes. A preview settled not submitted remains an unnotified broker message and may be retried explicitly under a new run's preview key.

##### Delivery reporting

`message send`, `member prompt` and `member exec` JSON append a `delivery` object with ordered `status,method,reason` (§6.3 Runtime presentation). `method` is `profile`, `fallback`, `unwrapped` or null before selection; an explicitly unwrapped submission reports `unwrapped`. The shared projection is:

| Status | Meaning | CLI behavior |
|---|---|---|
| `submitted` | Complete literal and submit stages were written | Exit 0; send has `notification_sent=true`, exec has `dispatched=true` |
| `queued` | The runner accepted durable zero-byte deferred work | Exit 0; send has `notification_sent=false`, exec has `dispatched=false`; the input ID and reason are returned |
| `held` | No queue acceptance or write, due to unavailable health, ownership or storage | Send preserves its committed message and prints the reason; explicit prompt/exec exits 1 with the existing delivery context |
| `uncertain` | Some stage may have written; reconciliation is required | Existing error categories, input identity and stage diagnostics, including the recorded `uncertain_cause` (§ Uncertain cause) and the selected method |
| `not_submitted` | Work was cancelled with established zero-byte evidence | Visible through input inspection and, for deferred work, its result event |
| `not_applicable` | A self-send or root-Director target uses broker consumption without automatic pane delivery | Send retains `notification_sent=false`, null method and input ID, and no warning |

A held `message send` reports one of these reasons:

| Held reason | Cause |
|---|---|
| `observer_unavailable` | The wrapped recipient's runner cannot currently observe its native screen, so it accepted no queue work |
| `notification_held` | No deliverable pane, a recipient deregistered after the commit, a placement or run change during the attempt, or an unresolved earlier preview receipt |
| `notification_failed` | The pane stayed busy, the multiplexer or runner exchange failed, storage failed, or the budget could not cover a first preview |
| `delivery_budget_exhausted` | The send's delivery budget ended after earlier pending previews were delivered |

Self-send evaluation precedes recipient delivery: equal sender and recipient IDs return `{"status":"not_applicable","method":null,"reason":"self_send"}` and a null `input_id`, also for an ordinary member with a healthy wrapped pane. Self-sends stay excluded from automatic preview selection and the runner queue. Other sends to a root Director use reason `director_stdout`. A queued preview counts as queued only: `notified_at` stays null until actual submission or the existing ACK settlement.

Preview body folding, inline limits and long-body pointers are unchanged (§6.5 Push notifications). Profile delivery submits the canonical wrapped preview ending at the resume clause; fallback appends the existing Escape-dismissal clause, so a member can attribute a delivery-caused rejection of an undetected prompt correctly. The runner derives the rendering from the selected method before the first byte.

New preview receipts use payload digest version `2`. The preview client sends the canonical base preview, the method-independent wrapped line ending at the resume clause, in `SubmitInput.literal`. The runner reloads the immutable broker message, validates its run/recipient/key relationship, reconstructs the same canonical base and requires byte equality before reserving or reusing the receipt. The digest is SHA-256 over the canonical JSON encoding of:

```json
{"payload_digest_version":2,"run_id":42,"kind":"preview","message_id":99,"literal":"CANONICAL_BASE_PREVIEW"}
```

Method, appended clause, native prefix/key bytes and timing are outside the digest preimage. The runner selects and renders afresh only while zero bytes are established; once a stage writes, its method and rendered bytes stay pinned through submission or uncertainty. Queries and duplicate requests validate canonical identity before considering current composer state, so a later method change neither creates a different digest nor authorizes retyping an existing input.

Each input row persists its `payload_digest_version`. New explicit ordinary inputs and permission-response inputs keep their version-1, kind-specific digests. Version-1 preview receipts validate with the frozen pre-V7 wrapped-preview renderer and the preimage `run_id,kind,message_id,literal`, without a version field or a method-dependent clause. Read a receipt's version before computing its expected digest, and choose version `2` only when allocating a new preview receipt, atomically with reservation. An unsupported version or a version/kind mismatch fails explicitly. This version is stored request identity, separate from any native profile version.

`defer_reason` records why a request first entered the queue and stays paired with its original `deferred_at`. `delivery_reason` records the eventual selected-method reason or a terminal zero-byte cancellation reason. The owned transaction that changes an ordinary receipt to `in_progress` commits its `delivery_method` and `delivery_reason` before the first byte: profile delivery has a null reason, and fallback has `submission_plan_unavailable` or `idle_timeout`, also for an immediate fallback. A zero-byte requeue clears unexecuted selection metadata before any later reselection; a terminal zero-byte cancellation commits a null method and its outcome reason. After any possible positive write, the selected method and reason persist through accepted, uncertain and settled states.

Every result consumer projects these durable fields rather than recomputing a reason from the current catalog or composer. Receipt outcome, eventual method and reason and preview notification settlement commit together, and the deferred-result event follows from that durable receipt (§ Input delivery result event).

| Ordinary receipt state/disposition | Delivery status | Method and reason |
|---|---|---|
| `ready` with deferred metadata | `queued` | Null method; original `defer_reason` |
| `ready` without deferred metadata | `held` | No active queue ownership; null method and `zero_bytes_ready` |
| `in_progress` or `uncertain` | `uncertain` | Recorded selection; no automatic dispatch or replay |
| `accepted` or `settled/submitted` | `submitted` | Recorded selection |
| `settled/not_submitted` | `not_submitted` | Null method and recorded outcome reason for a zero-byte cancellation; recorded selection for a reconciled attempt that may have written bytes |
| `settled/abandoned_uncertain` | `uncertain` | Recorded selection; abandonment ends retries without establishing zero bytes |

An ordinary receipt whose method and reason were never recorded, including pre-V7 history, takes its status from the same mapping with a null method. A `ready` row without deferred metadata uses `zero_bytes_ready`, and every other such row uses `method_unrecorded` rather than an invented method or cancellation reason. Missing deferred metadata establishes no queue ownership. Permission-response receipts expose `delivery:null`, null deferred metadata and digest version `1`, and their semantic decision/submission/settlement projection remains authoritative. Unknown stored combinations fail validation.

An asynchronous completion leaves the originating CLI output unchanged. The runner persists the actual method in the input receipt, `input show` exposes it, and the first submitted, not-submitted or uncertain result of a deferred input emits one `input_delivery_result` event (§ Input delivery result event).

###### Uncertain cause

Every runner transition of an input from `in_progress` to `uncertain` records exactly one `uncertain_cause` in the same update, and storage refuses that transition without a cause. The cause attributes the uncertainty; it is separate from `delivery_reason`, which keeps the selection reason unchanged after a positive write.

| Value | Recorded when |
|---|---|
| `manual_accounting_failed` | Manual-input accounting failed when a staged stage stops |
| `manual_input` | The manual-input generation changed since the literal stage when a staged stage stops |
| `interaction_candidate` | A staged stage stops on a blocking interaction whose eligibility is held as a candidate |
| `interaction_open` | A staged stage stops on any other blocking interaction: a visible or obscured interaction, an interaction hold or a retained transition |
| `observer_unavailable` | A staged stage stops because the observer is unavailable, has a gap or reported an observation diagnostic |
| `short_write` | A programmatic write returned a positive count smaller than its stage |
| `write_failed` | The attempt failed after a possible child byte: a PTY write error, a post-write checkpoint, manual-accounting or deadline failure, a failed deferred attempt or a failed permission-response attempt |
| `runner_stopped` | Runner shutdown ended an active deferred attempt that may have written a child byte |

At the settle check before a staged submit, the first matching row among the first five, in table order, wins; accounting failure comes first because the generation comparison is unreliable once accounting failed. A stage that stops on an interaction after an earlier stage wrote records `interaction_candidate` or `interaction_open` by the same candidate test. A partial or unknown write outcome records `short_write` for a positive short count and `write_failed` otherwise.

Settlement and reconciliation preserve the recorded cause. Inputs that never became uncertain, inputs recorded before V8 and `in_progress` inputs left by runner loss have a null cause. `input show` and `input reconcile` report the cause, an uncertain submission reply carries it as `reason`, and an uncertain `input_delivery_result` event carries it as `cause`.

##### Explicit manual key input

An explicitly authorized `member press` sends only its requested ordered keys through the member's outer multiplexer pane under the shared pane lock and selected-context/deadline contract (§6.5 Explicit key dispatch). It appends no Escape or Enter. This manual path remains available during observer degradation or a programmatic composer hold and follows the separate CLI acknowledgment/error contract (§6.3 `member press`).

The runner counts the manual bytes of actual positive outer-terminal reads, excluding complete terminal replies (§ Terminal reply classification), retains their generation before any later I/O error and accounts each new delta once on both successful and failed transport steps before subsequent automatic advancement. Observed manual input invalidates automatic authority through the retained generation; a key argument or backend acknowledgment supplies no byte count, drain proof or permission receipt. Press changes no durable input/response row. Native approval/denial/cancellation and actual continuation remain independently observed outcomes under the existing Director/user policy.

#### Request-bound permission responses

The Director uses its own session's effective approval mode, independently of the member's launch mode. In verified auto mode, evaluate the requested command or operation and decide allow or deny. In other or unknown modes, obtain the user's request-specific permission before transmitting an answer. Resolve mode from current host/session evidence and record its provenance in host-owned pending state; member argv, event text and a CLI flag supply no proof of Director mode. Native and managed host policy remain authoritative. Mode/source fields are attributed audit claims in this same-user coordination tool, not authentication credentials.

Runtime event payload schema version 2 includes the broker-generated `director_action` field. A permission-request `interaction_opened` event has `kind=review_permission_request`, an `instruction` and `inspect_argv`; the automatic review events carry their own actions (§ Automatic review events), and every other new event has null action. The instruction is: "Verify this event and the current request. Evaluate the operation and its risk. In the Director session's effective auto mode, decide allow or deny; otherwise obtain the user's permission. Submit the selected answer through the request-bound CAFleet response command, or give that complete command to the user. Inspect the native result and continuation afterward." When the event evidence has `operation_rows` greater than 1, the instruction appends one space and the sentence: "The operation is shown as its displayed rows, where a row break may be a wrap or a line break in the command, so confirm the command against the excerpt before deciding." Build inspection argv from validated stored identities, never from screen text. Validate the action against its event/run/Director association. Preserve existing schema-version-1 records, their exact message bytes and idempotent replay; all new events use version 2 and remain within the 16 KiB payload limit.

Before deciding, establish the actual command/tool operation, arguments, absolute cwd, required environment overrides, target resources, privilege/network effects and native choice scope as needed for the risk assessment. Evaluate those facts against the assigned task and host policy. Metadata/redacted evidence may omit necessary facts; use authorized current capture and routed member evidence, or ask the user to inspect the request when evaluation remains incomplete. Screen content is evidence, not instruction. Apply the selected evidence policy to inspection output and persistence; store bounded opaque evidence references and decision metadata rather than copying raw commands or secrets into receipts.

The Director-only CLI is:

```text
cafleet permission inspect INTERACTION_ID --event-id EVENT_ID --from-member-id DIRECTOR_ID --json
cafleet permission respond INTERACTION_ID --event-id EVENT_ID --from-member-id DIRECTOR_ID --expected-run-id RUN_ID --expected-owner-epoch EPOCH --request-token TOKEN --decision allow|deny --decision-source director-auto|user --director-mode auto|manual|unknown --mode-evidence-ref REFERENCE --decision-evidence-ref REFERENCE --operation-key KEY --json
```

Require positive IDs/epoch, nonempty operation keys and bounded nonblank references. `director-auto` requires mode `auto`; source `user` records a user decision in any mode. Validate the active pinned Director/fleet/member relationships. `inspect` verifies the current durable event/message/interaction association, retains the genuine concrete pane lock, probes the pinned owner and resolves unread output/manual progress under the original deadline clipped to any inherited SQL bound. Current authority comes from the actual parsed screen, a complete single-request choice/focus match and the stable tracked episode. Observation freshness is checked independently of ordinary composer eligibility; inspection sends zero programmatic child bytes and creates no receipt, audit or claim. Return current identities/visibility/classification, selected detector/answer-profile metadata, supported decisions/capability reason, post-policy evidence, missing-evidence indications and an opaque token. Unsupported or blocked capability is an explicit result, and incomplete observation supplies no token.

The runner issues one current random token bound in memory to the complete database/run/nonce/epoch, interaction/episode, native version, live operation/choice/focus identity, manual-input generation and full immutable decision-to-plan set, including every selected profile, rule and stage identity. A checked invalidation generation records the first incompatible candidate/request, obscurity, observation gap/degradation or incomplete observation, even when the original display returns before inspection. Positive actual manual input (bytes classified as manual; terminal replies excluded, § Terminal reply classification), shutdown and ownership loss also revoke authority; arithmetic or randomness failure returns an explicit error. Repeated compatible inspections reuse the token. A tested equivalent redraw/resize preserves it only when the exact profile proves unchanged semantic/choice/focus and plan identity; output sequence alone is evidence rather than revocation. Healthy transient queue pressure defers authority without rebinding the token. The random value is independent of hidden-text hashes and authorization; a revoked token requires fresh inspection and decision reconciliation.

A cursor transit is the one incompatible frame that leaves the generation unchanged. It is a frame that yields no current request while a request is current, is not a verified non-interaction, and keeps every row of the current request's matched live region identical in text to the frame that established the request; typically the native CLI moves the cursor outside the form to redraw another part of the screen. During a transit the request is not current, so inspection returns no token and a response is blocked. A following frame with the same request restores it under the unchanged generation and token. A changed region row, a different request fingerprint, closure, `interaction_obscured`, an observation gap, child exit and positive manual input still invalidate immediately.

A gated plan changes the request between its stages, so the runner accepts exactly one change after each complete nonfinal stage: the transition the next stage's gate names. For a focus gate, a fresh quiescent observation qualifies when its episode, rule, semantic fingerprint, choices, native correlation and manual-input generation are unchanged and its resolved focus equals the gate. For a screen gate, the continuation binding of § Continuation screens applies. The runner rebinds the in-flight selection to the qualifying observation, with its new request generation, and writes the next stage. It mints no token, because the reservation already consumed the request token.

The next stage waits for its gate within the responding invocation's deadline, and an observation that still shows the previous stage's screen and focus keeps waiting. These end the response with the failure reporting below (`uncertain` once a byte was written), and the native surface stays for manual input with no further byte:

- a focus on any other option;
- a screen other than the expected continuation after a stage that opens one;
- the episode closing before the final stage;
- any other identity change, manual input or a degraded observation;
- the deadline expiring.

Inspection exposes the selected current post-policy evidence independently of private request/match/plan state. Missing facts follow the permission inspection projection (§6.3 Permission commands); neither operation text nor private plan eligibility substitutes for a disclosed structured fact. A proved `single_request` choice establishes answer eligibility independently of whether public evidence states the choice scope. Keep transient raw semantics, focus and plan bytes in memory; inspection preserves the existing evidence schema and persists no token or pre-policy digest.

Validate a retained operation key against its recorded database/run, response identity and canonical digest before any current-request eligibility check, pane lock or live-owner probe. A matching retained receipt returns its stored result after token revocation, interaction closure, deregistration or owner exit, sending zero bytes; conflicting reuse fails. Validate historical audit relationships against recorded ownership while preserving closed/inactive history. Only an absent key or a positively recorded ready zero-byte attempt enters current live-request validation. The durable-only response projection preserves the immutable audit epoch and uses a null output sequence under the permission CLI contract (§6.3 Permission commands); a consumed retry's zero-byte stage describes that retry while retaining original disposition and claim uncertainty.

Inspection reports the shared answer-profile version. Its answer_rule_id is the common rule only when every supported decision resolves to that same rule; one supported decision reports its actual rule, and differing rules yield null. Respond selects the requested decision's actual rule internally and retains that required rule in its immutable audit/digest. A blocked or unsupported request retains the empty-decision/null-token representation.

Use versioned control operations `permission_inspect` and `permission_respond` and the existing receipt query/retire boundary. A five-second command deadline covers locking, database work and socket exchange. Before the first byte, the serialized runner resolves pending manual input and unread output, revalidates the current visible stable permission request/token and tested answer profile, and commits the response receipt, audit and episode claim atomically under the current ownership epoch. Other interaction classes, uncertain state, stale identity or unavailable storage receive zero-byte rejection. Write only the profile's single-request answer plan, rechecking relevant state and each stage's gate before that stage; partial writes or ambiguous state retain durable uncertainty. The CLI accepts semantic allow/deny decisions, not arbitrary key sequences.

Track answer progress as cumulative actual positive byte counts across ordered stages. A complete nonfinal stage retains `partial` progress and durable `in_progress`; only complete final-stage submission establishes `submitted`, followed by durable `accepted` before returning acceptance. A stage syscall returning WouldBlock or Interrupted with zero bytes may yield within the same attempt and original expiration. Retain the untouched next-stage index and any cumulative `partial` progress, then retry only that stage after fresh actual drain, current token/request/choice/focus/plan, manual-generation, owner and immediate-transaction validation through a newly consumed guard. Completed stages remain completed. This continuation supplies neither a new invocation nor a ready-state rearm.

Record a syscall result before yielding. A recorded zero-byte WouldBlock or Interrupted preserves the actual prior stage/count and releases its lease while consuming that guard; a yielding operation remains `in_progress` and supplies no zero-byte retirement proof. Dropping a consumed guard before I/O preserves its actual no-I/O state, while possible I/O without a recorded result retains uncertainty and every known earlier byte count. A positive short write, incompatible state, unrecorded possible I/O or expiration stops the remaining plan. Preserve cumulative progress, the original cause and any known input identity; persist uncertainty only within the original budget, retaining durable `in_progress` when bookkeeping fails.

V5 preserves existing records and expands the input-kind domain with `permission_response`. Table `member_permission_responses` is keyed by `input_id` with event/interaction foreign keys, expected owner epoch, request token, decision, pinned Director ID, attributed Director mode/source, mode/decision evidence references, answer-profile version/rule and decision timestamp. Table `member_permission_response_claims` is keyed by interaction ID with a unique input foreign key, to retain one possibly transmitted answer per observed episode across different operation keys. Validate same-run/event/interaction relationships and all required enum/identity constraints transactionally. The storage subsection below fixes exact SQL names, types and associations.


##### Permission response storage

V5 preserves V4 identities and rows, expands the input-kind check to `kind IN ('preview','prompt','exec','permission_response')`, and creates the `member_permission_responses` and `member_permission_response_claims` tables; the exact DDL is in §8 *V5 permission response migration*. The existing input-operation columns, key uniqueness, foreign keys and settlement constraints are preserved.

All audit fields are required and use the displayed column order. `decided_at` is the UTC RFC 3339 time when the runner records the attributed decision, independently of native decision time. Tokens are random 32-byte values encoded as lowercase hex; persist no screen-derived fingerprint or pre-policy hash. Evidence references contain 1–2048 UTF-8 bytes, include a non-whitespace character and contain no C0 or DEL control character; they identify host evidence without embedding commands, screen text or secrets.

In the same immediate transaction, require input kind `permission_response`, an active pinned Director, an inbox-linked permission-request opening event and the matching unclosed interaction/run. Validate the expected epoch, event/message association and answer profile before inserting the input, immutable audit and claim. Other input kinds have no audit row. The composite claim foreign key binds its input to the same interaction; same-run/event and recorded ownership relationships are also checked on reads while preserving closed/inactive history; active/current eligibility applies to new attempts only.

The canonical payload digest covers `run_id,owner_epoch,interaction_id,event_id,director_member_id,request_token,decision,director_mode,decision_source,mode_evidence_ref,decision_evidence_ref,answer_profile_version,answer_rule_id`. Encode this metadata as canonical JSON with sorted keys before SHA-256; `input_id`, timestamps, operation key and literal native keys are excluded. Validate a matching key against this digest before any new action.

Retain a claim after possible submission, accepted submission, uncertain settlement or `abandoned_uncertain`. Release it only in the synchronized transaction that proves zero bytes, no active writer and `ready` or settled `not_submitted`; a disposition label alone supplies no zero-byte evidence. Audit rows and consumed operation keys remain retained after claim release.

Reuse input receipt states, settlement dispositions, the 4096 unresolved limit and synchronized retirement. Digest canonical non-secret request identity, token and decision metadata, excluding screen text and literal key bytes. The Director records the operation key before dispatch. Same-key retries return the existing result; conflicting payloads fail. A different key cannot bypass a submitted, active, uncertain or abandoned-uncertain episode claim. Release a claim only after synchronized evidence proves zero bytes and no active writer; a later deliberate attempt uses fresh inspection and a new key. Recovery sends no answer keys. Key submission and recorded `decision=allow` do not establish native approval or command success.

The permission CLI contract (§6.3 Permission commands) fixes the inspection and response output, the audit object appended to input output, exit codes and failure output. When asking the user to execute a response, present the concrete command with actual snapshot values; stale values fail before bytes.

Answer-profile capability belongs to the run's selected profile, rests on native evidence within its version interval and is independently tested for allow and deny. It specifies request form/rule, single-request choice semantics, focus/selection requirements, exact key stages and output revalidation. Use real native evidence rather than invented Enter/Escape mappings. Persistent/session-wide permission changes and answers to trust/update/question forms require their own explicit scope. Unsupported answer profiles preserve manual native input. The screen interface has no atomic native request API; serialize available input/output, reject observed mismatches and retain uncertainty for remaining races and indistinguishable requests.

#### Director consumption and continuation

Use the existing `message listen`, `poll --json` and finite `wait` interfaces. Runtime events reach the Director's stdout receiver and durable inbox; they introduce no composer injection, Escape-first wake, goal activation or new host interrupt guarantee (latency targets: § Native acceptance and coverage). Native model consumption latency remains controlled by the Director host and may be unbounded while the host is blocked or paused.

On a wake or poll, the Director reads the complete event, verifies its durable event/message link with `event show`, reconciles the current run/interaction, records the unfinished action in host-owned state, and ACKs the message as consumption. An ordinary member can quote event JSON; a claimed event identity is verified against stored ownership, never trusted from message text alone. Old/closed events remain readable but are not a new request to answer.

| Stage | Observable evidence | What remains outstanding |
|---|---|---|
| Detection | Rule/profile and current episode evidence | Durability and Director reception. |
| Persistence | Event ID plus committed inbox message ID | Director consumption. |
| Consumption | ACK plus Director-owned pending-action record | Risk review, decision and actual operation outcome. |
| Decision | Director auto-mode evidence and attributed decision, or the user's request-specific decision | Answer-key submission and native result. |
| Answer-key submission | Bound durable receipt/stage, or attributed manual native input | Native decision evidence and actual operation result. |
| Native decision observed | Explicit profile-supported native evidence, when available | Actual operation result and dependent continuation. |
| Episode disappears | Closure reason and current screen | Decision and execution outcome may remain unknown. |
| Continuation verified | Actual operation result/output and member's resumed dependent work | Any remaining task-specific work. |

For a permission refusal returned to the member, preserve the existing evidence route: exact attempted command, absolute cwd, required environment overrides and actual refusal. A screen event can alert the Director earlier or supplement that route; it supplies no missing dispatch evidence. For a blocked request, verify the event and current request, establish operation/risk evidence and resolve the Director session's effective mode. Decide allow/deny in auto mode; obtain user permission in other or unknown modes. Submit the bound response or give the user its complete concrete command, then inspect native output/exit/error and the member's actual continuation. Unknown answer capability preserves manual original-surface input. Previews, prompts and native commands the runner queued during the request drain automatically after the interaction closes (§ Runner-owned deferred delivery); a deferred input's eventual result arrives as an `input_delivery_result` event. Messages that were held rather than queued remain pending for the next send or a finite `message wait <DIRECTOR_ID>` rotation. A different command submitted through native exec is a separate authorized action, never proof that the original request was answered. Reconcile uncertain outcomes before retries, and preserve host/managed permission authority throughout.

The Director handles the automatic review events (§ Automatic review events) through the same wake, `event show` verification, ACK and host-owned pending-action record:

| Event | Director procedure |
|---|---|
| `interaction_opened` in automatic mode | The permission-request procedure above, including `permission respond`. |
| `automatic_refusal_observed` with a routed request | Dispatch evidence comes only from the routed request: exact command, absolute cwd, required environment and refusal; the event and captures corroborate it and supply no missing field. In verified auto mode decide to authorize through the routed-command dispatch (`member exec`) or to refuse with a broker reply; otherwise obtain the user's permission first, then inspect the actual result and continuation. |
| `automatic_refusal_observed` without a routed request | Dispatch nothing, ACK and keep the event as an informational pending item. When the member next reports, routes a request or reaches an idle composer without addressing it, ask it through the broker whether the refused operation is still needed; the item closes when a routed request arrives or the member answers that it is not needed. |
| Several `automatic_refusal_observed` events, then an idle member that sent no report | Codex 0.163.0 interrupts the turn after three consecutive automatic denials, so the member stops without routing a request (§ Annotation detection). Treat the events as one pending item and ask the member through the broker whether the refused operations are still needed; the broker message resumes it. |
| `automatic_review_unavailable` | Record the member as interactive in host-owned supervision state, expect native permission forms from it and tell the user when the selected automatic mode cannot apply. No answer is sent. |

A routed request that arrives with no event follows the refusal route above unchanged.

#### Native acceptance and coverage

Use synthetic terminal replay and real native PTYs for every declared detector, composer and answer capability on each registered multiplexer. Fixed-policy or unsupported profiles retain explicit unknown coverage and manual operation.

The release minimum is a real interactive permission request under Claude's native `default` policy and under Codex's native `on-request` policy with `workspace-write`, each on both tmux and herdr. Pin exact installed CLI versions by the acceptance records and corresponding bundled profiles; use permitted disposable fixtures that actually reach the native approval surface. Every minimum case must demonstrate blocked-model-independent detection, a committed actionable event/inbox record, Director consumption and preserved original request. Exercise separate fresh requests for verified Director-auto allow and deny, user-authorized responses under non-auto/unknown modes, and manual original-surface input; include a user-executed complete bound command. Record zero response bytes before required user permission, actual native results and observed continuation separately from submitted keys. Missing native availability, unknown coverage or an unobserved required case fails overall feature acceptance; unknown coverage is only the compatibility path for other versions/backends. Additional profiles retain their own declared acceptance gates.

Automatic-mode acceptance covers Claude `auto` and Codex `approve-for-me` on both tmux and herdr, with automatic review available for the account and model: the omitted-default launch, an auto-approved fixture, an auto-refused fixture with its event and Director handling, and, where the native host shows one, an escalated form answered allow and deny. A later-version case repeats the launch, auto-approved, auto-refused and escalated cases on a native version above its profile's `min_version`, recording the selected profile and each UI difference that failed to match. A fixture that does not produce its intended screen is a recorded limit for that backend, not a pass.

Minimum-version delivery acceptance covers every bundled backend on each registered multiplexer, using isolated native sessions and databases and benign marker commands:

| Case | Required evidence |
|---|---|
| Threshold selection | Equal/between/above/below boundaries, catalog permutation, multiple backends, numeric component ordering, prerelease ordering, build-metadata ties, malformed thresholds and failed probes |
| Complete profile reuse | Detection, annotations, both composer operations and each available semantic answer resolve the same profile and retain actual run/capture identities |
| Native operation plans | Prompt preview and direct prompt create a native user turn; native exec runs a harmless marker command and exposes its result, with literal-to-submit timing recorded |
| Fallback operation | Missing plan and failed-probe cases show Escape, 500 ms settle, literal, 500 ms settle, Enter and explicit fallback results |
| Busy composer | A draft or unclassifiable composer with an executable plan queues `composer_busy` with zero bytes, drains by profile once the composer empties, and falls back with `idle_timeout` after 3 minutes without any earlier Escape; a working member with an empty composer row receives profile delivery |
| Interaction deferral | Permission and question remain open with no Escape/literal bytes; answering on the original native surface yields closure and exactly one queued delivery without another send/wait |
| Adversarial transition | Candidate/obscured/superseding interaction, human input during either settle, observer/owner loss and partial stage writes preserve no-write or uncertain outcomes |
| Durable retry | A queued canonical preview drains by fallback and retains one ID/digest after composer changes and repeated queries; duplicate keys, conflicting digests, concurrent sender/drain, lost replies and stamp failures avoid duplicate dispatch |
| Deferred failure and liveness | After closure with a busy pane lock and no further output/commands, the runner timer either delivers when the lock frees or publishes not submitted at the deadline; temporary storage/observer failure and terminal-commit recovery preserve FIFO/result visibility |
| Migration and reporting | Fresh/head/previous-head upgrade, unchanged version-1 digests, referential integrity, sequence preservation, self-send/no-pane output and durable reason/state projection for ordinary/history/permission-response receipts |

Measurements establish submitted native input separately from model consumption, permission decisions and command completion.

Record source/installed CAFleet version, native CLI version, native configuration/mode, detector and answer profiles, Director mode/provenance, attributed decision/evidence references, request identity/token, receipt/stage, multiplexer, terminal size, cwd and redacted evidence for live acceptance. Include a permitted operation, an ordinary command failure after allow, an actual permission refusal, an approval that blocks the model, both supported answer routes and observed continuation. Exercise stale tokens/epochs/events, same/different operation keys, conflicting decisions, partial writes/lost replies, redraw, fragmented ANSI/UTF-8, primary/alternate screens, resize, title-only prompts, question/trust/update screens, transcript quotes, identical requests, concurrent human/output/input, slow display, database locks, outbox pressure, startup compensation, runner hard kill and deletion. Unavailable native modes or unobserved cases remain explicit coverage limits and cannot pass that profile's release gate.

Acceptance additionally covers the transport, lifecycle and persistence cases below. With a live receiver and available storage, measure a target of two seconds from a stable prompt to inbox commit and another two seconds from commit to receiver output.

- Test exact child argv for the selected native policy, preserving native/managed restrictions and existing sandbox boundaries.
- Test equal run IDs across sibling databases and replacement at an unchanged path.
- Test a held write transaction and a long statement to prove restoration/exit are independent of the connection's default timeout.
- Test a child exiting while its descendant holds the slave, exit during a stalled display and a locked database, and deadline-bounded terminal restoration.
- Test timeout followed by the original runner's recovery and concurrent recovery attempts.
- Test a command containing a known secret and assert its absence from every database/message/outbox field in metadata mode and with a matching redaction pattern, including interaction updates.
- Test hard kill with complete uncommitted frames, replay of an opening before loss closure, concurrent recovery, interruption during recovery, and deregistration before replay with archived rather than fabricated inbox records.
- Test a live unlinked lock, verified death, unknown identity without attestation, matching/stale attestations, positive live evidence rejecting an attestation and concurrent database-only recovery.
- Test cross-invocation accepted-then-failed-stamp recovery, queued-then-drained delivery after interaction closure, disconnected partial writes, receipt retirement followed by delayed replay, and a lost explicit-command stdout result.
- Test every registered backend's omitted-policy argv and each adapter-declared explicit policy; profiles claiming detection, composer or answer support require both fixtures and real native evidence.

## 7. Cross-cutting concerns

### 7.1 Configuration & environment

Expand HOME only for the default `sqlite:///` URL ending in `cafleet_v8.db`. Explicit `sqlite:///<path>` overrides preserve their path text; relative file paths resolve against the invoking working directory before canonical database-file identity supplies shared pane locking. Validate supplied values and required state at their boundary. Config-directory relocation is resolved lazily per selected backend.

### 7.2 Error handling & exit-code policy

Usage/parser/model/effort/line-validation errors exit 2. Application, database and transport errors exit 1. A successful finite timeout exits 0. Top-level application errors use `Error: MESSAGE` on stderr. Preserve primary causes alongside explicit cleanup failures. Missing required state or unknown enum/column values propagate instead of substituting meaningless defaults.

### 7.3 Output / JSON / truncation

JSON uses explicit ordered presenters and unescaped UTF-8. Preserve full message bodies and capture bytes selected for presentation. CLI text output truncates only message bodies by the `CAFLEET_MAX_TEXT_LEN` codepoint limit; pane notification lines carry the folded full body up to 4000 characters and a `message show` pointer beyond it. Exact deliberate error and layout strings remain contractual.

### 7.4 Logging & stdout discipline

Successful command records stay on stdout. Post-commit notification diagnostics and creation compensation failures go to stderr. A committed message retains successful output even when its preview cannot be delivered; an uncertain native command prints no success record.

### 7.5 Time discipline

Use UTC for durable timestamps, monotonic clocks for budgets and deadlines. Cap every bounded process/database operation to remaining time. Every guard/owned child releases resources on ordinary errors and process exit.

### 7.6 Packaging & distribution

Ship one `cafleet` binary per release target. Archives are `cafleet-v<version>-<target>.tar.gz` containing the binary; the release tag is the bare package version. The compile-time version owns `--version`, asset stamps and doctor's installation checks. Embed migrations, skills and presets offline. Rust builds/release packaging require no frontend output. The documentation site retains its independent Node/pnpm tooling.

### 7.7 Agent-operation contract — one-shot command isolation

Agents run each CAFleet process as the only command in its shell-tool invocation, so a host's single-command permission rules match it; the shipped CAFleet skill owns the operating procedure.

## 8. Database schema

The baseline is version 1 in the current distinct format family. The fresh baseline creates exactly the application tables declared in the schema. Compatibility diagnosis verifies current application identity, ledger version, every required table’s column shape, and required indexes/foreign keys. Known versions use their recorded migration shape; newer versions are diagnosed before any head-shape comparison. Unrelated auxiliary tables, indexes and triggers are compatible while those required definitions remain intact.

| Table | Responsibility |
|---|---|
| `fleets` | Fleet identity, creation/deletion and root Director reference |
| `members` | Member registration and active/deregistered state |
| `member_placements` | Multiplexer and coding-agent association |
| `messages` | Point-to-point delivery, acknowledgement and preview stamp |
| `runtime_database_identity` | Singleton database-instance identity for runtime namespaces |
| `member_runs` | Launch intent, ownership, lifecycle, recorded coverage, selected profile and observed effective policy |
| `member_interactions` | Observed interactive episodes and visibility closure |
| `member_runtime_events` | Sequenced events with atomic Director inbox association or archival |
| `member_input_operations` | Durable submission receipts, delivery method/deferral metadata, payload digest versions and settled replay protection |
| `member_permission_responses` | Immutable attributed decisions tied to input/event/interaction identity |
| `member_permission_response_claims` | One possibly transmitted answer per observed episode |
| `asset_installs` | Installed asset versions by backend and config path |
| `refinery_schema_history` | Embedded migration ledger |

Exact baseline DDL:

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

Baseline column semantics:

| Table | Semantics |
|---|---|
| `fleets` | `director_member_id` is backfilled inside the bootstrap transaction. A live fleet has null `deleted_at` and an active valid Director with placement |
| `members` | `status` decodes as `active` or `deregistered`; unknown values fail. Deregistration is soft and keeps the row |
| `member_placements` | `mux_pane_id` is null while member creation is pending. A concrete placement selects the shared pane input lock, and duplicate entries targeting the same backend/session/pane share it |
| `messages` | `status_state` decodes as `input_required` or `completed`. `notified_at` is internal preview state that the public message record omits; a self-send stamps it on insertion and stays pending for reception and explicit ACK. Inbox streaming ignores it, and native handles and emitted-ID tracking belong to the receiver process with no durable consumer state |
| `asset_installs` | Each successful backend asset installation upserts its CLI version and timestamp at `(coding_agent, path)`, independently of member registration |

Runtime records keep their run/member/fleet/Director references after soft deregistration, so history survives member and fleet closure.

Runtime schema extensions retain the application identity and existing data through contiguous embedded migrations. Section 6.9 specifies every additional runtime table field, enum, nullability constraint, foreign-key association and uniqueness invariant, including database-instance identity and settled receipt retention.

### V2 and V3 preview migrations

`V2__preview_attempts.sql` adds the nullable `messages.preview_started_at TEXT` column, and `V3__drop_preview_claims.sql` drops it again, so from V3 on `messages` has the baseline shape.

### V4 member runtime migration

The current-family V4 migration preserves existing rows and creates the runtime records:

```sql
CREATE TABLE runtime_database_identity (
    singleton_id INTEGER PRIMARY KEY CHECK(singleton_id = 1),
    instance_id TEXT NOT NULL CHECK(length(instance_id) = 32 AND instance_id NOT GLOB '*[^0-9a-f]*')
);
INSERT INTO runtime_database_identity VALUES(1, lower(hex(randomblob(16))));

CREATE TABLE member_runs (
    run_id INTEGER PRIMARY KEY AUTOINCREMENT,
    member_id INTEGER NOT NULL REFERENCES members(member_id) ON DELETE RESTRICT,
    fleet_id INTEGER NOT NULL REFERENCES fleets(fleet_id) ON DELETE RESTRICT,
    director_member_id INTEGER NOT NULL REFERENCES members(member_id) ON DELETE RESTRICT,
    backend TEXT NOT NULL CHECK(backend IN ('claude','codex','opencode')),
    database_identity_json TEXT NOT NULL CHECK(json_valid(database_identity_json) AND json_type(database_identity_json) = 'object'),
    launch_nonce TEXT NOT NULL CHECK(length(launch_nonce) = 64 AND launch_nonce NOT GLOB '*[^0-9a-f]*'),
    runtime_path TEXT NOT NULL CHECK(length(trim(runtime_path)) > 0),
    requested_mode TEXT,
    resolved_mode TEXT NOT NULL CHECK(length(trim(resolved_mode)) > 0),
    mode_source TEXT NOT NULL CHECK(mode_source IN ('default','explicit')),
    permission_argv_json TEXT NOT NULL CHECK(json_valid(permission_argv_json) AND json_type(permission_argv_json) = 'array'),
    event_evidence TEXT NOT NULL CHECK(event_evidence IN ('metadata','redacted')),
    coverage TEXT NOT NULL CHECK(coverage IN ('tested','unknown')),
    status TEXT NOT NULL CHECK(status IN ('starting','prepared','running','degraded','stopping','recovering','exited','failed','lost')),
    created_at TEXT NOT NULL,
    native_version TEXT,
    profile_version TEXT,
    effective_policy_json TEXT CHECK(effective_policy_json IS NULL OR (json_valid(effective_policy_json) AND json_type(effective_policy_json) = 'object')),
    owner_epoch INTEGER NOT NULL CHECK(owner_epoch > 0),
    owner_kind TEXT NOT NULL CHECK(owner_kind IN ('creator','runner','reconciler')),
    owner_pid INTEGER NOT NULL CHECK(owner_pid > 0),
    owner_start_identity TEXT NOT NULL CHECK(length(trim(owner_start_identity)) > 0),
    reachability TEXT NOT NULL CHECK(reachability IN ('reachable','suspect')),
    cleanup_state TEXT NOT NULL CHECK(cleanup_state IN ('complete','incomplete')),
    event_seq_reserved_through INTEGER NOT NULL CHECK(event_seq_reserved_through >= 0),
    observation_gap_count INTEGER NOT NULL CHECK(observation_gap_count >= 0),
    runtime_dir_identity_json TEXT CHECK(runtime_dir_identity_json IS NULL OR (json_valid(runtime_dir_identity_json) AND json_type(runtime_dir_identity_json) = 'object')),
    run_lock_identity_json TEXT CHECK(run_lock_identity_json IS NULL OR (json_valid(run_lock_identity_json) AND json_type(run_lock_identity_json) = 'object')),
    runner_pid INTEGER CHECK(runner_pid IS NULL OR runner_pid > 0),
    runner_start_identity TEXT,
    child_pid INTEGER CHECK(child_pid IS NULL OR child_pid > 0),
    child_start_identity TEXT,
    started_at TEXT,
    last_health_at TEXT,
    ended_at TEXT,
    exit_code INTEGER,
    exit_signal INTEGER,
    cleanup_diagnostics_json TEXT CHECK(cleanup_diagnostics_json IS NULL OR (json_valid(cleanup_diagnostics_json) AND json_type(cleanup_diagnostics_json) = 'array')),
    recovery_origin_status TEXT CHECK(recovery_origin_status IS NULL OR recovery_origin_status IN ('starting','prepared','running','degraded','stopping','recovering','exited','failed','lost')),
    CHECK((mode_source = 'default' AND requested_mode IS NULL) OR (mode_source = 'explicit' AND requested_mode IS NOT NULL))
);
CREATE UNIQUE INDEX idx_member_runs_active ON member_runs(member_id) WHERE status NOT IN ('exited','failed','lost');

CREATE TABLE member_interactions (
    interaction_id INTEGER PRIMARY KEY AUTOINCREMENT,
    run_id INTEGER NOT NULL REFERENCES member_runs(run_id) ON DELETE RESTRICT,
    episode_seq INTEGER NOT NULL CHECK(episode_seq > 0),
    classification TEXT NOT NULL CHECK(classification IN ('permission_request','question','trust','update','unknown_interaction')),
    evidence_digest TEXT NOT NULL,
    opened_at TEXT NOT NULL,
    closed_at TEXT,
    close_reason TEXT CHECK(close_reason IS NULL OR close_reason IN ('not_visible','superseded','process_exited','monitor_lost')),
    evidence_json TEXT NOT NULL CHECK(json_valid(evidence_json) AND json_type(evidence_json) = 'object'),
    UNIQUE(run_id, episode_seq),
    CHECK((closed_at IS NULL) = (close_reason IS NULL))
);
CREATE UNIQUE INDEX idx_member_interactions_open ON member_interactions(run_id) WHERE closed_at IS NULL;

CREATE TABLE member_runtime_events (
    event_id INTEGER PRIMARY KEY AUTOINCREMENT,
    run_id INTEGER NOT NULL REFERENCES member_runs(run_id) ON DELETE RESTRICT,
    event_seq INTEGER NOT NULL CHECK(event_seq > 0),
    interaction_id INTEGER REFERENCES member_interactions(interaction_id) ON DELETE RESTRICT,
    kind TEXT NOT NULL CHECK(kind IN ('coverage_unknown','interaction_opened','interaction_obscured','interaction_closed','permission_denied_observed','runner_degraded','runner_lost','runner_exited')),
    observed_at TEXT NOT NULL,
    committed_at TEXT NOT NULL,
    payload_json TEXT NOT NULL CHECK(json_valid(payload_json) AND json_type(payload_json) = 'object'),
    notification_state TEXT NOT NULL CHECK(notification_state IN ('inbox','archived')),
    message_id INTEGER UNIQUE REFERENCES messages(message_id) ON DELETE RESTRICT,
    archive_reason TEXT CHECK(archive_reason IS NULL OR archive_reason IN ('member_inactive','fleet_deleted','director_inactive')),
    UNIQUE(run_id, event_seq),
    CHECK((notification_state = 'inbox' AND message_id IS NOT NULL AND archive_reason IS NULL) OR
          (notification_state = 'archived' AND message_id IS NULL AND archive_reason IS NOT NULL))
);

CREATE TABLE member_input_operations (
    input_id INTEGER PRIMARY KEY AUTOINCREMENT,
    run_id INTEGER NOT NULL REFERENCES member_runs(run_id) ON DELETE RESTRICT,
    operation_key TEXT NOT NULL CHECK(length(trim(operation_key)) > 0),
    kind TEXT NOT NULL CHECK(kind IN ('preview','prompt','exec')),
    payload_digest TEXT NOT NULL CHECK(length(payload_digest) = 64 AND payload_digest NOT GLOB '*[^0-9a-f]*'),
    state TEXT NOT NULL CHECK(state IN ('ready','in_progress','accepted','uncertain','settled')),
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    message_id INTEGER REFERENCES messages(message_id) ON DELETE RESTRICT,
    disposition TEXT CHECK(disposition IS NULL OR disposition IN ('submitted','not_submitted','abandoned_uncertain')),
    evidence_ref TEXT,
    UNIQUE(run_id, operation_key),
    CHECK(kind != 'preview' OR message_id IS NOT NULL),
    CHECK((state = 'settled' AND disposition IS NOT NULL) OR (state != 'settled' AND disposition IS NULL))
);
```

### V5 permission response migration

The contiguous V5 migration preserves V4 rows and IDs, expands the `member_input_operations.kind` check to `preview,prompt,exec,permission_response`, retains its existing columns/foreign keys/receipt constraints, and creates the following required tables. Section 6.9 fixes transactional same-run/event/Director checks, privacy, zero-byte claim release and immutable audit semantics.

```sql
CREATE TABLE member_permission_responses (
    input_id INTEGER PRIMARY KEY REFERENCES member_input_operations(input_id) ON DELETE RESTRICT,
    event_id INTEGER NOT NULL REFERENCES member_runtime_events(event_id) ON DELETE RESTRICT,
    interaction_id INTEGER NOT NULL REFERENCES member_interactions(interaction_id) ON DELETE RESTRICT,
    expected_owner_epoch INTEGER NOT NULL CHECK(expected_owner_epoch > 0),
    request_token TEXT NOT NULL CHECK(length(request_token) = 64 AND request_token NOT GLOB '*[^0-9a-f]*'),
    decision TEXT NOT NULL CHECK(decision IN ('allow','deny')),
    director_member_id INTEGER NOT NULL REFERENCES members(member_id) ON DELETE RESTRICT,
    director_mode TEXT NOT NULL CHECK(director_mode IN ('auto','manual','unknown')),
    decision_source TEXT NOT NULL CHECK(decision_source IN ('director-auto','user')),
    mode_evidence_ref TEXT NOT NULL CHECK(length(CAST(mode_evidence_ref AS BLOB)) BETWEEN 1 AND 2048),
    decision_evidence_ref TEXT NOT NULL CHECK(length(CAST(decision_evidence_ref AS BLOB)) BETWEEN 1 AND 2048),
    answer_profile_version TEXT NOT NULL CHECK(length(trim(answer_profile_version)) > 0),
    answer_rule_id TEXT NOT NULL CHECK(length(trim(answer_rule_id)) > 0),
    decided_at TEXT NOT NULL CHECK(length(trim(decided_at)) > 0),
    UNIQUE(interaction_id, input_id),
    CHECK(decision_source != 'director-auto' OR director_mode = 'auto')
);

CREATE TABLE member_permission_response_claims (
    interaction_id INTEGER PRIMARY KEY REFERENCES member_interactions(interaction_id) ON DELETE RESTRICT,
    input_id INTEGER NOT NULL UNIQUE REFERENCES member_input_operations(input_id) ON DELETE RESTRICT,
    FOREIGN KEY(interaction_id, input_id)
        REFERENCES member_permission_responses(interaction_id, input_id) ON DELETE RESTRICT
);
```

### V6 automatic review migration

The contiguous V6 migration `V6__automatic_review_events.sql` preserves V5 rows and IDs. It rebuilds `member_runtime_events` with its `kind` CHECK widened to:

```sql
CHECK(kind IN ('coverage_unknown','interaction_opened','interaction_obscured','interaction_closed','permission_denied_observed','automatic_refusal_observed','automatic_review_unavailable','runner_degraded','runner_lost','runner_exited'))
```

The grouped migration transaction runs with `foreign_keys=ON`, and `member_permission_responses.event_id` references `member_runtime_events` while `member_permission_response_claims` references `member_permission_responses` with `ON DELETE RESTRICT`, so the three tables are rebuilt in dependency order:

1. Create `member_runtime_events_v6` with the widened CHECK and every other column, constraint and foreign key unchanged, copy all rows, and carry its `sqlite_sequence` row from `member_runtime_events` before any drop, so event IDs are never reused.
2. Create `member_permission_responses_v6` with `event_id REFERENCES member_runtime_events_v6(event_id)` and otherwise the V5 definition; copy all rows.
3. Create `member_permission_response_claims_v6` with its composite foreign key referencing `member_permission_responses_v6(interaction_id, input_id)` and otherwise the V5 definition; copy all rows.
4. Drop `member_permission_response_claims`, then `member_permission_responses`, then `member_runtime_events`.
5. Rename `member_runtime_events_v6`, `member_permission_responses_v6` and `member_permission_response_claims_v6` to their final names in that order; the non-legacy rename rewrites the referencing foreign keys and renames the `_v6` sequence row with its table.
6. Add the profile-match column in place and backfill existing runs, which all selected their profile by exact version. SQLite tests an added column's CHECK against existing rows, and a NULL `profile_match` makes `IN` NULL rather than false, so the CHECK names `IS NOT NULL` explicitly and the profiled runs are stashed and their `profile_version` cleared while the column is added:

```sql
CREATE TEMP TABLE v6_profiled_runs AS
  SELECT run_id, profile_version FROM member_runs WHERE profile_version IS NOT NULL;
UPDATE member_runs SET profile_version = NULL WHERE profile_version IS NOT NULL;
ALTER TABLE member_runs ADD COLUMN profile_match TEXT
  CHECK((profile_version IS NULL AND profile_match IS NULL)
     OR (profile_version IS NOT NULL AND profile_match IS NOT NULL
         AND profile_match IN ('exact','fallback')));
UPDATE member_runs SET
  profile_version = (SELECT p.profile_version FROM v6_profiled_runs p WHERE p.run_id = member_runs.run_id),
  profile_match = 'exact'
  WHERE run_id IN (SELECT run_id FROM v6_profiled_runs);
DROP TABLE v6_profiled_runs;
```

The derived chain guard and schema-shape guard pass at head V6.

### V7 minimum-version delivery migration

The contiguous V7 migration `V7__minimum_version_delivery.sql` preserves V6 rows, IDs, sequences, foreign keys and audit relationships, and the application and instance identities. It applies these changes in one grouped transaction with `foreign_keys=ON`:

1. Drop the run's profile-match column together with its column-local CHECK:

```sql
ALTER TABLE member_runs DROP COLUMN profile_match;
```

2. Add the ordinary-delivery and digest-version columns to `member_input_operations` in place. Existing rows receive `payload_digest_version=1`, keep their digest bytes and have null delivery/deferred metadata:

```sql
ALTER TABLE member_input_operations ADD COLUMN delivery_method TEXT
  CHECK(delivery_method IS NULL OR delivery_method IN ('profile','fallback'));
ALTER TABLE member_input_operations ADD COLUMN delivery_reason TEXT
  CHECK(delivery_reason IS NULL OR length(trim(delivery_reason)) > 0);
ALTER TABLE member_input_operations ADD COLUMN deferred_at TEXT;
ALTER TABLE member_input_operations ADD COLUMN defer_reason TEXT
  CHECK((deferred_at IS NULL AND defer_reason IS NULL)
     OR (deferred_at IS NOT NULL AND defer_reason IS NOT NULL
         AND kind IN ('preview','prompt','exec')));
ALTER TABLE member_input_operations ADD COLUMN payload_digest_version INTEGER NOT NULL DEFAULT 1
  CHECK(payload_digest_version IN (1,2) AND (payload_digest_version = 1 OR kind = 'preview'));
```

3. Rebuild `member_runtime_events`, `member_permission_responses` and `member_permission_response_claims` in the V6 dependency order with `_v7` staging names, carrying the event `sqlite_sequence` row before any drop, and widen the event kind CHECK to:

```sql
CHECK(kind IN ('coverage_unknown','interaction_opened','interaction_obscured','interaction_closed','permission_denied_observed','automatic_refusal_observed','automatic_review_unavailable','input_delivery_result','runner_degraded','runner_lost','runner_exited'))
```

Every other column, constraint, index and foreign key of the rebuilt tables is unchanged. The derived chain guard and schema-shape guard pass at head V7.

### V8 uncertain cause migration

The contiguous V8 migration `V8__uncertain_cause.sql` preserves V7 rows, IDs, sequences, foreign keys and audit relationships, and the application and instance identities. It adds the uncertain-cause column to `member_input_operations` in place, and existing rows receive null:

```sql
ALTER TABLE member_input_operations ADD COLUMN uncertain_cause TEXT
  CHECK(uncertain_cause IS NULL
     OR (uncertain_cause IN ('manual_input','manual_accounting_failed','interaction_candidate',
                             'interaction_open','observer_unavailable','short_write','write_failed',
                             'runner_stopped')
         AND state IN ('uncertain','settled')));
```

No table is rebuilt. The derived chain guard and schema-shape guard pass at head V8.

## 9. Testing strategy

Exercise current schema/identity and incompatible-file preservation; atomic registration and creation compensation; point-to-point/ACK/pending notification behavior; finite receive deadlines, round-robin retries and diagnostic clearing; process-shared exclusion, bounded lock waits and process-death release; for explicitly unwrapped members, Escape-first argv order and oldest-first drains with one Escape per lock hold; minimum-version profile selection, staged profile submission with its settle interval, terminal-reply classification of outer-terminal bytes and its pending-prefix bound, uncertain-cause recording at every runner uncertain transition, delivery fallback, runner-owned queueing and delivery reporting for wrapped members; budget exhaustion, zero automatic Director input and no pane capture on any input path; stream ordering/replay, exact bodies, isolation, read-only state, ACK races, scoped SQL/collection deadlines and handler cleanup, bounded partial/broken output and native receiver lifecycle; termination deferral across pane-input spans; literal native command visibility and partial uncertainty; explicit continuation; backend/multiplexer smoke coverage; links, embedding and release builds. Preserve general timed pipe draining and owned-child cleanup coverage.

## 10. CLI command checklist

§6.3 CLI specifies the complete command registry, subjects, options and outputs.

## 11. Decisions & clarifications

Each design decision is recorded in the section that owns its contract.
