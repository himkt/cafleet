# CAFleet — Reimplementation Specification

This is the self-contained interface and observable-behavior specification for the `cafleet` CLI, broker, independent coding-agent sessions and embedded assets. Exact authored errors, JSON fields/order and command layouts govern. Text that an operating-system or library error contributes after an authored prefix (written `<io error>`, `<error>` or `<cause>`) is non-normative: an implementation preserves the error's meaning, not its wording, quoting or numeric suffix. An errno name shown in a diagnostic (such as `ECONNREFUSED`) is normative; the description after it is not. Complete module contracts appear in §6 Module specifications.

## 1. Overview & goals

CAFleet multiplexes coding-agent sessions in tmux or herdr and coordinates them with durable point-to-point SQLite messages. The host owns task continuation and native execution results; the Director owns delegation, wake handling, evidence inspection, authorization and cleanup. The shipped product consists of the CLI, embedded skills/presets and documentation site.

Reproduce the documented command/option/exit surface, configuration defaults, current schema, enum values, output shapes, lifecycle, consuming poll, notification, locking, termination deferral and native dispatch semantics. The current format identity defines compatibility; required data corruption surfaces at its read boundary.

## 2. Architecture stance

The core is synchronous. A send previews pending ordinary-member messages within its budget, and each ordinary member's runner retries its own still-pending previews (§6.9 Runner pending-preview sweep). A persistent read-only listener notifies pending messages through JSONL without pane operations. Separate processes coordinate SQLite changes through transactions and complete pane submissions through process-shared file locks. Host-native tools own receiver handles and collect native results; a poll consumes every delivery it returns, and deferred actions and native handles remain in host-owned state. Automatic Director reception sends no Escape, text, Enter or queued composer input.

## 3. Module layout

| Concern | Responsibility |
|---|---|
| Configuration | Selected database from `--database-url` |
| Persistence | Connection factory, format/shape diagnosis and the embedded schema baseline |
| Broker | Typed fleet/member/message/asset records and synchronous transactions |
| Output | Ordered wire presenters, truncation and capture formatting |
| Multiplexer | Discovery, pane creation/capture/input/kill and layout compensation |
| Pane input | Stable OS locks, runner-state gating and durable receipt settlement |
| Member runtime | Owned PTY transport, typed observations, durable event/inbox transactions and identity-pinned recovery |
| Notification / receiving | Ordinary-member draining delivery and read-only persistent inbox streams, optionally forwarded to a Codex thread |
| Coding-agent backends | Registry, availability, model/effort validation and spawn argv |
| CLI | Parse/guard/compose these contracts and emit results |

## 4. Architecture & module dependency graph

```text
CLI -> receiving / notification -> broker -> persistence
       pane input -> multiplexer -> bounded process runner
CLI -> coding-agent backends -> availability probes
CLI -> ordered output presenters
```

The broker persists/queries typed data. It performs no pane I/O. Multiplexer utilities run processes through one bounded runner; bounded pipe draining and owned-child cleanup remain independent of native command submission. Pane ownership transfers only after a successful member pane spawn, with explicit rollback on every failure path.

## 5. Shared domain model

### 5.1 Timestamps

Serialize timestamps as fixed-width UTC `YYYY-MM-DDTHH:MM:SS.ffffff+00:00` text with six fractional digits. Compare canonical timestamps lexicographically for SQL ordering/maxima. Parse only for age/idle arithmetic; externally supplied parsing accepts missing fractional parts and UTC offset spellings. Durations/deadlines use monotonic time.

### 5.2 Core entities

| Entity | Required shape |
|---|---|
| Fleet | ID, name, creation/deletion timestamp, root Director reference and last allocated window ordinal |
| Member | ID, fleet, name, description, active/deregistered status and registration timestamp |
| Placement | Member, multiplexer session, window ordinal and column side, window ID/name and pane (all pending until published), mux backend, agent backend and creation time |
| Message | Sender, integer recipient, exact text, creation and consumption state/timestamp |
| Asset install | Backend/config-path identity and installation timestamp |

The full column and index declarations are in §8. Records decode required columns with typed errors.

### 5.3 Enums

A member's `status` is `active` or `deregistered`; a message's `status_state` is `input_required` or `completed`. Unknown required enum strings fail. Supported backend tokens come from their registries.

### 5.4 Member kind discriminator

Derive `director` exactly when `fleets.director_member_id == members.member_id`, otherwise `member`.

### 5.5 Recipient and pending placement

Every message has an integer `to_member_id` identifying its inbox owner. Nullable `mux_pane_id` represents pending member creation and documented diagnostics; `mux_window_id` and `mux_window_name` are null exactly when it is, because an overflow window's ID exists only after its creation. Required concrete pane actions fail contextually until all three are patched.

### 5.6 Typed records and wire presenters

Broker records carry typed values independently of CLI formatting. Public message records are ordered `message_id, from_member_id, to_member_id, created_at, status_state, status_timestamp, text`. Full JSON remains untruncated; internal notification state stays in storage.

## 6. Module specifications

### 6.1 Persistence & Schema

Schema compatibility and the application identity are specified in §8 Database schema.

Connections use SQLite foreign-key enforcement and a normal five-second busy timeout; a bounded operation caps that wait to its remaining budget. The selected file path resolves per §6.3 Database selection, and its canonical database file keys lock-directory construction.

Format validation, baseline initialization, incompatible-file refusal and file preservation follow §6.3 Schema-version guard. Every operation that writes the database validates the current format first.

### 6.2 Broker

#### Fleets

Bootstrap validates the Director's multiplexer server, pane and backend token, then inserts the fleet in a transaction to allocate its ID. A placement factory receives that ID once, names and verifies the Director window (§6.3 `fleet create`), and supplies the placement before the broker inserts the Director and placement and backfills the Director reference. Construct the returned Director from rows read inside the transaction, then commit. The Director placement is ordinal `0`, column `left`; public creation records `cafleet-<fleet_id>-0`, and the fleet starts with `last_window_ordinal` `0`. The current session remains the Director; no pane split or process launch occurs.

List, show and delete follow §6.3 `fleet list`, `fleet show` and `fleet delete`; deletion preserves messages, and repeat deletion returns zero newly deregistered members.

#### Members

Registration requires a live fleet and active valid root Director. Insert the member and pending placement atomically, recording the chosen slot's `window_ordinal` and `column_side` with a null window ID, window name and pane (§6.5 Fleet window layout). A new-window choice advances the fleet's ordinal in the same transaction with `UPDATE fleets SET last_window_ordinal=<new> WHERE fleet_id=<fleet> AND last_window_ordinal=<read>`; the fleet layout lock makes a mismatch impossible, so a changed-row count other than 1 rolls the transaction back and fails creation with the ordinal invariant error (§6.3 Error Messages). Creation owns compensation for later prompt/spawn/patch failures. Root Director individual deletion fails. Member deletion locks its concrete pane and the fleet layout, kills the pane and deregisters it; a pending member requires no concrete pane lock.

Slot occupancy counts the fleet's active members with a concrete placement by `window_ordinal` and `column_side`, excluding the Director row; a pending placement holds no slot (§6.5 Fleet window layout).

Roster reads active members of live fleets, derive kind (§5.4 Member kind discriminator) and include nullable activity. Activity is the maximum sent, received and consumption timestamp (`last_consumed`); `idle` is the elapsed seconds since the latest known activity, truncated to an integer, and null is the correct state without activity. Detailed views retain registration, description and placement data. Placement mutations acquire the same pane input locks as input writers. Required Director/placement/enum corruption fails rather than producing placeholder values.

#### Messaging

Send requires active sender/recipient in the same live fleet and persists exact text. Every individual fan-out send commits independently; keep already committed IDs when another recipient fails. Poll selects every incoming `input_required` message by descending status timestamp/message ID and completes exactly those rows in the same write transaction (§6.3 `message poll`), which removes their notification eligibility (§6.4 Persisted shape). Show requires existence and changes no state.

Message ID order is commit order. `messages.message_id` is `AUTOINCREMENT`, allocated inside the inserting write transaction, and SQLite serializes writers, so a message with a larger ID always commits after every message with a smaller ID.

Pending preview selection chooses the oldest eligible row by `(created_at, message_id)` for an active ordinary recipient with a concrete pane; root Directors and self-sends are ineligible. Revalidate ordinary-member kind, fleet, placement, message status and null notification stamp while holding pane input ownership. Stamp success conditionally only after complete submission; delivery leaves `status_state` unchanged until the recipient's poll consumes the message. Listening selects every pending record independently of notification fields, including stamped pending self-sends, without changing broker state.

#### Broker errors

Preserve the validation boundary and explicit cause. Missing member/message, inactive fleet/Director, and cross-fleet pair fail with application errors; §6.3 CLI *Error Messages* lists their exact strings.

### 6.3 CLI

#### Subcommand summary

| Command | Purpose | Subject | JSON |
|---|---|---|---|
| `setup` | Initialize the version's database and install assets | — | no |
| `doctor` | Diagnose multiplexer, database and agent assets | — | yes |
| `fleet create` | Register the calling Director atomically | — | yes |
| `fleet list` / `fleet show` | Read fleet registry | `FLEET_ID` for show | yes |
| `fleet delete` | End a fleet's ordinary members, then deregister the fleet | `FLEET_ID` | yes |
| `member create` | Register and spawn an independent agent | `--fleet-id` relationship | yes |
| `member delete` | Kill its owned pane and deregister | `MEMBER_ID` | yes |
| `member show` / `member list` | Read detail / active roster | `MEMBER_ID` / `FLEET_ID` | yes |
| `member prompt` / `member exec` | Submit authorized native input | `MEMBER_ID` | yes |
| `member press` | Request explicit ordered keys | `MEMBER_ID` | yes |
| `member capture` | Read a pane tail | `MEMBER_ID` | yes |
| `event show` | Read a durable runtime event and current state | `EVENT_ID` | yes |
| `input show` / `input reconcile` | Read / settle a durable input receipt | `INPUT_ID` | yes |
| `permission inspect` / `permission respond` | Inspect / answer the current native permission request | `INTERACTION_ID` | yes |
| `message send` | Persist a point-to-point message for the recipient's runner to preview | Sender/recipient flags | yes |
| `message poll` | Read and consume incoming deliveries | `MEMBER_ID` | yes |
| `message listen` | Notify pending messages through stdout | `MEMBER_ID` | JSONL only |
| `message show` | Read one message | `MESSAGE_ID` | yes |

#### Database selection

The global option `--database-url <url>` selects the SQLite database. It is accepted before or after the subcommand, so a prefix allow rule such as `Bash(cafleet message *)` matches `cafleet message poll 7 --database-url <url>`. Omitted, the default `sqlite:///` + `~/.local/share/cafleet/cafleet_<VERSION>.db` applies, `<VERSION>` being the release version (§7.6), e.g. `0.31.1`.

The default expands HOME once. An explicit URL has the form `sqlite:///<path>`; resolve a relative file path against the invoking working directory before canonicalizing the database file for shared pane-lock identity. A literal tilde in an explicit URL is not expanded.

CAFleet reads no `CAFLEET_*` environment variable. Member creation renders the selected database into the spawn prompt (§ Spawn-prompt substitution), and a member whose `DATABASE URL:` line names a URL passes `--database-url <url>` on every `cafleet` command. A member that omits the option on a custom-database fleet reaches the default database: when that file is absent the command fails with `no cafleet database; run 'cafleet setup'`, and when it exists its literal IDs may match another fleet. This is the same cooperative limit as the Director-only command guard (§ Director-only command guard).

#### Global Options

`cafleet --version` prints `cafleet <version>` and exits 0 before dispatch; `--database-url` follows § Database selection.

#### Help text

`--help` on any command prints its usage line, its subcommands and its options, each with a one-line description, to stdout and exits 0 without opening the database. Its layout is non-normative except for the strings this specification quotes, such as the `--permission-mode` description. Command group descriptions state the purpose given in the subcommand summary. Help derives choice lists from the same registries that validate the values:

| Option | Help shows | Source |
|---|---|---|
| `member create --permission-mode MODE` | Each backend's accepted modes and its omitted default, in backend registry order, as `BACKEND: MODE, MODE (default MODE)` joined by `; ` | The backend policy catalog (§6.9 Launch policy and capability registry) |
| `permission respond --decision` | Its possible values | The decision enum of §6.3 Permission commands |
| `input reconcile --disposition` | Its possible values | Its command enum |

An invalid value for an option with possible values exits 2 with a usage error that lists those values. `member create` resolves its backend at run time, so a `--permission-mode` value outside that backend's catalog keeps its run-time usage error `--permission-mode for BACKEND must be one of CHOICES (got 'VALUE')`, also with exit 2. Help reads `Launch-only native permission policy for the resolved backend — claude: default, manual, dontAsk, acceptEdits, auto, bypassPermissions, plan (default auto); codex: on-request, never, approve-for-me (default approve-for-me); opencode: cafleet, cafleet-ask (default cafleet-ask); hermes: smart (default smart); pi: cafleet, cafleet-ask (default cafleet-ask)`.

#### Output shapes

| Command | Text | JSON |
|---|---|---|
| `fleet create` | `<fleet-id> director=<member-id>` | Ordered `fleet_id, name, created_at, director`, with Director identity/description/registration/placement |
| `fleet list` | `FLEET_ID DIRECTOR NAME MEMBERS CREATED_AT` columns | Fleet records |
| `fleet show` | Fleet row with `deleted_at:` | Fleet including nullable `deleted_at` |
| `fleet delete` | `Deleted fleet ID. Deregistered N members.` plus one line per ordinary member | `{fleet_id, deregistered_count, members}` |
| `message send` | `Message sent.` then envelope | `{message}` |
| `message poll` | Envelopes, or `No messages found.` | Full record array |
| `message show` | Envelope | `{message}` |
| `message listen` | — | Tagged `ready`, `pending`, `closed` JSONL records; no `--json` flag |
| `member create` | `ID NAME backend=AGENT pane=PANE window=WINDOW` plus runtime line | Member with placement and runtime |
| `member delete` | `Member deleted.` with member/pane lines | `{member_id, pane_status}` |
| `member show` | `ID NAME STATUS window=WINDOW` plus wrapped runtime line | Detail including kind, placement and runtime |
| `event show` | Event, runtime and optional interaction lines | Event and current state |
| `input show` / `input reconcile` | Input receipt and optional decision lines | `{input}` |
| `permission inspect` | Request identity/capability lines | `{request}` |
| `permission respond` | Answer submission status/stage line | `{response}` |
| `member list` | Roster rows or `0 members.` | Roster array |
| `member prompt` | `Sent prompt TEXT_REPR to member NAME (PANE).` or the queued line, plus delivery and input lines | `{member_id, pane_id, text, input_id, delivery}` |
| `member exec` | `Dispatched native command to member ID (PANE). Inspect the pane for its result.` or the queued line, plus delivery and input lines | `{member_id, pane_id, coding_agent, command, dispatched, input_id, delivery}` |
| `member press` | `Acknowledged keys KEYS_JSON for member ID (PANE).` | Ordered `member_id, pane_id, keys, status` |
| `member capture` | Exact selected content | `{member_id, pane_id, lines, content, captured_at, content_sha256}` |
| `doctor` | One `STATUS NAME: DETAIL` row per check | Array of check objects |

##### Member list output

Roster records include `member_id`, `name`, `kind`, `placement`, `last_sent`, `last_recv`, `last_consumed` and `idle`. `last_consumed` is the latest `status_timestamp` of the member's completed incoming messages, the time its poll consumed them. `kind` follows §5.4 Member kind discriminator. Missing activity is null; the placement pane ID may be pending. Text uses `-` for absent values and `(pending)` for a pending window or pane.

Every JSON `placement` object, in roster, `member create`, `member show` and `fleet create` output, has ordered keys `backend, mux_session, mux_window_id, mux_window_name, window_ordinal, column_side, mux_pane_id, coding_agent, created_at`; `mux_window_id` and `mux_window_name` are null exactly when `mux_pane_id` is null.

Text rows use the format `"  {:<9}  {:<13}  {:<8}  {backend:<8}  {window:<13}  {pane:<7}  {}"` for member ID, name, kind, backend, window name, pane ID and idle:

```text
2 members:
  member_id  name           kind      backend   window         pane_id  idle
  8          director       director  claude    main           %3       12s
  9          drafter        member    claude    cafleet-3-1    %12      5s
```

`member create` and `member show` text render `WINDOW` as the placement's `mux_window_name`, or `(pending)` while the placement is pending.

#### JSON output (`--json`)

Place `--json` among the subcommand options, before a `--` separator that introduces literal key arguments. It emits one line of JSON (§6.4 JSON encoding) with complete untruncated message text. Application errors are stderr text, and a parse error exits 2 (§6.3 Error Messages). Commands marked no reject the flag, including the root group.

#### Positional subject ids

Place the command’s subject ID immediately after its subcommand; event, input and permission subjects use their positive durable event, input and interaction IDs. The subject row supplies related fleet/recipient state. IDs are complete DB-assigned integers, without prefix resolution or environment defaults; a Director-only command also takes its caller's literal ID as `--from-member-id` (§ Director-only command guard). A missing or malformed subject is a parse error (§6.3 Error Messages).

Spawned members read literal `FLEET ID:`, `YOUR MEMBER ID:`, `DIRECTOR MEMBER ID:`, `CODING AGENT:` and `DATABASE URL:` lines, pass these literal IDs on each invocation and apply `DATABASE URL:` per § Database selection.

#### Sender and recipient (`--from-member-id`, `--to-member-id`)

Send requires integer sender and recipient flags, validated per §6.2 Messaging; the sender row determines the fleet. IDs are coordination identifiers within the local trust model.

#### Director-only command guard

The Director-only commands are exactly `member prompt`, `member exec` and `member press` (explicit native pane input), `permission respond` (native permission answers) and `input reconcile` (input-receipt settlement), which write into a member's live native session or its runtime record. Every other subcommand, including `permission inspect`, `input show` and `event show`, is unguarded; the lifecycle commands `member create`, `member delete`, `fleet create` and `fleet delete`, roster listing (`member list`) and captures (`member capture`) remain Director-owned by role.

Each requires `--from-member-id <director-member-id>`, a positive integer: the caller's ID, or the pinned Director for `permission respond`. A missing or malformed value is a parse error, exit 2.

After argument parsing and the invocation connection's schema-version guard (§ Schema-version guard), and before any multiplexer, lock, socket or file access or write, the command resolves its subject's fleet — a member's own fleet, or the fleet of the run that owns an input or interaction — and requires that fleet to be live with `director_member_id` equal to `--from-member-id`. An unknown subject's not-found error precedes this check. Otherwise the command exits 1, prints nothing on stdout and writes this application error to stderr, `permission respond` included (no response attempt began, so the §6.3 Permission commands structured-response exception does not apply):

```
Error: 'cafleet <command>' is a Director-only command and member <id> is not the root Director of fleet <fleet_id>; send the request to your Director.
```

A Director-only command dispatched into a member pane with `member exec` is checked by the ID it carries.

The guard is cooperative, not a security boundary: it stops a member that passes its own `YOUR MEMBER ID:` value and a call without an ID (a parse error), but a member that copies its `DIRECTOR MEMBER ID:` value passes it. Member authority remains defined by the member role and the Director's assignment.

#### `permissions.allow` coverage

Prefix rules such as `Bash(cafleet message *)` cover broker operations, including `cafleet message listen ID --timeout 300`. Native Monitor authorization is separate: a Bash-rule check establishes no Monitor permission. Literal IDs and trailing JSON flags preserve prefix matching.

Ask entries for the Director's pane-input prefixes follow §6.7 Claude permission posture, which states the permission-profile contract.
`cafleet permission respond` runs under the Director host's effective policy after its request-specific decision and bypasses no ask, deny or managed rule. Ordinary member role authority covers broker reception and evidence routing, while the Director owns permission responses.

#### Message Body Truncation

CLI text output truncates only message bodies, to a fixed 200 codepoints per §6.4 Text-mode projection; `--json` carries full bodies, pane captures preserve their complete content, and pane notification lines follow §6.5 Push notifications.

#### Text bodies (positional `TEXT` / `--file`)

Send and member creation require exactly one positional body or `--file PATH`; `--file -` reads UTF-8 stdin. Preserve exact nonblank body text. Missing/both sources or a blank positional body exit 2; blank file/stdin sources and file-read errors exit 1 with the authored diagnostics below. File read/UTF-8 failures retain the actual cause.

`member prompt` takes one positional line. Native exec has the separate command contract (§6.3 `member exec`).

#### `cafleet setup` — Onboarding and Schema Management

`cafleet setup [--coding-agent AGENT...]` prepares the database first, then installs assets. It classifies the selected database before any write per §6.3 Schema-version guard, leaving a current file of this version unchanged. Compatible setup then installs embedded assets offline. Backend selection is multi-value/repeatable, deduplicated and defaults to the supported backend registry; selected backends install in registry order.

After initializing, setup prints `Initialized database at {path}.`; a database that is already current prints `Database already initialized at {path}.`. Asset installation stops at the first backend whose config-dir resolution or installation fails: backends earlier in registry order keep their installation, and setup exits 1 with that backend's cause.

##### Config-dir resolution

Every surface that installs to or checks a backend's user-level config directory resolves its base from the backend's native config-location environment variable, else the default:

| Backend | Base | Skills dir | Preset target |
|---|---|---|---|
| claude | `$CLAUDE_CONFIG_DIR`, else `~/.claude` | `<base>/skills` | — |
| codex | `$CODEX_HOME`, else `~/.codex` | `<base>/skills` | `<base>/rules/cafleet.rules` |
| opencode (skills) | `~/.config/opencode` (fixed discovery path) | `<base>/skills` | — |
| opencode (preset) | `$OPENCODE_CONFIG_DIR`, else `~/.config/opencode` | — | `<base>/agents/cafleet.md`, `<base>/agents/cafleet-ask.md` |
| hermes | `$HERMES_HOME`, else `~/.hermes` | `<base>/skills` | — (setup installs the `cafleet` plugin under `<hermes home>` and enables it in the Hermes configuration; § Asset installation) |
| pi | `$PI_CODING_AGENT_DIR`, else `~/.pi/agent` | `<base>/skills` | — (setup installs the CAFleet pi extension under `<base>/extensions`; § Asset installation) |

OpenCode discovers skills only at the fixed path but loads agent definitions from `OPENCODE_CONFIG_DIR`, so only its presets follow the variable.

**Validation.** A set variable must hold an absolute path; any other value (the empty string, a relative path, a literal unexpanded `~/…`) fails at resolution time with exit 1 and `Error: <VAR> must be an absolute path (got '<value>')`. A set config-location variable or `HOME` whose value is not valid UTF-8 fails at the same point with exit 1 and `Error: <VAR> is not valid UTF-8` instead of selecting the default.

Validation is lazy: a site reads and validates a variable only when it resolves that backend's directory. `cafleet setup --coding-agent claude` with an invalid `CODEX_HOME` succeeds because the selector resolves only the targeted agent's directory; plain `cafleet setup` resolves each backend in registry order, so an invalid variable stops installation at that backend. `member create` resolves only the selected backend's directory during its availability check and forwards the validated value into the member environment (§6.3 Member environment). `doctor` reports a per-agent resolution error as that agent's `<agent>.configuration` check instead of aborting (§6.3 `cafleet doctor` — Environment Diagnosis).

##### Asset installation

The binary carries the data files of the `assets/` tree listed below. The native screen fixtures are catalog provenance records: the bundled catalog cites them, and setup installs none of them.

| Source tree | Contents | Consumer |
|---|---|---|
| `assets/skills/` | One directory per skill, embedded file by file | Skill installation (this section) |
| `assets/presets/` | One directory per backend with presets | Preset installation (table below) |
| `assets/extensions/` | One directory per backend with an extension | Extension installation (table below) |
| `assets/migrations/` | The schema baseline `V1__baseline.sql` | Baseline initialization (§8 Database schema) |
| `assets/profiles/` | The embedded bundled catalog `bundled.json`, and the native screen fixtures `native/<backend>.json` that it cites | The profile catalog (§6.9 Bundled profiles and native screen fixtures) |

Each agent's presets and extensions, where they exist, are static files embedded in the binary next to the skills; their targets resolve per §6.3 Config-dir resolution. Per backend, setup replaces every embedded skill sequentially, then each preset in table order, then each extension target followed by its executable record, then (for `hermes`) runs the plugin enablement:

| Agent | Bundled presets → install target | Bundled extension → install target | Executable record |
|---|---|---|---|
| claude | — | — | — |
| codex | `assets/presets/codex/cafleet.rules` → `<codex base>/rules/cafleet.rules` | — | — |
| opencode | `assets/presets/opencode/cafleet.md` → `<preset base>/agents/cafleet.md`; `assets/presets/opencode/cafleet-ask.md` → `<preset base>/agents/cafleet-ask.md` | `assets/extensions/opencode/cafleet.ts` → `<preset base>/plugins/cafleet.ts` | `<preset base>/plugins/cafleet.executable` |
| hermes | — | `assets/extensions/hermes/cafleet/` (every file) → `<hermes home>/plugins/cafleet/` | `<hermes home>/plugins/cafleet/executable` |
| pi | — | `assets/extensions/pi/cafleet.ts` → `<pi base>/extensions/cafleet.ts` | `<pi base>/extensions/cafleet.executable` |

Existing targets are deleted before their replacements are written; a symlink target is unlinked as an entry, keeping its referent. Unrelated skills and plugins remain untouched.

`<preset base>` is the opencode preset base (`$OPENCODE_CONFIG_DIR`, else `~/.config/opencode`). `<hermes home>` is the parent directory of the `<config path>` that `hermes config path` prints (§ Hermes plugin enablement step 1); Hermes discovers user plugins under its home, and a sticky `<base>/active_profile` re-homes both that directory and `hermes config path` to the profile directory. Setup for `hermes` runs `hermes config path` once, in the read batch before the extension install, so installing the Hermes extension requires `hermes` on `PATH`. `<pi base>` is the pi base (`$PI_CODING_AGENT_DIR`, else `~/.pi/agent`); setup for `pi` runs no `pi` command, so it needs no `pi` on `PATH`. Extension files are embedded verbatim, so the embedded bytes equal the installed bytes.

The *executable record* is a one-line file holding the absolute path of the running `cafleet` executable as the OS reports it (Linux: the target of `/proc/self/exe`; macOS: the kernel-reported image path), with every symbolic link resolved, written after the extension files. It is neither an embedded asset nor byte-compared; OpenCode plugin discovery loads only `.ts`/`.js` files and directories, and pi extension discovery loads only `*.ts`, `*.js`, `*/index.ts` and `*/package.json` entries, so both ignore the record. The plugins and extensions execute only the recorded path.

Skills-operation errors retain `failed to install skills into <skills_dir>: <error>`, preset-operation errors `failed to install preset into <target>: <error>`, and extension-operation errors, including the executable record, `failed to install extension into <target>: <error>`. A failure on any preset, extension or plugin enablement step fails setup.

Setup prints each success line after its operation completes: the skills line, one preset line per installed preset target in install order, then one extension line per extension target once the target and its executable record are written; a directory extension prints its directory once. Claude prints only the skills line; pi prints the skills line and the extension line; Hermes prints the skills line, the extension line and a plugin enablement line (§ Hermes plugin enablement):

```
<agent>: installed cafleet, cafleet-design-doc (v<version>) -> <skills dir>
<agent>: installed preset (v<version>) -> <target>
<agent>: installed extension (v<version>) -> <target>
```

`cafleet setup --coding-agent hermes` on a home whose plugin is already enabled prints:

```
hermes: installed cafleet, cafleet-design-doc (v<version>) -> <skills dir>
hermes: installed extension (v<version>) -> <hermes home>/plugins/cafleet
hermes: plugin cafleet already enabled -> <config path>
```

`cafleet setup --coding-agent pi` prints:

```
pi: installed cafleet, cafleet-design-doc (v<version>) -> <pi base>/skills
pi: installed extension (v<version>) -> <pi base>/extensions/cafleet.ts
```

###### Hermes plugin enablement

Setup for `hermes` runs every Hermes command through the local `hermes` binary, with `HERMES_HOME` set to the resolved base on every call, and parses no `config.yaml` itself. Under the Hermes home it touches exactly the skills, `<hermes home>/plugins/cafleet/`, `plugins.enabled`, `plugins.disabled` (read only) and `plugins.entries.cafleet.allow_gateway_injection`.

A key is *not set* when its `hermes config get` exits 1 and one trimmed stderr line equals exactly `Config key not set: <key>`; the rule matches a line, not the whole stream, because Hermes prints unrelated stderr lines, such as an install out-of-sync warning, on every command. Any other non-zero exit is a failed command whose cause is its stderr text.

`hermes config get --json plugins` prints the resolved `plugins` subtree as a JSON object. A missing `disabled`, `enabled` or `entries.cafleet.allow_gateway_injection` member is that key's *not set* state, and a *not set* `plugins` key means all three are *not set*. A present `disabled` or `enabled` member must be a JSON array of strings.

After installing the skills, setup for `hermes` runs:

1. One concurrent read batch under one ten-second deadline: `hermes config path` and `hermes config get --json plugins`. `<config path>` is the trimmed stdout of `config path`. Hermes applies a sticky `<base>/active_profile` to every command, so this path, the extension's `<hermes home>`, the plugin keys, the member's `hermes config path` (§6.7 Hermes availability preflight) and the member's own `hermes chat` name the same file and home. A `config path` failure, evaluated before the `plugins` result, fails setup with `failed to install extension into <resolved Hermes base>: <cause>`, `<resolved Hermes base>` being `$HERMES_HOME`, else `~/.hermes`.
2. Install the extension into `<hermes home>/plugins/cafleet/` (§ Asset installation) and print the extension line.
3. Evaluate `plugins`. When `disabled` lists `cafleet`, setup fails with the cause `plugins.disabled lists cafleet; remove it from plugins.disabled, then rerun 'cafleet setup --coding-agent hermes'`: the deny-list wins over `plugins.enabled`, preserving the operator's explicit disable. A *not set* `enabled` is an empty list. When `cafleet` is missing from `enabled`, run `hermes config set plugins.enabled '<existing entries in order, then "cafleet", as compact JSON>'`. When `allow_gateway_injection` is *not set* or not `true`, run `hermes config set plugins.entries.cafleet.allow_gateway_injection true`. The writes run serially, each under its own ten-second deadline, because each rewrites `config.yaml`.
4. Only when step 3 wrote, re-read `hermes config get --json plugins` once under its own ten-second deadline and require `cafleet` in `enabled` and `true` for `allow_gateway_injection`, because a `config set` can exit 0 without the value sticking under a managed scope.
5. Print `hermes: plugin cafleet already enabled -> <config path>` when step 3 wrote nothing, otherwise `hermes: enabled plugin cafleet -> <config path>`.

Any failed command, a timeout, an unexpected value or a listed `plugins.disabled` entry after step 1 fails setup with `failed to enable hermes plugin cafleet in <config path>: <cause>`; Hermes's writer refusing a key pinned by its managed scope is such a cause.

Hermes's ruamel round-trip writer replaces the file atomically, writing through a symlinked `config.yaml` to its target and preserving comments, key order, quoting and untouched nodes. A rerun after a successful enablement writes nothing.

A missing `plugins.enabled` enables no user plugin, and Hermes grandfathers installed user plugins only while the key is missing; after setup writes it, other installed but not enabled user plugins stay disabled until the operator enables them. Setup never runs `hermes plugins enable`, whose dependency admission re-syncs the Hermes environment, which the plugin does not need and which fails on installs without `uv.lock`.

Setup runs only the local binary and stays offline. `cafleet setup --coding-agent hermes` requires `hermes` on `PATH`; its absence fails with the cause `binary hermes not found on PATH`. Plain `cafleet setup` with `hermes` absent from `PATH` installs the Hermes skills, skips the extension and the plugin enablement and prints:

```
hermes: skipped the plugin (binary hermes not found on PATH); run 'cafleet setup --coding-agent hermes' after installing Hermes
```

#### Schema-version guard

Every fleet, member, message, event, input and permission command validates the current format before any other database read or write. The classifier runs the §8 Database schema compatibility diagnosis; setup, doctor and every guarded command share it, and the check and setup work reuse the invocation's connection. Setup initializes a missing file, or an empty SQLite file with identity zero and no user tables, with the baseline atomically; every other guarded command refuses such a file with `no cafleet database; run 'cafleet setup'`. Every other populated, identified or malformed file, including one recorded by another cafleet version, fails before database mutation, asset installation or asset-install recording with `database format is incompatible at 'PATH'; choose an empty database path and run 'cafleet setup'.` Preserve database open/read causes, incompatible file bytes and existing sidecars. Doctor reports the classification rather than blocking, completing its other read-only diagnostics even when the database is unavailable.

`--help` renders at parse time and exits before any command body runs, so neither group-level help (`cafleet fleet --help`) nor subcommand help (`cafleet fleet create --help`) triggers the guard; both print help and exit 0 even under a missing database.

#### `cafleet fleet` — Fleet Management

##### `fleet create`

`cafleet fleet create --name NAME --coding-agent AGENT [--pane PANE_ID [--mux-server SERVER]] [--json]`

`fleet create` and `doctor`'s `multiplexer` and `multiplexer.caller_pane` rows are the only surfaces that read the caller's multiplexer environment, and both verify the caller's pane against its process ancestry (§ Caller pane verification). Require the selected backend token, then resolve each input, first match wins:

| Input | Source |
|---|---|
| Backend | With `--pane`, its ID form (§ `--pane` ID form); else `TMUX`/`HERDR_ENV` auto-detection (§6.5 Backend selection) after the shared-server check, where a detection failure exits 1 with `cafleet fleet create must be run inside a tmux or herdr session` |
| `mux_server` (tmux) | `--mux-server`; else everything before the last two commas of a non-empty `TMUX`, so a socket path containing `,` is kept whole; else `env -u TMUX tmux display-message -p '#{socket_path}'` (the default server). With `--pane`, these environment sources apply only when the detected backend equals the pane's (§ `--pane` ID form) |
| `mux_server` (herdr) | `--mux-server`; else a non-empty `HERDR_SESSION`; else the `herdr session list` row whose socket equals `HERDR_SOCKET_PATH`; else `default`. With `--pane`, these environment sources apply only when the detected backend equals the pane's (§ `--pane` ID form) |
| Director pane | `--pane`, an operator assertion checked only for existence on the chosen server; else a non-empty `TMUX_PANE` (tmux) or `HERDR_PANE_ID` (herdr), verified against the caller's process ancestry (§ Caller pane verification) |

Read the pane on the chosen server — `tmux -S <server> display-message -p -t <pane> '#{session_name}|#{window_id}|#{pane_id}'` or `herdr --session <server> pane current --pane <pane>` — to resolve its concrete session, window and pane IDs (§6.5 Multiplexer context). tmux prints `||` and exits 0 for a missing pane, so a reply with an empty field, or whose `pane_id` differs from the requested pane, means the pane was not found. Target resolution and this initial pane-context read precede registration and naming. `cafleet doctor` reads only pane context and process information. Each failure below exits 1 before any write:

| Condition | Error |
|---|---|
| tmux finds no server | `Error: no tmux server found; pass --mux-server <socket path>` |
| The pane is not on the chosen server | `Error: pane <pane-id> was not found on the <backend> server <server>; pass --pane and --mux-server for the Director's pane` |
| `--pane` is neither ID form | `Error: --pane <pane> is neither a tmux pane ID (%N) nor a herdr pane ID (<workspace>:<pane>)` |
| Without `--pane`, an ancestor is a shared server | `Error: cafleet fleet create ` followed by the shared-server body (§ Refusal messages) |
| Without `--pane`, `TMUX_PANE` (tmux) or `HERDR_PANE_ID` (herdr) is unset | `Error: cafleet fleet create ` followed by the unset body (§ Refusal messages) |
| Without `--pane`, the env pane fails verification | `Error: cafleet fleet create ` followed by the unverified body (§ Refusal messages) |

| Option | Help |
|---|---|
| `--pane` | `The Director's pane; its tmux %N or herdr <workspace>:<pane> form selects the multiplexer [default: the caller's TMUX_PANE or HERDR_PANE_ID, verified against the caller's process ancestry]` |
| `--mux-server` | `The Director's tmux socket path or herdr session [default: the caller's server on the pane's multiplexer]` |

`--pane`, optionally with `--mux-server`, is the only override of caller pane verification.

Registration follows the §6.2 Fleets bootstrap transaction. After allocating the fleet ID, overwrite the resolved Director window/tab name with `cafleet-<fleet_id>-0` through the Rename window operation, read it back and require exact equality (§6.5 Backend operations). Re-read the Director pane context and require the original session, window ID and pane ID. Record the canonical name as `mux_window_name`, construct the result before commit, and emit success only after confirmed commit. The Director has `name="Director"` and `description="Root Director for this fleet"`; its pane title/label retains its current value.

Any naming, verification, insertion or result-construction failure exits 1 and rolls back active uncommitted registration, with no success payload even under `--json`. Keep the registration transaction open until the commit result is known: a failed commit rolls back an active transaction, or reports the registry outcome as unconfirmed if SQLite already ended it. Append a rollback failure to the original error. Once naming is attempted, diagnostics identify the attempted fleet ID, backend/server, concrete window and pane IDs, requested name and cause, describe the terminal name as potentially changed, and direct inspection through `fleet list` and explicit-server multiplexer commands. A rolled-back fleet ID can be reused.

Applied names remain after a failure, crash or fleet deletion, and the Director pane keeps running. The rename and database commit are separate operations: a crash can leave a changed name without registration, or committed registration without delivered output. An output error after confirmed commit preserves registration and the name. Existing fleets keep their recorded names, and subsequent user renames leave placement snapshots unchanged.

Run `fleet create` from the coding-agent session declared by `--coding-agent`; root-Director reception follows §6.3 `message listen` and §6.5 Push notifications.

##### Caller pane verification

A coding agent whose tool commands run in a long-lived shared server passes that server's environment to `cafleet`, and that environment can name another pane or another multiplexer. `fleet create` therefore trusts `TMUX_PANE` / `HERDR_PANE_ID` only when the pane's process is an ancestor of the `cafleet` process, and `doctor` reports the same verification (§ `cafleet doctor` — Environment Diagnosis).

Without `--pane`, `fleet create` completes the caller checks before registration or naming, then runs the registration sequence:

1. **Shared-server check.** Walk the caller's ancestry (§ Ancestry walk) and apply row 1 (§ Verification rows), before any multiplexer command; the walk itself runs none.
2. **Backend.** Detect the backend (§6.5 Backend selection).
3. **Env pane.** Read a non-empty `TMUX_PANE` (tmux) or `HERDR_PANE_ID` (herdr); an unset variable fails with the unset body (§ Refusal messages).
4. **Server.** Resolve the server and check availability (§ `fleet create`), keeping their error messages.
5. **Verification.** Apply rows 2–7.
6. **Registration.** Read the pane context, allocate the fleet ID, name and verify its window, then commit registration (§ `fleet create`).

With `--pane`, `fleet create` reads no process entry: the pane's ID form selects the backend, the server follows § `--pane` ID form, availability is checked, then step 6 runs.

###### Ancestry walk

The walk starts at the `cafleet` process's own pid and is complete at pid 1. At any other pid it reads the process entry:

| Entry | Walk |
|---|---|
| The process no longer exists | Stops at that pid |
| The read fails | Ends unreadable at that pid, carrying the whole read error as its cause |
| Parent `0` | Records the pid and argv, then stops at that pid |
| Any other parent | Records the pid and argv, then continues at the parent |

A walk that records 4096 entries stops at the last recorded pid.

| Platform | Parent and argv | Nested PID namespace |
|---|---|---|
| Linux | `/proc/<pid>/stat` field 4 (the second field after the last `)`) is the parent, and `/proc/<pid>/cmdline` split on NUL is the argv, with a trailing empty segment dropped and an empty `cmdline` giving an empty argv. An `ENOENT` (file not found) read of either file means the process no longer exists | `/proc/self/status` `NSpid:` lists more than one pid; no `NSpid:` line means not nested |
| Other (macOS) | `ps -o ppid= -o args= -p <pid>` with null stdin: the first whitespace field is the parent and the remaining fields are the argv. Output with no non-blank line means the process no longer exists | Never nested |

| Reader | Condition | Error |
|---|---|---|
| Linux entry | A read error other than `ENOENT` (file not found); `<file>` is `stat` or `cmdline` | `cannot read /proc/<pid>/<file>: <io error>` |
| Linux entry | The parent field is absent or not an integer | `malformed /proc/<pid>/stat` |
| Linux namespace | Reading `/proc/self/status` fails | `cannot read /proc/self/status: <io error>` |
| Linux namespace | An `NSpid:` field is not an integer | `malformed /proc/self/status NSpid line: <line>` |
| Other entry | `ps` cannot start | `cannot run ps: <io error>` |
| Other entry | The first field is absent or not an integer | `ps printed no parent for <pid>: <line>` |
| Other entry | No field follows the parent | `ps printed no command for <pid>: <line>` |

The §6.3 Codex forwarding session pin reads entries through the same reader and propagates its errors.

###### Verification rows

The first matching row decides. Row 1 runs right after the walk, and rows 2–7 run after server availability:

| # | Condition | Result |
|---|---|---|
| 1 | A recorded ancestor's argv matches a shared-server signature of any backend (§6.7 Shared-server signatures), nearest to the caller first | Refused with the shared-server body |
| 2 | The pane process read (§6.5 Backend operations) finds no such pane on the server | Refused with the unverified body |
| 3 | The caller runs in a nested PID namespace | Refused with the unverified body |
| 4 | The pane's pid equals a recorded ancestor pid | Verified |
| 5 | The walk ended unreadable | Refused with the unverified body |
| 6 | The walk stopped | Refused with the unverified body |
| 7 | Otherwise (the walk completed) | Refused with the unverified body |

Row 1 runs first because a daemon started from pane A can serve a session in pane C while pane A's process is still an ancestor, so row 4 alone would accept pane A. It also refuses a cross-multiplexer caller before its stale environment selects a backend or server. Row 3 precedes row 4 because pids inside a nested namespace are not comparable with the multiplexer's host pids. A pane-process command failure and a nested-namespace read error propagate as errors, not refusals.

###### `--pane` ID form

| `--pane` form | Backend |
|---|---|
| `%` followed by one or more ASCII digits (e.g. `%12`) | tmux |
| Exactly one `:` with non-empty text on both sides and no leading `%` (e.g. `w4:p3N`) | herdr |
| Anything else | The `--pane` form error (§ `fleet create`) |

The server comes from the first matching row. An ambiguous environment (both `HERDR_ENV` and `TMUX` set) is not an error under `--pane`, because the pane form already names the backend:

| Environment's backend detection (§6.5 Backend selection) | Pane's backend | Server |
|---|---|---|
| Any result | Either | `--mux-server` when given |
| Yields the pane's backend | Either | The environment sources of § `fleet create` |
| Fails as ambiguous or absent, or yields the other backend | tmux | The output of `env -u TMUX tmux display-message -p '#{socket_path}'` (the default server); a failure or empty output exits 1 with `Error: no tmux server found; pass --mux-server <socket path>` |
| Fails as ambiguous or absent, or yields the other backend | herdr | `default`, with no command |

###### User-run pane identification

Each refusal body names a command the user runs in the Director pane. It reports both the pane ID and the server, because the caller environment's server is as untrusted as its pane:

| Multiplexer | Guidance text | Reply |
|---|---|---|
| tmux | `the tmux command prompt (the user presses the tmux prefix key in the Director pane, then types :display-message -d 0 "#{pane_id} #{socket_path}" and Enter; the status line shows the pane ID, then the server socket)` | Pane ID (e.g. `%12`) and socket path (e.g. `/tmp/tmux-1000/default`) |
| herdr | `! env -u HERDR_PANE_ID herdr --session <session> pane current --current (typed by the user into the Director's composer while the Director pane is focused; <session> is the herdr session the user attached, default unless herdr was started with --session <name>, and herdr session list shows the running sessions; the pane ID is result.pane.pane_id and the server is <session>)` | Pane ID (e.g. `w4:p3N`) and session name |

The tmux command prompt evaluates both formats in the attached client's current pane without a shell. The herdr command asks the named session's server for its focused pane, which is the pane the user is typing in: it unsets `HERDR_PANE_ID` because a set value overrides `--current`, and its explicit `--session` overrides `HERDR_SESSION` and `HERDR_SOCKET_PATH`. CAFleet never fills `<session>` from the caller environment.

###### Refusal messages

`fleet create` prefixes each body with `cafleet fleet create `, and `doctor` prints the body alone. Every body ends with `then run cafleet fleet create with --pane <pane-id> --mux-server <server>`:

| Body | Text |
|---|---|
| Shared-server | `cannot verify this session's pane: a shared <label> server (pid <pid>) runs this command, so its multiplexer environment can belong to another pane; ask the user to read the Director pane ID and server with <tmux guidance> in tmux, or with <herdr guidance> in herdr, then run cafleet fleet create with --pane <pane-id> --mux-server <server>` |
| Unverified | `cannot verify that <VAR>=<pane> is this session's pane: <cause>; ask the user to read the Director pane ID and server with <guidance>, then run cafleet fleet create with --pane <pane-id> --mux-server <server>` |
| Unset | `cannot identify its <backend> pane because <VAR> is not set; ask the user to read the Director pane ID and server with <guidance>, then run cafleet fleet create with --pane <pane-id> --mux-server <server>` |

The shared-server body names both multiplexers because a shared server's environment can name the wrong one. The other bodies follow a shared-server-free ancestry, so `<VAR>` and `<guidance>` belong to the detected backend.

| Row | `<cause>` in the unverified body |
|---|---|
| 2 | `the pane does not exist on the <backend> server <server>` |
| 3 | `this command runs in a nested PID namespace` |
| 5 | `the process ancestry could not be read: <read error>` (e.g. `… could not be read: cannot read /proc/412/stat: <io error>`, where `<io error>` is the OS description of `EACCES`) |
| 6 | `the process ancestry stopped at pid <pid> before reaching pid 1` |
| 7 | `the pane's process <pane_pid> is not an ancestor of this command` |

Example (a Codex Director whose tool commands run under the shared daemon):

```
Error: cafleet fleet create cannot verify this session's pane: a shared codex app-server server (pid 407596) runs this command, so its multiplexer environment can belong to another pane; ask the user to read the Director pane ID and server with the tmux command prompt (the user presses the tmux prefix key in the Director pane, then types :display-message -d 0 "#{pane_id} #{socket_path}" and Enter; the status line shows the pane ID, then the server socket) in tmux, or with ! env -u HERDR_PANE_ID herdr --session <session> pane current --current (typed by the user into the Director's composer while the Director pane is focused; <session> is the herdr session the user attached, default unless herdr was started with --session <name>, and herdr session list shows the running sessions; the pane ID is result.pane.pane_id and the server is <session>) in herdr, then run cafleet fleet create with --pane <pane-id> --mux-server <server>
```

##### `fleet list`

List non-deleted fleets by descending creation timestamp/fleet ID. Rows retain the Director reference so its ID remains discoverable.

##### `fleet show`

`cafleet fleet show FLEET_ID [--json]` includes soft-deleted rows. Unknown ID: `fleet 'X' not found.`, exit 1.

##### `fleet delete`

`cafleet fleet delete FLEET_ID [--json]` ends every active ordinary member that owns a concrete pane, then deregisters the fleet. It retains messages and is idempotent; an unknown ID fails with `fleet 'X' not found.`, exit 1. One operation-wide five-second deadline bounds the whole command, extended by the time each Hermes work settlement takes (§6.3 `member delete`).

1. **Lock.** Lock every active concrete placement in sorted key order and revalidate the placement set, retrying contention every 50 milliseconds; a set change releases every acquired lock and restarts within the same deadline. Contention until the deadline releases every acquired lock, leaves every pane and registry row unchanged and exits 1 with `fleet ID pane input remained busy for 5 seconds at member MEMBER_ID; fleet was not deleted.` After all pane locks, acquire the fleet layout lock once within the layout lock wait (60 s) (§6.5 Fleet window layout), moving the deadline later by the wait's duration; expiry exits 1 with the fleet delete busy error (§6.3 Error Messages) and leaves every member unchanged.
2. **End members.** Under the held locks, end each active ordinary member with a concrete pane, in lock-key order, through the `member delete` procedure with the held layout lock: runner shutdown and recovery for a wrapped member, pane close in the placement's recorded window, that member's deregistration, then optional layout restoration of that window. The root Director's pane stays open.
3. **Deregister the fleet.** One transaction revalidates the placement set, stamps deletion, deregisters the remaining active members (the root Director and members with a pending placement) and removes their placements.

The first member that cannot be ended stops the command, and the fleet is not deleted. Members ended before it stay deregistered with closed panes; the failing member and every later member keep their registry rows and panes, so `member list` still shows them. The command exits 1 with `fleet ID was not deleted: member MEMBER_ID: CAUSE. Members already deleted: IDS.`, where `CAUSE` is the member deletion cause and `IDS` is a comma-separated id list or `none`. A placement-set change after cleanup began exits 1 with `fleet ID placement or runtime ownership changed during cleanup; fleet was not deleted.` Rerunning `fleet delete` resumes with the members that remain.

On success the text output is `Deleted fleet ID. Deregistered N members.` followed by one `  member MEMBER_ID: PANE_STATUS` line per ordinary member in ascending id. `N` counts every member the command deregistered, root Director included; `PANE_STATUS` takes the `member delete` values `PANE (killed)` and `(pending — no pane)`. JSON is ordered `fleet_id, deregistered_count, members`, with one `{member_id, pane_status}` entry per ordinary member in the same order.

#### `cafleet doctor` — Environment Diagnosis

A full-environment diagnosis that reports **every** check, with no early abort when the multiplexer is unavailable. `doctor` is exempt from the schema-version guard (§6.3 Schema-version guard), so a missing or incompatible database is reported, not fatal. Its optional flag is `--json` (§6.3 JSON output (`--json`)).

Text output prints one `<status> <name>: <detail>` row per check in check order (§ Check rows), with each LF in the detail escaped as `\n` and each CR as `\r`. `--json` prints one JSON array of check objects in the same order, each ordered `name, status, detail`; each detail equals its text form without the `<status> <name>: ` prefix.

```text
ok multiplexer: herdr server=default
ok multiplexer.caller_pane: HERDR_PANE_ID=w2:p2N session=w2 window=w2:tP
warn codex.skills: skills not installed at /home/user/.codex/skills; run 'cafleet setup --coding-agent codex'
```

```json
[{"name":"multiplexer","status":"ok","detail":"herdr server=default"},
 {"name":"multiplexer.caller_pane","status":"ok","detail":"HERDR_PANE_ID=w2:p2N session=w2 window=w2:tP"}]
```

Statuses are `ok`, `info`, `warn` and `error`. After printing, doctor exits 1 with
`Error: doctor found errors` when any check has status `error`; otherwise it
exits 0, `warn` included.

##### Check rows

Each check has a stable `name`, a `status` and a one-line `detail`. Checks run in table order; the rows from `<agent>.configuration` through `<agent>.skills` repeat for each backend in registry order, each backend emitting only its own preset and extension rows.

| Check name | `ok` | `info` | `warn` | `error` |
|---|---|---|---|---|
| `multiplexer` | `<backend> server=<server>` | `not checked: a shared <label> server (pid <pid>) supplies the caller environment` | — | The backend detection, server resolution or availability error |
| `multiplexer.caller_pane` | `<VAR>=<pane_id> session=<session> window=<window_id>` | `unavailable: the multiplexer is unavailable` | A refusal body (§ Caller pane verification) | A propagated verification error, or the context read of a verified pane |
| `database` | `cafleet <VERSION> at <path>` | — | — | The schema-version guard error for an empty or incompatible database, or the open/inspection cause |
| `member.<member_id>.runtime` | § Member runtime checks | § Member runtime checks | § Member runtime checks | § Member runtime checks |
| `<agent>.configuration` | The resolved config path (§ Config-dir resolution) | — | — | The config-path validation error, or `HOME is not set` |
| `<agent>.executable` | `<binary> is on PATH` | `<binary> is optional and absent from PATH` | — | — |
| `opencode.preset[.<mode>]` | § OpenCode preset checks | § OpenCode preset checks | — | § OpenCode preset checks |
| `opencode.extension`, `hermes.extension`, `hermes.plugin_enabled`, `pi.extension` | § Extension checks | § Extension checks | — | § Extension checks |
| `<agent>.skills` | The skills dir, when every embedded skill file equals its installed copy | `unavailable: configuration failed: <cause>`; `unavailable: executable is absent` | `skills not installed at <skills dir>; run 'cafleet setup --coding-agent <agent>'` | `<path>: <cause or "differs from the embedded copy">; run 'cafleet setup --coding-agent <agent>'` |

The multiplexer rows follow the `fleet create` steps without `--pane` (§ Caller pane verification), and `<VAR>` is the verified pane variable:

| Situation | `multiplexer` | `multiplexer.caller_pane` |
|---|---|---|
| Row 1 refuses, before any multiplexer command | `info` | `warn` with the shared-server body |
| Backend detection, server resolution or availability fails | `error` | `info` |
| The env pane variable is unset | `ok` | `warn` with the unset body |
| Rows 2, 3 or 5–7 refuse | `ok` | `warn` with the unverified body |
| Row 4 verifies the pane | `ok` | `ok` |
| A pane-process command failure, a nested-namespace read error, or a failed context read of the verified pane | `ok` | `error` with the error message |

###### Member runtime checks

When the `database` row is `ok`, doctor adds one `member.<member_id>.runtime` row right after it for each active ordinary member whose run status is not terminal (`exited`, `failed` or `lost`), in member-ID order. A `running` or `degraded` run is probed through the runner's control socket health request; the probes run concurrently, each under a one-second deadline, and the batch overlaps the other checks the way the Hermes batch does, with the rows keeping their check order. `<eligibility>` and `<code>` are the observation's eligibility rendering and diagnostic code (§6.3 Runtime presentation). The first matching row applies:

| Situation | Status | Detail |
|---|---|---|
| Run `starting`, `prepared` or `recovering` | `info` | `run=<run_id> status=<status>: not probed` |
| Probe failed | `error` | `run=<run_id> health probe failed: <cause>; <hint>` |
| Probe answered without an observation | `info` | `run=<run_id> observation=pending` |
| `gap_open_since` more than 30 seconds before doctor's wall clock | `error` | `run=<run_id> observation=<eligibility> diagnostic=<code> gap_open_since=<ts>; <hint>` |
| `degraded` otherwise (a younger gap, no pending gap, or a gap recorded after the native child exited) | `warn` | `run=<run_id> observation=<eligibility> diagnostic=<code>` |
| Otherwise | `ok` | `run=<run_id> observation=<eligibility>` |

`<hint>` is exactly `inspect with 'cafleet member show <member_id> --json'; if it does not recover, run 'cafleet member delete <member_id>' and create a replacement member`, with ` --database-url URL` appended to each quoted command for a non-default database. An `error` member row makes doctor exit 1 with `Error: doctor found errors`, like every other `error` row.

###### OpenCode preset checks

`doctor` has no selected launch policy, so it checks the preset `<preset base>/agents/<mode>.md` of every OpenCode launch-policy choice (§6.9 Launch policy and capability registry), in catalog order. The default choice's check is `opencode.preset`; every other choice's is `opencode.preset.<mode>`. After the shared `info` states of § Extension checks, a readable preset is `ok` with its path. Setup installs every preset, so a missing or unreadable one is `error` `<path>: <cause>; run 'cafleet setup --coding-agent opencode'`: the installation predates that preset or was altered.

OpenCode presets installed only at another base are not the resolved installation; doctor reports through the checks at the resolved base, `~/.config/opencode` with `OPENCODE_CONFIG_DIR` unset: `opencode.configuration` is `ok` with that base and every preset check is `error`. Rerunning setup installs every preset at the resolved base and leaves the files at the other base in place.

###### Extension checks

Each extension check is read-only. An executable record is usable when it names an existing executable file. `opencode.extension` checks `<preset base>/plugins/cafleet.ts` and `<preset base>/plugins/cafleet.executable`; `hermes.extension` checks `<hermes home>/plugins/cafleet/`, located by the batch's `hermes config path`; `hermes.plugin_enabled` reads `disabled`, `enabled` and `entries.cafleet.allow_gateway_injection` from the batch's one bounded `hermes config get --json plugins` call under the *not set* and subtree rules (§ Hermes plugin enablement); `pi.extension` follows `pi.executable`, precedes `pi.skills` and checks `<pi base>/extensions/cafleet.ts` and `<pi base>/extensions/cafleet.executable`.

`hermes.extension` and `hermes.plugin_enabled` share one concurrent read batch, `hermes config path` and `hermes config get --json plugins`, run with null stdin and `HERMES_HOME` set to the resolved base under one two-second deadline. The batch starts before the multiplexer, database and OpenCode checks and overlaps them; the rows keep their check order. A `hermes` call against a missing home makes Hermes create one, so when the resolved Hermes base does not exist doctor runs no `hermes` process.

A preset or extension check reports the first matching state:

| Check | State | Status | Detail |
|---|---|---|---|
| Every preset, extension and skills check | The backend configuration failed | `info` | `unavailable: configuration failed: <cause>` |
| Every preset, extension and skills check | The backend executable is absent from `PATH` | `info` | `unavailable: executable is absent` |
| `hermes.extension`, `hermes.plugin_enabled` | The resolved Hermes base does not exist | `info` | `unavailable: no Hermes home at <base>` |
| `hermes.extension` | `hermes config path` fails or times out | `error` | `cannot read hermes configuration through 'hermes config path': <cause>` |
| `opencode.extension`, `hermes.extension`, `pi.extension` | An embedded file's installed copy is missing, unreadable or differs | `error` | `<path>: <cause or "differs from the embedded copy">; run 'cafleet setup --coding-agent <agent>'` |
| `opencode.extension`, `hermes.extension`, `pi.extension` | The executable record is missing, unreadable or names no executable file | `error` | `<record path>: <cause>; run 'cafleet setup --coding-agent <agent>'` |
| `opencode.extension`, `hermes.extension`, `pi.extension` | Otherwise | `ok` | The plugin path; for Hermes, the plugin directory; for pi, the extension path |
| `hermes.plugin_enabled` | The call fails other than *not set*, times out or prints a member of the wrong JSON type | `error` | `cannot read hermes configuration through 'hermes config get': <cause>` |
| `hermes.plugin_enabled` | `plugins.disabled` lists `cafleet` | `error` | `hermes plugins.disabled lists cafleet; remove it, then run 'cafleet setup --coding-agent hermes'` |
| `hermes.plugin_enabled` | `plugins.enabled` lacks `cafleet`, or `allow_gateway_injection` is not `true` | `error` | `hermes plugin cafleet is not enabled: <key>, …; run 'cafleet setup --coding-agent hermes'` |
| `hermes.plugin_enabled` | Otherwise | `ok` | `cafleet enabled with gateway injection` |

#### `cafleet message` — Message Broker

##### `message send`

`cafleet message send --from-member-id ID --to-member-id ID (TEXT | --file PATH) [--json]`

Resolve the complete text body, including a `--file -` stdin wait, before starting the five-second persistence budget. Persist one point-to-point delivery and return; the send writes no pane input and waits for no delivery. An ordinary member's runner types the pending message at its next preview sweep (§6.5 Push notifications; §6.9 Runner pending-preview sweep), and the recipient's poll reads it whether or not a preview reached its pane. A self-send is committed with its notification stamp set, so no runner types it; a message to a root Director reaches it through its own reception (§6.5 Push notifications).

JSON is `{message}`. Storage failure before commit exits 1 with its cause.

##### `message poll`

`cafleet message poll MEMBER_ID [--json]` returns and consumes every incoming `input_required` message in one write transaction:

1. Open one `BEGIN IMMEDIATE` transaction under the ordinary database timeout.
2. Validate an active member. An unknown or inactive member returns `Member ID not found`, exits 1 and consumes nothing.
3. Select every `to_member_id=MEMBER_ID AND status_state='input_required'` row, ordered by `status_timestamp DESC, message_id DESC`; the order is evaluated before the update.
4. Update exactly the selected IDs to `status_state='completed'` with one `status_timestamp` (now), and re-read them in the step-3 order.
5. Commit, then print. JSON is the array of post-consumption records, each with `status_state` `completed`. Text keeps its 200-codepoint body truncation (§ Message Body Truncation), and `message show MESSAGE_ID --json` recovers a full body.

Writers serialize on the transaction, so two concurrent polls of one member return disjoint sets whose union is the pending set. An empty inbox prints `[]` (text: `No messages found.`), commits a no-op and exits 0. A failure before commit rolls back and consumes nothing. A stdout failure after commit leaves the returned messages consumed; no later poll returns them and no later listener notice's pending set includes them, and `message show` still reads each one. Consumption is independent of the assignment's progress (§6.4 Persisted shape).

##### `message listen`

`cafleet message listen MEMBER_ID [--timeout SECONDS] [--codex-thread THREAD]`

Notify any active member's pending messages as UTF-8 JSONL. Derive and pin its fleet at startup through the configured database and the ordinary schema-version guard. Require a live root Director with a concrete registered placement, without resolving a multiplexer or checking its executable/physical pane; the subject's own placement may be absent. Unknown/inactive startup subjects use `Member ID not found`, exit 1.

| Surface | Contract |
|---|---|
| Lease | Optional integer from 1 through 1800 inclusive; absence runs until closure, cancellation or failure |
| Expiry | One monotonic lease from startup; activity leaves expiry unchanged |
| Output | Compact tagged JSON object plus LF, flushed per record; JSON strings escape controls/newlines |
| Startup | Validate/pin identity, then emit and flush `ready` before replay |
| Idle | Check immediately, then wait one second after a snapshot that emits nothing or whose notice is refused, shortened by the lease |
| Completion | Flush `closed`, then exit 0 for expiry or normal subject/fleet closure |
| Failure | Diagnostic on stderr and exit 1; a parse error (§6.3 Error Messages) exits 2 |

The command always emits JSONL; text mode and `--json` are absent. Stdout contains protocol records only:

```json
{"event":"ready","member_id":1,"fleet_id":1}
{"event":"pending","next":"cafleet message poll 1 --json"}
{"event":"closed","member_id":1,"reason":"timeout"}
```

A `pending` record is a wake notice: it carries no message ID or body, only the consuming command in `next`, and the receiver consumes full records by running that command:

| Field | Type | Contract |
|---|---|---|
| `event` | string | `"pending"` |
| `next` | string | The consuming command for the listened subject, identical for every notice of one listener process |

A notice's *pending set* is `P` of the snapshot that produced it (§ Notice hold). The listener keeps it internally for the hold; the record does not carry it. `pending(P)` denotes a `pending` notice whose pending set is `P`.

`next` is built once at startup:

1. Start with `cafleet message poll <MEMBER_ID> --json`, where `<MEMBER_ID>` is the listener's positional `MEMBER_ID` as an integer.
2. When the listener process was invoked with `--database-url <url>`, append ` --database-url ` and the shell word for `<url>`, verbatim as supplied, even when it names the default database file. When `--database-url` was omitted, append nothing.

**Shell word.** A value whose every character is in `[A-Za-z0-9_./:~@%+=,-]` is used as is. Any other value is POSIX single-quoted: `'`, then the value with each `'` replaced by `'\''`, then `'`. An empty URL never reaches emission, because startup fails opening the database before `ready`.

```json
{"event":"pending","next":"cafleet message poll 7 --json --database-url sqlite:///tmp/fleet.db"}
{"event":"pending","next":"cafleet message poll 7 --json --database-url 'sqlite:///tmp/my fleet.db'"}
```

Key order is `event`, `next`. A `pending` record is never emitted for an empty pending set. `closed.reason` is `timeout`, `member_inactive`, `fleet_deleted` or, only in forwarding mode (§ Codex forwarding), `session_exited`; fleet deletion takes precedence when both lifecycle closures are observed. Check normal closures before revalidating a continuing live fleet. Missing persisted rows, changed fleet identity, invalid stored enums/root relationships or missing root placement are fatal errors.

Each short read transaction validates identity and selects every `input_required` record for the pinned subject ordered by `(created_at ASC, message_id ASC)`, committing before output or sleep; a later commit with an earlier timestamp joins the pending set when first observed.

**Notice hold.** The listener's only emission state is the *hold*. A hold exists exactly while some ID in the last delivered notice's pending set is still pending; it carries the *held set* `H` (those IDs, never empty) and `held_at`, the monotonic instant that notice was delivered. The listener starts with no hold. The hold ceiling is 300 s. On every successful snapshot with ordered pending set `P`, in order:

1. **Reconcile:** `H ← H ∩ P`. Consumed IDs leave the hold; when `H` becomes empty, the hold and its `held_at` end.
2. **Fresh IDs:** `F ← P \ H`.
3. **Decide** by the table below.
4. **Commit:** when the output reports the notice delivered (stdout: the complete record plus LF flushed; forwarding: § Codex forwarding), the hold becomes `H ← P` with `held_at` set to now. A refused notice leaves the hold exactly as step 1 left it, and the next snapshot decides again. An output failure ends the listener with exit 1.

| Condition | Action |
|---|---|
| `P` empty | Emit nothing |
| No hold and `P` non-empty (hold released) | Emit `pending(P)` |
| A hold, `F` non-empty and at least the hold ceiling since `held_at` | Emit `pending(P)`, the ceiling re-notice: one bulk notice whose pending set is every unpolled ID |
| Otherwise (a hold with `F` empty, or the ceiling not reached) | Emit nothing |

Every notice's pending set is the full pending set, so every pending ID in an earlier notice's pending set is in `H`; that one set separates notified from fresh IDs, and the listener keeps no per-ID history. A failed snapshot leaves the hold unchanged. A snapshot that emits nothing, or whose notice is refused, waits one second (shortened by the lease) before the next snapshot; one whose notice is delivered rechecks immediately.

**Re-listing.** An ID leaves `P` only through a consuming `message poll`, which consumes the subject's whole pending set atomically, so a snapshot sees either all held IDs pending or none of them, and any poll by the receiver releases the hold. The next ordinary notice's pending set holds only IDs committed after that poll. Within one listener process an ID is in two notices' pending sets only through a ceiling re-notice, which fires only when no poll has run for the hold ceiling since the held notice was delivered, so every ID in its pending set, held or fresh, is unpolled. The only other re-listing is a new listener's startup replay of still-unpolled IDs. The ceiling is the liveness backstop for a delivered notice that no receiver turn acted on; without it, every later message would stay held.

**Notice bound.** Each ordinary notice after the first requires a hold release, and each release requires a poll that consumed held IDs. Each ceiling firing emits at most one notice and restarts the ceiling. A listener therefore emits at most 1 notice, plus one per poll that consumed held IDs, plus one per ceiling firing (at most one per hold ceiling period, and only while held IDs stay unpolled with fresh IDs present). A receiver turn with k mid-turn polls can queue at most k + 1 notices behind it, each whose pending set holds only IDs that turn already consumed, so the poll it prompts returns `[]` unless messages arrived since. A burst of any size adds no notice beyond the one outstanding.

| Situation | Behavior |
|---|---|
| A delivered notice whose pending set is never polled, and a new message arrives | One ceiling re-notice whose pending set holds the held and fresh IDs, at least the hold ceiling after `held_at` |
| A delivered notice whose pending set is never polled, and nothing new arrives | No re-notice; the ceiling fires only when fresh IDs exist |
| The receiver polls before any notice's pending set includes an ID | That ID is never notified |
| A message, including a runtime event such as `interaction_opened`, arrives while a notice is held | No notice until the receiver polls or the ceiling re-notice fires; any `poll` returns the ID immediately |
| The receiver polls mid-turn, then a queued notice is delivered | The queued notice's pending set holds only IDs that poll consumed; the poll it prompts returns `[]` unless messages arrived since |
| Listener restart | The new process emits `ready`, then one notice whose pending set is every pending (unpolled) ID, including IDs a previous listener notified |

Listening leaves consumption and `notified_at` unchanged and performs no pane retry, lock or dispatch; stamped pending self-sends remain notice-eligible. Concurrent listeners are independent read-only observers with no consumer lock, each with its own hold.

Each snapshot has a fresh database deadline at most five seconds away, shortened by the remaining lease. Lock waits and statement execution stay within that deadline: a running statement is interrupted once the deadline passes, and the deadline is checked before and after each statement, each decoded row and the commit. Expiry discards a partial snapshot; interrupted SQL maps to deadline exceeded only when that deadline expired. These cooperative boundaries cover row collection but give no hard real-time preemption of one allocation or SQL operation.

Output writes one serialized record at a time and applies backpressure before selecting more messages. Nonblocking filesystem/poll output preserves/restores descriptor flags on normal return, resumes partial writes at their byte offset and checks every write/flush, under a five-second deadline shortened by the lease. Broken pipe, write-zero, flush failure or output-deadline exhaustion exits 1 visibly, even after partial records; normal lease expiry between records writes `closed` with a separate bounded final write. Complete records survive a later error; an incomplete final line stays unconsumed and recovers through broker replay. Listener memory holds only the current snapshot, one record and the held IDs.

Database contention retries stay inside the snapshot budget; exhaustion or another fatal error ends the stream. The listener keeps its connection/PID across healthy events and ordinary signal behavior outside pane termination deferral; host cancellation may end it without `closed`. Native owners retain one handle, reconcile durable state after failure and establish old-process termination before replacement. Installed backend instructions own concrete native tool recipes and their evidence limits.

###### Codex forwarding

`--codex-thread THREAD` selects forwarding mode, which keeps the record, notice-hold, replay, lease and idle contracts and changes the output:

- **Stdout** carries only the `ready` record and the final `closed` record, so an unread PTY never stalls the five-second output deadline. Writing that `closed` record, with any reason including `session_exited`, is the listener's last action before exit; a write failure leaves the exit status unchanged.
- **Forwarded events.** Each `pending` notice, and each `closed` record whose reason is `timeout`, `member_inactive` or `fleet_deleted`, is forwarded in emission order. `ready` and `closed` `session_exited` are never forwarded.
- **Forwarding command:** `codex queue --thread THREAD --message <text>`, where the text is `[cafleet listen record] <compact record JSON>`, for example `[cafleet listen record] {"event":"pending","next":"cafleet message poll 7 --json"}`.
- **One at a time.** Each `codex queue` runs to exit, with a 10 s deadline, before the next record is forwarded. Exit 0 delivers the record and commits a `pending` notice's hold.
- **Non-zero exit.** Any non-zero `codex queue` exit ends forwarding, including Codex's refusal of a submission while the thread's queue holds 100 (`queue cannot contain more than 100 submissions`). The listener checks the pinned session first (§ Liveness), then exits as the table below states; it forwards nothing more and retries nothing.
- **Binary resolution.** `codex` resolves through the listener's `PATH` with the absolute-entry search of §6.9 Native executable resolution.

| Condition | Behavior |
|---|---|
| `codex` not on `PATH` at startup | `Error: binary codex not found on PATH`, exit 1, before `ready` |
| No Codex session ancestor at startup | `Error: --codex-thread requires a codex ancestor process`, exit 1, before `ready` |
| `codex queue` exits non-zero while the pinned session is alive | stderr `failed to forward record to codex thread <THREAD>: <codex stderr>`, exit 1 |
| `codex queue` exceeds 10 s while the pinned session is alive | kill it; stderr `codex queue for thread <THREAD> did not finish within 10 s`, exit 1 |
| Any other failure after `ready` (database, schema guard, deadline) | its existing stderr diagnostic, exit 1 |
| The pinned Codex process exits, including when the listener checks it first after a `codex queue` exits non-zero or exceeds 10 s | stdout `closed` with reason `session_exited`, exit 0; no diagnostic and nothing forwarded |

**Session pinning.** At startup the listener walks its ancestors, starting from its parent, through the process-entry reader of §6.3 Caller pane verification (`/proc/<pid>/stat` and `/proc/<pid>/cmdline` on Linux, `ps -o ppid= -o args= -p <pid>` elsewhere), propagating its read errors. It pins the nearest ancestor whose argv[0] basename is exactly `codex`; shells, `bwrap`, node launchers and the sandbox helper (the same binary, run with argv[0] `codex-linux-sandbox`) are skipped. The executable path is not compared, because the helper shares it. A sandbox that unshares the PID namespace hides the ancestor, and startup fails with the missing-ancestor error.

**Liveness.** Each idle tick (at most 1 s) checks the pinned pid with `kill(pid, 0)`. On Linux it also compares the pinned process's start time from `/proc/<pid>/stat`, so a reused pid counts as exited. The pinned process's exit ends the listener with `session_exited`. The listener sets no `PR_SET_PDEATHSIG`, which fires when the forking thread exits rather than the multi-threaded Codex process.

##### `message show`

`cafleet message show MESSAGE_ID [--json]` returns one full record in any state and changes no state; it is the recovery read for a consumed body. Existence is the read guard.

#### `cafleet member` — Member Lifecycle + Pane Interaction

##### Member targeting and key delivery

Require an active member and placement for pane operations; concrete pane IDs remain opaque. Pending placement is valid during creation and yields contextual prompt/exec/capture errors. Individual member deletion protects the root Director. Before pane I/O, prompt and exec reject the root Director (bodies at each command). They submit to the member's runner, which owns the pane input lock for actual stages (§6.5 Pane input lock) and selects the command's submission plan, delivery fallback or queueing (§6.9 Delivery method and interaction priority); exit 0 establishes complete submission or queue acceptance only.

##### `member create`

Register a member **and** spawn its coding-agent pane. A fleet has exactly one root Director, so the command takes no identity or override flag: it first resolves the acting Director from `fleets.director_member_id`, before registration, and uses that id for backend inheritance and spawn-prompt substitution. Per-backend child argv and omitted permission defaults are in §6.7 Per-backend spawn argv (exact, token-by-token).

| Argument | Required | Notes |
|---|---|---|
| `--fleet-id` | yes | The fleet the new member joins. |
| `--name` | yes | Display name; the member pane name derives from it (§6.5 Window and pane naming). Per-backend pane-title behavior: §6.7 Backend capabilities. |
| `--description` | yes | One-sentence purpose. |
| `--coding-agent` | no | A backend registry token; when omitted, the member inherits the spawning Director's placement backend. Availability failures exit 1 (below). |
| `--model` | no | Forwarded to the backend binary's own `--model` flag; opencode requires `<provider-id>/<model-id>`, and pi refuses a `:<thinking>` suffix. Per-backend formats and create-time validation: §6.7 Model selection. |
| `--permission-mode MODE` | no | Launch-only selection from the resolved backend’s policy catalog; omission selects the backend's default mode (§6.9 Launch policy and capability registry). |
| `--effort` | no | Reasoning-effort level forwarded to the backend binary, validated per backend before any side effect (levels, forwarding forms and rejection strings: §6.7 Reasoning effort). |
| positional `PROMPT` | one of | Inline spawn prompt (backend-neutral template). |
| `--file PATH` | one of | UTF-8 spawn-prompt file (`-` = stdin) under the shared body-input and transport contract (§6.3 Text bodies (positional `TEXT` / `--file`)). |
| `--json` | no | Output as JSON |

A missing backend binary exits 1 with `Error: binary <name> not found on PATH`. The opencode backend also requires the selected launch policy's agent preset `<preset base>/agents/<mode>.md` at the resolved preset path (§ Config-dir resolution); a missing or unreadable preset or an invalid config-location variable exits 1 (§6.3 Error Messages). The hermes backend also resolves its effective home through `hermes config path`, concurrently with the native version probe (§6.7 Hermes availability preflight). The pi backend also requires the CAFleet pi extension `<pi base>/extensions/cafleet.ts`; a missing or unreadable extension exits 1 (§6.7 Contract error strings), and a pi prompt beginning with `@` exits 2 before registration (§6.7 Per-backend spawn argv).

After prompt validation, `member create` acquires the fleet layout lock within the layout lock wait (60 s) (§6.5 Fleet window layout) and only then captures its ten-second creation deadline, so the native version probe, spawn, `prepared` handshake, publication and `start` keep their full budget. A busy lock exits 1 with the create busy error (§6.3 Error Messages) and registers nothing. Under the lock it resolves the Director placement, reads occupancy and each scanned window's live height, chooses the slot (§6.5 Fleet window layout) and registers the member with that slot (§6.2 Members). The lock is held until creation returns, after `started` or after compensation.

The pane target — a column of an existing window or a new overflow window in the Director's session — and the multiplexer server come from the registry and the fleet Director's recorded placement (§6.5 Multiplexer context); each new member placement copies the Director's `mux_server`. Publication patches the pane ID, window ID and window name together (§6.9 Runner boundary and creation transaction).

###### Member environment

The native child runs in the multiplexer pane's environment overridden by the spawn environment. The spawn environment holds only the selected backend's config-location variable (its Variable in §6.3 Config-dir resolution; for opencode the preset variable `OPENCODE_CONFIG_DIR`), with the creator's exact value, when the creator sets it; an unset variable stays unset, so the member uses the native default, which equals the CAFleet default base. `member create` validates that variable before registration and an invalid value fails with the pinned validation error, so the member reads the configuration `member create` validated and `setup` installed into. The runner launches as `<cafleet> --database-url sqlite:///<canonical database path> __member-run --descriptor <path>` (§6.9 Runner boundary and creation transaction), so no database selection enters any environment. The launch descriptor and launch evidence record the spawn environment; config locations are paths, not secrets.

###### Spawn-prompt substitution

`member create` renders its resolved spawn prompt body with a formatter that substitutes exactly these placeholders:

| Placeholder | Substituted value | How the spawned member sees it |
|---|---|---|
| `{fleet_id}` | The member's fleet id | `FLEET ID: <fleet_id>` |
| `{member_id}` | The member's own newly-allocated id | `YOUR MEMBER ID: <member_id>` |
| `{director_member_id}` | The fleet's root Director id | `DIRECTOR MEMBER ID: <director_member_id>` |
| `{coding_agent}` | The resolved backend's registry token | `CODING AGENT: <coding_agent>` |
| `{database_url}` | `default` when the canonical database path equals the default path (§6.3 Database selection), otherwise `sqlite:///<canonical database path>` | `DATABASE URL: <database_url>` |

The spawned member learns its identity and database from the literals rendered into its prompt; commands take every ID as a literal argument, and a member whose `DATABASE URL:` line names a URL passes `--database-url <url>` on every `cafleet` command. A prompt without `{database_url}` is accepted only for the default database; against a non-default database `member create` exits 1 before registration (§6.3 Error Messages). Besides the names above, the formatter accepts only doubled literal braces (`{{` / `}}`), not Python format specifications, conversions, or attribute/index access; an unknown placeholder or malformed brace expression exits 2 before registration (§6.3 Error Messages).

The spawn always creates the pane without stealing focus (tmux `split-window -d` / `new-window -d`, herdr `--no-focus`): the Director's pane and active window stay active (§6.5 Member pane creation). In the default output, `pane` and `window` render `(pending)` until the placement is published.

###### Creation failure compensation

Member creation validates policy and assignment, and records the installed native version (§6.9 Native version probe: a failed probe warns and creation continues with unknown coverage), then registers a pending placement and run. A known version below the backend's first boundary exits 1 before member/run registration or pane creation (§6.9 Profile selection by minimum version). The multiplexer starts a runner from a private descriptor; a `prepared` handshake precedes placement publication and `start`, and creation returns only after child spawn produces `started` within one handshake deadline. §6.9 Runner boundary and creation transaction owns descriptor cleanup and uncertain-launch compensation. Spawn-prompt placeholders are validated before registration, so a placeholder error exits 2 with nothing registered. The backend owns a newly spawned pane until the spawn returns (§6.5 Pane ownership during creation).

After registration, compensation ends with the descriptor/run/member rollback: remove the pending descriptor, then mark the run `failed` and deregister the member in one transaction. When compensation succeeds, the member is deregistered, its placement removed and the run `failed`.

| Failure point | Compensation order |
|---|---|
| Pending member/run registration | The registration transaction rolls back, leaving no new member, placement or run |
| Runtime artifact preparation or prompt rendering after registration | The descriptor/run/member rollback |
| Runner pane spawn | The backend closes a pane it already identified, then the rollback |
| After the spawn: handshake, placement publication, `start` or the started-state check | Shut down the owned runner and child when the runner already owns the run, close the created pane in its spawned window and restore that window, then the rollback |

Closing an overflow window's only pane closes that window. A new-window ordinal consumed at registration stays consumed after compensation (§6.5 Fleet window layout).

For a `hermes` run whose child started, compensation runs the Hermes work settlement (§6.7 Hermes work settlement) before the owned runner shutdown, under its own 20-second budget; an unverified settlement appends its cause as a cleanup diagnostic.

Compensation continues past a failed cleanup and never re-closes a pane the backend already tried to close. Creation keeps the primary cause and its exit category and appends each cleanup diagnostic on its own stderr line:

| Cleanup outcome | Appended diagnostic |
|---|---|
| Failed spawn with no confirmed id (no other pane is inferred or closed) | `pane ID unknown; pane cleanup unconfirmed` |
| Backend close failure | `cleanup failed for pane <id>: <detail>` |
| Unconfirmed owned-child shutdown | Its shutdown diagnostics, or `owned child cleanup remains unconfirmed` when it reports none |
| Failed descriptor removal, run update or deregistration | Its own cause; a recorded run with remaining diagnostics keeps `cleanup_state=incomplete` |
| An internal fault aborts the launch step before it returns a result | `native launch unwound; performing owned compensation`, after the same compensation |
| Interrupted pending creation | `pending creation was interrupted` |

Once the runner reports `started` under the matching identity, creation removes the descriptor and emits its result; placement publication alone keeps the rollback armed. Output keys keep their documented order and nullability; runtime fields follow §6.3 Runtime presentation.

##### `member delete`

Request owned runner shutdown under the member's recorded run identity before pane/registry cleanup, retaining explicit unconfirmed-child diagnostics and recoverable runtime records. Run recovery (§6.9 Storage pressure and durability boundary) before deregistration only within the remaining operation deadline. Lock a concrete placement before kill/deregistration (a member without a concrete pane needs no lock), retrying contention every 50 milliseconds against one five-second deadline; expiry exits 1 with `member ID's pane input remained busy for 5 seconds; member was not deleted.` and leaves state unchanged. Preserve pane/registry errors and layout cleanup ownership.

After the pane lock and before any cleanup step (Hermes settlement, runner shutdown, pane close), acquire the fleet layout lock within the layout lock wait (60 s) (§6.5 Fleet window layout) and hold it through deregistration and layout restoration. The deletion deadline moves later by the wait's duration, as it does for Hermes settlement. Expiry exits 1 with the delete busy error (§6.3 Error Messages) and leaves the run, pane and row unchanged.

For a `hermes` run, after the pane lock is held and before runner shutdown, run the Hermes work settlement (§6.7 Hermes work settlement) under its own 20-second budget; the deletion deadline moves later by the settlement's duration, so lock acquisition stays bounded by the original five seconds and the remaining steps keep the time they had when settlement began. An unverified stop exits 1 (§6.3 Error Messages) and retains the runner's cleanup records and the member row; a retry re-runs the settlement.

Revalidate pending placement and deregister atomically. For a concrete pane, retain input ownership while closing it outside a SQLite write transaction, then revalidate and finalize the registry. After successful close, mandatory registry finalization uses the later of the original deadline and one second after finalization begins; actual database failures report that the pane was closed and preserve the database cause. The close passes the placement's recorded `mux_window_id`, and optional layout restoration of that window follows mandatory finalization within the original deadline (§6.5 Delete-time pane layout). A failed or expired restoration emits a stderr diagnostic while preserving successful deregistration.

##### `member show`

`cafleet member show MEMBER_ID [--json]` reads any active registry entry (root Director included), with no multiplexer requirement. Text is the compact one-line row `ID NAME STATUS window=WINDOW` (§6.3 Member list output); an ordinary member's row appends the runtime text line, and the root Director's stays its registry row. The `--json` payload is the detailed view, including `kind` (`director` / `member`, §5.4 Member kind discriminator), the placement object and the appended runtime summary (§6.3 Runtime presentation).

After its registry read, member show probes the ordinary member's runner through its control socket health request under a one-second deadline when the run's status is `running` or `degraded`, and reports the outcome in the four member-show runtime keys (§6.3 Runtime presentation). A failed probe is reported in those keys and leaves the exit status 0.

##### `member list`

`cafleet member list FLEET_ID [--json]` lists every **active** registry entry of the fleet (root Director and ordinary members) and takes no identity flag; an empty roster prints `0 members.`. Field projection, the `window` column and absent values: §6.3 Member list output; rows and activity: §6.2 Members. Text idle values render as `Ns` / `Nm` / `Nh`. Per-member `description` and `registered_at` are on §6.3 `member show`.

##### `member prompt`

`cafleet member prompt MEMBER_ID --from-member-id DIRECTOR_ID TEXT [--json]` validates a nonblank line without CR/LF or C0/DEL controls, otherwise exits 2 with `text must be one non-empty line without control characters.`

A root Director target exits 1 with `cannot prompt the Director's own pane`. The runner's submission plan is profile delivery (§6.3 Member targeting and key delivery); success returns ordered `{member_id, pane_id, text, input_id, delivery}`.

A submitted prompt prints `Sent prompt TEXT_REPR to member NAME (PANE).`; a queued prompt prints `Queued prompt for member ID (PANE).` Both add the delivery and input lines (§6.3 Runtime presentation). `TEXT_REPR` follows §6.4 Quoted text.

Pre-write error, including a held runner reply: `prompt was not dispatched to member ID: CAUSE`. Partial uncertainty: `prompt delivery to member ID is uncertain: CAUSE. Inspect the pane before retrying.` These exit 1 without a success record.

A prompt whose staged write stops or whose submission is not confirmed records `uncertain` and exits 1 with its recorded cause, method and input ID (§6.3 Runtime presentation). Prompt input requires a deliberate retry. Oversized input without safe framing is refused before reservation or bytes; a queued refusal settles `not_submitted/plain_limit_exceeded` and publishes its result (§6.9 Measured submission plans and settle intervals). Insufficient immediate-attempt budget accepts a zero-byte queue entry with reason `delivery_budget_exhausted`.

##### `member exec`

`cafleet member exec MEMBER_ID --from-member-id DIRECTOR_ID COMMAND [--json]` or `cafleet member exec MEMBER_ID --from-member-id DIRECTOR_ID --file PATH [--json]`; `--file -` reads stdin. Exactly one source is required. File/stdin input is UTF-8 with at most one terminal LF or CRLF stripped; inline bytes are preserved exactly.

Require a nonblank line without CR/LF, NUL or other C0/DEL controls, including tabs. Invalid input exits 2 before pane I/O with `command must be one non-empty line without control characters.` A multiline task supplies a one-line explicit script invocation.

Resolve an active ordinary member and concrete placement. The runner submits literal `! ` plus the command, with the native-exec plan as its submission plan (§6.3 Member targeting and key delivery). The backend owns working directory, environment, permissions, output and exit rendering.

Ordered JSON is:

```json
{"member_id":42,"pane_id":"w1:p2","coding_agent":"codex","command":"printf example","dispatched":true,"input_id":17,"delivery":{"status":"submitted","method":"profile","reason":null}}
```

`dispatched` is `true` for `submitted` and `false` for `queued`. Text: `Dispatched native command to member ID (PANE). Inspect the pane for its result.` for a submission, or `Queued native command for member ID (PANE).` for queue acceptance, each followed by the delivery and input lines (§6.3 Runtime presentation).

| Condition | Error body (exit 1) |
|---|---|
| Root Director | `cannot exec in the Director's own pane` |
| Runner refusal before typing | `native command was not dispatched to member ID: CAUSE` |
| Typing may have begun | `native command delivery to member ID is uncertain: CAUSE. Inspect the pane before retrying.` |

An exec whose staged write stops or whose submission is not confirmed records `uncertain`, exits 1 and requires a deliberate retry. An immediate oversized command with no safe framing is refused with zero bytes; a queued refusal settles `not_submitted/plain_limit_exceeded` with a result event. Insufficient immediate budget queues with reason `delivery_budget_exhausted` (§6.9 Measured submission plans and settle intervals).

The first text-writing call can partially type even if it fails. Failure after Enter may mean execution began. A wrapped operation durably records its operation key, payload digest and submission state; the receipt contains no command text. The Director authorizes native input under its host rules; visibility establishes no second member-tool permission check.

##### `member press`

`cafleet member press MEMBER_ID --from-member-id DIRECTOR_ID KEY... [--json]` requests only the specified ordered keys. Require a positive signed 64-bit member ID and one through 64 keys, each at most 32 UTF-8 bytes, using the common key grammar and backend mappings (§6.5 Explicit key dispatch). Preserve literal character case and render named keys canonically. Validate the complete vector before any backend probe or writing invocation.

Use the ordinary `--` option separator before literal punctuation that could be read as an option; put `--json` before that separator. For example, `cafleet member press 42 --from-member-id 1 -- '-' '+' ';'` requests three keys. It supplies no Escape prelude, Enter suffix, prompt text or shell-command submission.

Resolve an active ordinary member and its concrete placement under the shared input lock, selected-context validation and original budget (§6.5 Explicit key dispatch). Wrapped members receive the request through their outer pane as manual native input (§6.9 Explicit manual key input). Success means backend acknowledgment of this ordered request; actual PTY bytes, native approval and completion require their own observation.

Exit 0 prints compact ordered JSON when `--json` is selected:

```json
{"member_id":42,"pane_id":"%5","keys":["Ctrl+a",";"],"status":"acknowledged"}
```

Otherwise print `Acknowledged keys KEYS_JSON for member ID (PANE).`, where `KEYS_JSON` is the compact JSON array of canonical keys. Both modes end with one newline. The result carries no native-byte count, delivered-prefix count or permission receipt.

Missing/extra positional arguments and a nonpositive/malformed member ID are parse errors (§6.3 Error Messages); key validation uses these bodies, with one-based `N` and JSON-quoted invalid input:

| Condition | Error body (exit 2) |
|---|---|
| Key count outside the domain | `press requires between 1 and 64 keys` |
| A key exceeds its UTF-8 bound | `press key at position N exceeds 32 UTF-8 bytes` |
| A key is outside the common grammar | `invalid press key at position N: JSON_QUOTED_TOKEN` |

Operational failures exit 1 with empty stdout, including under `--json`. Retain the original typed cause and any cleanup diagnostics, then append the effect diagnostic below on its own stderr line.

| Observed boundary | Effect diagnostic |
|---|---|
| Before any writing process starts, including a failed metadata probe | `keys were not dispatched to member ID.` |
| A writing process starts and fails, times out or returns an unreadable acknowledgment | `key delivery to member ID is uncertain; any prefix may have been handled. Inspect the pane before an explicit retry.` |

| Local refusal | Primary error body |
|---|---|
| Root Director target | `cannot press keys in the Director's own pane` |
| Input lock remains busy | `member ID's pane input remained busy` |
| Revalidated target changes | `member ID placement changed before key dispatch` |
| Selected live context differs from placement | `press target context does not match the recorded placement` |

A missing member or placement, a pending pane (§6.3 Error Messages) and a native process error report their own primary diagnostic. Binary-not-found, not-started and prelaunch expiration have zero writing invocations. No started failure, including a pane-gone response, is retried automatically. Request-bound permission decisions retain the separate `permission respond` contract and host authorization.

##### `member capture`

`cafleet member capture MEMBER_ID [--lines N] [--ansi] [--json]`

- `--lines`: Number of trailing lines to capture (default: **20**).
- `--ansi`: Preserve ANSI escapes in the raw capture. The default strips ANSI escapes and cleans carriage-return redraws.

Output shapes: §6.3 Output shapes; target resolution: §6.3 Member targeting and key delivery. A pending placement is a hard error (§6.3 Error Messages).

JSON stamps `captured_at` from the local UTC clock at the capture read boundary and sets `content_sha256` to the lowercase SHA-256 hex of the exact emitted `content`'s UTF-8 bytes, including Unicode and empty captures: the ANSI-stripped, carriage-return-defragmented string by default, or the ANSI-preserving string with `--ansi`, with no later normalization. Capture content is never stored in SQLite. Text capture prints content without adding a newline.

#### Runtime presentation

Member create and member show append `runtime` to their ordered `member_id,name,registered_at,placement` and `member_id,name,description,status,registered_at,kind,placement` keys respectively. Member creation records the member's run in its registration transaction, so every ordinary member has a recorded run. `runtime` is `null` for the root Director. The runtime object has ordered keys `run_id,resolved_mode,status,owner_epoch,last_health_at,coverage,current_interaction_id,unresolved_input_ids,owner_kind,owner_pid,owner_start_identity,runner_pid,runner_start_identity,child_pid,child_start_identity`, preserving stored nullability. `unresolved_input_ids` lists, in input-ID order, the run's `in_progress`, `accepted` and `uncertain` inputs and its `ready` inputs carrying deferred metadata: `in_progress` and `uncertain` inputs need Director reconciliation, an accepted input needs none and stays listed until a settlement records its disposition, and a deferred ready input stays listed while the runner owns it in its queue. These registry fields make no live-health claim; member show's probe keys below do.

Member show alone appends four keys to the runtime object, after `child_start_identity`; member create, `member list` and `event show` keep the runtime object above:

| Key | Value |
|---|---|
| `observation_probe` | `skipped` (the run's status is neither `running` nor `degraded`), `answered` or `failed` |
| `observation` | The runner's observation health with ordered keys `visibility,eligibility,degraded,diagnostic,writer_diagnostic,pending_count,gap_count,gap_open_since`; null when skipped, failed, or answered by a runner whose child has not started |
| `observation_error` | The probe failure's message when `failed`, otherwise null |
| `diagnostics` | The run's latest 5 `member_runner_diagnostics` rows (§6.9 Storage pressure and durability boundary), newest `last_at` then `diagnostic_id` first, each ordered `source,message,first_at,last_at,count` |

`visibility` is `visible`, `obscured`, `not_visible` or `unknown`. `eligibility` is the string `composer_empty` or `hold:<reason>`, where `<reason>` is `candidate`, `interaction`, `unknown`, `observer_unavailable` or `gap`; doctor member rows use the same rendering. `degraded` is a boolean, and `diagnostic` is the observation failure code, `sink_unavailable` while the event writer is degraded, or null. `writer_diagnostic` is the event writer's sanitized failure text, non-null only when `degraded`, and `gap_open_since` is the RFC 3339 start of a pending observation gap, non-null only with `hold:gap` and `degraded`. A gap recorded after the native child exits has no pending start, so it reports `hold:gap`, `degraded` and a null `gap_open_since`. `pending_count` and `gap_count` are the runner's uncommitted-transition and observation-gap counters.

Wrapped create/show prints its `ID NAME backend=AGENT pane=PANE` or `ID NAME STATUS` row and appends `Runtime: run=RUN_ID mode=MODE status=STATUS coverage=COVERAGE`. Member show's line appends ` observation=STATE`, followed by ` degraded` when the observation is degraded:

| Probe outcome | `STATE` |
|---|---|
| `answered` with an observation | The eligibility rendering |
| `answered` without one | `pending` |
| `failed` | `unavailable` |
| `skipped` | `-` |

For example, `Runtime: run=37 mode=auto status=running coverage=tested observation=hold:gap degraded`. `event show` uses the runtime object and line without the member-show additions. Prompt/exec JSON appends `delivery` after `input_id` (§6.9 Delivery reporting). Their text output appends `Delivery: status=STATUS method=METHOD reason=REASON`, using `-` for a missing method or reason, then `Input: INPUT_ID` when a durable ID exists. The `Sent prompt …` and `Dispatched native command …` lines print only for actual submissions; a fallback submission always prints `method=fallback` with reason `submission_plan_unavailable`, and a queued result names `interaction_open`, `interaction_candidate`, `composer_busy`, `earlier_input` or `delivery_budget_exhausted`, states that the runner will retry and points to `cafleet input show INPUT_ID --json` for the eventual result. A prompt/exec uncertain error exits 1, names the receipt's recorded `stop_cause` and the selected method, as in `native command delivery to member 110 is uncertain: write_failed (method=profile)`, and includes known receipt identity as `input_id=INPUT_ID`; run/interaction context accompanies held/rejected input.

#### Runtime event and input commands

`cafleet event show EVENT_ID [--json]` returns ordered `event,notification_state,message_id,archive_reason,run,interaction`. `input_delivery_result` and `input_delivery_stalled` expose their complete receipt/recovery fields (§6.9 Input delivery result event; §6.9 Input delivery stalled event). The event is the complete versioned runtime payload (§6.9 Persistence, notification and read interfaces), run is the current runtime summary, and nullable interaction contains the stored interaction fields in the interaction field order below. Text is `Event: EVENT_ID run=RUN_ID seq=EVENT_SEQ kind=KIND notification=STATE message=MESSAGE_ID` followed by the runtime summary line and `Interaction: INTERACTION_ID classification=CLASS closed_at=TIMESTAMP close_reason=REASON` when linked.

`cafleet input show INPUT_ID [--json]` and `cafleet input reconcile INPUT_ID --from-member-id DIRECTOR_ID --disposition DISPOSITION --evidence-ref REFERENCE [--json]` return `{input}` with the durable row ordered `input_id,run_id,operation_key,kind,payload_digest,state,created_at,updated_at,disposition,evidence_ref,delivery,deferred_at,defer_reason,stop_cause`. `delivery` is the ordered `status,method,reason` projection of the stored receipt (§6.9 Delivery reporting); `deferred_at` and `defer_reason` are null together unless the runner queued the input. `stop_cause` is the cause the runner recorded when the input stopped as `uncertain` (§6.9 Stop cause), retained through settlement, or null when no cause was recorded. Text is `Input: INPUT_ID run=RUN_ID kind=KIND state=STATE disposition=DISPOSITION`.

Reconciliation is Director-owned. CLI disposition tokens are `submitted`, `not-submitted` and `abandoned-uncertain`, persisted with underscores as defined by the receipt contract (§6.9 Durable input receipts and settlement). Require a nonempty evidence reference. A settled ordinary receipt with the same disposition returns its original row and evidence reference before live work; a conflicting disposition fails unchanged. Settlement sends no input and establishes no native command result. One original five-second budget covers the pane wait, current-owner synchronization, supplied-connection SQL and socket exchange. Owner synchronization and CAS settlement follow §6.9 Durable input receipts and settlement; uncertainty persists while the owner is unreachable or an active writer cannot be ruled out.

All event/input IDs are positive database-assigned integers; malformed IDs/options fail with usage exit 2. Missing rows and invalid stored state fail explicitly. Event and input text render `-` for an unavailable optional value. Interaction fields are ordered `interaction_id,run_id,episode_seq,classification,evidence_digest,opened_at,evidence_json,closed_at,close_reason`.

#### Permission commands

`cafleet permission inspect INTERACTION_ID --event-id EVENT_ID --from-member-id DIRECTOR_ID [--json]` reads the current request through its pinned runner. Require positive IDs, then an existing interaction (an unknown `INTERACTION_ID` is reported as not found before any event, run or database-identity check), then an active ordinary target, pinned active Director and live fleet; verify the event/message association and interaction ownership under the member's pane-input lock (§6.5 Pane input lock). One original five-second deadline, clipped to any inherited SQL bound, covers locking, current observation/manual progress, database work and the control exchange. Inspection sends zero programmatic child bytes and creates no receipt, audit or claim. Unknown answer capability is an inspection result; stale ownership or an unavailable owner is an operational error.

The inspection JSON is `{request}`. Its object has ordered keys:

```text
event_id,message_id,member_id,fleet_id,director_member_id,run_id,owner_epoch,interaction_id,episode_seq,classification,visibility,backend,native_version,answer_rule_id,answer_capability,capability_reason,supported_decisions,supported_choices,request_token,output_seq,evidence,missing_evidence
```

IDs/epoch/episode are positive integers and `output_seq` an unsigned integer; `message_id` is the verified inbox message ID. `classification` is the current interactive class or null; `visibility` is `visible`, `obscured`, `not_visible` or `unknown`. `native_version` and `answer_rule_id` are null when unavailable. `answer_rule_id` is the unique common rule of the currently supported decision plans (permission request) or choice plans (question), so one supported decision or choice yields its actual rule and differing rules yield null; respond derives its chosen rule internally. The runner's reply and evidence carry the run's backend and native version. `answer_capability` is `tested` or `unknown`; `capability_reason` is null when the current request is answerable, otherwise a bounded explanation. `tested` requires a non-null run `native_version`, a non-null, non-blank `answer_rule_id` of at most 256 bytes, a token, a null capability reason, and a visible permission request with a non-empty decision set or a visible question with a non-empty choice set.

| Field | Permission request | Question |
|---|---|---|
| `supported_decisions` | The currently tested subset of `allow,deny`, in that order | `[]` |
| `supported_choices` | `[]` | One `{choice_id,label,scope}` object (keys in that order) per tested choice plan, in the rule's choice order; `label` is the extracted choice label and `scope` the declared choice scope (`single_request` or `persistent`) |
| `missing_evidence` | The missing public facts below | `[]`; the operation-centred facts do not apply to a question |

A blocked or unsupported request returns empty arrays and a null token. A question without a tested choice plan, such as a question of another backend or one whose rule declares no answer, returns `answer_capability=unknown` with `capability_reason` `question has no tested answer`.

`request_token` is a nullable random lowercase 64-hex value issued by the current runner, bound to the request and invalidated under the response contract (§6.9 Request-bound permission responses). `evidence` is the request's current metadata evidence object (§6.9 Persistence, notification and read interfaces). For a permission request, `missing_evidence` is `operation,arguments,cwd,environment,targets,effects,choice_scope`: metadata evidence carries none of these facts, even with a valid token and a proved private `single_request` plan. The Director obtains necessary missing facts through authorized current capture/routed evidence before deciding. Token identity and private plan eligibility are independent of public evidence completeness and authorization. The token binds the full immutable decision-to-plan or choice-to-plan set, including each rule and stage identity; a null common-rule field grants no authority to select a replacement plan.

Inspection text is:

```text
Permission: interaction=INTERACTION_ID event=EVENT_ID member=MEMBER_ID run=RUN_ID epoch=EPOCH visibility=VISIBILITY answers=ANSWERS
Request: token=TOKEN output_seq=SEQUENCE rule=ANSWER_RULE
```

`ANSWERS` joins with commas the supported decisions of a permission request or the supported choice ids of a question; unavailable values and an empty set render as `-`. Append `Capability: REASON` when nonnull and `Missing evidence: FIELDS` (comma-joined) when the array is nonempty. Text includes no raw screen content.

`respond` requires this complete request snapshot and decision provenance:

```text
cafleet permission respond INTERACTION_ID --event-id EVENT_ID --from-member-id DIRECTOR_ID --expected-run-id RUN_ID --expected-owner-epoch EPOCH --request-token TOKEN (--decision allow|deny | --choice CHOICE_ID) [--json]
```

Exactly one of `--decision` (for a permission request) and `--choice` (for a question) is required. `CHOICE_ID` is 1–64 bytes of lowercase hyphen-separated words (`^[a-z0-9]+(?:-[a-z0-9]+)*$`). Require positive IDs/epoch and the token's exact hex form; enum and shape errors exit 2 before lock or byte effects. Resolve Director mode from current host evidence independently of member launch policy and obtain user permission in other or unknown modes. Native and managed rules still authorize running the command.

The response JSON is `{response}`, with ordered keys:

```text
status,member_id,run_id,owner_epoch,interaction_id,event_id,decision,choice,stage,output_seq,reason
```

`status` is `accepted`, `held`, `rejected` or `uncertain`. Decision and choice retain their command values; exactly one of them is non-null. `stage` is `zero_bytes`, `partial`, `submitted` or `unknown`: accepted has `submitted`, held/rejected `zero_bytes`, and uncertainty the observed partial/submitted stage or `unknown`. `output_seq` is an unsigned integer, null when no current sequence can be established. `reason` is a nullable bounded explanation; an uncertain reply reports its stop cause (§6.9 Stop cause), or `manual_input` when keyboard or pointer input, or failed manual-input accounting, revoked the answer's authority after a written stage.

The runner accepts one answer per episode (§6.9 One answer per episode). Nothing about a response is stored: no input receipt, audit or claim, so `input show` and `input reconcile` have no permission rows.

Response text is:

```text
Permission response: interaction=INTERACTION_ID run=RUN_ID decision=DECISION choice=CHOICE status=STATUS stage=STAGE output_seq=SEQUENCE
```

Render null values as `-` and append `Reason: REASON` when nonnull. Accepted means answer-key submission, independently of native approval and execution.

Exit 0 means accepted submission. A structured held/rejected/uncertain reply prints the response in the selected text/JSON form and exits 1 (§6.3 Error Messages), with stderr `Error: permission response STATUS for interaction INTERACTION_ID: REASON`. Pre-reply operational failures retain their ordinary error category and include known run/interaction context; usage failures exit 2.

An attempt against a blocked current request fails before any byte. It exits 1 with the primary stderr line `Error: runner control: permission request is blocked: REASON`, followed by the known run/interaction context. Respond checks the blocker at admission, after its admission drain, and again before each answer stage. `REASON` is the bounded explanation `permission inspect` returns as `capability_reason` for that state:

| Reason | Blocking state |
|---|---|
| `permission input is fenced` | Programmatic input is fenced for shutdown, ownership loss or recovery. |
| `manual input accounting failed` | The runner could not account for actual manual input. |
| `observation degraded: CODE` | The observer recorded a gap or degradation with diagnostic `CODE`. |
| `observation transitions pending` | Observed transitions await persistence. |
| `observation update pending` | A retained transition or draft has not completed. |
| `observation incomplete` | Child output was read but its observation has not completed. |
| `terminal input interrupted` | A terminal read was interrupted during the drain. |
| `outer terminal closed` | The outer terminal reached end of input. |
| `native output closed` | The native child's output reached end of file. |
| `terminal output still arriving` | The drain's bounded rounds ended before human and child input were quiet. |
| `terminal reply pending` | An outer-terminal reply prefix awaits completion within its 100 ms classification bound. |
| `permission request is not visible` | The request is obscured, closed or not observed as visible. |
| `permission request is not the current interaction` | Input eligibility is not held for the current permission interaction. |

An answer of the wrong kind for the current interaction, a choice without a tested plan, or a second answer for the episode returns a structured `rejected` response with `zero_bytes` stage and exit 1, before any byte:

| Condition | `reason` |
|---|---|
| `--choice` on an interaction that is not a question | `choice answers apply only to question interactions` |
| `--decision` on a question | `decision answers apply only to permission requests` |
| `--choice` naming no tested plan of the current question | `choice CHOICE_ID has no tested answer for this question` |
| An answer was already written for this episode | `an answer was already sent for this episode` |

The CLI takes semantic decisions and question choices. The pinned runner validates the current stable permission request or question, token, answer capability, manual input and output state, records in memory that the episode has an answer, and sends only a tested single-request choice for a decision, or a tested declared dismissal choice for a question. Unknown capability preserves manual native input. `member prompt`, `member exec`, message previews, receipt settlement and recovery remain separate operations.

#### Error Messages

Application errors print to stderr even with `--json`, which selects only successful output; the permission-response exception (§6.3 Permission commands) also prints its structured held/rejected/uncertain response to stdout, alongside its stderr diagnostic, before exit 1. Input errors use the documented usage/application category; a parse error follows its row below. Dispatch, input-lock, compatibility and deletion-lock errors are specified at their action owners above. The executable prefixes the first stderr line of every application error with `Error: `, including messages this document quotes without it. `<quoted>` in a message follows §6.4 Quoted text.

| Command | Situation | Error message | Exit |
|---|---|---|---|
| (any surface resolving a config dir) | A set config-path variable holds a non-absolute value (empty string and relative paths included); `doctor` reports it as the agent's configuration check instead | §6.3 Config-dir resolution validation | 1 |
| (any surface resolving a config dir) | A set config-path variable or `HOME` holds a value that is not valid UTF-8 (§6.3 Config-dir resolution) | `Error: <VAR> is not valid UTF-8` | 1 |
| (any command except `setup` and `doctor`) | A missing, empty or incompatible database | §6.3 Schema-version guard | 1 |
| (any command with output) | Writing stdout fails with a broken pipe, e.g. `cafleet member list 1` piped into `head -1`; the command stops writing and nothing goes to stderr | — | The command's own result |
| (any command with output) | Writing stdout fails with any other error | `Error: failed to write output: <cause>` | 1 |
| `member prompt` / `member exec` / `member press` / `permission respond` / `input reconcile` | `--from-member-id` is not the live subject fleet's root Director, checked after the schema-version guard and the subject's not-found check, before multiplexer, lock, socket or file access; stdout stays empty | §6.3 Director-only command guard | 1 |
| `input reconcile` | The pinned input has no recorded placement (deregistered member or deleted fleet), so no live runner can synchronize the receipt; the receipt stays uncertain (§6.9 Durable input receipts and settlement) | `Error: receipt reconciliation requires the member's live runner; the input stays uncertain` | 1 |
| (runner queue housekeeping) | A queued receipt is no longer `ready`; the runner fences ordinary input (§6.9 Queue housekeeping) | `queued input INPUT_ID changed to STATE outside the runner queue` | — |
| `member prompt` / `member exec` | Above the plain-input limit and profile framing is `plain`; CAUSE in the command's pre-write error | `the input is N UNIT, above the BACKEND plain-input limit of MAX UNIT, and the BACKEND profile has no bracketed-paste delivery` | 1 |
| `member prompt` / `member exec` | Above the plain-input limit and profile framing is `bracketed_paste`, but mode 2004 is off; CAUSE in the pre-write error | `the input is N UNIT, above the BACKEND plain-input limit of MAX UNIT, and the native CLI has not enabled bracketed paste` | 1 |
| `fleet create` | No tmux server, the pane is not on the chosen server, a malformed `--pane`, or without `--pane` a shared-server caller or an unset or unverifiable env pane; before any write | §6.3 `fleet create` | 1 |
| `member create` | `--coding-agent hermes` with an unreadable `hermes config path`, before registration or multiplexer effects | §6.7 Hermes availability preflight | 1 |
| `member create` | A non-default database and a spawn prompt without `{database_url}`, before registration (§6.3 Spawn-prompt substitution) | `Error: the fleet database is not the default; the spawn prompt must include {database_url} so the member passes --database-url` | 1 |
| `member delete` | A `hermes` run whose work settlement cannot be verified; the member row and runner cleanup records are kept | `Error: hermes work for member <ID> could not be verified as stopped: <cause>; member was not deleted.` | 1 |
| `fleet create` | Director window naming, exact name read-back or pane-context verification fails; active uncommitted registration rolls back (§6.5 Backend operations) | The cause and attempted target diagnostics of § `fleet create` | 1 |
| `fleet create` | Registration or commit fails after naming is attempted | The registration cause, attempted target diagnostics and transaction-resolution outcome of § `fleet create`; rollback failures accompany the original error | 1 |
| `member create` | The fleet layout lock stays busy for the layout lock wait (60 s); nothing is registered (§6.5 Fleet window layout) | `Error: fleet <fleet-id> pane layout remained busy for 60 seconds; member was not created.` | 1 |
| `member delete` | The fleet layout lock stays busy for the layout lock wait (60 s); the run, pane and row are unchanged | `Error: fleet <fleet-id> pane layout remained busy for 60 seconds; member was not deleted.` | 1 |
| `fleet delete` | The fleet layout lock stays busy for the layout lock wait (60 s); every member is unchanged | `Error: fleet <fleet-id> pane layout remained busy for 60 seconds; fleet was not deleted.` | 1 |
| `member create` | An overflow window's height read fails or is unparsable; nothing is registered (§6.5 Fleet window layout) | `Error: window <window-name> (ordinal <ordinal>) of fleet <fleet-id> cannot be read: <cause>. Its active members <member-ids> may have lost their panes; run 'cafleet member delete MEMBER_ID' for each, then retry.` | 1 |
| `member create` | The conditional `last_window_ordinal` update changes a row count other than 1; the registration transaction rolls back (§6.2 Members) | `Error: fleet <fleet-id> window ordinal changed under the layout lock` | 1 |
| `member create` | The tmux `new-window` reply is malformed; the cleanup diagnostic follows (§6.5 Member pane creation) | `Error: unexpected tmux new-window output: <quoted>` | 1 |
| `member create` | The herdr `tab create` reply lacks `result.root_pane.pane_id` or `result.tab.tab_id`; the cleanup diagnostic follows | `Error: herdr tab create missing '<key>' field` | 1 |
| `member create` | A herdr re-parenting move failed; a stderr diagnostic while the spawn continues (§6.5 herdr re-parenting) | `layout re-parenting incomplete: <cause>` | The command's own result |
| `setup` | An extension file or its executable record cannot be installed, or the Hermes `config path` read fails (§6.3 Asset installation, Hermes plugin enablement) | `failed to install extension into <target>: <error>` | 1 |
| `setup` | The Hermes plugin enablement fails (§6.3 Hermes plugin enablement) | `failed to enable hermes plugin cafleet in <config path>: <cause>` | 1 |
| `message listen --codex-thread` | `codex` is not on `PATH`, or no Codex ancestor process exists; before `ready` | §6.3 Codex forwarding | 1 |
| `member create` | `--coding-agent opencode` with the selected agent preset missing or unreadable at the resolved preset path, before registration or multiplexer effects. `<preset>` is the resolved `<preset base>/agents/<mode>.md` for the resolved launch policy, and `<cause>` is the read error | `Error: cannot read opencode agent preset at <preset>: <cause>; run 'cafleet setup --coding-agent opencode' first` | 1 |
| (any command-line argument) | A missing positional subject id (missing-required-argument error naming the positional); a non-integer id or a `permission respond` `--choice` outside the choice-id shape (invalid-value error); neither or both of the positional body and `--file` (`message send` / `member create`) or of `--decision` and `--choice` (`permission respond`) (argument-group error). `permission respond` errors precede lock or byte effects | A parse error — an unknown subcommand or option, a missing required argument, an invalid value, or conflicting arguments — writes to stderr, leaves stdout empty and exits 2 before any command body runs. Its first line begins with `error: ` (lowercase, distinguishing it from the `Error: ` prefix of application errors). It names the offending argument: a missing required argument by its placeholder (for example `<MEMBER_ID>`), an invalid value by the argument and the rejected value, a fixed-value option additionally by its accepted values, and a conflict by both arguments. The rest of its wording is non-normative. | 2 |
| `fleet show` / `fleet delete` / `member list` / `member create` | Unknown fleet ID; for `member create`, raised by the first fleet check, after the schema-version guard and before Director resolution | `Error: fleet '<fleet-id>' not found.` | 1 |
| `member create` | Into a soft-deleted fleet | `Error: fleet X is deleted` | 1 |
| `member create` | The live-fleet guard: the fleet's root Director is not an active member | `Error: fleet <fleet-id>'s root Director (member <id>) is not active.` | 1 |
| `member delete` | Against the root Director's id | `Error: cannot deregister the root Director; use 'cafleet fleet delete' instead` | 1 |
| `message show` | An unknown `MESSAGE_ID` | `Error: Message <message-id> not found` | 1 |
| `message send` | The sender is unknown or inactive | `Error: Sender member not found or not active: <from-member-id>` | 1 |
| `message send` | The recipient is unknown or inactive | `Error: Destination member not found: <to-member-id>` | 1 |
| `message send` | Sender and recipient in different fleets | `Error: members <from-member-id> and <to-member-id> are not in the same fleet.` | 1 |
| `member capture` / `member prompt` / `member exec` / `member press` | The member has a pending placement | <code>Error: member &lt;id&gt; has no pane yet (pending placement) — nothing to &lt;capture&#124;prompt&#124;exec&#124;press&gt;.</code> | 1 |
| `member show` / `prompt` / `exec` / `capture` | An unknown or inactive target member id | `Error: Member <member-id> not found` | 1 |
| `message send` / `member create` | Positional body empty or whitespace-only | `Error: text may not be empty.` | 2 |
| `message send` / `member create` | `--file <path>` to an empty (zero-byte or whitespace-only) file | `Error: --file <path>: file is empty.` | 1 |
| `message send` / `member create` | `--file -` with empty or whitespace-only stdin | `Error: --file -: stdin is empty.` | 1 |
| `message send` / `member create` | `--file <path>` to a non-existent path or non-regular file | `Error: --file <path>: file does not exist or is not a regular file.` | 1 |
| `message send` / `member create` | `--file <path>` to an unreadable file | `Error: --file <path>: file is not readable.` | 1 |
| `message send` / `member create` | `--file <path>` to a file containing invalid UTF-8 | `Error: --file <path>: file is not valid UTF-8.` | 1 |
| `member create` | A `--model` or `--effort` value the selected backend rejects, before any side effect | §6.7 *Model selection* and *Reasoning effort* | 2 |
| `member create` | An unknown `{placeholder}` in the prompt, before registration | `Error: Unknown placeholder '<name>' in custom prompt. Supported placeholders: {fleet_id}, {member_id}, {director_member_id}, {coding_agent}, {database_url}. Double literal braces ({{, }}) to keep them as text.` | 2 |
| `member create` | A malformed brace expression in the prompt, before registration | `Error: Malformed custom prompt: <detail>. Double literal braces ({{, }}) to keep them as text.` | 2 |
| `permission inspect` | An unknown `INTERACTION_ID`, ordered per §6.3 Permission commands | `Error: Interaction <interaction-id> not found` | 1 |
| `permission respond` | `--choice` on a non-question, `--decision` on a question, a choice without a tested plan, or a second answer for the episode; the structured `rejected` response prints on stdout first, with reasons per §6.3 Permission commands | `Error: permission response rejected for interaction <interaction-id>: <reason>` | 1 |
| `permission inspect` | The inspected run records a database identity other than the selected database's; the known interaction context follows | `Error: invalid stored value for member_runs.database_identity_json: selected database identity mismatch` | 1 |

Left-in-composer prompt/exec errors append known receipt identity as `input_id=INPUT_ID`. In the plain-limit errors, UNIT is `bytes` for a byte limit and `characters` for a scalar limit; N measures rendered prefix plus literal. Uncertain errors retain their command forms with the recorded cause, for example `write_failed`.

### 6.4 Output & Formatting

A message is a durable point-to-point delivery. The recipient owns its inbox, and its poll consumes every message it returns.

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

`to_member_id` is always an integer. The recipient's consuming poll (§6.3 `message poll`) changes `status_state` to `completed` and stamps `status_timestamp` with the consumption time; completion means consumption, independent of the assignment's progress.

#### Rendered shape

§6.3 Output shapes owns each command's wrapper. Poll returns an array of full records it consumed; show wraps one record as `message`; send wraps the stored record as `message`; listen carries message IDs, not the full record, in tagged `pending` notices, with separate `ready`/`closed` lifecycle records, and leaves consumption to the poll the notice prompts. Capture timestamping, hashing, mode selection and text output follow §6.3 `member capture`, and its JSON key order §6.3 Output shapes.

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

Text bodies use a fixed 200 codepoints plus `…`; `--json` preserves the complete exact text. Pane preview lines, their folding, inline limit and long-body `message poll` pointer follow §6.5 Push notifications.

#### Runtime observation messages

A runtime notification is an ordinary inbox record whose `text` is the complete versioned event JSON. The pinned observed member is its sender and the Director its recipient; verify the durable event/message association through `event show` before acting. Its typed source distinguishes runner observation from model-authored speech. Version-2 permission-opening events carry a broker-built `director_action` (review instruction and inspection argv), validated against the stored event/Director association (§6.9 Request-bound permission responses). The Director's poll consumes the message independently of the attributed decision, answer submission, event visibility and actual continuation. §6.9 Persistence, notification and read interfaces owns the evidence object, atomic association and archived recovery.

#### ANSI stripping and CR-defrag

The CSI pattern (§6.9 Pattern syntax) is exactly `\x1b\[[0-?]*[ -/]*[@-~]`: ESC, `[`, any run of parameter bytes (`0x30`–`0x3F`), any run of intermediate bytes (`0x20`–`0x2F`), then one final byte (`0x40`–`0x7E`). Only CSI sequences are stripped; OSC, DCS and single-character escapes deliberately remain. After stripping, split on `\n`; per line, trim the trailing run of `\r` (a CR with nothing written after it, such as a CRLF terminator's, overwrites nothing), then keep only the substring after the **last** remaining `\r` (CR-redraw defrag: a TUI redraw `prefix\rNEW` keeps `NEW`, a CRLF-terminated `row\r\n` keeps `row`); re-join with `\n`. An empty input yields the empty string.

#### Formatter helper semantics

Absent ordinary member/fleet cells use ASCII `-`. ISO-to-HMS takes the first eight characters after `T`; an absent value or one without a `T` renders `-`. Idle renders null as `-`, seconds below 60, floor minutes below 3600 and floor hours afterward. Required fields fail if absent; nullable activity and pending placement are documented valid cases.

##### Quoted text

`TEXT_REPR` and `<quoted>` denote the value wrapped in `"` with these escapes: `"` → `\"`, `\` → `\\`, U+0009 → `\t`, U+000A → `\n`, U+000D → `\r`, U+0000 → `\0`; any other scalar whose general category is Cc, Cf, Cs, Co, Cn, Zl, Zp or Zs (except U+0020), or which has the Grapheme_Extend property, → `\u{H}` with lowercase hex and no leading zeros. Every other scalar, including `'`, is written as itself. Categories and properties follow the Unicode version the implementation ships; the rendering of a scalar whose classification differs between Unicode versions is non-normative.

#### JSON encoding

Every JSON document the `cafleet` binary writes — `--json` output, JSONL listener records, stored JSON columns and inbox payloads — is compact: no whitespace between tokens, object members in the order this specification lists, arrays in order. Strings escape `"` as `\"`, `\` as `\\`, U+0008 as `\b`, U+000C as `\f`, U+000A as `\n`, U+000D as `\r`, U+0009 as `\t`, and every other code point below U+0020 as `\u00` followed by two lowercase hex digits; every other scalar, including `/`, U+007F, non-ASCII text, U+2028 and U+2029, is written raw as UTF-8. Numbers in these documents are integers, written in decimal with a leading `-` when negative.

Canonical JSON is this encoding with every object's members sorted by the UTF-8 bytes of their keys, recursively. A digest named "SHA-256 of the canonical JSON of X" is the lowercase hex SHA-256 of those bytes.

JSON that a bundled extension emits on its host's behalf is any valid JSON; the binary parses it by value, independent of whitespace, escaping and member order.

### 6.5 Multiplexer (tmux + herdr)

CAFleet hosts coding-agent sessions in tmux or herdr panes. Both backends are driven as CLI subprocesses, and wrapped input on both goes through the pinned PTY runner control socket. The adapters preserve pane ownership, layout, literal input and bounded subprocess behavior.

#### Backend matrix

| Behavior | tmux | herdr |
|---|---|---|
| Pane ID example | `%7` | `w1:p1` |
| Window | tmux window | herdr tab |
| Spawn working directory | Splitting pane inheritance; explicit `new-window -c` for an overflow window | Explicit `--cwd` |
| Overflow window creation | `new-window -d -n <name>` | `tab create --label <name> --no-focus` |
| Member pane name | Pane title through `select-pane -T` | Pane label through `pane rename` |
| Column equalization | `select-layout -E` per column | Arithmetic `pane resize` per column, with re-parenting for a full-height column |
| Capture for `member capture` | `capture-pane -p -e` | `pane read --format ansi` |
| Delete layout | Best-effort per-column equalization of the closed pane's recorded window | Best-effort restoration of the closed pane's recorded tab |

#### Backend selection

`fleet create` and the `cafleet doctor` `multiplexer` and `multiplexer.caller_pane` rows are the only readers of the caller's multiplexer environment. Both run the shared-server check before reading it and verify the env pane afterwards (§6.3 Caller pane verification). Both use one resolver, where a non-empty `HERDR_ENV` signals herdr and a non-empty `TMUX` signals tmux; an empty value counts as unset:

| `HERDR_ENV` | `TMUX` | Result |
|---|---|---|
| non-empty | non-empty | Error `ambiguous multiplexer environment: both HERDR_ENV and TMUX are set; unset the one that does not belong to this session` |
| non-empty | unset or empty | herdr |
| unset or empty | non-empty | tmux |
| unset or empty | unset or empty | Error `no supported multiplexer detected: neither HERDR_ENV nor TMUX is set; run cafleet inside a tmux or herdr session` |

An explicit `fleet create --pane` selects the backend by its ID form instead of this resolver (§6.3 Caller pane verification). `fleet create` records the backend, its server and the Director's pane context (§6.3 `fleet create`); `cafleet doctor` reports the detected backend, its server and the caller pane verification (§6.3 `cafleet doctor` — Environment Diagnosis). Every other multiplexer-driving command — `member create`, `member delete`, `fleet delete`, `member prompt`, `member exec`, `member press`, `member capture`, runner recovery and Hermes work settlement — builds it from the fleet Director's placement row (backend, `mux_server`, session, window, pane) and targets the recorded server explicitly (§ Multiplexer context).

#### Error taxonomy

Both backends report failures through one multiplexer error type carrying the backend's own message text, so CLI boundaries handle both uniformly. A creation error also records its pane-cleanup outcome (below), and a key-dispatch error whether a writing process had started.

#### Pane ownership during creation

The member pane spawn owns a newly created pane until it returns successfully. The backend takes ownership as soon as it extracts the pane id from the split, `new-window` or `tab create` response; herdr closes that pane itself when the subsequent `pane run` fails, returning the original run error. An owned-pane close inside the spawn receives the Column target's window ID or the window ID parsed from the `new-window` / `tab create` reply. A creation error carries one of these pane-cleanup outcomes:

| Outcome | Meaning and caller action |
|---|---|
| Closed | The backend already closed the known pane, tolerating an already-missing pane; the caller performs the remaining database compensation without another close |
| Close failed | The backend tried to close the known pane and failed; the error keeps the pane id and the original error and appends `cleanup failed for pane <id>: <detail>`, and the caller performs the remaining compensation without retrying the close |
| Unknown | A failed or malformed split, `new-window` or `tab create` response left no confirmed pane id; the error appends `pane ID unknown; pane cleanup unconfirmed`, and the caller closes no pane |

On success, the spawn returns the pane ID and its window ID, and ownership transfers immediately to member creation, whose compensation follows the creation failure order (§6.3 Creation failure compensation) and closes only the pane owned by that operation, passing the spawned window ID with it. Pane naming, column equalization and herdr re-parenting are best effort (§ Member pane creation).

#### Subprocess output and deadlines

The shared subprocess runner captures stdout and stderr without truncation. With a timeout, it drains both pipes concurrently from spawn using nonblocking I/O: each loop checks the monotonic deadline and the direct child's exit status, then reads at most 64 KiB per stream, polling at most 20 ms or the remaining deadline, whichever is shorter. Interrupted operations retry against the same deadline. Completion requires the child's exit and EOF on both streams. Success returns stdout, and a nonzero exit reports stderr, decoded as UTF-8 with each maximal ill-formed subsequence replaced by one U+FFFD. Calls without a timeout retain the unbounded output-collection path.

At the deadline, including when the direct child has exited but a descendant still holds a pipe open, the runner kills and reaps the direct child, closes both read pipes and reports a timeout; descendant termination is outside this contract. FD setup, read, poll and child-status failures also release the pipes and kill/reap the child; cleanup failures accompany the primary error instead of replacing it. The deadline bounds observation and the start of cleanup; an unresponsive operating system can delay cleanup itself.

#### Pane spawn working directory

A member pane spawned by `cafleet member create` starts in the invoking process's working directory (the Director's pane cwd): a tmux split inherits it from the splitting pane and a tmux overflow window receives it as `new-window -c <cwd>`, and herdr receives it verbatim as `--cwd` rather than relying on herdr's own inheritance, which spawns `/bin/sh` instead of the passwd login shell when `SHELL` is unset ([herdr discussion #1517](https://github.com/ogulcancelik/herdr/discussions/1517)). An unresolvable cwd fails the spawn with `cannot resolve the working directory for pane spawn: <cause>`; there is no fallback directory.

#### Delete-time pane layout

Every close receives its window ID from the caller: deletion passes the placement's recorded `mux_window_id`, creation compensation the window ID the spawn returned. Neither backend reads the pre-close window from the multiplexer.

Deletion closes the pane (tmux `kill-pane`, herdr `pane close`), then, after registry finalization and within the original operation deadline, restores the recorded window. tmux equalizes every column of the window (§ Member pane creation), and a `list-panes` failure whose stderr contains `can't find window` means the closed pane was the window's last, leaving nothing to restore. herdr applies the delete-time restoration of § herdr column equalization to the recorded tab. Creation compensation and the spawn's owned-pane rollback run the same close and restoration in one call. An emptied overflow window closes natively on both backends; the Director window always keeps the Director pane. A failed or expired restoration emits a stderr diagnostic; `member delete` retains its successful close and deregistration result.

#### Pane input lock

Every operation that writes pane input uses one process-shared OS file lock. Acquire the lock before placement revalidation. For wrapped prompt/exec/preview submissions the runner owns the lock: wrapped clients validate and submit without holding it while awaiting the runner, and the runner acquires it nonblockingly from its event loop under the attempt deadline, revalidates the concrete placement and retains it across an active staged write through durable receipt/stamp settlement (§6.9 Runner-owned deferred delivery). A merely queued request holds no lock. Explicit key requests hold it through their outer-pane dispatch and bounded cleanup. Hold the lock through exactly one open file descriptor and close it on every exit path; no duplicate of that descriptor outlives the operation.

| Property | Contract |
|---|---|
| Directory | `<canonical-database-file>.pane-locks/` adjacent to the database |
| Key | SHA-256 of UTF-8 `backend + NUL + mux_session + NUL + mux_pane_id`, shared by duplicate placements for one pane |
| File | `<key>.lock`, read/write create without truncation, owner-only creation permissions |
| Acquisition | Exclusive nonblocking OS file lock; preserve unexpected errors with path/member context |
| Identity | Keep lockfiles stable while processes may use the database; unlinking a held file would let another process lock a different inode for the same pane |
| Contention | Deletion, wrapped clients and explicit keys retry a busy lock every 50 milliseconds; explicit keys stop waiting one second before their deadline. The runner retries an active attempt's lock on each event-loop pass and a queued request's lock every 100 milliseconds; a lock still busy at its wait bound fails that writer |

No database statement runs while a writer waits for the lock.

| Writer | Lock still busy when the wait ends |
|---|---|
| `member prompt`, `member exec` | Exit 1 with the runner input failure and recorded runtime context (§6.3 Runtime presentation) |
| `member press` | Exit 1 with the primary and zero-dispatch diagnostics (§6.3 `member press`) |

Process exit releases ownership. Read-only capture needs no input lock. Deletion locking follows §6.3 `member delete` and `fleet delete`.

#### Termination during pane input

Interrupting a host turn, including through Escape-first delivery fallback, can also end the turn's in-flight shell tool, such as a running `cafleet` command. The CLI therefore defers `SIGINT`, `SIGTERM` and `SIGHUP` across every pane-input and lifecycle span: a signal received meanwhile takes its default action when the outermost span ends; spans nest.

| Command | Span |
|---|---|
| `member prompt`, `member exec` | From the runner request through its reply |
| `member press` | Begin before acquiring the lock; retain it through the writing process and bounded cleanup, releasing the lock before restoring signals |
| `member create` | From fleet layout lock acquisition (§ Fleet window layout) through `started` or the end of its compensation |
| `member delete`, `fleet delete` | From pane lock acquisition through fleet layout lock acquisition, deregistration and layout cleanup |

Every span is bounded by its own deadline, so a deferred signal waits at most one span. Multiplexer client processes started inside a span inherit the blocked set and stay bounded by their timeouts; the member session, started by the multiplexer server, does not inherit it.

`SIGKILL` cannot be deferred; a kill inside a span leaves:

| Killed command | State left | Discovery and recovery |
|---|---|---|
| `member prompt` / `member exec` after typing began | Partial or complete input | `member capture`; re-dispatch only after inspecting the pane |
| `member press` after its writing process starts | Any prefix, including empty or complete, may have been handled | Inspect the pane before an explicitly authorized new action |
| `member create` | A pane without a registry row, or a row whose placement stays pending; the OS releases the fleet layout lock, and a consumed overflow ordinal stays consumed | `member list` and `member capture`; `member delete` the row, and close an unregistered pane with the multiplexer's own kill command |
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

Accept only these productions, with the CLI's count/UTF-8 bounds and complete-vector validation. The dedicated key adapter sends only requested keys, using shell-free argv and no prompt phase or settling sleep.

Each backend exposes target validation against the recorded placement and key sending, which accepts only a validated vector (canonical names plus backend arguments) and performs the single bounded writing invocation. The press entry keeps its original connection and deadline and holds the validated lock through process cleanup.

Acquire the canonical pane-input lock shared with deletion and validate its retained descriptor/path binding. Build the adapter from the fleet Director's recorded placement (§ Backend selection) and, on that recorded server, verify the target's complete tuple against the recorded session/workspace and window: tmux reads `#{pane_id}|#{window_id}|#{session_name}` for the target and splits only the first two separators; herdr reads its pane-list JSON and requires exactly one matching workspace/tab/pane. Re-read the active member after metadata, compare its fixed fleet/kind/full placement and validate the held lock immediately before writing. A missing, ambiguous, malformed or changed target fails before the writing invocation.

Validation establishes the tuple within the current recorded server only: the placement records no socket/server-generation identity, so it supplies no cross-server fallback or proof of historical server/build equivalence. Keep one adapter/environment across every probe and the write.

Capture one inherited-or-new five-second invocation deadline before invocation database validation. Every press process — the availability, current-context and target-metadata probes and the write, for `member press` and every other pane-input caller — runs through the shared bounded subprocess runner (§ Subprocess output and deadlines) with a timeout equal to the remaining invocation deadline, so its spawn, collection and cleanup follow that runner's contract. The lock wait reserves one second for metadata/dispatch/cleanup (not the prompt reservation). Every later check uses the remaining invocation deadline, and an expired deadline starts no new writing process.

Both writing adapters acknowledge only successful process completion with empty/whitespace stdout. Unexpected non-whitespace stdout on success is unreadable acknowledgment with uncertain effects; herdr metadata retains its command-specific JSON parsing. Backend keyboard modes or key interception can affect actual PTY bytes even after acknowledgment. Use the CLI result/error contract (§6.3 `member press`), including its zero-write and no-retry cases, and observe actual native effects separately.

#### Prompt dispatch

Every ordinary member uses a pinned, healthy control socket and its runner's owned child PTY (§6.9 Input arbitration and the original native answer). The runner selects profile delivery without Escape, zero-byte queueing or Escape-first delivery fallback by §6.9 Delivery method and interaction priority. Uncertain reachability preserves manual terminal use while holding generated input.

Validate a nonblank literal line without CR/LF or C0/DEL controls before starting any process. No input path captures the pane or consults its state. Native shell input starts with `! ` followed by the actual validated command; composer controls begin the submitted input.

An OpenCode member (`opencode mini`, §6.7 Per-backend spawn argv) receives pushed notifications and direct prompts as submitted user turns. Native `! ` input executes a shell command.

Deletion and prompt/exec operations start their five-second budgets after the invocation's schema-version guard. Resolve native command bodies before starting their input budget. Runner-owned staged writes use the original attempt deadline (§6.9 Runner-owned deferred delivery).

A started write with an ambiguous result retains uncertainty; failure after submission can mean the command ran. Inspect native output and durable receipts before any newly authorized retry. The lock excludes CAFleet writers while the user's keyboard and host UI remain independent; successful dispatch establishes submission only.

#### Push notifications

Active ordinary recipients with concrete panes receive previews from their own runner: `message send` only stores the message (§6.3 `message send`), and the recipient's runner types each pending message at its next preview sweep (§6.9 Runner pending-preview sweep). The root Director receives through native `message listen` (§6.3) or manual broker reads, with zero automatic pane input. Self-sends persist without pane input. A member with no live runner keeps its messages pending.

Profile delivery submits unchanged the canonical wrapped preview, this exact line ending with the resume clause:

```text
[cafleet msg MESSAGE_ID from SENDER_ID TIMESTAMP] BODY [cafleet] Resume your work if something was still running.
```

Delivery fallback (§6.9 Delivery method and interaction priority), whose submission leads with Escape, uses the full line with both clauses:

```text
[cafleet msg MESSAGE_ID from SENDER_ID TIMESTAMP] BODY [cafleet] Resume your work if something was still running. [cafleet] If this delivery's Escape dismissed a pending prompt in your pane, that rejection did not come from the user; re-issue the tool call if you still need it.
```

Stored text remains exact. Both forms fold CRLF/LF/CR to `⏎`, tabs to `␉`, and other C0/DEL controls to visible hexadecimal escapes (`\xNN`). A folded body of at most 4000 Unicode scalar values appears inline; a longer body becomes `Body is N characters; run cafleet message poll RECIPIENT_ID --json to read it.`, where N counts stored text and `RECIPIENT_ID` is the message's `to_member_id`. The pointer is identical for every database and never carries `--database-url`. A preview above its selected profile's measured plain-input limit uses bracketed paste when declared and enabled, otherwise this same pointer form (§6.9 Measured submission plans and settle intervals). This limit is independent of the 200-codepoint text-output limit (§6.4 Text-mode projection). The runner renders the form from the selected delivery method before its first byte.

The runner stamps `notified_at` once a preview is submitted: confirmed by post-submit verification on a verified-composer run, or with every stage written on a run without composer verification, where an uncertain preview is also stamped before its result is published (§6.9 Runner pending-preview sweep).

Delivery leaves `status_state` unchanged: an inline or pointer preview stays `input_required` until its recipient polls, and the consuming poll identifies consumption independently of input delivery.

##### Held delivery

A preview stays pending, with `notified_at` unset, while the recipient's composer is not verified empty on a verified-composer run, an interaction is open, the runner's FIFO holds prompt or exec input or is stopped, the observer is degraded, ownership or storage is unavailable, the pane is not yet placed or the pane input lock is busy; the runner's sweep retries it with no caller command. Pending delivery creates no permission event and grants no native answer.

##### Accepted trade-offs

These trade-offs apply to delivery fallback, which leads with Escape. A single Escape's effect depends on the backend, which may need more than one Escape to interrupt a running session. Delivery is correct whatever Escape does: when Escape does not interrupt, the line is still typed into the composer and the host submits or queues it, and both conditional trailing clauses stay accurate.

- Where Escape interrupts an ordinary member's working turn, the resume clause tells the recipient to continue.
- Where Escape dismisses an ordinary member's question, confirmation or approval prompt, the host can report a user rejection before the input arrives; Claude Code reports `The user doesn't want to proceed with this tool use.` The preview's dismissed-prompt clause marks that rejection as delivery-caused, and the member re-issues the required tool call or re-asks the question. `member prompt` and `member exec` input carries no clause; the Director checks the pane for an open prompt before that dispatch.
- Escape does not reliably clear typed composer text on every host; unsent text in an ordinary member's pane can be submitted together with the delivered line.
- A failed or interrupted notification is retried, so a recipient can see the same `[cafleet msg ID …]` line more than once; it polls and deduplicates by ID.
- Two sends in quick succession from different senders each lead with Escape; the second can interrupt the turn the first delivery started.
- A wrapped runner suppresses fallback for detected and candidate interactions only (§6.9 Delivery method and interaction priority). On a run with unknown or partial detection coverage, fallback can dismiss an undetected permission request or question; a changed native UI can trigger fallback while its unsupported permission answer remains unavailable.
- OpenCode `mini` captures can expose only the status bar. Inspect retained session records for submitted input and native shell output when capture omits them; a message's `completed` state, read with `message show`, establishes its consumption by the member's poll.

Verified-composer delivery additionally accepts these trade-offs (§6.9 Durable input receipts and settlement):

- A preview whose submission is uncertain is typed again once the composer is verified empty, so a recipient can see it twice, for example after a human or Director Enter submitted the typed line; recipients deduplicate by message ID.
- A human draft blocks previews until it is cleared or submitted.
- After a prompt or exec records `uncertain`, no preview and no queued input reaches the member until `input reconcile` settles it (§6.9 Runner-owned deferred delivery).
- A composer rule that never verifies empty after clearing keeps previews and queued input waiting. `input_delivery_stalled` signals the Director after 3 minutes; recovery inspects the whole literal and presses Enter when appropriate, sends other keys the capture shows are needed, or asks the user to clear or submit the composer, and reconciles uncertain input from that evidence. A composer that never verifies empty after clearing is a profile defect reported for correction.
- Human typing during the write can mix with the typed literal, and a human key during the post-submit verification window can make the composer nonempty; the result is then `submit_unconfirmed` and a preview can be delivered twice.

#### Multiplexer context

A multiplexer context is a plain value of three non-empty strings: `session` (tmux session name or herdr workspace ID), `window_id` (tmux window or herdr tab, e.g. `@N`) and `pane_id` (e.g. `%N`). `fleet create` reads it for the Director's pane on the chosen server and records it with that server as `mux_server`, which every later CAFleet invocation for the fleet targets explicitly:

| Backend | `mux_server` holds | Every CAFleet invocation after `fleet create` |
|---|---|---|
| tmux | The absolute server socket path | `tmux -S <mux_server> …` |
| herdr | The herdr session name (`default` for the default session) | `herdr --session <mux_server> …` |

Each member placement copies the Director's `mux_server`. `fleet create` names the resolved Director window and verifies both its exact name and unchanged pane context before committing (§ Backend operations); the multiplexer context itself remains three strings.

A member pane spawn takes the slot's pane target (§ Fleet window layout), built from the registry and the Director's recorded session:

| Target | Fields |
|---|---|
| Column | The window ID, the column side, the column's concrete member panes in registry order and the opposite column's panes (the Director pane for ordinal 0) |
| New window | The Director's recorded session and the overflow window name |

The spawn returns the new pane ID and its window ID. A missing or invalid created-pane ID is an unconfirmed cleanup, never a license to infer another target (§ Pane ownership during creation).

#### Fleet window layout

A fleet spreads its member panes across several windows. Every `member create` takes the first free slot in a height-derived column layout and opens an overflow window when every column is full.

| Term | Meaning |
|---|---|
| Window | A tmux window or a herdr tab |
| Director window | The window recorded in the Director's placement at `fleet create`; ordinal `0` |
| Overflow window | A window CAFleet opens for the fleet; ordinal `1` or greater |
| Column | The `left` or `right` half of a window; the Director window's left column holds only the Director |
| Slot | One member position in a column |

| Constant | Value |
|---|---|
| Minimum pane height | 15 rows |
| Maximum panes per column | 8 |
| Layout lock wait | 60 seconds |

The constants are fixed; no flag or configuration changes them. A column in a window whose live height is `H` rows has capacity `capacity(H) = min(8, floor(H / 15))`. `H` is the tmux `#{window_height}`, which excludes the status line, or the herdr `layout.area.height`; the 15-row minimum is a per-slot budget that includes the tmux pane separator. The Director window's right column holds `capacity(H)` members and is skipped when that is 0. An overflow column holds `max(1, capacity(H))` members, so a new overflow window always makes progress.

| Window height `H` | Capacity per column | Director window members | Overflow window members |
|---|---|---|---|
| < 15 | 0 | 0 (skipped) | 2 (one per column, by the overflow minimum) |
| 15 | 1 | 1 | 2 |
| 30 | 2 | 2 | 4 |
| 50 | 3 | 3 | 6 |
| 60 | 4 | 4 | 8 |
| ≥ 120 | 8 | 8 | 16 |

##### Slot selection

1. Scan the fleet's windows by ascending ordinal. For ordinal 0, consider only the right column. For each overflow window, consider the left column, then the right column.
2. Choose the first column whose member count is below its capacity.
3. Otherwise open a new overflow window with ordinal `fleets.last_window_ordinal + 1`, named `cafleet-<fleet_id>-<ordinal>`; its first member takes the left column, which the overflow minimum always makes available, so a new window needs no height read.

Occupancy comes from the registry: the fleet's active members whose placement is concrete, grouped by `window_ordinal` and `column_side`, excluding the Director's own row (ordinal 0, `left`). A pending placement holds no slot: every creation holds the fleet layout lock until it completes or compensates, so a pending row seen under the lock belongs to a killed creation (§ Termination during pane input), which `member delete` removes. An overflow ordinal with no concrete member is a closed window and is not scanned.

Each scanned window's height is read live before registration (§ Backend operations Window height). The anchors are the Director pane for ordinal 0 and every concrete member pane of the window, in registry order, for an overflow window, so one member whose pane died does not make a live herdr tab unreadable. A failed Director-window read propagates the multiplexer error unchanged. A failed or unparsable overflow-window read (the user closed the window, or every member pane in it died while the members stayed registered) fails `member create` with the unreadable-window error (§6.3 Error Messages), whose `<member-ids>` lists the window's active member IDs ascending, separated by `, `. Neither failure registers anything.

##### Placement policy

- **First free slot.** A slot freed by `member delete` is reused by the next creation, so a newer member can land in the Director window while overflow windows hold older members.
- **No migration.** Existing members never change window or column; capacity changes from a resized terminal affect only later placements. herdr re-parenting (§ herdr re-parenting) moves a pane only visually inside its own window, keeping its pane ID, window and column.
- **Ordinals only increase.** `fleets.last_window_ordinal` advances in the registration transaction of every new-window choice and never decreases, even when that creation later fails, so a fleet's overflow windows can skip a number.
- **Emptied overflow windows close.** When the last member of an overflow window is deleted, the window closes natively on both backends. The Director window always stays open.

##### Window and pane naming

| Object | Name | Mechanism |
|---|---|---|
| Overflow window | `cafleet-<fleet_id>-<ordinal>`, e.g. `cafleet-3-1` | tmux `new-window -n`, which also turns `automatic-rename` off for that window; herdr `tab create --label` |
| Director window | `cafleet-<fleet_id>-0` | tmux `rename-window`, which also turns `automatic-rename` off for that window; herdr `tab rename`; verify the name before registration commits (§ Backend operations) |
| Member pane | `<member_id> - <member_name>`, e.g. `12 - drafter` | tmux `select-pane -T`; herdr `pane rename` |

Window names are creation-time placement snapshots; existing registrations retain their recorded names, and renaming a window later leaves the registry unchanged. Fleet deletion leaves the Director window's current name in place. Director window naming and verification are required for successful registration. Ordinary member pane naming is best effort. A tmux pane title is visible only when `pane-border-status` is on, and a coding agent may retitle its pane (§6.7 Backend capabilities); a herdr pane label is independent of the agent's terminal title.

##### Fleet layout lock

A fleet-scoped OS file lock serializes every operation that creates or closes a member pane, beside the pane input lock (§ Pane input lock).

| Property | Contract |
|---|---|
| Directory | `<canonical-database-file>.layout-locks/` adjacent to the database, created on demand |
| Key | SHA-256 hex of the fleet ID's UTF-8 decimal text |
| File | `<key>.lock`, created `0600` |
| Acquisition | Exclusive OS file lock retried every 50 milliseconds within the layout lock wait (60 s), a budget separate from each command's operation deadline; the OS releases it when its holder exits, including after `SIGKILL` |

| Holder | Acquired | Held through |
|---|---|---|
| `member create` | After prompt validation, before resolving the Director placement; the ten-second creation deadline starts after acquisition | `started`, or the end of compensation |
| `member delete` | After its pane lock, before any cleanup step | Deregistration and layout restoration |
| `fleet delete` | Once, after all its pane locks and before the first member's cleanup | The last member's layout restoration |

A deletion's operation deadline moves later by the layout-lock wait. Expiry exits 1 with the holder's busy error (§6.3 Error Messages) and changes no state. Lock order is always pane lock → layout lock; `member create` takes no pane lock and a runner's own delivery takes only its pane lock, so no cycle exists.

Creation holds the lock until it completes because before publication its new pane is invisible to other creations: occupancy counts only concrete placements, and the pane ID is published only after the `prepared` handshake (§6.9 Runner boundary and creation transaction). After publication, `start` can still fail, and its compensation closes the published pane, which a concurrent creation could otherwise already target as a split anchor, bottom pane or re-parenting move. A creation holds the lock for at most its ten-second deadline plus compensation, so the 60-second wait admits several queued creations.

#### Capture windowing

Both backends reject `lines <= 0` before any process with `capture_pane: lines must be positive, got <lines>` and over-fetch by a fixed 1000 lines using their exact argv below. Split raw output on `"\n"` only, preserving `\r` for the CLI's later CR-defrag. Drop the trailing run of visually blank lines: test each line for whitespace-only content after per-line CSI stripping, retaining the original bytes of kept lines. This recognizes TUI-painted empty rows without altering retained ANSI sequences.

Return the last `lines` remaining lines joined by `"\n"`, with no trailing newline; an all-blank buffer becomes the empty string. Preserve interior blank lines. Enforce the last-N window client-side even when the backend returns more than requested, so the window shows the drawn bottom rather than blank cursor space.

#### Backend operations

Both backends run their CLI as an argv subprocess without a shell. Opaque pane IDs are
passed verbatim. Every argv below starts with the recorded server selector,
`tmux -S <mux_server>` or `herdr --session <mux_server>` (§ Multiplexer
context), shown as plain `tmux` and `herdr`. Prompt and key operations use the
argv in § Prompt dispatch and § Explicit key dispatch.

| Operation | tmux argv | herdr argv | Failure handling |
|---|---|---|---|
| Availability | `tmux` on `PATH` | `herdr` on `PATH` | `<backend> binary not found on PATH` |
| Pane context (`fleet create` and `doctor`) | <code>tmux display-message -p -t &lt;pane&gt; "#{session_name}&#124;#{window_id}&#124;#{pane_id}"</code>, split on the first two <code>&#124;</code> | `herdr pane current --pane <pane>`; `result.pane` supplies `workspace_id`, `tab_id`, `pane_id` | tmux: `unexpected tmux display-message output: <quoted>`; herdr: `herdr <context> missing '<key>' field`; an unknown pane fails per §6.3 `fleet create` |
| Pane process (`fleet create` and `doctor`, §6.3 Caller pane verification) | <code>tmux display-message -p -t &lt;pane&gt; "#{pane_id}&#124;#{pane_pid}"</code>, split on the first <code>&#124;</code> | `herdr pane process-info --pane <pane>`; `result.process_info.shell_pid` supplies the pid | tmux: an empty `pane_id` or one that differs from `<pane>` is a missing pane, and with a matching `pane_id` a pid that is not a positive integer fails with `unexpected tmux display-message output: <quoted>`; herdr: error code `pane_not_found` is a missing pane, and a missing or non-integer field fails with `herdr pane process-info missing 'shell_pid' field` |
| Rename window (`fleet create`) | `tmux rename-window -t <window_id> <name>` | `herdr tab rename <tab_id> <name>` | Propagate command failures; herdr also requires its JSON `result` envelope. Each value is one argv element on the resolved server |
| Window name (`fleet create` verification) | <code>tmux display-message -p -t &lt;window_id&gt; "#{window_id}&#124;#{window_name}"</code>; remove exactly the final LF, split on the first <code>&#124;</code>, require the echoed window ID and preserve the remaining name exactly, including CR and embedded LF | `herdr tab get <tab_id>`; require `result.tab.tab_id` to equal the requested ID and a string `result.tab.label` | Propagate command failures and reject malformed output, missing fields and mismatched IDs; an empty name is valid for the reader, and registration requires exact equality with its canonical name |
| Window height (`member create`) | `tmux display-message -p -t <window_id> "#{window_height}"`; the anchors are ignored | `herdr pane layout --pane <anchor>` for each anchor in order; the first successful `result.layout.area.height` is the height | tmux: output that is not a positive integer, including the empty line of a missing window, fails with `unexpected tmux display-message output: <quoted>`; herdr: a reply without the area fails with `herdr pane layout missing 'area' field`, any failure moves to the next anchor, and the read fails with the last anchor's error only when no anchor answers |
| Spawn member pane | Per pane target, § Member pane creation | Per pane target, § Member pane creation | A failed or malformed creation response is an unknown pane-cleanup outcome (§ Pane ownership during creation); naming, equalization and re-parenting failures never fail the spawn |
| Capture | `tmux capture-pane -p -e -t <pane> -S -<lines + 1000>` | `herdr pane read <pane> --format ansi --source recent-unwrapped --lines <lines + 1000>` | `lines <= 0` rejection (§ Capture windowing) |
| Close | `tmux kill-pane -t <pane>`, then the restoration of the caller-supplied window | `herdr pane close <pane>`, then the delete-time restoration of the caller-supplied tab | Close tolerates only a recognized pane-gone cause (tmux stderr `can't find pane` / `no such pane`, herdr error code `pane_not_found`); restoration is best effort (§ Delete-time pane layout) |

`pane run` submits one text line into the pane's shell, so the argv command is rendered as a single POSIX-shell-quoted string. Every herdr JSON reply is parsed as a `result` envelope; non-JSON output fails with `herdr returned non-JSON output: <quoted>` and a reply without a `result` object with `herdr output missing 'result' object: <quoted>`. `<quoted>` follows §6.4 Quoted text.

Process failures preserve their context in the multiplexer error:
`<backend> binary not found: <detail>`, `<backend> command was not started:
<detail>: <argv>`, `<backend> command timed out at the pane input deadline:
<argv>` for a deadline-bounded call and `<backend> command failed: <argv>` followed by a
`stderr: <trimmed stderr>` line. Other errors propagate.

#### Member pane creation

The spawn creates the member pane for its pane target (§ Multiplexer context) without stealing focus, names it with the member pane name (§ Window and pane naming) and equalizes every column of its window. Naming, equalization and herdr re-parenting are best effort: their failures never fail the spawn.

tmux creates the pane with one argv per target:

| Target | Creation argv |
|---|---|
| Column with member panes | `tmux split-window -v -d -t <last column pane> -P -F "#{pane_id}" <command>...` |
| Empty `right` column | `tmux split-window -h -f -l 50% -d -t <first opposite pane> -P -F "#{pane_id}" <command>...` |
| Empty `left` column (an overflow window whose left column emptied) | `tmux split-window -h -f -b -l 50% -d -t <first opposite pane> -P -F "#{pane_id}" <command>...` |
| New window | <code>tmux new-window -d -t &lt;session&gt;: -n &lt;window_name&gt; -c &lt;cwd&gt; -P -F "#{window_id}&#124;#{pane_id}" &lt;command&gt;...</code> |

A Column target's window ID is the target's own. A `new-window` reply splits on the first `|` and requires both fields non-empty; any other reply fails with `unexpected tmux new-window output: <quoted>` and an unknown pane-cleanup outcome. After creation tmux runs `tmux select-pane -t <new pane> -T <member pane name>`, then equalizes every column of the window: `tmux list-panes -t <window> -F "#{pane_id}|#{pane_left}"` groups the panes by `pane_left`, and `tmux select-layout -E -t <first pane of the group>` runs for each group of 2 or more panes. Deletion runs the same equalization (§ Delete-time pane layout).

herdr reads every column order (top to bottom: `L1..Lk` in the left column, `R1..Rm` in the right column, the bottom pane last) from a live `herdr pane layout` read, as the panes at the column's `rect.x` sorted by `rect.y`; the target's pane lists supply only the anchor for that read. Every split, move and tab creation passes `--no-focus`, and `pane swap` is never used.

| Target | Creation |
|---|---|
| Column with member panes | Read `herdr pane layout --pane <first column pane>`, then `herdr pane split <bottom pane of the column> --direction down --no-focus --cwd <cwd>` |
| Empty `right` column, one pane in the left column | `herdr pane split <that pane> --direction right --no-focus --cwd <cwd>` |
| Empty `right` column, two or more panes in the left column | `herdr pane split L1 --direction right --no-focus --cwd <cwd>`, then Right re-parenting |
| Empty `left` column | `herdr pane split R1 --direction right --no-focus --cwd <cwd>`, then Left re-parenting |
| New window | `herdr tab create --workspace <session> --cwd <cwd> --label <window_name> --no-focus`; `result.root_pane.pane_id` is the member pane and `result.tab.tab_id` its window |

A `tab create` reply without either field fails with `herdr tab create missing '<key>' field` and an unknown pane-cleanup outcome. After creation, in order:

1. `herdr pane run <new pane> "<shell-quoted command>"`. On failure the backend closes the new pane (§ Pane ownership during creation) and returns the run error; no re-parenting has run, so every other pane stays where it was.
2. `herdr pane rename <new pane> <member pane name>`.
3. Right or Left re-parenting, when the target requires it (§ herdr re-parenting).
4. Equalize every column of the tab (§ herdr column equalization), anchored at the new pane.

The member's agent starts at the pre-re-parenting size and receives a terminal resize when its pane moves or is equalized.

#### herdr re-parenting

herdr has no full-height column split: `pane split --direction right` splits only its target pane, and a `pane move` within one tab changes nothing. Re-parenting therefore rebuilds a column chain through one temporary tab `T'`: `pane move <pane> --new-tab --no-focus` creates it, and herdr closes it when the last move empties it. Each move keeps the pane ID and terminal within one workspace.

**Right re-parenting** (left column `L1..Lk`, k ≥ 2; the new pane `R` was split off `L1`, so the tab is `down(right(L1, R), L2..Lk)`) rebuilds the left chain under `L1`:

1. `herdr pane move L2 --new-tab --no-focus` creates `T'`; for `i = 3..k`, `herdr pane move Li --tab T' --split down --target-pane L(i-1) --no-focus`.
2. For `i = 2..k`, `herdr pane move Li --tab <tab> --split down --target-pane L(i-1) --no-focus`. The last move empties `T'`.

**Left re-parenting** (right column `R1..Rm`, m ≥ 1; the new pane `X` was split off `R1`, so the tab is `down(right(R1, X), R2..Rm)`) moves the right chain to the right of `X`:

1. `herdr pane move R1 --new-tab --no-focus` creates `T'`, leaving the tab `down(X, R2..Rm)`.
2. For `i = 2..m`, `herdr pane move Ri --tab T' --split down --target-pane R(i-1) --no-focus`, leaving `X` alone in the tab.
3. `herdr pane move R1 --tab <tab> --split right --target-pane X --no-focus` places `X` left and `R1` right.
4. For `i = 2..m`, `herdr pane move Ri --tab <tab> --split down --target-pane R(i-1) --no-focus`. The last move empties `T'` (for m = 1, step 3 empties it).

A failed move leaves the remaining panes where they are, emits the stderr diagnostic `layout re-parenting incomplete: <cause>` and lets the spawn continue. Re-parenting changes no registry placement. While a pane sits in `T'`, a concurrent `member press` to it fails its target validation against the recorded window (§ Explicit key dispatch); wrapped delivery revalidates only the registry placement.

#### herdr column equalization

herdr has no single reflow command, so it rebalances every column of a tab arithmetically. A layout read `herdr pane layout --pane <anchor>` returns `result.layout` with `panes[].rect{x,y,width,height}` and `splits[].{direction,rect,ratio}` for the anchor pane's tab, whichever tab or pane holds focus, so the read needs no tab-ID comparison. A column is every pane sharing one `rect.x`, sorted by `rect.y`. For a column of N ≥ 2 panes, the `down` splits whose `rect.x` and `rect.width` equal the column's form a right-leaning chain where split *k* (top→bottom) separates pane *k* from the panes below. A single-pane column, such as the Director's, has nothing to equalize.

Equal heights ⇔ split *k* has ratio `1/(N-k)` (top → `1/N`, …, bottom pair →
`1/2`). `herdr pane resize --amount` is a signed delta on a split's ratio, so
each split is driven to target by one resize: `delta = 1/(N-k) - ratio`
(rounded to 4 decimal places half away from zero, written as the shortest decimal that round-trips, without exponent or trailing zeros, such as `0.1667` or `0.5`; skipped if `|delta| < 1e-3`); `delta > 0` →
`herdr pane resize --pane <pane k> --direction down --amount <delta>`, else
`--pane <pane k+1> --direction up --amount <|delta|>`. A column whose matching
`down` split count is not `N - 1` is left untouched.

Delete-time restoration anchors at the first surviving pane in `herdr pane list` whose `tab_id` equals the caller-supplied window ID:

| Tab state after close | Action |
|---|---|
| No surviving pane | Nothing to do: herdr closed the tab with its last pane |
| One pane with exactly one residual `right` split | `herdr pane resize --pane <that pane> --direction right --amount <1 - ratio>`, rounded and written as for column equalization, unless the delta is below `1e-3` |
| Otherwise | Equalize every column |

A close error that the teardown does not tolerate propagates unchanged and no
restoration runs. Every rebalance is best effort: its failure never fails a
spawn or a delete.

### 6.6 Receiving and pane input

An ordinary member's still-pending previews are retried by that member's own runner (§6.9 Runner pending-preview sweep), with no caller command; submission budgets and pane input locking follow §6.5 Multiplexer. Director reception is the persistent listener plus the consuming `message poll` each notice prompts (§6.3 `message listen`, §6.3 `message poll`).

The persistent listener follows §6.3 `message listen` and performs no preview retry, pane lock, multiplexer resolution or termination-deferral span. Goal activation/pause/budget and pending decisions retain host authority.

### 6.7 Coding-agent backends

Every member pane runs a coding-agent binary selected from the backend registry and recorded per member in `member_placements.coding_agent`. This section specifies the backend interface, each backend's spawn argv, model and effort validation, permission posture, bundled presets and host evidence.

#### Role requirements

| Role | Tier | Required backend capability |
|---|---|---|
| Director | Eligible | A goal requirement (§ Goal requirement) and a listen requirement (§ Listen requirement), evaluated independently. Both are required. |
| Member | Eligible | A runner-compatible PTY lifecycle, an explicit detection-coverage declaration and a classified default launch mode, `auto` or `manual`, with approval-prompt coverage: the bundled profile for the evaluated native version detects every approval form the default mode renders and declares request-bound allow and deny answers for it, at every pane geometry outside its recorded layout limits. A default mode that renders no approval form meets this without request rules. |
| Member | Bundled support | A native version the version probe parses, selecting a bundled profile that declares detection, composer plans and request-bound answers independently from native evidence. Capabilities beyond approval-prompt coverage are optional, and an undeclared one is `unknown` coverage or unsupported. |

An *approval form* is a native permission request that blocks the member until answered. Question and startup dialogs are separate interactions outside approval-prompt coverage, with or without answer plans. A *recorded layout limit* is a profile-documented rendering that no request rule matches, such as a pane narrower than the profile's `W_min`, a wrapped or clipped operation, or an uncaptured geometry; a form rendered inside one produces no `interaction_opened` event and is answered manually on the native surface.

Evaluate each role independently; a backend ships for the roles it meets. A verdict is `yes` only when recorded evidence establishes every Eligible-tier requirement of that role against the profile selected for the recorded native version; otherwise it is `no` with "not established" or the observed failure as evidence, and a Director `no` names each unmet requirement. Default-mode approval-prompt coverage counts toward the Member verdict; other bundled-support capabilities never change a verdict. Each requirement group's evidence holds for its own recorded native version; a newer installed version carries the verdict forward until the next re-check, which follows any change to the backend's bundled profile boundary or recorded evidence. A Member-eligible backend whose version probe fails or selects no profile still launches: it records `coverage=unknown`, and any approval form it shows is answered manually on the native surface. A backend with unknown detector/composer coverage, including such a run, permits manual terminal work and delivers ordinary generated input through delivery fallback. § Role eligibility records each backend's verdicts.

Every non-default launch mode is an explicit `--permission-mode` choice under §6.9 Launch policy and capability registry, which records each mode's `auto`/`manual` class. Preserve native guards, managed policy, explicit denies, sandbox boundaries and the host's activation, pause, completion and budget authority. The decision surface and native receive support are evaluated independently of Director eligibility: a Director meeting both requirements may use manual `poll` when its reception mechanism is unavailable, and a tier-3 listen establishes no unsolicited wake of an idle session.

The bundled backend policy catalog owns launch-mode choices and defaults, exact argv mapping, version eligibility and detection/composer profiles. The omitted native argv is specified below; an explicit native mode changes only its permission argument and preserves the remaining argv and literal assignment.

##### Listen requirement

A *subscribe-style* listen delivers each record the running `cafleet message listen` emits into the session as model-visible input that the host pushes; the model neither polls nor reads the process. Its semantics:

- An idle session starts a new turn.
- A busy session receives the record no later than the end of the running turn, never interrupting it; earlier delivery between tool calls inside the turn also passes.
- No record waits for an unrelated future turn. A record queued behind earlier undelivered records is delivered after their turns; that wait passes, because each of those turns consumes an earlier record.
- Records keep their emission order.

| Tier | Name | Definition | Standing |
|---|---|---|---|
| 1 | Native subscribe | The push facility is part of the backend as installed; CAFleet contributes only CLI invocations: `cafleet message listen` and its forwarding options. | Preferred |
| 2 | Extension subscribe | Delivery through a CAFleet-vendored extension or plugin that `cafleet setup` installs (§6.3 Asset installation). | Accepted |
| 3 | Built-in compromise | Built-in tools, where the model reads the running process's output or receives periodic batches; it must pass the stream probe below. | Degraded fallback |

Tiers rank 1 > 2 >> 3: tier 3 spends model turns on reads, delays records to read or batch points, and needs heartbeat-style wake and log reconciliation.

A backend records the highest tier its evidence establishes, and tier 3 only together with negative evidence for tiers 1 and 2 on that version:

- **Tier 1 negative evidence:** the installed version's built-in tools and CLI expose no facility that pushes externally produced text into a running session as model-visible input that starts a turn when the session is idle, checked in built-in tool docs, `--help` of the binary and its subcommands, and the native source or official documentation for that version.
- **Tier 2 negative evidence:** no extension surface (plugins, hooks, extensions, MCP notifications, any client/server protocol an extension can call) lets an extension do the same, checked in the native source at that version's tag or the official extension documentation.
- **Recording:** each source is named with its version, plus `checked YYYY-MM-DD`.

Both probes are stated in message-ID terms. A probe message ID is *observed through a notice* when the one `poll` that a delivered `pending` record (§6.3 `message listen`) prompts returns it, the record having reached the session from the listener's output through the mechanism under test.

| Probe | Pass condition | Invalidation rule |
|---|---|---|
| Subscribe | Each probe message ID is first returned by a notice-prompted poll | A `poll` or `show` that first reveals a probe ID without a preceding notice turn invalidates the run; a notice-prompted `poll` is part of consumption |
| Stream | Each probe message ID is first returned by a poll that a notice in the running listener's output prompted, while the same listener process still runs | Any other `poll` or `show` invalidates the run |

**Subscribe probe** (tier 1 or 2, on an installed native version). The candidate Director starts the receiver through its tier's mechanism while another member sends broker messages. After it consumes the probe member's ready, the Director runs under this instruction:

> Consume broker messages only through your receiver. On each `[cafleet listen record]` turn, run the command in the record's `next` field once. Run no other `poll` or `show` unless a step below directs it. When I say `hold`, end only the next notice turn without running its command, then apply the rule above to every later notice turn, including the ceiling re-notice.

| Step | Procedure | Pass condition |
|---|---|---|
| Idle | Send one message to the idle Director | A notice starts a turn without input; the poll it prompts returns the message |
| Busy | During a foreground command of at least 60 s, send two messages | The command is not interrupted; a notice arrives no later than the end of that turn, and the poll it prompts returns both IDs |
| Burst | During a foreground command of at least 60 s and under 300 s, send 150 messages back to back | At most two notice turns arrive for the burst, and every ID is returned by a notice-prompted poll. On Codex the forwarder stays alive and no `codex queue` refusal occurs. |
| Consumed notice | Direct one turn: a 30 s foreground command while m1 is sent, one poll, a second 30 s command while m2 is sent, one poll, then end the turn | Every notice turn delivered after the directed turn runs one poll, which returns `[]` |
| Hold ceiling | Say `hold`; send `hold-probe 1` to the idle Director and record t1, the time its notice turn starts (the Director ends it without polling). After that turn ends and 60 s have passed since t1, send `hold-probe 2`. | No notice arrives before t1 + 285 s. One notice arrives between t1 + 285 s and t1 + 315 s, and the poll it prompts returns both IDs |
| Owner exit | End the Director session | No `cafleet message listen` survives (`ps`) |

t1 observes the held notice's delivery from the session side. The listener sets `held_at` when delivery succeeds, which precedes t1 by the receiver latency, and checks the ceiling once per one-second idle interval; the ±15 s tolerance around t1 + 300 s covers both.

**Stream probe** (tier 3, on an installed native version). The candidate Director starts `cafleet message listen` through its native background-execution facility and continues unrelated work in the same session under the task prompt "Learn about broker messages only from that listener's output; run no other `poll` or `show`. When a `pending` record appears there, run the command in its `next` field once and continue." The facility must deliver each record the running listener emits without waiting for the process to exit, at the latest at the session's next turn boundary, whether the host pushes records into the session or the model reads the running process's accumulated output on demand. Another member sends probe messages at separate times during that work. Output that reaches the session only when the process exits, including a completion notification for a finished background process, does not count as observation. Mid-turn delivery and waking an idle session are not required.

| Stream-probe run | Classification |
|---|---|
| A probe ID first revealed outside a notice-prompted poll, or any `poll` or `show` other than one notice-prompted poll | Invalid |
| Every probe ID first returned by a notice-prompted poll before the listener exited | Pass |
| A probe ID first returned only after the listener exited, or never returned by a notice-prompted poll | Fail |

Each run records the mechanism, native version and check date.

##### Goal requirement

| Tier | Name | Definition |
|---|---|---|
| 1 | Native | Built-in goal lifecycle controls, such as a goal command |
| 2 | Extension | A CAFleet-vendored extension that meets the minimum bar below |
| — | No | Neither |

Native outranks extension. **Extension-goal minimum bar**, each item verified live on the installed native version:

- **Operations:** set, status, pause, resume, clear and complete.
- **Persistence:** the goal survives context compaction and a process restart on the same session.
- **Continuation:** turns continue automatically until the goal completes, is paused or is cleared.

#### Role eligibility

| Backend | Role | Requirement | Source | Delivery | Eligible | Evidence |
|---|---|---|---|---|---|---|
| `claude` | Director | `goal` | `native` | — | `yes` | [Claude Code goal](https://code.claude.com/docs/en/goal), 2.1.296, checked 2026-10-22. |
| `claude` | Director | `listen` | `native` | `subscribe` | `yes` | One Monitor task running `cafleet message listen`: subscribe probe in message-ID terms (idle wake, a notice after a 90 s foreground command, a 150-message burst consumed through one notice, consumed notices delivered between tool calls of the directed turn, the 300 s hold ceiling and owner exit ending the listener), 2.1.319, checked 2026-11-17. The run isolated the binary and database but used the shared Claude config root, from which Monitor reception reads no CAFleet asset. |
| `claude` | Director | `verdict` | — | — | `yes` | `goal and listen rows` |
| `claude` | Member | — | — | — | `yes` | PTY lifecycle, coverage declaration and omitted `auto` automatic review: live runner acceptance on tmux and herdr (omitted-default launch, auto-approved and auto-refused cases), 2.1.302, checked 2026-10-29. Bundled support: the Claude profile from `2.1.302`. |
| `codex` | Director | `goal` | `native` | — | `yes` | [Codex goals](https://developers.openai.com/cookbook/examples/codex/using_goals_in_codex), 0.162.1, checked 2026-10-22. Native `/goal` stayed active through the 200-record burst, idle and forced-failure steps, 0.165.0, checked 2026-11-06. |
| `codex` | Director | `listen` | `native` | `subscribe` | `yes` | `cafleet message listen --codex-thread` forwarding through `codex queue` on the escalated path: subscribe probe in message-ID terms (idle wake, a notice after a 90 s foreground command, a 150-message burst consumed through one notice with the forwarder alive and no queue refusal, queued consumed notices drained one per turn, the 300 s hold ceiling and owner exit ending the forwarder), 0.167.0, checked 2026-11-17. The sandboxed path is unverified, and the run answered escalations through the `user` reviewer because automatic review refused them in an untrusted working directory. |
| `codex` | Director | `verdict` | — | — | `yes` | `goal and listen rows` |
| `codex` | Member | — | — | — | `yes` | PTY lifecycle, coverage declaration and omitted `--approve-for-me` automatic review: live runner acceptance on tmux and herdr (omitted-default launch and auto-refused cases, host-limited cases recorded as limits), 0.163.0, checked 2026-10-29. Bundled support: the Codex profile from `0.163.0`. |
| `opencode` | Director | `goal` | `extension` | — | `yes` | `cafleet` plugin goal extension under `opencode mini --standalone`: set, status, pause, resume, clear, complete, compaction, restart on the same session and the no-progress stop, 2.2.0 (minimum), checked 2026-11-06. A continuation in flight at pause was not separately observed. |
| `opencode` | Director | `listen` | `extension` | `subscribe` | `yes` | `cafleet` plugin `cafleet_listen` with queued `session.synthetic` delivery under `opencode mini --standalone`: subscribe probe in message-ID terms (idle wake, a notice after a 90 s foreground command, a 150-message burst consumed through one notice, consumed notices delivered as separate turns, the 300 s hold ceiling and client exit ending the child), 2.2.6, checked 2026-11-17. The supported minimum is 2.2.0. |
| `opencode` | Director | `verdict` | — | — | `yes` | `goal and listen rows` |
| `opencode` | Member | — | — | — | `yes` | PTY lifecycle and coverage declaration: live runner acceptance on herdr only (`coverage=tested`), 2.1.4, checked 2026-10-29. Default-mode approval-prompt coverage: the omitted `cafleet-ask` preset's shell permission form, whose detection and allow and deny answers the OpenCode profile from `2.1.4` declares `tested` (§6.9 Bundled profiles and native screen fixtures). Bundled support: the OpenCode profile from `2.1.4`. |
| `hermes` | Director | `goal` | `native` | — | `yes` | Native `/goal` set, status, pause, resume and clear on the TUI client, 2026.9.24, checked 2026-11-06. |
| `hermes` | Director | `listen` | `extension` | `subscribe` | `yes` | `cafleet` plugin `cafleet_listen` with ordered gateway injection on the TUI client: subscribe probe in message-ID terms (idle wake, a notice after a 90 s foreground command, a 150-message burst consumed through one notice, consumed notices delivered as separate turns, the 300 s hold ceiling and `/exit` ending the child as the session finalized), 2026.9.24, checked 2026-11-17. |
| `hermes` | Director | `verdict` | — | — | `yes` | `goal and listen rows` |
| `hermes` | Member | — | — | — | `yes` | PTY lifecycle, coverage declaration and default `smart` approval-prompt coverage (ESCALATE and override, allow-once and deny): live runner acceptance on tmux and herdr, 2026.9.24, checked 2026-11-02. Bundled support: the Hermes profile from `2026.9.24`. |
| `pi` | Director | `goal` | `extension` | — | `yes` | CAFleet pi extension `/cafleet-goal` and goal tools under plain `pi`: set, status, pause, resume, clear, complete, automatic continuation, the no-progress stop, an interrupt pausing, manual `/compact` and `/quit` then `pi --continue` on the same session, 1.5.0 (minimum), checked 2026-11-08. A threshold compaction inside a continuation chain and a continuation by the re-arm turn after restart were not separately observed. |
| `pi` | Director | `listen` | `extension` | `subscribe` | `yes` | CAFleet pi extension `cafleet_listen` with `followUp` delivery under plain `pi`: subscribe probe in message-ID terms (idle wake, a notice after a 90 s foreground command, a 150-message burst consumed through one notice, consumed notices delivered as separate turns, the 300 s hold ceiling and `/quit` ending the child), 1.5.0 (minimum), checked 2026-11-17. The kimi-k2.6 Director model applied `hold` to the ceiling re-notice as well and polled both IDs after an explicit release; a pane kill ending the child and `/reload` delivering the runtime-replaced notice were checked 2026-11-08. |
| `pi` | Director | `verdict` | — | — | `yes` | `goal and listen rows` |
| `pi` | Member | — | — | — | `yes` | PTY lifecycle, coverage declaration and the omitted `cafleet-ask` gate mode's approval form: live runner acceptance on tmux and herdr (protected directories, routed command and `member exec`, busy delivery, and `cafleet-ask` allow and deny answers), 1.5.0, checked 2026-11-08. Bundled support: the pi profile from `1.5.0`. |

| Column | Values |
|---|---|
| Requirement | `goal`, `listen` or `verdict` for Director rows; — for the Member row |
| Source | `native` or `extension` for `goal` and `listen` rows; — otherwise |
| Delivery | `subscribe` or `compromise` for `listen` rows; — otherwise |
| Eligible | `yes` or `no` per § Role requirements |
| Evidence | Requirement rows: mechanism, native version and `checked YYYY-MM-DD`, in at most two sentences. A `verdict` row: `goal and listen rows` for `yes`, or the unmet requirement names for `no`. |

#### Interface

Each backend has a registry token (its `--coding-agent` value, stored `placement.coding_agent` value and executable name) and zero or more shared-server signatures (§ Shared-server signatures). `member create` checks, in order, model, effort, prompt body, availability (config directory, executable on `PATH`, the backend's own prerequisite), then builds argv. A model, effort or prompt failure is a usage error (exit 2) with the backend's message and precedes every side effect; an availability failure exits 1. An omitted `--model` or `--effort` is always valid.

The availability check resolves the config directory per §6.3 Config-dir resolution (an invalid config-location variable fails first), resolves the executable per §6.9 Native executable resolution, failing with `binary <name> not found on PATH`, then runs the backend's own check: opencode reads the selected preset file, hermes passes § Hermes availability preflight, and pi reads the CAFleet pi extension `<pi base>/extensions/cafleet.ts`. A backend without a reasoning-effort control rejects every given `--effort`. The prompt check reads the raw body, before placeholder substitution, Director placement lookup and registration; only pi rejects a body (§ Per-backend spawn argv). Argv construction takes the prompt, the member pane name `<display_name>` (§6.5 Window and pane naming), the member's resolved absolute working directory `<cwd>` (§6.5 Pane spawn working directory; only hermes emits it), the model and the effort. Its leading token is the executable name; the launched `argv[0]` follows §6.9 Native executable resolution. Broker value-errors (§7.2) exit 1.

An omitted `--model` emits no `--model` tokens (never an empty `--model ""`) and an omitted `--effort` emits no effort tokens, so each argv is byte-identical to its omitted form. An invalid effort never reaches argv construction.

#### Registry resolution

The backend registry is the fixed set of supported backends in registry order, each keyed by its `name`. Resolution exactly matches the persisted `placement.coding_agent` token, with no fuzzy matching or default; a token with no entry fails with `invalid stored value for member_placements.coding_agent: VALUE`, exit 1.

#### Shared-server signatures

A shared-server signature identifies a long-lived backend server whose tool commands carry the environment of the pane that started it, not of the session they serve. It has `programs`, the final path components that name the server's program token, and `subcommand`, the tokens that follow that program token:

| Backend | `programs` | `subcommand` | Label |
|---|---|---|---|
| `codex` | `codex` | `app-server` | `codex app-server` |
| `hermes` | `hermes`, `hermes_cli.main` | `gateway`, `run` | `hermes gateway run` |
| `claude`, `opencode`, `pi` | — | — | — |

An argv matches when some token `argv[i]`, whose final `/`-separated component equals one of `programs`, is followed somewhere after `i` by the `subcommand` tokens as a contiguous run; options may sit between the program token and the run. The label is the backend name followed by the `subcommand` tokens joined by spaces. §6.3 Caller pane verification row 1 tests every ancestor against the signatures of every backend in registry order, whatever `--coding-agent` declares. OpenCode has no signature; the ancestry check refuses a second client of `opencode serve --service`.

#### Per-backend spawn argv (exact, token-by-token)

The blocks below are the child argv with omitted permission selection. The leading token names the backend binary (launched `argv[0]`: §6.9 *Native executable resolution*). The multiplexer starts the CAFleet PTY runner, which preserves the literal child argv, cwd and required environment. Every backend honors the leading-`!` shell shortcut that `member exec` dispatches through, and every member can run its broker commands from the moment it is spawned (§ Backend capabilities). Every backend's model and effort validation follows § Model selection and § Reasoning effort.

**claude** — Availability resolves the claude config directory and checks `claude` on `PATH`. The member pane name becomes `--name` (§ Backend capabilities).

```
["claude", "--permission-mode", "auto", "--prompt-suggestions", "false", "--allowedTools", <rule>...,
 "--name", <display_name>]                           # explicit selection replaces "auto" with <mode>
  (+ ["--model", <model>]  when --model is given)
  (+ ["--effort", <effort>]  when --effort is given)
  (+ <prompt>)                                       # bare trailing positional
```

The `--allowedTools` values are the claude member's **spawn-time allow rules**, one argv token per rule, so the member depends on no `Bash(cafleet ...)` rule in the user's `settings.json`. Every claude member receives exactly `Bash(cafleet message *)`, covering its broker commands including `message listen`. `--allowedTools` takes several values, so it precedes `--name`, which ends the list and keeps the prompt positional. The rules are fixed at spawn: a later loss of allow rules in the user's settings does not affect a running member. These cases stay with the host:

- A `deny` or `ask` rule at any settings level, including managed policy, still blocks the command.
- The managed setting `allowManagedPermissionRulesOnly` makes Claude Code ignore `--allowedTools`.
- A blocking `PreToolUse` hook stops a tool call before permission rules are evaluated.
- Settings can change after spawn, so overdue member readiness or replies are inspected directly.
- Work commands (test runners, `git commit`, package managers) stay under the user's own read/edit/command rules; a denied one is routed through `member exec`.

**codex** — Availability resolves the codex home and checks `codex` on `PATH`. The member pane name is ignored.

```
["codex", "--no-daemon", "--approve-for-me"]
  (+ ["--model", <model>]  when --model is given)
  (+ ["--config=model_reasoning_effort=<effort>"]  when --effort is given)  # ONE token
  (+ <prompt>)                                       # bare trailing positional
```

An explicit `on-request` or `never` replaces `"--approve-for-me"` with `"--ask-for-approval", <mode>, "--sandbox", "workspace-write"`; `"--approve-for-me"` selects the workspace-write sandbox itself, and Codex 0.163.0 rejects it together with `--sandbox`.

Every launch policy keeps `"--no-daemon"`, so the member runs its own Codex session instead of starting or reusing the shared background app-server. The member's tool commands are then children of the pane's own `codex` process and inherit that pane's multiplexer variables; a shared app-server keeps the environment of the pane that started it. Codex 0.163.0 accepts `--no-daemon`.

**opencode** — Model validation examples: `"openai/gpt-4"` accepted; `"a/b/c"` accepted as provider `a` / model `b/c`; `"a/"`, `"/b"`, `"abc"` rejected. Effort validation rejects every given `--effort`, so the opencode argv never carries effort tokens. `member create` resolves the launch policy before the availability check, whose opencode step (§ Interface) reads only the selected preset `<preset base>/agents/<mode>.md` for the resolved mode (§ opencode preset); only opencode has a required preset. The member pane name is ignored; the prompt is a two-token `--prompt <prompt>` pair, unlike claude/codex's bare positional.

```
["opencode", "mini", "--standalone", "--agent", <mode>]   # <mode> is cafleet or cafleet-ask
  (+ ["--model", <model>]  when --model is given)
  (+ ["--prompt", <prompt>])                         # prompt via flag — TWO tokens
```

The pane runs `opencode mini --standalone`, the long-lived minimal interactive interface with a private server per pane; `mini` takes no positional message. The `--agent` value is the only argv difference between the two launch policies, and `member create` separately probes the native version for profile selection. Spawn, preset loading, broker input and native shell input are validated against opencode 2.1.4; the OpenCode profile from `2.1.4` is the first boundary, so `member create` refuses an earlier probed version (§6.9 Profile selection by minimum version).

**hermes** — Availability resolves the Hermes home per §6.3 *Config-dir resolution* (`HERMES_HOME`, default `~/.hermes`), checks `hermes` on `PATH`, then runs § Hermes availability preflight. The member pane name is ignored; Hermes has no `--name` analog.

```
["hermes", "chat", "--cli", "--source", "tool", "--in", <cwd>]
  (+ ["--model", <model>]       when --model is given)
  (+ ["--reasoning", <effort>]  when --effort is given)
  (+ ["-q", <prompt>])                               # prompt via flag — TWO tokens
```

The argv carries no permission tokens (the operator configuration's `approvals.mode` governs the native session) and never `--provider` (the operator's configuration selects the provider). Every spawn starts a fresh conversation: the argv never contains `--resume`, `--continue` or `--create-if-missing`. `--source tool` keeps member sessions out of the operator's default session lists, and `-q` on a terminal seeds the interactive session with the prompt as its literal first turn.

**pi** — Availability resolves the pi base per §6.3 *Config-dir resolution* (`PI_CODING_AGENT_DIR`, default `~/.pi/agent`), checks `pi` on `PATH`, then reads the CAFleet pi extension `<pi base>/extensions/cafleet.ts` in every launch mode, because every pi member depends on its gate (§ pi member gate). The member pane name becomes `--name`; the working directory is unused.

```
["pi", "--cafleet-gate", "cafleet", "--no-approve", "--tui-mode", "fullscreen", "--name", <display_name>]
                                                     # explicit selection replaces "cafleet" with <mode>
  (+ ["--model", <model>]     when --model is given)
  (+ ["--thinking", <effort>] when --effort is given)
  (+ ["--", <prompt>])                               # bare trailing positional after --
```

The permission argv is the `--cafleet-gate <mode>` pair, where `<mode>` is `cafleet` or `cafleet-ask`. `--no-approve` ignores trust-gated project-local configuration and resources, so no project-trust dialog blocks a member; CAFleet skills load from `<pi base>/skills`, and `AGENTS.md` and `CLAUDE.md` still load. `--tui-mode fullscreen` pins the layout the pi profile matches, whatever the operator's `tuiMode` setting. `--` keeps a prompt that begins with `-` positional. Every spawn starts a fresh session: the argv never contains `--continue`, `--resume`, `--session`, `--session-id` or `--fork`. Spawn is validated against pi 1.5.0; the pi profile from `1.5.0` is the first boundary, so `member create` refuses an earlier probed version (§6.9 Profile selection by minimum version).

**Prompt rule.** pi reads a positional argument beginning with `@` as a file attachment, so pi's prompt check rejects a prompt body whose first character is `@` with usage exit 2 and `Error: a pi spawn prompt begins with text, not '@' (pi reads a leading '@' argument as a file attachment).`, before registration: no member row, run or pane is created. Checking the raw body is equivalent to checking the rendered prompt, because a leading placeholder renders to digits, `default`, a backend token or `sqlite:///…`. No pi argv carries a prompt that begins with `@`.

#### Model selection

`member create --model <m>` inserts `--model <m>` immediately before the prompt tokens; omitted, no model tokens are emitted and the binary uses its configured default.

| Backend | Accepted value format | Create-time validation |
|---|---|---|
| `claude` | Passes through verbatim | None; the binary rejects unknown models, so newly released models need no CAFleet release |
| `codex` | Passes through verbatim | None; the binary rejects unknown models |
| `opencode` | `<provider-id>/<model-id>`, split on the **first** `/` into two non-empty segments; model ids may contain further slashes | Exit 2 with `Error: --model for the opencode backend must be '<provider-id>/<model-id>' (got '<value>').` |
| `hermes` | Passes through verbatim, including provider-prefixed ids; omitted, the operator's configured default model and provider apply | None; the binary rejects unknown models |
| `pi` | Passes through verbatim: `provider/id`, an exact id or a fuzzy name; omitted, pi's `defaultProvider` and `defaultModel` settings apply | The value names a model without a thinking suffix: when the text after its last `:` equals one of `off`, `minimal`, `low`, `medium`, `high`, `xhigh`, `max`, exit 2 with `Error: --model for the pi backend names a model without a ':<thinking>' suffix; pass the thinking level with --effort (got '<value>').` Any other text after a `:`, such as a `:free` variant, passes through; the binary rejects unknown models |

#### Reasoning effort

`member create --effort <level>` is validated against the backend's accepted set before registration or multiplexer effects and emitted right after the model tokens, before the prompt; omitted, no effort tokens are emitted.

| Backend | Accepted levels | Forwarded as | Rejected with (exit 2) |
|---|---|---|---|
| `claude` | `low`, `medium`, `high`, `xhigh`, `max` | `--effort <level>` | `Error: --effort for the claude backend must be one of low, medium, high, xhigh, max (got '<value>').` |
| `codex` | `minimal`, `low`, `medium`, `high`, `xhigh` | The single token `--config=model_reasoning_effort=<level>` | `Error: --effort for the codex backend must be one of minimal, low, medium, high, xhigh (got '<value>').` |
| `opencode` | None; the backend exposes no reasoning-effort control | — | `Error: opencode does not support reasoning effort.` |
| `hermes` | `none`, `minimal`, `low`, `medium`, `high`, `xhigh`, `max`, `ultra` | `--reasoning <level>` | `Error: --effort for the hermes backend must be one of none, minimal, low, medium, high, xhigh, max, ultra (got '<value>').` |
| `pi` | `off`, `minimal`, `low`, `medium`, `high`, `xhigh`, `max` | `--thinking <level>` | `Error: --effort for the pi backend must be one of off, minimal, low, medium, high, xhigh, max (got '<value>').` |

pi clamps a thinking level to the model's capability, and a non-reasoning model runs `off`; CAFleet performs no per-model check.

#### Backend capabilities

| Backend | OS-level sandbox | Sets the pane title | Shell-command posture | Preset or config prerequisite |
|---|---|---|---|---|
| `claude` | None | Yes, via `--name <member pane name>` | Runs its spawn-time allow rules and every command matching `permissions.allow`; auto mode's classifier reviews the rest, and a refused command routes to the Director | A Claude Code permission profile and native directory consent when requested; a permission posture, not a spawn dependency |
| `codex` | Kernel-enforced workspace-write sandbox; matching allow rules and approved escalations authorize commands outside it | No; `member list` `window` and `pane_id` locate the pane | Runs commands its workspace/host rules permit; automatic review decides escalations, and a refused required command routes to the Director | `<codex home>/rules/cafleet.rules`, applicable sandbox access below and native directory consent when requested; a permission posture, not a spawn dependency |
| `opencode` | None | No; `member list` `window` and `pane_id` locate the pane | `cafleet-ask` (default): the member broker commands run and every other shell command opens a native permission form; `cafleet`: the same broker commands, every other command denied | The selected policy's preset `<preset base>/agents/<mode>.md`; a spawn precondition |
| `hermes` | None | No; `member list` `window` and `pane_id` locate the pane | `smart`: commands outside Hermes's dangerous-command patterns run unreviewed, and the guardian model reviews the rest and may open an approval prompt. Hardline floors, the sudo guard and `approvals.deny` block before any review | The operator configuration's `approvals.mode` |
| `pi` | None | Yes, via `--name <member pane name>`; pi titles the terminal `π - <member pane name> - <cwd basename>` | `cafleet-ask` (default): the member broker commands run and every other bash command opens a native permission form, the gate's approval form for a command without a control character (§ pi member gate); `cafleet`: the same broker commands, every other command denied | The CAFleet pi extension `<pi base>/extensions/cafleet.ts`; a spawn precondition |

"Sets the pane title" is the backend's own title mechanism. Independently of it, CAFleet names every member pane with the member pane name on the multiplexer, as a tmux pane title or a herdr pane label (§6.5 Window and pane naming); a backend that sets its own title can retitle the tmux pane, while the herdr label stays independent of the agent's terminal title.

The omitted claude and codex policies are native automatic approval modes (§6.9 Launch policy and capability registry). The omitted hermes policy `smart` is also an `auto` mode: an ESCALATE or a guardian DENY opens a native approval prompt that the Director answers allow once or deny through the permission commands (§6.9 Bundled profiles and native screen fixtures), and an unanswered prompt fails closed after the configuration's `approvals.timeout`. Every session that reads a Hermes configuration applies its `approvals.deny`, so an operator glob there that matches a Director-only command also blocks it in a Hermes Director's own session, reported as `BLOCKED: this command matches the user-defined deny rule '<glob>' …`. CAFleet needs no `approvals.deny` entry, because the runner guard covers members (§6.3 Director-only command guard). The omitted OpenCode and pi policy `cafleet-ask` is a `manual` mode whose form the Director answers; explicit `cafleet` is an `auto` mode with a fixed allow/deny policy that renders no approval form (§ opencode preset, § pi member gate).

#### Claude permission posture

Native directory consent, when requested, precedes broker readiness and is independent of command permissions and auto-mode availability.

Under the omitted `--permission-mode auto`, a member runs its spawn-time allow rules and every tool call matching `permissions.allow` without review; Claude Code's auto mode classifier reviews each other call, and a refused call returns to the member, which routes it to the Director like any denial. Native rules, hooks and managed policy remain authoritative (a `deny` or `ask` rule still applies), and auto mode must be available for the account and model; where it is not, the member can show native permission forms. The runner holds the auto-mode dialogs `Make auto mode your default permission mode?` (shown when the user settings pin a non-`auto` default) and `Teach auto mode about your environment?` (shown in auto mode) as `question` interactions, which the Director dismisses through `permission respond --choice` (§6.9 *Bundled profiles and native screen fixtures* states the choices and when each dialog stops reappearing).

Explicit native modes keep their own permission behavior. Under explicit `dontAsk`, a member runs without a prompt its spawn-time allow rules, Claude Code's built-in read-only commands, file reads inside its working directory and every tool call matching `permissions.allow`; every other call, a shell command or a file edit including one matching `permissions.ask`, is denied without a prompt. A write to one of Claude Code's protected paths, such as a project's `.git` or `.claude` directory, is denied whatever the allow list holds.

Members need no settings entry for their broker commands, which they receive as the spawn-time allow rule `Bash(cafleet message *)`. Under `auto`, the classifier reviews each Director command that no rule matches, and auto mode approves the Director's file edits inside the working directory. An ask rule outranks every allow rule, so `ask` entries for the `Bash(cafleet member prompt *)`, `Bash(cafleet member exec *)` and `Bash(cafleet member press *)` prefixes beside `Bash(cafleet *)` still make the operator confirm each explicit native pane-input dispatch of the Director. Inside a member, the Director-only commands refuse through the runner guard (§6.3 Director-only command guard). An `Edit` or `Write` allow entry applies to every session that reads the settings file. When `CLAUDE_CONFIG_DIR` relocates the config directory, a `Read` entry for the installed skills names that directory's `skills/**`.

#### Codex sandbox and configuration

Every Codex member launch policy selects the kernel-enforced workspace-write sandbox. Commands execute within its boundary unless a matching allow rule or approved escalation authorizes execution outside it (§ codex rules file). §6.9 Launch policy and capability registry maps each choice to its argv, approval behavior and inherited Codex configuration.

The inherited `config.toml` controls sandbox access and directory trust:

| Setting | Effect | Applicable condition |
|---|---|---|
| `sandbox_workspace_write.network_access` | `true` permits network access for sandboxed commands | A command remains sandboxed and its multiplexer socket access is blocked by that boundary |
| `sandbox_workspace_write.writable_roots` | Adds writable locations for sandboxed commands | A command remains sandboxed and the selected database directory is outside every effective writable location, including automatic temporary-directory grants |
| `trust_level` | Records project trust under `[projects."<absolute path>"]` | Native directory consent blocks readiness until the requested scope is trusted |

Directory consent follows the native repository or directory scope, including worktrees.

The approval policy determines whether an operation can request escalation; `approvals_reviewer` selects who decides an eligible request. Under `on-request`, the `user` reviewer receives a native approval prompt and `auto_review` receives automatic review. The native `--approve-for-me` launch supplies automatic review. Under `never`, a matching authorized command can run, while an operation requiring a new approval fails. Managed restrictions and stricter rules remain authoritative. [OpenAI's approval documentation](https://learn.chatgpt.com/docs/sandboxing/auto-review) defines the policy/reviewer distinction.

#### codex rules file

The codex auto-approval rules for `cafleet` commands are a static file embedded in the binary (source `assets/presets/codex/cafleet.rules`) and installed by `setup`'s asset installation (§6.3) to `<codex home>/rules/cafleet.rules`, with `<codex home>` resolved per §6.3 *Config-dir resolution* (`CODEX_HOME`, default `~/.codex`), overwriting any existing target. Codex loads every `*.rules` file under its rules directory (decision precedence below), so operator customizations live in a separate rules file there. The checked-in `assets/presets/codex/cafleet.rules` is the content contract: exactly the one line `prefix_rule(pattern = ["cafleet"], decision = "allow")` followed by one trailing newline.

The broad allow matches every `cafleet` command for every fleet. Matching commands run outside the sandbox, subject to stricter rules and managed restrictions; shell wrappers and variable expansion can affect matching. Codex loads installed rules at startup.

Codex members have no spawn-time allow rules, so their broker commands use installed rules; the runner guard refuses Director-only commands inside a member (§6.3 Director-only command guard). Codex applies the strictest decision when several rules match (`forbidden` > `prompt` > `allow`), so operator `prompt` rules in a separate file request confirmation of matching Director commands.

A rules directory maintained without the installed file needs an `allow` rule for the `["cafleet", "message"]` prefix to cover the broker commands members run themselves (those a claude member receives as spawn-time allow rules). The installed file also covers the Director's other `cafleet` subcommands. Operations without a matching allow follow the session's approval policy, reviewer and sandbox configuration (§ Codex sandbox and configuration).

#### opencode preset

Each opencode launch policy names an agent definition of the same name, a
static file installed by §6.3 Asset installation:

| Launch policy | Spawn argument |
|---|---|
| `cafleet` | `--agent cafleet` |
| `cafleet-ask` (default) | `--agent cafleet-ask` |

The preset base is opencode's `--agent <mode>` discovery path: opencode 2.1.4 loads agent definitions from `~/.config/opencode` and from `OPENCODE_CONFIG_DIR`, so a relocated preset remains discoverable, and definitions with the same name can merge across those sources. `member create` forwards a set `OPENCODE_CONFIG_DIR` into the member environment (§6.3 *Member environment*), so the member's opencode loads the preset the precondition read.

The selected policy's preset is a spawn precondition (§ Interface) whose error (§ Contract error strings) names the selector as the targeted repair; plain `cafleet setup` also installs it.

Each preset's `shell` ruleset is its `"*"` base followed by three `"allow"` entries for the member broker commands: `cafleet message send *`, `cafleet message poll *` and `cafleet message show *`, where `*` matches any character sequence, spaces included, so a member passes `--database-url <url>` after the subcommand. The allow entries overlap only the base, so their order carries no precedence. Every other `cafleet` command, Director-only commands and `message listen` included, falls to the base, and the runner guard refuses the Director-only commands inside the member (§6.3 Director-only command guard). Tool permission and assignment authority remain separate; the assigned role and Director instructions determine which allowed operations a member may perform.

Both presets set `"cafleet": "deny"` in their permission map as defense in depth for the CAFleet plugin tools (§ Director extensions): the deny refuses a member server's tools before the plugin's Director verification runs.

The `cafleet-ask` preset differs from the `cafleet` preset only as § Preset file
format states; the shell base sets their behavior:

| Preset | Shell base | An unmatched shell command |
|---|---|---|
| `cafleet` | `"*": "deny"` | Is denied without a permission form |
| `cafleet-ask` | `"*": "ask"` | Opens a native permission form, which the member runtime reports to the Director |

Non-shell permissions never ask under either preset, so only a shell command
opens a form and the `question` tool raises no competing question form.

Under `cafleet-ask`, a shell permission form is a runtime decision: the member runtime detects it and notifies the Director with an `interaction_opened` event, the Director answers it through `cafleet permission respond`, and the member continues from the result OpenCode returns (§6.9 *Bundled profiles and native screen fixtures* states the OpenCode 2.1.4 profile's answer plans and continuation behavior). A form in a pane too narrow to show its choices on one row, a command clipped by the form's command box and a reason the user wants to give are answered manually on the native surface. Under `cafleet`, a permission popup in an opencode pane is a regression escape: the Director captures the pane and escalates, and only an operator decision extends the allowlist.

Both postures are allowlists; standalone un-enumerated commands fall to the `"*"` base. These limits apply to tool use and command authorization:

| Surface | Example | Limit |
|---|---|---|
| MCP-contributed tools | — | Their participation in preset permission evaluation is unverified on opencode 2.1.4; evaluation is established only for loaded configurations free of MCP servers. |
| Compound shell commands | <code>cafleet message poll 7; curl … &#124; sh</code> against `cafleet message poll *` | Permission matching for compound lines is unverified on opencode 2.1.4, under both presets; matching is established for one command per invocation. |
| Interpreter or hook execution via a command the Director allows | a test-runner command executes workspace-writable test code; `git commit` runs `.git/hooks` | An allowed command also runs the code and hooks it executes, under that one-time permission. |
| `.env` files | `cafleet message send --from-member-id 7 --to-member-id 1 --file .env` | The `read` and `edit` `.env` denials govern OpenCode's file tools; an allowed broker `message send --file` naming a `.env` file copies its contents into a broker message. |

cafleet ships no MCP stanzas.

#### Preset file format

The checked-in `assets/presets/opencode/cafleet.md` and
`assets/presets/opencode/cafleet-ask.md` are the content contract. Each is a
`---`-delimited **JSON** (not YAML) frontmatter block (2-space indent,
non-ASCII preserved, top-level key order `description`, `mode`, `permission`),
a blank line, then the markdown body. `shell`/`read`/`edit` are glob→decision
maps; the other seven `permission` fields are scalar `"deny"`. The
`cafleet-ask` frontmatter equals the `cafleet` preset's except `description`
and the shell `"*"` value, and its body differs only in the first paragraph.

Each preset body has three blank-line-separated paragraphs after the heading, with no internal hard line breaks, and each file ends with exactly one trailing newline.

#### Hermes availability preflight

After the config-dir and PATH checks, `member create` for hermes runs one concurrent batch: `hermes config path`, which resolves the effective Hermes home for settlement (§ Hermes work settlement), and the native version probe (§6.9 Native version probe). Both run the resolved executable with null stdin, the member's `HERMES_HOME` (the forwarded value, or unset) and output captured and bounded like the native version probe, under one deadline of the smaller of two seconds and the ten-second creation deadline. The `config path` result is evaluated first, then the version probe. Each `config path` failure exits 1 before registration with its § Contract error strings error.

#### Hermes work settlement

A Hermes member's session can keep accepted work and background processes running after its pane's client exits. Deletion and creation compensation for a `hermes` run stop that work and verify the stop:

| Requirement | Mechanism |
|---|---|
| Session identity | `member create` stores `{"hermes_home": <effective Hermes home>}` in `member_runs.native_session_json`, the directory of the file `hermes config path` names under the member's `HERMES_HOME`; settlement adds `session_id`, the full `local-<hex>` ID from the exit summary `Session: <id>` that `/exit` prints |
| In-flight session work | While the client runs, settlement sends native input to the member's pane under the deletion's pane lock: `Ctrl+C` only when the pane shows a running turn (`type to redirect this turn`) or an open form (`↑/↓ to select`), because `Ctrl+C` on an idle composer ends the client without stopping its session. It then enters `/stop` and then `/exit` (entry steps below the table) |
| Registered background processes | `/stop` stops the session's registered processes; settlement waits for its report (`Stopped N process(es).` or `No running background processes.`) |
| Verification | The client printed its exit summary and the run's native child exited; no `<home>/processes.json` entry whose `owner_task_id` is the session ID has a live `pid` with its `host_start_time` (the `/proc/<pid>/stat` start field), where `<home>` is the directory of `hermes config path` under the stored base. On a platform without a comparable start time, a live `pid` counts as running. |

To enter each command, settlement clears the composer by the bounded loop below, types the command, closes its completion menu with Escape, and presses Enter until it takes effect, at most three times one second apart while the client remains live.

Each check reads one SGR-preserving capture and indexes its rows identically to the ANSI-stripped view. The bottom border is the last stripped row consisting only of `─` across the pane width. The top border is the nearest preceding full-width border beginning and ending with `─`, with an embedded title allowed; the composer region is strictly between them. Empty means exactly one region row whose stripped text starts `❯ ` and whose remaining cells in that same SGR row are blank or italic placeholder text. Multiple region rows, including blank ones, mean nonempty. Progress compares stripped region text, retaining SGR only for the placeholder decision.

1. Capture and inspect the region; an empty composer completes clearing.
2. Send exactly 16 batches back to back: `Escape` plus 63 `Backspace` keys, then 15 batches of 64 `Backspace` keys. Pause once for the settlement poll interval (200 ms), then capture again.
3. Fail with `the Hermes composer still holds text after clearing` if stripped region text is unchanged; otherwise repeat until empty or the settlement budget expires.

Each round sends 1023 Backspace keys. A draft of 8192 scalars clears within 9 rounds (144 batches) and 10 seconds on tmux and herdr, leaving at least 10 seconds of the 20-second settlement budget for `/stop` and `/exit`; their clear steps complete after one empty capture. Complete nine-round native measurements took 2.676 seconds on tmux and 2.782 seconds on herdr, with the composer first observed empty after round 3; the loop stops after its first verified empty capture. Excess Backspace on an empty composer had no effect. `Ctrl+U` and `Ctrl+A` followed by `Ctrl+K` cleared a visually wrapped single logical line, but the latter retained a multi-line draft after an ineffective submit, so the fixed clear rounds cover both layouts. The total measured clear time satisfies the bound independently of any individual batch duration.

Deletion runs settlement for a `hermes` run before the runner shutdown. Each `hermes` member's settlement, in `member delete` and in `fleet delete`, has its own 20-second budget, and the time it takes extends the 5-second deletion deadline for the remaining cleanup and for later fleet members. A step that does not complete within the settlement budget, or a running registered process, fails with `Error: hermes work for member <ID> could not be verified as stopped: <cause>; member was not deleted.` and keeps the member row. A retry after a recorded session ID and an exited client re-runs only the verification. When the client had already exited before settlement recorded a session ID, deletion prints `warning: hermes client for member <ID> had already exited; its session work may still be running in the Hermes gateway` to stderr and completes. Creation compensation runs the same settlement for a started `hermes` run, and a failure becomes a compensation diagnostic.

CAFleet never stops or restarts a Hermes gateway, kills by process name or
touches a session it does not own.

#### pi member gate

The CAFleet pi extension (§6.3 Asset installation) auto-loads in every pi session and holds both the Director extension (§ Director extensions) and the member gate, a `tool_call` handler that enforces each pi launch mode (§6.9 Launch policy and capability registry). pi 1.5.0 has no per-tool approval and no OS sandbox, so the gate is the member's whole permission posture.

**Activation.** The extension registers the string flag `--cafleet-gate`, and on `session_start` the gate reads `pi.getFlag("cafleet-gate")`:

| Flag value | Gate |
|---|---|
| Absent (a Director or an operator session) | Inert: every `tool_call` passes through the gate untouched |
| `cafleet` | Enforces the tool policy, with the bash allowlist for bash |
| `cafleet-ask` | Enforces the tool policy, and opens the approval form for a bash command the allowlist does not allow |
| Any other value | Blocks every tool call with `the CAFleet member gate received an unknown --cafleet-gate value '<value>'` |
| Reading the flag fails | Blocks every tool call with `the CAFleet member gate could not read --cafleet-gate: <cause>` |

An active gate sets the status `ctx.ui.setStatus("cafleet-gate", "cafleet gate: <mode>")`, so a pane capture shows the posture. The `tool_call` handler awaits a mode promise that the `session_start` handler settles in a `finally` block with the resolved mode or the read failure, so no tool call is evaluated before the mode is known, waits forever or runs ungated after a failed read.

The permission argv stays independent of the pi base, so runtime observation re-derives it from the stored launch policy (§6.9 Launch policy and capability registry). pi 1.5.0 rejects a long option that no loaded extension registers, printing `Error: Unknown option: --cafleet-gate`: a gated member whose extension cannot register the flag (excluded from loading, a stale copy or a load failure) exits at startup, and the runner reports `runner_exited` with the native exit code instead of running the member ungated.

**Tool policy.** Both launch modes apply:

| Tool | `cafleet` | `cafleet-ask` |
|---|---|---|
| `bash` | A command holding a control character is blocked; an allowlist match runs; otherwise blocked | A command holding a control character is blocked without a form; an allowlist match runs; otherwise the approval form |
| `read` | Allowed for a path that is not a `.env` file | Same |
| `grep`, `find`, `ls` | Allowed for a path that is not a `.env` file; a missing or empty `path` is `.`, pi's documented default | Same |
| `edit`, `write` | Allowed for a path inside `ctx.cwd` that is not a `.env` file and not below a protected directory | Same |
| A tool whose name begins with `cafleet_` | Blocked | Same |
| Any other tool, MCP and extension tools included | Blocked | Same |

`read`, `edit` and `write` require a non-empty string `path`; without one the call is blocked, because the gate cannot evaluate a path it cannot read. A path resolves in the order pi 1.5.0's file tools apply (`resolveToCwd`), so each rule judges the file the tool touches: U+00A0, U+2000–U+200A, U+202F, U+205F and U+3000 each become an ASCII space without trimming, and one leading `@` is removed. A text that then begins with `file:`, compared case-insensitively, is blocked, because pi percent-decodes a `file://` URL (`%2Egit` names `.git`) that `path.resolve` reads as a literal relative path. Otherwise `~` becomes the home directory, `~/<rest>` is joined to it, and the result resolves against `ctx.cwd` with `path.resolve`, following no symbolic link. A path is a `.env` file when its final component, compared case-insensitively, is `.env` or begins with `.env.`. A path is inside `ctx.cwd` when its path relative to `ctx.cwd` is not absolute, not `..` and does not begin with `..` and the path separator, so a component that only begins with two dots, such as `..foo`, stays inside. `edit` and `write` are blocked for a path any of whose components relative to `ctx.cwd`, compared case-insensitively, is `.git`, `.pi`, `.agents`, `.claude`, `.codex` or `.opencode`: `.git/hooks` runs whenever a Git command such as `git commit` runs in the working directory, including one the Director allows or dispatches, `.pi/extensions`, `.pi/settings.json`, `.pi/SYSTEM.md` and `.agents/skills` load into a later trusted-project pi session from the same directory, and `.claude`, `.codex` and `.opencode` hold the project configuration, such as Claude Code hooks and permission rules, that a later Claude Code, Codex or OpenCode session from the same directory loads. The rule governs the `edit` and `write` tools only; code that a command the Director allows or dispatches runs writes outside it (Limits below). The case-insensitive comparison covers a case-insensitive file system, such as the macOS default, where `.Git/hooks` names `.git/hooks`. Leading-`!` input runs as pi's `user_bash`, outside `tool_call`, so Director-dispatched `member exec` commands stay under the Director's authorization.

**Block reasons.** A block reason reaches the member as the tool's error result. Besides the activation failures above, the gate returns exactly these reasons, where `<tool>` is the tool name; a file tool checks the path input, then the `file:` rule, then the `.env` rule, then the working directory, then the protected directories:

| Case | Reason |
|---|---|
| Bash command holding a control character, either mode | `the CAFleet member gate refuses a command holding a control character; route the command to the Director if it is required` |
| Bash command not allowlisted, `cafleet` mode | `the CAFleet member gate runs only cafleet message send, poll and show; route the command to the Director if it is required` |
| Bash command not allowlisted, `cafleet-ask` mode in a session without the interactive TUI | `the approval form needs the interactive pi TUI; route the command to the Director if it is required` |
| Form answered `Deny` | `the operator denied this command` |
| Form dismissed | `the approval form was dismissed; the command did not run` |
| `read` without a path input | `the CAFleet member gate cannot read the path input of read; route the read to the Director if it is required` |
| `read`, `grep`, `find` or `ls` on a `file:` path | `the CAFleet member gate refuses a file: URL path for <tool>; route the read to the Director if it is required` |
| `read`, `grep`, `find` or `ls` on a `.env` file | `the CAFleet member gate protects .env files from <tool>; route the read to the Director if it is required` |
| `edit` or `write` without a path input | `the CAFleet member gate cannot read the path input of <tool>; route the change to the Director if it is required` |
| `edit` or `write` on a `file:` path | `the CAFleet member gate refuses a file: URL path for <tool>; route the change to the Director if it is required` |
| `edit` or `write` on a `.env` file | `the CAFleet member gate protects .env files from <tool>; route the change to the Director if it is required` |
| `edit` or `write` outside `ctx.cwd` | `the CAFleet member gate allows edit and write only inside the working directory; route the change to the Director if it is required` |
| `edit` or `write` below a protected directory | `the CAFleet member gate protects .git, .pi, .agents, .claude, .codex and .opencode from edit and write; route the change to the Director if it is required` |
| A tool whose name begins with `cafleet_` | `cafleet tools run only in a Director session` |
| Any other tool | `the CAFleet member gate allows only bash, read, grep, find, ls, edit and write` |

**File-tool cases.** With `ctx.cwd` `/work/proj` outside the home directory, both launch modes return:

| Call | Outcome | Rule |
|---|---|---|
| `write` to `.Git/hooks/x` | Blocked | Protected directory; `.Git` compares equal to `.git` |
| `read` of `.ENV` | Blocked | `.env` file |
| `write` to `src/.Github/x.yml` | Allowed | `.github` is no protected directory |
| `write` to `file:///work/proj/%2Egit/hooks/pre-commit` | Blocked | `file:` path; pi would write `.git/hooks/pre-commit` |
| `read` of `file:///work/proj/%2Eenv` | Blocked | `file:` path |
| `write` to `FILE:///tmp/x` | Blocked | `file:` path; `FILE:` compares equal to `file:` |
| `write` to `@file:///tmp/x` | Blocked | `file:` path; the leading `@` is removed first |
| `write` to `.git<U+00A0>/x` | Allowed | U+00A0 becomes a space, so the gate and pi both name `.git /x` |
| `write` to `@.git/hooks/x` | Blocked | Protected directory; the leading `@` is removed first |
| `write` to `~/x` | Blocked | Outside `ctx.cwd`; `~/x` names a path in the home directory |

**Bash allowlist.** The gate's patterns are a constant in the extension and its content contract:

```ts
const MEMBER_ALLOWLIST = [
  "cafleet message send *", "cafleet message poll *", "cafleet message show *",
];
```

The patterns admit exactly the member role's broker commands, with every word after the subcommand free, so a member passes `--database-url <url>` after the subcommand; every other command, other `cafleet` commands and Director-owned lifecycle and pane-input commands included, matches no pattern.

A command is allowlisted when both hold:

1. **Simple.** The command holds no control character: in both launch modes, a bash command holding a C0 control character other than tab (U+0000–U+0008, U+000A–U+001F), DEL (U+007F) or a C1 control (U+0080–U+009F), anywhere and quoted or not, is blocked with the control-character reason before any allowlist check or approval form. A newline or carriage return separates commands, and bash removes a backslash-newline pair even inside double quotes; an escape sequence or carriage return would make the form render an operation other than the command bash runs. The command, trimmed of surrounding whitespace, is then one simple command. A lexer tracks single quotes, double quotes and backslash escapes; single-quoted text is literal. The command is not simple when an unquoted `;`, `&`, `|`, `<`, `>`, `(`, `)` or backtick, or a backtick inside double quotes, appears; when a quote is unterminated; or when it ends in an unquoted backslash. An unescaped `$` outside single quotes, unquoted or inside double quotes, is accepted only when the next character starts a name (`A-Z`, `a-z` or `_`); any other `$`, such as `$(`, `${`, `$[`, `$((`, `$'`, `$"`, `$?`, `$1`, `$$` or a trailing `$`, makes the command not simple.
2. **Matching.** The whole trimmed command matches at least one pattern, where `*` matches any character sequence, spaces included, and every other character matches itself.

A leading `NAME=value` assignment matches no pattern, so a member passes `--database-url <url>` after the broker subcommand instead.

**Approval form.** Under `cafleet-ask` the gate calls `ctx.ui.select(title, ["Allow once", "Deny"])` for a command that holds no control character and is not allowlisted, with the title:

```
CAFleet member gate: allow this bash command?

$ <command>
```

| Result | Gate action |
|---|---|
| `Allow once` | The command runs once |
| `Deny` | Blocked: `the operator denied this command` |
| `undefined` (Escape, Ctrl+C) | Blocked: `the approval form was dismissed; the command did not run` |

The form has no timeout. Concurrent bash calls of one assistant message are evaluated in sequence through one promise chain, so one form is open at a time, and `Allow once` grants no persistent approval. The title, the two choice labels and pi's select hint line anchor the pi profile's request rule (§6.9 Bundled profiles and native screen fixtures).

**Limits.**

| Surface | Limit |
|---|---|
| Allowed tooling | The gate constrains a cooperative member's tool calls and is not containment. A command the Director allows on the form or dispatches with `member exec` runs the workspace code it executes, which a member can edit (test code, build scripts, task-runner definitions), and that code can write anywhere the user can, protected directories included |
| No sandbox | `read`, `grep`, `find` and `ls` reach any path, and a symbolic link inside the cwd can target a path outside it |
| `.env` files | The `.env` rule refuses only a `read`, `grep`, `find`, `ls`, `edit` or `write` call whose path names a `.env` file; a directory search (`grep` on `.`), an allowlisted broker `message send --file` naming a `.env` file, which copies its contents into a broker message, and a command the Director allows or dispatches still read `.env` contents |
| Protected directories | The rule covers only `edit` and `write`, and its path check follows no symbolic link, so a link elsewhere in the cwd that targets a protected directory stays writable; a command the Director allows or dispatches writes where its own tool writes, as `git` does under `.git/`, and workspace code it runs writes these directories like any other path |
| Project files | Project-root configuration files such as `opencode.json` and context files such as `AGENTS.md` and `CLAUDE.md` stay writable, because members edit them as ordinary work; a later session of any backend loads what a member writes there |
| Delivery fallback | Escape interrupts a running pi turn and cancels an open form. While coverage is `tested`, profile delivery queues instead of falling back to Escape; a `coverage=unknown` member's Escape-first delivery interrupts its run or dismisses the form |
| Multi-line commands | Every command holding a newline is blocked in both launch modes without a form, including `cafleet message send` with a multi-line positional body; a member sends a multi-line body with `--file PATH` from a file written inside the cwd |

#### Native suggestions

A native suggestion is text the native CLI draws in the composer that is not part of its input buffer. A composer showing only a suggestion is empty for every purpose: deferred delivery and held-composer clearing.

A `composer_empty` rule may declare an optional `suggestion` object, `{"style": STYLE}`, where STYLE is `{"faint": true}` or `{"foreground": COLOUR}` with `COLOUR` in the fixture `cell_backgrounds` colour form (§6.9 Bundled profiles and native screen fixtures), indexed or rgb. The rule's prompt marker is the match of its region's `start_pattern` on the region's first row. Such a rule matches when its region matches, the cursor sits on that row at the column right after the match (the rule cursor value `after_prompt`), and every cell from that column up to the row's last non-blank cell carries STYLE. A profile whose checked native version below renders inline suggestions has a fixture-backed suggestion rule:

| Backend | Checked version | Suggestion rendering | STYLE |
|---|---|---|---|
| `claude` | 2.1.322 on tmux and Herdr 0.9.5 | A startup hint `Try "…"` and, while a message is queued, `Press up to edit queued messages`, both after `❯` followed by U+00A0. The post-turn suggestion was not reproduced in 4 runs, with and without `--prompt-suggestions false` | `{"faint": true}` |
| `codex` | 0.166.0 and 0.167.0 | `Ask Codex to do anything` at startup and after every turn | `{"faint": true}` |
| `opencode` | 2.2.7 | `Ask anything, / for commands, @ for context…` at startup only | `{"foreground": {"kind": "rgb", "r": 128, "g": 128, "b": 128}}` |
| `hermes` | 2026.9.24 | A rotating italic placeholder at startup and after a failed turn; post-turn rendering after a completed turn is not researched, because the checked installation has no provider token | `{"foreground": {"kind": "indexed", "index": 136}}` |
| `pi` | 1.5.2 | None | — |

The profiles from claude 2.1.322, codex 0.167.0 and opencode 2.2.7 pair their suggestion rule with a strict idle rule: the backend's idle `composer_empty` rule requires a blank remainder after the prompt marker (its prompt matcher ends in ` *$`), so a hint is empty only through the style-checked suggestion rule, and a typed draft with the cursor moved back to the marker is not empty. These boundaries are the installed versions on which every capability of the profile was re-captured with cell styles. The earlier claude, codex and opencode profiles, the hermes profile (its continuation and submitted evidence needs a completed model turn, which the checked installation cannot run) and the pi profile keep an idle rule that accepts any text after the marker while the cursor sits on it, so on those versions a typed draft with the cursor moved back to the marker reads as an empty composer. Among those, only hermes also declares a suggestion rule.

The OpenCode `composer_empty` rule's region spans the composer row through the footer row and ends on the normal-mode footer, which names the member's agent (`cafleet` or `cafleet-ask`). Shell mode keeps the `┃` marker and replaces that footer with an `esc normal` footer, so a shell-mode composer never counts as empty.

#### Contract error strings

Model and effort rejections are in § Model selection and § Reasoning effort, and the hermes deletion error in § Hermes work settlement. The availability errors are:

| Condition | Error |
|---|---|
| PATH miss | `binary <name> not found on PATH` |
| Selected opencode preset missing or unreadable | `cannot read opencode agent preset at {preset}: {cause}; run 'cafleet setup --coding-agent opencode' first` |
| CAFleet pi extension missing or unreadable | `cannot read the CAFleet pi extension at {path}: {cause}; run 'cafleet setup --coding-agent pi' first` |
| Invalid config-path variable (§6.3 *Config-dir resolution*) | `{var} must be an absolute path (got '{value}')` |
| Non-UTF-8 config-path variable or `HOME` (§6.3 *Config-dir resolution*) | `{var} is not valid UTF-8` |
| `hermes config path` cannot start, times out or exits nonzero | `cannot read hermes configuration through 'hermes config path': {cause}` |
| `hermes config path` prints no absolute file path | `cannot read hermes configuration through 'hermes config path': printed no absolute file path: {stdout}` |

#### Host evidence and native-input authorization

**claude**: A separate evaluator checks the session condition against the transcript; native background work defers evaluation and returns a host result. Director pending notices arrive through one native Monitor task (§ Role eligibility); the installed CAFleet skill's [Claude native recipe](https://github.com/himkt/cafleet/blob/main/assets/skills/cafleet/reference/coding-agents.md#claude-native-director-reception) owns discovery, invocation, renewal and evidence limits. Monitor stops its command at `timeout_ms` (default 300,000 ms, cap 1,800,000 ms), so every start and renewal passes `timeout_ms` 1,800,000 with the 1,770-second `message listen` lease, and manual `poll` reception remains available when Monitor is unavailable or denied.

Native shell probes displayed command/stdout/stderr followed by a model turn; the trigger and any second permission check were not established. The Director authorizes each routed command under its host rules.

CAFleet input cancels no Claude tool approval or `AskUserQuestion` in the root Director's pane, which receives no automatic pane input, or a detected one in a runner-managed member's pane, where input queues while the interaction is open (§6.9 Delivery method and interaction priority). A runner-managed member's Escape-first input (delivery fallback for previews, `member prompt` and `member exec`) can cancel an open prompt the runner did not detect, and Claude Code then reports a user rejection before the input arrives. A rejection followed by a `[cafleet msg …]` line ending with the Escape-dismissal clause is delivery-caused: the member re-asks the question or re-issues the call. A profile-delivered preview ends with the resume clause only and sends no Escape, so a rejection before it stands. A rejection followed by Director prompt or native-command input follows that input, and the Director checks the pane for an open prompt before such dispatch.

**codex**: The objective belongs to the thread, with model-callable lifecycle tools and optional token-budget accounting. Director reception is `cafleet message listen <director-id> --codex-thread "$CODEX_THREAD_ID"`, started once through native `exec_command` (§6.3 Codex forwarding); Codex injects `CODEX_THREAD_ID` into every exec shell. Each forwarded `pending` notice arrives as a queued user turn: it starts a turn on an idle session, waits for the end of a running turn otherwise, and interleaves with native goal continuation turns at turn boundaries. Where the sandbox cannot start the forwarder, for example when bwrap cannot create namespaces, it runs on the escalated path under the host's approval rules. The installed CAFleet skill's [Codex native recipe](https://github.com/himkt/cafleet/blob/main/assets/skills/cafleet/reference/coding-agents.md#codex-native-director-reception) owns the tool call, consumption and recovery.

Codex 0.167.0 and later start or reuse the shared `codex app-server` daemon (openai/codex#58940 retired `daemon_auto_start`), and Director reception through `codex queue` needs that daemon, so a Codex Director cannot launch with `--no-daemon`. Its tool commands then run under the daemon, and its `fleet create` without `--pane` usually refuses at the shared-server check (§6.3 Caller pane verification).

A native shell probe displayed the actual command, exit code, duration and output. Installed protocol metadata describes unsandboxed dispatch while public CLI guidance describes current approval/sandbox settings; the TUI's effective boundary and protocol path were not established. Native input is a Director-authorized action, with no second member-tool permission check assumed.

**opencode**: OpenCode members perform supplied assignments with their selected provider/model, workspace reads/edits, permitted tools and broker communication; the Director coordinates next steps and routed native shell commands and owns explicit pane input dispatch. Leading-`!` input runs a native shell command in `mini`; its effective permission boundary and any second member-tool permission check remain unestablished, so the preset's member-tool permissions and the Director's command authorization stay in force. An OpenCode Director runs OpenCode 2.2.0 or later as `opencode mini --standalone` from the project directory with the default agent, so plugins load in its private server with the launcher's environment; the shared background service passes no client environment to plugins and is not a supported Director launch. The `cafleet` plugin supplies reception and the goal extension (§ Director extensions). OpenCode 2.2.0 gives every agent a base policy whose first rule allows every action, so the plugin's `cafleet` action resolves to allow on a default Director; a configuration whose broad `ask` or `deny` rule overrides that base allow restores it with `{ "action": "cafleet", "resource": "*", "effect": "allow" }`.

**hermes**: Hermes 2026.9.24 ships a user-operated `/goal` command with `status`, `show`, `pause`, `resume`, `clear`, `wait <pid>`, `unwait` and `gate` subcommands. A Hermes Director runs the TUI client (`hermes chat --tui`, or `display.interface: tui`), because turns injected through the gateway run without rendering in the classic CLI; members keep `--cli`. The Director receives pending notices through the `cafleet` plugin's `cafleet_listen` tool, which runs the listener inside the per-home gateway (§ Director extensions): each notice becomes its own turn, and a notice injected during a busy turn runs after that turn ends without interrupting it. Native `/goal` set, status, pause, resume and clear work in the TUI client, and continuation turns already queued before `/goal clear` still run after it. In the classic REPL and the TUI client, leading-`!` input runs a native shell command locally; the Director authorizes each routed command under its host rules. A Hermes Director's terminal tools run in the per-home gateway (`hermes gateway run`), so its `fleet create` without `--pane` always refuses at the shared-server check (§6.3 Caller pane verification). CAFleet makes no goal lifecycle calls and preserves activation, pause, completion and gate/wait authority.

**pi**: pi 1.5.0 has no native goal, no native facility that pushes external text into a session and no per-tool approval; the CAFleet pi extension supplies reception, the goal extension and the `cafleet_ask` decision surface (§ Director extensions), and the member gate (§ pi member gate). A pi Director runs plain `pi` (the fullscreen TUI) from the project directory, so the extension auto-loads; pi's bash tool runs under the pane's `pi` process, so `fleet create` verifies and registers the pane without `--pane`. A pi Director runs its own tool calls without approval, and the member gate is inert in its session. The installed CAFleet skill's [pi native recipe](https://github.com/himkt/cafleet/blob/main/assets/skills/cafleet/reference/coding-agents.md#pi-native-director-reception) owns the tool calls, consumption and recovery. Leading-`!` input runs a shell command as `user_bash` outside the member gate, stores its output for the next prompt and starts no turn, so after `member exec` the Director sends an explicit continuation; the Director authorizes each routed command under its host rules. Enter while a member works queues the text as a steering message.

#### Director extensions

The extensions §6.3 Asset installation installs supply Director reception for `hermes`, `opencode` and `pi`, and the `opencode` and `pi` goal extensions. Each runs only the `cafleet` executable its executable record names, never a path from a tool argument, as `<recorded cafleet> message listen <director_id>` without `--timeout`, adding `--database-url <url>` when the tool received a `database_url` (§6.3 Database selection). The plugins replace no receiver automatically; every replacement belongs to the Director's receiver recovery.

**Pass-through.** Each receiver delivers every listener stdout line verbatim as `[cafleet listen record] <line>`, so each `pending` notice (§6.3 `message listen`) reaches the session as one delivery; the receivers inspect only the `{"event":"closed"` prefix, which ends the receiver. Each `cafleet_listen` tool carries this model-facing description:

| Extension | `cafleet_listen` description |
|---|---|
| Hermes plugin | `Start CAFleet Director reception for this session: each turn beginning '[cafleet listen record]' carries a pending notice; run the command in its 'next' field once, which consumes and returns every unpolled message, and an empty [] result means an earlier poll already consumed them.` |
| OpenCode plugin | The Hermes text |
| pi extension | The Hermes text with `each turn beginning` replaced by `each message beginning` |

**Director verification.** `message listen` accepts any active member, so each Director-only tool verifies its `director_id` itself: it runs `<recorded cafleet> member show <director_id> --json`, adding `--database-url <url>` when given, and requires exit 0 with `"kind": "director"`; any other outcome returns the tool's member-refusal error with the CLI's stderr appended. Like the Director-only command guard (§6.3 Director-only command guard), the check is cooperative: it stops a member that passes its own ID.

| Surface | Hermes plugin | OpenCode plugin | pi extension |
|---|---|---|---|
| Receiver tool | `cafleet_listen(director_id: integer, database_url?: string)` in toolset `cafleet` | `cafleet_listen(director_id: integer, database_url?: string)` | `cafleet_listen(director_id: integer, database_url?: string)` |
| Verified tools | `cafleet_listen` | `cafleet_listen`, `cafleet_goal_status`, `cafleet_goal_progress` and `cafleet_goal_complete`, each with a required `director_id` and an optional `database_url` | `cafleet_listen`, `cafleet_ask`, `cafleet_goal_status`, `cafleet_goal_progress` and `cafleet_goal_complete`, each with a required `director_id` and an optional `database_url` |
| Session scope | One receiver per calling session key | One receiver per calling `sessionID` | One receiver per extension runtime |
| Success result | `{"ok": true, "pid": <pid>, "session_key": <key>}`, plus `"existing": true` for a live receiver | `{ ok, pid, sessionID }`, plus `existing: true` for a live receiver | `{"ok": true, "pid": <pid>, "sessionId": <id>}`, plus `"existing": true` for a live receiver |
| Delivery per stdout line | Ordered injection of `[cafleet listen record] <line>` into session `<key>`'s durable input queue, each committed before the next line | `session.synthetic({ sessionID, text: "[cafleet listen record] <line>", delivery: "queue" })` | `pi.sendMessage({customType: "cafleet-listen", content: "[cafleet listen record] <line>", display: true}, {triggerTurn: true, deliverAs: "followUp"})`, in line order |
| Member refusal | A failed Director verification returns `{"ok": false, "error": "cafleet_listen runs only in a Director session: <stderr>"}` | A failed Director verification refuses the tool with `cafleet tools run only in a Director session: <stderr>` | A failed Director verification returns `{"ok": false, "error": "cafleet tools run only in a Director session: <stderr>"}` with `isError: true`. An active member gate blocks every `cafleet_*` tool first (§ pi member gate) |
| Executable record error | `{"ok": false, "error": "executable record <path>: <cause>; run 'cafleet setup --coding-agent hermes'"}` | `ok: false` with the record path and `run 'cafleet setup --coding-agent opencode'` | `{"ok": false, "error": "executable record <path>: <cause>; run 'cafleet setup --coding-agent pi'"}` |
| Child environment | The gateway's environment | The server's environment | The pi process's environment |
| Receiver end | Session finalize, plugin unload, 30 s with no client subscriber, a raising or overdue injection, or 600 s of refusals for one record kills the child | Plugin cleanup kills every child when the server ends | `session_shutdown`, for every reason, kills the child with `SIGTERM` |

A rejected `synthetic` call kills the child. A Hermes injection that raises or exceeds its 10 s deadline, or one record refused for 600 s, kills the child, logs the cause and attempts the closed notice below once through ordered injection. A child that exits unexpectedly, without a `closed` record and without the plugin stopping it, queues `[cafleet listen closed] exit <code>: <last stderr line>` into the owning session. A Hermes plugin unload for a still-attached session injects `[cafleet listen closed] the plugin unloaded: exit <code>: <last stderr line>` once through ordered injection after the child exits; a draining gateway refuses it, without retry. A child the plugin stops through session finalize, client detach, plugin cleanup or a Hermes unload for a detached session queues no closed notice, and each receiver attempts at most one. The OpenCode plugin decides the notice on the child's `close` event, after its stdio drains, so a final `closed` record is read first; a child `error` event, such as an asynchronous spawn failure, removes the receiver and queues the notice. An OpenCode child exiting by `SIGTERM` is an intentional stop by plugin cleanup or host shutdown (which can signal the child before cleanup runs) and queues no closed notice; a non-zero exit code or any other signal queues it. A `closed` record is delivered as a record and ends the receiver.

**Downstream delivery queues.** Every Director receiver, native or extension, hands notices to a host queue that drains at turn boundaries. Installed upstream source, checked 2026-11-15, records the limits of every row below except `codex`, whose provenance follows the table:

| Backend | Queue | Version | Cap and refusal | Drain |
|---|---|---|---|---|
| `claude` | Monitor stdout line events, each queued as a task notification | 2.1.316 | No cap. A token bucket of 10 events, refilled one per 2 s, suppresses (drops) events above that rate and later reports `[N events suppressed — …]`; suppression sustained past 30 s stops the monitor. | Lines within 200 ms join one event |
| `codex` | Thread submission queue (`codex queue`) | 0.165.0 (measured), 0.166.0 (source) | 100 submissions; a further submission exits non-zero with `queue cannot contain more than 100 submissions` | One submission per turn boundary; no list or retract |
| `opencode` | Session inbox rows from `session.synthetic` with `delivery: "queue"` | 2.2.5 | No cap and no refusal; each call inserts one persisted inbox row | One `queue` row per turn boundary |
| `hermes` | Gateway busy queue for ordered injection | 2026.9.24 | 32 pending rows; a further injection returns `False` (§ Ordered injection) | One row per turn |
| `pi` | In-memory follow-up queue from `deliverAs: "followUp"` | 1.5.0 | No cap and no refusal | One message per agent stop under the default `followUpMode` `one-at-a-time` |

The `codex` 100-submission cap was measured live on Codex 0.165.0, checked 2026-11-06, and its refusal string is present in the installed Codex 0.166.0 source, checked 2026-11-15. The 2026-11-17 subscribe probe on Codex 0.167.0 observed no queue refusal.

Claude Code Monitor also truncates each line at 500 characters and each event at 3,000. Its official documentation states no batching window, rate limit or truncation; those values come from the installed source. Under the §6.3 `message listen` notice bound, a notice stays far below the Monitor rate.

**Hermes plugin.** The plugin declares tool `cafleet_listen` and hook `on_session_finalize`. It depends on three private Hermes core seams outside the plugin API, which the tool resolves on every call:

| Seam | Use |
|---|---|
| `hermes_cli.plugin_session.session_key()` | Calling session key |
| `gateway.session_authorities.authority_for_home(...)` → `authority.sessions[...].subscribers` | Client attachment |
| The published gateway runner (`ctx._manager._gateway_message_injector[0]`), its `_gateway_loop` and `_dispatch_plugin_message_injection(session_key=, content=, plugin_id=, plugin_home=)` | Ordered injection |

`hermes plugins validate` flags the first two seams' imports as private core imports. A failed import, lookup or attachment read returns `{"ok": false, "error": "<seam>: <exception>"}` and starts no receiver; an empty session key returns `{"ok": false, "error": "cafleet_listen must be called from a session turn"}`. Ordered injection bypasses the consent check of `ctx.inject_message`, so before spawning the child the tool requires `ctx._gateway_injection_allowed()`, which holds when `plugins.entries.cafleet.allow_gateway_injection` is `true` (§6.3 Hermes plugin enablement), and otherwise returns `{"ok": false, "error": "plugins.entries.cafleet.allow_gateway_injection is not true; run 'cafleet setup --coding-agent hermes'"}`; it then verifies `director_id`. The child selects its database only through `--database-url`, so an omitted `database_url` selects the default database exactly as in the Director's shell. The plugin decodes child output as UTF-8, replacing invalid bytes, and treats a stdout read failure as the end of output. A watchdog checks every 5 s that the session has at least one subscriber; a read error kills the child. `on_session_finalize` resolves the finalized session's route through the session authority and stops the receiver with that session key, so a session id rotated by context compression still matches. The unload callback stops every receiver and confirms its exit, so plugin re-discovery leaves no untracked receiver; re-discovery leaves the gateway and its sessions running, so the unload closed notice, sent outside the callback after the receiver's delivery ends, tells a still-attached Director to re-arm reception.

**Ordered injection.** `ctx.inject_message` returns `True` once it schedules its dispatch coroutine on the gateway loop, before the record is committed, so consecutive calls can commit out of order. The plugin instead submits each record's `_dispatch_plugin_message_injection(...)` coroutine to the runner's `_gateway_loop` with `asyncio.run_coroutine_threadsafe` and waits up to 10 s for its result before reading the next line. A `False` result is a refused injection, treated as transient (for example the session authority's 32-row admission queue cap, a draining gateway or a briefly detached client): the plugin keeps the record, waits 1 s and retries it, and later records wait behind it. The retry checks no attachment, so a detached session keeps the 30 s detach grace and the watchdog alone ends its receiver. Under the §6.3 `message listen` notice bound, listen alone fills the 32-row queue only when more than 32 consuming polls or ceiling firings queue notices behind one busy turn; otherwise the refusal path is reached when other injections fill the queue.

**OpenCode plugin.** One V2 plugin file defines `Plugin.define({ id: "cafleet", setup })`. Each tool calls `ctx.permission.assert({ sessionID, agent, source, action: "cafleet", resources: [<tool name>] })` before any work, then verifies its `director_id` (Director verification above). The `context` hook registers in every session; it and the event loop act only on a session holding a goal stored by the operator-typed `/cafleet-goal` command or by a goal tool whose `director_id` verified. `cafleet_listen` returns `ok: false` when the spawned child has no pid. Its goal extension stores `{objective, status, generation, continuations, idle_streak}` under storage key `goal/<sessionID>` and mirrors it in the session instruction entry `cafleet-goal`, which first appears in 2.2.0:

| Operation | Surface | Effect |
|---|---|---|
| set | `/cafleet-goal <objective>` | Store status `active`, the next `generation`, `continuations: 0` and `idle_streak: 0`, put the instruction entry, queue the first continuation |
| status | `/cafleet-goal status`; tool `cafleet_goal_status` | The command queues a synthetic `[cafleet goal] status: <json>` turn; the tool returns the stored JSON |
| pause | `/cafleet-goal pause` | Status `paused`, bump `generation`, update the entry |
| resume | `/cafleet-goal resume` | Status `active`, bump `generation`, reset `idle_streak` and `continuations`, queue a continuation |
| clear | `/cafleet-goal clear` | Bump `generation`, remove the `goal/<sessionID>` key and the entry |
| complete | tool `cafleet_goal_complete(summary)` | Status `complete`, bump `generation`, update the entry |
| progress | tool `cafleet_goal_progress(note)` | Reset `idle_streak` |

- **Generations.** Generations increase monotonically per session. Every bump, and the generation set assigns, stores the new value under storage key `goal-generation/<sessionID>`, which clear keeps; the next generation is that value plus one, so a continuation queued before a clear never matches a later goal.
- **Continuation.** On `session.execution.succeeded` for a session with an `active` goal, the plugin queues a synthetic item with `delivery: "queue"`, metadata `{ cafleetGoalGeneration: <generation> }` and text `[cafleet goal] Continue working toward the active goal: <objective>. Call cafleet_goal_progress after each step and cafleet_goal_complete when it is achieved.` A `session.execution.interrupted` with `reason: "user"` pauses the goal.
- **Serialization.** Every goal operation for a session (event handler, command and tools) runs through one per-session promise chain and re-reads the stored goal inside it before writing, so a concurrent pause or clear is never overwritten.
- **Stale continuations.** The `context` session hook removes each `[cafleet goal]` continuation after the last assistant message, the undelivered tail, whose generation differs from the stored one or whose goal is not `active`, so a continuation queued just before `pause`, `clear` or `complete` never reaches the model; earlier history stays intact.
- **Event failures.** A failure while handling one event is logged and leaves the event loop running. Any such failure while the session's goal is `active`, a transient storage or `synthetic` error included, pauses the goal and queues `[cafleet goal] paused: the goal cannot continue (<cause>); the operator resumes it with /cafleet-goal resume` with `resume: false`.
- **No-progress stop.** A continuation turn that ends without a `cafleet_goal_progress` or `cafleet_goal_complete` call increments `idle_streak`. At 3 the goal pauses and the plugin queues `[cafleet goal] paused: no progress in 3 consecutive continuation turns; the operator resumes it with /cafleet-goal resume` with `resume: false`. The 50-continuation cap also pauses the goal and queues `[cafleet goal] paused: reached 50 continuations; the operator resumes it with /cafleet-goal resume` with `resume: false`; resume starts a fresh budget of 50.
- **Persistence.** The instruction entry carries the goal across compaction. A stored `active` goal for a `sessionID` is restored on the server's first event, tool call or command for that `sessionID`, so after a restart the Director's re-arming `cafleet_listen` turn restores it and the next `session.execution.succeeded` queues the continuation. Storage is shared by every server on the same OpenCode data directory, while restoration stays scoped to the sessions a server serves; setup scans no storage, so a member server or any other server sharing the config directory never restores the Director's goal.

**pi extension.** One file loads in every pi session, members included, and holds the member gate (§ pi member gate) beside the Director tools. Each tool reads the executable record at `new URL("./cafleet.executable", import.meta.url)` and runs Director verification through `node:child_process` `execFile` before any work; its result text is its JSON object. The goal hooks act only on a session holding a goal set by the operator-typed `/cafleet-goal` command or by a goal tool whose `director_id` verified, so they stay inert in members.

**pi receiver.** `cafleet_listen` spawns the listener with `node:child_process` `spawn` and returns `{"ok": false, "error": "cafleet message listen did not start"}` when `child.pid` is undefined; the asynchronous spawn error is not reported, because the receiver is already marked stopped. `followUp` delivery starts a turn on an idle session; on a busy session it waits until the agent has no more tool calls, so a record never interrupts a running turn and arrives no later than its end, and records that arrive while a run is starting join that run.

| Event | Extension action |
|---|---|
| The child closes (`close` event, after its stdio drains) without a `closed` record, and the extension did not stop it | Sends `[cafleet listen closed] exit <code>: <last stderr line>` the same way |
| A child `error` event, such as an asynchronous spawn failure | Removes the receiver and sends the same closed notice |
| A `sendMessage` call throws | Kills the child and sends the closed notice once |
| A `closed` record | Delivered as a record; the receiver ends |
| `session_shutdown`, for every reason | Marks the receiver stopped and kills the child with `SIGTERM`; a stopped receiver sends no closed notice |

For a `session_shutdown` reason of `reload`, `new`, `resume` or `fork`, the pi process keeps running and loads a replacement runtime. When that shutdown stopped a live receiver, the old runtime stores `{reason}` under `globalThis[Symbol.for("cafleet.pi.receiver-stopped")]`; a runtime with no live receiver stores nothing and the new runtime sends no notice. The new runtime's `session_start` reads and clears it, then sends `[cafleet listen closed] the extension runtime was replaced (<reason>); re-arm with cafleet_listen` with `triggerTurn: true`. The reason `quit` (Ctrl+C, Ctrl+D, `SIGHUP`, `SIGTERM`) stores nothing, because the process exits, and a `SIGKILL` of the pi process runs no handler.

**pi goal extension.** The state `{objective, status, generation, continuations, idle_streak}` is persisted with `pi.appendEntry("cafleet-goal", state)` after every change, outside LLM context, and restored on `session_start` from the last `cafleet-goal` entry on `ctx.sessionManager.getBranch()`. `status` is `active`, `paused`, `complete` or `cleared`.

| Operation | Surface | Effect |
|---|---|---|
| set | `/cafleet-goal <objective>` | Status `active`, next generation, both counters 0; sends the first continuation |
| status | `/cafleet-goal status`; tool `cafleet_goal_status` | The command shows `[cafleet goal] status: <json>` through `ctx.ui.notify`; the tool returns the stored JSON |
| pause | `/cafleet-goal pause` | Status `paused`, bump `generation` |
| resume | `/cafleet-goal resume` | Status `active`, bump `generation`, reset both counters; sends a continuation |
| clear | `/cafleet-goal clear` | Status `cleared`, bump `generation`; the objective is dropped |
| complete | tool `cafleet_goal_complete(director_id, summary, database_url?)` | Status `complete`, bump `generation` |
| progress | tool `cafleet_goal_progress(director_id, note, database_url?)` | Reset `idle_streak` |

- **Generations.** Generations increase monotonically per session; the `cleared` entry keeps the last generation, and set uses that value plus one.
- **Continuation message.** A `custom_message` with `customType` `cafleet-goal`, `details: {generation}`, `display: true` and the content `[cafleet goal] Continue working toward the active goal: <objective>. Call cafleet_goal_progress after each step and cafleet_goal_complete when it is achieved.` Set and resume send it with `pi.sendMessage(..., {triggerTurn: true, deliverAs: "followUp"})`.
- **Automatic continuation.** For an `active` goal, `agent_before_settle` (1) increments `idle_streak` when the run was a continuation run without a `cafleet_goal_progress` or `cafleet_goal_complete` call; (2) at 3, pauses and returns the entry `[cafleet goal] paused: no progress in 3 consecutive continuation turns; the operator resumes it with /cafleet-goal resume` with `continue: false`; (3) at 50 continuations, pauses and returns `[cafleet goal] paused: reached 50 continuations; the operator resumes it with /cafleet-goal resume`; (4) otherwise increments `continuations`, persists and returns the continuation entry with `continue: true`.
- **Tracking.** pi 1.5.0 skips `before_agent_start` for a continuation that `agent_before_settle` requests, so the extension tracks runs with two in-memory flags. `pendingContinuation` is set whenever the extension starts a continuation (a returned `continue: true`, or the set and resume messages); `progressed` is set by every `cafleet_goal_progress` or `cafleet_goal_complete` execution. Each `agent_before_settle` reads both flags and clears them before it returns, a returned continuation sets `pendingContinuation` again, and an aborted run clears both at `agent_settled`.
- **Interrupt.** `agent_settled` with `event.aborted` on an `active` goal pauses it and shows `[cafleet goal] paused: the run was interrupted; the operator resumes it with /cafleet-goal resume` through `ctx.ui.notify`.
- **Stale continuations.** The `context` hook removes each `cafleet-goal` continuation after the last assistant message whose generation differs from the stored one or whose goal is not `active`; earlier history stays intact.
- **Goal context.** The `context` hook runs before every model request, inside a continuation chain too. While the goal is `active`, it appends one transient user-role message, `[cafleet goal] Active CAFleet goal: <objective>`, after the last message of `event.messages`; the message is never persisted and is rebuilt on every request, so it survives manual and threshold compaction and keeps the provider's cached prefix intact. A `paused`, `complete` or `cleared` goal appends nothing, so a paused goal gives the model no standing instruction; resume sends the objective again in its continuation message.
- **Restart.** `pi --continue` (or `--session <id>`) on the same session restores the state at `session_start`; the Director's re-arming `cafleet_listen` turn then ends at `agent_before_settle`, which continues an active goal without further input.
- **Failures.** Each handler is wrapped in `try`/`catch` and logs a failure's cause. While the goal is `active`, a failure also pauses the goal and shows `[cafleet goal] paused: the goal cannot continue (<cause>); the operator resumes it with /cafleet-goal resume`.

**pi decision surface.** `cafleet_ask(director_id, question, choices, database_url?)` registers with `executionMode: "sequential"`. After Director verification it requires `ctx.hasUI`, otherwise returning `{"ok": false, "error": "cafleet_ask needs the interactive pi TUI"}`, and requires `choices` to be 1–9 non-empty, distinct strings, none equal to `Other`, otherwise returning `ok: false` naming the violated rule. It calls `ctx.ui.select(question, [...choices, "Other"])`, and for `Other` then `ctx.ui.input(question, "")`:

| Outcome | Result |
|---|---|
| A listed choice | `{"ok": true, "status": "answered", "answer": "<choice>", "choice": <1-based index>}` |
| `Other` with non-empty text | `{"ok": true, "status": "answered", "answer": "<text>", "choice": null}` |
| Cancelled, or `Other` with empty text | `{"ok": true, "status": "cancelled"}`, which records no decision |

### 6.8 Configuration

Settings retain only the selected SQLite URL, built from `--database-url` (§6.3 Database selection). CAFleet reads no `CAFLEET_*` environment variable; it reads the backend config-location variables and `HOME` (§6.3 Config-dir resolution), and only `fleet create` and doctor's `multiplexer` and `multiplexer.caller_pane` rows read the caller's multiplexer environment, verified against the caller's process ancestry (§6.5 Backend selection, §6.3 Caller pane verification). Spawned members receive, when the creator sets it, the selected backend's config-location variable (§6.3 *Member environment*); identity and the database are rendered into prompt literals (§6.3 Spawn-prompt substitution).

### 6.9 Member PTY runtime

#### Launch policy and capability registry

`cafleet member create --permission-mode <MODE>` selects a launch policy. Resolve the coding-agent backend first, validate `MODE` against that backend's typed launch-policy catalog, then allocate the member or pane. An omitted value selects the backend default below; an empty, unsupported or unavailable explicit value fails with usage exit `2`, naming the backend and supported choices. Selection is launch-only and is independent of Director policy; it introduces no fleet inheritance, environment default or live policy mutation.

| Backend | `auto` modes | `manual` modes | Explicit selection mapping |
|---|---|---|---|
| Claude | `auto` (default), `dontAsk`, `bypassPermissions` | `default`, `manual`, `acceptEdits`, `plan` | The native permission-mode value through `--permission-mode`; retain broker-tool allowance, prompt suggestions, name and model/effort boundaries. |
| Codex | `approve-for-me` (default), `never` | `on-request` | `on-request` and `never` map to `--ask-for-approval <mode> --sandbox workspace-write`; `approve-for-me` maps to the single `--approve-for-me` flag, which selects the workspace-write sandbox itself. |
| OpenCode | `cafleet` preset | `cafleet-ask` preset (default) | `--agent <mode>`, selecting the agent preset of that name; explicit `cafleet` records `mode_source=explicit`. |
| Hermes | `smart` (default) | — | No permission argv for any native version; the choice is recorded only, and the operator configuration's `approvals.mode` governs the native session. |
| pi | `cafleet` | `cafleet-ask` (default) | `--cafleet-gate <mode>`, selecting the CAFleet member gate's mode (§6.7 pi member gate); explicit `cafleet` records `mode_source=explicit`. |

Each backend has one default (omitted) mode, marked in the table above. Every launch mode carries an `auto` or `manual` class, a documentation and catalog property: `--permission-mode` values stay backend-native, and the class changes no argv, run-record field or CLI contract.

| Class | Definition |
|---|---|
| `auto` | Native automatic review or a fixed allow/deny policy: a command the policy does not settle goes to the automatic reviewer, or is allowed or denied without a native prompt. An `auto` mode may still raise an approval prompt when its reviewer escalates. |
| `manual` | Native interactive approval: the backend asks a human on its native surface for operations its rules do not settle. |

Claude `dontAsk` denies tools that are not pre-approved without prompting, Claude `bypassPermissions` allows without prompting, and Codex `never` returns failures without asking, so each is a fixed policy. Claude `acceptEdits` and `plan` still prompt. When an approval prompt that the run's bundled profile covers appears in a member pane, the runner reports it as `interaction_opened`, and the Director answers it with a single-request allow or deny through the permission commands (§6.3 Permission commands).

The omitted Claude and Codex selections are each backend's native automatic approval mode: an automatic reviewer decides the operations that no native rule settles, and a refused operation returns to the member, which routes it to the Director like any denial. An omitted selection is not validated against the version catalog, so a member whose probed version is newer than every profile boundary still launches with the default; a native version that rejects `auto` or `--approve-for-me` exits immediately with `runner_exited` (below).

The bundled backend policy catalog is the single source of truth for each backend's launch-policy catalog, default, exact argv mapping and detector/composer/answer profiles; help and usage errors report its choices, and each backend accepts them for every native version. Claude accepts the native `--permission-mode` values, documented aliases included, verified from installed help and parser evidence: the verified Claude Code 2.1.302 and 2.1.312 accept canonical `default` and its `manual` alias, help displays `manual`, and the [official permission-mode reference](https://code.claude.com/docs/en/permission-modes) documents both. Preserve the explicitly requested spelling in argv and launch intent. The Codex choices are verified from the installed `codex --help` of Codex 0.163.0; `approve-for-me` names the native `--approve-for-me` flag, which routes approval requests through automatic review, and explicit `on-request` and `never` inherit the user's Codex configuration, including an `approvals_reviewer` setting that can answer escalations without a native form. The OpenCode choices select agent presets rather than a native permission flag: `cafleet` resolves every shell check to `allow` or `deny`, and `cafleet-ask` asks for every shell command outside the member broker commands, so OpenCode opens a native permission form. Hermes emits no argv because no Hermes launch flag selects an approval mode. The pi choices select the mode of the CAFleet member gate, because pi 1.5.0 has no native per-tool approval: `cafleet` runs the member broker commands and blocks every other bash command without a form, so it renders no approval form, and `cafleet-ask` opens the gate's approval form for every bash command outside the member broker commands.

Creation success establishes that the child was spawned, not that its native policy was accepted. A fast native exit after spawning, including policy rejection, produces `runner_exited` with the actual code/signal and a Director inbox event; the runner preserves the chosen mode rather than retrying with another one. An error from the spawn operation itself still fails creation and follows compensation. There is no post-spawn liveness window.

Record `requested_mode` as nullable text, `resolved_mode` as required text, `mode_source` as `default` or `explicit`, and the exact non-secret permission/sandbox argv in the run record. These fields state launch intent:

| Selection | `requested_mode` | `resolved_mode` | `mode_source` | `permission_argv_json` |
|---|---|---|---|---|
| Omitted Claude | `null` | `auto` | `default` | `["--permission-mode","auto"]` |
| Omitted Codex | `null` | `approve-for-me` | `default` | `["--approve-for-me"]` |
| Explicit Codex `never` | `never` | `never` | `explicit` | `["--ask-for-approval","never","--sandbox","workspace-write"]` |
| Omitted Hermes | `null` | `smart` | `default` | `[]` |
| Omitted pi | `null` | `cafleet-ask` | `default` | `["--cafleet-gate","cafleet-ask"]` |
| Explicit pi `cafleet` | `cafleet` | `cafleet` | `explicit` | `["--cafleet-gate","cafleet"]` |

An installed version, argv or selected profile alone proves no effective authorization decision.

Launch is independent of every profile capability; detection, composer, submission and answer capabilities resolve independently through the selected profile (§ Profile selection by minimum version), and answer profiles declare each semantic decision independently. A bundled tested answer declaration rests on measured single-request answer stages. Unverified components remain unknown or unsupported. A failed probe or a backend without bundled profiles yields `coverage=unknown` and an initial coverage notification; it still permits manual terminal operation, records the selected mode and delivers ordinary programmatic input through delivery fallback, and automatic/fixed policies remain useful where screen detection is unknown. The `cafleet` OpenCode preset's returned denials continue through member evidence routing; returned tool responses are observable only when the native terminal exposes them.

##### Native executable resolution

`member create` resolves the backend's binary to one absolute executable path during the backend availability check, using the creator process's `PATH`. It searches the absolute `PATH` entries in order, skipping empty and relative entries, and selects the first `<entry>/<binary>` that is a regular file with an execute permission bit after following symbolic links. The selected path is used as found, without symbolic-link resolution, so a link or wrapper keeps its own name. When no entry matches, creation fails before registration with exit 1 and `Error: binary <name> not found on PATH`.

The version probe below runs that exact path, and the same path is `argv[0]` of the runner's literal child argv; the pane's own `PATH` and shell configuration play no part in selecting the native executable.

##### Native version probe

`member create` records the installed native version of the resolved backend. After launch-policy validation and the backend availability check, and before registration, it runs the resolved executable (§ Native executable resolution) with exactly the argument `--version`, so each backend reports its own version. The probe has null stdin and captures at most 4096 bytes each of stdout and stderr. It ends within two seconds, and those two seconds belong to the command's ten-second creation budget, which starts before the probe. A probe still running at its bound is killed.

The probe yields a version only when it exits with status `0` and the first stdout line has exactly one whitespace-separated token that, after stripping one leading `v`, parses as canonical SemVer `major.minor.patch` with optional prerelease and build metadata. That stripped token is recorded as the run's `native_version`. An incomplete version such as `2.1` is not a version token and is never normalized into one:

| First stdout line | Recorded `native_version` |
|---|---|
| `2.1.302 (Claude Code)` | `2.1.302` |
| `codex-cli 0.163.0` | `0.163.0` |
| `opencode v2.1.4` | `2.1.4` |
| `1.5.0` | `1.5.0` |
| `claude 2.1.310-beta.1+build.7` | `2.1.310-beta.1+build.7` |
| `v2.1.4 2.1.5` | Probe failure: several version tokens |
| `tool 2.1` | Probe failure: no version token |

Hermes replaces the one-token rule with its own first-line rule. The first stdout line must match `^Hermes Agent v\S+ \((?P<version>[^)]+)\)$` (§ Pattern syntax), and the captured value must parse as canonical SemVer; that value is the recorded `native_version`. A non-matching line, or a capture that is not canonical SemVer, is a probe failure:

| First stdout line (hermes) | Recorded `native_version` |
|---|---|
| `Hermes Agent vunknown (2026.9.24)` | `2026.9.24` |
| `Hermes Agent v0.22.4 (2026.9.24)` | `2026.9.24` |
| `Hermes Agent 2026.9.24` | Probe failure: no Hermes version line |
| `Hermes Agent vunknown (2026.09.24)` | Probe failure: not canonical SemVer |

For hermes the probe runs concurrently with `hermes config path` under their shared deadline (§6.7 Hermes availability preflight). The probed version then selects the run's bundled profile (§ Profile selection by minimum version).

A probe that fails a rule above, cannot start, times out or prints non-UTF-8 output records `native_version` null and `coverage=unknown`, and creation continues. The command writes one stderr line, `Warning: native version unavailable for BACKEND: CAUSE; coverage unknown.`, and leaves its stdout text and JSON unchanged.

##### Profile selection by minimum version

Each profile declares one SemVer `min_version`, and a backend's profiles form an ordered stack of compatibility boundaries. Multiplexer, platform and permission mode introduce no additional selection axis. For a successfully parsed native version `v`:

```text
eligible = profiles where profile.backend == backend and profile.min_version <= v
selected = eligible profile with greatest min_version
```

| Native version and catalog state | Creation behavior | `coverage` |
|---|---|---|
| At a boundary or between boundaries | Launch with the selected profile | `tested` when the selected profile declares detection, otherwise `unknown` |
| Newer than every boundary | Launch with the newest eligible profile | The same; there is no upper cap and no version-mismatch warning |
| Below the backend's first boundary | Exit 1 before member/run registration or pane creation | — |
| Probe failure | Continue launch with the probe warning | `unknown`, and ordinary input uses delivery fallback |
| Backend with no profile entries | Continue launch | `unknown`, and ordinary input uses delivery fallback |

A known version below the first boundary fails with `Error: unsupported native version for BACKEND: VERSION; minimum supported version is MIN_VERSION`. For example, boundaries `2.1.302` and `2.1.310` select the first profile for `2.1.308` and `2.1.310-beta.1` and the second for `2.1.310` and `2.2.0`, while `2.1.301` is unsupported. A prerelease can itself be a boundary when a measured prerelease requires a new profile.

One SemVer 2.0.0 precedence comparison serves profile validation, native-version admission, fixture provenance validation and selection. Numeric components compare numerically, prerelease identifiers by SemVer precedence, and a release ranks above its prereleases. Build metadata contributes no precedence. Selection is independent of catalog order.

The run retains the actual probed `native_version`; the backend and that version determine the selected profile within one binary. A composer plan and an answer capability retain the actual run version, while each fixture record keeps its original capture version; input validation compares the backend and native version with the run rather than requiring the run version to equal a capture version or boundary.

The runner pins its selected profile for its lifetime. Fresh creation and observer initialization verify that deterministic selection from the recorded backend and native version yields the recorded coverage, so a `tested` coverage requires a selected profile and a null native version requires `unknown`. Historical run and event reads preserve their recorded identities; recovery reports a recorded `tested` coverage for which no profile is selected now explicitly rather than attaching another profile. A catalog update affects newly launched runners only.

The rules match screen patterns, so a changed native UI fails safe for detection: an unmatched form yields the `unknown` or `unknown_interaction` observation and offers no answer plan, and an unverified composer selects delivery fallback for ordinary input. An answer plan writes only after its request rule, choices and gates match the fresh observation.

Native evidence is the provenance of a profile interval. Catalog provenance validity (§ Bundled profiles and native screen fixtures) requires every cited fixture's backend to match its profile and its capture version to lie at or above the profile's `min_version` and, when the backend has a next boundary, below it. Each subsequent native release within the interval reuses the declaration without another exact-version capture. A new profile is added only when an observed native change requires different rules or plans, with `min_version` set to the first native version requiring the change.

##### Bundled profiles and native screen fixtures

The binary embeds the bundled catalog; the native screen fixtures recorded in its fixture registry are its provenance records. A shipped bundled catalog satisfies every provenance rule below. Each profile's capabilities rest on accepted native captures taken on a native version within its interval. A bundled profile entry has required `backend`, `min_version`, `rules` and `capabilities`, and no other key; documentation and messages name a profile by its backend and boundary, as in "the OpenCode profile from `2.1.4`". Each fixture entry keeps its own `native_version` as the actual capture version. Each existing profile's `min_version` is the capture version of its earliest native fixtures; fixtures captured on later versions inside the interval add declarations to the same entry, and adding operation plans to an existing entry changes its contents without introducing a new boundary.

The fixture registry uses `assets/profiles/native/<backend>.json` (§6.3 Asset installation) for each backend with catalogued native screens. Every catalog pattern this section quotes uses § Pattern syntax. A fixture entry has required `id,kind,backend,native_version,rows,cols,source_ref,sha256` and optional `composer_evidence`; `source_ref` selects `native/<backend>.json`, and `sha256` is the SHA-256 of the canonical JSON (§6.4 JSON encoding) of that entry's screen object. Provenance validation covers every catalog fixture entry, including screens that no profile capability cites. Run-time catalog loading ignores the `fixtures` registry contents and every `fixture_ids` value; only provenance validation reads them. Its rules are unique ids, lowercase 32-byte digests, non-empty citations, citations inside the profile's backend interval and continuations citing native fixtures.

`composer_evidence` is a nonempty array of explicit native measurement records. Each record requires `operation` (`prompt` or `exec`), `multiplexer` (`tmux` or `herdr`) and one `kind` from the table below. Provenance validation rejects unknown fields or values and duplicate `(operation,multiplexer,kind)` records on one fixture. The native trial and its captured state establish each record's meaning; fixture naming alone supplies no claim.

| Kind | Additional required fields | Evidence |
|---|---|---|
| `framing` | `literal_framing` | The operation's selected measured framing |
| `plain_limit` | `plain_literal_limit`, including explicit `null` | The measured plain boundary, conservative tested cap or maximum plain success |
| `post_submit` | None | A verified empty composer after submission |
| `unsubmitted_literal` | None | A retained literal distinguishable from an empty composer |

Each executable operation's cited fixtures must cover every kind on both multiplexers. Framing and limit records must equal that operation's declared `literal_framing` and `plain_literal_limit`. Generic valid citations supplement this evidence; they supply no missing category. A fixture can carry several records when its original native measurement establishes each claim.

Every bundled executable composer plan declares measured `literal_framing` and `plain_literal_limit` (§ Executable native plans) and empty-composer observation. Its fixture citations establish framing, plain boundaries, post-submit verification and retained unsubmitted literals for each operation independently. New composer fixtures use `<backend>-<version>-composer-<op>-<case>-<mux>-<cols>-<n>`; existing fixture IDs retain their patterns. Captures taken inside an existing profile interval extend its declarations without changing `min_version`.

The selected measured plans use these declarations for each profile interval. The native version identifies the evidence capture, independently of the profile's minimum:

| Profile | Native evidence version | Prompt framing | Exec framing | Prompt plain limit | Exec plain limit |
|---|---|---|---|---|---|
| `claude` from `2.1.302` | `2.1.308`, `2.1.319` | `plain` | `plain` | `null` | `null` |
| `claude` from `2.1.322` | `2.1.322` | `plain` | `plain` | `null` | `null` |
| `codex` from `0.163.0` | `0.163.0`, `0.166.0` | `bracketed_paste` | `bracketed_paste` | `{"unit":"bytes","max":1024}` | `{"unit":"bytes","max":1000}` |
| `codex` from `0.167.0` | `0.167.0` | `bracketed_paste` | `bracketed_paste` | `{"unit":"bytes","max":1024}` | `{"unit":"bytes","max":1000}` |
| `opencode` from `2.1.4` | `2.1.4`, `2.2.5` | `plain` | `plain` | `null` | `null` |
| `opencode` from `2.2.7` | `2.2.7` | `plain` | `plain` | `null` | `null` |
| `pi` from `1.5.0` | `1.5.0` | `plain` | `plain` | `null` | `null` |
| `hermes` from `2026.9.24` | `2026.9.24` | `bracketed_paste` | `plain` | `{"unit":"scalars","max":1024}` | `{"unit":"scalars","max":1024}` |

The `claude` 2.1.322, `codex` 0.167.0 and `opencode` 2.2.7 profiles carry their predecessor's rules, capabilities and plans with the strict idle rule and the suggestion rule of §6.7 Native suggestions, and cite only evidence captured on those versions. Their predecessors keep their original idle rule and no suggestion rule. The `codex` 0.163.0 profile's composer evidence is the 0.166.0 capture series, because that profile's earlier composer evidence was captured on 0.167.0, which now lies in the next interval.

Codex's plain prompt boundary is 1024 submitting bytes and 1025 retained bytes on both multiplexers. Its faithful plain native-exec boundary is 1000 bytes: 1001 bytes enters chat instead of executing the command, while the selected bracketed exec remains faithful through the maximum preview. Hermes's 1024-scalar limit is a conservative tested cap: its Enter guard depends on the 50 ms since the last processed text change, so larger inputs can submit and size alone establishes no adjacent deterministic boundary. Maximum plain Japanese input remains retained or unverified, which rules out a null limit. Bracketed Hermes prompts submit, while large bracketed exec input collapses to a paste placeholder that is dispatched instead of the original command; its exec plan therefore uses plain input within the tested cap. Native command output or independently checked output artifacts establish original-literal execution separately from an empty composer.

| Composer evidence | Required observation on each multiplexer and operation |
|---|---|
| Mode 2004 | Child state at an empty composer, idle and working |
| Plain limit | A size-gated backend's adjacent submitting/retained boundary; a timing-gated backend's selected tested cap with processing-guard and actual failure evidence; maximum-preview success for a null limit |
| Bracketed submission | Below and above the plain limit and at 4000 Japanese scalars, with literal/pre-submit and post-submit screens after the measured settle/key; failed or placeholder-substituting exec framing remains explicit evidence for selecting plain input |
| Post-submit | Verified empty composer within 2 s, idle and working |
| Unsubmitted literal | Ineffective-key or over-limit plain input remains distinguishable from a verified empty composer |
| Long literal | Chunked 4000-scalar Japanese body (4085 rendered scalars, 12085 bytes in the maximum measured preview), with literal-to-submit and submit-to-empty timings within 5 s; native exec output independently proves literal fidelity |

Before each pinned native process's capture series, verify its evidence version from actual `--version` output in a pane of the capturing multiplexer server or from that process's original native startup output recorded before the first frame. Record the absolute executable path, process/series identity and raw-output offsets that bind every frame to this version evidence. An echoed command supplies launch intent; the program's output establishes its version. Evidence supplies each operation's measured framing/limit values and fixture citations. Hermes additionally records 2057-character and 8192-scalar composers and clear-sequence candidates for §6.7 Hermes work settlement.

Every decision answer plan selects a single-request choice, and every choice answer plan selects a declared dismissal choice of a question; other broadened, persistent or session-wide choices have no answer plan, and a focus on a choice without a choice entry satisfies no gate. Native lines a capture shows after an answer or a timeout are decision evidence for the Director's continuation check, not bundled rules. Codex 0.163.0 shows no startup notice or opt-in dialog for `approve-for-me`, so its profile declares no startup rule.

The Claude profile from `2.1.302` applies to Claude Code under every Claude launch policy:

| Capability | Status | Declaration |
|---|---|---|
| Detection | `tested` | Rule `claude-bash-permission`: the native `default` Bash permission form (`Bash command` heading, approval text, proceed question, focused choice and the `Yes`/`No` single-request choices) on the primary or alternate screen, with a plain or boxed operation, unwrapped or wrapped, within 24 rows from its heading to its `Esc to cancel · Tab to amend` line. Request rules `claude-bash-permission-auto-tip`, `claude-bash-permission-no-tip`, `claude-auto-ask-permission`, `claude-read-outside-permission`, `claude-read-file-permission`, `claude-edit-file-permission` and `claude-create-file-permission`, and question rules `claude-auto-default-nudge`, `claude-question-pending-notice`, `claude-auto-startup` and `claude-auto-env-setup-wizard`: described below |
| Allow answer | `tested` | One stage per request rule selecting its single-request allow choice as `allow-once` (answer table below) |
| Deny answer | `tested` | One stage per request rule selecting its single-request deny choice as `deny-once` (answer table below) |
| Question answers | `tested` | Choice `decline` of `claude-auto-default-nudge`: stage `0e` (Ctrl+N) while `accept` is focused, then `0d` (Enter) while `decline` is focused. Choices `not-now` (byte `32`) and `dont-show-again` (byte `33`) of `claude-auto-startup`, one ungated stage each |
| Empty-composer observation | `tested` | The idle prompt line with the terminal cursor immediately after the prompt prefix; a verified non-interaction boundary |
| Prompt submission | `tested` | Composer plan `prompt` (§ Executable native plans) |
| Native-exec submission | `tested` | Composer plan `exec` (§ Executable native plans) |

Every permission answer and both Teach-auto-mode choices select their choice by its number, independent of focus; the Make-auto-mode-default dialog has no numbered choices, so its `decline` plan gates each stage on the marked focus. The empty-composer observation reads the composer as empty when the cursor sits at the end of the prompt prefix, including a native ghost hint displayed after that cursor: typing replaces the hint and moves the cursor after the typed text. Typed text with its native cursor after the literal is nonempty; a draft whose cursor was moved back to the prefix reads as empty.

| Request rule | Allow byte (digit, label) | Deny byte (digit, label) | Native outcome in the captures |
|---|---|---|---|
| `claude-bash-permission` | `31` (`1`, `Yes`) | `34` (`4`, `No`) | — |
| `claude-bash-permission-auto-tip` | `31` (`1`, `Yes`) | `34` (`4`, `No`) | Allow ran the command; deny printed `Interrupted · What should Claude do instead?` |
| `claude-auto-ask-permission` | `31` (`1`, `Yes`) | `32` (`2`, `No`) | Allow ran the command; deny printed `Interrupted · What should Claude do instead?` |
| `claude-bash-permission-no-tip` | `31` (`1`, `Yes`) | `33` (`3`, `No`) | Allow ran the command and deny refused it, with no persistent change |
| `claude-read-outside-permission` | `34` (`4`, `Yes, but ask again next time`) | `33` (`3`, `No, and ask again next time`) | Allow performed the read; deny made the Read tool return `The user did not allow this read outside the working directories`. Neither changed a persistent setting |
| `claude-read-file-permission` | `31` (`1`, `Yes`) | `33` (`3`, `No`) | Allow read the file; deny refused with `Interrupted · What should Claude do instead?` |
| `claude-edit-file-permission` | `31` (`1`, `Yes`) | `33` (`3`, `No`) | Allow applied the edit; deny refused with `User rejected update to <path>` |
| `claude-create-file-permission` | `31` (`1`, `Yes`) | `33` (`3`, `No`) | Allow created the file; deny created no file |

On the three file forms the digit alone closed a fresh form and no answer changed a persistent setting. Their plans are `claude-read-file-allow-digit`, `claude-read-file-deny-digit` and the matching `claude-edit-file-*` and `claude-create-file-*` plans; the no-tip plans are `claude-bash-no-tip-allow-digit` and `claude-bash-no-tip-deny-digit`.

Detection rests on native captures of the form unwrapped at 120 columns, wrapped plain at 52 columns, boxed at 89 and 120 columns, on the primary screen at 120 columns and beside the diff sidebar at 166 and 144 columns; both answers rest on the 120-column and the 52-column plain captures, and allow also on the 166-column sidebar capture. A narrow pane or a long command makes Claude wrap the form: the tip, the description and the choice labels continue on further rows, and the operation between the dashed rules spans several rows. The operation rows take one of two native shapes:

| Shape | Row layout | Where Claude breaks the operation | Native evidence |
|---|---|---|---|
| Plain | One-space margin, then the operation text | At the row's last usable column, one column before the pane's right edge, even inside a token | Captures at 52 columns |
| Boxed | One-space margin, then the `│ ` gutter and the operation text | At a space | Captures at 89 columns (two rows) and 120 columns (one row) |

The shape follows the operation's length rather than the pane width. In the captures, operations of 57 and 61 characters render plain, and an 86-character operation renders boxed, on one row at 120 columns and over two rows at 89 columns.

Claude positions every row explicitly, so the terminal records no soft wrap, and a displayed row break cannot be told from a line break in the command itself. The rule therefore reports the operation as its displayed rows: it removes exactly the one-space margin from each operation row, keeps the boxed `│ ` gutter (also on a single boxed row), and joins the rows with line breaks. Every Claude request and question rule has priority 100 and matches on the primary or alternate screen; every request rule also requires the cursor inside, spans at most 24 rows and, except the Edit file and Create file rules, extracts its operation through this shared operation matcher with its one-space `row_margin`.

The request identity follows the displayed rows. A resize that rewraps a multi-row operation, or moves it between one row and several, replaces its episode; a resize that leaves every operation row unchanged keeps it.

Claude Code 2.1.302 in `auto` shows a native permission form only through an explicit `permissions.ask` rule. That form differs from the `default` form: it carries a `│ Ask rule … overrides auto mode for this command.` row and a `/permissions to let auto mode decide` row, has no `This command requires approval` line, and offers `2. No` as its deny choice. Rule `claude-auto-ask-permission` spans from `^ Bash command$` to `^ Esc to cancel · Tab to amend$` and requires these line matchers:

- `^ │ Ask rule [^\t-\r \x{85}\x{a0}\x{1680}\x{2000}-\x{200a}\x{2028}\x{2029}\x{202f}\x{205f}\x{3000}]`
- `^ /permissions to let auto mode decide$`
- `^ Do you want to proceed\?[\t-\r \x{85}\x{a0}\x{1680}\x{2000}-\x{200a}\x{2028}\x{2029}\x{202f}\x{205f}\x{3000}]*$`
- `^ ❯ [1-9]\. ` for the focus, `^ [❯ ] 1\. (?P<label>Yes)$` and `^ [❯ ] 2\. (?P<label>No)$` for the choices

Under a non-automatic mode such as an explicit `default`, Claude Code 2.1.302 can show another Bash form for a workspace file write. It replaces the `This command requires approval` line with a `Tip: auto mode handles these prompts for you — choose "switch to auto mode" below` row and a description row, and offers four choices. Rule `claude-bash-permission-auto-tip` spans from `^ Bash command$` to `^ Esc to cancel · Tab to amend$` and requires these line matchers:

- `^ Do you want to proceed\?$` and `^ ❯ [1-9]\. ` for the focus
- `^ [❯ ] 1\. (?P<label>Yes)$` and `^ [❯ ] 4\. (?P<label>No)$` for the single-request choices
- `^ [❯ ] 2\. Yes, and always allow access to [^\t-\r \x{85}\x{a0}\x{1680}\x{2000}-\x{200a}\x{2028}\x{2029}\x{202f}\x{205f}\x{3000}].* from this project$` and `^ [❯ ] 3\. Yes, and switch to auto mode · auto mode handles these prompts for you$` for the broadened choices

The form was captured at 120 columns only, so a wrapped or boxed rendering of it is not detected.

In `auto`, Claude Code can show `Teach auto mode about your environment?` after a turn. It appears when the native configuration's `.claude.json` records `numStartups` of at least 5 and `autoModeEnvSetup.denials` of at least 5, no auto-mode environment is configured, `autoModeEnvSetup.dismissed` is unset, `autoModeEnvSetup.dismissedAt` is unset or older than 7 days, and the auto-mode entry warning has been seen (`hasSeenAutoModeEntryWarning` `true`, or a settings source sets `skipAutoPermissionPrompt`). The offer shows three choices above the footer `Enter to confirm · Esc to cancel`; a single digit selects its choice without Enter, and a choice key within 500 ms of the offer appearing is ignored:

| Choice | Effect |
|---|---|
| `1. Yes` | Opens a wizard with `How you use Claude here`, `Also scan shell history`, `Also scan your other repos` and `Continue` that scans repositories and shell history and writes settings |
| `2. Not now` | Sets `autoModeEnvSetup.dismissedAt` (a 7-day snooze); Escape resolves as `Not now` |
| `3. Don't show again` | Sets `autoModeEnvSetup.dismissed` `true` |

Two question rules (cursor inside, at most 14 rows) claim the two stages as separate episodes:

| Rule | Start | End | Matchers and choices |
|---|---|---|---|
| `claude-auto-startup` (offer) | `^  Teach auto mode about your environment\?[\t-\r \x{85}\x{a0}\x{1680}\x{2000}-\x{200a}\x{2028}\x{2029}\x{202f}\x{205f}\x{3000}]*$` | `^  Enter to confirm · Esc to cancel[\t-\r \x{85}\x{a0}\x{1680}\x{2000}-\x{200a}\x{2028}\x{2029}\x{202f}\x{205f}\x{3000}]*$` | heading, footer, focus `^  ❯ [1-9]\. `, `^  [❯ ] 1\. Yes[\t-\r \x{85}\x{a0}\x{1680}\x{2000}-\x{200a}\x{2028}\x{2029}\x{202f}\x{205f}\x{3000}]*$`, and the choices `not-now` (`^  [❯ ] 2\. (?P<label>Not now)[\t-\r \x{85}\x{a0}\x{1680}\x{2000}-\x{200a}\x{2028}\x{2029}\x{202f}\x{205f}\x{3000}]*$`) and `dont-show-again` (`^  [❯ ] 3\. (?P<label>Don't show again)[\t-\r \x{85}\x{a0}\x{1680}\x{2000}-\x{200a}\x{2028}\x{2029}\x{202f}\x{205f}\x{3000}]*$`), both `persistent` |
| `claude-auto-env-setup-wizard` | `^   Teach auto mode about your environment\?[\t-\r \x{85}\x{a0}\x{1680}\x{2000}-\x{200a}\x{2028}\x{2029}\x{202f}\x{205f}\x{3000}]*$` | `^   ←/→ to change · Enter to continue · Esc to cancel[\t-\r \x{85}\x{a0}\x{1680}\x{2000}-\x{200a}\x{2028}\x{2029}\x{202f}\x{205f}\x{3000}]*$` | heading, footer, focus `^   ❯ [^\t-\r \x{85}\x{a0}\x{1680}\x{2000}-\x{200a}\x{2028}\x{2029}\x{202f}\x{205f}\x{3000}]`; no choices |

Neither rule declares an operation. The offer and the wizard differ in their two- and three-space margins, so the wizard, which opens only after a manual `1. Yes`, starts its own episode. The offer's answer plans `claude-auto-env-setup-not-now-digit` (byte `32`) and `claude-auto-env-setup-dont-show-again-digit` (byte `33`) are one ungated stage each; no plan selects `1. Yes`, and the wizard has no answer plan. In the 2.1.312 captures each digit closed the offer and changed only its documented `autoModeEnvSetup` key.

Claude Code 2.1.312 can show `Make auto mode your default permission mode?` when onboarding is complete, `.claude.json` lacks `hasSeenAutoDefaultNudge`, the server-side gate `tengu_maple_pier` is on, the user settings pin a non-`auto` `permissions.defaultMode` and no project, local, flag or policy settings set `defaultMode`; a `--permission-mode` launch flag does not suppress it. Under a CAFleet launch it appears after the first turn and replaces the composer, its borders and the footer:

```text
────────────────────────────── (full-width rule)
 Make auto mode your default permission mode?

   Auto mode lets Claude handle permission prompts automatically. Claude checks each tool call for risky actions and
   prompt injection before executing, runs the ones it assesses as lower-risk, and blocks the rest.

   ❯ Yes, set auto mode as my default permission mode
     No, keep manual mode
```

The second label names the user-settings default (`No, keep manual mode` for `default`, `No, keep accept edits` for `acceptEdits`). Focus starts on `Yes` and is marked only by the `❯` glyph and a foreground colour; Ctrl+N and Ctrl+P (also the arrow keys) move and wrap it, Enter confirms the focused choice, typed text is discarded and Escape declines. `Yes` writes `permissions.defaultMode` `auto` into the user settings and switches the session; `No` and Escape set `hasSeenAutoDefaultNudge` `true`. The hidden terminal cursor sits at column 3 of the focused choice row. Question rule `claude-auto-default-nudge` (cursor inside, at most 10 rows) spans from `^ Make auto mode your default permission mode\?[\t-\r \x{85}\x{a0}\x{1680}\x{2000}-\x{200a}\x{2028}\x{2029}\x{202f}\x{205f}\x{3000}]*$` to `^   (?:❯ |  )No, keep [^\t-\r \x{85}\x{a0}\x{1680}\x{2000}-\x{200a}\x{2028}\x{2029}\x{202f}\x{205f}\x{3000}].*$` and declares the interaction frame guard rejecting `^❯\x{a0}`, so a quoted dialog above a live composer never matches:

- `heading` `^ Make auto mode your default permission mode\?[\t-\r \x{85}\x{a0}\x{1680}\x{2000}-\x{200a}\x{2028}\x{2029}\x{202f}\x{205f}\x{3000}]*$`
- `focus` `^   ❯ [^\t-\r \x{85}\x{a0}\x{1680}\x{2000}-\x{200a}\x{2028}\x{2029}\x{202f}\x{205f}\x{3000}]` with `focus_style` `marker` (§ Focused-choice resolution)
- choice `accept` (`^   (?:❯ |  )(?P<label>Yes, set auto mode as my default permission mode)[\t-\r \x{85}\x{a0}\x{1680}\x{2000}-\x{200a}\x{2028}\x{2029}\x{202f}\x{205f}\x{3000}]*$`) and choice `decline` (`^   (?:❯ |  )(?P<label>No, keep [^\t-\r \x{85}\x{a0}\x{1680}\x{2000}-\x{200a}\x{2028}\x{2029}\x{202f}\x{205f}\x{3000}].*?)[\t-\r \x{85}\x{a0}\x{1680}\x{2000}-\x{200a}\x{2028}\x{2029}\x{202f}\x{205f}\x{3000}]*$`), both `persistent`

The `decline` label carries the kept default's name and is part of the fingerprint; the explanatory paragraph has no matcher, and the rule declares no operation. Answer plan `claude-auto-default-decline` for choice `decline` has two focus-gated stages: `0e` (Ctrl+N) while `accept` is focused moves the focus to `No, keep …`, then `0d` (Enter) while `decline` is focused closes the dialog as declined. Ctrl+N wraps, so a focus already on `decline` fails the first gate before any byte. `accept` is declared only so a gate can name it and has no plan. In the captures the plan closed the dialog and changed only `hasSeenAutoDefaultNudge`.

When a draft reaches the composer before the dialog mounts, Claude hides the dialog and shows the notice row `Claude has a question for you — it shows once you send or clear what you're typing.` at column 0 above the composer. Question rule `claude-question-pending-notice` (cursor any, one row) matches that row, `^Claude has a question for you — it shows once you send or clear what you're typing\.[\t-\r \x{85}\x{a0}\x{1680}\x{2000}-\x{200a}\x{2028}\x{2029}\x{202f}\x{205f}\x{3000}]*$`, and declares the frame guard (§ Interaction frame guard) that requires `^❯\x{a0}` (the live composer prefix) and rejects `^  Showing detailed transcript · ctrl\+o to toggle` (the transcript viewer's footer); model text quoting the words renders with the `● ` or two-space transcript margin and does not match. It declares no choices, operation or answer plan. As a candidate interaction the notice stops a profile submission after its literal stage and before its submit key (§ Measured submission plans and settle intervals) and queues later input. Two native outcomes remove it and close its episode as `not_visible`:

| User action on the native surface | Native outcome in the 2.1.312 captures |
|---|---|
| Clear the draft | The notice disappears and the dialog is drawn in its place at once, opening its own `question` episode |
| Send the draft | For a draft typed during a turn, the first Enter submits it and the notice disappears; a draft typed at startup before any turn needs a second Enter. The dialog does not reappear in that process and `hasSeenAutoDefaultNudge` stays unset, so it returns at the next launch |

Claude Code 2.1.309 asks before a file tool reads outside the working directories, also in `auto`. The dialog shows the read operation between dashed rules, an explanatory paragraph, the question and four choices, and opens focused on choice 1. Once the native configuration's `.claude.json` holds `hasSeenAutoModeOutsideReadPrompt` `true`, later reads outside the working directories in `auto` proceed without asking; when Claude writes the flag is not established. Rule `claude-read-outside-permission` spans from `^ Read outside the working directories[\t-\r \x{85}\x{a0}\x{1680}\x{2000}-\x{200a}\x{2028}\x{2029}\x{202f}\x{205f}\x{3000}]*$` to `^ Esc to cancel · Tab to amend[\t-\r \x{85}\x{a0}\x{1680}\x{2000}-\x{200a}\x{2028}\x{2029}\x{202f}\x{205f}\x{3000}]*$` and requires `^ Read outside the working directories[\t-\r \x{85}\x{a0}\x{1680}\x{2000}-\x{200a}\x{2028}\x{2029}\x{202f}\x{205f}\x{3000}]*$` for the heading, `^ Allow this read outside the working directories\?[\t-\r \x{85}\x{a0}\x{1680}\x{2000}-\x{200a}\x{2028}\x{2029}\x{202f}\x{205f}\x{3000}]*$` for the question, `^ ❯ [1-9]\. ` for the focus and one line matcher per choice:

| Choice matcher | Scope |
|---|---|
| `^ [❯ ] 1\. Yes, and keep allowing any reads outside the working directories[\t-\r \x{85}\x{a0}\x{1680}\x{2000}-\x{200a}\x{2028}\x{2029}\x{202f}\x{205f}\x{3000}]*$` | Changes a persistent setting |
| `^ [❯ ] 2\. No, and block reads outside the working directories from now on[\t-\r \x{85}\x{a0}\x{1680}\x{2000}-\x{200a}\x{2028}\x{2029}\x{202f}\x{205f}\x{3000}]*$` | Changes a persistent setting |
| `^ [❯ ] 3\. (?P<label>No, and ask again next time)[\t-\r \x{85}\x{a0}\x{1680}\x{2000}-\x{200a}\x{2028}\x{2029}\x{202f}\x{205f}\x{3000}]*$` | This read only, as `deny-once` |
| `^ [❯ ] 4\. (?P<label>Yes, but ask again next time)[\t-\r \x{85}\x{a0}\x{1680}\x{2000}-\x{200a}\x{2028}\x{2029}\x{202f}\x{205f}\x{3000}]*$` | This read only, as `allow-once` |

The operation is the displayed `Read(<path>)` rows. A long path continues on a further row with the one-space margin. The explanatory paragraph has no matcher. The rule rests on 2.1.309 captures at 120 columns; an earlier version that never renders the dialog never matches it.

| Limit | Behavior |
|---|---|
| Panes narrower than 120 columns | Only 120-column renderings were captured; a narrower rendering whose rows differ from the matchers is an `unknown` frame, and the dialog is answered on the native surface |
| `Tab to amend` | CAFleet offers no amend action |
| Panes too short to show the heading row | The region has no start row, so the frame is `unknown` until a resize redraws the dialog in full |
| Other file tools | In the 2.1.309 captures, Edit first read its target and showed this same dialog, Write ran without a dialog under the auto mode classifier, and the session offered no Glob or Grep tool. An outside-working-directory dialog with other literals is an `unknown` frame with no event |

In `default` mode, Claude Code 2.1.312 asks before a file tool touches a path outside the working directories. Each form has a heading, dashed `╌` rules around the operation, a question, three numbered choices and the footer ` Esc to cancel · Tab to amend`; the cursor sits on the focused choice row. Three request rules, ending at `^ Esc to cancel · Tab to amend[\t-\r \x{85}\x{a0}\x{1680}\x{2000}-\x{200a}\x{2028}\x{2029}\x{202f}\x{205f}\x{3000}]*$`, claim them. They share the focus matcher `^ ❯ [1-9]\. `, the single-request choices `allow-once` (`^ [❯ ] 1\. (?P<label>Yes)[\t-\r \x{85}\x{a0}\x{1680}\x{2000}-\x{200a}\x{2028}\x{2029}\x{202f}\x{205f}\x{3000}]*$`) and `deny-once` (`^ [❯ ] 3\. (?P<label>No)[\t-\r \x{85}\x{a0}\x{1680}\x{2000}-\x{200a}\x{2028}\x{2029}\x{202f}\x{205f}\x{3000}]*$`), and a choice-2 matcher anchored on the label's start, because choice 2 can wrap onto a six-space continuation row:

| Rule | Heading | Operation | Question | Choice 2 (no plan) |
|---|---|---|---|---|
| `claude-read-file-permission` | ` Read file` | The `Read(<absolute path>)` rows, through the shared operation matcher | ` Do you want to proceed?` | `Yes, allow reading from <dir> during this session` |
| `claude-edit-file-permission` | ` Edit file` | The path row below the heading, relative to the working directory | ` Do you want to make this edit to <file>?` | `Yes, and switch to accept edits …` (session) |
| `claude-create-file-permission` | ` Create file` | The path row below the heading, relative to the working directory | ` Do you want to create <file>?` | `Yes, and switch to accept edits …` (session) |

The Edit form shows the diff and the Create form the new content between the dashed rules; neither is extracted, and the Director reviews it from the capture excerpt. Edit first shows the Read form for its target unless reads of that path are allowed.

When auto mode cannot be offered (`permissions.disableAutoMode` `disable`, an unavailable plan or model, fast mode, an ask rule or hook, or `plan` mode), Claude Code 2.1.312 drops the Tip row and the `switch to auto mode` choice from its Bash form and shows three choices. Rule `claude-bash-permission-no-tip` (from `^ Bash command[\t-\r \x{85}\x{a0}\x{1680}\x{2000}-\x{200a}\x{2028}\x{2029}\x{202f}\x{205f}\x{3000}]*$` to `^ Esc to cancel · Tab to amend[\t-\r \x{85}\x{a0}\x{1680}\x{2000}-\x{200a}\x{2028}\x{2029}\x{202f}\x{205f}\x{3000}]*$`) claims both of its variants:

| Variant | Shown for | Rows after the description row |
|---|---|---|
| Approval | A command inside the working directories | The operation with the one-space margin, ` This command requires approval`, a blank row, the question and choice 2 ` Yes, and don’t ask again for: <command prefix> *` (U+2019 apostrophe) |
| Access | A command touching a path outside the working directories | The operation in the boxed `│ ` gutter, then the question with no blank row, and choice 2 ` Yes, and always allow access to <dir> from this project` |

Besides the shared operation matcher (which keeps the boxed gutter, as for `claude-bash-permission`), the rule requires `^ Do you want to proceed\?[\t-\r \x{85}\x{a0}\x{1680}\x{2000}-\x{200a}\x{2028}\x{2029}\x{202f}\x{205f}\x{3000}]*$`, the focus `^ ❯ [1-9]\. `, `allow-once` `^ [❯ ] 1\. (?P<label>Yes)[\t-\r \x{85}\x{a0}\x{1680}\x{2000}-\x{200a}\x{2028}\x{2029}\x{202f}\x{205f}\x{3000}]*$`, the choice-2 matcher `remember` (either label, no choice entry) and `deny-once` `^ [❯ ] 3\. (?P<label>No)[\t-\r \x{85}\x{a0}\x{1680}\x{2000}-\x{200a}\x{2028}\x{2029}\x{202f}\x{205f}\x{3000}]*$`. The approval line has no matcher, and the model-authored description row has none either. `claude-bash-permission` and `claude-bash-permission-auto-tip` require `4. No`, and `claude-auto-ask-permission` requires its `│ Ask rule` row and `2. No`, so the four Bash rules match disjoint forms.

When Claude Code 2.1.312 issues several tool calls in one turn, it asks for them one form at a time and right-aligns a request counter `N of M` at the end of each form's heading row (` Read file` … `1 of 2`). The `claude-read-file-permission`, `claude-edit-file-permission` and `claude-create-file-permission` rules and every rule anchored on the ` Bash command` heading accept an optional `(?:[\t-\r \x{85}\x{a0}\x{1680}\x{2000}-\x{200a}\x{2028}\x{2029}\x{202f}\x{205f}\x{3000}]+[0-9]+ of [0-9]+)?` before the end of their region start pattern and their heading matcher; the Edit and Create rules also accept it on the heading line of their operation matcher. The counter is not evidence, not part of the operation and not part of the request identity, so a batched form classifies with the same rule and evidence as its single form. The answer plans stay the single-stage digits:

| Answer on a batched Read file form | Native outcome in the 2.1.312 captures |
|---|---|
| Allow `31` on `1 of 2` | The answer settles only the current form: it redraws as `2 of 2` with the second call's `Read(<path>)` operation, a new request |
| Deny `33` on `1 of 2`, or on `2 of 2` after that allow | The turn is interrupted: the transcript shows `Read 2 files` and `Interrupted · What should Claude do instead?`, no form remains and no file content is reported |

A deny answer on any batched form therefore interrupts the turn and refuses every call that has not completed. A read leaves no artifact, so whether an allowed call runs before the later forms are answered is not established. No answer changed a persistent setting.

The Claude process that performs first-launch onboarding (theme, security notes and folder trust on the native surface) renders on the primary screen for its whole lifetime, with the composer, forms and dialogs in the same rows as on the alternate screen; in that process `Make auto mode your default permission mode?` appears after the first turn. Every Claude rule therefore matches on either screen. Earlier transcript rows stay visible above the live region on the primary screen; the cursor rules, the frame guards and the transcript prompt prefix `❯` + U+0020 (the live composer uses U+00A0) keep them from matching, and in the captures neither a closed form nor a resize left a matching remnant.

Claude Code opens a diff sidebar beside the transcript at 144 columns or more (110 or more with `.claude.json` `diffSidebarOpen` `true`) in a git repository after the working-tree change set changes, and `diffSidebarOpen` `false` disables it. The sidebar draws a dim `│` separator and panel text on every row above the composer border, so a form's rows continue past the separator; the composer border, composer row and footer stay full width. Every Claude rule except `claude-idle-composer` and `claude-auto-mode-indicator` declares the separator glyph and matches against the clipped rows (§ Diff sidebar clipping). In the captures the separator sat at column 92 of a 166-column pane and column 80 of a 144-column pane; in that left area, narrower than 120 columns, the `claude-bash-permission` form wraps its Tip row (at 80 columns) and its choice 2 (at 80 and 92 columns), which its matchers leave unmatched. Allow `1` beside the sidebar ran the command.

| Limit | Behavior |
|---|---|
| `Yes, set auto mode as my default permission mode`, `1. Yes` and the wizard | Answered by the user on the native surface |
| A user default other than `default` or `acceptEdits` | Claimed through the `No, keep …` pattern; only those two labels were captured |
| `tengu_maple_pier` off | `Make auto mode your default permission mode?` never appears |
| Bash title variants `Run command`, `Bash command (unsandboxed)`, `Bash command (runs on <host>)` | Not rendered in the captures; an `unknown` frame |
| File forms in `acceptEdits` or `plan`, a wrapped path or question, panes narrower than 120 columns | Uncaptured; a differing rendering is an `unknown` frame |
| A Bash form with a request counter | Unobserved: two parallel Bash calls in one turn rendered the form without a counter. The Bash rules accept the counter as the Read file rule does; batched Edit file and Create file forms were not captured either |
| Sidebar widths other than 144, 166 and 180 columns | The separator is found per frame, so other widths clip the same way. Live acceptance observed 166 and 180 columns (separator at column 92 and 99 on tmux), and 144 columns rests on its fixture only |
| The pending-question notice, the `1. Yes` wizard, `Make auto mode your default permission mode?` on an onboarding process's primary screen, and its `No, keep accept edits` label | Not produced in live acceptance; their rules rest on the native fixtures |

The Codex profile from `0.163.0` applies to Codex CLI under every Codex launch policy; the request and answer rules come from captures under `--ask-for-approval on-request --sandbox workspace-write`:

| Capability | Status | Declaration |
|---|---|---|
| Detection | `tested` | The native command-approval form in the alternate screen, from its `Would you like to run the following command?` heading to its `Press enter to confirm or esc to cancel` line within 24 rows: the `Environment:` and `Reason:` lines, the single-line `$ ` command, the focused `›` choice and the `1. Yes, proceed (y)` / `3. No, and tell Codex what to do differently (esc)` single-request choices, with the cursor inside the form |
| Allow answer | `tested` | One stage, byte `79` (`y`), selecting `Yes, proceed` as `allow-once` |
| Deny answer | `tested` | One stage, byte `33` (digit `3`), selecting `No, and tell Codex what to do differently` as `deny-once` |
| Empty-composer observation | `tested` | The idle `› ` composer line with the terminal cursor immediately after the prefix, its placeholder text shown; a verified non-interaction boundary |
| Prompt submission | `tested` | Composer plan `prompt` (§ Executable native plans) |
| Native-exec submission | `tested` | Composer plan `exec` (§ Executable native plans) |

The rules anchor on the live form's heading and closing lines rather than on absolute rows, and the reason line is model-authored text that the rule requires but excludes from the request identity. The deny answer uses the choice's digit because that choice displays only `esc` as its shortcut, and CAFleet never sends a lone Escape to a held form. Choice 2 approves a command prefix persistently. The native captures showed the decision on the native surface: after allow, Codex printed `✔ You approved codex to run <command> this time` and ran the command; after deny, it printed `✗ You canceled the request to run <command>` and `■ Conversation interrupted`, and ran nothing. Under an `approvals_reviewer` setting (§ Launch policy and capability registry), detection observes no request. The detection covers a single-line command; a command that wraps in a narrow pane is a recorded layout limit (§6.7 Role requirements), and ordinary programmatic input is not suppressed and may reach the form through delivery fallback.

Codex 0.163.0 in `approve-for-me` routes every escalation to its automatic reviewer and shows no native permission form, so the profile adds no request rule.

The captures showed an allowed escalation only as an ordinary `• Ran …` transcript line, and the reviewer at work as `• Reviewing approval request (Ns • esc to interrupt)`; both are verified non-interaction frames with the composer visible, so neither needs a rule.

The OpenCode profile from `2.1.4` applies to OpenCode `mini` under either OpenCode launch policy; only `cafleet-ask` shows permission forms:

| Capability | Status | Declaration |
|---|---|---|
| Detection | `tested` | Rules `opencode-shell-permission` (wide and split layouts, ending at the `⇆ select  enter confirm  esc reject` control line) and `opencode-shell-permission-compact` (compact layout, ending at `pgup/pgdn scroll`): the shell permission form on the primary screen from its `△ Permission required` heading, with the `$ ` operation over one or more displayed rows shown in full, the focused choice and the `Allow once` and `Reject` choices |
| Allow answer | `tested` | One gated stage: byte `0d` (Enter) while `Allow once` is focused, as `allow-once` |
| Deny answer | `tested` | Three gated stages (stage table below), as `deny-once` |
| Empty-composer observation | `tested` | The idle `┃` composer line with the terminal cursor immediately after the prefix; a verified non-interaction boundary |
| Prompt submission | `tested` | Composer plan `prompt` (§ Executable native plans) |
| Native-exec submission | `tested` | Composer plan `exec` (§ Executable native plans) |

The form offers no focus-independent key: `h`/`l` cycle the focus, Enter selects the focused option and Escape rejects. Focus starts on `Allow once`, and `h` from there wraps to the last option, `Reject`, in both the three-option (`Allow once` / `Always allow` / `Reject`) and the two-option layout. The form marks the focused option by cell background only, so the `focus` matcher uses `focus_style: "background"` (§ Focused-choice resolution). In the captures the focused label sits on rgb(250,178,131) and the other labels on the row background rgb(30,30,30), but the rule compares backgrounds rather than these colours. `Always allow` approves persistently and has no choice entry.

`mini` lays out the form itself, so its row breaks are hard rows rather than terminal soft wraps, and the layout depends on the pane width:

| Layout | Widths (36 rows) | Choice row | Control text | Request rule |
|---|---|---|---|---|
| Wide | 77 and above | `   Allow once   Always allow   Reject`, followed on the same row by the control line | `⇆ select  enter confirm  esc reject`, sharing the choice row | `opencode-shell-permission` |
| Split | 56–76 | The three choices on one row, with the wide margins | `  ⇆ select  enter confirm  esc reject` on its own row below the choice row | `opencode-shell-permission` |
| Compact | 36–55 | ` Allow once   Always allow   Reject` with a one-space margin; the heading, `# Shell command` and `$ ` rows start at column 0 | `pgup/pgdn scroll`, sharing the choice row at 53–55 columns and on its own row at 36–52 | `opencode-shell-permission-compact` |
| Broken | 35 and below | The choices wrap (`Reject` moves to its own row) | Any | None: no request matches and the form is answered manually |

Three breakpoints separate the layouts: `W_split` = 76, the widest split width; `W_compact` = 55, the widest compact width; and `W_min` = 36, the narrowest width that still renders the three choices on one row. The supported range is every width at or above `W_min`. The two-option layout (`Allow once` / `Reject`) follows the same rules. At every width at or above `W_min`, a form whose command is unclipped and that fits within `max_rows` matches exactly one of the two request rules, and at any width at most one matches, because their heading margins differ. Both have the same choices, answers and focus resolution.

Each request rule's `focus` matcher matches the choice row with or without its trailing control text, and its `deny` matcher accepts a choice row that ends at `Reject`. Its region matcher `controls` requires either a choice-and-control row to end the region, or a control-only row to end it with the choice row as the last non-blank row above it. A control-only row carries no label and takes no part in focus resolution.

The focus matcher's full capture is part of the request identity, and the layouts capture different text. A resize across `W_split` or `W_compact` therefore starts a new request generation, as a focus change does: an earlier request token stops validating, and a response in flight fails before its next byte, or ends `uncertain` once a byte was written.

Each operation matcher is region-scoped. It captures from after the `$ ` prefix on the first command row through each following row that continues the command, and its extraction removes the scrollbar glyph `█` beside the command box from every row (§ Operation extraction). A wide or split continuation row starts with the two-space continuation margin and then a non-space character, and the extraction removes that margin from every row after the first. A compact continuation row starts with a non-space character at column 0 and has no margin to remove. The capture is anchored at both ends: after the last captured row, only rows that are blank or hold only `█` may precede the choice row, and any other row fails the match. The operation is the displayed rows joined with `\n`; a one-row command extracts the same text. The semantic fingerprint keeps the row breaks, so a resize that moves a row break closes the episode and opens a new one.

A command taller than the form's command box scrolls inside the box: in the 57-column capture the box keeps its height, clips the command and shows a partial thumb `▀` on the first track row with the rest of the track blank. Each request rule therefore requires its region matcher `unclipped`, which holds only when the full-track thumb `█` ends every row from `# Shell command` through the last command row. A clipped command matches no request, so the Director never approves a command it cannot read in full.

| Rule | Element | Pattern |
|---|---|---|
| `opencode-shell-permission` | Region | Start `^  △ Permission required[\t-\r \x{85}\x{a0}\x{1680}\x{2000}-\x{200a}\x{2028}\x{2029}\x{202f}\x{205f}\x{3000}]*$`, end `  ⇆ select  enter confirm  esc reject[\t-\r \x{85}\x{a0}\x{1680}\x{2000}-\x{200a}\x{2028}\x{2029}\x{202f}\x{205f}\x{3000}]*$`, 24 rows |
| `opencode-shell-permission` | `heading`, `shell` (line) | `^  △ Permission required[\t-\r \x{85}\x{a0}\x{1680}\x{2000}-\x{200a}\x{2028}\x{2029}\x{202f}\x{205f}\x{3000}]*$`, <code>^   # Shell command(?:[\t-\r \x{85}\x{a0}\x{1680}\x{2000}-\x{200a}\x{2028}\x{2029}\x{202f}\x{205f}\x{3000}]&#124;$)</code> |
| `opencode-shell-permission` | `operation` (region) | `(?m)^  \$ (?P<operation>[^\t-\r \x{85}\x{a0}\x{1680}\x{2000}-\x{200a}\x{2028}\x{2029}\x{202f}\x{205f}\x{3000}][^\n]*(?:\n  [^\t-\r \x{85}\x{a0}\x{1680}\x{2000}-\x{200a}\x{2028}\x{2029}\x{202f}\x{205f}\x{3000}][^\n]*)*)(?:\n *(?:█ *)?)*\n   Allow once   `, with `row_margin` two spaces, `margin_rows` `continuation` and `row_trailer` `█` |
| `opencode-shell-permission` | `unclipped` (region) | `(?m)^   # Shell command +█ *\n(?: *█ *\n)*  \$ [^\n]*█ *(?:\n  [^\t-\r \x{85}\x{a0}\x{1680}\x{2000}-\x{200a}\x{2028}\x{2029}\x{202f}\x{205f}\x{3000}][^\n]*█ *)*(?:\n *(?:█ *)?)*\n   Allow once   ` |
| `opencode-shell-permission` | `controls` (region) | <code>(?m)^   Allow once   (?:Always allow   )?Reject(?: +⇆ select  enter confirm  esc reject *&#124; *(?:\n *)*\n  ⇆ select  enter confirm  esc reject *)\z</code> |
| `opencode-shell-permission` | `focus`, `allow`, `deny` (line) | `^   Allow once   (?:Always allow   )?Reject(?: +⇆ select  enter confirm  esc reject)?[\t-\r \x{85}\x{a0}\x{1680}\x{2000}-\x{200a}\x{2028}\x{2029}\x{202f}\x{205f}\x{3000}]*$` with `focus_style` `background`, `^   (?P<label>Allow once)   `, <code>(?:Allow once&#124;Always allow)   (?P&lt;label&gt;Reject)(?: &#124;$)</code> |
| `opencode-shell-permission-compact` | Region | Start `^△ Permission required[\t-\r \x{85}\x{a0}\x{1680}\x{2000}-\x{200a}\x{2028}\x{2029}\x{202f}\x{205f}\x{3000}]*$`, end `pgup/pgdn scroll[\t-\r \x{85}\x{a0}\x{1680}\x{2000}-\x{200a}\x{2028}\x{2029}\x{202f}\x{205f}\x{3000}]*$`, 24 rows |
| `opencode-shell-permission-compact` | `heading`, `shell` (line) | `^△ Permission required[\t-\r \x{85}\x{a0}\x{1680}\x{2000}-\x{200a}\x{2028}\x{2029}\x{202f}\x{205f}\x{3000}]*$`, <code>^# Shell command(?:[\t-\r \x{85}\x{a0}\x{1680}\x{2000}-\x{200a}\x{2028}\x{2029}\x{202f}\x{205f}\x{3000}]&#124;$)</code> |
| `opencode-shell-permission-compact` | `operation` (region) | `(?m)^\$ (?P<operation>[^\t-\r \x{85}\x{a0}\x{1680}\x{2000}-\x{200a}\x{2028}\x{2029}\x{202f}\x{205f}\x{3000}][^\n]*(?:\n[^\t-\r \x{85}\x{a0}\x{1680}\x{2000}-\x{200a}\x{2028}\x{2029}\x{202f}\x{205f}\x{3000}][^\n]*)*)(?:\n *(?:█ *)?)*\n Allow once   `, with `row_trailer` `█` and no `row_margin` |
| `opencode-shell-permission-compact` | `unclipped` (region) | `(?m)^# Shell command +█ *\n(?: *█ *\n)*\$ [^\n]*█ *(?:\n[^\t-\r \x{85}\x{a0}\x{1680}\x{2000}-\x{200a}\x{2028}\x{2029}\x{202f}\x{205f}\x{3000}][^\n]*█ *)*(?:\n *(?:█ *)?)*\n Allow once   ` |
| `opencode-shell-permission-compact` | `controls` (region) | <code>(?m)^ Allow once   (?:Always allow   )?Reject(?: +pgup/pgdn scroll *&#124; *(?:\n *)*\npgup/pgdn scroll *)\z</code> |
| `opencode-shell-permission-compact` | `focus`, `allow`, `deny` (line) | `^ Allow once   (?:Always allow   )?Reject(?: +pgup/pgdn scroll)?[\t-\r \x{85}\x{a0}\x{1680}\x{2000}-\x{200a}\x{2028}\x{2029}\x{202f}\x{205f}\x{3000}]*$` with `focus_style` `background`, `^ (?P<label>Allow once)   `, <code>(?:Allow once&#124;Always allow)   (?P&lt;label&gt;Reject)(?: &#124;$)</code> |

`mini` hides the cursor while a form or dialog shows, and its recorded position varies, including the pending-wrap position on the row below the form. Every OpenCode permission rule therefore matches with cursor `any` and relies on its heading and control-text anchors.

| Stage | Bytes | Gate | Native effect |
|---|---|---|---|
| Allow 1 | `0d` | Focus `allow-once` | Selects `Allow once`; OpenCode runs the command once |
| Deny 1 | `68` | Focus `allow-once` | Moves the focus to `Reject` |
| Deny 2 | `0d` | Focus `deny-once` | Selects `Reject`; OpenCode opens the rejection-reason dialog and keeps the request pending |
| Deny 3 | `0d` | Screen `opencode-reject-reason` (wide, split) or `opencode-reject-reason-compact` (compact) | Submits an empty reason; OpenCode rejects the request without running it |

The stages are the same for both request rules, and each rule has its own allow and deny answer entries. The captures at 120, 57, 55 and 36 columns showed the same native effects in every layout.

The continuation rules `opencode-reject-reason` and `opencode-reject-reason-compact` (§ Continuation screens) continue `opencode-shell-permission` and `opencode-shell-permission-compact` after `deny-once`, and each region matcher `empty_reason` recognises an empty reason. In the wide layout the placeholder textarea shares the control line's row; in the split layout the control line moves to its own row and the textarea row reads the same text as the prompt row above it. The compact dialog has no prompt row; its heading, placeholder and control text sit at column 0, the control text on the row after the placeholder. In either dialog, typed text replaces the placeholder and fails the gate. CAFleet never types a reason and never sends Escape to the dialog.

| Rule | Element | Pattern |
|---|---|---|
| `opencode-reject-reason` | Region | Start `^  △ Reject permission[\t-\r \x{85}\x{a0}\x{1680}\x{2000}-\x{200a}\x{2028}\x{2029}\x{202f}\x{205f}\x{3000}]*$`, end `  enter confirm  esc cancel[\t-\r \x{85}\x{a0}\x{1680}\x{2000}-\x{200a}\x{2028}\x{2029}\x{202f}\x{205f}\x{3000}]*$`, 24 rows |
| `opencode-reject-reason` | `heading` (line), `empty_reason` (region) | `^  △ Reject permission[\t-\r \x{85}\x{a0}\x{1680}\x{2000}-\x{200a}\x{2028}\x{2029}\x{202f}\x{205f}\x{3000}]*$`; <code>(?m)^  △ Reject permission *(?:\n *)*\n  Tell OpenCode what to do differently *(?:\n *)*\n  Tell OpenCode what to do differently(?: {2,}enter confirm  esc cancel *&#124; *(?:\n *)*\n  enter confirm  esc cancel *)\z</code> |
| `opencode-reject-reason-compact` | Region | Start `^△ Reject permission[\t-\r \x{85}\x{a0}\x{1680}\x{2000}-\x{200a}\x{2028}\x{2029}\x{202f}\x{205f}\x{3000}]*$`, end `^enter reject esc back[\t-\r \x{85}\x{a0}\x{1680}\x{2000}-\x{200a}\x{2028}\x{2029}\x{202f}\x{205f}\x{3000}]*$`, 24 rows |
| `opencode-reject-reason-compact` | `heading` (line), `empty_reason` (region) | `^△ Reject permission[\t-\r \x{85}\x{a0}\x{1680}\x{2000}-\x{200a}\x{2028}\x{2029}\x{202f}\x{205f}\x{3000}]*$`; `(?m)^△ Reject permission *(?:\n *)*\nTell OpenCode what to do differently *(?:\n *)*\nenter reject esc back *\z` |

The native captures showed this decision evidence:

| Answer | Native surface | Continuation |
|---|---|---|
| Allow | The hint area briefly shows `Waiting for permission event…`, then the form disappears without transcript text, the command runs and the member prints its report, a `<agent> · <model> · <duration>` line and the idle composer | The member continues on its own |
| Deny | After the final Enter the hint area briefly shows `Waiting for permission event…`, then the dialog disappears and the transcript shows `Step interrupted`, a `<agent> · <model> · <duration>` line and the idle composer; nothing runs | OpenCode does not continue after a rejection; the Director's next message reaches the idle composer through the prompt submission plan and continues the member |

Because an answered form or dialog leaves no transcript text, the false-positive guard rests on the live-region anchors. The fixtures are native 36-row captures, each with one reviewed masked row; the visible public model name and context-usage counter are neither account, plan nor path text:

| Fixtures | Geometry | Masked row | Supports |
|---|---|---|---|
| `opencode-2.1.4-ask-permission-1`, `-allow-pre-stage-1`, `-deny-pre-stage-1`, `-deny-pre-stage-2`, `-deny-pre-stage-3`, `-focus-always-1`, `-idle-composer-1` and the submission fixtures | 36x120 | 35 | The wide layout and its dialog, the composer and the submission plans |
| `opencode-2.1.4-narrow-permission-057-1`, `-narrow-deny-pre-stage-2-057-1`, `-narrow-deny-pre-stage-3-057-1`, `-narrow-long-command-057-1`, `-narrow-idle-composer-057-1` | 36x57 | 35 | The split layout with a wrapped command, its dialog, the clipped long command and the idle composer |
| `opencode-2.1.4-compact-permission-055-1`, `-compact-deny-pre-stage-2-055-1`, `-compact-deny-pre-stage-3-055-1` | 36x55 | 35, or 0 for the dialog | The compact layout with its control text sharing the choice row, and the compact dialog |
| `opencode-2.1.4-narrow-permission-min-1`, `-narrow-deny-pre-stage-2-min-1`, `-narrow-deny-pre-stage-3-min-1` | 36x36 | 0 for the form and the dialog, 35 for the stage-2 form | The compact layout at `W_min` with a control-only row, and the compact dialog |

A capture whose compact control text sits on row 35 masks row 0, a transcript row, so its rule region lies in unmasked rows. The OpenCode limits apply as follows:

| Limit | Behavior |
|---|---|
| A pane narrower than `W_min`; a command clipped by the command box; a form taller than `max_rows` or the pane; a command row that begins with whitespace after its layout's continuation margin, such as an indented line of a multi-line command | No request matches and the form is answered manually; below `W_min` the choices break and input stays held, and an indented row fails the anchored capture |
| A resize while the form is open | A new request generation, and a new episode when the command's row breaks move; an earlier token stops validating |
| Resizing a running OpenCode member pane on herdr | OpenCode can receive a fragment of a terminal colour reply as keys (`Invalid key name`), after which no request matches; size the herdr pane before creating the member |
| A non-shell permission | Never asks under the `cafleet-ask` preset |
| A typed reason identical to the placeholder | Cannot be told from an empty textarea; such typing is human input, which fails the deny stage-3 gate inside a response |

The Hermes profile from `2026.9.24` applies to `hermes chat --cli` under its `smart` launch choice; its request rules and answers come from captures under the operator configuration's `approvals.mode: smart`. An approval prompt shown under `approvals.mode: manual` is detected only when its screen matches a request rule; otherwise it is an `unknown` frame answered manually:

| Capability | Status | Declaration |
|---|---|---|
| Detection | `tested` | Request rule `hermes-approval-escalate` for the guardian ESCALATE form (choices `1. Allow once`, `2. Allow for this session`, `3. Add to permanent allowlist`, `4. Deny`) and `hermes-approval-override` for the guardian-DENY owner-override form (`1. Allow once`, `2. Deny`), on the primary screen from the box's `╭` row to the `↑/↓ to select` footer. The operation is the box's command rows. `smart` reached no `execute_code` or action gate and no other native form in the captures. |
| Allow answer | `tested` | One stage `1` (`31`) on both forms, as `allow-once` |
| Deny answer | `tested` | One stage `4` (`34`) on the ESCALATE form and `2` (`32`) on the override form, as `deny-once` |
| Empty-composer observation | `tested` | `hermes-idle-composer` observes the idle `❯ ` prefix; `hermes-working-composer` observes the complete working placeholder at `☤ ❯ `, with the terminal cursor immediately after its prefix. Both are verified non-interaction boundaries |
| Prompt submission | `tested` | Composer plan `prompt`, submit `0d` |
| Native-exec submission | `tested` | Composer plan `exec`, submit `0d` |

A digit selects and confirms its choice whatever the focus, so the answer stages have no focus gate. The focused choice carries the `❯ ` marker, and choices carry no background colour. Recorded layout limits: the ESCALATE form is detected at 37 columns and above, below which `3. Add to permanent allowlist` wraps; the override form is detected at 28 columns and above, below which the box border wraps. A command too long for the box adds `5. Show full command` and is answered manually.

The idle rule keeps its classic REPL prefix and rotating placeholder. The working rule has priority 50, uses a primary-screen single-row region with `cursor=at_end`, starts at `^☤ ❯ type to redirect this turn · Tab queues it for after *$` and ends at the prefix pattern `^☤ ❯ `; its line matcher `prompt` requires the same full placeholder as the start pattern. This binds the cursor to prefix column 4 while requiring the measured working placeholder, independently of submitted `❯ ...` history above it. A typed suffix, paste token, arbitrary working literal, cursor away from the prefix or multi-row retained draft supplies no empty-composer observation. Both operations cite native working/idle post-submit and unsubmitted-literal fixtures under § Bundled profiles and native screen fixtures.

The Director answers single-request choices only: allow once or deny, on both the ESCALATE and the override prompt. The session and always choices have no choice entry. When the guardian model is unavailable, Hermes shows the same human prompt, detected by the same rule. An unanswered prompt fails closed after `approvals.timeout` (default 300 seconds): Hermes prints its timeout notice and returns a BLOCKED result to the model, the form disappears, and the runner closes the episode with the `not_visible` close reason. A later bound response then fails request-token revalidation.

The pi profile from `1.5.0` applies to pi under every pi launch mode; only `cafleet-ask` shows an approval form, the CAFleet member gate's form (§6.7 pi member gate), and the argv pins `--tui-mode fullscreen`:

| Capability | Status | Declaration |
|---|---|---|
| Detection | `tested` | Request rule `pi-gate-approval`: the gate's form on the alternate screen, from the `CAFleet member gate: allow this bash command?` title row to pi's select hint row within 24 rows, with the `$ ` operation row, the focused choice and the `Allow once` and `Deny` choice rows. The operation is the single `$ ` row |
| Allow answer | `tested` | Answer rule `pi-gate-allow-once`: one stage, Enter (`0d`) gated on focus `allow-once`, as `allow-once` |
| Deny answer | `tested` | Answer rule `pi-gate-deny-once`: two stages, `j` (`6a`) gated on focus `allow-once`, then Enter (`0d`) gated on focus `deny-once`, as `deny-once` |
| Empty-composer observation | `tested` | `pi-idle-composer` observes an empty editor between plain `─` borders; `pi-working-composer` observes an empty editor below its measured `Working` border. Both require the terminal cursor inside the alternate-screen editor region and establish a verified non-interaction boundary |
| Prompt submission | `tested` | Composer plan `prompt`, submit `0d` (§ Executable native plans) |
| Native-exec submission | `tested` | Composer plan `exec` for `! COMMAND`, submit `0d` (§ Executable native plans) |

The title, the `Allow once` and `Deny` labels and the `$ ` operation prefix are CAFleet-authored, so they stay fixed across pi versions; pi renders the select list, its focus marker and its hint row. The form sits between two full-width `─` rules in place of the editor:

```text
 CAFleet member gate: allow this bash command?

 $ <command>

 → Allow once
   Deny

 ↑↓ navigate  enter select  escape/ctrl+c cancel
```

`Allow once` is the first choice and holds the initial focus. The focused choice carries the `→ ` marker in the accent colour, so the `focus` matcher uses `focus_style` `marker` (§ Focused-choice resolution). The list moves with Up/Down and with `j`/`k`, and stops at either end. The deny answer moves the focus with `j`, because the Down key's bytes begin with Escape. It sends no Escape, because Escape dismisses the form and the gate then blocks the command with the dismissal reason. pi hides the terminal cursor and places it on the footer or the focused choice row while the form shows, so the request rule matches with cursor `any`.

| Rule | Element | Pattern |
|---|---|---|
| `pi-gate-approval` | Region | Start `^ CAFleet member gate: allow this bash command\? *$`, end `^ ↑↓ navigate  enter select  escape/ctrl\+c cancel *$`, 24 rows, cursor `any` |
| `pi-gate-approval` | `heading`, `footer` (line) | The region's start and end patterns |
| `pi-gate-approval` | `operation` (region) | <code>(?m)^ CAFleet member gate: allow this bash command\? *\n *\n \$ (?P&lt;operation&gt;[^\n]*?) *\n *\n (?:→ &#124;  )Allow once *$</code> |
| `pi-gate-approval` | `focus`, `allow`, `deny` (line) | <code>^ → (?:Allow once&#124;Deny) *$</code> with `focus_style` `marker`, <code>^ (?:→ &#124;  )(?P&lt;label&gt;Allow once) *$</code>, <code>^ (?:→ &#124;  )(?P&lt;label&gt;Deny) *$</code> |
| `pi-idle-composer` | Region | Start and end `^─+ *$`, 3 rows, cursor `inside` |
| `pi-idle-composer` | `editor` (region) | `\A─+ *\n *\n─+ *\z` |
| `pi-working-composer` | Region | Start `^── [⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏] Working ─+ *$`, end `^─+ *$`, 3 rows, cursor `inside` |
| `pi-working-composer` | `editor` (region) | `\A── [⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏] Working ─+ *\n *\n─+ *\z` |

The operation is the single row after `$ `: the gate blocks a command holding a control character, a newline included, before the form opens (§6.7 pi member gate), so the form shows one command line. A command line too long for its row wraps onto a further row, which the operation matcher does not accept. The `operation` evidence reports that command line.

Both composer rules have priority 50, require exactly one blank editor row between the matched borders and preserve the alternate-screen/cursor constraint. The working rule accepts only the measured spinner frames in the table and the `Working` title; its empty editor can confirm submission during a running native command or model turn. A typed row or paste token remains nonempty, a multi-row draft exceeds the three-row region, and the measured `↑ N more` overflow title supplies no working-empty match. The transcript draws command results between `─` borders too, with the cursor in the live editor below them; history alone establishes no empty composer. Each operation cites its native idle/working post-submit and retained-literal fixtures under § Bundled profiles and native screen fixtures.

| Answer | Native surface in the captures | Continuation |
|---|---|---|
| Allow | The form closes and pi runs the command once; the tool result shows its output | The member continues on its own |
| Deny | The form closes and nothing runs; the tool result reads `the operator denied this command` | The member continues its run with that result |

| Operation | Native result in the captures |
|---|---|
| Prompt | The literal fills the editor; Enter clears it and starts a turn |
| Native exec | `! COMMAND` fills the editor; Enter runs the command locally, adds a `$ COMMAND` block with its output to the transcript, starts no turn and leaves the editor idle |

`W_min` is 49 columns, the narrowest width at which the hint row stays on one row. A resize while the form is open redraws it at the new width. The fixtures are native 36-row captures with rows 0–19 and 34 masked, covering the startup header with the installed skill list, earlier transcript and the token, cost and model footer:

| Fixtures (each `pi-1.5.0-<screen>-<multiplexer>-<width>-1`) | Geometry | Supports |
|---|---|---|
| `allow-pre-stage`, `deny-pre-stage-1`, `deny-pre-stage-2` | 36x120 and 36x49, `tmux` and `herdr` | Detection with `Allow once` or `Deny` focused, and each answer stage |
| `idle-composer`, `exec-empty-composer`, `allow-continuation`, `deny-continuation`, `exec-post-submit` | Same | The empty editor, including after each answer and after a native exec |
| `prompt-typed-literal`, `prompt-post-submit`, `exec-typed-literal` | Same | The submission plans' literal and post-submit screens |

The `herdr` fixtures were captured with herdr's terminal identity (`TERM`, `COLORTERM`, `TERM_PROGRAM`, `TERM_PROGRAM_VERSION` and `HERDR_ENV`) in the native child's environment. The pi limits apply as follows:

| Limit | Behavior |
|---|---|
| A pane narrower than `W_min`; a command line that wraps; a form taller than `max_rows` or the pane | No request matches and the form is answered manually on the native surface |
| A command line equal to a choice label, such as `Deny` | The label matchers match twice, so no request matches |
| The `cafleet` launch mode | Shows no approval form; the profile supplies the composer and submission plans only |

A backend fixture file has exactly these required fields:

| Field | Requirement |
|---|---|
| `schema_version` | `1` |
| `backend` | Equals the backend of every catalog fixture entry referring to this file |
| `provenance` | `native` |
| `screens` | An object mapping each catalog fixture `id` to its captured screen object |

Screen keys follow their entries' order in `bundled.json` `fixtures[]` for that backend. Backend fixture files use 2-space pretty JSON with one array element per line.

Each screen object has exactly these fields, of which only `cell_backgrounds` and `cell_styles` are optional:

| Field | Requirement |
|---|---|
| `rows`, `cols` | Positive measured geometry, equal to the catalog fixture entry |
| `alternate_screen`, `screen_seq`, `cursor_row`, `cursor_col` | Measured screen state; `cursor_row` is below `rows`, and `cursor_col` is at most `cols`, where `cursor_col` equal to `cols` records the terminal's pending-wrap position after the last column |
| `lines` | Exactly `rows` captured row texts, in viewport order |
| `masked_rows` | Unique zero-based row indices, each `{"row":N,"reason":"reviewed_privacy"}`; the corresponding `lines` entry is the empty string |
| `cell_backgrounds` | Optional runs `{"row":N,"start_col":C,"end_col":E,"bg":COLOUR}`, one per maximal run of cells in an unmasked row that share one non-default background, `end_col` exclusive; `COLOUR` is `{"kind":"indexed","index":I}` or `{"kind":"rgb","r":R,"g":G,"b":B}`. An absent key means every cell has the default background. |
| `cell_styles` | Optional runs `{"row":N,"start_col":C,"end_col":E,"faint":BOOL,"foreground":COLOUR_OR_NULL}`, one per maximal run of cells in an unmasked row that share one non-default `faint`/`foreground` pair, `end_col` exclusive; `COLOUR` uses the `cell_backgrounds` form. An absent key means every cell has the default style. |

A fixture's `cell_backgrounds` and `cell_styles` runs record the captured screen's per-cell background, faint and foreground attributes, so a fixture reproduces the focus styling a `focus_style` matcher reads, the separator backgrounds that sidebar clipping reads (§ Diff sidebar clipping) and the suggestion style a `composer_empty` `suggestion` rule reads (§6.7 Native suggestions).

A run spans the continuation cell of a wide character together with its leading cell. Writing a wide scalar resets its continuation cell to the default background and style (§ Screen model facts), so a run spanning both cells reproduces the runtime cells exactly. A `cell_styles` run lies at or before the last non-blank cell of its row, because no rule reads `faint` or `foreground` on a row's trailing blanks: a suggestion rule checks cells only up to the row's last non-blank cell, and draft extraction trims trailing blanks. `cell_backgrounds` runs keep trailing blanks, because separator clipping and background focus read them.

Masking applies only to rows removed by the reviewed capture privacy policy, such as home paths and account, model or plan text. Every other row keeps its captured text exactly, including trailing spaces. The detection region of every rule citing the fixture lies entirely in unmasked rows.

Each fixture file has `schema_version` `1`, `provenance` `native` and its catalog entry's `backend`. It holds a screen for every catalog fixture entry's `id` and no uncited screen, and each screen's canonical digest equals its entry's `sha256` and its `rows` and `cols` equal its entry's.

Each file and screen parses with exactly its fields above. Every row declared in `masked_rows` is in range and empty. Every `cell_backgrounds` run lies inside the geometry and outside masked rows, has `start_col` below `end_col`, overlaps no other run and carries a valid non-default colour. Every `cell_styles` run satisfies the same placement rules against the other `cell_styles` runs and carries at least one non-default attribute with a valid colour. JSON formatting leaves the canonical screen digest unchanged.

Every bundled catalog pattern spells its classes explicitly: it contains no Perl class or boundary (`\s \S \d \D \w \W \b \B`) and no escape outside the syntax (§ Pattern syntax).

A rule's cursor condition treats a pending-wrap cursor (§6.9 Screen model facts) as standing on the last cell of its row, both in fixtures and on live screens. A region spanning the full width therefore counts a cursor at column `cols` on one of its rows as inside the region.

###### Pattern syntax

Profile patterns, frame guards and region matchers of the bundled catalog and the built-in patterns this specification quotes (the Hermes version line, the ANSI CSI pattern) use this syntax: literals, and a backslash before any ASCII punctuation character matching that character literally (`\.`, `\?`, `\(`, `\[`, `\$`, …); `.` (any scalar except `\n`); bracket classes with ranges, negation, POSIX `[:name:]` classes and `\p{…}` general-category or script names; the escapes `\t \n \r \f \v` and `\xHH` / `\x{H…}` for a code point; `^ $` (line anchors under `(?m)`, text anchors otherwise), `\A`, `\z`; groups `(…)`, `(?:…)`, `(?P<name>…)`; alternation; greedy and lazy quantifiers `* + ? {n} {n,} {n,m}` with counts up to 1000; flags `i m s U` in `(?flags)` and `(?flags:…)`; and the Perl classes `\s \S \d \D \w \W` and boundaries `\b \B`. Perl classes have Unicode meaning: `\s` is White_Space (U+0009–U+000D, U+0020, U+0085, U+00A0, U+1680, U+2000–U+200A, U+2028, U+2029, U+202F, U+205F, U+3000), `\d` is general category Nd, `\w` is Alphabetic, M, Nd, Pc and Join_Control, `\b` is a boundary between `\w` and non-`\w`, and the capitals negate. Matching is leftmost-first over Unicode scalar values, and match offsets are UTF-8 byte offsets. Whether a pattern outside this syntax is accepted, and how it matches, is implementation-defined.

##### Interaction frame guard

An interaction rule (`permission_request`, `question`, `unknown_interaction`) may declare `frame`, `{ "require_patterns": [...], "reject_patterns": [...] }`, with zero or more compilable require patterns and at least one compilable reject pattern; one without a reject pattern fails with `interaction frame requires a reject pattern`. Other rules reject the field. Each require pattern must match a live-screen line of the same frame and no reject pattern may match one; a frame failing the guard yields no observation from that rule.

##### Operation extraction

A rule's `extract.operation` names the matcher and the named capture that supply the operation, plus optional row-processing keys:

| Key | Values | Behavior |
|---|---|---|
| `matcher_id`, `capture` | A declared matcher and one of its named captures | Required; the captured text, whose rows are separated by `\n` when the matcher is region-scoped |
| `row_margin` | A non-empty single-row string | The margin removed from the rows `margin_rows` selects; a selected row without it fails the match |
| `margin_rows` | `all` (default) or `continuation` | `all` selects every captured row. `continuation` exempts the first row, whose prefix the matcher already consumed. |
| `row_trailer` | A non-empty single-row literal without leading or trailing whitespace | A captured row that ends with the trailer, ignoring trailing whitespace, loses the trailer and the whitespace before it; every other row is unchanged at this step |

Catalog loading fails for an unknown key, an empty or multi-row `row_margin`, a `margin_rows` value other than `all` or `continuation`, `margin_rows` declared without `row_margin`, and an empty, multi-row or whitespace-bearing `row_trailer`.

A request or question rule processes each captured row in this order:

1. Remove the trailer when `row_trailer` is declared.
2. Remove `row_margin` from each row `margin_rows` selects.
3. Remove trailing whitespace.

The operation joins the processed rows with `\n`. The margin-then-trim order keeps a whitespace-only margined row, such as a blank line inside a multi-line Claude command, a match.

##### Diff sidebar clipping

A rule's region may declare `panel_separator`, a single non-whitespace glyph. When any rule of the profile declares it, the detector derives one clipped line set per screen:

1. A row is a panel row when, at some column `C` with `40 <= C < cols - 1`, the cell (§ Screen model facts) holds the separator glyph, its background differs from the cell at `C - 1` and equals the cell at `C + 1`.
2. Every panel row of the screen must share one `C`; otherwise no row is clipped.
3. Each panel row keeps cells `0..C-1`; other rows are unchanged. A clipped row ends a logical line and is never joined to the next row.

A rule that declares `panel_separator` matches against the clipped set, and its region rows, matcher rows, label cells and cursor test use the same row indexes; a rule without it sees the unclipped rows. A boxed operation's `│ ` gutter shares its row's background, so it is never a separator. The separator column depends on the pane width, so it is found per frame. Catalog loading fails with `panel separator requires one non-whitespace glyph` for an empty, multi-glyph or whitespace `panel_separator`.

##### Executable native plans

Each operation's eligibility for the run's selected profile is independent (§ Profile selection by minimum version): empty-composer observation, executable prompt submission and executable native-shell submission each have their own coverage, `tested` exactly when the selected profile has a validated executable definition for that operation and otherwise `unknown`, with ordinary input for that operation using delivery fallback. Observing an empty composer supplies input-state evidence; executable submission additionally requires its measured literal/submit plan, so an observation definition without a submission plan keeps its observation coverage and yields no executable composer capability. Synthetic plans run only in explicitly selected test services, and a catalog selected explicitly by test services describes its fixture coverage.

`capabilities.answers` holds two entry forms, distinguished by the `decision` key:

| Form | Keys | Request rule class | Selected choice scope | Unique by |
|---|---|---|---|---|
| Decision answer | `decision,request_rule_id,answer_rule_id,choice_id,focus_matcher_id,stages,fixture_ids` | `permission_request` | `single_request` | `request_rule_id` and `decision` |
| Choice answer | `request_rule_id,answer_rule_id,choice_id,focus_matcher_id,stages,fixture_ids` | `question` | Any declared scope (`single_request`, `persistent`) | `request_rule_id` and `choice_id` |

A decision answer on another rule class fails with `answer requires permission form`, a choice answer on another rule class with `choice answer requires question form`, and a decision answer selecting a choice of another scope with `answer requires single-request scope`. A choice id is 1–64 bytes matching `^[a-z0-9]+(?:-[a-z0-9]+)*$`. For a choice answer, the scope is evidence for the Director rather than a gate on the plan, and the plan's declared `choice_id` is the only choice it can select. An answer is looked up by its request rule and its key, the decision or the choice id.

An answer definition retains its validated `choice_id`, `focus_matcher_id` and ordered `stages`, in addition to its key, request, rule and provenance fields. Each stage has `bytes_hex`, `required_rule_id` and `required_choice_id`, where the required rule and choice equal the answer's request and selected choice, and at most one optional gate, checked on the fresh observation immediately before the runner writes that stage:

| Gate | Holds when |
|---|---|
| `required_focus_choice_id` | The plan's focus matcher resolves this choice id as focused on the request form |
| `required_screen_rule_id` | This `permission_continuation` rule matches as the continuation of the bound request (§ Continuation screens) |

A stage without a gate is focus-independent; every bundled stage is ungated except the two `claude-auto-default-decline` stages, the OpenCode stages and the pi stages. The catalog fails to load unless:

- every focus gate names a choice of the request rule;
- every screen gate names a `permission_continuation` rule whose `continues_rule_id` is the answer's request rule, and the stage immediately before it is gated on that rule's `continues_choice_id`;
- a plan with any gated stage gates every stage, and its final stage is gated either on the answer's own `choice_id` or on a continuation rule whose `continues_choice_id` is that choice;
- a plan with a focus gate has a focus matcher that declares `focus_style`, and every choice label it compares is captured by a line-scoped matcher;
- a `permission_continuation` rule declares no operation, choices or answers.

Provenance validity additionally requires every `permission_continuation` rule to cite a native fixture.

###### Focused-choice resolution

A focus matcher declares `focus_style` `background` or `marker`; a focus-gated plan whose focus matcher declares neither fails with `a focus-gated plan needs a focus_style matcher`. Under `marker`, the focused choice is the one choice whose label matcher matched the same row as the focus matcher; no such choice, or several, resolve to no choice.

A focus matcher with `focus_style: "background"` resolves the focused choice from cell styling, without depending on the theme:

1. Locate each choice label's cells through its line's cell boundaries.
2. Take the choice row background as the background that covers more than half of the choice line's cells. When no background does, no choice resolves.
3. Resolve the focused choice as the one choice whose label cells all carry a single background that differs from the row background, provided no other choice label carries a differing background.

The majority rule holds in every layout: the OpenCode compact layout has no left margin and its highlight covers the focused label plus one cell on each side, so with a focused first or last choice neither edge cell carries the row background.

A focus on an option without a choice entry, on several options or on none resolves to no choice, and a gate that does not hold is never written. The focus value compared in the observed request, the answer match and the write-time binding is the resolved focused choice id (or none) together with a capture that depends on the focus style. Under `background` the capture is the focus matcher's capture; the focus text alone does not change when a styling-only focus moves, so the resolved choice id makes a styling-only focus change start a new request generation. Under `marker` the capture is the choice-label row layout, the row each choice label matched, which stays constant while only the marker moves (the focus matcher's own text changes), so a between-stage focus gate compares an unchanged capture and only a change in the choice rows' layout changes it.

The composer object at `capabilities.composer` has required `rule_ids,fixture_ids` and `submission`. `submission` has exactly the required operation definitions `prompt,exec`, each an object with exactly `prefix_hex,submit_hex,literal_framing,plain_literal_limit,fixture_ids`. Prompt has an empty prefix; exec has prefix `2120`, the UTF-8 bytes of `! `; `submit_hex` is the measured native submit key. Prompt and exec carry independent provenance. Every plan's fixture references establish the complete composer/work boundary for that operation, and provenance validation checks them against the profile's interval provenance (§ Profile selection by minimum version). Unknown fields fail catalog loading, and incomplete or incompatible provenance fails provenance validation.

| Composer field | Required value |
|---|---|
| `literal_framing` | `plain` or `bracketed_paste`; the latter requires native evidence that the framed literal and measured submit key submit at every measured size through the maximum preview |
| `plain_literal_limit` | `null` when the maximum preview submits plainly, otherwise exactly `unit,max`, with unit `bytes` or `scalars` and integer max ≥ 1; this is the selected tested cap for plain prefix plus literal. A size-gated backend uses its stable measured boundary and a timing-gated backend uses a conservative tested cap |

Bracketed framing writes the operation prefix plainly, then opening marker `1b5b3230307e`, unchanged literal UTF-8 and closing marker `1b5b3230317e` as one literal stage. The transport exposes the child's current DEC private mode 2004 from its parsed screen.

| Plan property | Inclusive limit or required representation |
|---|---|
| Answer stages | 1–16 ordered stages. |
| Escape | No stage contains the byte `1b`, for every backend. |
| Answer bytes | 1–256 decoded bytes per stage; at most 4096 decoded bytes per plan. |
| Composer submit key | 1–32 decoded bytes. |
| Composer prefix | Zero bytes for prompt; exactly two bytes for exec as specified above. |
| Encoded native ordinary submission | At most 65536 bytes for prefix, literal UTF-8, both paste markers when framed (12 bytes) and submit key together; the complete control JSON independently satisfies the control-frame limit (§ Input arbitration and the original native answer). |
| Hex byte definitions | Even-length lowercase hexadecimal, with nonempty bytes wherever the domain above requires them. |

Validate the complete shape, reference set, hex form and byte/stage bounds before decoding and constructing an executable plan. Keep selected native bytes and live semantic identity in memory, outside error messages, diagnostic output and persistence. An ordinary submission writes its chunked literal stage and submit-key stage separately (§ Measured submission plans and settle intervals). A write that transfers zero bytes because the descriptor would block (`EAGAIN`/`EWOULDBLOCK`) or the call was interrupted (`EINTR`) yields and requires fresh validation before another write. Answer stages retain their independently measured order and request/choice/focus gates; a positive short answer stage stops the remaining plan with uncertainty.

##### Measured submission plans and settle intervals

Measurement establishes each operation plan. For each bundled profile, capture native empty-composer, typed-literal/pre-submit and post-submit behavior for prompt and native shell input on a native version within its interval. Record the native version, multiplexer, screen geometry, policy, exact stage bytes and times, the observed native result and privacy-reviewed fixture digests. An idle screen capture alone is insufficient evidence that either operation executes.

The runner uses these monotonic bounds within the original attempt deadline:

| Bound | Duration |
|---|---|
| Attempt | 5 s |
| Settle after a completed literal, and after fallback Escape | 500 ms |
| Post-submit verification on a verified-composer run | 2 s |
| Paste close reserve | 250 ms |
| Stage write margin | 250 ms |

Before the first byte and on each zero-byte reselection, render the selected delivery method and apply the first matching row:

| Condition | Literal stage or result |
|---|---|
| Plan declares `bracketed_paste` and the child has enabled mode 2004 | Bracketed literal |
| Plain limit is null, or rendered prefix plus literal is within it | Plain literal |
| Preview exceeds the plain limit | Re-render the body as the pointer form of §6.5 Push notifications and write plainly |
| Immediate prompt/exec exceeds the plain limit | Zero-byte refusal before receipt reservation (§6.3 Error Messages) |
| Queued prompt/exec exceeds the plain limit | Settle `not_submitted` with outcome reason `plain_limit_exceeded`, publish the result and advance the FIFO |

Fallback applies the same selection to its literal stage. A run without a selected profile writes plainly with no limit. The 4000-scalar inline limit remains unchanged; framing and inline/pointer rendering are outside the digest preimage and become pinned at the first byte.

A verified-composer run is one whose recorded backend and native version select empty-composer observation and an executable plan for the operation: `prompt` for previews/prompts, `exec` for native commands. The runner and every client derive this predicate from the same recorded identity. Such an attempt begins only when its remaining budget covers `500 ms × settles + 2 s + 250 ms`: one settle for profile delivery, two for fallback. An immediate attempt below that bound queues with zero bytes and `defer_reason=delivery_budget_exhausted`; a deferred head waits with zero bytes under the same obstacle and retries with its own five-second attempt budget.

1. Revalidate run, placement, observer and interaction state, then durably reserve the input before any child byte.
2. Write the selected literal stage. A positive short write retains the unwritten suffix and cumulative count, stays `partial` and returns to the pre-write wait without settling; each further chunk needs a fresh write authorization after fresh ordinary draining and an interaction check.
3. Start the 500 ms settle only after the last literal chunk, including any closing paste marker. Service child output, outer input and control traffic throughout it.
4. Revalidate identity and interaction state; keyboard and pointer input and terminal replies during this settle preserve the submit stage. On a verified-composer run, reserve the full 2 s verification window plus the stage write margin (250 ms) before writing the submit key; insufficient budget records `uncertain/deadline_expired`, stage `partial`.
5. Write the submit-key stage once. A positive short submit key records `uncertain/write_failed`.
6. On a verified-composer run, retain the active input and pane lock for post-submit verification. A fresh verified empty composer (not visible, no retained draft/evidence/diagnostic) or a newly opened detected interaction whose episode began after the key confirms submission; then record `accepted` and settle normally. Expiry records `uncertain/submit_unconfirmed`, stage `submitted`.

Keyboard input during verification reaches the child; a resulting nonempty composer can cause `submit_unconfirmed` and duplicate preview delivery (§6.5 Accepted trade-offs). A run without composer verification records `accepted` after the complete submit key, records every staged stop as `uncertain`, automatically re-delivers nothing and keeps the FIFO stopped until reconciliation. Chunk continuation and zero-byte selection still apply.

The literal chunk loop stops 250 ms before the attempt deadline. A complete prefix ends on a UTF-8 scalar boundary and outside paste markers. If a stop follows a complete opening marker but precedes the closing marker, a mandatory close stage writes `1b5b3230317e` within the reserve, even if the literal ends inside a scalar; the latter then records `uncertain/short_write`. A prefix inside the opening marker writes no close and records `uncertain/short_write`; a failed or short close records `uncertain/write_failed`. Outcome recording follows the close stage and § Stop cause.

#### Runner boundary and creation transaction

The internal, hidden `cafleet --database-url sqlite:///<canonical database path> __member-run --descriptor <ABSOLUTE_PATH>` command accepts only a descriptor allocated by `member create`, the public launch interface, and selects its database through that option rather than its environment. Its dispatch validates storage but bypasses ordinary command deadlines for its long-lived event loop.

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

The runner owns the child PTY and reconstructs terminal state independently of the multiplexer.

Store runtime artifacts in `.cafleet-runs/<database_namespace>/<run_id>/` beside the canonical selected database. The database identity is the canonical path, filesystem device/inode and the stored `runtime_database_identity` instance ID (§8); `database_namespace` is the first 26 characters of the unpadded RFC 4648 base32 encoding (alphabet `a`–`z`, `2`–`7`, most-significant bit first) of the SHA-256 digest of the canonical JSON (§6.4 JSON encoding) of the database identity object. A private namespace manifest stores the complete identity; a prefix collision with a different manifest fails explicitly rather than sharing artifacts.

Directories use `0700`, descriptor files `0600`, and a control socket is accessible only to its owner. Create each run directory exclusively and reconcile an existing directory before reuse. Reject symlinks, ownership mismatches, malformed descriptors and socket paths exceeding the platform limit against the complete resulting path before pane creation. The descriptor contains complete database identity, fleet/member/Director/run identities, ownership epoch, a random launch nonce, literal child argv, absolute cwd, required environment overrides and initial terminal dimensions. The child inherits the native configuration; CAFleet neither copies the entire environment nor creates per-member native profiles.

Replacing a database file creates a different filesystem identity and namespace even when restored bytes retain the instance ID. Runners validate the selected path's file/instance identity before input and storage mutations; replacement fences the old run from the new database, holds input and initiates owned-child cleanup. Retain the old namespace for explicit reconciliation; a new creator never adopts its socket.

The descriptor and control socket are local coordination mechanisms, not an authorization boundary against the same operating-system user; matching identities and the nonce reject stale endpoints. The creator's guarded cleanup deletes the descriptor on every outcome (success, handshake failure, timeout, rollback and uncertain post-`start` timeout) and reports a failed unlink explicitly. Failed run records retain only the non-secret `permission_argv_json`, never the assignment-bearing prompt element or raw environment. After a creator crash, deletion's recovery removes the descriptor only after identity/ownership validation. The runner removes its loaded descriptor after preparation when safe, independently of the creator's idempotent cleanup.

During verified ordinary recovery, retain the original ownership lock and pin a leftover launch descriptor by its opened file identity and bounded content, validating the recorded namespace/directory, private regular file, immutable database/fleet/member/Director/run identity and nonce. A positive historical descriptor epoch at or below the captured current epoch remains valid across takeover and retry; native launch loading still requires current creator ownership and the exact current epoch. Expected absence is idempotent while the pathname remains absent.

Before unlink, revalidate the fixed current run/placement, original lock/directory, named file type/permissions/device/inode/content and original deadline; report complete cleanup only after removing the pinned descriptor. Preserve a replacement or symlink with an explicit identity error; an unlink failure is reported explicitly and leaves recovery incomplete and retryable. Diagnostics exclude assignment and environment content. Exceptional database-only reconciliation leaves surviving unverified descriptors untouched.

Prepare connection setup waits, before any request write and within the original handshake deadline shared by all retries, only for an absent endpoint, a same-user socket awaiting mode 0600 or a transient bound-but-not-listening socket. A wrong type, owner or symlink, including a dangling symlink, fails explicitly; current-run health keeps strict endpoint checks. A possible prepare write, malformed reply or lost reply returns its explicit error or uncertainty without reconnecting.

Creation follows this ordered protocol:

1. Validate assignment input/placeholders, backend policy eligibility, runtime paths and backend/multiplexer prerequisites. Preserve one assignment substitution and one literal prompt argv element.
2. Register the pending member with its chosen slot and allocate its run row in one transaction (§6.2 Members). Write the private descriptor and create the pane in that slot (§6.5 Fleet window layout) with the absolute current CAFleet executable and the internal runner argv.
3. The runner validates the descriptor, acquires the precreated identity-pinned ownership lock, records its process identity, opens the PTY, initializes terminal state and event machinery, and binds its control endpoint. It reports `prepared` without spawning the native CLI.
4. The creator connects, validates the nonce/run identity, publishes the concrete placement with `status=prepared` and only then sends the control `start` frame. Publication patches the pane ID, the spawned window ID and the window name together in one update conditioned on a null `mux_pane_id`, which must change exactly one row. The runner revalidates placement, spawns the child and records `running` before reporting `started` with child PID/start identity.
5. Return creation success only after `started`, meaning child spawn succeeded without a native-policy/liveness guarantee; include the run identity and detection coverage with the member result. A child that immediately exits is reported by `runner_exited`, including its native status and Director notification. Use a ten-second monotonic handshake budget and preserve the rollback/cleanup diagnostics of any phase that fails.

Until `start`, the runner exits if its creator disconnects or the handshake expires. Failure after child spawn kills and reaps that owned child, closes the known pane, deregisters the pending member and retains a failed run record. A failed cleanup retains the original failure plus explicit orphan/pane diagnostics. A timeout after `start` is an uncertain launch: compensation inspects the existing run and stops its owned child, never retrying with a second child. Registration and pane ownership remain guarded until the handshake is complete.

#### Terminal transport and lifecycle

The runner relays terminal input and child output as bytes, with the outer terminal in raw mode, restoring its settings on every exit path. Preserve interactive control sequences, UTF-8 fragments, bracketed paste and alternate-screen transitions. Size the PTY from the outer terminal, propagate resize to the PTY and the screen model (§ Screen model facts), and preserve the native child's cwd, intended environment and exit status. Child bytes and runner diagnostics use separate logical channels, so diagnostics never become child input or detector evidence.

Forward outer-terminal input so a burst spanning several reads reaches the child together. One read of the raw-mode outer terminal returns at most the line discipline's 4096-byte buffer, and a longer terminal reply that reaches the child a tick late can be parsed as keys:

| Concern | Contract |
|---|---|
| Read | Each tick repeats the non-blocking outer read until it would block, reaches EOF or fills the 65536-byte input queue |
| Write | The tick then writes the whole queue to the PTY master in one write; unaccepted bytes stay queued, in order, for the next tick |
| EOF | Bytes read before EOF are forwarded before outer EOF is reported |
| Pacing | A tick that moved input bytes, by reading outer bytes or writing a positive count to the PTY master, is followed immediately by the next tick; every other tick, including one whose queued input the master did not accept, keeps the loop's idle sleep |
| Framing | None: forwarded bytes pass unchanged and in order, with no holding or timeout. Keyboard bytes advance `human_generation` and complete pointer reports advance `pointer_generation` (§ Terminal reply classification); replies advance neither |

The PTY-reader path updates terminal state before publishing the same bytes to display and passes immutable observations to the classifier. Track cursor/cell state, primary/alternate screens, OSC title and a monotonic output sequence. Keep scrollback out of live-prompt matching. Terminal queries and replies continue through the terminal transport, and the screen model never answers a terminal query itself; every reply comes from the outer terminal.

Terminal transport, classification and event storage are separate: database waits, Director receiver loss and event-commit retries never stop draining child output or accepting a human's native answer. Use bounded byte buffers and ordinary terminal backpressure for a slow outer display; a broken outer terminal triggers owned-child cleanup. Classification may coalesce intermediate working-screen snapshots, but it processes every potential interaction transition from the ordered parser stream before such coalescing. This is an observation contract for output actually rendered/read, not a guarantee to capture a dialog opened and closed between observable states.

| Condition | Required lifecycle behavior |
|---|---|
| Native child exits | Record its actual code/signal immediately, then apply the shutdown clock below and close the episode with `process_exited`. Restore the outer terminal independently of storage completion. |
| Runner receives termination or outer EOF/HUP | Apply the shutdown clock below and record the real termination cause. |
| Runner fails while child is live | Mark observation degraded and apply the shutdown clock below; return failure rather than a fabricated native success. |
| Runner is killed without cleanup | Closing the PTY may cause native hangup, but descendants may survive. Treat the run as unreachable until exclusive recovery establishes loss; inspect verified PID/start identity before signaling. |
| Member/fleet deletion | Acquire the pane-input locks in the order §6.3 `member delete` and `fleet delete` specify, revalidate placement/run, request runner shutdown and finish pane/registry cleanup as those commands specify. Report any unconfirmed child cleanup explicitly. |
| Restart or replacement | Create a fresh member/run after explicit reconciliation. Keep earlier episodes/events; a process generation never inherits an earlier pending request or causes automatic command re-execution. |

One shutdown clock starts at normal native-child exit, runner failure or termination request, and every wait shares its absolute deadlines:

| Boundary | Action |
|---|---|
| Start | Stop programmatic input and send TERM to verified surviving descendants in the owned process group. During the first two seconds, drain available PTY bytes. |
| 2 s | KILL verified survivors and reap, and close the master even if a descendant retains the slave. |
| 3 s | Nonblocking outer-display flushing ends; discard only undelivered display-tail bytes, recording their count and a truncation diagnostic. |
| 4 s | Event-worker SQLite attempts end, including transaction completion and retry delays, with no additional grace budget. Record any uncommitted transitions as an unsaved in-memory gap. |

Run every runner SQLite attempt under a bounded database budget from the remaining absolute step/shutdown deadline (§ Storage pressure and durability boundary): each attempt is bounded by its own deadline, and after each attempt the connection's default busy timeout and handlers are in effect again.

Restore terminal settings and release ownership after stopping/joining the writer, regardless of EOF, display or database completion. Retain actual child code/signal separately from `cleanup_state=complete|incomplete` and bounded diagnostic reasons for truncated output, pending events or unconfirmed descendants. When cleanup/restoration is complete, return the native status (`128 + signal` for a signal exit). With incomplete cleanup, preserve a native nonzero return; use runner exit `1` if the native status was zero or unavailable, and report its distinct recorded native status. Hard-kill or restoration failure remains an explicit host-recovery condition.

Maintain liveness counters only in process and expose them through `health`; persist `last_health_at` on lifecycle transitions and explicit input/recovery checks, not as a per-second heartbeat. An old recorded timestamp is historical evidence, not a staleness threshold. `member show` returns the recorded fields without a multiplexer and adds a bounded live health probe whose failure it reports without failing (§6.3 `member show`). Input dispatch requires a fresh matching health/ownership check. A failed probe holds input but closes no interaction, changes no terminal status and allocates no event sequence.

The creator precreates `owner.lock`, records its device/inode and runtime-directory identity, and records the current owner PID/start identity. Every ownership transfer advances `owner_epoch` with a CAS, including creator-to-runner transfer after the runner verifies/acquires that inode; the runner's `prepared` reply reports its new epoch for subsequent control frames. The runner holds the ownership lock through every writer/input thread and bounded shutdown. Validate the open descriptor's identity against both registry and current pathname; reopening a replacement lock is not acquisition of the original lock. A deleted directory/lock or changed inode requires the distinct missing-path recovery branch below.

Normal recovery acquires the identity-matching lock before a CAS epoch takeover. Every event mutation validates complete database identity, owner kind and epoch; input additionally validates its ownership/lock identities immediately before child writes. A delayed live runner retaining the original lock prevents normal takeover. The missing-path branch also requires verified original-owner termination before fencing; missing files alone establish no exclusive ownership or safe input. Every takeover records the recovering process identity and advances the epoch, rejecting stale writes/input.

Only the current lock/epoch owner allocates event sequences. Confirmed loss and its events are produced by the recovery procedure below after exclusive takeover, ordered after every committed event. Concurrent reconcilers contend for the same lock and CAS rather than allocating competing loss events. PID/start identity assists child reconciliation; it never substitutes for ownership.

##### Screen model facts

The runner reconstructs the child's screen as a grid of rows × columns of cells and offers detection a snapshot of it after each step. A cell holds one base scalar plus any following zero-width scalars. A wide scalar (width 2; East Asian Ambiguous scalars are width 1) occupies a leading cell and a continuation cell, and writing it resets the continuation cell to the default background and style. Each row records whether its text continued onto the next row by automatic wrap. After a write into the last column the cursor reports column `cols` (pending wrap). A cell's colours are default, one of 256 indexed colours (SGR 30–37/40–47 and 90–97/100–107 denote indexed colours 0–15, equal to their `38;5;n`/`48;5;n` forms) or RGB, and its style includes faint. The model tracks the primary and alternate screens, the OSC window title and the bracketed-paste mode (DEC private mode 2004), resizes with the PTY, and never answers a terminal query itself.

##### Terminal reply classification

The runner classifies every positive outer-terminal read for input accounting: classification forwards every byte unchanged and retains only parse state and pending counts. `human_generation` counts bytes resolved as keyboard/manual input; `pointer_generation` counts complete pointer-report bytes. Terminal replies advance neither counter, and tick pacing counts every byte read.

Every byte resolves exactly once: as manual input, pointer input or a complete reply, with unresolved prefixes pending until completion or expiry. A sequence is a terminal reply only when it is complete and matches one allowlist row. Reply control prefixes are 7-bit; `ESC` is `0x1b`, `CSI` is `ESC [`, `ST` is `ESC \` and `BEL` is `0x07`, and spaces in the byte forms are for readability only. Two productions are shared by every row:

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

Outside a human bracketed paste, complete SGR reports `CSI < L(3,3) M` and `CSI < L(3,3) m`, and X10 reports `CSI M Cb Cx Cy` whose three trailing bytes are each `0x20`–`0xFF`, are pointer input. A parameterless/intermediateless CSI ending in `M` enters `x10` and consumes exactly three bytes; an invalid byte below `0x20` or expiry resolves pending bytes as manual input. Every other input counts as manual:

| Input | Classification |
|---|---|
| Printable text, UTF-8, C0 controls (Enter, Tab, Ctrl-keys), DEL | Manual immediately |
| Any CSI outside the allowlist: arrow/function keys, `CSI L(1,2) ~`, kitty key events `CSI L(1,3) u` | Manual on its final byte |
| Incomplete or malformed SGR/X10 reports | Manual on invalid continuation or pending-prefix expiry |
| Bracketed paste `CSI 200 ~` … `CSI 201 ~`, including every byte between the markers | Manual, even when the pasted body contains allowlisted forms |
| SS3 `ESC O x` and Alt-modified `ESC x` for any `x` other than `[`, `]`, `P`, `_` | Manual once the second byte arrives |
| SOS `ESC X` and PM `ESC ^` strings | Manual through the Alt-modified row; terminals send no SOS/PM replies |
| 8-bit C1 bytes (`0x9b`, `0x9d`, `0x90`, `0x9f`) | Manual, as ordinary bytes in UTF-8 mode |
| A CSI whose parameter and intermediate bytes exceed 64 | Manual at the byte that exceeds the limit |
| An OSC, DCS or APC body that exceeds 4096 bytes | Manual at the byte that exceeds the limit, so a larger reply such as an OSC 52 clipboard read counts as manual |

The classifier is in exactly one state: ground, escape (after `ESC`), CSI, x10, string body (OSC, DCS or APC), string escape (after `ESC` inside a body) or paste.

| State | Valid continuation bytes | Completes on |
|---|---|---|
| Escape | `[` → CSI; `]`, `P`, `_` → string body | — |
| CSI | Parameter bytes `0x30`–`0x3F`, then intermediate bytes `0x20`–`0x2F` | Final byte `0x40`–`0x7E`; allowlisted reply, complete SGR pointer report, entry to X10, or manual input |
| X10 | Exactly three bytes in `0x20`–`0xFF` | The third byte completes a pointer report |
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

The paste state follows a manual `CSI 200 ~`: every byte counts as manual, and `CSI 201 ~` counts as manual and returns to ground. A sequence split across reads stays pending, within one tick or across ticks.

| Consumer | Keyboard generation | Pointer generation |
|---|---|---|
| Ordinary preview/prompt/exec drain proof | yes | no |
| Permission token minting/revocation, gated answer stages and permission drain proof | yes | yes |
| `member press` accounting | yes | — |

An ordinary staged write reads neither generation after its first byte: keyboard and pointer input during it leave the write running (§ Stop cause).

A pending prefix resolves as manual input once 100 milliseconds pass since its last byte:

- The runner resolves expired prefixes at the start of each tick's outer read and of every drain round, before reading, and advances `human_generation` by the resolved manual count.
- While a prefix is pending, the transport is not quiescent: a drain is *pending* with blocker `terminal reply pending`, an earlier drain proof fails validation with `input transport is no longer quiescent`, and no programmatic stage, permission answer or deferred write proceeds.
- Outer EOF resolves a pending prefix as manual before EOF is reported.
- A lone Escape key is pending for at most 100 milliseconds and then counts as manual.
- The bound runs inside the deadline of the write attempt it delays (the runner's five-second ordinary-input attempt budget for a deferred write, or the invoking command's five-second deadline for a prompt, exec or permission answer), adding no deadline of its own.
- Each new byte of a prefix restarts the bound; the 64-byte CSI and 4096-byte string body limits cap how long a continuously fed prefix stays pending.

Human input whose bytes exactly equal an allowlisted reply and arrive within the bound counts as a reply. The realistic member of this accepted collision class is a modified F3 key (`CSI 1 ; Pmod R`), matching the cursor-position-report form; the others need an exact multi-key sequence typed within 100 milliseconds: Alt-`[` then `I`/`O` (a focus report), Alt-`]` … Ctrl-G (an OSC reply), and Alt-Shift-P or Alt-`_` … Alt-`\` (a DCS or APC reply). A colliding key still reaches the child unchanged; it only fails to interrupt a staged delivery or revoke automatic authority.

#### Classification and episode identity

Each detector consumes a reconstructed screen, title, cursor/input-region evidence, output sequence and known backend version, and returns a typed observation with its rule identifier, matched region, extracted visible operation/choices, a normalized semantic fingerprint and a confidence/evidence designation. Rules compile at startup, and an invalid rule definition fails. Versioned profiles are bundled locally in CAFleet, never loaded from remote rules or herdr manifests.

| Classification | Evidence needed | Meaning |
|---|---|---|
| `permission_request` | A backend-specific live permission heading/operation and its choice/input region | An observed request awaiting a native decision; title text alone is insufficient. |
| `permission_continuation` | A tested screen the native agent shows for a pending request after one of its choices is selected, such as a rejection-reason dialog | A continuation of the active permission episode; it never opens an episode (§ Continuation screens). |
| `question` | A tested general input/question form | Human input is required without establishing a tool-permission request. |
| `unknown_interaction` | A live interactive form without enough evidence for a more precise type | Queue ordinary programmatic input with zero bytes and request inspection. |
| `composer_empty` / `unknown` | The corresponding tested composer rule, or lack of one | Input eligibility is evaluated separately; absence of permission text is not an empty composer. |

The persisted episode classes are exactly the §8 `member_interactions.classification` values; every other classification creates no interaction row. Each frame has exactly one of these primary observations.

A title such as `Action Required`, a quoted approval transcript, a transcript viewer and an old permission line cannot independently create `permission_request`. Profiles must bind controls to the same live region, distinguish general questions/startup forms, and remain explicit about wrapping, layout changes and competing rules. Conflicting strongest classifications yield `unknown_interaction` and preserve the screen. A run with no selected profile, or whose profile declares no detection, advertises unknown coverage; missing matches never imply that all requests were observed.

An episode is a CAFleet observation identity, `(run_id, episode_seq)`, not a native request ID. Allocate `episode_seq` monotonically within a run when a stable live interaction first appears. A candidate is *stable* after two compatible observations at least 100 ms apart, rechecked after 100 ms even if no bytes arrive; an episode opens only for a stable candidate. Gate programmatic input immediately on the first candidate, before this notification debounce.

The fingerprint contains backend, interaction class, visible operation and stable choice labels. Exclude spinner/counter/cursor decoration. The visible operation includes its displayed row breaks, so a resize that rewraps the operation changes the fingerprint and replaces the episode. Redraws and resize matching the fingerprint keep the episode, its stored evidence and any issued request token without another opening notification. A different operation/choice/class candidate holds input immediately and replaces the active episode only once stable; the old episode stays open until then. An incompatible candidate restarts stability, while return to the original fingerprint cancels the pending replacement. After a verified native noninteraction boundary, an identical request gets a new episode. If two native requests are visually indistinguishable with no observable boundary, preserve one episode and report that identity limitation; never claim one-to-one native request coverage.

| Episode transition | Event and interpretation |
|---|---|
| Stable candidate appears | `interaction_opened`; record its classification and observed evidence. |
| Partial redraw or temporary overlay | Keep the episode open, gate input and update local visibility to `obscured`; emit no event. |
| Verified noninteraction boundary stable for 250 ms | `interaction_closed` with `not_visible`; record the observation rather than inferring a decision. |
| Different interaction replaces the active form | Once the replacement is stable, close with `superseded` and open the next episode in order. |
| Child exits or exclusive recovery confirms observer loss | Close with `process_exited` or `monitor_lost`; a suspect/unreachable observer leaves the episode open and approval outcome remains unknown. |
| Continuation screen of the active permission episode | Keep the episode open and `visible` with ordinary input queued for the interaction. Emit no event and keep the stored request evidence. |

The interaction keeps the evidence of its opening event; the closing frame's evidence appears only in the `interaction_closed` event payload.

Keep decision outcome (`approved`, `denied`, `cancelled`) and operation outcome separate from episode visibility. Screen disappearance, Enter, Escape, a timeout, a message's consumption and resumed animation establish none of those outcomes by themselves. Machine decision outcome is `unknown` unless explicitly documented native evidence establishes it; human/Director reports remain attributed reports, not authenticated native events.

##### Continuation screens

A `permission_continuation` rule declares `continues_rule_id`, the permission rule it continues, and `continues_choice_id`, the choice whose selection opens it. Showing no operation, it cannot identify a request by itself; episode continuity and succession bind it to one:

| Concern | Contract |
|---|---|
| Episode | While a permission episode of its `continues_rule_id` is active, a continuation observation keeps that episode open. Without such an active episode it is an `unknown` screen. |
| Request binding | Inside a response, a continuation qualifies for the next stage only as the first fresh quiescent observation after the complete write of the stage gated on its `continues_choice_id`, with the episode, `human_generation`, `pointer_generation` and runner ownership unchanged and no observation gap or degradation in between. The runner then rebinds the in-flight selection to the continuation's rule id and match fingerprint within the same episode. |
| Inspection | `permission inspect` on a continuation offers no request token or answer capability, so a continuation reached by human input is completed manually. |

#### Persistence, notification and read interfaces

The baseline creates the runtime records with the broker tables (§8). The running CLI never opens a file of another cafleet version for writes, migrates, renames or deletes it, and shows none of its fleets, messages or runtime history.

The §8 DDL fixes the runtime tables' exact columns, types, enums, nullability, foreign keys, uniqueness and partial unique indexes, and typed conversion enforces the same enums. Terminal `member_runs.status` values are `exited`, `failed` and `lost`. Nullable fields represent genuinely unavailable evidence or phases not reached. A missing required identity, invalid enum, malformed payload or cross-run interaction reference fails the write/read explicitly. Member history is soft-deregistered; runs, interactions, runtime events and input operations are retained for the lifetime of the database file. Timestamps use UTC RFC 3339, and ordering uses the run/event sequences rather than wall-clock uniqueness.

| Record | Semantics beyond the DDL |
|---|---|
| `member_runs` | `owner_epoch` starts at 1. `runtime_path` is canonical, each start identity is a verified platform-specific process-start token, and `recovery_origin_status` preserves the original lifecycle status while recovering. |
| `member_interactions` | `evidence_json` and `evidence_digest` are the opening event's evidence. |
| `member_runtime_events` | `payload_json` is the final event payload, and unique `(run_id, event_seq)` supports idempotent retries. |
| `member_input_operations` | `ready` certifies a zero-byte attempt, and a ready row with deferred metadata is a runner-queued ordinary input; `stop_cause` is a value from § Stop cause. Settled rows are retained as replay-rejection records for the lifetime of the database file. |
| `member_runner_diagnostics` | One row per run, source and sanitized message, with its repeat `count` and first/last times; each run keeps its newest 256 rows (§ Storage pressure and durability boundary). |

Readers return existing records and their associated message bytes unchanged on replay. `cleanup_state` begins `incomplete` and becomes complete only after successful shutdown cleanup, so a live/crashed run makes no prior completion claim.

`coverage` is `tested` when the profile selected for the run's backend and native version declares detection, and `unknown` otherwise (§6.9 Profile selection by minimum version); it describes the selected profile's declared detection, not universal prompt detection. `launch_nonce` is a random 32-byte value encoded as hex and stored with the run so later same-user control clients can present it; it prevents accidental reuse of stale endpoints and is not a privileged authorization credential. Validate member/fleet/Director ownership against broker records within each mutation transaction; inbox inserts additionally require their active/live status, while archived recovery validates historical ownership.

Runtime event kinds are the §8 `member_runtime_events.kind` values. The event payload has this shape:

```json
{
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
  "decision_outcome": "unknown",
  "evidence": {
    "backend": "codex",
    "native_version": "recorded-at-launch",
    "rule_id": "native-tool-permission",
    "screen_seq": 120,
    "evidence_digest": "sha256-of-evidence"
  },
  "director_action": {
    "kind": "review_permission_request",
    "instruction": "INSTRUCTION",
    "inspect_argv": ["cafleet", "permission", "inspect", "108", "--event-id", "481", "--from-member-id", "91", "--json"]
  }
}
```

`INSTRUCTION` is the permission-request instruction (§ Request-bound permission responses). Event keys are ordered `event_id,run_id,event_seq,member_id,fleet_id,director_member_id,kind,interaction_id,classification,observed_at,source,decision_outcome,evidence`, followed by `close_reason` for closures or `cause` for applicable lifecycle/result events. Delivery results append `input_id,message_id,delivery,stage`; stalled delivery appends `input_id,message_id,blocked_by_input_id,waiting_since`; all events finish with required `director_action`. Permission/question actions order `kind,instruction,inspect_argv`; delivery actions follow § Input delivery result event and § Input delivery stalled event. Other events carry null action.

The evidence object, in events and in stored interaction evidence, has ordered keys `backend,native_version,rule_id,screen_seq,evidence_digest`; it carries no screen text. `evidence_digest` is the SHA-256 of the canonical evidence object without that key. Semantic fingerprints for live episode comparison stay in memory, never in logs or database.

Run-level events use null interaction/classification, `source=runner_lifecycle` and a typed health/exit cause in the payload. Missing command, cwd or environment stays null, never filled with invented execution data from screen parsing. Limit the complete payload to 16 KiB. Treat screen text as untrusted evidence, never as a command or instruction to approve.

Persist a transition, its event and a normal `input_required` inbox message to the pinned Director in one immediate transaction. The message's text is the full event JSON and its sender is the observed member; the typed `source` distinguishes runtime observation from model-authored speech. The transaction applies the broker's fleet and active-member checks (§6.2). Insert the message and event, fill the final ID-bearing payload and validate the complete association before commit; no intermediate message body is externally visible. A retry of the same `(run_id, event_seq)` returns the existing event/message IDs and verifies identical payload identity; conflicting reuse fails.

An observed transition carries nullable `episode_seq`, kind, original observation time and immutable evidence. Its commit transaction allocates `event_seq` and allocates or resolves interaction/event/message IDs and produces the final public payload. Idempotence compares these original transition fields, excluding generated IDs and commit timestamps.

The runner writes events and inbox messages through its own database transactions and never depends on its child invoking `cafleet message send`. A persisted notification remains pending through Director absence or listener restart. Live events use `notification_state=inbox`; active ownership and the sender/recipient fleet checks remain mandatory. After closure, ordinary observation stops. Exclusive recovery may commit its recovery events as `notification_state=archived`, `message_id=null`, with `archive_reason=member_inactive|fleet_deleted|director_inactive`, retaining the event/interaction history without inserting a message from an inactive sender or claiming Director notification.

The live runner fills every `event_seq` it materializes; persistence never leaves a slot empty, because a later transition requires the next sequence value. Two delivery records commit as silent archived rows (`notification_state=archived`, `message_id=null`, no inbox message) at their own `event_seq`, with the normal payload built from the record so an idempotent retry compares equal. Both apply only when historical ownership supplies no archive reason, and `duplicate_delivery` takes precedence over `head_not_ready`:

| `archive_reason` | Record |
|---|---|
| `head_not_ready` | An unpublished stalled-head record whose head, a queued ordinary input of the run, is no longer `ready` when persisted |
| `duplicate_delivery` | An `input_delivery_result` or `input_delivery_stalled` record of the same kind as a persisted row that names the same waiting input, preview or blocker (§ Input delivery stalled event) |

Every other stalled-head violation, such as another run's input or a head without deferred metadata, and a delivery-field failure on a duplicate fail the transition permanently and take the rejection path (§ Storage pressure and durability boundary).

**Rejection event.** A transition the runner can never persist is replaced at its own `event_seq` by a run-level `runner_degraded` event with null `episode_seq` and classification, `observed_at` the wall-clock time of the rejection, the run's lifecycle evidence (`rule_id` `runner-lifecycle`, the rejected transition's `screen_seq`), null `director_action` and the cause:

```json
{"kind":"event_rejected","rejected_kind":"interaction_opened","rejected_observed_at":"2026-11-21T12:04:08Z","error":"MESSAGE"}
```

The cause has exactly the keys `kind,rejected_kind,rejected_observed_at,error`, and `event_rejected` is valid only on `runner_degraded`. `rejected_kind` is a runtime event kind, `rejected_observed_at` is the rejected transition's RFC 3339 observation time, and `error` is the failure text under the runner diagnostic text rule (§ Storage pressure and durability boundary). A rejected `interaction_opened` creates no interaction, so a later `interaction_closed` for that episode is rejected with its own `event_rejected`; the native request stays visible on the member's surface.

`event show` returns the complete durable event with its notification/archive state and current interaction/run state, and `input show` returns a durable programmatic-input receipt (§ CLI presentation contract). Runtime events use the ordinary broker message envelope and poll-time consumption.

##### Input delivery result event

The runner emits one `input_delivery_result` event to the run's Director for a deferred prompt or exec input's first submitted, not-submitted or uncertain result, for every immediate prompt or exec stopped staged write (`uncertain`), and for an uncertain preview (below). It creates no interaction row and carries no literal payload. Immediate submitted results remain in command output and receipts only, and `input show` reports every receipt.

| Field | Value |
|---|---|
| `kind` | `input_delivery_result` |
| `interaction_id`, `classification` | `null` |
| `source` | `runner_lifecycle` |
| `decision_outcome` | `"unknown"` |
| `evidence` | The run's ordered evidence object with null screen-derived fields |
| `cause` | `{"kind":STOP_CAUSE}` whenever a stop cause is recorded (§ Stop cause), absent otherwise; for prompt and exec, live and recovery publishers derive it from the durable receipt |
| `input_id` | The prompt or exec input's ID, or null for a preview |
| `message_id` | The preview's message ID, or null for prompt and exec input |
| `delivery` | The ordered `status,method,reason` projection of the stored receipt (§ Delivery reporting); for a preview, `{"status":"uncertain","method":METHOD,"reason":null}` |
| `stage` | `zero_bytes`, `partial`, `submitted` or `unknown` |
| `director_action` | `null` |

For prompt and exec, the terminal receipt is the durable source: publication follows commit, recovery publishes missing first results, and run/input identity deduplicates through shutdown and recovery. Shutdown publishes pending results before `runner_exited`; event listen/poll injects no Director-pane input.

**Uncertain preview.** A sweep preview attempt that stops, or whose submission is not confirmed, is uncertain. The runner keeps its stage and cause in memory and publishes one event with `input_id` null, `message_id` the message, `delivery` `{"status":"uncertain","method":METHOD,"reason":null}`, `stage` the written stage, `cause` its stop cause and `director_action` null, deduplicated by run plus `message_id`. On a verified-composer run the message stays unnotified and the sweep types it again once the composer is verified empty; on a run without composer verification the runner stamps `notified_at` first and then publishes the event, so the preview is not typed again. The live runner is the event's only publisher: recovery publishes no preview result, so an event lost to a runner crash between stamp and publication is covered by the run's `runner_lost` event, and the message itself stays in the recipient's inbox for `message poll`, since a preview only notifies. An uncertain preview never stops the FIFO; the FIFO stop (§ Runner-owned deferred delivery) covers only prompt and exec.

##### Input delivery stalled event

The runner publishes one `input_delivery_stalled` event when input has waited the stall bound (3 minutes) without a delivery attempt, in three cases:

| Waiting input | Wait starts | Deduplicated by |
|---|---|---|
| A queued prompt or exec head that has started no attempt | When the entry becomes the head; an interaction or an uncertain blocker that holds the head clears the wait, and its end restarts it | Run plus `input_id` |
| The sweep's selected unnotified preview, held because the composer is not verified empty on a verified-composer run | When the sweep first holds that preview on the composer; an interaction that blocks it clears the wait and its closure restarts it | Run plus `message_id` |
| A queued head or a pending preview held behind the run's lowest-ID `uncertain` prompt or exec (the blocker) | When queue housekeeping first sees that blocker with a waiter; it clears when no waiter remains or the blocker settles | Run plus `blocked_by_input_id` |

Time an open interaction blocks the input never counts: the `interaction_opened` event already notifies the Director with its own action. Queue housekeeping (§ Queue housekeeping) publishes the head and blocked events and the sweep (§ Runner pending-preview sweep) publishes the preview event. A restarted wait can stall again for the same input, preview or blocker; that repeat commits as a silent `duplicate_delivery` archived row (§ Persistence, notification and read interfaces), so the Director receives one event per key. The event creates no interaction and stores no literal:

| Field | Value |
|---|---|
| `kind` | `input_delivery_stalled` |
| `interaction_id`, `classification` | `null` |
| `source` | `runner_lifecycle` |
| `decision_outcome` | `"unknown"` |
| `evidence` | Ordered run evidence with null screen-derived fields |
| `input_id` | The waiting head's input ID; `null` otherwise |
| `message_id` | The waiting preview's message ID; `null` otherwise |
| `blocked_by_input_id` | The blocker's input ID; `null` otherwise |
| `waiting_since` | UTC RFC 3339 start of the wait |
| `director_action` | `review_stalled_delivery` for a head or preview, `reconcile_uncertain_input` for a blocker (below) |

```json
{"input_id": null, "message_id": 5120, "blocked_by_input_id": null,
 "waiting_since": "2026-11-19T12:04:08Z",
 "director_action": {"kind": "review_stalled_delivery", "instruction": "...",
   "inspect_argv": ["cafleet", "member", "capture", "91", "--json"]}}
```

The runner's stalled record has exactly the keys `kind,input_id,message_id,blocked_by_input_id,waiting_since`, and persistence requires exactly one of the three IDs to be a positive integer and the others null. A head names a queued ordinary input of the run; when it is no longer `ready` at first persistence, the record commits as a silent `head_not_ready` archived row. A preview names a message addressed to the run's member. A blocker names a `prompt` or `exec` input of the run in any state, so a reconciliation racing the publication cannot reject it; re-validation of a published event applies the same rule.

A head or preview event's `director_action` is `{"kind":"review_stalled_delivery","instruction":INSTRUCTION,"inspect_argv":["cafleet","member","capture","MEMBER_ID","--json"]}`, where `MEMBER_ID` is the run's member, and INSTRUCTION is exactly:

```text
Capture the member's pane. When the composer holds text, submit or clear it with member press keys the capture establishes, or ask the user to clear or submit it on the member's native surface.
```

A blocker event's `director_action` is `{"kind":"reconcile_uncertain_input","instruction":INSTRUCTION,"inspect_argv":["cafleet","input","show","BLOCKER_ID","--json"]}`, where `BLOCKER_ID` is `blocked_by_input_id`, and INSTRUCTION is exactly:

```text
Inspect the uncertain input and the member's pane, then settle the input with input reconcile from that evidence. Queued input and message previews resume once it settles.
```

A verified empty composer resumes delivery on the next queue turn or sweep, and a settled blocker resumes queued input and previews. Every argv array gets `--database-url URL` appended for a non-default database. A composer that stays unverified after clearing indicates a profile composer-rule defect; each later input can stall in turn until corrected. Orderly shutdown and recovery preserve deduplication of head events; the live runner is the only publisher of preview events.

#### CLI presentation contract

§6.3 CLI *Runtime presentation* and *Runtime event and input commands* fix the exact keys, order, nullability and text layouts of the runtime outputs of member create/show, `event show`, `input show`/`reconcile` and wrapped prompt/exec. Printed submission or settlement establishes no native execution result.

#### Storage pressure and durability boundary

An event worker owns SQLite. The parser/control loop submits immutable transitions through a bounded channel of 64 records. The worker commits each transition in its own ownership-checked immediate transaction, which allocates its `event_seq` as the next value after the run's latest committed event; no intermediate file holds it. Each worker step, receiving a transition and then persisting it with its ownership validation and transaction, is bounded by five seconds of real time, equal to the connection's busy timeout, and by the run's shutdown deadline. Lowering the shutdown deadline interrupts an in-flight database statement within 1,000 SQLite VM steps or one 10 ms busy-wait slice, and an expired step fails as a deadline. At shutdown the runner waits for the worker until 50 ms past the shutdown deadline and then joins it, so no transition commits after the writer's unsaved count is taken. Retry transient failures with bounded exponential delays from 100 ms to two seconds rather than adding them to the connection's default timeout. Keep terminal transport and socket health service independent.

Every failed step is classified by its cause alone:

| Class | Causes | Outcome |
|---|---|---|
| Transient | `SQLITE_BUSY`, `SQLITE_LOCKED`, an expired step bound, and the I/O-class codes `IOERR`, `FULL`, `CANTOPEN`, `NOMEM`, `READONLY` and `PROTOCOL` | Retry the same transition after the backoff |
| Permanent | Every other cause: transition content, a missing referenced row, a constraint or trigger refusal, a sequence-slot conflict, lost live ownership, a corrupt file | Replace the transition with its rejection event (§ Persistence, notification and read interfaces) |

A full channel or a transient failure holds all programmatic input, records the writer's failure text and retains the failed transition at the front, plus an explicit observation-gap counter. Stop further semantic classification rather than silently discarding events, continuing manual terminal I/O, and resume with a fresh screen and an explicit gap event after the writer recovers. `runner_degraded` carries the gap interval/count, and events potentially missed during that gap remain unverified.

A permanent failure is replaced within the same step: the rejection event takes the failed transition's `event_seq` and is persisted at once under a fresh step bound. A committed replacement records no writer failure text and opens no gap, and the worker continues with the next transition. A transient failure of the replacement keeps it at the front under the transient rule. A transition the parser/control loop cannot hand off because its evidence is invalid is replaced the same way before hand-off. When the replacement itself fails permanently, as a sequence-slot conflict, lost live ownership or a corrupt file does, the writer fences:

- it discards its queue, counting the discarded transitions as unsaved;
- it keeps the failure text `event writer fenced: <cause>`;
- it accepts no further transitions;
- the observation gap stays open, so input stays held.

A fenced run recovers only through `member delete` and a replacement member; member show and doctor report the open gap (§6.3 Member runtime checks).

Durability begins at a successful SQLite commit, and notification is claimed only after it. A hard runner crash loses the transitions that had not committed; recovery records the run's loss with `runner_lost`, which identifies that coverage gap. While the database stays unavailable, the control health response, the runner diagnostics and the local diagnostic preserve the actual error, and memory stays bounded.

**Diagnostic text.** Every runner diagnostic text that is stored or transmitted (the `event_rejected` cause's `error`, the health observation's `writer_diagnostic` and `member_runner_diagnostics.message`) has each control character U+0000–U+001F and U+007F replaced with a space, is trimmed and is truncated to at most 512 bytes at a UTF-8 boundary. Each text starts with a fixed non-empty label and is an error text; it carries no input literal, message body or pane text.

**Runner diagnostics.** The live runner stores its diagnostics in `member_runner_diagnostics` (§8), one row per run, source and message, and keeps printing each to its standard error:

| `source` | Recorded diagnostic |
|---|---|
| `event_writer` | A writer failure text that differs from the current one, a rejection, a fence, a buffer overflow |
| `observation` | An observation gap opening, as `observation gap opened: <code>` with the observation failure code |
| `deferred_input` | `member runtime: deferred input: …` |
| `queue_housekeeping` | `member runtime: queue housekeeping: …` |
| `preview_sweep` | `member runtime: preview sweep: …` |
| `input_result` | `member runtime: input … result is pending storage: …` and `member runtime: input … result is pending recovery: …` |
| `permission_answer` | `member runtime: permission answer for interaction … is uncertain: …` |
| `shutdown` | Each `member runtime: …` shutdown outcome diagnostic |

The runner buffers at most 32 entries keyed by source and message, each with its count and first and last times; an overflow drops the oldest entry and adds its count to a dropped counter. The event worker flushes the buffer before each loop iteration, idle iterations included, when the buffer is non-empty or the dropped counter is positive and the next flush time has arrived. Each flush is one ownership-validated immediate transaction bounded by 500 ms of real time and by the shutdown deadline, and runs this write sequence:

1. Upsert each entry: a new row, or for an existing `(run_id, source, message)` row a `last_at` update and the entry's count added to `count`.
2. When entries were dropped, upsert one `event_writer` row `diagnostic buffer overflow: N entries dropped`.
3. Delete the run's rows beyond the newest 256 by `last_at`, then `diagnostic_id`.

| Flush outcome | Effect |
|---|---|
| Committed | Clears the flushed entries and the dropped counter; the next flush may run at once |
| Transient failure | Keeps the buffer and retries one second later |
| Permanent failure | Drops the flushed entries, adds their count to the dropped counter and retries one second later |

A flush never records a writer failure text, so diagnostics cannot open an observation gap. After the event writer finishes, shutdown writes the remaining buffer, the dropped counter and one `shutdown` entry per shutdown outcome diagnostic with the same write sequence, in the transaction that records `cleanup_state` and `cleanup_diagnostics_json`. When that transaction fails, the printed lines are the only copy, and the `cleanup evidence pending` diagnostic reports it. Recovery writes no runner diagnostics.

Member and fleet deletion run recovery on the recorded run after successful runner shutdown and before deregistration when the remaining deadline permits (§6.3 `member delete`); otherwise they retain the namespace, and a later deletion retries. Recovery never starts an agent. A five-second deadline bounds each recovery pass. Start one cleanup clock at entry to the common recovery coordinator, before snapshot, locking and process/health evidence; its two/three/four-second boundaries share that origin and are clipped to the deletion caller's deadline. An incomplete pass retains the `recovering` state for the next pass. A run that is already terminal needs no work: recovery preserves existing status and diagnostics and advances no ownership, even if the recorded current reconciler remains alive.

Recovery follows this sequence:

1. Validate complete database/run identity, acquire the member's pane-input lock when a concrete placement remains, then open/verify the recorded runtime directory and lock inode without creating a replacement. A matching held lock returns `live_or_unreachable`. A creator-owned run still in startup also requires creator/pinned-process reconciliation even if its precreated lock is free. A missing path, changed inode or unregistered startup owner follows the special branch below, not a free-lock result.
2. After acquiring the matching ownership lock, CAS the run's owner epoch to the next integer, record the recovering PID/start identity, set `owner_kind=reconciler`, preserve `recovery_origin_status` on the first pass and set `status=recovering`. A later pass takes a fresh epoch and retains the same original status. This fences prior event producers and input clients.
3. Under that exclusive ownership, derive the terminal outcome from the original status and the durable terminal records. If neither establishes an exit/failure/loss, close a pending interaction with `monitor_lost` and then commit `runner_lost`; otherwise finish the recorded terminal outcome without inventing a loss. Repeated passes detect existing closure/loss records instead of regenerating them. Settle every deferred `ready` receipt as `not_submitted` with reason `runner_lost`, and publish each durable first deferred or immediate-stopped result that has no `input_delivery_result` event yet, deduplicated by run and input (§ Input delivery result event). Choose inbox or archived insertion from current member/fleet/Director status within each transaction.
4. Persist the final `lost`, `exited` or `failed` status, ended/cleanup evidence and notification disposition, then release ownership. Reconcile verified descendants under the bounded cleanup rules; unknown ownership remains a diagnostic. Retain archived events permanently under database retention.

Recovery and deletion count a recorded process as alive only while its PID/start identity matches and it has not exited. A process that has exited but is not yet reaped (state `Z`; macOS `SZOMB`) is terminated evidence for the run owner's process evidence, for deletion's wait for a `hermes` runner to exit after its work settlement, and for recovery's descendant-group checks, which count only non-zombie group members. Closing a herdr pane by hand kills the pane shell and reparents its runner, which shuts down, exits and stays a zombie when the reparenting process never reaps; recovery settles that run like any exited runner, so `member delete` and `fleet delete` complete.

For a missing directory/lock or identity mismatch, a recorded owner still alive under its PID/start identity, or an owner whose process identity cannot be read, returns `live_or_unreachable` and preserves the recorded run status and the open interaction. An owner may hold an unlinked original lock, so creating/acquiring a new pathname supplies no death proof; `member delete` resolves such a runner by force-closing its pane, after which recovery observes a terminated owner. Startup without a registered runner requires reconciling the creator, pinned pane and any started process before treating ownership as stopped.

Once the recorded owner's termination is machine-verified, perform database-only reconciliation in one bounded immediate transaction, rereading the fixed database/run identity, placement, epoch and owner tuple after obtaining the SQLite write lock. A completed terminal result uses the no-work return above; a concurrent live reconciler on unfinished recovery causes deferral. For unfinished recovery, advance the epoch and record the recovering process identity, increment the gap count once and add `runtime_path_missing` to cleanup diagnostics.

Derive the terminal outcome from durable terminal records or the preserved original terminal status, retaining the exact recorded exit code, the code/signal cause of any recorded lifecycle event and existing event/message bytes. When a terminal outcome is established, finish that outcome with no new lifecycle event. Otherwise close any persisted interaction with `monitor_lost` before `runner_lost`, and finish as `lost`. In the same transaction, settle every deferred `ready` receipt as `not_submitted` with reason `runner_lost`, then publish each durable first deferred or immediate-stopped result that has no `input_delivery_result` event yet, deduplicated by run and input (§ Input delivery result event). Choose inbox or archived insertion from current lifecycle state within the same transaction. This branch leaves surviving mismatched artifacts untouched; missing observations remain an explicit gap. Commit the terminal outcome and all new bookkeeping/records atomically. A failed transaction leaves the original state for retry.

#### Input arbitration and the original native answer

For a runner-managed member, every `member prompt` and `member exec` goes through the pinned runner control socket instead of multiplexer pane input, and the runner's own sweep types previews (§ Runner pending-preview sweep). The client validates and submits without holding the outer pane input lock while awaiting the runner. The runner owns that lock for wrapped writes: it acquires it nonblockingly from its event loop under the attempt deadline, revalidates the concrete placement and holds it across an active staged write, so explicit press, deletion and other programmatic writers serialize; it releases the lock while a request is only queued. Its own terminal-state loop serializes the final check and child writes.

Control frames are length-prefixed JSON with protocol version, run/nonce/owner epoch, operation key, kind and literal payload. A four-byte unsigned big-endian prefix encodes the serialized UTF-8 JSON body length; the inclusive 65536-byte (64 KiB) body limit excludes that prefix, and an oversized declared length fails immediately after the header is read. Requests and replies require the top-level integer `protocol_version: 1`; missing or unsupported versions and truncated frames fail explicitly. § Input control frames and deadlines lists the complete set of request kinds and `receipt` actions. Submission replies distinguish `accepted`, `queued`, `held`, `rejected` and `uncertain`, carrying a nullable input ID, output-sequence evidence, byte/submission stage and the delivery projection; the ID is null for a zero-byte attempt with no durable row, and `queued` preserves zero-byte stage evidence. Conflicting reuse of an operation key with another payload digest fails; store no literal prompt/command in the receipt table.

##### Input control frames and deadlines

New input, permission and receipt requests require top-level `protocol_version,kind,identity,expires_at_mono_ns`. `identity` has exactly `database_identity_json,fleet_id,member_id,director_member_id,run_id,owner_epoch,launch_nonce`, binding the complete selected database and recorded owner tuple. Submissions validate recorded database/run/nonce and historical associations before durable key lookup; current epoch/owner and active eligibility checks apply only to a new or live ready attempt. IDs/epoch are positive signed 64-bit integers, sequences and byte counts unsigned 64-bit integers, and `expires_at_mono_ns` a positive unsigned 64-bit CLOCK_MONOTONIC nanosecond expiration. Each `kind` adds required fields:

| Kind | Additional exact fields |
|---|---|
| `submit_prompt` | `operation_key,input_kind,literal`; input_kind is prompt. |
| `submit_exec` | `operation_key,input_kind,literal`; input_kind is exec. |
| `permission_inspect` | `interaction_id,event_id,director_member_id`. |
| `permission_respond` | `interaction_id,event_id,director_member_id,expected_run_id,expected_owner_epoch,request_token` and exactly one of `decision` and `choice`. |
| `receipt`, action `query` | `action,operation_key`. |
| `receipt`, action `retire` | `action,input_id`. |
| `receipt`, action `reconcile` | `action,input_id,expected_state,disposition,evidence_ref`. |

`literal` is the unchanged prompt/command text; the exec plan supplies its own `! ` prefix. Require nonblank operation keys, validate the ordinary single-line/control-character contract and enforce matching outer kind/input_kind. Permission IDs/expected epoch must agree with identity and the stored association. Reconcile is the Director-owned input coordinator's private action; its enum values match the durable state/disposition domains. Reject unknown, duplicate, missing or wrongly typed fields before effects. Lifecycle requests carry `protocol_version`, `kind` and `identity` and no `expires_at_mono_ns`: the handshake's `prepare` adds `remaining_ms` and `start` adds `pane_id`, and `health` and `shutdown` add nothing.

Replies retain exactly `protocol_version` plus one of `result` or textual `error`. Input results have exactly `kind` plus the following field: submission/`submission`, inspection/`request`, permission_response/`response`, or receipt/`receipt`. Submission contains exactly `status,input_id,stage,output_seq,reason,delivery`, where `delivery` is the ordered `status,method,reason` projection (§ Delivery reporting); permission response and inspection carry the complete ordered public objects fixed by the CLI contract. A receipt object has exactly `status,input_id,submission`: missing has null ID/submission; present has a positive ID and submission object; retired or settled has a positive ID and null submission. Result kind must match the requested operation. Clients validate and reload durable input rows from their supplied database connection; socket receipt metadata never replaces durable identity.

The original invocation captures one expiration and carries it through framing, transit, queues, SQL and every native stage. Convert using three samples taken in order: the process's internal monotonic clock, `CLOCK_MONOTONIC`, and the internal monotonic clock again, validating sample order and overflow-checked arithmetic. Sender expiration adds the remaining original deadline measured at the third sample to the `CLOCK_MONOTONIC` sample. Receiver deadline adds the remaining expiration measured at the `CLOCK_MONOTONIC` sample to the first sample. Scheduling delay can shorten this budget and grants no extension; use no wall clock. Clock failure, overflow or expiration fails before new reservation/native bytes while preserving prior committed or possibly transmitted state.

The service also caps new-operation admission at five seconds from the first observed frame-header byte; delayed framing and queueing consume that cap. Client exchange, lock acquisition, supplied-connection SQL scopes and stage advancement retain their earlier original/inherited deadline. Validate the declared frame size (above) before allocation and the complete serialized size before sending.

##### Bounded input draining

Before a programmatic input attempt, drain actual human and child input through the runner's manual queue, terminal parser, ordered observer and outer display. A drain ends *pending* or *quiescent* and reports cumulative `human_generation`, `pointer_generation` (§ Terminal reply classification) and terminal `output_seq`. The drain proof records both counters: ordinary input validation compares the keyboard generation, and permission validation compares both. Quiescence describes transport and observation health; the input coordinator separately validates composer/request eligibility, ownership, reservation and the original deadline before a write.

Each call executes at most eight rounds under the original deadline. A round attempts at most one human read, one manual-queue PTY write, one child read, one display-queue write and one display flush, in that order. Skip an operation only for unavailable capacity or remaining quota, an empty queue or a previously observed end of file on the stream. Let B be 16384 bytes, the per-drain transfer quota; each actual read buffer is bounded by the minimum of B, the remaining stream quota and available queue capacity.

| Work per drain call | Inclusive bound |
|---|---|
| Human reads, child reads, manual PTY writes, display writes | Eight calls and B positive bytes for each; child bytes include previously deferred child bytes. |
| Display flushes | Eight calls. |
| Final observation-health samples | One sample, taken only for a candidate completed quiet round. |
| Rounds | Eight; unresolved work leaves the drain *pending*. |

A *quiescent* drain requires both actual reads to report `EAGAIN`/`EWOULDBLOCK` in the same completed round, an empty manual queue, consumed deferred child bytes, successful parser/observer processing and positive free display capacity. The final health sample must be present, nondegraded and have `pending_count=0`. An absent health adapter supplies no proof; an observer that is healthy but holds input eligibility with unknown visibility may establish physical quiescence while remaining ineligible for programmatic input. A *quiescent* drain also requires no pending terminal-reply prefix: each round first resolves expired prefixes, and a round that would otherwise certify quiescence while a prefix is pending leaves the drain *pending* with blocker `terminal reply pending` and installs no proof. The eighth round may certify these facts; quota exhaustion, a skipped read, `EINTR` or end of file leaves it *pending* instead. A continuously readable source terminates at the round or byte bound. Deferred child bytes precede subsequent reads and consume the same child quota.

In both ordinary progress and input draining, classify every actual positive human read immediately and advance keyboard and pointer counts with checked cumulative unsigned 64-bit arithmetic; the counts and literal queued bytes survive a later PTY, child, display, parser or observer failure. The service delivers both observed before/after deltas to authority invalidation after successful and failed progress alike, before propagating any error; a simultaneous notification failure accompanies the primary error. Nonrecoverable errors propagate, and a zero-length write fails with `terminal write made no progress`. A read interrupted with `EINTR` requires a later fresh read pair; end of file records the stream state and leaves shutdown to the normal service path.

Clear stored drain proof at entry, check the original deadline before I/O and before each endpoint/callback, and recheck after completed work, retaining the bytes/counts of successful completed I/O before reporting expiry. In-memory bookkeeping and observation of a completed read may finish after expiry; an observed expiration permits no later endpoint call or proof installation. Synchronous I/O and callbacks have no preemption guarantee, so finite work bounds and clock checks impose no hard syscall or callback latency cap. Every call keeps the original deadline, with no internal sleep or deadline renewal.

Retain every completed child read for display and preserve the exact prefix processed by the terminal parser and ordered observer. When a deadline expiry or an observer error interrupts the observation of a positive child chunk, the transport retains the suffix the parser has not consumed as a pending observation remainder; the fed prefix is never replayed and no byte is dropped. The next drain round or ordinary progress feeds that remainder before any new child read, under its own deadline and the child byte quota, and observation becomes complete again once the remainder is fed with a successful observer callback. Until then drains continue bounded manual and display progress and stay *pending* with blocker `observation incomplete` and no proof. A terminal parser failure leaves no resumable boundary, so its observation-incomplete fence persists for the lifetime of that runner's terminal transport. Fully completed observation remains complete when only a later deadline or display operation fails.

Only a successful final check installs an opaque, single-use proof bound to its transport instance, drain turn and actual health/generation/sequence facts. The visible result fields describe evidence and grant no write authority. Taking the stored proof consumes it once, and validating it while a terminal-reply prefix is pending fails with `input transport is no longer quiescent`. Ordinary progress, resize, prefetch, display mutation/discard and a new drain invalidate it before their validation or I/O, including later error paths; pure generation/snapshot reads preserve it. A new transport begins with generation 0 and no proof. Programmatic writing consumes actual coordinator-owned authority, never a caller-constructed result.

##### Durable input receipts and settlement

A held/rejected zero-byte attempt creates no side-effect receipt; a reserved row returns to `ready` only after positively recording zero bytes. Conflicting key/digest reuse fails.

An ordinary `receipt query` reads the durable operation for its fixed run/key and reports its durable status with the last real retained stage and output sequence. `missing` requires an absent durable row and absent matching cache entry. A durable row remains `present` after cache retirement; a cache entry without its durable reservation is an identity error. Validate the retained immutable input identity before using its stage. The receipt projection table in § Delivery reporting maps each durable receipt to its `submission.status` and its stage without a cache entry. With a matching retained entry, return its actual stage and output sequence alongside that durable status; otherwise the output sequence is null. Query preserves pending writers, clients, permits, receipts and native bytes. Its results describe observations: new write authority and zero-byte retirement proof still require their separate validated authorizations. A settled `not_submitted` or `abandoned_uncertain` disposition alone supplies no observed byte stage; its absent-cache result remains `unknown` and leaves the original transmission uncertainty intact.

Before the first child byte, commit `in_progress` to `member_input_operations` under the current ownership epoch and bounded database budget. A failed commit permits zero child bytes. After writing, persist `accepted` or `uncertain` before promising that outcome; when that persistence fails, the runner records the row `uncertain/write_failed` (§ Immediate attempt failure bookkeeping) and returns uncertainty, and the row stays in progress only when that record also fails. The live runner may reconcile retained stage evidence into the same row. A stopped or unconfirmed prompt or exec records `uncertain` with its stop cause (§ Stop cause), exits 1 and is retried only by a deliberate action; the runner sends no second submit key and leaves composer clearing to the human or Director.

Retire only the in-memory receipt after validating durable settlement, the current full run/owner and original run lock, exclusion of every active writer, and absence of a held write authorization or available attempt permit for that input. Cache absence is idempotent only after the same durable and ownership checks. For prompt/exec, the durable accepted input row itself is the queryable submission record: the creator or runner can settle it as submitted and retire the cache after verifying that stored row. Printed stdout only presents its input ID and supplies no durable acknowledgement. Settled operation keys/digests/dispositions persist for the lifetime of the database file, so a delayed request still returns the prior disposition and never types again.

Bound unretired side-effect receipts, including queued deferred inputs, to 4096: while that unresolved set is full, new side-effect attempts fail with an explicit zero-byte capacity error; settled receipts leave the set. Keep uncertain receipts until the Director records an evidence-based disposition through `input reconcile` (§6.3 Runtime event and input commands), which settles the key and permits cache retirement. For prompt/exec, a consumed not-submitted key requires a new deliberate operation key.

Reconciliation runs under the original deadline clipped to any inherited SQL bound, and every transaction it commits is a supplied-connection immediate transaction. It pins the original input row in its supplied-connection snapshot: `input_id`, `run_id`, `operation_key`, `kind`, `payload_digest`, `created_at` and expected state remain fixed through live reconciliation and settlement. For a recorded concrete placement, acquire its genuine pane/input lock and hold that same lock through current-owner synchronization. Synchronize with the pinned runner's receipt handler so an active byte-writing operation cannot be settled concurrently. An unreachable writer leaves the receipt uncertain, and the Director records `abandoned-uncertain` through `input reconcile`. Compare-and-swap the original expected operation state in the settlement transaction; a changed row requires a separate invocation. Settlement records the observed input disposition, not the native command's execution outcome.

For a receipt already settled with the requested disposition, reconciliation returns the unchanged validated durable row and its original `evidence_ref` before live locks, health probes or exchange. A conflicting disposition fails with the row unchanged. This historical return grants no new input authority. For a live nonsettled receipt, validate the full fixed identity and original ownership lock inside the transaction of the expected-state settlement CAS, after the SQL wait. After a new reconciliation, require the matching `settled` acknowledgment and a validated durable reread, then exactly one `retire` exchange with a matching input ID under the same held pane lock and original expiration. A post-commit failure retains the known input context and original cause while the durable settlement remains authoritative. A changed expected state requires a separately reconciled attempt.

Reconciliation accepts the fixed placement state from that original snapshot. A present placement requires its matching, continuously held pane/input lock. An absent placement (a deregistered member or a deleted fleet) has no live runner, so reconciliation fails with `receipt reconciliation requires the member's live runner; the input stays uncertain` and the receipt stays uncertain. Revalidate the exact placement, complete fixed run/input identity and historical relationships inside the settlement transaction after preceding waits. A changed placement, identity or expected input state invalidates the attempt.

A durable ready row is positive zero-byte evidence only under actual writer exclusion and exact associations. Owner death or an attributed not-submitted disposition alone supplies no such proof; recovered in-progress, accepted and uncertain records keep their transmission uncertainty. Proven-zero settlement commits within the same proof lifetime.

Live input authorization distinguishes native-write authority and receipt-only writer exclusion. A zero-byte proof is valid only while the writer exclusion it was taken under is still held and only on the runner thread that holds it; it binds the complete immutable input and current ownership, and only the input coordinator can create one. Receipt-only exclusion authorizes bookkeeping without a write permit or current visible prompt. With a retained entry, proof requires its actual zero-byte stage and cumulative count zero; without one, only a validated durable ready row under genuine writer exclusion supplies it. A projected retry stage or a settled not-submitted label supplies no replacement observation.

A process loss, ownership change or unavailable receipt preserves durable uncertainty. Explicit prompt/exec use generated keys and printed input IDs and require a separate deliberate action.

Native-exec success means submission, not execution success. A pending message keeps `notified_at` unset until the sweep stamps it (§ Runner pending-preview sweep); a consuming poll (§6.3 `message poll`) removes it from preview eligibility. Queueing supplies no new permission event and never closes the native request.

Screen observation cannot create an atomic native request API: unread child output may race with a write, an unknown CLI can redraw unexpectedly, and a person can type concurrently. The runner therefore guarantees zero ordinary-input bytes for detected and candidate interactions and refuses uncertain states; tested composer profiles bound the remaining race for profile delivery without asserting universal prompt preservation. Release evidence must include this race and preserve uncertainty instead of authorizing a second submission.

Human keystrokes from the outer terminal pass to the child as native input, including the human's intentional approval, denial or cancellation. A Director may submit a request-bound allow or deny through the permission-response path below after applying its session decision policy; previews stay pending and prompt/exec input stays queued with zero bytes during the request. Keep attributed decisions, key submission, native outcomes and actual continuation separately observable. Unknown answer capability or uncertain request identity preserves manual original-surface operation.

##### Delivery method and interaction priority

These ordered rules select the delivery of every sweep preview, `member prompt` and `member exec`; where a rule queues input, a preview stays pending instead and the sweep retries it (§ Runner pending-preview sweep):

1. Validate target/run identity, reachability, writer ownership, storage and receipt history. An uncertain prior attempt under this operation's own key is an operational failure; while another prompt or exec of this run is uncertain, the FIFO is stopped (§ Runner-owned deferred delivery). Unknown detection coverage is valid capability absence.
2. A candidate or open detected interaction queues ordinary input with zero bytes. An obscured interaction remains preserved, and `permission_request`, `question` and `unknown_interaction` are all blocking interactions.
3. A verified empty composer with the operation's executable plan uses profile delivery without Escape (§ Measured submission plans and settle intervals). This includes a working member whose composer row is empty: the native CLI queues the submitted input behind its current turn without interrupting it.
4. With an executable plan and a busy composer, queue with zero bytes under `composer_busy` and drain by profile delivery when verified empty. On a verified-composer run every preview and queued input waits for a verified empty composer.
5. Otherwise use delivery fallback: Escape, settle 500 ms, literal prefix and text, settle 500 ms, Enter (`0d`). This covers missing operation plans, missing profile coverage and failed version probes.

| Selection | Bytes | `delivery.method` | Selection reason |
|---|---|---|---|
| Queued behind an interaction, composer, earlier input or insufficient attempt budget | Zero | `null` until a later attempt selects one | `interaction_open`, `interaction_candidate`, `composer_busy`, `earlier_input` or `delivery_budget_exhausted` |
| Profile delivery | Literal stage, 500 ms settle, measured submit key | `profile` | `null` |
| Fallback | Escape, settle, literal stage, settle, `0d` | `fallback` | `submission_plan_unavailable` (rule 5) |

The Director interrupts a working member explicitly with `cafleet member press MEMBER_ID --from-member-id DIRECTOR_ID Escape`; ordinary delivery sends a member whose profile has an executable plan no Escape.

Fallback runs through the owned child PTY and the same input coordinator and receipts and preserves the literal and exec `! ` prefix. Revalidate the higher-priority interaction rule before Escape and every later stage: a newly detected interaction before the first byte requeues input; one after a positive write follows § Stop cause, including a stop during the first fallback settle after Escape.

Fallback may interrupt work, mix text with a draft, and dismiss an undetected permission request or question on an unknown-coverage run (§6.5 Accepted trade-offs). The guarantee is suppression for detected and candidate interactions, not detection of every native form. Semantic permission answers continue to require their independently tested request-bound plans and never use fallback. Human keystrokes and request-bound permission responses keep their separate paths and can resolve the interaction that blocks ordinary input; closing that interaction grants no permission and supplies no native execution result.

##### Runner-owned deferred delivery

The runner owns a FIFO of prompt and exec requests; previews stay outside it (§ Runner pending-preview sweep). It accepts a blocked request into the queue only after allocating its durable input ID and committing a `ready` receipt with `deferred_at` and `defer_reason` under its operation key, digest and target identity. The literal or control payload stays in runner memory; receipt tables, result events and diagnostics retain metadata only.

Only one ordinary request writes at a time, in enqueue order, and a request queued behind another receives the same explicit queued result. Queue processing is scheduled from the runner event loop:

| Trigger | Runner action |
|---|---|
| `interaction_closed` after its event transaction commits | Drain pending output and revalidate the current state; a superseding interaction keeps the queue blocked, and closure due to exit or monitor loss dispatches nothing |
| A candidate clears | Retry the queued head after rechecking the current interaction state |
| The active input completes | Retry the queued head after rechecking the current interaction state |
| A temporary zero-byte obstacle during an active attempt | Retry on a 100-millisecond timer within the same five-second attempt deadline |

No external send or Director wake is required to resume the accepted queue. Queue housekeeping (§ Queue housekeeping) cancels the head whose target is removed, independently of the queue turn, and each attempt also rechecks the head's target immediately before its first byte.

A queued request outlives the originating CLI connection and command deadline. Each actual drain attempt has its own five-second budget, and waiting for an interaction consumes none of it. A client timeout after queue acceptance reports the known input ID and directs the caller to inspect the receipt instead of generating a new operation key. The timer retry runs even without child output or broker commands, preserving the input ID, FIFO position and `ready` receipt without renewing the deadline. A reappearing candidate or open interaction pauses dispatch and ends the zero-byte attempt; the next closure or clearance starts another bounded attempt.

| Deferred-attempt condition | Receipt and queue outcome | Result publication |
|---|---|---|
| Busy pane lock or temporary storage/observer unavailability before the deadline, with zero bytes | Retain the queued head and retry on the timer | Input inspection remains `queued` with its original defer reason |
| Five-second deadline expires with zero bytes | Settle `not_submitted` with reason `pane_input_busy`, `storage_unavailable`, `observer_unavailable` or `delivery_budget_exhausted`, according to the last obstacle; clear the unused selected method, and advance after the terminal transaction commits | `input_delivery_result` |
| Target/run/placement identity fails revalidation, with zero bytes | Settle `not_submitted` with reason `target_inactive` or `target_changed` immediately and advance after commit | The result with its reason and diagnostic context |
| Queued prompt/exec exceeds its plain limit with no safe framing | Settle `not_submitted/plain_limit_exceeded` with zero bytes and advance | `input_delivery_result` |
| A prompt or exec attempt records `uncertain` | Keep the FIFO stopped until `input reconcile` settles it | The uncertain result |

**FIFO stop.** On every run, with or without composer verification, after a prompt or exec records `uncertain` the FIFO stays stopped, and no queued input and no sweep preview is written, until `input reconcile` settles that input. No later head can therefore type into a composer that may still hold the stopped text.

A head that has started no attempt within 3 minutes of becoming the head publishes one `input_delivery_stalled` event (§ Input delivery stalled event). Time an interaction blocks the head does not count. Each head is deduplicated by run/input identity through orderly shutdown and recovery.

If storage prevents the terminal transaction, discard the zero-byte payload and retain a metadata-only pending finalization at the FIFO head. Retry that transaction on the 100 ms timer without sending bytes or starting another input; commit the receipt when storage recovers, then advance, and its result event follows from that durable receipt. While publication is unavailable, write a runner-stderr diagnostic naming the input ID and persistence failure; the Director receives the durable result after recovery. Ownership loss transfers finalization to fenced recovery. Accepted requests become queued, submitted, not submitted or uncertain; `held` describes work the runner did not accept.

Orderly shutdown cancels queued zero-byte requests as `settled/not_submitted` with reason `runner_stopped`. After runner loss, recovery may settle a deferred `ready` receipt as not submitted with `runner_lost` once ownership fencing establishes its payload and writer are gone. Explicit payloads remain memory-only and require a new deliberate request. A pending preview stays unnotified, and a new run's sweep delivers it (§ Runner pending-preview sweep).

##### Immediate attempt failure bookkeeping

Every path that ends an immediate `prompt` or `exec` attempt after its first child byte records the receipt `uncertain` with `stop_cause=write_failed` before the runner replies to the client. This covers an error while advancing the attempt's stages, including an expired deadline, and an input-drain failure while the runner completes the in-flight attempt. When the caller's deadline has passed, the bookkeeping transaction uses the deferred finalization interval (1 s) from now as its storage deadline. A zero-byte attempt returns to `ready`. The uncertain receipt stops the FIFO (§ Runner-owned deferred delivery). A sweep preview that fails after its first child byte is an uncertain preview (§ Input delivery result event).

##### Queue housekeeping

Queue housekeeping runs from the runner event loop every housekeeping interval (1 s) while ordinary input is not fenced. Each pass first runs the blocked-input watch below; the sweep that follows runs only while the FIFO is non-empty. It runs before the in-flight client attempt is driven and independently of the queue turn, the input drain and the finalization retry, so none of their gates holds it. It writes no PTY byte and takes no pane lock. The 1 s cadence bounds target-removal detection latency to about one second, at one sweep read transaction per second per queued member, plus the blocked-input watch's read transaction on every pass.

Each sweep:

1. Opens one read transaction that loads every queued entry's receipt state and delivery method, the head's target inputs (run identity, member activity and placement), and the lowest-ID other `in_progress` prompt or exec receipt of the run.
2. Acts on each entry in FIFO order:

   | Durable receipt state | Action |
   |---|---|
   | `ready` | Keep the entry |
   | Any other state | Fail with `queued input INPUT_ID changed to STATE outside the runner queue`, fencing ordinary input as a permanent invariant violation |

3. Applies the head's target result: a removed or changed target settles the head `not_submitted/target_inactive` or `target_changed` and removes it.
4. Runs the stall watchdog (§ Input delivery stalled event).

**Blocked-input watch.** In its own read transaction, separate from the sweep's, the watch reads the run's blocker, the lowest-ID `prompt` or `exec` input in state `uncertain`, and whether a waiter exists: a non-empty FIFO, or a pending preview the sweep would select (§ Runner pending-preview sweep), independent of the sweep's cadence and backoff. A storage failure skips the watch for that pass and keeps its wait unchanged.

| Blocker | Waiter | Effect |
|---|---|---|
| None | — | Clears the blocked wait |
| Present | None | Clears the blocked wait, because the waiting input was delivered, consumed or settled elsewhere |
| Present | Present | Keeps the wait for that blocker, or starts it now when no wait exists or the wait names another blocker |

A kept wait that has lasted the stall bound publishes one blocked `input_delivery_stalled` event (§ Input delivery stalled event). While a blocked wait exists, the head's stall watch is cleared as it is for an interaction, so the head publishes no `review_stalled_delivery` event; once the blocker settles, the head's watch starts afresh.

A transient storage failure (SQLite busy/locked contention or an expired storage deadline) on the read leaves every entry unchanged. A sweep first retries a pending zero-byte finalization; a cancellation that storage prevents becomes that pending finalization, and the sweep settles nothing further until it commits.

Every receipt write for a queued entry is made by the runner itself, so a non-`ready` queued receipt is an invariant violation. The queue turn writes a non-fatal attempt error as a runner-stderr diagnostic. The receipt `reconcile` action that settles a queued entry removes it from the FIFO in the same call and publishes nothing. It also accepts a queued `ready` prompt or exec with disposition `not_submitted` and settles it with zero bytes, which is how the Director cancels a queued prompt or exec.

##### Runner pending-preview sweep

Each ordinary member's runner delivers that member's own unnotified pending messages with no caller command; `message send` only stores the message (§6.3 `message send`).

| Property | Contract |
|---|---|
| Cadence | From the runner event loop, at most once per preview sweep interval (1 s), and not before the run-wide backoff expires |
| Preconditions | The run is live and its native child is running; no ordinary attempt is active and no sweep attempt awaits its result; the runner FIFO is empty and not stopped |
| Selection | One short read snapshot confirms that the run's member is an active ordinary member whose recorded run is this run with a concrete placement, then takes the oldest `to_member_id=<member> AND status_state='input_required' AND notified_at IS NULL` row by `(created_at, message_id)` |
| Order | Strict oldest first: the sweep types only the selected message and waits for its outcome before it selects again |

Stamped self-sends are excluded by `notified_at`, and a consumed message is never selected. A selected preview held on the composer for the stall bound publishes one `input_delivery_stalled` event (§ Input delivery stalled event). A pending preview held behind an `uncertain` prompt or exec counts as a waiter for queue housekeeping's blocked-input watch, which publishes the blocked event (§ Queue housekeeping). Because the sweep starts a preview only when the FIFO is empty and not stopped and no ordinary attempt is active, a queued prompt or exec always precedes a sweep-selected preview.

**Delivery state.** A preview's delivery state is `messages.notified_at` plus the runner's in-memory record of a preview typed and awaiting verification; it holds no durable input receipt. On a verified-composer run the sweep types a preview only into a verified empty composer and stamps `notified_at` after confirmed submission; a preview whose submission is not confirmed is uncertain (§ Input delivery result event), stays unnotified and is typed again once the composer is verified empty. On a run without composer verification the sweep types by § Delivery method and interaction priority and stamps `notified_at` after its complete stages; an uncertain preview is stamped first and then published, so it is not typed again.

**Submission.** The sweep acquires no lock beforehand. It hands the canonical preview to the runner-internal input entry point that serves `member prompt`, built with the run's own control identity, which applies § Delivery method and interaction priority; a preview that cannot be written now stays pending for a later tick. For actual stages the runner acquires the pane input lock nonblockingly from its event loop (§6.5 Pane input lock).

**Clientless tracking.** A sweep request has no control-socket client. The runner tracks a sweep attempt separately from client-owned attempts, holding its attempt deadline:

- When the entry point leaves an immediate sweep attempt awaiting its result, the runner records it as the pending sweep attempt. The completion path that drives client-owned attempts drives it to its outcome without replying to any client, feeding that outcome to the backoff.
- An outcome returned at once is handled the same way, and no pending sweep attempt is recorded.
- An accepted immediate attempt requires that no client-owned attempt and no sweep attempt is awaiting its result. A control-socket request arriving while a sweep attempt awaits its result is handled exactly as one arriving while a client-owned attempt is in flight.
- Orderly shutdown ends a pending sweep attempt with no reply; a preview it wrote bytes for is an uncertain preview.

**Run-wide backoff.** A sweep attempt that cannot write because the pane input lock is busy, storage or the observer is unavailable, or its attempt budget is insufficient sets a memory-only run-wide backoff: 1 s, doubling, capped at 30 s. A submitted preview, an `interaction_closed` or a candidate clearing resets it to zero. The message stays at the head, so strict oldest-first order is preserved.

**Lifecycle.** Orderly shutdown and runner loss stop the sweep. A recovered or new run sweeps under its own run key. A member with no live runner keeps its messages pending.

##### Delivery reporting

`member prompt` and `member exec` JSON append a `delivery` object with ordered `status,method,reason` (§6.3 Runtime presentation). `method` is `profile`, `fallback`, or null before selection. The shared projection is:

| Status | Meaning | CLI behavior |
|---|---|---|
| `submitted` | Submission confirmed on a verified-composer run, or complete stages written on another run | Exit 0; exec has `dispatched=true` |
| `queued` | The runner accepted durable zero-byte deferred work | Exit 0; exec has `dispatched=false`; the input ID and reason are returned |
| `held` | No queue acceptance or write, due to unavailable health, ownership or storage | Prompt/exec exits 1 with its delivery context |
| `uncertain` | Some stage may have written; reconciliation is required | Prompt/exec exit 1 with input identity, recorded `stop_cause` (§ Stop cause) and method |
| `not_submitted` | Work was cancelled with established zero-byte evidence | Visible through input inspection and, for deferred work, its result event |

Self-sends stay excluded from automatic preview selection (§6.3 `message send`).

Preview forms, body folding, inline limits and long-body pointers follow §6.5 Push notifications.

Prompt and exec inputs use the SHA-256 over the canonical JSON (§6.4 JSON encoding) of the object with keys `run_id`, `kind` and `literal`, where `literal` is the submitted prompt or command text. A prompt preimage is:

```json
{"kind":"prompt","literal":"PROMPT_TEXT","run_id":42}
```

Method, appended clause, framing, inline/pointer rendering, native prefix/key bytes and timing are outside the digest preimage. The runner selects and renders afresh only while zero bytes are established; once a stage writes, its method and rendered bytes stay pinned through its result. Queries and duplicate requests validate canonical identity before current composer state, so a later method change preserves the digest and grants no replay of an existing key.

`defer_reason` records why a request first entered the queue and stays paired with its original `deferred_at`. `delivery_reason` records the eventual selected-method reason or a terminal zero-byte cancellation reason. The owned transaction that changes an ordinary receipt to `in_progress` commits its `delivery_method` and `delivery_reason` before the first byte: profile delivery has a null reason, and fallback has `submission_plan_unavailable`, also for an immediate fallback. A zero-byte requeue clears unexecuted selection metadata before any later reselection; a terminal zero-byte cancellation commits a null method and its outcome reason. After any possible positive write, the selected method and reason persist through accepted, uncertain and settled states.

Every result consumer projects these durable fields rather than recomputing a reason from the current catalog or composer. Receipt outcome and eventual method and reason commit together, and the deferred-result event follows from that durable receipt (§ Input delivery result event).

| Ordinary receipt state/disposition | `submission.status` | Stage without a cache entry | `delivery.status` | Method and reason |
|---|---|---|---|---|
| `ready` with deferred metadata | `queued` | `zero_bytes` | `queued` | Null method; original `defer_reason` |
| `ready` without deferred metadata | `held` | `zero_bytes` | `held` | No active queue ownership; null method and `zero_bytes_ready` |
| `in_progress` or `uncertain` | `uncertain` | `unknown` | `uncertain` | Recorded selection; no automatic dispatch or replay |
| `accepted` or `settled/submitted` | `accepted` | `submitted` | `submitted` | Recorded selection |
| `settled/not_submitted` | `rejected` | `unknown` | `not_submitted` | Null method and recorded outcome reason for a zero-byte cancellation; recorded selection for a reconciled attempt that may have written bytes |
| `settled/abandoned_uncertain` | `rejected` | `unknown` | `uncertain` | Recorded selection; abandonment ends retries without establishing zero bytes |

`submission.status` is the runner's outcome vocabulary for the attempt (§ Durable input receipts and settlement); `delivery.status` is the separate delivery projection. Every attempted ordinary receipt records its delivery method, and a stored attempted receipt without one fails as an invalid stored value. A not-submitted receipt whose method and reason were never recorded reports a null method and reason `method_unrecorded`; no method or cancellation reason is invented. Permission-response receipts expose `delivery:null` and null deferred metadata; their semantic decision/submission/settlement projection remains authoritative. Unknown stored combinations fail validation.

The stage vocabulary is `zero_bytes,partial,submitted,unknown`. A stopped or in-progress attempt reports `partial` for every written prefix short of the submit key, a complete literal and a lone fallback Escape included, and `submitted` once the submit key is written. Asynchronous completion leaves originating CLI output unchanged; `input show` and § Input delivery result event expose the outcome. Queue reasons include `delivery_budget_exhausted`, and terminal zero-byte reasons include `plain_limit_exceeded`.

###### Stop cause

A staged ordinary write starts only when no interaction is open or a candidate, the composer is verified empty on a verified-composer run, and the input drain is quiescent. It then writes the literal, settles, writes the submit key and, on a verified-composer run, verifies the submission (§ Measured submission plans and settle intervals). Only an interaction that appears during these stages, one that opens or becomes a candidate, stops the remaining stages. Keyboard and pointer input, terminal replies and observer changes leave the write running, and post-submit verification detects a literal that was not submitted.

An attempt that ends after its first byte without confirmed submission records `uncertain` with one `stop_cause` in the same transaction; storage refuses the transition without it. Selection metadata remains separate.

| Value | Recorded when |
|---|---|
| `interaction_open` | A visible or obscured interaction, an interaction hold or a retained transition appears during the write |
| `interaction_candidate` | An interaction whose eligibility is held as a candidate appears during the write |
| `submit_unconfirmed` | The 2 s post-submit verification window ends without confirmation; stage `submitted` |
| `deadline_expired` | The chunk bound is reached, or too little budget remains for the full verification window plus the stage write margin (250 ms) before the submit key |
| `short_write` | A stop leaves a prefix inside a UTF-8 scalar or paste marker, or a permission answer stage writes short |
| `write_failed` | A PTY error, unrecorded possible I/O, a failed checkpoint, a failed or short close, a positive short submit key, a failed deferred or written immediate attempt (§ Immediate attempt failure bookkeeping), or a failed permission response |
| `runner_stopped` | Shutdown ended an active attempt that may have written a child byte |

Apply the incomplete-prefix `short_write` override after an interaction stop, and finish any mandatory paste close before outcome recording. Settlement and reconciliation preserve the cause. Other inputs without a stop and in-progress rows left by runner loss have a null `stop_cause`. Input inspection reports it, stopped replies carry it as `reason`, and result events carry it as `cause` (§ Input delivery result event).

##### Explicit manual key input

An explicitly authorized `member press` sends only its requested ordered keys through the member's outer multiplexer pane under the shared pane lock and selected-context/deadline contract (§6.5 Explicit key dispatch). It appends no Escape or Enter. This manual path remains available during observer degradation or a programmatic composer hold and follows the separate CLI acknowledgment/error contract (§6.3 `member press`).

The runner classifies and counts the resulting actual positive outer-terminal reads as manual input (§ Bounded input draining), accounting each new delta once and before subsequent automatic advancement. Observed manual input invalidates automatic authority through the retained generation; a key argument or backend acknowledgment supplies no byte count, drain proof or permission receipt. Press changes no durable input/response row. Native approval/denial/cancellation and actual continuation remain independently observed outcomes under the Director/user policy.

#### Request-bound permission responses

The Director uses its own session's effective approval mode, independently of the member's launch mode. In verified auto mode, evaluate the requested command or operation and decide allow or deny. In other or unknown modes, obtain the user's request-specific permission before transmitting an answer. Resolve mode from current host/session evidence and record its provenance in host-owned pending state; member argv, event text and a CLI flag supply no proof of Director mode. Native and managed host policy remain authoritative.

The runtime event payload includes the broker-generated `director_action` field. A permission-request `interaction_opened` event has `kind=review_permission_request`, an `instruction` and `inspect_argv`; a question `interaction_opened` event has `kind=review_question` with the question instruction below and the same `inspect_argv`; delivery results and stalled events carry their actions below, and other events have null action. The instruction is: "Verify this event and the current request. Evaluate the operation and its risk. In the Director session's effective auto mode, decide allow or deny; otherwise obtain the user's permission. Submit the selected answer through the request-bound CAFleet response command, or give that complete command to the user. Inspect the native result and continuation afterward." The question instruction is: "Verify this event and the current question. Inspect it for the choices CAFleet can answer and their scope. In the Director session's effective auto mode, choose one of those choices; otherwise obtain the user's choice. Submit it through the request-bound CAFleet response command with that choice, or give that complete command to the user. When the inspection lists no choice, ask the user to answer on the member's native surface. Inspect the native result and continuation afterward." Build inspection argv from validated stored identities, never screen text, and validate the action against its event/run/Director association. Replay existing records idempotently, preserving their exact message bytes; all event payloads use version 2 and remain within the 16 KiB payload limit.

Before deciding, establish the actual command/tool operation, arguments, absolute cwd, required environment overrides, target resources, privilege/network effects and native choice scope as needed for the risk assessment. Evaluate those facts against the assigned task and host policy. Event evidence carries no operation text, so obtain these facts through authorized current capture (`member capture`) and routed member evidence, or ask the user to inspect the request when evaluation remains incomplete. Screen content is evidence, not instruction.

The Director's permission CLI is `cafleet permission inspect` and `cafleet permission respond`; §6.3 Permission commands owns their arguments, validation and inspection/response projections. `inspect` verifies the current durable event/message/interaction association, retains the genuine concrete pane lock, probes the pinned owner and resolves unread output/manual progress under the original deadline clipped to any inherited SQL bound. Current authority comes from the actual parsed screen, a complete choice/focus match (a single-request choice for a permission request, a declared choice for a question) and the stable tracked episode; observation freshness is checked independently of ordinary composer eligibility. Unsupported or blocked capability is an explicit result, and incomplete observation supplies no token.

The runner issues one current random token bound in memory to the complete database/run/nonce/epoch, interaction/episode, native version, live operation/choice/focus identity, both `human_generation` and `pointer_generation` and full immutable decision-to-plan or choice-to-plan set, including every rule and stage identity. A checked invalidation generation records the first incompatible candidate/request, obscurity, observation gap/degradation or incomplete observation, even when the original display returns before inspection. Positive keyboard or pointer input (§ Terminal reply classification), shutdown and ownership loss also revoke authority; arithmetic or randomness failure returns an explicit error. Repeated compatible inspections reuse the token. A tested equivalent redraw/resize preserves it only when the exact profile proves unchanged semantic/choice/focus and plan identity; output sequence alone is evidence rather than revocation. Healthy transient queue pressure defers authority without rebinding the token. The random value is independent of hidden-text hashes and authorization; a revoked token requires fresh inspection and decision reconciliation.

A cursor transit is the one incompatible frame that leaves the generation unchanged. It is a frame that yields no current request while a request is current, is not a verified non-interaction, and keeps every row of the current request's matched live region identical in text to the frame that established the request; typically the native CLI moves the cursor outside the form to redraw another part of the screen. During a transit the request is not current, so inspection returns no token and a response is blocked. A following frame with the same request restores it under the unchanged generation and token. A changed region row, a different request fingerprint, closure, `obscured` visibility, an observation gap, child exit and positive manual input still invalidate immediately.

A gated plan changes the request between its stages, so the runner accepts exactly one change after each complete nonfinal stage: the transition the next stage's gate names. For a focus gate, a fresh quiescent observation qualifies when its episode, rule, semantic fingerprint, choices, native correlation, `human_generation` and `pointer_generation` are unchanged and its resolved focus equals the gate. For a screen gate, the continuation binding of § Continuation screens and both-counter checks apply. Pointer input stops the gated plan with `manual_input` and uncertainty after a written stage. The runner rebinds the in-flight selection to the qualifying observation, with its new request generation, and writes the next stage, minting no token because the reservation already consumed the request token.

The next stage waits for its gate within the responding invocation's deadline, and an observation that still shows the previous stage's screen and focus keeps waiting. These end the response with the failure reporting below (`uncertain` once a byte was written), and the native surface stays for manual input with no further byte:

- a focus on any other option;
- a screen other than the expected continuation after a stage that opens one;
- the episode closing before the final stage;
- any other identity change, manual input or a degraded observation;
- the deadline expiring.

Inspection exposes the current metadata evidence only through its §6.3 projection, independently of private request/match/plan state: neither operation text nor private plan eligibility substitutes for a disclosed structured fact, and a proved `single_request` choice establishes answer eligibility whether or not public evidence states the choice scope. Keep transient raw semantics, focus and plan bytes in memory; persist no token or screen-text digest.

Use versioned control operations `permission_inspect` and `permission_respond`. A five-second command deadline covers locking, database reads and socket exchange. Before the first byte, the serialized runner resolves pending manual input and unread output, revalidates the current visible stable permission request/token and tested answer capability, and records in memory that the episode has an answer under the current ownership epoch. The capability binds to the response without a profile identifier: its backend and native version equal the run's, its request rule equals the event evidence `rule_id`, and its plan key equals the requested decision or choice with a non-empty fixture list. The request token, expected owner epoch and event/message association bind the response to the inspection that minted the token. A decision on a question, a choice on another class, a choice without a tested plan of the current question, other interaction classes, a second answer for the episode, stale identity or unavailable storage receive zero-byte rejection. Write only the profile's answer plan for that key, rechecking relevant state and each stage's gate before that stage; partial writes or ambiguous state return `uncertain`. The CLI accepts semantic allow/deny decisions and declared question choice ids, not arbitrary key sequences.

Track answer progress as cumulative actual positive byte counts across ordered stages. A complete nonfinal stage retains `partial` progress; only complete final-stage submission establishes `submitted` and returns acceptance. A stage write returning `EAGAIN`/`EWOULDBLOCK` or `EINTR` with zero bytes may yield within the same attempt and original expiration. Retain the untouched next-stage index and any cumulative `partial` progress, then retry only that stage after fresh actual drain, current token/request/choice/focus/plan, `human_generation`, `pointer_generation` and owner validation under a fresh write authorization. Completed stages stay completed, and this continuation is not a new invocation.

Record a syscall result before yielding. A recorded zero-byte `EAGAIN`/`EWOULDBLOCK` or `EINTR` preserves the actual prior stage/count and releases its write authorization; a yielding operation stays in progress. Abandoning a write authorization before any I/O records that no I/O occurred, while possible I/O without a recorded result retains uncertainty and every known earlier byte count. A positive short write, incompatible state, unrecorded possible I/O or expiration stops the remaining plan. Preserve cumulative progress and the original cause, and return `uncertain`.

##### One answer per episode

The runner keeps, in memory, which observed episodes already have an answer written; nothing about a response is stored. Once the first byte of an answer is written, the episode accepts no further answer: a later `permission respond` for it returns `rejected/zero_bytes` with reason `an answer was already sent for this episode`. An answer refused with zero bytes leaves the episode answerable. The record ends with the episode or the runner; a new episode or a restarted runner issues a new token. Recovery sends no answer keys. Key submission and decision `allow` establish neither native approval nor command success.

§6.3 Permission commands fixes the inspection and response output, exit codes and failure output. When asking the user to execute a response, present the concrete command with actual snapshot values; stale values fail before bytes.

Answer-profile capability belongs to the run's selected profile, rests on native evidence that provenance validation verifies (§ Profile selection by minimum version) and is independently tested for allow and deny, and for each question choice. It specifies request form/rule, single-request choice semantics for a decision or the declared choice for a question, focus/selection requirements, exact key stages and output revalidation. Use real native evidence rather than invented Enter/Escape mappings. Persistent/session-wide permission changes have no decision answer. Unsupported answer profiles preserve manual native input. Lacking an atomic native request API, serialize available input/output, reject observed mismatches and retain uncertainty for remaining races and indistinguishable requests.

#### Director consumption and continuation

The Director consumes runtime events through the `poll --json` each `message listen` notice prompts, or through manual `poll --json` when no receiver is available. Runtime events reach the Director's stdout receiver and durable inbox; they provide no composer injection, Escape-first wake, goal activation or host interrupt guarantee (latency targets: § Coverage and latency). Native model consumption latency remains controlled by the Director host and may be unbounded while the host is blocked or paused.

The poll consumes the message. Before running any other command after it, the Director records the unfinished action in host-owned state; it then reads the complete event, verifies its durable event/message link with `event show` and reconciles the current run/interaction. An ordinary member can quote event JSON; a claimed event identity is verified against stored ownership, never trusted from message text alone. Old/closed events remain readable but are not a new request to answer.

| Stage | Observable evidence | What remains outstanding |
|---|---|---|
| Detection | Rule/profile and current episode evidence | Durability and Director reception. |
| Persistence | Event ID plus committed inbox message ID | Director consumption. |
| Consumption | The consuming poll plus Director-owned pending-action record | Risk review, decision and actual operation outcome. |
| Decision | Director auto-mode evidence and attributed decision, or the user's request-specific decision | Answer-key submission and native result. |
| Answer-key submission | Bound durable receipt/stage, or attributed manual native input | Native decision evidence and actual operation result. |
| Native decision observed | Explicit profile-supported native evidence, when available | Actual operation result and dependent continuation. |
| Episode disappears | Closure reason and current screen | Decision and execution outcome may remain unknown. |
| Continuation verified | Actual operation result/output and member's resumed dependent work | Any remaining task-specific work. |

For a permission refusal returned to the member, the evidence route is the member's routed request: exact attempted command, absolute cwd, required environment overrides and actual refusal. A screen event can alert the Director earlier or supplement that route; it supplies no missing dispatch evidence. For a blocked request, follow the `review_permission_request` instruction (§ Request-bound permission responses); the native result it inspects is the native output/exit/error and the member's actual continuation. Previews, prompts and native commands the runner queued during the request drain automatically after the interaction closes (§ Runner-owned deferred delivery); a deferred input's eventual result arrives as an `input_delivery_result` event. Messages that were held rather than queued remain pending for the member runner's sweep (§ Runner pending-preview sweep). A different command submitted through native exec is a separate authorized action, never proof that the original request was answered. Reconcile uncertain outcomes before retries, and preserve host/managed permission authority throughout.

The Director handles these events through the same wake, consuming poll, host-owned pending-action record and `event show` verification:

| Event | Director procedure |
|---|---|
| `interaction_opened` in automatic mode | The permission-request procedure above, including `permission respond`. |
| `interaction_opened` for a `question` | Follow the `review_question` instruction (§ Request-bound permission responses), submitting the choice with `permission respond --choice`. |

A routed request that arrives with no event follows the refusal route above.

#### Coverage and latency

A run without a selected profile, and a capability its selected profile does not declare, retain explicit `unknown` coverage and manual operation; fixed-policy launch modes use the version-selected profile like every other mode.

With a live receiver and available storage, the latency target is two seconds from a stable prompt to inbox commit and, while no notice is held (§6.3 `message listen`), another two seconds from commit to receiver output; while a notice is held, the next notice announces the committed message, after the receiver's poll releases the hold or the ceiling fires.

## 7. Cross-cutting concerns

### 7.1 Configuration & environment

The database URL resolves per §6.3 Database selection, and config directories resolve lazily per selected backend (§6.3 Config-dir resolution). Validate supplied values and required state at their boundary.

### 7.2 Error handling & exit-code policy

Usage errors, parse errors (§6.3 Error Messages) and model/effort/line-validation errors exit 2. Application, database and transport errors exit 1 and print per §6.3 Error Messages. A successful finite timeout exits 0. Preserve primary causes alongside explicit cleanup failures. Missing required state or unknown enum/column values propagate instead of substituting meaningless defaults.

### 7.3 Output / JSON / truncation

JSON follows §6.4 JSON encoding with the member orders §6.3 lists; message-body truncation follows §6.3 Message Body Truncation.

### 7.4 Logging & stdout discipline

Successful command records stay on stdout. Post-commit notification diagnostics and creation compensation failures go to stderr. A committed message retains successful output even when its preview cannot be delivered; an uncertain native command prints no success record.

### 7.5 Time discipline

Use UTC for durable timestamps, monotonic clocks for budgets and deadlines. Cap every bounded process/database operation to remaining time. Every acquired lock, descriptor and owned child process is released on every error path and at process exit.

### 7.6 Packaging & distribution

Ship one `cafleet` binary per release target. The release version, printed by `cafleet --version` and used bare as the release tag, names the default database file and asset stamps. Archives are `cafleet-v<version>-<target>.tar.gz` containing the binary, where `<target>` is the platform name in target-triple form, for example `x86_64-unknown-linux-musl`. The release version (above) owns `--version`, asset stamps and doctor's installation checks. Embed the `assets/` tree (§6.3 Asset installation) offline. Building the binary requires no documentation-site output.

### 7.7 Agent-operation contract — one-shot command isolation

Agents run each CAFleet process as the only command in its shell-tool invocation, so a host's single-command permission rules match it; the shipped CAFleet skill owns the operating procedure.

## 8. Database schema

The binary embeds one complete schema baseline, `assets/migrations/V1__baseline.sql` (§6.3 Asset installation), whose application identity is `PRAGMA application_id` `1128351289` (`0x43414639`, ASCII `CAF9`). Each cafleet version owns its default database file, `cafleet_<VERSION>.db`, where `<VERSION>` is the release version (§7.6), so each version starts from a fresh file; baseline initialization records that version in the `cafleet_database` row. The fresh baseline creates exactly the application tables declared in the schema, and its ledger holds exactly the row `(1, 'baseline')`. Compatibility diagnosis verifies, in order, the application identity, that baseline ledger, every required table’s column shape with required indexes/foreign keys, and a recorded version equal to the running binary's. Any other identified or populated file, including one recorded by another cafleet version or one whose ledger holds a row beyond the baseline, is an incompatible format whose bytes are preserved, and `cafleet doctor` reports the version's default file as absent until `cafleet setup` creates it. Unrelated auxiliary tables, indexes and triggers are compatible while those required definitions remain intact.

| Table | Responsibility |
|---|---|
| `fleets` | Fleet identity, creation/deletion and root Director reference |
| `members` | Member registration and active/deregistered state |
| `member_placements` | Multiplexer and coding-agent association |
| `messages` | Point-to-point delivery, poll consumption and preview stamp |
| `runtime_database_identity` | Singleton database-instance identity for runtime namespaces |
| `cafleet_database` | Singleton record of the cafleet version that initialized the file |
| `member_runs` | Launch intent, ownership, lifecycle and recorded coverage |
| `member_interactions` | Observed interactive episodes and visibility closure |
| `member_runtime_events` | Sequenced events with atomic Director inbox association or archival |
| `member_input_operations` | Durable submission receipts, delivery method/deferral metadata, payload digests and settled replay protection |
| `member_runner_diagnostics` | Deduplicated, bounded runner diagnostics per run |
| `refinery_schema_history` | Baseline ledger |

Exact baseline DDL; the baseline itself declares the ledger table as its last statement:

```sql
PRAGMA application_id = 1128351289;

CREATE TABLE members (
    member_id INTEGER NOT NULL PRIMARY KEY AUTOINCREMENT,
    fleet_id INTEGER NOT NULL REFERENCES fleets (fleet_id) ON DELETE RESTRICT,
    name TEXT NOT NULL,
    description TEXT NOT NULL,
    status TEXT NOT NULL,
    registered_at TEXT NOT NULL
);
CREATE INDEX idx_members_fleet_status ON members (fleet_id, status);

CREATE TABLE fleets (
    fleet_id INTEGER NOT NULL PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL,
    created_at TEXT NOT NULL,
    deleted_at TEXT,
    director_member_id INTEGER REFERENCES members (member_id) ON DELETE RESTRICT,
    last_window_ordinal INTEGER NOT NULL DEFAULT 0 CHECK(last_window_ordinal >= 0)
);

CREATE TABLE member_placements (
    member_id INTEGER NOT NULL PRIMARY KEY REFERENCES members (member_id) ON DELETE CASCADE,
    mux_session TEXT NOT NULL,
    mux_window_id TEXT,
    mux_window_name TEXT,
    window_ordinal INTEGER NOT NULL CHECK(window_ordinal >= 0),
    column_side TEXT NOT NULL CHECK(column_side IN ('left','right')),
    mux_pane_id TEXT,
    backend TEXT NOT NULL,
    mux_server TEXT NOT NULL,
    coding_agent TEXT NOT NULL,
    created_at TEXT NOT NULL,
    CHECK((mux_pane_id IS NULL) = (mux_window_id IS NULL)
          AND (mux_window_id IS NULL) = (mux_window_name IS NULL))
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
CREATE INDEX idx_messages_from_member_created_at ON messages (from_member_id, created_at);

CREATE TABLE runtime_database_identity (
    singleton_id INTEGER PRIMARY KEY CHECK(singleton_id = 1),
    instance_id TEXT NOT NULL CHECK(length(instance_id) = 32 AND instance_id NOT GLOB '*[^0-9a-f]*')
);
INSERT INTO runtime_database_identity VALUES(1, lower(hex(randomblob(16))));

CREATE TABLE cafleet_database (
    singleton_id INTEGER PRIMARY KEY CHECK(singleton_id = 1),
    cafleet_version TEXT NOT NULL CHECK(length(trim(cafleet_version)) > 0)
);

CREATE TABLE member_runs (
    run_id INTEGER PRIMARY KEY AUTOINCREMENT,
    member_id INTEGER NOT NULL REFERENCES members(member_id) ON DELETE RESTRICT,
    fleet_id INTEGER NOT NULL REFERENCES fleets(fleet_id) ON DELETE RESTRICT,
    director_member_id INTEGER NOT NULL REFERENCES members(member_id) ON DELETE RESTRICT,
    backend TEXT NOT NULL CHECK(backend IN ('claude','codex','opencode','hermes','pi')),
    database_identity_json TEXT NOT NULL CHECK(json_valid(database_identity_json) AND json_type(database_identity_json) = 'object'),
    launch_nonce TEXT NOT NULL CHECK(length(launch_nonce) = 64 AND launch_nonce NOT GLOB '*[^0-9a-f]*'),
    runtime_path TEXT NOT NULL CHECK(length(trim(runtime_path)) > 0),
    requested_mode TEXT,
    resolved_mode TEXT NOT NULL CHECK(length(trim(resolved_mode)) > 0),
    mode_source TEXT NOT NULL CHECK(mode_source IN ('default','explicit')),
    permission_argv_json TEXT NOT NULL CHECK(json_valid(permission_argv_json) AND json_type(permission_argv_json) = 'array'),
    coverage TEXT NOT NULL CHECK(coverage IN ('tested','unknown')),
    status TEXT NOT NULL CHECK(status IN ('starting','prepared','running','degraded','recovering','exited','failed','lost')),
    native_version TEXT,
    owner_epoch INTEGER NOT NULL CHECK(owner_epoch > 0),
    owner_kind TEXT NOT NULL CHECK(owner_kind IN ('creator','runner','reconciler')),
    owner_pid INTEGER NOT NULL CHECK(owner_pid > 0),
    owner_start_identity TEXT NOT NULL CHECK(length(trim(owner_start_identity)) > 0),
    cleanup_state TEXT NOT NULL CHECK(cleanup_state IN ('complete','incomplete')),
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
    cleanup_diagnostics_json TEXT CHECK(cleanup_diagnostics_json IS NULL OR (json_valid(cleanup_diagnostics_json) AND json_type(cleanup_diagnostics_json) = 'array')),
    recovery_origin_status TEXT CHECK(recovery_origin_status IS NULL OR recovery_origin_status IN ('starting','prepared','running','degraded','recovering','exited','failed','lost')),
    native_session_json TEXT CHECK(native_session_json IS NULL OR (json_valid(native_session_json) AND json_type(native_session_json) = 'object')),
    CHECK((mode_source = 'default' AND requested_mode IS NULL) OR (mode_source = 'explicit' AND requested_mode IS NOT NULL))
);
CREATE UNIQUE INDEX idx_member_runs_active ON member_runs(member_id) WHERE status NOT IN ('exited','failed','lost');

CREATE TABLE member_interactions (
    interaction_id INTEGER PRIMARY KEY AUTOINCREMENT,
    run_id INTEGER NOT NULL REFERENCES member_runs(run_id) ON DELETE RESTRICT,
    episode_seq INTEGER NOT NULL CHECK(episode_seq > 0),
    classification TEXT NOT NULL CHECK(classification IN ('permission_request','question','unknown_interaction')),
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
    kind TEXT NOT NULL CHECK(kind IN ('coverage_unknown','interaction_opened','interaction_closed','input_delivery_result','input_delivery_stalled','runner_degraded','runner_lost','runner_exited')),
    observed_at TEXT NOT NULL,
    payload_json TEXT NOT NULL CHECK(json_valid(payload_json) AND json_type(payload_json) = 'object'),
    notification_state TEXT NOT NULL CHECK(notification_state IN ('inbox','archived')),
    message_id INTEGER UNIQUE REFERENCES messages(message_id) ON DELETE RESTRICT,
    archive_reason TEXT CHECK(archive_reason IS NULL OR archive_reason IN ('member_inactive','fleet_deleted','director_inactive','head_not_ready','duplicate_delivery')),
    UNIQUE(run_id, event_seq),
    CHECK((notification_state = 'inbox' AND message_id IS NOT NULL AND archive_reason IS NULL) OR
          (notification_state = 'archived' AND message_id IS NULL AND archive_reason IS NOT NULL))
);

CREATE TABLE member_input_operations (
    input_id INTEGER PRIMARY KEY AUTOINCREMENT,
    run_id INTEGER NOT NULL REFERENCES member_runs(run_id) ON DELETE RESTRICT,
    operation_key TEXT NOT NULL CHECK(length(trim(operation_key)) > 0),
    kind TEXT NOT NULL CHECK(kind IN ('prompt','exec')),
    payload_digest TEXT NOT NULL CHECK(length(payload_digest) = 64 AND payload_digest NOT GLOB '*[^0-9a-f]*'),
    state TEXT NOT NULL CHECK(state IN ('ready','in_progress','accepted','uncertain','settled')),
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    disposition TEXT CHECK(disposition IS NULL OR disposition IN ('submitted','not_submitted','abandoned_uncertain')),
    evidence_ref TEXT,
    delivery_method TEXT CHECK(delivery_method IS NULL OR delivery_method IN ('profile','fallback')),
    delivery_reason TEXT CHECK(delivery_reason IS NULL OR length(trim(delivery_reason)) > 0),
    deferred_at TEXT,
    defer_reason TEXT CHECK((deferred_at IS NULL AND defer_reason IS NULL)
        OR (deferred_at IS NOT NULL AND defer_reason IS NOT NULL
            AND kind IN ('prompt','exec'))),
    stop_cause TEXT CHECK(stop_cause IS NULL
        OR (stop_cause IN ('interaction_open','interaction_candidate','submit_unconfirmed',
                          'deadline_expired','short_write','write_failed','runner_stopped')
            AND state IN ('uncertain','settled'))),
    UNIQUE(run_id, operation_key),
    CHECK((state = 'settled' AND disposition IS NOT NULL) OR (state != 'settled' AND disposition IS NULL))
);

CREATE TABLE member_runner_diagnostics (
    diagnostic_id INTEGER PRIMARY KEY AUTOINCREMENT,
    run_id INTEGER NOT NULL REFERENCES member_runs(run_id) ON DELETE RESTRICT,
    source TEXT NOT NULL CHECK(source IN ('event_writer','observation','deferred_input','queue_housekeeping','preview_sweep','input_result','permission_answer','shutdown')),
    message TEXT NOT NULL CHECK(length(trim(message)) > 0 AND length(CAST(message AS BLOB)) <= 512),
    first_at TEXT NOT NULL,
    last_at TEXT NOT NULL,
    count INTEGER NOT NULL CHECK(count > 0),
    UNIQUE(run_id, source, message)
);
CREATE INDEX idx_member_runner_diagnostics_recent ON member_runner_diagnostics(run_id, last_at);

CREATE TABLE refinery_schema_history (
    version INT4 PRIMARY KEY,
    name VARCHAR(255),
    applied_on VARCHAR(255),
    checksum VARCHAR(255)
);
```

Initialization inserts the ledger row with `version` 1 and `name` `'baseline'`. Its `applied_on` is the §5.1 canonical timestamp of initialization, and its `checksum` is the 64-character lowercase hexadecimal SHA-256 digest of the embedded `assets/migrations/V1__baseline.sql` bytes, ledger declaration included. Diagnosis compares only `version`, `name` and the column shape.

Baseline column semantics:

| Table | Semantics |
|---|---|
| `fleets` | `director_member_id` is backfilled inside the bootstrap transaction. A live fleet has null `deleted_at` and an active valid Director with placement |
| `members` | `status` decodes as `active` or `deregistered`; unknown values fail. Deregistration is soft and keeps the row |
| `member_placements` | `mux_pane_id`, `mux_window_id` and `mux_window_name` are null exactly while member creation is pending. `mux_server` is the server every invocation for the fleet targets (§6.5 Multiplexer context); member placements copy the Director's |
| `messages` | `status_state` decodes as `input_required` or `completed`; the recipient's consuming poll sets `completed`. `notified_at` is internal preview state that the public message record omits; a self-send stamps it on insertion and stays pending until a poll consumes it |

Window layout columns follow §6.5 Fleet window layout:

| Column | Semantics |
|---|---|
| `fleets.last_window_ordinal` | Highest overflow ordinal ever allocated for the fleet; `0` until the first overflow window, and never decreased |
| `member_placements.mux_window_name` | Creation-time window name: public fleet creation sets `cafleet-<fleet_id>-0`, overflow creation sets `cafleet-<fleet_id>-<ordinal>`, and members using an existing window copy its recorded name |
| `member_placements.window_ordinal` | Chosen at registration; `0` for the Director and Director-window members |
| `member_placements.column_side` | Chosen at registration; `left` for the Director row and `right` for every Director-window member. A stored value outside `left` / `right` fails decoding |

A concrete placement selects the shared pane input lock, and duplicate entries targeting the same backend/session/pane share it. Inbox streaming ignores `notified_at`, and native handles and emitted-ID tracking belong to the receiver process with no durable consumer state.

Runtime records keep their run/member/fleet/Director references after soft deregistration, so history survives member and fleet closure.

Initialization runs in one `BEGIN IMMEDIATE` transaction, so concurrent initializers of one file serialize on the SQLite write lock within the connection's busy timeout. Inside that transaction it classifies the file:

| File state | Condition | Result |
|---|---|---|
| Empty | No ledger table, application identity zero and no user tables | Applies the baseline and inserts the ledger row and the `cafleet_database` row for the running version, then commits |
| Baseline | The ledger's `(version, name)` rows are exactly `(1, 'baseline')` | Writes nothing and succeeds |
| Other | Every other file, including a ledger with zero rows or any row beyond the baseline | Writes nothing and fails with the §6.3 incompatible-format error naming the database path |

A failed initialization leaves no identity, table, ledger or version row, and two concurrent initializations of one empty file both succeed with exactly one ledger row and one version row.

`native_session_json` records a `hermes` run's native session identity for work settlement (§6.7 Hermes work settlement) and is null for every other backend and for a run whose identity is not yet captured.

The DDL above owns every runtime table column, type, enum, nullability constraint, foreign key, uniqueness invariant and partial unique index. Section 6.9 specifies their runtime semantics, including database-instance identity and privacy. Setup's required-shape check includes `stop_cause` on `member_input_operations`; runtime validates the free-text defer/outcome reasons of §6.9 Delivery reporting.

## 9. Testing strategy

A conforming implementation is verified at four levels. Command level: every §6.3 command's exit code, stdout/stderr text and JSON bytes, the multiplexer argv it issues, and the §8 rows it leaves, driven through the CLI with stand-in multiplexer and native executables. Function level: argv construction, multiplexer command choreography, terminal-reply and key parsing, ANSI stripping, catalog validation and screen classification of every catalogued native screen. Runtime level: the §6.9 runner fed scripted screens, commands and storage faults under a controlled clock, asserting events, rows, receipts and bytes written to the child. Operating-system level: real PTYs, process groups, signals, pane-lock exclusion and release on process death, and shutdown deadlines in real time.
