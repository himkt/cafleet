# CAFleet — Reimplementation Specification

This is the self-contained interface and observable-behavior specification for the `cafleet` CLI, broker, independent coding-agent sessions and embedded assets. Exact authored errors, JSON fields/order and command layouts govern; incidental host-language quoting and operating-system error suffixes preserve intent. Complete module contracts appear in [§6](#6-module-specifications).

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

The fresh baseline creates exactly the application tables declared in the schema. Compatibility diagnosis verifies current application identity, ledger version, every required table’s column shape, and required indexes/foreign keys. Known versions use their recorded migration shape; newer versions are diagnosed before any head-shape comparison. Unrelated auxiliary tables, indexes and triggers are compatible while those required definitions remain intact.

Connections use SQLite foreign-key enforcement and a normal five-second busy timeout. A bounded operation caps that wait to its remaining budget. Resolve a relative selected SQLite file path against the invoking working directory and canonicalize the database file before lock-key directory construction.

Validate database identity, ledger version and required table/column shapes for the recorded supported version before command writes and before either setup half installs assets. An invocation's validated connection serves its internal send, ACK and notification stamp operations; standalone public broker write functions retain their own format guard. Schedule schema upgrades between active command invocations. Empty zero-identity files initialize with the baseline atomically. Current-family known older versions migrate; an ahead ledger version rejects both setup write paths before database mutation, asset installation or asset-install recording. Independent half-error reporting applies only after the compatibility gate. Other populated/identified or malformed files fail without content writes:

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

Pending preview selection chooses the oldest eligible row by `(created_at, message_id)` for an active ordinary recipient with a concrete pane; root Directors and self-sends are ineligible. Revalidate ordinary-member kind, fleet, placement, message status and null notification stamp while holding pane input ownership. Stamp success conditionally only after complete submission; delivery leaves `status_state` unchanged until explicit ACK. Listening selects every pending record independently of notification fields, including stamped pending self-sends, without changing broker state.

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
| `member create --permission-mode MODE` | Each backend's accepted modes and its omitted default, in backend registry order, as `BACKEND: MODE, MODE (default MODE)` joined by `; ` | The backend [policy catalog](docs/docs/spec/member-runtime.md#launch-policy-and-capability-registry) |
| `permission respond --decision`, `--decision-source`, `--director-mode` | Their possible values | The stored enum values of the [permission response storage](docs/docs/spec/member-runtime.md#permission-response-storage) |
| `input reconcile --disposition`, `member create --event-evidence` | Their possible values | Their command enums |

An invalid value for an option with possible values exits 2 with a usage error that lists those values. `member create` resolves its backend at run time, so a `--permission-mode` value outside that backend's catalog keeps its run-time usage error `--permission-mode for BACKEND must be one of CHOICES (got 'VALUE')`, also with exit 2. For opencode, help reads `opencode: cafleet, cafleet-ask (default cafleet)` and the error reads `--permission-mode for opencode must be one of cafleet, cafleet-ask (got 'VALUE')`.

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
| `member prompt` | `Sent prompt TEXT_REPR to member NAME (PANE).` plus wrapped input line | `{member_id, pane_id, text, input_id}` |
| `member exec` | `Dispatched native command to member ID (PANE). Inspect the pane for its result.` plus wrapped input line | `{member_id, pane_id, coding_agent, command, dispatched, input_id}` |
| `member press` | `Acknowledged keys KEYS_JSON for member ID (PANE).` | Ordered `member_id, pane_id, keys, status` |
| `member capture` | Exact selected content | `{member_id, pane_id, lines, content, captured_at, content_sha256}` |
| `fleet capture` | One annotated section per entry | Ordered capture-entry array |
| `doctor` | Complete diagnosis and issue footer | Diagnosis object |

##### Member list output {#member-list-output}

Roster records include `member_id`, `name`, `kind`, `placement`, `last_sent`, `last_recv`, `last_ack` and `idle`. `kind` is `director` when the fleet reference matches, otherwise `member`. Missing activity is null; placement may be null, or its pane ID pending. Text uses `-` for absent values and `(pending)` for a pending pane.

#### JSON output (`--json`) {#json-output}

Place `--json` among the subcommand options, before a `--` separator that introduces literal key arguments. It emits compact single-line JSON with UTF-8 and complete untruncated message text. Application errors remain stderr text, with the [structured permission-response exception](#permission-commands); parse errors exit 2. Commands marked no reject the flag, including the root group.

#### Positional subject ids {#positional-subject-ids}

Place the command’s subject ID immediately after its subcommand; event, input and permission subjects use their positive durable event, input and interaction IDs. The subject row supplies related fleet/recipient state. IDs are complete DB-assigned integers, without prefix resolution or environment defaults. A missing or malformed subject uses the parser's native error, exit 2.

Spawned members read literal `FLEET ID:`, `YOUR MEMBER ID:`, `DIRECTOR MEMBER ID:` and `CODING AGENT:` lines. Pass these literal IDs on each invocation.

#### Sender and recipient (`--from-member-id`, `--to-member-id`) {#from-to-member-id}

Send requires integer sender and recipient flags. Both must be active in the same live fleet. The sender row determines the fleet; IDs are coordination identifiers within the local trust model.

#### `permissions.allow` coverage

Use prefix rules such as `Bash(cafleet message *)` to cover broker operations, including `cafleet message listen ID --timeout 300` and finite `cafleet message wait`. Verify native Monitor authorization separately: a Bash-rule check establishes no Monitor permission. Director capture is covered by `Bash(cafleet fleet capture *)` or a broader authorized prefix. Literal IDs and trailing JSON flags preserve prefix matching.

Keep commands that send explicit native pane input under `permissions.ask`, including the `Bash(cafleet member prompt *)`, `Bash(cafleet member exec *)` and `Bash(cafleet member press *)` prefixes. Ask rules outrank allow rules. Authorize `cafleet permission respond` under the Director host’s effective policy after its request-specific decision; mode/source flags bypass no ask, deny or managed rule. Ordinary member role authority covers broker reception and evidence routing, while the Director owns permission responses. The [permission profiles](docs/docs/spec/coding-agent-backends.md#claude-profile-entries) describe each role's needs.

#### Message Body Truncation

CLI text bodies use `CAFLEET_MAX_TEXT_LEN` Unicode codepoints followed by `…` when truncated. Full JSON and pane captures preserve their complete content. Pane notification lines follow their own [inline-body limit](docs/docs/spec/multiplexer-backends.md#push-notifications). [Message envelope](docs/docs/spec/message-envelope.md#rendered-shape) owns the compact rendering.

#### Text bodies (positional `TEXT` / `--file`) {#text-body-input}

Send and member creation require exactly one positional body or `--file PATH`; `--file -` reads UTF-8 stdin. Preserve exact nonblank body text. Missing/both sources or a blank positional body exit 2; blank file/stdin sources and file-read errors exit 1 with the authored diagnostics below. File read/UTF-8 failures retain the actual cause. Compact role-by-path spawn prompts remain necessary because resolved prompts enter backend/multiplexer argv.

`member prompt` takes one positional line. Native exec has the [separate command contract](#member-exec).

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
| opencode (preset) | `OPENCODE_CONFIG_DIR` | `$OPENCODE_CONFIG_DIR` | `~/.opencode` | — | `<base>/agents/cafleet.md`, `<base>/agents/cafleet-ask.md` |

opencode splits by purpose: `agents/` is in `OPENCODE_CONFIG_DIR`'s
documented search list, so the presets may relocate and remain valid
`--agent <mode>` discovery paths; skills are not in that list — opencode
discovers them only at fixed paths — so the skills install ignores the
variable.

`OPENCODE_CONFIG_DIR` adds a preset search directory alongside default home configuration. Agent definitions with the same name can merge across these sources; inspect the effective configuration when customizing a preset.

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


##### Assets half {#assets-half}

Each agent's presets, where they exist, are static files embedded in the
binary next to the skills; install targets resolve per
[Config-dir resolution](#config-dir-resolution). Setup installs every embedded
preset of the agent, in this order:

| Agent | Bundled preset | Install target |
|---|---|---|
| claude | — (skills only) | — |
| codex | `presets/codex/cafleet.rules` | `<codex base>/rules/cafleet.rules` (default `~/.codex/rules/cafleet.rules`) |
| opencode | `presets/opencode/cafleet.md` | `<preset base>/agents/cafleet.md` (default `~/.opencode/agents/cafleet.md`) |
| opencode | `presets/opencode/cafleet-ask.md` | `<preset base>/agents/cafleet-ask.md` (default `~/.opencode/agents/cafleet-ask.md`) |

Each backend replaces the two embedded skills sequentially, removes a leftover
`cafleet-research` entry, then replaces each of its presets in order. Existing targets
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


#### Shared diagnosis and connection reuse {#diagnosis-reuse}

Use the invocation’s shared classifier and connection for guard/setup work. Doctor completes read-only sections even when the database is unavailable. Compatibility includes identity, ledger version and expected table/column shapes.

#### Schema-version guard {#schema-version-guard}

The fresh baseline creates exactly the application tables declared in the schema. Compatibility diagnosis verifies current application identity, ledger version, every required table’s column shape, and required indexes/foreign keys. Known versions use their recorded migration shape; newer versions are diagnosed before any head-shape comparison. Unrelated auxiliary tables, indexes and triggers are compatible while those required definitions remain intact.

Every fleet/member/message command validates current format before writes and asset checks. Identity is `application_id=1128351288`; malformed or incompatible files fail with `database format is incompatible at 'PATH'; choose an empty database path and run 'cafleet setup'.` Preserve file bytes and existing sidecars. Empty state reports `no cafleet database; run 'cafleet setup'`. Behind state reports `database schema is outdated (schema M, head N); run 'cafleet setup'`; ahead reports `database schema M is newer than this cafleet (head N); upgrade cafleet`. Database-open/read errors preserve their cause. Setup initializes/repairs compatible known versions; doctor reports rather than blocks its other diagnostics.

#### Stale-assets guard {#stale-assets-guard}

Every fleet-scoped surface — the `fleet`, `member`, and `message` groups — validates the recorded assets installs after the
[schema-version guard](#schema-version-guard) passes and before any
subcommand body runs. The guard resolves each agent's identity path per
[Config-dir resolution](#config-dir-resolution) and checks only the row at
that path. Apply the following precedence:

| Recorded install state | Result | Exit |
|---|---|---|
| A config-path variable fails validation | The pinned [validation error](#config-dir-resolution) | 1 |
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

The calling pane becomes the Director's concrete placement. Run `fleet create` from the coding-agent session declared by `--coding-agent`, then arm native [message reception](#message-listen) before depending on member replies. Root-Director messages remain in the durable inbox for native or manual reads; automatic pane previews target ordinary members.

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

JSON entries are ordered `member_id, name, kind, coding_agent, pane_id, lines, content, captured_at, content_sha256, error`. Kind is director/member. On error, content/time/hash are null; pane_id is null only without a concrete pane. Successful hash/timestamp semantics match [member capture](#member-capture).

#### `cafleet doctor` — Environment Diagnosis {#cafleet-doctor}

A full-environment diagnosis that renders **all** sections even when the
multiplexer is unavailable or the database is missing or stale — no early
abort. Diagnosis order: multiplexer, database, coding agents, member
permissions. `doctor` is
exempt from the [schema-version guard](#schema-version-guard) and the
[stale-assets guard](#stale-assets-guard) — a missing or outdated database
and a stale or missing install are reported, not fatal.

The only flag is the optional [`--json`](#json-output), a trailing
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
| `path` | The resolved identity path ([Config-dir resolution](#config-dir-resolution)), with `~` abbreviation when under `$HOME`. On a resolution error: the raw invalid variable value. |
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

###### OpenCode preset checks {#doctor-opencode-preset}

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

##### Member permissions section {#member-permissions}

Reports every Claude Code setting that would stop a claude member from running
its broker commands, so the failure is visible before a spawn. The section
runs for `claude` when the claude assets are installed, and reads each of
these files that exists:

| File | Location |
|---|---|
| User settings | `<claude config dir>/settings.json`, per [Config-dir resolution](#config-dir-resolution) |
| Project settings | `<cwd>/.claude/settings.json` and `<cwd>/.claude/settings.local.json` |
| Managed settings | `/Library/Application Support/ClaudeCode/managed-settings.json` on macOS, `/etc/claude-code/managed-settings.json` on Linux |

Each rule is tested against the broker-command probe registry, maintained as `PROBE_COMMANDS` in the [permission probe source](https://github.com/himkt/cafleet/blob/main/cafleet/src/member_permissions.rs). A representative probe is `cafleet message listen 1 --timeout 300`. This checks Bash command rules; native Monitor authorization is verified separately.

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
[Spawn-time allow rules](docs/docs/spec/coding-agent-backends.md#spawn-time-allow-rules).

##### Member runtime checks

With a compatible at-head database, doctor diagnoses every recorded member run in ascending `run_id` order without mutating registry or runtime state. Each run produces one entry named `member_runtime.<run_id>`:

| Outcome | Status | Detail |
|---|---|---|
| The read-only diagnosis completes | `info` | The run's diagnostics joined by `; `, such as a leftover launch descriptor, the owner-lock observation and the health-probe result |
| The diagnosis fails | `error` | The diagnosis cause; it counts as an issue |

One five-second budget covers all runs, and each run's diagnosis is bounded by the smaller of 100 ms and the remaining budget. When the budget is spent, doctor adds one `member_runtime` entry with status `info` and detail `runtime diagnosis budget exhausted; inspect remaining runs explicitly`, and skips the remaining runs. A database that is not at head produces no runtime entries. The recovery contract (§ Storage pressure and durability boundary) owns the probes and their recovery actions.

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

Resolve the complete text body before starting the five-second persistence/notification budget, including when `--file -` waits for stdin. Persist one point-to-point delivery, then drain an ordinary recipient's pending messages oldest-first into its concrete pane within the remaining budget, the new message included. Root-Director messages use native or manual broker reads: sends succeed with `notification_sent=false`, no pane-delivery diagnostic and no multiplexer requirement. Self-sends persist with no pane input. A committed/persisted send remains successful with exit 0; held/unavailable ordinary-member notifications preserve the stdout record and `notification_sent=false`.

`notification_sent` is `true` exactly when this invocation delivered the newly committed message, including when it was drained after an older one. When another invocation delivered the new message, `notification_sent` stays `false` and stderr carries no diagnostic; the recipient still received the line. Messages left pending when the budget no longer covers another submission stay pending with no diagnostic; the next send to that recipient or a [message wait](#message-wait) rotation retries them. [Push notifications](docs/docs/spec/multiplexer-backends.md#push-notifications) specifies the line, the drain and under-lock revalidation.

###### Delivery outcome {#message-send-delivery}

A notification failure prints `Message NEW_ID was persisted. ` followed by its diagnostic on stderr, without suppressing successful stdout. Every per-attempt failure uses this diagnostic:

```text
message MESSAGE_ID was not notified to member MEMBER_ID: CAUSE; the next send to member MEMBER_ID or the next message wait retries it.
```

| Failure | `CAUSE` |
|---|---|
| The pane input lock stays busy through the [lock wait](docs/docs/spec/multiplexer-backends.md#pane-input-lock) | `pane input remained busy` |
| Lock acquisition error | `pane input: <lock error>` |
| Multiplexer resolution or availability error | The multiplexer error text |
| Budget cannot cover literal input, settle and Enter before the first line | `insufficient pane input budget for literal input, settle and Enter` |
| Transport error before or during typing | The transport error text |
| Database busy or deadline exceeded during the under-lock snapshot or notification stamp | The storage error text; names the current candidate, or the candidate from placement revalidation when selection has not completed |
| `message wait` attempt fails with database busy or deadline exceeded before selecting a candidate | The storage error text; names the selected rotation entry |

A lock wait that ends after another sender delivered every pending message reports nothing. Other failures, including corrupt, incompatible or unreadable storage and a termination-span start failure, retain their causes after the persisted prefix. Storage failure before commit exits 1 with its cause. Consume an existing committed ID once; notification failure supplies no reason to create another delivery.

After a failed delivery the sender sends exactly one follow-up to the same recipient with the body `message N was not delivered to your pane; poll your inbox`. Its drain retries `N` first. When the follow-up also fails, the sender stops.

##### `message poll`

`cafleet message poll MEMBER_ID [--json]` returns only incoming `input_required` records, descending by status timestamp/message ID. Unknown or inactive member: `Member ID not found`, exit 1.

##### `message wait`

`cafleet message wait MEMBER_ID [--timeout SECONDS] [--json]`

Timeout is an integer in `1..=1800`, default `300`. Validate an active subject and live fleet. Existing unread deliveries return immediately; otherwise check at one-second monotonic intervals. The invocation is a blocking receive for scripts and manual use and never ACKs messages.

Between inbox checks, retry pending deliveries for at most one ordinary member per interval with an in-memory round-robin cursor. Select concrete ordinary-member placements in the fleet, excluding its root Director, the wait's own subject, self-notifications, ACKed and already-notified rows. Each attempt drains that member's pending messages under the [delivery procedure](docs/docs/spec/multiplexer-backends.md#push-notifications) without consulting pane state. One invocation attempts each pending message at most once; a failed message is retried by a later send or wait invocation, while later messages for the same member remain eligible. Recheck the inbox before starting another attempt. Shared input locks and under-lock revalidation keep multiple waiters safe.

Start each receive check or notification attempt before the monotonic receive deadline. An attempt already in flight retains its original five-second budget for the lock wait, revalidation, Escape, typing, settle and Enter. A busy pane lock makes the attempt wait up to 1.5 seconds; a lock still busy then records the [not-notified diagnostic](#message-send-delivery) in `notification_errors`. Cap subprocess/database busy waits to its remaining budget and start input only with time reserved for settle and Enter. Mandatory database finalization uses the later of the original deadline and one second after finalization begins, keeping pane transport within its original budget. A receive returns by its deadline plus at most one six-second in-flight attempt/finalization allowance. A snapshot blocked until receive expiry returns `timeout` with retained notification diagnostics; earlier transient contention can retry before expiry. Corruption and other fatal storage errors retain their causes.

The ordered JSON result is:

```json
{"member_id":41,"outcome":"timeout","messages":[],"notification_errors":[]}
```

`outcome` is `messages` with nonempty full poll records, or `timeout` with an empty array. Each diagnostic has ordered keys `{member_id, message_id, error}`. Keep one diagnostic per not-notified message, ordered by message ID.

Pending placement and a placement change under the lock are ordinary holds that keep entries unchanged. A drain that notifies a recorded message removes that message's entry. Reconcile diagnostics with each successful receive snapshot and the typed observations of its owned notification attempt. Remove entries when these observations show that message ACKed, its recipient gone or the message notified by any invocation. After receive expiry, return the last successful snapshot reconciled with the completed attempt's observations. Subsequent external changes become visible to the next invocation; final reconciliation starts no additional I/O or tail budget.

Text is `Messages available for member ID.` plus normal messages, or `Timed out waiting for member ID.`; diagnostics go to stderr. Both outcomes exit 0. Deleted subject/fleet or corrupt/unreadable state exits 1 with its cause and retains earlier notification diagnostics on stderr, including when JSON output was requested. Ctrl+C outside a delivery attempt terminates the process normally; inside an attempt it takes effect when the attempt's [termination-deferral span](docs/docs/spec/multiplexer-backends.md#termination-during-pane-input) ends.

A script or operator uses `message wait` for finite manual reception and ordinary-member retry recovery, then polls and ACKs consumed messages. Native Director reception retains the existing listener process between events.

##### `message listen`

`cafleet message listen MEMBER_ID [--timeout SECONDS]`

Stream any active member's pending inbox through UTF-8 JSONL. Derive and pin its fleet at startup, using the configured database and ordinary schema/assets guards. Require a live root Director with a concrete registered placement, without resolving a multiplexer or checking its executable/physical pane. The subject's own placement may be absent. Unknown/inactive startup subjects use `Member ID not found`, exit 1.

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

The inner message is exactly the [full message record](docs/docs/spec/message-envelope.md#persisted-shape), preserving original text beyond CLI/pane limits. `closed.reason` is `timeout`, `member_inactive` or `fleet_deleted`; fleet deletion takes precedence when both lifecycle closures are observed. Check normal closures before revalidating a continuing live fleet. Missing persisted rows, changed fleet identity, invalid stored enums/root relationships or missing root placement are fatal errors.

Each short read transaction validates identity and selects every `input_required` record for the pinned subject ordered by `(created_at ASC, message_id ASC)`. Commit before output or sleep. A later commit with an earlier timestamp emits when first observed. A process-local ID set suppresses already-emitted pending IDs; insert only after the complete record and LF flush successfully. Retain only IDs still pending in successful snapshots; a failed snapshot leaves the set unchanged. A snapshot containing only previously emitted pending IDs waits one second, while newly emitted records permit immediate recheck.

Listening leaves consumption and `notified_at` unchanged and performs no pane retry, lock or dispatch. Stamped pending self-sends remain stream-eligible. Concurrent listeners are independent read-only observers with no consumer lock; restart replays all pending IDs. ACK-before-snapshot removes a message, while ACK racing with buffered output may leave a stale pending envelope. Reconcile known/replayed IDs with `poll --json` or `show --json` before acting, record deferred actions and explicitly ACK consumption. Completed records need no second action or ACK.

Each snapshot has a fresh database deadline at most five seconds away, shortened by the remaining lease. Scoped busy waiting and an execution progress handler every 1,000 VM instructions bound lock/SQL work. Check that deadline before/after prepare and execution, before/after decoding each row and before commit. Expiry discards partial snapshots; interrupted SQL maps to deadline exceeded only when that deadline expired. Restore busy handling and remove the listener-owned progress handler on every path, preserving cleanup failures alongside the original cause. These cooperative boundaries include row collection without claiming hard real-time preemption of one allocation or SQL operation.

Output owns one serialized record at a time and applies backpressure before selecting more messages. Nonblocking filesystem/poll output preserves/restores descriptor flags on normal return, resumes partial writes from their byte offset and checks every write/flush. Its five-second deadline is shortened by the lease. Broken pipe, write-zero, flush failure or output deadline exhaustion terminates visibly with exit 1, even after partial records. Normal lease expiry between records uses a separate bounded final write for `closed`. Complete records survive a later error; an incomplete final line remains unconsumed and recovers through broker replay. Only the current snapshot, one record and pending emitted IDs occupy listener memory.

Database contention retries stay inside the snapshot budget; exhaustion or another fatal error terminates the stream. The listener retains its connection/PID across healthy events and ordinary signal behavior outside pane termination deferral. Host cancellation may end it without `closed`. Native owners retain one handle, reconcile durable state after failure and establish old-process termination before replacement. The [manual lifecycle](docs/docs/how-to/mixed-backend-team.md#manual-lifecycle) explains operation; installed backend instructions own concrete native tool recipes and their evidence limits.

##### `message ack`

`cafleet message ack MESSAGE_ID [--json]` transitions an existing unread message to completed, updating its status timestamp. The recipient/fleet derive from that row. ACK signifies consumption.

##### `message show`

`cafleet message show MESSAGE_ID [--json]` returns one full record. Existence is the read guard.

#### `cafleet member` — Member Lifecycle + Pane Interaction {#cafleet-member}

##### Member targeting and key delivery

Require an active member and placement for pane operations; concrete pane IDs remain opaque. Pending placement is valid during creation and yields contextual prompt/exec/capture errors. Missing placement fails pane actions; list/show retain diagnostic nulls. Prompt and exec reject the root Director before pane I/O; individual member deletion protects it.

##### `member create` {#member-create}

Register a member **and** spawn its coding-agent pane. It takes no identity
flag: the acting Director is auto-resolved from `fleets.director_member_id`
first thing, before registration (the resolved id also feeds the member's
backend inheritance and the spawn-prompt substitution). A fleet has exactly
one root Director by construction, so no override flag exists.

| Argument | Required | Notes |
|---|---|---|
| `--fleet-id` | yes | The fleet the new member joins — a relationship flag; the subject of the command is the member being created. |
| `--name` | yes | Display name — see [Known asymmetries](docs/docs/concepts/coding-agents.md#known-asymmetries-intentional-non-goals) for pane-title behavior. |
| `--description` | yes | One-sentence purpose. |
| `--coding-agent` | no | One of `claude`, `codex`, or `opencode`; when omitted, the member — every role — inherits the spawning Director's placement backend. Exits 1 with `Error: binary <name> not found on PATH` when the binary is missing. The opencode backend additionally requires the selected launch policy's agent preset, `<preset base>/agents/<mode>.md`, at the resolved preset path (via `OPENCODE_CONFIG_DIR` — see [Config-dir resolution](#config-dir-resolution)); a missing or unreadable selected preset or an invalid variable exits 1 per [Error Messages](#error-messages). |
| `--model` | no | Model forwarded to the backend binary's own `--model` flag. The opencode backend additionally requires `<provider-id>/<model-id>`; per-backend formats and create-time validation are in [Model selection](docs/docs/spec/coding-agent-backends.md#model-selection). |
| `--permission-mode MODE` | no | Launch-only selection from the resolved backend’s [policy catalog](docs/docs/spec/member-runtime.md#launch-policy-and-capability-registry); omission preserves its default. For opencode the choices are `cafleet` (default) and `cafleet-ask`, each selecting the agent preset of that name. |
| `--event-evidence POLICY` | no | `metadata` by default, or explicit `redacted` evidence under the [privacy contract](docs/docs/spec/member-runtime.md#persistence-notification-and-read-interfaces). |
| `--event-redaction-file PATH` | for redacted | Absolute readable UTF-8 redaction configuration; invalid combinations/configuration fail with usage exit 2 before registration. |
| `--effort` | no | Reasoning-effort level forwarded to the backend binary, validated per backend before any side effect. Accepted levels, forwarding forms, and rejection strings are in [Reasoning effort](docs/docs/spec/coding-agent-backends.md#reasoning-effort). |
| positional `PROMPT` | one of | Inline spawn prompt (backend-neutral template). Exactly one of `PROMPT` / `--file`. |
| `--file PATH` | one of | Path to a UTF-8 file whose contents are the spawn prompt (`-` = stdin). Follow the shared [body-input and transport contract](#text-body-input). |
| `--json` | no | Output as JSON |



###### Spawn command per backend

The per-backend child argv and omitted permission defaults live in
[Coding-agent backends](docs/docs/spec/coding-agent-backends.md#spawn-argv).

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
(see [Error Messages](#error-messages)).

The spawn always creates the pane without stealing focus (tmux
`split-window -d`): the Director's pane and active window stay active. In the
default output, `pane` renders `(pending)` until the pane id is patched onto
the placement.

###### Creation failure compensation {#creation-failure-compensation}

Member creation validates policy, evidence configuration and assignment, and records the installed native version through the [native version probe](docs/docs/spec/member-runtime.md#native-version-probe), then registers a pending placement and run. A failed probe adds the stderr line `Warning: native version unavailable for BACKEND: CAUSE; coverage unknown.` and creation continues with unknown coverage. The multiplexer starts a runner from a private descriptor; a `prepared` handshake precedes placement publication and `start`, and creation returns only after child spawn produces `started` within one handshake deadline. The [runner creation contract](docs/docs/spec/member-runtime.md#runner-boundary-and-creation-transaction) owns descriptor cleanup and uncertain-launch compensation. The CLI owns a registration guard after successful
registration and acquires a pane guard only when `split_window` succeeds.
The backend owns the pane before that return; see
[pane ownership](docs/docs/spec/multiplexer-backends.md#pane-creation-ownership).

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
or deregistration; `Drop` handles only remaining unhandled ownership. Once the runner reports `started` under the matching identity, disarm creation guards before emitting the result. Placement publication alone keeps registration and pane guards armed; a post-start timeout reconciles and cleans up the already-started child. Existing output keys retain their order and nullability; runtime fields append under the [runtime presentation contract](#runtime-presentation). No new stdout-failure exit or diagnostic contract
is introduced.



##### `member delete` {#member-delete}

For wrapped members, request owned runner shutdown under the recorded identity before pane/registry cleanup, retaining explicit unconfirmed-child diagnostics and recoverable runtime records. Attempt remaining event replay before deregistration only within the remaining operation deadline. Lock the concrete placement before kill/deregistration. A member without a concrete pane needs no lock. Retry contention every 50 milliseconds against one five-second deadline; expiry exits 1 with `member ID's pane input remained busy for 5 seconds; member was not deleted.` and leaves state unchanged. Preserve pane/registry errors and layout cleanup ownership.

Revalidate pending or absent placement and deregister atomically. For a concrete pane, retain input ownership while closing it outside a SQLite write transaction, then revalidate and finalize the registry. After successful close, mandatory registry finalization uses the later of the original deadline and one second after finalization begins, allowing short reader/writer contention to clear; actual database failures report that the pane was closed and preserve the database cause. Optional layout work follows mandatory finalization within the original deadline. A failed or expired restoration emits a stderr diagnostic while preserving successful deregistration.

##### `member show` {#member-show}

`cafleet member show MEMBER_ID [--json]` — the positional `MEMBER_ID` names
any active registry entry — placed or placementless (root Director included).

Registry read — no multiplexer requirement. Text is the compact one-line row;
the detailed view — including `kind` ([`director` / `member`](docs/docs/spec/data-model.md#members)) and the
placement sub-dict and appended runtime summary — is the `--json` payload (see
[Runtime presentation](#runtime-presentation)). A wrapped result appends the runtime text line; an unwrapped result retains its existing row.


##### `member list` {#member-list}

`cafleet member list FLEET_ID [--json]` — the positional `FLEET_ID` names the
fleet whose roster is listed; no identity flag. Lists every **active**
registry entry of the fleet —
the root Director, ordinary members, and placementless rows. An empty roster
prints `0 members.`.

Field projection and absent values are in [Member list output](#member-list-output).

Rows and activity follow the [data-model query contract](docs/docs/spec/data-model.md#query-and-activity-contracts).
Text idle values render as `Ns` / `Nm` / `Nh`.
Per-member detail such as `description` and `registered_at` lives on
[`member show`](#member-show).


##### `member prompt` {#member-prompt}

`cafleet member prompt MEMBER_ID TEXT [--json]` validates a nonblank line without CR/LF or C0/DEL controls, otherwise exits 2 with `text must be one non-empty line without control characters.`

A root Director target exits 1 with `cannot prompt the Director's own pane` before any pane I/O, because Escape would interrupt the Director's own running turn. Otherwise acquire the shared input lock within its [bounded wait](docs/docs/spec/multiplexer-backends.md#pane-input-lock) and revalidate activity/placement. For a wrapped member, submit literal text and the native submit key through the verified runner composer profile, gated by the [input arbitration contract](docs/docs/spec/member-runtime.md#input-arbitration-and-the-original-native-answer). Explicitly unwrapped members retain Escape, settle, literal text, settle and Enter; that legacy path can interrupt work or dismiss a prompt. Exit 0 means submission only, with ordered `{member_id, pane_id, text, input_id}`; unwrapped input has a null ID.

A lock still busy when the wait ends uses `member ID's pane input remained busy; prompt was not dispatched.` Pre-write error, including an Escape failure: `prompt was not dispatched to member ID: CAUSE`. Partial uncertainty: `prompt delivery to member ID is uncertain: CAUSE. Inspect the pane before retrying.` These exit 1 without a success record.

##### `member exec` {#member-exec}

`cafleet member exec MEMBER_ID COMMAND [--json]` or `cafleet member exec MEMBER_ID --file PATH [--json]`; `--file -` reads stdin. Exactly one source is required. File/stdin is UTF-8 and strips at most one terminal LF or CRLF. Preserve inline bytes exactly.

Require a nonblank line without CR/LF, NUL or other C0/DEL controls, including tabs. Invalid input exits 2 before pane I/O with `command must be one non-empty line without control characters.` A multiline task supplies a one-line explicit script invocation.

Resolve an active ordinary member and concrete placement, reject the root Director, lock input within its [bounded wait](docs/docs/spec/multiplexer-backends.md#pane-input-lock) and reload. Wrapped input uses the tested native-shell composer profile, literal `! ` plus the command, and the native submit key through the runner. Explicitly unwrapped input retains Escape, settle, literal native shell input, settle and Enter. The backend owns working directory, environment, permissions, output and exit rendering.

Successful exit 0 establishes complete submission/dispatch only. Ordered JSON is:

```json
{"member_id":42,"pane_id":"w1:p2","coding_agent":"codex","command":"printf example","dispatched":true,"input_id":null}
```

Text: `Dispatched native command to member ID (PANE). Inspect the pane for its result.`

| Condition | Error body (exit 1) |
|---|---|
| Root Director | `cannot exec in the Director's own pane` |
| Pending placement | `member ID has no pane yet (pending placement) — nothing to exec.` |
| Input lock still busy when the wait ends | `member ID's pane input remained busy; native command was not dispatched.` |
| Escape or transport failure before typing | `native command was not dispatched to member ID: CAUSE` |
| Typing may have begun | `native command delivery to member ID is uncertain: CAUSE. Inspect the pane before retrying.` |

The first text-writing call can partially type even if it fails. Failure after Enter may mean execution began. Inspect native pane output before any newly authorized retry. A wrapped operation durably records its operation key, payload digest and submission state; the receipt contains no command text. The result describes dispatch, independently of native execution. The Director authorizes native input under its host rules; visibility establishes no second member-tool permission check.

After native completion, inspect the pane and send an explicit prompt to use the output and continue when the host has not resumed the member.

##### `member press` {#member-press}

`cafleet member press MEMBER_ID KEY... [--json]` requests only the specified ordered keys. Require a positive signed 64-bit member ID and one through 64 keys, each at most 32 UTF-8 bytes, using the [common key grammar and backend mappings](docs/docs/spec/multiplexer-backends.md#explicit-key-dispatch). Preserve literal character case and render named keys canonically. Validate the complete vector before any backend probe or writing invocation.

Use the ordinary `--` option separator before literal punctuation that could be read as an option; put `--json` before that separator. For example, `cafleet member press 42 -- '-' '+' ';'` requests three keys. It supplies no Escape prelude, Enter suffix, prompt text or shell-command submission.

Resolve an active ordinary member and its concrete placement, preserving the Director-target restriction and the [shared input lock, selected-context validation and original budget](docs/docs/spec/multiplexer-backends.md#explicit-key-dispatch). Wrapped members receive the request through their outer pane as [manual native input](docs/docs/spec/member-runtime.md#explicit-manual-key-input). Success means backend acknowledgment of this ordered request; actual PTY bytes, native approval and completion require their own observation.

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

##### `member capture` {#member-capture}

`cafleet member capture MEMBER_ID [--lines N] [--ansi] [--json]` — the
positional `MEMBER_ID` names the target member.

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
fleet capture. It hashes the final selected content's UTF-8 bytes, including
Unicode and empty captures. Existing line validation/windowing and timestamp
format stay unchanged. Text member capture prints content without adding a
newline; JSON retains its existing fields and ordering.


#### Runtime presentation {#runtime-presentation}

Member create preserves ordered `member_id,name,registered_at,placement` and appends `runtime`; member show preserves ordered `member_id,name,description,status,registered_at,kind,placement` and appends `runtime`. An explicitly unwrapped member has `runtime: null`. The runtime object has ordered keys `run_id,resolved_mode,status,reachability,owner_epoch,last_health_at,coverage,current_interaction_id,unresolved_input_ids,owner_kind,owner_pid,owner_start_identity,runner_pid,runner_start_identity,child_pid,child_start_identity`, preserving stored nullability. `unresolved_input_ids` lists the run's inputs in `in_progress`, `accepted` or `uncertain`, in input-ID order; an accepted input needs no reconciliation and stays listed until a settlement records its disposition, while `in_progress` and `uncertain` inputs need Director reconciliation. These are registry reads and make no live-health claim.

Wrapped create/show preserves its existing text row and appends `Runtime: run=RUN_ID mode=MODE status=STATUS coverage=COVERAGE`. Wrapped prompt/exec preserves its existing success line and appends `Input: INPUT_ID` when a durable ID exists; JSON appends nullable `input_id` after existing keys. An uncertain error retains its existing exit category and includes known receipt identity in the cause as `input_id=INPUT_ID`; run/interaction context accompanies held/rejected input.

#### Runtime event and input commands

`cafleet event show EVENT_ID [--json]` returns ordered `event,notification_state,message_id,archive_reason,run,interaction`. The event is the complete [versioned runtime payload](docs/docs/spec/member-runtime.md#persistence-notification-and-read-interfaces), run is the current runtime summary, and nullable interaction contains the stored interaction fields in the interaction field order below. Text is `Event: EVENT_ID run=RUN_ID seq=EVENT_SEQ kind=KIND notification=STATE message=MESSAGE_ID` followed by the runtime summary line and `Interaction: INTERACTION_ID classification=CLASS closed_at=TIMESTAMP close_reason=REASON` when linked. Unavailable optional text values render `-`.

`cafleet input show INPUT_ID [--json]` and `cafleet input reconcile INPUT_ID --disposition DISPOSITION --evidence-ref REFERENCE [--json]` return `{input}` with the durable row ordered `input_id,run_id,operation_key,kind,payload_digest,state,created_at,updated_at,message_id,disposition,evidence_ref,permission_response`. The nullable audit object and additional text line follow the [permission contract](#permission-response-audit-output). Text is `Input: INPUT_ID run=RUN_ID kind=KIND state=STATE disposition=DISPOSITION message=MESSAGE_ID`, with `-` for absent optional fields.

Reconciliation is Director-owned. CLI disposition tokens are `submitted`, `not-submitted` and `abandoned-uncertain`, persisted with underscores as defined by the [receipt contract](docs/docs/spec/member-runtime.md#input-arbitration-and-the-original-native-answer). Require a nonempty evidence reference, synchronize with the current owner before CAS settlement and preserve uncertainty while an active writer cannot be ruled out. A settled ordinary receipt with the same disposition returns its original row and evidence reference before live work; a conflicting disposition fails unchanged. Settlement sends no input and establishes no native command result. One original five-second budget covers the pane wait, current-owner synchronization, supplied-connection SQL and socket exchange. If the writer is unreachable, use the [receipt-only exclusive ownership contract](docs/docs/spec/member-runtime.md#input-arbitration-and-the-original-native-answer), preserving the fixed placement state, any original pane guard, the original identity-matching run lock and lifecycle history; unavailable proof retains uncertainty. Ordinary member recovery keeps its completed-terminal no-work behavior.

All event/input IDs are positive database-assigned integers; malformed IDs/options fail with usage exit 2. Missing rows and invalid stored state fail explicitly. Interaction fields are ordered `interaction_id,run_id,episode_seq,classification,evidence_digest,opened_at,evidence_json,closed_at,close_reason`.

##### `member recover` {#member-recover}

`cafleet member recover MEMBER_ID [--json]` is Director-owned and reconciles a recorded run of an active or deregistered member. A member without a run fails explicitly before mutation. The [recovery contract](docs/docs/spec/member-runtime.md#storage-pressure-and-durability-boundary) owns identity, ownership, replay, archival and its bounded pass.

The exceptional form is `cafleet member recover MEMBER_ID --owner-terminated --expected-run-id RUN_ID --expected-owner-epoch EPOCH --evidence-ref REFERENCE [--json]`. Require all four options together, positive snapshot IDs/epoch and a nonempty operator evidence reference; incomplete or invalid input exits 2. A changed owner rejects the attestation, and positive live evidence preserves the live owner.

Ordered JSON keys are `member_id,run_id,outcome,replayed_count,inbox_count,archived_count,status,cleanup_diagnostics`. Outcome is `recovered`, `recovering`, `live_or_unreachable` or `operator_reconciliation_required`; diagnostics retain cleanup/attestation provenance. Counters measure work newly committed by this invocation under the runtime recovery count domains below. A matching completed terminal return validates supplied current run/epoch expectations and preserves existing history with zero counts. Text is `Recovery: member=MEMBER_ID run=RUN_ID outcome=OUTCOME status=STATUS replayed=COUNT inbox=COUNT archived=COUNT`, followed by each bounded diagnostic as `Cleanup: DIAGNOSTIC` in array order. A string diagnostic renders its contained text; a structured diagnostic renders compact JSON. Incomplete or unreachable outcomes remain distinct from confirmed recovery.

#### Permission commands {#permission-commands}

`cafleet permission inspect INTERACTION_ID --event-id EVENT_ID --from-member-id DIRECTOR_ID [--json]` reads the current request through its pinned runner. Require positive IDs, then an existing interaction (an unknown `INTERACTION_ID` is reported as not found before any event, run or database-identity check), then an active ordinary target, pinned active Director and live fleet; verify the event/message association and interaction ownership under the existing pane lock. One original five-second deadline, clipped to any inherited SQL bound, covers locking, current observation/manual progress, database work and the control exchange. Inspection sends zero programmatic child bytes and creates no receipt, audit or claim. Unknown answer capability is an inspection result; stale ownership or an unavailable owner is an operational error.

The inspection JSON is `{request}`. Its object has ordered keys:

```text
event_id,message_id,member_id,fleet_id,director_member_id,run_id,owner_epoch,interaction_id,episode_seq,classification,visibility,backend,native_version,profile_version,answer_profile_version,answer_rule_id,answer_capability,capability_reason,supported_decisions,request_token,output_seq,evidence,missing_evidence
```

IDs/epoch/episode are positive integers; `output_seq` is an unsigned integer. `message_id` is the verified inbox message ID. `classification` is the current interactive class or null, and `visibility` is `visible`, `obscured`, `not_visible` or `unknown`. Native/detector/answer-profile identifiers are nullable when unavailable. `answer_profile_version` is shared by the currently supported decision plans. `answer_rule_id` is their common rule when unique, including the actual rule for one supported decision; differing supported rules yield null. Respond derives its chosen nonnullable rule internally for the immutable audit. `answer_capability` is `tested` or `unknown`; `capability_reason` is null when the current request is answerable and otherwise a bounded explanation. `supported_decisions` contains the currently tested subset of `allow,deny`, in that order; a blocked or unsupported request returns an empty array and null token.

`request_token` is a nullable random lowercase 64-hex value issued by the current runner, bound to the request and invalidated under the [response contract](docs/docs/spec/member-runtime.md#request-bound-permission-responses). `evidence` is the complete selected current post-policy object. `missing_evidence` lists unavailable public facts in order from `operation,arguments,cwd,environment,targets,effects,choice_scope`. Treat operation as available only when its post-policy value is nonblank, unredacted and untruncated; metadata-only, `[redacted]` or truncated evidence leaves it missing. The current detector supplies no independent structured arguments, cwd, environment, targets or effects, so those facts remain missing rather than being parsed from operation text. Include `choice_scope` whenever the public post-policy evidence has no explicit scope fact, including scope-free evidence accompanied by a valid token and a proved private `single_request` plan. The Director obtains necessary missing facts through authorized current capture/routed evidence before deciding. Token identity and private plan eligibility are independent of public evidence completeness and authorization. The token binds the full immutable decision-to-plan set, including each profile, rule and stage identity; a null common-rule field supplies no authority to select a replacement plan.

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

Require positive IDs/epoch, the token's exact hex form, a nonblank operation key and the [bounded evidence references](docs/docs/spec/member-runtime.md#permission-response-storage). Source `director-auto` requires mode `auto`; source `user` records the user's decision in any mode. Enum/shape/source-mode errors exit 2 before lock or byte effects. These fields are attributed claims: resolve Director mode from current host evidence independently of member launch policy and obtain user permission in other/unknown modes. Native and managed rules still authorize running the command.

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

For a consumed key, `zero_bytes` describes this retry only and supplies no zero-byte evidence about the original attempt. A prior abandoned-uncertain claim remains possibly transmitted. This projection reads retained history even after current owner/interaction eligibility is lost.


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
| `permission request is not visible` | The request is obscured, closed or not observed as visible. |
| `permission request is not the current interaction` | Input eligibility is not held for the current permission interaction. |

A retained operation key is resolved before the admission drain, so its stored result returns without typing even when the current request is blocked.

The CLI takes semantic decisions. For a new attempt, the pinned runner validates the current stable permission request, token, profile, manual input and output state, atomically reserves its audit/claim/receipt, and sends only a tested single-request native choice. Same-key retries return the prior receipt; a conflicting decision/digest or another key bypassing the episode claim fails. Unknown capability preserves manual native input. `member prompt`, `member exec`, message previews, receipt settlement and recovery remain separate operations.

##### Permission-response audit output

`input show` and `input reconcile` append nullable `permission_response` after `evidence_ref`. For a response kind it contains ordered `input_id,event_id,interaction_id,expected_owner_epoch,request_token,decision,director_member_id,director_mode,decision_source,mode_evidence_ref,decision_evidence_ref,answer_profile_version,answer_rule_id,decided_at`; every field is required and follows its SQL type/nullability. Other input kinds return null.

Preserve the existing input text line and append `Permission decision: interaction=INTERACTION_ID event=EVENT_ID decision=DECISION source=SOURCE mode=MODE director=DIRECTOR_ID epoch=EPOCH` for a response audit. Settlement records disposition and sends zero bytes; it neither grants a native answer nor releases a possibly transmitted episode claim.

#### Error Messages

Application errors print as stderr text even with `--json`; the flag selects successful output. The [permission-response exception](#permission-commands) also prints a structured held/rejected/uncertain response to stdout before exit 1 and its stderr diagnostic. Input errors use the documented usage/application category. Dispatch, input-lock, compatibility and deletion-lock errors are specified at their action owners above.

| Command | Situation | Error message | Exit | Notes |
|---|---|---|---|---|
| (any surface resolving a config dir) | A set config-path variable holds a non-absolute value (empty string and relative paths included) | [Config-path validation](#config-dir-resolution) | 1 | See [Config-dir resolution](#config-dir-resolution); `doctor` renders it as a per-agent issue instead |
| (any non-setup command) | The recorded schema version is behind the embedded head | [Schema-version guard](#schema-version-guard) | 1 | See [Schema-version guard](#schema-version-guard) |
| (any non-setup command) | No schema ledger and no app tables (missing or empty DB file) | [Schema-version guard](#schema-version-guard) | 1 | See [Schema-version guard](#schema-version-guard) |
| (any non-setup command) | No schema ledger but app tables present | [Schema-version guard](#schema-version-guard) | 1 | See [Schema-version guard](#schema-version-guard) |
| (any non-setup command) | The recorded schema version is ahead of the embedded head | [Schema-version guard](#schema-version-guard) | 1 | See [Schema-version guard](#schema-version-guard) |
| (any fleet-scoped command) | No agent has an `asset_installs` row at its currently-resolved path | [Stale-assets guard](#stale-assets-guard) | 1 | See [Stale-assets guard](#stale-assets-guard) |
| (any fleet-scoped command) | An `asset_installs` row at a resolved path differs from the runtime CLI version | [Stale-assets guard](#stale-assets-guard) | 1 | See [Stale-assets guard](#stale-assets-guard) |
| `member create` | `--coding-agent opencode` with the selected agent preset missing or unreadable at the resolved preset path | `Error: cannot read opencode agent preset at <preset>: <cause>; run 'cafleet setup --coding-agent opencode' first` | 1 | `<preset>` is the resolved `<preset base>/agents/<mode>.md` for the resolved launch policy, and `<cause>` is the read error; fires before registration or multiplexer effects — see [Config-dir resolution](#config-dir-resolution) |
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
| `member prompt` | The target is the fleet's root Director | `Error: cannot prompt the Director's own pane` | 1 | Before any pane I/O |
| `member exec` | The target is the fleet's root Director | `Error: cannot exec in the Director's own pane` | 1 | — |
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
| `member create` | `--coding-agent opencode --model` violating the `<provider-id>/<model-id>` format | [Backend model error](docs/docs/spec/coding-agent-backends.md#model-selection) | 2 | Fires before any side effect |
| `member create` | `--effort` with a level unknown to the claude backend | [Backend effort errors](docs/docs/spec/coding-agent-backends.md#reasoning-effort) | 2 | Fires before any side effect |
| `member create` | `--coding-agent codex --effort` with an unknown level | [Backend effort errors](docs/docs/spec/coding-agent-backends.md#reasoning-effort) | 2 | Fires before any side effect |
| `member create` | `--coding-agent opencode --effort` with any value | [Backend effort errors](docs/docs/spec/coding-agent-backends.md#reasoning-effort) | 2 | Fires before any side effect |
| `member create` | An unknown `{placeholder}` in the prompt | `Error: Unknown placeholder '<name>' in custom prompt. Supported placeholders: {fleet_id}, {member_id}, {director_member_id}, {coding_agent}. Double literal braces ({{, }}) to keep them as text.` | 2 | `member create` attempts deregistration; cleanup failures follow the primary error |
| `member create` | A malformed brace expression in the prompt | `Error: Malformed custom prompt: <detail>. Double literal braces ({{, }}) to keep them as text.` | 2 | `member create` attempts deregistration; cleanup failures follow the primary error |
| `member create` | `--coding-agent` omitted and the spawning Director not found in the fleet | `Error: cannot resolve the member's coding agent: Director <director-id> not found in fleet <fleet-id>. Re-run with an explicit --coding-agent.` | 1 | Nothing spawned |
| `member create` | `--coding-agent` omitted and the spawning Director has no placement row | `Error: cannot resolve the member's coding agent: Director <director-id> has no placement row recording its backend. Re-run with an explicit --coding-agent.` | 1 | Nothing spawned |
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

Text bodies use `CAFLEET_MAX_TEXT_LEN` codepoints plus `…`; JSON preserves the complete exact text. A pane notification line folds the full body under its own line contract, independent of `CAFLEET_MAX_TEXT_LEN`.


#### Pane preview forms

A wrapped member receives the exact line:

```text
[cafleet msg MESSAGE_ID from SENDER_ID TIMESTAMP] BODY [cafleet] Resume your work if something was still running.
```

An explicitly unwrapped member retains the exact legacy line:

```text
[cafleet msg MESSAGE_ID from SENDER_ID TIMESTAMP] BODY [cafleet] Resume your work if something was still running. [cafleet] If this delivery's Escape dismissed a pending prompt in your pane, that rejection did not come from the user; re-issue the tool call if you still need it.
```

Both forms preserve the existing body folding, inline limit and long-body `message show` pointer. Wrapped delivery follows the [input receipt contract](docs/docs/spec/member-runtime.md#input-arbitration-and-the-original-native-answer); held input preserves the native request and leaves its notification stamp unset.

#### Runtime observation messages

A runtime notification is an ordinary inbox record whose `text` is the complete versioned event JSON. The pinned observed member is its sender, and the Director is its recipient; verify the durable event/message association through `event show` before acting. Its typed source distinguishes runner observation from model-authored speech. Version-2 permission-opening events carry a broker-built review instruction and inspection argv, validated against the stored event/Director association. ACK consumes the message independently of the attributed decision, answer submission, event visibility and actual continuation. The [runtime contract](docs/docs/spec/member-runtime.md#persistence-notification-and-read-interfaces) owns evidence policy, atomic association and archived recovery.

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
| Capture for `member capture` / `fleet capture` | `capture-pane -e` | `pane read --format ansi` |
| Unwrapped input | Optional Escape lead, literal text, settle, Enter | Optional Escape lead, literal text, settle, Enter |
| Wrapped input | Pinned PTY runner control socket | Pinned PTY runner control socket |
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
identifiers (see [CLI options](docs/docs/spec/cli-options.md#cafleet-doctor)).


#### Error taxonomy

Backend failures share a base `MultiplexerError`, with the per-backend
subclasses in the [backend matrix](#backend-matrix). CLI boundaries catch
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
[creation failure order](docs/docs/spec/cli-options.md#creation-failure-compensation).
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
this differently, per the [backend matrix](#backend-matrix).

On herdr, `<dir>` is the invoking process's current working directory. herdr's own
inheritance is not relied upon because herdr spawns `/bin/sh` instead of the
passwd login shell when `SHELL` is unset
([herdr discussion #1517](https://github.com/ogulcancelik/herdr/discussions/1517)).
An unresolvable cwd fails the spawn loudly with `HerdrError`; there is no
fallback directory.


#### Delete-time pane layout {#delete-time-pane-layout}

Closing a member pane leaves the two backends asymmetric on layout reflow, per
the [backend matrix](#backend-matrix).

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
rebalance uses the original operation deadline after registry finalization.
A failed or expired restoration emits a stderr diagnostic; `member delete`
retains its successful close and deregistration result.


#### Pane input lock {#pane-input-lock}

Every operation that writes pane input uses one process-shared OS file lock. Acquire the guard before placement revalidation. Wrapped prompt/exec/preview submissions hold it through short database validation, bounded runner submission and durable receipt/stamp settlement; explicitly unwrapped submissions hold it through the Escape lead, literal text, settle, Enter and the notification stamp of the last drained message. Explicit key requests hold it through their outer-pane dispatch and bounded cleanup. Close the non-cloned owned handle on every path.

| Property | Contract |
|---|---|
| Directory | `<canonical-database-file>.pane-locks/` adjacent to the database |
| Key | SHA-256 of UTF-8 `backend + NUL + mux_session + NUL + mux_pane_id` |
| File | `<key>.lock`, read/write create without truncation, owner-only creation permissions |
| Acquisition | Exclusive `std::fs::File::try_lock`, retried every 50 milliseconds while another writer holds it; preserve unexpected errors with path/member context |
| Identity | Keep lockfiles stable while processes may use the database; unlinking a held file would let another process lock a different inode for the same pane |
| Contention | Prompt/exec/preview submission reserves 3.5 seconds, leaving at most 1.5 seconds to wait. Explicit keys use their [dispatch reservation](#explicit-key-dispatch); a lock still busy at its wait bound fails that writer |

The 3.5-second input reservation covers a 0.5-second revalidation margin, the 0.5-second Escape settle, a 0.5-second minimum literal budget, the one-second submit settle and one second for Enter. A writer that acquires the lock within its wait therefore still submits; a lock released just before the wait ends still leads to a successful submission. No database statement runs while a writer waits for the lock.

| Writer | Lock still busy when the wait ends |
|---|---|
| `message send` / `message wait` delivery | `message MESSAGE_ID was not notified to member MEMBER_ID: pane input remained busy; the next send to member MEMBER_ID or the next message wait retries it.` |
| `member prompt` | Exit 1: `member ID's pane input remained busy; prompt was not dispatched.` |
| `member exec` | Exit 1: `member ID's pane input remained busy; native command was not dispatched.` |
| `member press` | Exit 1 with the [primary and zero-dispatch diagnostics](docs/docs/spec/cli-options.md#member-press) |

When a delivery's lock wait ends, one short read selects the oldest pending message this invocation has not attempted for the recipient. When none remains, the attempt reports nothing. Otherwise the lock-busy diagnostic names that message.

Duplicate placements for one pane share the same lock key. Process exit releases ownership. Read-only capture needs no input lock.

Member deletion locks its concrete placement before killing/deregistering. Fleet deletion acquires active placement keys in sorted order and revalidates the placement set in its transaction. Retry contention every 50 milliseconds against one operation-wide five-second monotonic deadline. A changed set releases all guards and restarts within the same deadline. On timeout release all acquired guards and leave registry/panes unchanged.

Errors are `member ID's pane input remained busy for 5 seconds; member was not deleted.` and `fleet ID pane input remained busy for 5 seconds at member MEMBER_ID; fleet was not deleted.`

#### Termination during pane input {#termination-during-pane-input}

Interrupting a host turn can also end the turn's in-flight shell tool, including a running `cafleet` command. The CLI therefore blocks `SIGINT`, `SIGTERM` and `SIGHUP` across every pane-input and lifecycle span; a signal received meanwhile takes its default action when the span ends. Spans nest and restore the outer signal mask.

| Command | Span |
|---|---|
| `message send`, each `message wait` attempt | From lock acquisition through the last `notified_at` stamp |
| `member prompt`, `member exec` | From lock acquisition through Enter |
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
| `member create` | A pane without a registry row, or a row whose placement stays pending | `member list` and `fleet capture`; `member delete` the row, and close an unregistered pane with the multiplexer's own kill command |
| `member delete` / `fleet delete` | A closed pane whose row remains, or the reverse | `member list`; rerun the delete |

#### Explicit key dispatch {#explicit-key-dispatch}

`member press` maps the complete validated ordered vector from the [CLI contract](docs/docs/spec/cli-options.md#member-press) to one writing invocation: `tmux send-keys -t PANE KEY...` or `herdr pane send-keys PANE KEY...`. The typed registry owns the common grammar and mappings below. Single printable characters preserve case; named keys and modifiers match case-insensitively, and control letters normalize to lowercase.

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

Both writing adapters acknowledge only successful process completion with empty/whitespace stdout. Unexpected non-whitespace stdout on success is unreadable acknowledgment with uncertain effects; herdr metadata retains its command-specific JSON parsing. Binary-not-found/not-started means zero writing invocations; every started failure, including pane-gone stderr, timeout or unreadable output, retains possible prefix delivery and receives no automatic retry. Backend keyboard modes or key interception can affect actual PTY bytes even after acknowledgment. Use the [CLI result/error contract](docs/docs/spec/cli-options.md#member-press) and observe actual native effects separately.

#### Prompt dispatch (`send_prompt`) {#prompt-dispatch}

The sequences below specify the explicitly unwrapped multiplexer path. Runner-managed members use a pinned, healthy control socket and the tested composer profile under the [runtime input contract](docs/docs/spec/member-runtime.md#input-arbitration-and-the-original-native-answer); they send literal text and the native submit key without Escape, hold previews for an observed/candidate interaction and reject unsafe explicit input before writing. Unknown coverage and uncertain reachability preserve manual terminal use while holding generated input.

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

Deletion and prompt/exec operations start their five-second budgets after invocation schema and asset guards. Resolve native command bodies before starting their input budget. Each submission has one five-second transport budget:

1. Reserve the one-second settle delay and one second for Enter, plus the 0.5-second Escape settle for the Escape lead. When no time remains beyond the reservation, fail with `insufficient pane input budget for literal input, settle and Enter` and start no input.
2. For the Escape lead, send Escape bounded by one second or the unreserved time, whichever is shorter, then wait the Escape settle. Any Escape failure (launch, missing pane, nonzero exit or timeout) reports no dispatch, because no text was typed.
3. Recompute the literal budget as the remaining time minus the settle delay and Enter. When Escape consumed the slack, fail with the same insufficient-budget error and type no text. Otherwise type the literal line within that budget, wait the settle delay and submit Enter.

A confirmed launch failure or recognized missing pane reports no dispatch. A started text write with an ambiguous result, or any failure after successful literal input, is uncertain. A failure after Enter can mean the command ran. Inspect the pane before authorizing any retry.

The lock excludes CAFleet writers; the user's keyboard and host UI can change at any time. Successful dispatch establishes complete submission only.

#### Push notifications {#push-notifications}

Active ordinary recipients with concrete panes receive previews through their recorded transport. The root Director receives through native [message listen](docs/docs/spec/cli-options.md#message-listen) or manual broker reads, with zero automatic pane input. Self-sends persist without pane input.

Wrapped previews use this exact line through eligible runner input:

```text
[cafleet msg MESSAGE_ID from SENDER_ID TIMESTAMP] BODY [cafleet] Resume your work if something was still running.
```

Explicitly unwrapped previews retain their full legacy line:

```text
[cafleet msg MESSAGE_ID from SENDER_ID TIMESTAMP] BODY [cafleet] Resume your work if something was still running. [cafleet] If this delivery's Escape dismissed a pending prompt in your pane, that rejection did not come from the user; re-issue the tool call if you still need it.
```

Stored text remains exact. Both forms fold CRLF/LF/CR to `⏎`, tabs to `␉`, and other C0/DEL controls to visible hexadecimal escapes (`\xNN`). A folded body of at most 4000 Unicode scalar values appears inline; a longer body becomes `Body is N characters; run cafleet message show MESSAGE_ID --json to read it.`, where N counts stored text. This limit is independent of `CAFLEET_MAX_TEXT_LEN`. Every wrapped line ends with the resume clause; every unwrapped line carries both clauses.

One synchronous drain owns the shared pane lock and five-second attempt budget. Commit short read snapshots before socket or multiplexer I/O.

1. Read recipient/message eligibility, excluding the root Director before preview selection or pane locking. An inactive/ineligible recipient, fleet mismatch or no pending row ends the attempt; pending placement holds it.
2. For wrapped delivery, query durable key `preview:<run_id>:<message_id>` before pane acquisition or live input eligibility. Conflicting reuse fails; in-progress or uncertain holds for inspection, and a settled key returns its existing disposition without replay. An accepted receipt permits settlement without typing. Capture one expiration from the original deadline before the pane wait for every path that will submit, settle or retire, including retained accepted/submitted history.
3. Acquire the pane input lock, then revalidate activity, member kind, fleet, concrete placement, message status and notification stamp. Changed placement holds delivery.
4. Resolve the recorded transport. A wrapped new input attempt requires a fresh pinned identity/epoch and socket health; retained accepted history proceeds to settlement. An unwrapped placement resolves the multiplexer and checks availability.
5. Select the oldest unattempted pending message by `(created_at,message_id)` and revalidate the recipient/placement. Each invocation attempts a selected message at most once. Query the wrapped receipt for each later candidate before current input eligibility; retain the same owned lock.
6. A wrapped new eligible attempt commits its durable in-progress row before child bytes, then records accepted or uncertain submission. Reuse its captured expiration for submission and any later retirement.
7. Settle wrapped accepted delivery in one transaction that stamps `notified_at` or verifies ACK and marks the receipt settled/submitted. The serialized service verifies that same durable settled/submitted receipt and excludes every active pending writer before removing an unleased cache entry with no available attempt permit. Retirement uses the remaining original expiration and sends zero native bytes. Failure after possible input preserves the actual receipt state and notification stamp for reconciliation.
8. For unwrapped delivery, reserve the lead/settle/Enter time plus the minimum literal budget before input. Use Escape for the first line and direct input for later drained lines, then stamp `notified_at` after full submission; an intervening ACK counts as notified.
9. Continue at step 5 while the budget and input state permit. Wrapped input rechecks current output and composer state for every new attempt; a busy/unsafe state holds the next message.

A retirement failure after settlement preserves the committed sent prefix and `notification_sent=true`. Report `message MESSAGE_ID was notified to member MEMBER_ID; input INPUT_ID is settled/submitted; receipt retirement failed: CAUSE`, retaining known run/input/key context and the original cause. This warning uses the existing attempt error shape; durable settlement remains authoritative and grants no submission replay.

The lock remains held through final submission/settlement. Wrapped delivery follows the [durable input contract](docs/docs/spec/member-runtime.md#input-arbitration-and-the-original-native-answer); only absent receipts or certified ready/zero-byte state permit a new eligible attempt. Explicitly unwrapped delivery remains at-least-once: a failed stamp leaves the message pending, so later send/wait may repeat partial or complete input. Poll and ACK identify consumption independently of either transport.

The [notification diagnostic](docs/docs/spec/cli-options.md#message-send-delivery) applies to failed input attempts and database contention, retaining actual causes and the committed message. Mandatory finalization uses the later of the original deadline and one second after finalization begins, while the transport remains within its original deadline. Other fatal storage errors retain their cause; a send preserves its committed result. A held state remains pending for external retry and reports no successful notification.

Delivery leaves `status_state` unchanged. An inline or pointer preview remains `input_required` until its recipient consumes and ACKs the message ID.

##### Held delivery {#held-delivery}

Wrapped delivery additionally remains held for a candidate/open interaction, draft/working/unknown composer, untested profile, degraded/recovering/lost observer or uncertain receipt/reachability. Only a verified empty composer allows a new submission. Held delivery leaves `notified_at` unset, creates no new permission event and grants no native answer. After inspecting the attributed native response, its actual result and continuation, consume/ACK available inbox records and explicitly invoke finite `message wait` to retry previews; the runner and listener perform no autonomous retry.

Pending placement and a placement change under the lock are holds. Lock contention waits for the lock within the attempt budget. A message remains pending while the lock stays busy, the pane is not yet placed, the budget runs out or an attempt fails. The next send to that ordinary recipient, or a [message wait](docs/docs/spec/cli-options.md#message-wait) rotation, retries it. Rotation selects concrete ordinary-member placements, excluding the root Director and the wait's own subject, and attempts at most one member per interval. One invocation attempts each pending message at most once; a later invocation retries a failed one. Successful snapshots can establish ACK, departure or notification by another invocation.

Transport and stamp failures after persistence remain diagnostics alongside a successful send result. No notification outcome erases a committed delivery.

##### Accepted trade-offs {#accepted-trade-offs}

The following trade-offs apply to explicitly unwrapped input. What a single Escape does depends on the backend, and a backend may need more than one Escape to interrupt a running session. Delivery is correct whatever Escape does: when Escape does not interrupt, the line is still typed into the composer and the host submits or queues it. Both trailing clauses are conditional, so they stay accurate either way.

- Where Escape interrupts an ordinary member's working turn, the resume clause tells the recipient to continue.
- Where Escape dismisses an ordinary member's question, confirmation or approval prompt, the dismissed-prompt note tells that member the rejection did not come from the user and to re-issue the required tool call.
- Escape does not reliably clear typed composer text on every host; unsent text in an ordinary member's pane can be submitted together with the delivered line.
- A failed or interrupted notification is retried, so a recipient can see the same `[cafleet msg ID …]` line more than once; it polls and ACKs by ID.
- Two sends in quick succession from different senders each lead with Escape; the second can interrupt the turn the first delivery started.
- Interrupting a host turn can also end its in-flight shell tool; the CLI defers termination signals across its spans ([termination during pane input](#termination-during-pane-input)).
- Opencode mini captures can expose only the status bar. Inspect retained session records for submitted input and native shell output when capture omits them; use broker poll and ACK to establish message consumption.

##### Delivery outcomes

`notification_sent` is `true` exactly when this invocation delivered the newly committed message. Delivering only older messages, or another invocation delivering the new message, leaves it `false` without a diagnostic. The [send contract](docs/docs/spec/cli-options.md#message-send) owns exact output and errors; [message wait](docs/docs/spec/cli-options.md#message-wait) owns diagnostic revalidation and bounded retries.

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


#### Explicit key adapter surface

The validated `PressKeys` value owns its private typed vector and produces canonical names plus backend-specific arguments. `AnyMultiplexer` and each concrete backend expose `validate_key_target(placement, deadline)` and `send_keys(pane_id, keys, deadline)`; the latter accepts only that validated value and performs the [single bounded writing invocation](#explicit-key-dispatch). The runtime press entry accepts the original supplied connection/deadline and retains its validated borrowed pane lock through process cleanup.

The press chain uses `CommandRunner::run_until(argv, absolute_deadline)` for every process, including availability/current-context/target probes. `SystemRunner` retains that absolute bound through spawn, collection and cleanup; `DeadlineRunner` clips it by any earlier retained bound. A `TimedRunError` retains `source: RunError` and ordered `cleanup_diagnostics`, preserving existing run-error variants. Legacy relative and untimed APIs retain their independent behavior.

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

Use argv subprocesses without a shell for multiplexer utilities. For the Escape lead, send tmux `send-keys Escape` or herdr `pane send-keys escape` and wait the Escape settle. Type literal payload with tmux `send-keys -l` or herdr `pane send-text`, then the configured settle and a separate Enter. Every notification/prompt/exec uses the shared owned guard and operation deadline.

Preserve binary-not-found, nonzero stderr and timeout context in the backend error type. Pane-gone tolerance applies only to an explicit ignore-missing teardown and a recognized pane-not-found cause; other errors propagate. Layout rebalance remains best effort after confirmed close.

### 6.6 Receiving and pane input

The receive task is the synchronous caller-owned process. Its round-robin cursor, attempted-message set and per-message diagnostics live only in memory. Check the subject/inbox and reconcile diagnostics against each successful snapshot before another attempt. Retry one eligible ordinary member other than the subject per one-second interval, excluding the root Director. A member remains eligible while it has a pending message this invocation has not attempted. Each failed attempt records its message ID in the attempted set and retains one diagnostic keyed by that ID, emitted in ascending message-ID order. Successful snapshots remove diagnostics for ACKed messages, departed recipients or messages notified by any invocation; the owned drain also removes entries for messages it notifies. Database busy or deadline exceeded before candidate selection records the same not-notified diagnostic for the selected rotation entry and adds that ID to the attempted set. A fatal error retains every notification diagnostic with its cause. Start a receive check or attempt only before the monotonic receive deadline; after expiry return the last successful snapshot reconciled with its owned attempt's typed result. A later invocation observes subsequent external changes.

Each notification or composer submission has one five-second budget. Wrapped composer input includes pane locking, bounded identity/health validation, runner submission and durable receipt settlement; explicitly unwrapped composer input includes Escape, literal input, settle, Enter and database waits with its documented reservations. Explicit manual key input follows its separate [absolute dispatch and cleanup contract](#explicit-key-dispatch), including invocation database validation. Mandatory database finalization uses the later of the original deadline and one second after finalization begins. A receive returns by its own deadline plus one in-flight attempt and at most one second of finalization grace, retaining diagnostics on snapshot contention at expiry. Explicit prompt and native command dispatch distinguish confirmed non-launch or missing-pane failures from ambiguous started writes.

The shared OS guard uses `File::try_lock`, retried every 50 milliseconds until the writer's lock deadline (its budget minus the input reservation). Store stable, owner-created lockfiles under `<canonical-database-file>.pane-locks/`, keyed by SHA-256 of backend, NUL, session, NUL, pane ID. Acquire before reload; retain ownership through final submission and durable settlement. No pane capture precedes input. Close on every path. Keep lockfiles in place. Placement deletion locks the same keys with sorted acquisition, revalidation and one five-second deadline.

The persistent listener owns one connection and process-local pending-ID set. Each short validated snapshot ends before stdout/backpressure or sleep; its scoped SQL progress handler and Rust row-boundary checks enforce a fresh bounded deadline. Output preserves exact full records with checked nonblocking partial writes and flushes, restoring descriptor flags on normal return. Listening performs no preview retry, pane lock, multiplexer resolution or termination-deferral span. The full JSONL, replay, closure and deadline contract is in §6.3, message listen. Native adapters retain one handle, reconcile replay and establish process termination before recovery/renewal; goal activation/pause/budget and pending decisions retain host authority.

### 6.7 Coding-agent backends

The backend registry may use a name-to-backend map or a backend enum; either representation must satisfy the same registry contract.

A Member backend requires a compatible PTY lifecycle and explicit coverage declaration, with launch, detection and composer capability evaluated separately. A Director backend requires native goal lifecycle controls and background execution while supervision continues; native receive support is evaluated independently, and an eligible Director may use manual finite reception. Preserve host activation, pause, completion, budget, managed policy and sandbox authority.

`PermissionAdapter` owns launch-mode choices/defaults, exact argv mapping, version eligibility and detection/composer profiles. Omitted native argv remains as specified below; an explicit native mode changes its permission argument while preserving the remaining argv and literal assignment.

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
- **`binary_name`** — the executable name resolved against `PATH` (`"claude"` /
  `"codex"` / `"opencode"`).
- **`ensure_available()`** — raises if any spawn precondition is unmet: the
  binary resolves on `PATH` and (for backends with bundled presets) the preset
  file exists on disk. A shared helper resolves `binary_name` to an absolute
  executable path per § Native executable resolution and, on a miss, raises
  `binary {binary_name} not found on PATH`.
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
`member create` resolves the launch policy before `ensure_available`, which
runs the PATH check on `opencode` **first**, then reads the selected preset
file at the resolved preset path `<preset base>/agents/<mode>.md` for the
resolved mode — `<preset base>` resolved through the probe's `env_var` lookup
per §6.3 *Config-dir resolution* (`OPENCODE_CONFIG_DIR`, default
`~/.opencode`); an invalid variable surfaces the pinned validation error
before the read (see § opencode preset). The unselected preset is not
consulted. The preset requirement is `required_preset(config, mode)`, which
returns `<config>/agents/<mode>.md` for opencode and nothing for the other
backends. `display_name` is ignored; the prompt is passed as a `--prompt
<prompt>` flag pair (two tokens), unlike claude/codex's bare positional.

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

prefix_rule(
    pattern = ["cafleet", "member", "press"],
    decision = "prompt",
    justification = "cafleet member press sends explicit native keys to a member pane",
)

prefix_rule(
    pattern = ["cafleet", "permission", "respond"],
    decision = "prompt",
    justification = "cafleet permission respond writes a native permission answer into a member pane",
)

prefix_rule(
    pattern = ["cafleet", "input", "reconcile"],
    decision = "prompt",
    justification = "cafleet input reconcile records a Director-owned input disposition",
)

prefix_rule(
    pattern = ["cafleet", "member", "recover"],
    decision = "prompt",
    justification = "cafleet member recover reconciles a member run under exclusive ownership",
)
```

Codex applies the strictest matching decision, so the Director-owned explicit
pane-input, native-answer, input-reconciliation and run-recovery commands retain
their specific approval rules. Commands without a more
specific rule match the broad allow.

#### opencode preset

Each opencode launch policy names an agent definition of the same name. Both
are static files embedded in the binary and installed, in this order, by the
assets half of `setup` (§6.3), overwriting any existing target — `<preset
base>` resolved per §6.3 *Config-dir resolution* (`OPENCODE_CONFIG_DIR`,
default `~/.opencode`):

| Launch policy | Spawn argument | Source | Install target |
|---|---|---|---|
| `cafleet` (default) | `--agent cafleet` | `presets/opencode/cafleet.md` | `<preset base>/agents/cafleet.md` |
| `cafleet-ask` | `--agent cafleet-ask` | `presets/opencode/cafleet-ask.md` | `<preset base>/agents/cafleet-ask.md` |

**Two opencode base directories serve two distinct purposes** and are not
interchangeable: the agent presets live under the resolved preset base
(default `~/.opencode/`), which is opencode's `--agent <mode>` discovery path
(`agents/` is in `OPENCODE_CONFIG_DIR`'s documented search list, so a
relocated preset remains discoverable); `setup`'s skills install (§6.3)
targets the fixed `~/.config/opencode/`, cafleet's own skills-install target
for the opencode agent.

The selected policy's preset is a spawn precondition (the spawn argv references
`--agent <mode>`): opencode's `ensure_available` reads the file at the resolved
install target for the resolved mode and raises `cannot read opencode agent
preset at {preset}: {cause}; run 'cafleet setup --coding-agent opencode'
first` when the read fails — the remedy names the selector as the targeted
repair; plain `cafleet setup` also installs it, as it covers all three agents.
The unselected preset is not consulted.

Preset discovery includes the relocated directory and default home configuration.
Agent definitions with the same name can merge across those sources; inspect the
effective configuration when customizing a preset.

Each preset's `shell` ruleset places its `"*"` base before its explicit allowlist. Opencode selects the last matching rule: later denies for the Director-owned commands — explicit pane input (`cafleet member prompt *`, `cafleet member exec *`, `cafleet member press *`), request-bound native answers (`cafleet permission respond *`), input reconciliation (`cafleet input reconcile *`) and run recovery (`cafleet member recover *`) — win over `cafleet *`. A member's denied command receives no permission prompt, while permitted broker commands run normally. Tool permission and assignment authority remain separate; the assigned role and Director instructions determine which allowed operations a member may perform.

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

#### Exact preset file contents (verbatim)

The checked-in `presets/opencode/cafleet.md` is the content contract: a
`---`-delimited **JSON** (not YAML) frontmatter block (2-space indent,
non-ASCII preserved, top-level key order `description`, `mode`, `permission`),
a blank line, then the markdown body. `shell`/`read`/`edit` are glob→decision
maps (`"allow"`/`"deny"`); the other seven `permission` fields are scalar
`"deny"`. Reproduce this file faithfully (the body contains literal backticks
around command names and `.env`):

````markdown
---
{
  "description": "CAFleet-spawned member with a deny-by-default shell allowlist derived from the operator's Claude Code permission set.",
  "mode": "primary",
  "permission": {
    "shell": {
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
      "cafleet member press *": "deny",
      "cafleet member recover *": "deny",
      "cafleet permission respond *": "deny",
      "cafleet input reconcile *": "deny"
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

You are a CAFleet member spawned by the Director. The frontmatter is the literal tool-permission map: Shell defaults to deny and allows its listed patterns, including `cafleet *`, the listed Git commands, inspection utilities and CAFleet mise tasks. Read/edit permit their listed paths with `.env` exclusions; external-directory access and the other named tools retain their declared denials. A denied command receives no permission prompt.

Your role and the Director's assignment determine which permitted tools you may use for this task. Broad tool patterns do not grant Director-only actions or Git write authority to an ordinary member. Follow the CAFleet member protocol and route a genuinely needed denied command through prompt-routing after reconsidering it. Read the Director's spawn instructions for your assigned scope.

Perform assignments with the selected provider/model and permitted workspace tools. Poll after ready, before reports and on every wake; ACK consumed messages and handle queued work before becoming idle. The Director owns explicit pane input dispatch, including native keys and shell commands, and inspects native output before explicit continuation when needed.
````

The checked-in `presets/opencode/cafleet-ask.md` follows the same format. Its
frontmatter equals the `cafleet` preset's except `description` and the shell
`"*"` value, and its body replaces only the first paragraph:

````markdown
---
{
  "description": "CAFleet-spawned member whose unmatched shell commands open a native permission form for the Director to answer.",
  "mode": "primary",
  "permission": {
    "shell": {
      "*": "ask",
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
      "cafleet member press *": "deny",
      "cafleet member recover *": "deny",
      "cafleet permission respond *": "deny",
      "cafleet input reconcile *": "deny"
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

You are a CAFleet member spawned by the Director. The frontmatter is the literal tool-permission map: Shell allows its listed patterns, including `cafleet *`, the listed Git commands, inspection utilities and CAFleet mise tasks, denies the Director-owned CAFleet commands, and asks for every other command. An asked command opens a native permission form; CAFleet notifies the Director, who answers it, and you continue from the result OpenCode returns. Read/edit permit their listed paths with `.env` exclusions; external-directory access and the other named tools retain their declared denials.

Your role and the Director's assignment determine which permitted tools you may use for this task. Broad tool patterns do not grant Director-only actions or Git write authority to an ordinary member. Follow the CAFleet member protocol and route a genuinely needed denied command through prompt-routing after reconsidering it. Read the Director's spawn instructions for your assigned scope.

Perform assignments with the selected provider/model and permitted workspace tools. Poll after ready, before reports and on every wake; ACK consumed messages and handle queued work before becoming idle. The Director owns explicit pane input dispatch, including native keys and shell commands, and inspects native output before explicit continuation when needed.
````

In each preset, the body has three paragraphs after the heading, separated by a
blank line, with no internal hard line breaks in any paragraph. Each file ends
with exactly one trailing newline.

#### Contract error strings

- PATH miss: `binary {binary_name} not found on PATH`
- opencode model format: `--model for the opencode backend must be
  '<provider-id>/<model-id>' (got '{model}').`
- claude effort level: `--effort for the claude backend must be one of low,
  medium, high, xhigh, max (got '{effort}').`
- codex effort level: `--effort for the codex backend must be one of minimal,
  low, medium, high, xhigh (got '{effort}').`
- opencode effort: `opencode does not support reasoning effort.`
- missing or unreadable selected opencode preset: `cannot read opencode agent
  preset at {preset}: {cause}; run 'cafleet setup --coding-agent opencode'
  first`
- invalid config-path variable: `{var} must be an absolute path (got
  '{value}')` (§6.3 *Config-dir resolution*)

#### Host evidence and native-input authorization

**claude**: A separate evaluator checks the session condition against the transcript; native background work defers evaluation and returns a host result. Use one native Monitor task for Director inbox records, retaining its handle across separate events and renewing after observed source-task exit. The installed CAFleet skill's [Claude native recipe](https://github.com/himkt/cafleet/blob/main/skills/cafleet/reference/coding-agents.md#claude-native-director-reception) owns discovery, invocation, renewal and evidence limits. Preserve supplied `/goal` state and pending user decisions; manual finite reception remains available when Monitor is unavailable or denied.

Native shell probes displayed command/stdout/stderr followed by a model turn; the trigger and any second permission check were not established. The Director authorizes each routed command under its host rules.

**codex**: The objective belongs to the thread, with model-callable lifecycle tools and optional token-budget accounting. Retain one native execution session handle for Director inbox reception and explicitly read it while delegated work remains, independently of whether a host goal exists. The installed CAFleet skill's [Codex native recipe](https://github.com/himkt/cafleet/blob/main/skills/cafleet/reference/coding-agents.md#codex-native-director-reception) owns tool calls, bounded reads, leases and recovery. Preserve supplied goal activation, pause, completion and budget authority. A background process alone supplies no unsolicited per-line wake after a model turn ends; fully idle Codex reconciles its inbox on the next user/native activation.

A native shell probe displayed the actual command, exit code, duration and output. Installed protocol metadata describes unsandboxed dispatch while public CLI guidance describes current approval/sandbox settings; the TUI's effective boundary and protocol path were not established. Treat native input as a Director-authorized action rather than assume a second member-tool permission check.

**opencode**: OpenCode members perform supplied assignments with their selected provider/model, workspace reads/edits, permitted tools and broker communication. The Director uses explicit manual broker reads when no verified native adapter is available, states that limitation and coordinates authorized work. Leading-`!` input runs a native shell command in `mini`. The native path's effective permission boundary and any second member-tool permission check remain unestablished; preserve the preset's member-tool permissions and the Director's command authorization.



### 6.8 Configuration

Settings retain the selected SQLite URL, nonnegative text limit and optional multiplexer override. The process reads only the explicitly named environment variables in §6.3. Spawned member panes inherit `CAFLEET_DATABASE_URL`; identity is rendered into prompt literals.

### 6.9 Member PTY runtime

#### Launch policy and capability registry

`cafleet member create --permission-mode <MODE>` selects a launch policy. Resolve the coding-agent backend first, validate `MODE` against that backend's typed launch-policy catalog, then allocate the member or pane. An omitted value selects the backend default below; an empty, unsupported or unavailable explicit value fails with usage exit `2`, naming the backend and supported choices. Selection is launch-only and is independent of Director policy; it introduces no fleet inheritance, environment default or live policy mutation.

| Backend | Omitted selection | Explicit selection mapping |
|---|---|---|
| Claude | `dontAsk` | Use the native permission-mode value through `--permission-mode`; retain broker-tool allowance, prompt suggestions, name and model/effort boundaries. |
| Codex | `never` with `workspace-write` | Map native approval-policy values through `--ask-for-approval`; retain the explicit workspace sandbox. |
| OpenCode | `cafleet` preset | Map `cafleet` and `cafleet-ask` to `--agent <mode>`, selecting the agent preset of that name; explicit `cafleet` records `mode_source=explicit`. |

The bundled backend policy catalog is the single source of truth for each backend's launch-policy catalog, default, exact argv mapping, native-version eligibility and detector/composer/answer profiles. Help and usage errors report choices from that catalog. Claude accepts native `--permission-mode` values verified from installed help and parser evidence, including documented aliases, recorded by its version-qualified adapter. Claude Code 2.1.302 accepts canonical `default` and its `manual` alias; help displays `manual`, while the [official permission-mode reference](https://code.claude.com/docs/en/permission-modes) documents both. Preserve the explicitly requested spelling in argv and launch intent. Codex accepts the native `--ask-for-approval` values read from the installed `codex --help` and recorded the same way. OpenCode accepts `cafleet` and `cafleet-ask` for every native version, because both select an agent preset rather than a native permission flag; every other explicit token fails with usage exit `2`. The `cafleet` preset resolves every shell check to `allow` or `deny`; the `cafleet-ask` preset asks for every shell command outside its allowlist and its Director-only denies, so OpenCode opens a native permission form.

Creation success establishes that the child was spawned, not that its native policy was accepted. A fast native exit after spawning, including policy rejection, produces `runner_exited` with the actual code/signal and a Director inbox event; the runner preserves the chosen mode rather than retrying with another one. An error from the spawn operation itself still fails creation and follows compensation. There is no post-spawn liveness window.

Record `requested_mode` as nullable text, `resolved_mode` as required text, `mode_source` as `default` or `explicit`, and the exact non-secret permission/sandbox argv in the run record. These fields state launch intent. `effective_policy_json` remains null unless an actual native observation supplies it; an installed version, argv or selected profile alone proves no effective authorization decision.

Keep launch, detection, empty-composer observation, prompt submission, native-exec submission and request-bound answer capabilities separate. Answer profiles declare support for each semantic decision independently. Profiles declare exact backend versions, rule-set version and capability-specific fixture provenance; bundled tested declarations require matching native captures and measured single-request answer stages for the capability claimed. Synthetic fixtures exercise rule and capability validation while unverified components remain unknown or unsupported. A missing version/profile yields `coverage=unknown`, an initial coverage notification and held programmatic input; it still permits manual terminal operation and records the selected mode. Automatic/fixed policies remain useful even where screen detection is unknown. The `cafleet` OpenCode preset's returned denials continue through member evidence routing; returned tool responses are observable only when the native terminal exposes them.

##### Native executable resolution

`member create` resolves the backend's binary to one absolute executable path during the backend availability check, using the creator process's `PATH`. It searches the absolute `PATH` entries in order, skipping empty and relative entries, and selects the first `<entry>/<binary>` that is a regular file with an execute permission bit after following symbolic links. The selected path is used as found, without symbolic-link resolution, so a link or wrapper keeps its own name. When no entry matches, creation fails before registration with exit 1 and `Error: binary <name> not found on PATH`.

The version probe below runs that exact path, and the same path is `argv[0]` of the runner's literal child argv. The probed binary and the launched binary are therefore the same path on every multiplexer; the pane's own `PATH` and shell configuration play no part in selecting the native executable.

##### Native version probe

`member create` records the installed native version of the resolved backend. After launch-policy validation and the backend availability check, and before registration, it runs the resolved executable (§ Native executable resolution) with exactly the argument `--version`, so `claude`, `codex` and `opencode` each report their own version. The probe has null stdin and captures at most 4096 bytes each of stdout and stderr. It ends within two seconds, and those two seconds belong to the command's existing ten-second creation budget, which starts before the probe. A probe still running at its bound is killed.

The probe yields a version only when it exits with status `0` and the first stdout line has exactly one whitespace-separated token matching `^v?[0-9]+(\.[0-9]+)+([-+][0-9A-Za-z.]+)?$`. That token, without a leading `v`, is recorded as the run's `native_version`:

| First stdout line | Recorded `native_version` |
|---|---|
| `2.1.302 (Claude Code)` | `2.1.302` |
| `codex-cli 0.163.0` | `0.163.0` |
| `opencode v2.1.4` | `2.1.4` |
| `v2.1.4 2.1.5` | Probe failure: several version tokens |

The bundled catalog entry for that backend and exact version then supplies `profile_version`, and `coverage` is `tested` when that profile declares detection. A version without a bundled profile records `profile_version` null and `coverage=unknown`.

A probe that cannot start, times out, exits nonzero, prints non-UTF-8 output or has zero or several version tokens records `native_version` null, `profile_version` null and `coverage=unknown`. Creation continues. The command writes one stderr line, `Warning: native version unavailable for BACKEND: CAUSE; coverage unknown.`, and leaves its stdout text and JSON unchanged. The probed version identifies the installed binary only; it establishes no effective native policy.

##### Bundled profiles and native screen fixtures

The bundled catalog is embedded in the CAFleet binary together with the native screen fixtures that its declarations cite. Each profile's capabilities rest on accepted native captures of its exact native version.

Claude Code `2.1.302`, profile `claude-2.1.302-default-3`:

| Capability | Status | Declaration |
|---|---|---|
| Detection | `tested` | The native `default` Bash permission form (`Bash command` heading, approval text, proceed question, focused choice and the `Yes`/`No` single-request choices) in the alternate screen, with a plain or boxed operation, unwrapped or wrapped, within 24 rows from its heading to its `Esc to cancel · Tab to amend` line |
| Allow answer | `tested` | One stage, byte `31` (digit `1`), selecting `Yes` as `allow-once` |
| Deny answer | `tested` | One stage, byte `34` (digit `4`), selecting `No` as `deny-once` |
| Empty-composer observation | `tested` | The idle prompt line with the terminal cursor immediately after the prompt prefix; a verified non-interaction boundary |
| Prompt submission | `unknown` | — |
| Native-exec submission | `unknown` | — |

Both answers select their choice by its number, so they do not depend on which choice is focused. The broadened choices 2 and 3 have no answer plan. The empty-composer observation reads the composer as empty only when the cursor sits at the end of the prompt prefix; a draft whose cursor was moved back to the prefix reads as empty. Every backend and native version without a bundled profile yields `coverage=unknown`.

Detection rests on native captures of the form unwrapped at 120 columns, wrapped plain at 52 columns, and boxed at 89 and 120 columns; both answers rest on the 120-column and the 52-column plain captures. A narrow pane or a long command makes Claude wrap the form: the tip, the description and the choice labels continue on further rows, and the operation between the dashed rules spans several rows. The operation rows take one of two native shapes:

| Shape | Row layout | Where Claude breaks the operation | Native evidence |
|---|---|---|---|
| Plain | One-space margin, then the operation text | At the row's last usable column, one column before the pane's right edge, even inside a token | Captures at 52 columns |
| Boxed | One-space margin, then the `│ ` gutter and the operation text | At a space | Captures at 89 columns (two rows) and 120 columns (one row) |

The shape follows the operation's length rather than the pane width. In the captures, operations of 57 and 61 characters render plain, and an 86-character operation renders boxed, on one row at 120 columns and over two rows at 89 columns.

Claude positions every row explicitly, so the terminal records no soft wrap, and a displayed row break cannot be told from a line break in the command itself. The rule therefore reports the operation as its displayed rows: it removes exactly the one-space margin from each operation row, keeps the boxed `│ ` gutter (also on a single boxed row), and joins the rows with line breaks. No row is joined to another, whichever column it ends at. The profile declares this margin removal in its operation extraction. The evidence records the number of displayed rows as `operation_rows`, and an operation over more than one row extends the Director's [review instruction](#request-bound-permission-responses).

The request identity follows the displayed rows. A resize that rewraps a multi-row operation, or moves it between one row and several, replaces its episode; a resize that leaves every operation row unchanged keeps it.

Codex CLI `0.163.0` launched with `--ask-for-approval on-request --sandbox workspace-write`, profile `codex-0.163.0-on-request-1`:

| Capability | Status | Declaration |
|---|---|---|
| Detection | `tested` | The native command-approval form in the alternate screen, from its `Would you like to run the following command?` heading to its `Press enter to confirm or esc to cancel` line within 24 rows: the `Environment:` and `Reason:` lines, the single-line `$ ` command, the focused `›` choice and the `1. Yes, proceed (y)` / `3. No, and tell Codex what to do differently (esc)` single-request choices, with the cursor inside the form |
| Allow answer | `tested` | One stage, byte `79` (`y`), selecting `Yes, proceed` as `allow-once` |
| Deny answer | `tested` | One stage, byte `33` (digit `3`), selecting `No, and tell Codex what to do differently` as `deny-once` |
| Empty-composer observation | `tested` | The idle `› ` composer line with the terminal cursor immediately after the prefix, its placeholder text shown; a verified non-interaction boundary |
| Prompt submission | `unknown` | — |
| Native-exec submission | `unknown` | — |

The rules anchor on the live form's heading and closing lines rather than on absolute rows, and the reason line is model-authored text that the rule requires but excludes from the request identity. The deny answer uses the choice's digit because that choice displays only `esc` as its shortcut, and CAFleet never sends a lone Escape to a held form. Choice 2, which approves a command prefix persistently, has no answer plan. The native captures showed the decision on the native surface: after allow, Codex printed `✔ You approved codex to run <command> this time` and ran the command; after deny, it printed `✗ You canceled the request to run <command>` and `■ Conversation interrupted`, and ran nothing. These lines are decision evidence for the Director's continuation check, not bundled rules. A Codex configuration that routes approvals to its automatic reviewer (`approvals_reviewer`) answers escalations without any native form, so detection observes no request. The detection covers a single-line command; a command that wraps in a narrow pane yields no detection, so no `interaction_opened` event is produced, programmatic input stays held, and the request is answered manually on the native surface.

OpenCode `2.1.4` `mini`, profile `opencode-2.1.4-cafleet-ask-1`, applies under either OpenCode launch policy; only `cafleet-ask` shows permission forms:

| Capability | Status | Declaration |
|---|---|---|
| Detection | `tested` | Rule `opencode-shell-permission`: the shell permission form on the primary screen, from its `△ Permission required` heading to its `⇆ select  enter confirm  esc reject` control line within 24 rows, with the single-line `$ ` operation, the focused choice and the `Allow once` and `Reject` choices |
| Allow answer | `tested` | One gated stage: byte `0d` (Enter) while `Allow once` is focused, as `allow-once` |
| Deny answer | `tested` | Three gated stages: `68` (`h`) while `Allow once` is focused, `0d` while `Reject` is focused, then `0d` on the empty rejection-reason dialog, as `deny-once` |
| Empty-composer observation | `tested` | The idle `┃` composer line with the terminal cursor immediately after the prefix; a verified non-interaction boundary |
| Prompt submission | `unknown` | — |
| Native-exec submission | `unknown` | — |

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
| Deny | After the final Enter the hint area briefly shows `Waiting for permission event…`, then the dialog disappears and the transcript shows `Step interrupted`, a `<agent> · <model> · <duration>` line and the idle composer; nothing runs | OpenCode does not continue after a rejection; the Director continues the member with an ordinary message |

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

Keep detection, empty-composer observation, prompt submission, native-exec submission and each semantic answer decision independently eligible for the exact backend/native version. Observing an empty composer supplies input-state evidence; executable submission additionally requires its measured literal/submit plan. Existing observation definitions without a submission plan retain their observation coverage and yield no executable composer capability. Production capabilities require accepted exact-version native evidence for the claimed operation; synthetic plans run only in explicitly selected test services.

NativeCapabilities reports empty-composer observation through `composer`, executable prompt submission through `prompt_submission`, and executable native-shell submission through `native_exec_submission`. Each operation field is `tested` exactly when the selected backend/exact native version has a validated executable definition for that operation; otherwise it is `unknown`. A catalog selected explicitly by test services describes its fixture coverage; production declarations additionally require accepted native evidence. Observation coverage and support for another operation leave each operation field independent.

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

The existing composer object has required `rule_ids,fixture_ids` and optional `submission`. When present, `submission` has exactly `prompt,exec`, each null or an object with exactly `prefix_hex,submit_hex,fixture_ids`. Prompt has an empty prefix; exec has prefix `2120`, the UTF-8 bytes of `! `, with independently tested native-shell coverage. Every plan's fixture references resolve to the same backend and every declared exact native version, and establish the complete composer/work boundary for that operation. Unknown fields and incomplete or incompatible provenance fail validation.

| Plan property | Inclusive limit or required representation |
|---|---|
| Answer stages | 1–16 ordered stages. |
| Escape | No stage contains the byte `1b`, for every backend. |
| Answer bytes | 1–256 decoded bytes per stage; at most 4096 decoded bytes per plan. |
| Composer submit key | 1–32 decoded bytes. |
| Composer prefix | Zero bytes for prompt; exactly two bytes for exec as specified above. |
| Encoded native ordinary submission | At most 65536 bytes for prefix, unchanged literal UTF-8 and submit key together; the complete control JSON must independently satisfy its frame limit. |
| Hex byte definitions | Even-length lowercase hexadecimal, with nonempty bytes wherever the domain above requires them. |

Validate the complete shape, reference set, hex form and byte/stage bounds before decoding and constructing an executable plan. Keep selected native bytes and live semantic identity in memory, outside ordinary errors, Debug output and persistence. An ordinary submission attempts its complete encoded payload as one stage; a positive short write stops remaining bytes and records partial uncertainty. Before any positive ordinary write, WouldBlock or Interrupted yields and requires fresh input/output/state validation before another attempt. Answer stages follow their independently measured order and revalidate request, choice, focus and the stage's gate before each stage; a positive short stage stops the remaining plan with uncertainty.

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

#### Classification and episode identity

Each detector consumes a reconstructed screen, title, cursor/input-region evidence, output sequence and known backend version. Return a typed observation with profile/rule identifiers, matched region, extracted visible operation/choices, a normalized semantic fingerprint and a confidence/evidence designation. Compile rules at startup and fail on invalid rule definitions. Bundle versioned profiles in CAFleet; profiles are bundled locally rather than loaded from remote rules or herdr manifests.

| Classification | Evidence needed | Meaning |
|---|---|---|
| `permission_request` | A backend-specific live permission heading/operation and its choice/input region | An observed request awaiting a native decision; title text alone is insufficient. |
| `permission_denied` | A tested permission-specific returned-result region | A displayed refusal, not an arbitrary nonzero exit and not proof the user refused. |
| `permission_continuation` | A tested screen the native agent shows for a pending request after one of its choices is selected, such as a rejection-reason dialog | A continuation of the active permission episode; it never opens an episode (§ Continuation screens). |
| `question` | A tested general input/question form | Human input is required without establishing a tool-permission request. |
| `trust` / `update` | A tested startup trust or update form | A distinct native interaction, preserving its own consequences. |
| `unknown_interaction` | A live interactive form without enough evidence for a more precise type | Hold programmatic input and request inspection. |
| `working` / `composer_empty` / `unknown` | The corresponding tested activity/composer rule, or lack of one | Input eligibility is evaluated separately; absence of permission text is not an empty composer. |

The exact persisted episode-class domain is `permission_request`, `question`, `trust`, `update`, `unknown_interaction`; `trust` and `update` are distinct values. `permission_denied` is a returned-result observation recorded only through `permission_denied_observed`, with a nullable episode link. `permission_continuation`, `working`, `composer_empty` and `unknown` create no interaction row.

A title such as `Action Required`, a quoted approval transcript, a transcript viewer and an old permission line cannot independently create `permission_request`. Profiles must bind controls to the same live region, distinguish general questions/startup forms, and remain explicit about wrapping, layout changes and competing rules. Conflicting strongest classifications yield `unknown_interaction` and preserve the screen. Unsupported versions advertise unknown coverage; missing matches never imply that all requests were observed.

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
| Continuation screen of the active permission episode | Keep the episode open and `Visible` with input held for the interaction. Emit no event, start no obscured timer and keep the stored request evidence. |

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

#### Persistence, notification and read interfaces

The `cafleet_v8.db` format preserves its application identity and existing rows through contiguous embedded migrations. V4 establishes runtime records and V5 adds the permission-response audit/claim relationships specified below. The singleton `runtime_database_identity(singleton_id INTEGER PRIMARY KEY CHECK(singleton_id=1), instance_id TEXT NOT NULL)` has a random 128-bit hex instance ID initialized once during migration. Runtime records have these fields:

| Table | Required fields and constraints |
|---|---|
| `member_runs` | `run_id INTEGER PRIMARY KEY AUTOINCREMENT`; member/fleet/Director foreign keys; backend, launch nonce, canonical runtime path, requested/resolved mode, mode source, permission argv JSON, native version, profile version, coverage, status and lifecycle/health timestamps; nullable runner/child PID plus verified start identity and exit code/signal. At most one nonterminal run per member. |
| `member_interactions` | `interaction_id INTEGER PRIMARY KEY AUTOINCREMENT`, `run_id` foreign key, `episode_seq`, classification, post-policy evidence digest, opened timestamp, nullable closed timestamp/reason, and latest bounded evidence JSON. Unique `(run_id, episode_seq)` and at most one unclosed interaction per run. |
| `member_runtime_events` | `event_id INTEGER PRIMARY KEY AUTOINCREMENT`, `run_id` foreign key, positive `event_seq`, nullable interaction foreign key, `kind`, observed/committed timestamps, versioned payload JSON, notification state and nullable unique `message_id` foreign key. Unique `(run_id, event_seq)` supports idempotent retries. |
| `member_input_operations` | `input_id INTEGER PRIMARY KEY AUTOINCREMENT`, run foreign key, unique `(run_id, operation_key)`, kind, payload digest, nullable message foreign key, durable receipt/settlement state and timestamps. Retain settled rows as replay-rejection records for the run's lifetime. |
| `member_permission_responses` | Immutable required decision/provenance fields keyed by the input ID, with event/interaction/Director foreign keys. |
| `member_permission_response_claims` | Interaction primary key and unique input reference, retaining one possibly transmitted answer per episode. |

Use the enum values specified in this document as SQL CHECK and Rust conversion contracts. `member_runs.status` is `starting`, `prepared`, `running`, `degraded`, `stopping`, `recovering`, `exited`, `failed` or `lost`; terminal states are `exited`, `failed` and `lost`. Nullable fields represent genuinely unavailable evidence or phases not reached. A missing required identity, invalid enum, malformed payload or cross-run interaction reference fails the write/read explicitly. Member history is soft-deregistered; retain these foreign-key-linked records under existing database ownership/retention behavior.

The table below fixes names/types beyond the primary-key fields above. Use `INTEGER` for identities, counters, PIDs and exit values; `TEXT` for enum/string/digest/JSON/timestamp values. Timestamps use UTC RFC 3339 and ordering uses the run/event sequences rather than wall-clock uniqueness.

| Record | Exact fields |
|---|---|
| Run identity/policy | Required `member_id`, `fleet_id`, `director_member_id`, `backend`, `database_identity_json`, `launch_nonce`, `runtime_path`, `resolved_mode`, `mode_source`, `permission_argv_json`, `event_evidence`, `coverage`, `status`, `created_at`; nullable `requested_mode`, `native_version`, `profile_version`, `effective_policy_json`. |
| Run lifecycle/ownership | Required `owner_epoch INTEGER` starts at 1, `owner_kind TEXT` is `creator`, `runner` or `reconciler`, `owner_pid INTEGER`, `owner_start_identity TEXT`, `reachability TEXT` is `reachable` or `suspect`, `cleanup_state TEXT` is `complete` or `incomplete`, `event_seq_reserved_through INTEGER` starts at zero, and `observation_gap_count INTEGER` starts at zero. Nullable `runtime_dir_identity_json`, `run_lock_identity_json`, `runner_pid`, `runner_start_identity`, `child_pid`, `child_start_identity`, `started_at`, `last_health_at`, `ended_at`, `exit_code`, `exit_signal`, `cleanup_diagnostics_json`, `recovery_origin_status`; the latter preserves the original lifecycle status while recovering. |
| Interaction | Required `run_id`, `episode_seq`, `classification`, `evidence_digest`, `opened_at`, `evidence_json`; nullable `closed_at`, `close_reason`. `classification` is an interactive class from the classification table; both closure fields are null together or present together. |
| Runtime event | Required `run_id`, `event_seq`, `kind`, `observed_at`, `committed_at`, `payload_json`, `notification_state`; nullable `interaction_id`, `message_id`, `archive_reason`. `notification_state=inbox` requires a message and null archive reason; `archived` requires no message and a closure reason. |
| Input operation | Required `run_id`, `operation_key TEXT`, `kind TEXT` (`preview`, `prompt`, `exec`, `permission_response`), `payload_digest TEXT`, `state TEXT` (`ready`, `in_progress`, `accepted`, `uncertain`, `settled`), `created_at`, `updated_at`; nullable `message_id`, `disposition TEXT`, `evidence_ref TEXT`. `ready` certifies a zero-byte attempt; settled disposition is `submitted`, `not_submitted` or `abandoned_uncertain`, and preview kind requires its original message ID. Permission responses additionally require the audit and episode-claim relationships specified below. |

Store process-start identities as platform-specific tokens in `TEXT`. JSON fields validate as objects except `permission_argv_json` and `cleanup_diagnostics_json`, which are arrays. New event payloads carry `schema_version=2`; readers validate v1 and v2 explicitly and return existing records and their associated message bytes unchanged on replay. `event_evidence` is `metadata` or `redacted`. `cleanup_state` begins `incomplete` and becomes complete only after successful shutdown cleanup, so a live/crashed run makes no prior completion claim.

`coverage` is `tested` or `unknown`; tested means the pinned profile passed its declared release gates, not universal prompt detection. `launch_nonce` is a random 32-byte value encoded as hex and stored with the run so later same-user control clients can present it; it prevents accidental reuse of stale endpoints and is not a privileged authorization credential. Validate member/fleet/Director ownership against broker records within each mutation transaction; inbox inserts additionally require their active/live status, while archived recovery validates historical ownership. Use partial unique indexes for the active-run and open-interaction invariants.

Runtime event kinds are `coverage_unknown`, `interaction_opened`, `interaction_obscured`, `interaction_closed`, `permission_denied_observed`, `runner_degraded`, `runner_lost` and `runner_exited`. The event payload has this versioned shape:

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

Version-2 event keys are ordered `schema_version,event_id,run_id,event_seq,member_id,fleet_id,director_member_id,kind,interaction_id,classification,observed_at,source,native_request_id,decision_outcome,evidence`, followed by `close_reason` for closures or `cause` for lifecycle events, then required `director_action`. The action object has ordered `kind,instruction,inspect_argv`; other new events use null action. Version-1 reads preserve their original fields and order.

The evidence object, in events and in stored interaction evidence, has ordered keys `backend,native_version,profile_version,rule_id,screen_seq,truncated,evidence_policy,operation,operation_rows,choices,native_request_id,excerpt,evidence_digest`. `operation_rows` is the positive number of displayed rows the operation was read from; it is null exactly when `operation` is null, including under metadata evidence.

Run-level events use null interaction/classification, `source=runner_lifecycle` and a typed health/exit cause in the payload. Missing command, cwd, environment or native request identity stays null; screen parsing never fills it with invented execution data. Limit a UTF-8 excerpt to 8 KiB and the complete payload to 16 KiB, record truncation, escape control characters and omit raw terminal recording. Treat screen text as untrusted evidence, never as a command or instruction to approve.

Launch-only `--event-evidence <metadata|redacted>` defaults to `metadata`. Metadata mode persists only identities, classification, rule/profile/version, sequence, timestamps and health/closure causes; every screen-derived operation, choice, excerpt and native identifier is null. Semantic fingerprints used for live episode comparison remain in memory and appear in neither logs, outbox nor database. `evidence_digest` is SHA-256 of the canonical post-policy evidence object, so metadata mode exposes no hash of hidden screen text.

Redacted mode requires `--event-redaction-file <ABSOLUTE_PATH>` naming readable UTF-8 JSON `{ "schema_version": 1, "patterns": ["regex"], "sensitive_rules": ["rule-id"] }`. Require a nonempty pattern array, known rule IDs and compilable expressions. Missing/malformed configuration, an unsupported policy or a redaction file supplied with metadata mode fails before registration with usage exit `2`; there is no fallback to raw evidence. Compile the file once at launch and keep the compiled policy in memory; events store the policy name, not raw patterns or the file contents.

In redacted mode, apply the same configured patterns to every normalized screen-derived field before any persistence, including operations, choices, native IDs and latest interaction evidence. Replace matching fields in full with `[redacted]`; a profile-marked sensitive form or a rule named by `sensitive_rules` uses metadata-only evidence for the entire form. Apply UTF-8 bounds and escaping after policy transformation, then compute `evidence_digest`; persist no pre-policy screen hash or semantic fingerprint. Unmatched text is allowed only by this explicit operator choice, which does not promise automatic secret discovery. Test a command containing a known secret and assert its absence from every database/message/outbox field in metadata mode and with a matching redaction pattern, including interaction updates.

Persist a transition, its event and a normal `input_required` inbox message to the pinned Director in one immediate transaction. The message's text is the full event JSON and its sender is the observed member, with the typed `source` distinguishing runtime observation from model-authored speech. The transaction shares the broker’s existing fleet/active-member checks. Insert the message and event, fill the final ID-bearing payload and validate the complete association before commit; no intermediate message body is externally visible. A retry of the same `(run_id, event_seq)` returns the existing event/message IDs and verifies identical payload identity; conflicting reuse fails.

Before persistence, the outbox stores an `ObservedTransition` keyed by `(run_id, event_seq)` and nullable `episode_seq`, with kind, original observation time and immutable evidence. The transaction allocates or resolves interaction/event/message IDs and produces the final public payload. Idempotence compares these original transition fields; generated IDs and commit timestamps are not inputs to that comparison.

The runner uses the broker library directly and never depends on its child invoking `cafleet message send`. A persisted notification remains pending through Director absence or listener restart. Live events use `notification_state=inbox`; active ownership and the sender/recipient fleet checks remain mandatory. After closure, ordinary observation stops. Exclusive recovery may replay historical frames as `notification_state=archived`, `message_id=null`, with `archive_reason=member_inactive|fleet_deleted|director_inactive`; it retains the event/interaction history without inserting a message from an inactive sender or claiming Director notification.

`cafleet event show <EVENT_ID> --json` returns the complete durable event, its notification/archive state, the nullable associated message ID and the current interaction/run state. `cafleet input show <INPUT_ID> --json` returns a durable programmatic-input receipt. These IDs are positive database-assigned integers. Member detail/create JSON carries a `runtime` object with the run ID, resolved mode, status, recorded reachability, owner epoch, last health observation, coverage, current interaction ID and unresolved input IDs; an unwrapped member has `runtime: null`. The CLI reference fixes exact layouts and key order. Existing broker message envelopes and ACK semantics stay intact; the wrapped pane-preview text differs as specified below.

#### CLI presentation contract

Member create/show JSON preserves existing fields and appends `runtime`. An unwrapped member has `runtime: null`. The runtime object has ordered keys `run_id`, `resolved_mode`, `status`, `reachability`, `owner_epoch`, `last_health_at`, `coverage`, `current_interaction_id`, `unresolved_input_ids`, `owner_kind`, `owner_pid`, `owner_start_identity`, `runner_pid`, `runner_start_identity`, `child_pid`, `child_start_identity`. Process fields preserve their stored nullability. `unresolved_input_ids` lists the run's inputs in `in_progress`, `accepted` or `uncertain`, in input-ID order. An accepted input is a durable submitted receipt that needs no reconciliation and stays listed until a settlement records its disposition; `in_progress` and `uncertain` inputs need Director reconciliation before a separate deliberate action. These registry-only fields let the Director bind a recovery attestation to the inspected owner snapshot without probing a live socket. Existing member text rows remain intact; a wrapped create/show appends `Runtime: run=RUN_ID mode=MODE status=STATUS coverage=COVERAGE` on a separate line.

| Command | Ordered JSON keys and values |
|---|---|
| `event show` | `event`, `notification_state`, `message_id`, `archive_reason`, `run`, `interaction`; `event` is the complete versioned event payload, `run` is the current runtime summary above, and nullable `interaction` contains the stored interaction fields in the interaction wire order below. |
| `input show` / `input reconcile` | `input`, containing the durable input-operation row in the input-operation wire order below. |
| `member recover` | `member_id`, `run_id`, `outcome`, `replayed_count`, `inbox_count`, `archived_count`, `status`, `cleanup_diagnostics`; outcome is `recovered`, `recovering`, `live_or_unreachable` or `operator_reconciliation_required`. A member with no recorded run fails explicitly before mutation. |
| `member prompt` / `member exec` | Preserve existing keys and append nullable `input_id`; a wrapped durable submission returns its input ID, and an unwrapped submission returns null. |

New read/recovery commands emit JSON when `--json` is supplied and compact human-readable text otherwise, with exact text layouts in the shipping CLI reference. Wrapped prompt/exec text appends `Input: INPUT_ID` on a separate line when a durable ID exists. Uncertain submission errors include the durable input ID when known, so a caller can inspect the receipt even though the command reports failure. Existing non-dispatch/uncertainty errors retain their exit category and add run/interaction/input context in the cause. Recovery outcomes distinguish incomplete or unreachable work from confirmed recovery; printed submission and settlement establish no native execution result.


Compact text layouts preserve the existing member rows and use these additional lines (`-` represents an unavailable optional value):

| Command | Text layout |
|---|---|
| `event show` | `Event: EVENT_ID run=RUN_ID seq=EVENT_SEQ kind=KIND notification=STATE message=MESSAGE_ID`, the runtime summary line, then `Interaction: INTERACTION_ID classification=CLASS closed_at=TIMESTAMP close_reason=REASON` when linked. |
| `input show` / `input reconcile` | `Input: INPUT_ID run=RUN_ID kind=KIND state=STATE disposition=DISPOSITION message=MESSAGE_ID`. |
| `member recover` | `Recovery: member=MEMBER_ID run=RUN_ID outcome=OUTCOME status=STATUS replayed=COUNT inbox=COUNT archived=COUNT`, followed by each bounded diagnostic as `Cleanup: DIAGNOSTIC`. |

Input-operation wire fields are ordered `input_id,run_id,operation_key,kind,payload_digest,state,created_at,updated_at,message_id,disposition,evidence_ref,permission_response`; the final field is the nullable permission-response audit object specified below. Interaction wire fields are ordered `interaction_id,run_id,episode_seq,classification,evidence_digest,opened_at,evidence_json,closed_at,close_reason`. Uncertain submission errors include `input_id=INPUT_ID` when the durable identity exists.

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

Derive the terminal outcome from durable terminal records or the preserved original terminal status, retaining the exact recorded exit code/signal and existing event/message bytes. When a terminal outcome is established, finish that outcome with zero new event counts. Otherwise reserve sequences above the durable high-water mark, close any persisted interaction with `monitor_lost` before `runner_lost`, and finish as `lost`. Choose inbox or archived insertion from current lifecycle state within the same transaction. This branch performs no outbox replay and leaves surviving mismatched artifacts untouched; missing observations remain an explicit gap. Commit the terminal outcome and all new bookkeeping/records atomically. A failed transaction leaves the original state for retry.

Cap the outbox at 16 MiB per run. Retry database contention with bounded exponential delays from 100 ms to two seconds, each clipped to the same absolute database/work/shutdown budget rather than added to the connection's default timeout. Keep terminal transport and socket health service independent. A full channel/outbox or filesystem failure changes the runner to `degraded`, holds all programmatic input, emits one local health diagnostic and retains the failed transition plus an explicit observation-gap counter. Stop further semantic classification rather than silently discarding events, while continuing manual terminal I/O; resume with a fresh screen and an explicit gap event after the writer recovers. `runner_degraded` carries the gap interval/count, and events potentially missed during that gap remain unverified.

Durability begins at a successful SQLite commit; a spooled record is recoverable pending delivery, not a delivered notification. A hard crash before spool fsync may lose an observation; runner-loss reconciliation identifies that coverage gap. If both database and filesystem writes fail, use the control health response and local diagnostic, and preserve the actual error. Neither endless memory growth nor a claim of successful notification is a recovery strategy.

#### Input arbitration and the original native answer

For a runner-managed member, every CAFleet-generated preview, `member prompt` and `member exec` goes through the pinned runner control socket rather than multiplexer pane input. Retain the existing outer `PaneLock` and placement revalidation for ordering with deletion; the runner serializes the final check and child writes with its own terminal-state loop. The lock order is pane lock, short database validation, then bounded socket request; the runner never waits for the external pane lock while serving that request.

Control frames are length-prefixed JSON, capped at 64 KiB, with protocol version, run/nonce/owner epoch, operation key, kind and literal payload. Encode the serialized UTF-8 JSON body length as a four-byte unsigned big-endian prefix; the inclusive 65536-byte body limit excludes that prefix, and an oversized declared length fails immediately after the header is read. Requests and replies require the top-level integer `protocol_version: 1`; missing or unsupported versions and truncated frames fail explicitly. The runner supports `health`, handshake `prepare`/`start`, `receipt`, `submit_prompt`, `submit_exec`, `permission_inspect`, `permission_respond` and `shutdown`. `receipt` has `query`, `retire` and synchronized Director reconciliation actions. Submission replies distinguish `accepted`, `held`, `rejected` and `uncertain`, carrying a nullable input ID, output-sequence evidence and byte/submission stage; the ID is null for a zero-byte attempt with no durable row. Conflicting reuse of an operation key with another payload digest fails; store no literal prompt/command in the receipt table.

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

Replies retain exactly `protocol_version` plus one of `result` or textual `error`. Input results have exactly `kind` plus the following field: submission/`submission`, inspection/`request`, permission_response/`response`, or receipt/`receipt`. Submission contains exactly `status,input_id,stage,output_seq,reason`; permission response and inspection carry the complete ordered public objects fixed by the CLI contract. A receipt object has exactly `status,input_id,submission`: missing has null ID/submission; present has a positive ID and submission object; retired or settled has a positive ID and null submission. Result kind must match the requested operation. Clients validate and reload durable input/audit rows from their supplied database connection; socket receipt metadata supplies no replacement durable identity.

The original invocation captures one expiration and carries it through framing, transit, queues, SQL and every native stage. Convert using paired clock samples `Instant_before,CLOCK_MONOTONIC,Instant_after`, validating sample order and checked arithmetic. Sender expiration adds the remaining original deadline measured at Instant_after to the earlier monotonic sample. Receiver deadline adds remaining expiration measured at the later monotonic sample to Instant_before. Scheduling delay can shorten this budget and grants no extension; use no wall clock. Clock failure, overflow or expiration fails before new reservation/native bytes while preserving prior committed or possibly transmitted state.

At the service, also cap new-operation admission to five seconds from the first observed frame-header byte and any earlier shutdown deadline. Delayed framing and queueing consume that same cap. Client exchange, lock acquisition, supplied-connection SQL scopes and stage advancement retain their earlier original/inherited deadline. The frame body limit remains 65536 serialized UTF-8 bytes, excluding its four-byte prefix; validate declared size before allocation and the complete serialized size before sending.

##### Bounded input draining

Before a programmatic input attempt, drain actual human and child input through the existing manual queue, terminal parser, ordered observer and outer display. A drain returns either `Pending` or `Quiescent` with the actual cumulative `human_generation` and terminal `output_seq`. Quiescence describes transport and observation health; the input coordinator separately validates composer/request eligibility, ownership, reservation and the original deadline before a write.

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

`Quiescent` requires both actual reads to return `WouldBlock` in the same completed round, an empty manual queue, consumed deferred child bytes, successful parser/observer processing and positive free display capacity. The final health sample must be present, nondegraded, have `pending_count=0` and `pending_evidence=false`. An absent health adapter supplies no proof; a healthy `Hold(Unknown)` state may establish physical quiescence while remaining ineligible for programmatic input. The eighth round may certify these actual facts; quota exhaustion, skipped reads, `Interrupted`, `EOF`, outstanding manual bytes or display pressure supply `Pending` instead. A continuously readable source terminates at the round or byte bound. Deferred child bytes precede subsequent reads and consume the same child quota.

Count every actual positive human read immediately with checked cumulative unsigned 64-bit arithmetic, in both ordinary progress and input draining. Retain those counts and literal queued bytes when later PTY, child, display, parser or observer work fails. The service delivers the observed before/after count delta to manual-input invalidation on both successful and failed progress before propagating any error; a simultaneous notification failure accompanies the primary error. Actual nonrecoverable errors propagate, and a zero-length write retains the existing no-progress error. `Interrupted` requires a later fresh read pair; `EOF` records the stream state and leaves shutdown handling to the normal service path.

Clear stored drain proof at entry, then check the original deadline before I/O and before each endpoint/callback. Recheck after completed work, retaining the actual bytes/counts from successful completed I/O before reporting expiry. In-memory bookkeeping and observation of a completed read may finish after expiry; an observed expiration permits no later endpoint call or proof installation. Synchronous I/O and callbacks have no preemption guarantee, so finite work bounds and clock checks supply no hard syscall or callback latency cap. Every call retains the original deadline and performs no internal sleep or deadline renewal.

Retain every completed child read for display and preserve the exact prefix processed by the terminal parser and ordered observer. If a positive child chunk fails to complete that observation, retain an observation-incomplete fence for the `Transport` lifetime. Subsequent drains continue bounded manual and display progress and return `Pending` without proof, including when later health is healthy or the caller supplies a new deadline. Ordinary progress, resize and display flushing retain the fence. Fully completed observation remains complete when only a later deadline or display operation fails. Partly processed chunks are neither replayed nor treated as observed suffixes; the runtime exposes no repair/reset operation for this fence.

Only a successful final check installs an opaque, single-use proof bound to its transport instance, drain turn and actual health/generation/sequence facts. The visible result fields describe evidence and grant no write authority. Taking the stored proof consumes it once. Ordinary progress, resize, prefetch, display mutation/discard and a new drain invalidate it before their validation or I/O, including later error paths; pure generation/snapshot reads preserve it. A new transport begins with generation 0 and no proof. Programmatic writing consumes actual coordinator-owned authority rather than a caller-constructed result.

##### Durable input receipts and settlement

Preview keys are exactly `preview:<run_id>:<message_id>` and are independent of CLI invocation or connection. Every submission first checks the durable operation row, including retired dispositions. Before a later send/wait retries an unstamped message, it queries the receipt. Durable `accepted` permits stamping without typing; `in_progress` or `uncertain` holds for inspection, and a settled key returns its disposition without replay. Only absence of a row under the same verified run/database identity, or a recorded `ready` zero-byte state, permits reevaluating input eligibility. A held/rejected zero-byte attempt creates no side-effect receipt; if a row was already reserved, move it to `ready` only after positively recording that zero bytes were written.

An ordinary `receipt query` reads the durable operation for its fixed run/key and reports its durable status together with the last real retained stage and output sequence. `missing` requires an absent durable row and absent matching cache entry. A durable row remains `present` after cache retirement; a cache entry without its durable reservation is an identity error. Validate the retained immutable input identity before using its stage.

| Durable receipt | Status | Stage without a cache entry |
| --- | --- | --- |
| `ready` | `held` | `zero_bytes`. |
| `in_progress` or `uncertain` | `uncertain` | `unknown`. |
| `accepted` or `settled/submitted` | `accepted` | `submitted`. |
| `settled/not_submitted` or `settled/abandoned_uncertain` | `rejected` | `unknown`. |

With a matching retained entry, return its actual stage and output sequence alongside that durable status; otherwise the output sequence is null. Query preserves pending writers, clients, permits, receipts and native bytes. Its results describe observations: new write authority and zero-byte retirement proof still require their separate validated guards. A settled `not_submitted` or `abandoned_uncertain` disposition alone supplies no observed byte stage; its absent-cache result remains `unknown` and leaves the original transmission uncertainty intact.

Before the first child byte, commit `in_progress` to `member_input_operations` under the current ownership epoch and the bounded database budget. If that commit fails, write no child bytes. After submission, persist `accepted` or `uncertain` before promising that durable outcome; failure to persist after a possible write leaves the durable row in progress and returns uncertainty. The live runner may reconcile its retained stage evidence into the same row later; a crashed runner's in-progress row is never an automatic replay instruction.

Retire only the in-memory receipt after validating durable settlement, the current full run/owner and original run lock, exclusion of every active writer, and absence of a leased guard or available attempt permit for that input. Cache absence is idempotent only after the same durable and ownership checks. For a preview, atomically stamp `notified_at` (or verify the message is already ACKed) and set its input row to `settled/submitted`; a later `receipt retire` verifies that transaction before freeing memory. For prompt/exec, the durable accepted input row itself is the queryable submission record: the creator or runner can settle it as submitted and retire the cache after verifying that stored row. Printed stdout is merely a presentation of its input ID and supplies no durable acknowledgement. Retain settled operation keys/digests/dispositions for the run's database lifetime, so a delayed request still returns the prior disposition and never types again.

Bound unretired side-effect receipts to 4096, rejecting additional new side-effect attempts while that unresolved set is full; settled receipts leave the bounded set. Keep uncertain receipts until the Director records an evidence-based disposition through `cafleet input reconcile <INPUT_ID> --disposition <submitted|not-submitted|abandoned-uncertain> --evidence-ref <REFERENCE> --json`. This Director-owned operation only records disposition, settles the key and permits cache retirement; it sends no bytes, grants no native permission and schedules no replay. `input show` exposes that durable record. Even a settled not-submitted key is consumed; a later deliberate action uses a new operation key after reconciliation.

Reconciliation pins the original input row in its supplied-connection snapshot: `input_id`, `run_id`, `operation_key`, `kind`, `payload_digest`, `message_id`, `created_at`, the complete immutable permission-response audit where present, and expected state remain fixed through live preference and exclusive settlement. For a recorded concrete placement, acquire its genuine pane/input lock and retain that same guard through current-owner synchronization and any receipt-only exclusive ownership proof. Synchronize with the pinned runner's receipt handler so an active byte-writing operation cannot be settled concurrently; an unreachable owner requires the exclusive proof below. Compare-and-swap the original expected operation state in the settlement transaction; a changed row requires a separate invocation. Settlement records the observed input disposition, not the native command's execution outcome.

For an ordinary or permission receipt already settled with the requested disposition, reconciliation returns the unchanged validated durable row and its original `evidence_ref` before live locks, health probes or exchange. A conflicting disposition fails with the row unchanged. This historical return uses the original deadline clipped to any inherited SQL bound and grants no new input authority. For a live nonsettled receipt, validate the full fixed identity and original ownership lock inside the same supplied-connection immediate transaction as the expected-state settlement CAS, after the SQL wait. After a new reconciliation, require the matching `settled` acknowledgment and a validated durable reread, then exactly one `retire` exchange with a matching input ID under the same pane guard and original expiration. A post-commit failure retains the known input context and original cause while the durable settlement remains authoritative. A changed expected state requires a separately reconciled attempt.

Receipt-only reconciliation for ordinary and permission inputs accepts the fixed placement state from that original snapshot. A present placement requires its matching, continuously held pane/input guard. An absent placement requires an absent guard, valid historical member/fleet/Director relationships and a deregistered member or deleted fleet; an active member in a nondeleted fleet requires its recorded placement. Revalidate the exact placement or its exact absence, complete fixed run/input identity and historical relationships inside the supplied-connection immediate transaction after preceding waits. A changed placement, identity or expected input state invalidates the attempt.

Both placement states require the recorded identity-matching original run lock and positive exclusion of the previous owner and relevant startup processes under the original deadline clipped to any inherited SQL bound. A held original lock, matching live process/health, unresolved termination or competing live reconciler preserves uncertainty. Missing or replaced runtime paths supply no original-lock proof. Placement absence, a failed live call and prior exceptional bookkeeping each grant no takeover authority; retain the original live failure when typed exclusion cannot be established. Valid absent-placement history proceeds directly to the same exclusive proof.

Its receipt-only CAS advances only the ownership tuple: increment `owner_epoch`, set `owner_kind=reconciler` and record the current reconciler PID/start identity. Commit that CAS, expected-state settlement and any matching claim removal authorized by a genuine zero-byte proof atomically in one supplied-connection immediate transaction; any failure rolls all those mutations back. Preserve lifecycle status, recovery origin, exit fields, cleanup state/diagnostics, events, reservations, spool and immutable input/audit history. Retain the exclusive guard through commit and release it on every outcome. This path performs no observation recovery, child operation or native input; ordinary member recovery retains its completed-terminal no-work behavior.

A durable ready row is positive zero-byte evidence only under actual writer exclusion and exact associations. Owner death and an attributed not-submitted disposition alone supply no such proof; recovered in-progress, accepted and uncertain records retain their transmission uncertainty. Proven-zero settlement and matching claim release commit atomically under the same proof lifetime. A ready response from a different epoch/token requires a fresh inspection and separate deliberate key; its immutable metadata is never rebound.

The live input guard distinguishes native-write authority, receipt-only writer exclusion and current-plan ready rearm. A production zero-byte proof borrows its still-held guard, binds the complete immutable input/audit and current ownership, and carries a non-Send marker; its constructors and fields remain private. Receipt-only exclusion authorizes bookkeeping without a write permit or current visible prompt. With a retained entry, proof requires its actual zero-byte stage and cumulative count zero; without one, only a validated durable ready row supplies that fact under genuine writer exclusion. A projected retry stage or a settled not-submitted label supplies no replacement observation.

Rearm additionally requires the unchanged old token/epoch/metadata and the actual current request, selected answer plan, manual generation and healthy complete observation drain. Revalidate owner/audit/claim under the bounded immediate transaction, CAS ready to in-progress once, then require fresh normal pre-stage checks before any native byte. Retain the input ID, original audit, decision timestamp, key and digest. Preserve its matching claim, or restore that exact pair only when a released ready row has no competing claim. The exclusion-only proof grants no rearm authority. Each explicit ready invocation captures one expiration; an in-progress stage continuation retains its original attempt expiration.

Live permission query, reconciliation and settled retirement use the existing serialized receipt operations and original run lock. Query preserves the last real retained stage and any pending writer/client. Reconciliation pins the original full input/audit snapshot, excludes active writers and leases, and commits a proven-zero not-submitted settlement plus matching claim deletion in one supplied-connection immediate transaction. Actual retained zero-byte in-progress evidence may establish that synchronized settlement; possible-byte or absent-cache uncertainty retains its claim. Submitted and abandoned-uncertain dispositions retain claims. A mutation failure rolls back settlement and claim removal together. Retirement removes only settled cache after validation; durable audit, key and claim history remain. The client retains its genuine pane guard and original inherited deadline through query, settlement, durable verification and one matching retire; already-settled same-disposition history returns unchanged before live work. An unavailable permission owner uses the same fixed-snapshot receipt-only exclusive guard. Only exact durable ready history supplies its zero-byte proof after positive prior-owner/startup exclusion and ownership CAS; disposition and matching claim removal share that transaction. Possibly transmitted history retains its claim. The native startup probe reports unknown, so unavailable exclusion preserves the original live failure.


Held-preview retry remains owned by external `message send` / finite `message wait`, using the existing pending-preview rotation. The runner never fetches/stamps the next preview merely because an interaction closes. After an attributed Director/user response or manual native answer and verification of the resulting native state, consume/ACK available inbox records and explicitly invoke finite `message wait <DIRECTOR_ID>` to retry pending previews. If wait returns new inbox records first, consume/ACK them and re-enter that receive step; if the composer remains busy, previews stay held for a later receive step. This preserves the scope of the runner as observer and input arbiter.

A process loss, ownership change or unavailable receipt permits no automatic replay of an attempted submission. Keep notification unstamped, retain its durable uncertainty and reconcile actual output before a separate deliberate action. Explicit prompt/exec use generated keys and printed durable input IDs with the same rule. Test cross-invocation accepted-then-failed-stamp recovery, held-then-eligible delivery triggered by an external wait, disconnected partial writes, receipt retirement followed by delayed replay, and a lost explicit-command stdout result.

| Current runner input state | CAFleet-generated input |
|---|---|
| Permission candidate or open interaction of any class | Hold previews; reject explicit prompt/exec before writing any byte, identifying the current interaction or candidate. |
| Working, draft composer, unknown screen/profile, suspect reachability, degraded/recovering/lost observer | Hold/reject before writing; preserve the displayed native state. |
| Verified empty native composer with no interaction or background-work evidence | Permit one literal submission through the tested backend composer profile; recheck unread output and state immediately before writing. |
| State changes after a submission begins | Stop remaining planned input, return `uncertain` with the written stage and require inspection before retry. |

Wrapped submissions send literal text and the native submit key without a leading Escape. Native-exec submission retains `! <command>` only for a backend with a tested native-shell composer profile.

The exact wrapped preview line is:

```text
[cafleet msg MESSAGE_ID from SENDER_ID TIMESTAMP] BODY [cafleet] Resume your work if something was still running.
```

Retain the existing body folding, inline limit and long-body `message show` pointer. Wrapped previews end at the resume clause because their submission sends no Escape. Unwrapped previews retain their existing full line and Escape-dismissal explanation. Held messages keep `notified_at` unset, and only confirmed submission or the existing ACK settlement stamps them. Holding supplies no new permission event and never closes the native request. Native-exec success means submission, not execution success.

Screen observation cannot create an atomic native request API: unread child output may race with a write, an unknown CLI can redraw unexpectedly, and a person can type concurrently. The runner therefore guarantees suppression for observed/candidate interactions and refuses uncertain states; tested composer profiles bound the remaining race, rather than asserting universal prompt preservation. If a profile cannot safely distinguish an empty composer and its concurrent-work boundary, that profile supports manual input only. Release evidence must include this race and preserve uncertainty instead of authorizing a second submission.

Human keystrokes from the outer terminal pass to the child as native input, including the human's intentional approval, denial or cancellation. A Director may submit a request-bound allow or deny through the permission-response path below after applying its session decision policy; ordinary preview/prompt/exec input remains held during the request. Keep attributed decisions, key submission, native outcomes and actual continuation separately observable. Unknown answer capability or uncertain request identity preserves manual original-surface operation. Retain the legacy mux path only for explicitly unwrapped members and preserve its narrower existing semantics.

##### Explicit manual key input {#explicit-manual-key-input}

An explicitly authorized `member press` sends only its requested ordered keys through the member's outer multiplexer pane under the shared pane lock and [selected-context/deadline contract](docs/docs/spec/multiplexer-backends.md#explicit-key-dispatch). It appends no Escape or Enter. This manual path remains available during observer degradation or a programmatic composer hold and follows the [separate CLI acknowledgment/error contract](docs/docs/spec/cli-options.md#member-press).

The runner counts actual positive outer-terminal reads, retains their generation before any later I/O error and accounts each new delta once on both successful and failed transport steps before subsequent automatic advancement. Observed manual input invalidates automatic authority through the retained generation; a key argument or backend acknowledgment supplies no byte count, drain proof or permission receipt. Press changes no durable input/response row. Native approval/denial/cancellation and actual continuation remain independently observed outcomes under the existing Director/user policy.

#### Request-bound permission responses

The Director uses its own session's effective approval mode, independently of the member's launch mode. In verified auto mode, evaluate the requested command or operation and decide allow or deny. In other or unknown modes, obtain the user's request-specific permission before transmitting an answer. Resolve mode from current host/session evidence and record its provenance in host-owned pending state; member argv, event text and a CLI flag supply no proof of Director mode. Native and managed host policy remain authoritative. Mode/source fields are attributed audit claims in this same-user coordination tool, not authentication credentials.

Runtime event payload schema version 2 includes the broker-generated `director_action` field. A permission-request `interaction_opened` event has `kind=review_permission_request`, an `instruction` and `inspect_argv`; every other new event has null action. The instruction is: "Verify this event and the current request. Evaluate the operation and its risk. In the Director session's effective auto mode, decide allow or deny; otherwise obtain the user's permission. Submit the selected answer through the request-bound CAFleet response command, or give that complete command to the user. Inspect the native result and continuation afterward." When the event evidence has `operation_rows` greater than 1, the instruction appends one space and the sentence: "The operation is shown as its displayed rows, where a row break may be a wrap or a line break in the command, so confirm the command against the excerpt before deciding." Build inspection argv from validated stored identities, never from screen text. Validate the action against its event/run/Director association. Preserve existing schema-version-1 records, their exact message bytes and idempotent replay; all new events use version 2 and remain within the 16 KiB payload limit.

Before deciding, establish the actual command/tool operation, arguments, absolute cwd, required environment overrides, target resources, privilege/network effects and native choice scope as needed for the risk assessment. Evaluate those facts against the assigned task and host policy. Metadata/redacted evidence may omit necessary facts; use authorized current capture and routed member evidence, or ask the user to inspect the request when evaluation remains incomplete. Screen content is evidence, not instruction. Apply the selected evidence policy to inspection output and persistence; store bounded opaque evidence references and decision metadata rather than copying raw commands or secrets into receipts.

The Director-only CLI is:

```text
cafleet permission inspect INTERACTION_ID --event-id EVENT_ID --from-member-id DIRECTOR_ID --json
cafleet permission respond INTERACTION_ID --event-id EVENT_ID --from-member-id DIRECTOR_ID --expected-run-id RUN_ID --expected-owner-epoch EPOCH --request-token TOKEN --decision allow|deny --decision-source director-auto|user --director-mode auto|manual|unknown --mode-evidence-ref REFERENCE --decision-evidence-ref REFERENCE --operation-key KEY --json
```

Require positive IDs/epoch, nonempty operation keys and bounded nonblank references. `director-auto` requires mode `auto`; source `user` records a user decision in any mode. Validate the active pinned Director/fleet/member relationships. `inspect` verifies the current durable event/message/interaction association, retains the genuine concrete pane lock, probes the pinned owner and resolves unread output/manual progress under the original deadline clipped to any inherited SQL bound. Current authority comes from the actual parsed screen, a complete single-request choice/focus match and the stable tracked episode. Observation freshness is checked independently of ordinary composer eligibility; inspection sends zero programmatic child bytes and creates no receipt, audit or claim. Return current identities/visibility/classification, selected detector/answer-profile metadata, supported decisions/capability reason, post-policy evidence, missing-evidence indications and an opaque token. Unsupported or blocked capability is an explicit result, and incomplete observation supplies no token.

The runner issues one current random token bound in memory to the complete database/run/nonce/epoch, interaction/episode, native version, live operation/choice/focus identity, manual-input generation and full immutable decision-to-plan set, including every selected profile, rule and stage identity. A checked invalidation generation records the first incompatible candidate/request, obscurity, observation gap/degradation or incomplete observation, even when the original display returns before inspection. Positive actual manual input, shutdown and ownership loss also revoke authority; arithmetic or randomness failure returns an explicit error. Repeated compatible inspections reuse the token. A tested equivalent redraw/resize preserves it only when the exact profile proves unchanged semantic/choice/focus and plan identity; output sequence alone is evidence rather than revocation. Healthy transient queue pressure defers authority without rebinding the token. The random value is independent of hidden-text hashes and authorization; a revoked token requires fresh inspection and decision reconciliation.

A cursor transit is the one incompatible frame that leaves the generation unchanged. It is a frame that yields no current request while a request is current, is not a verified non-interaction, and keeps every row of the current request's matched live region identical in text to the frame that established the request; typically the native CLI moves the cursor outside the form to redraw another part of the screen. During a transit the request is not current, so inspection returns no token and a response is blocked. A following frame with the same request restores it under the unchanged generation and token. A changed region row, a different request fingerprint, closure, `interaction_obscured`, an observation gap, child exit and positive manual input still invalidate immediately.

A gated plan changes the request between its stages, so the runner accepts exactly one change after each complete nonfinal stage: the transition the next stage's gate names. For a focus gate, a fresh quiescent observation qualifies when its episode, rule, semantic fingerprint, choices, native correlation and manual-input generation are unchanged and its resolved focus equals the gate. For a screen gate, the continuation binding of § Continuation screens applies. The runner rebinds the in-flight selection to the qualifying observation, with its new request generation, and writes the next stage. It mints no token, because the reservation already consumed the request token.

The next stage waits for its gate within the responding invocation's deadline, and an observation that still shows the previous stage's screen and focus keeps waiting. These end the response with the failure reporting below (`uncertain` once a byte was written), and the native surface stays for manual input with no further byte:

- a focus on any other option;
- a screen other than the expected continuation after a stage that opens one;
- the episode closing before the final stage;
- any other identity change, human input or a degraded observation;
- the deadline expiring.

Inspection exposes the selected current post-policy evidence independently of private request/match/plan state. Missing facts follow the [permission inspection projection](docs/docs/spec/cli-options.md#permission-commands); neither operation text nor private plan eligibility substitutes for a disclosed structured fact. A proved `single_request` choice establishes answer eligibility independently of whether public evidence states the choice scope. Keep transient raw semantics, focus and plan bytes in memory; inspection preserves the existing evidence schema and persists no token or pre-policy digest.

Validate a retained operation key against its recorded database/run, response identity and canonical digest before any current-request eligibility check, pane lock or live-owner probe. A matching retained receipt returns its stored result after token revocation, interaction closure, deregistration or owner exit, sending zero bytes; conflicting reuse fails. Validate historical audit relationships against recorded ownership while preserving closed/inactive history. Only an absent key or a positively recorded ready zero-byte attempt enters current live-request validation. The durable-only response projection preserves the immutable audit epoch and uses a null output sequence under the [permission CLI contract](docs/docs/spec/cli-options.md#permission-commands); a consumed retry's zero-byte stage describes that retry while retaining original disposition and claim uncertainty.

Inspection reports the shared answer-profile version. Its answer_rule_id is the common rule only when every supported decision resolves to that same rule; one supported decision reports its actual rule, and differing rules yield null. Respond selects the requested decision's actual rule internally and retains that required rule in its immutable audit/digest. A blocked or unsupported request retains the empty-decision/null-token representation.

Use versioned control operations `permission_inspect` and `permission_respond` and the existing receipt query/retire boundary. A five-second command deadline covers locking, database work and socket exchange. Before the first byte, the serialized runner resolves pending manual input and unread output, revalidates the current visible stable permission request/token and tested answer profile, and commits the response receipt, audit and episode claim atomically under the current ownership epoch. Other interaction classes, uncertain state, stale identity or unavailable storage receive zero-byte rejection. Write only the profile's single-request answer plan, rechecking relevant state and each stage's gate before that stage; partial writes or ambiguous state retain durable uncertainty. The CLI accepts semantic allow/deny decisions, not arbitrary key sequences.

Track answer progress as cumulative actual positive byte counts across ordered stages. A complete nonfinal stage retains `partial` progress and durable `in_progress`; only complete final-stage submission establishes `submitted`, followed by durable `accepted` before returning acceptance. A stage syscall returning WouldBlock or Interrupted with zero bytes may yield within the same attempt and original expiration. Retain the untouched next-stage index and any cumulative `partial` progress, then retry only that stage after fresh actual drain, current token/request/choice/focus/plan, manual-generation, owner and immediate-transaction validation through a newly consumed guard. Completed stages remain completed. This continuation supplies neither a new invocation nor a ready-state rearm.

Record a syscall result before yielding. A recorded zero-byte WouldBlock or Interrupted preserves the actual prior stage/count and releases its lease while consuming that guard; a yielding operation remains `in_progress` and supplies no zero-byte retirement proof. Dropping a consumed guard before I/O preserves its actual no-I/O state, while possible I/O without a recorded result retains uncertainty and every known earlier byte count. A positive short write, incompatible state, unrecorded possible I/O or expiration stops the remaining plan. Preserve cumulative progress, the original cause and any known input identity; persist uncertainty only within the original budget, retaining durable `in_progress` when bookkeeping fails.

V5 preserves existing records and expands the input-kind domain with `permission_response`. Table `member_permission_responses` is keyed by `input_id` with event/interaction foreign keys, expected owner epoch, request token, decision, pinned Director ID, attributed Director mode/source, mode/decision evidence references, answer-profile version/rule and decision timestamp. Table `member_permission_response_claims` is keyed by interaction ID with a unique input foreign key, to retain one possibly transmitted answer per observed episode across different operation keys. Validate same-run/event/interaction relationships and all required enum/identity constraints transactionally. The storage subsection below fixes exact SQL names, types and associations.


##### Permission response storage

V5 preserves V4 identities and rows, expands the input-kind check to `kind IN ('preview','prompt','exec','permission_response')`, and creates the following tables. Preserve the existing input-operation columns, key uniqueness, foreign keys and settlement constraints while replacing its kind check.

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

All audit fields are required and use the displayed column order. `decided_at` is the UTC RFC 3339 time when the runner records the attributed decision, independently of native decision time. Tokens are random 32-byte values encoded as lowercase hex; persist no screen-derived fingerprint or pre-policy hash. Evidence references contain 1–2048 UTF-8 bytes, include a non-whitespace character and contain no C0 or DEL control character; they identify host evidence without embedding commands, screen text or secrets.

In the same immediate transaction, require input kind `permission_response`, an active pinned Director, an inbox-linked permission-request opening event and the matching unclosed interaction/run. Validate the expected epoch, event/message association and answer profile before inserting the input, immutable audit and claim. Other input kinds have no audit row. The composite claim foreign key binds its input to the same interaction; same-run/event and recorded ownership relationships are also checked on reads while preserving closed/inactive history; active/current eligibility applies to new attempts only.

The canonical payload digest covers `run_id,owner_epoch,interaction_id,event_id,director_member_id,request_token,decision,director_mode,decision_source,mode_evidence_ref,decision_evidence_ref,answer_profile_version,answer_rule_id`. Encode this metadata as canonical JSON with sorted keys before SHA-256; `input_id`, timestamps, operation key and literal native keys are excluded. Validate a matching key against this digest before any new action.

Retain a claim after possible submission, accepted submission, uncertain settlement or `abandoned_uncertain`. Release it only in the synchronized transaction that proves zero bytes, no active writer and `ready` or settled `not_submitted`; a disposition label alone supplies no zero-byte evidence. Audit rows and consumed operation keys remain retained after claim release.

Reuse input receipt states, settlement dispositions, the 4096 unresolved limit and synchronized retirement. Digest canonical non-secret request identity, token and decision metadata, excluding screen text and literal key bytes. The Director records the operation key before dispatch. Same-key retries return the existing result; conflicting payloads fail. A different key cannot bypass a submitted, active, uncertain or abandoned-uncertain episode claim. Release a claim only after synchronized evidence proves zero bytes and no active writer; a later deliberate attempt uses fresh inspection and a new key. Recovery sends no answer keys. Key submission and recorded `decision=allow` do not establish native approval or command success.

Append nullable `permission_response` audit data to the ordered input-operation output, null for other input kinds. Inspection returns a `request` wrapper. Response output identifies status (`accepted`, `held`, `rejected`, `uncertain`), nullable input ID, interaction/run, decision/source, submission stage and output-sequence evidence. Exit 0 means accepted key submission or the matching previously submitted receipt; usage failures exit 2 and operational/uncertainty errors retain existing categories and known input IDs. The [permission CLI contract](docs/docs/spec/cli-options.md#permission-commands) fixes complete JSON key order, nullability, text layouts and failure output. When asking the user to execute a response, present the concrete command with actual snapshot values; stale values fail before bytes.

Answer-profile capability is exact-version and independently tested for allow and deny. It specifies request form/rule, single-request choice semantics, focus/selection requirements, exact key stages and output revalidation. Use real native evidence rather than invented Enter/Escape mappings. Persistent/session-wide permission changes and answers to trust/update/question forms require their own explicit scope. Unsupported answer profiles preserve manual native input. The screen interface has no atomic native request API; serialize available input/output, reject observed mismatches and retain uncertainty for remaining races and indistinguishable requests.

#### Director consumption and continuation

Use the existing `message listen`, `poll --json` and finite `wait` interfaces. Runtime events reach the Director's stdout receiver and durable inbox; they introduce no composer injection, Escape-first wake, goal activation or new host interrupt guarantee. With a live receiver and available storage, test a target of two seconds from a stable prompt observation to the pending inbox row, and another two seconds to receiver output. Native model consumption latency remains controlled by the Director host and may be unbounded while the host is blocked or paused.

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

For a permission refusal returned to the member, preserve the existing evidence route: exact attempted command, absolute cwd, required environment overrides and actual refusal. A screen event can alert the Director earlier or supplement that route; it supplies no missing dispatch evidence. For a blocked request, verify the event and current request, establish operation/risk evidence and resolve the Director session's effective mode. Decide allow/deny in auto mode; obtain user permission in other or unknown modes. Submit the bound response or give the user its complete concrete command, then inspect native output/exit/error and the member's actual continuation. Unknown answer capability preserves manual original-surface input. Consume/ACK any new inbox records and run an explicit finite `message wait <DIRECTOR_ID>` after that inspection to retry held previews; a new received batch repeats that consumption/receive step. The listener alone performs no preview retries, and a still-busy composer retains its pending messages. A different command submitted through native exec is a separate authorized action, never proof that the original request was answered. Reconcile uncertain outcomes before retries, and preserve host/managed permission authority throughout.

#### Native acceptance and coverage

Use synthetic terminal replay and real native PTYs for every declared detector, composer and answer capability on each registered multiplexer. Fixed-policy or unsupported profiles retain explicit unknown coverage and manual operation.

The release minimum is a real interactive permission request under Claude's native `default` policy and under Codex's native `on-request` policy with `workspace-write`, each on both tmux and herdr. Pin exact installed CLI versions by the acceptance records and corresponding bundled profiles; use permitted disposable fixtures that actually reach the native approval surface. Every minimum case must demonstrate blocked-model-independent detection, a committed actionable event/inbox record, Director consumption and preserved original request. Exercise separate fresh requests for verified Director-auto allow and deny, user-authorized responses under non-auto/unknown modes, and manual original-surface input; include a user-executed complete bound command. Record zero response bytes before required user permission, actual native results and observed continuation separately from submitted keys. Missing native availability, unknown coverage or an unobserved required case fails overall feature acceptance; unknown coverage is only the compatibility path for other versions/backends. Additional profiles retain their own declared acceptance gates.

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
- Test cross-invocation accepted-then-failed-stamp recovery, held-then-eligible delivery triggered by an external wait, disconnected partial writes, receipt retirement followed by delayed replay, and a lost explicit-command stdout result.
- Test every registered backend's omitted-policy argv and each adapter-declared explicit policy; profiles claiming detection, composer or answer support require both fixtures and real native evidence.
## 7. Cross-cutting concerns

### 7.1 Configuration & environment

Expand HOME only for the default `sqlite:///` URL ending in `cafleet_v8.db`. Explicit `sqlite:///<path>` overrides preserve their path text; relative file paths resolve against the invoking working directory before canonical database-file identity supplies shared pane locking. Validate supplied values and required state at their boundary. Config-directory relocation is lazy per selected backend except guards that deliberately inspect every supported asset identity.

### 7.2 Error handling & exit-code policy

Usage/parser/model/effort/line-validation errors exit 2. Application, database and transport errors exit 1. A successful finite timeout exits 0. Top-level application errors use `Error: MESSAGE` on stderr. Preserve primary causes alongside explicit cleanup failures. Missing required state or unknown enum/column values propagate instead of substituting meaningless defaults.

### 7.3 Output / JSON / truncation

JSON uses explicit ordered presenters and unescaped UTF-8. Preserve full message bodies and capture bytes selected for presentation. CLI text output truncates only message bodies by the `CAFLEET_MAX_TEXT_LEN` codepoint limit; pane notification lines carry the folded full body up to 4000 characters and a `message show` pointer beyond it. Exact deliberate error and layout strings remain contractual.

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

Runtime schema extensions retain the application identity and existing data through contiguous embedded migrations. Section 6.9 specifies every additional runtime table field, enum, nullability constraint, foreign-key association and uniqueness invariant, including database-instance identity and settled receipt retention.

### Runtime extension migration

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

### Permission response extension migration

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

## 9. Testing strategy

Exercise current schema/identity and incompatible-file preservation; atomic registration and creation compensation; point-to-point/ACK/pending notification behavior; finite receive deadlines, round-robin retries and diagnostic clearing; process-shared exclusion, bounded lock waits and process-death release; Escape-first argv order, oldest-first drains with one Escape per lock hold, budget exhaustion, zero automatic Director input and no pane capture on any input path; stream ordering/replay, exact bodies, isolation, read-only state, ACK races, scoped SQL/collection deadlines and handler cleanup, bounded partial/broken output and native receiver lifecycle; termination deferral across pane-input spans; literal native command visibility and partial uncertainty; explicit continuation; backend/multiplexer smoke coverage; links, embedding and release builds. Preserve general timed pipe draining and owned-child cleanup coverage.

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
| Stable input exclusion and Escape-first delivery | §6.5–6 |
| Backend argv, presets and evidence boundaries | §6.7 |
| Configuration, errors and release packaging | §7 |
