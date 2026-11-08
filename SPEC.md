# CAFleet — Reimplementation Specification

This is the self-contained interface and observable-behavior specification for the `cafleet` CLI, broker, independent coding-agent sessions and embedded assets. Exact authored errors, JSON fields/order and command layouts govern; incidental host-language quoting and operating-system error suffixes preserve intent. Complete module contracts appear in §6 Module specifications.

## 1. Overview & goals

CAFleet multiplexes coding-agent sessions in tmux or herdr and coordinates them with durable point-to-point SQLite messages. The host owns task continuation and native execution results; the Director owns delegation, wake handling, evidence inspection, authorization and cleanup. The shipped product consists of the CLI, embedded skills/presets and documentation site.

Reproduce the documented command/option/exit surface, configuration defaults, current schema, enum values, output shapes, lifecycle, finite receive, notification, locking, termination deferral and native dispatch semantics. The current format identity defines compatibility; required data corruption surfaces at its read boundary.

## 2. Architecture stance

The core is synchronous. A send previews pending ordinary-member messages within its budget; finite receive owns bounded waits and notification retries. A persistent read-only listener emits pending inbox records through JSONL without pane operations. Separate processes coordinate SQLite changes through transactions and complete pane submissions through process-shared file locks. Host-native tools own receiver handles and collect native results; consumed deliveries are explicitly ACKed, and deferred actions and native handles remain in host-owned state. Automatic Director reception sends no Escape, text, Enter or queued composer input.

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
| Notification / receiving | Ordinary-member draining delivery, caller-owned finite retries and read-only persistent inbox streams, optionally forwarded to a Codex thread |
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

Derive `director` exactly when `fleets.director_member_id == members.member_id`, otherwise `member`.

### 5.5 Recipient and pending placement

Every message has an integer `to_member_id` identifying its inbox owner. Nullable `mux_pane_id` represents pending member creation and documented diagnostics; required concrete pane actions fail contextually until it is patched.

### 5.6 Typed records and wire presenters

Broker records carry typed values independently of CLI formatting. Public message records are ordered `message_id, from_member_id, to_member_id, created_at, status_state, status_timestamp, text`. Full JSON remains untruncated; internal notification state stays in storage.

## 6. Module specifications

### 6.1 Persistence & Schema

Schema compatibility and the application identity are specified in §8 Database schema.

Connections use SQLite foreign-key enforcement and a normal five-second busy timeout; a bounded operation caps that wait to its remaining budget. The selected file path resolves per §6.3 Database selection, and its canonical database file keys lock-directory construction.

Format validation, baseline initialization, incompatible-file refusal and file preservation follow §6.3 Schema-version guard. An invocation's validated connection serves its internal send, ACK and notification stamp operations; standalone public broker write functions retain their own format guard.

### 6.2 Broker

#### Fleets

Bootstrap validates the Director's multiplexer server, pane and backend token, then inserts fleet, Director and placement and backfills the Director reference in one transaction. The current session remains the Director; no pane split or process launch occurs.

List, show and delete follow §6.3 `fleet list`, `fleet show` and `fleet delete`; deletion preserves messages, and repeat deletion returns zero newly deregistered members.

#### Members

Registration requires a live fleet and active valid root Director. Insert the member and pending placement atomically. Creation owns compensation for later prompt/split/patch failures. Root Director individual deletion fails. Member deletion locks its concrete pane, kills it and deregisters it; a placementless/pending member requires no concrete pane lock.

Roster reads active members of live fleets, derive kind (§5.4 Member kind discriminator) and include nullable activity. Activity is the maximum sent, received and ACK timestamp; `idle` is the elapsed seconds since the latest known activity, truncated to an integer, and null is the correct state without activity. Detailed views retain registration, description and placement data. Placement mutations acquire the same pane input locks as input writers. Required Director/placement/enum corruption fails rather than producing placeholder values.

#### Messaging

Send requires active sender/recipient in the same live fleet and persists exact text. Every individual fan-out send commits independently; keep already committed IDs when another recipient fails. Poll/wait read incoming unread messages by descending status timestamp/message ID. Show requires existence. ACK requires an unread existing row, stamps completion and removes notification eligibility (§6.4 Persisted shape).

Pending preview selection chooses the oldest eligible row by `(created_at, message_id)` for an active ordinary recipient with a concrete pane; root Directors and self-sends are ineligible. Revalidate ordinary-member kind, fleet, placement, message status and null notification stamp while holding pane input ownership. Stamp success conditionally only after complete submission; delivery leaves `status_state` unchanged until explicit ACK. Listening selects every pending record independently of notification fields, including stamped pending self-sends, without changing broker state.

#### Broker errors

Preserve the validation boundary and explicit cause. Missing member/message, inactive fleet/Director, cross-fleet pair and already-ACKed rows fail with application errors; §6.3 CLI *Error Messages* lists their exact strings.

### 6.3 CLI

#### Subcommand summary

| Command | Purpose | Subject | JSON |
|---|---|---|---|
| `setup` | Initialize the version's database and install assets | — | no |
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

#### Database selection

The global option `--database-url <url>` selects the SQLite database. It is accepted before or after the subcommand, so a prefix allow rule such as `Bash(cafleet message *)` matches `cafleet message poll 7 --database-url <url>`. Omitted, the default `sqlite:///` + `~/.local/share/cafleet/cafleet_<VERSION>.db` applies, `<VERSION>` being the package version (the version field of `cafleet --version`, e.g. `0.31.1`).

The default expands HOME once. An explicit URL has the form `sqlite:///<path>`; resolve a relative file path against the invoking working directory before canonicalizing the database file for shared pane-lock identity. A literal tilde in an explicit URL is not expanded.

CAFleet reads no `CAFLEET_*` environment variable. Member creation renders the selected database into the spawn prompt (§ Spawn-prompt substitution), and a member whose `DATABASE URL:` line names a URL passes `--database-url <url>` on every `cafleet` command. A member that omits the option on a custom-database fleet reaches the default database: when that file is absent the command fails with `no cafleet database; run 'cafleet setup'`, and when it exists its literal IDs may match another fleet. This is the same cooperative limit as the Director-only command guard (§ Director-only command guard).

#### Global Options

`cafleet --version` prints `cafleet <version>` and exits 0 before dispatch; `--database-url` follows § Database selection.

#### Help text

`--help` describes every visible command, subcommand and option with a one-line description. Command group descriptions state the purpose given in the subcommand summary. Help derives choice lists from the same registries that validate the values:

| Option | Help shows | Source |
|---|---|---|
| `member create --permission-mode MODE` | Each backend's accepted modes and its omitted default, in backend registry order, as `BACKEND: MODE, MODE (default MODE)` joined by `; ` | The backend policy catalog (§6.9 Launch policy and capability registry) |
| `permission respond --decision`, `--decision-source`, `--director-mode` | Their possible values | The stored enum values of the permission response storage (§6.9 Permission response storage) |
| `input reconcile --disposition`, `member create --event-evidence` | Their possible values | Their command enums |

An invalid value for an option with possible values exits 2 with a usage error that lists those values. `member create` resolves its backend at run time, so a `--permission-mode` value outside that backend's catalog keeps its run-time usage error `--permission-mode for BACKEND must be one of CHOICES (got 'VALUE')`, also with exit 2. Help reads `Launch-only native permission policy for the resolved backend — claude: default, manual, dontAsk, acceptEdits, auto, bypassPermissions, plan (default auto); codex: on-request, never, approve-for-me (default approve-for-me); opencode: cafleet, cafleet-ask (default cafleet-ask); hermes: smart, manual (default smart); pi: cafleet, cafleet-ask (default cafleet-ask)`.

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
| `doctor` | One `STATUS COMPONENT: DETAIL` summary line per component (`--verbose`: one `STATUS NAME: DETAIL` row per check) | `{summary, checks}` |

##### Member list output

Roster records include `member_id`, `name`, `kind`, `placement`, `last_sent`, `last_recv`, `last_ack` and `idle`. `kind` follows §5.4 Member kind discriminator. Missing activity is null; placement may be null, or its pane ID pending. Text uses `-` for absent values and `(pending)` for a pending pane.

#### JSON output (`--json`)

Place `--json` among the subcommand options, before a `--` separator that introduces literal key arguments. It emits compact single-line JSON with UTF-8 and complete untruncated message text. Application errors are stderr text (§6.3 Error Messages); parse errors exit 2. Commands marked no reject the flag, including the root group.

#### Positional subject ids

Place the command’s subject ID immediately after its subcommand; event, input and permission subjects use their positive durable event, input and interaction IDs. The subject row supplies related fleet/recipient state. IDs are complete DB-assigned integers, without prefix resolution or environment defaults; a Director-only command also takes its caller's literal ID as `--from-member-id` (§ Director-only command guard). A missing or malformed subject uses the parser's native error, exit 2.

Spawned members read literal `FLEET ID:`, `YOUR MEMBER ID:`, `DIRECTOR MEMBER ID:`, `CODING AGENT:` and `DATABASE URL:` lines, pass these literal IDs on each invocation and apply `DATABASE URL:` per § Database selection.

#### Sender and recipient (`--from-member-id`, `--to-member-id`)

Send requires integer sender and recipient flags, validated per §6.2 Messaging; the sender row determines the fleet. IDs are coordination identifiers within the local trust model.

#### Director-only command guard

The Director-only commands are exactly `member prompt`, `member exec` and `member press` (explicit native pane input), `permission respond` (native permission answers), `input reconcile` (input-receipt settlement) and `member recover` (run reconciliation), which write into a member's live native session or its runtime record. Every other subcommand, including `permission inspect`, `input show` and `event show`, is unguarded; the lifecycle commands `member create`, `member delete`, `fleet create` and `fleet delete`, roster listing (`member list`) and captures (`member capture`) remain Director-owned by role.

Each requires `--from-member-id <director-member-id>`, a positive integer: the caller's ID, or the pinned Director for `permission respond`. A missing or malformed value is a parse error, exit 2.

After argument parsing and the invocation connection's schema-version guard (§ Schema-version guard), and before any multiplexer, lock, socket or file access or write, the command resolves its subject's fleet — a member's own fleet, or the fleet of the run that owns an input or interaction — and requires that fleet to be live with `director_member_id` equal to `--from-member-id`. An unknown subject's not-found error precedes this check. Otherwise the command exits 1, prints nothing on stdout and writes this application error to stderr, `permission respond` included (no response attempt began, so the §6.3 Permission commands structured-response exception does not apply):

```
Error: 'cafleet <command>' is a Director-only command and member <id> is not the root Director of fleet <fleet_id>; send the request to your Director.
```

A Director-only command dispatched into a member pane with `member exec` is checked by the ID it carries.

The guard is cooperative, not a security boundary: it stops a member that passes its own `YOUR MEMBER ID:` value and a call without an ID (a parse error), but a member that copies its `DIRECTOR MEMBER ID:` value passes it. Member authority remains defined by the member role and the Director's assignment.

#### `permissions.allow` coverage

Prefix rules such as `Bash(cafleet message *)` cover broker operations, including `cafleet message listen ID --timeout 300` and finite `cafleet message wait`. Native Monitor authorization is separate: a Bash-rule check establishes no Monitor permission. Literal IDs and trailing JSON flags preserve prefix matching.

Ask entries for the Director's pane-input prefixes follow §6.7 Claude permission posture, which states the permission-profile contract. `cafleet permission respond` runs under the Director host's effective policy after its request-specific decision; mode/source flags bypass no ask, deny or managed rule. Ordinary member role authority covers broker reception and evidence routing, while the Director owns permission responses.

#### Message Body Truncation

CLI text output truncates only message bodies, to a fixed 200 codepoints per §6.4 Text-mode projection; `--json` carries full bodies, pane captures preserve their complete content, and pane notification lines follow §6.5 Push notifications.

#### Text bodies (positional `TEXT` / `--file`)

Send and member creation require exactly one positional body or `--file PATH`; `--file -` reads UTF-8 stdin. Preserve exact nonblank body text. Missing/both sources or a blank positional body exit 2; blank file/stdin sources and file-read errors exit 1 with the authored diagnostics below. File read/UTF-8 failures retain the actual cause.

`member prompt` takes one positional line. Native exec has the separate command contract (§6.3 `member exec`).

#### `cafleet setup` — Onboarding and Schema Management

`cafleet setup [--coding-agent AGENT...]` prepares the database first, then installs assets. It classifies the selected database before any write per §6.3 Schema-version guard, leaving a current file of this version unchanged. Compatible setup then installs embedded assets offline and records versions. Backend selection is multi-value/repeatable, deduplicated and defaults to the supported backend registry; selected backends install in registry order.

After initializing, setup prints `Initialized database at {path}.`; a database that is already current prints `Database already initialized at {path}.`. Asset installation stops at the first backend whose config-dir resolution or installation fails: backends earlier in registry order keep their installation and record, and setup exits 1 with that backend's cause.

##### Config-dir resolution

Every surface that installs to or checks a backend's user-level config directory resolves its base from the backend's native config-location environment variable, else the default:

| Backend | Base | Skills dir | Preset target | Recorded path |
|---|---|---|---|---|
| claude | `$CLAUDE_CONFIG_DIR`, else `~/.claude` | `<base>/skills` | — | `<base>` |
| codex | `$CODEX_HOME`, else `~/.codex` | `<base>/skills` | `<base>/rules/cafleet.rules` | `<base>` |
| opencode (skills) | `~/.config/opencode` (fixed discovery path) | `<base>/skills` | — | — |
| opencode (preset) | `$OPENCODE_CONFIG_DIR`, else `~/.config/opencode` | — | `<base>/agents/cafleet.md`, `<base>/agents/cafleet-ask.md` | `<base>` |
| hermes | `$HERMES_HOME`, else `~/.hermes` | `<base>/skills` | — (setup installs the `cafleet` plugin under `<hermes home>` and enables it in the Hermes configuration; § Asset installation) | `<base>` |
| pi | `$PI_CODING_AGENT_DIR`, else `~/.pi/agent` | `<base>/skills` | — (setup installs the CAFleet pi extension under `<base>/extensions`; § Asset installation) | `<base>` |

OpenCode discovers skills only at the fixed path but loads agent definitions from `OPENCODE_CONFIG_DIR`, so only its presets follow the variable.

**Validation.** A set variable must hold an absolute path; any other value (the empty string, a relative path, a literal unexpanded `~/…`) fails at resolution time with exit 1 and `Error: <VAR> must be an absolute path (got '<value>')`. A set config-location variable or `HOME` whose value is not valid UTF-8 fails at the same point with exit 1 and `Error: <VAR> is not valid UTF-8` instead of selecting the default.

Validation is lazy: a site reads and validates a variable only when it resolves that backend's directory. `cafleet setup --coding-agent claude` with an invalid `CODEX_HOME` succeeds because the selector resolves only the targeted agent's directory; plain `cafleet setup` resolves each backend in registry order, so an invalid variable stops installation at that backend. `member create` resolves only the selected backend's directory during its availability check and forwards the validated value into the member environment (§6.3 Member environment). `doctor` reports a per-agent resolution error as that agent's `<agent>.configuration` check instead of aborting (§6.3 `cafleet doctor` — Environment Diagnosis).

**Recorded-path identity.** `asset_installs` rows and `doctor`'s installation checks key on each agent's Recorded path, stored absolute exactly as resolved (no canonicalization beyond the absolute-path validation); the fixed opencode skills base carries no identity.

##### Asset installation

The binary embeds every data file from the repository's `assets/` tree at build time:

| Source tree | Contents | Consumer |
|---|---|---|
| `assets/skills/` | One directory per skill, embedded file by file | Skill installation (this section) |
| `assets/presets/` | One directory per backend with presets | Preset installation (table below) |
| `assets/extensions/` | One directory per backend with an extension | Extension installation (table below) |
| `assets/migrations/` | The schema baseline `V1__baseline.sql` | Baseline initialization (§8 Database schema) |
| `assets/profiles/` | The bundled catalog `bundled.json` and the native screen fixtures `native/<backend>.json` | The profile catalog (§6.9 Bundled profiles and native screen fixtures) |

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

The *executable record* is a one-line file holding the absolute, canonicalized path of the running `cafleet` executable (`std::env::current_exe()`), written after the extension files. It is neither an embedded asset nor byte-compared; OpenCode plugin discovery loads only `.ts`/`.js` files and directories, and pi extension discovery loads only `*.ts`, `*.js`, `*/index.ts` and `*/package.json` entries, so both ignore the record. The plugins and extensions execute only the recorded path.

Skills-operation errors retain `failed to install skills into <skills_dir>: <error>`, preset-operation errors `failed to install preset into <target>: <error>`, and extension-operation errors, including the executable record, `failed to install extension into <target>: <error>`. A successful install records one `asset_installs` row for the agent, covering its skills, presets and extensions; a failure on any preset, extension or plugin enablement step fails setup and records no installation.

Setup prints each success line after its operation completes and records the installed version after all operations succeed: the skills line, one preset line per installed preset target in install order, then one extension line per extension target once the target and its executable record are written; a directory extension prints its directory once. Claude prints only the skills line; pi prints the skills line and the extension line; Hermes prints the skills line, the extension line and a plugin enablement line (§ Hermes plugin enablement):

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

Any failed command, a timeout, an unexpected value or a listed `plugins.disabled` entry after step 1 fails setup with `failed to enable hermes plugin cafleet in <config path>: <cause>` and records no `asset_installs` row; Hermes's writer refusing a key pinned by its managed scope is such a cause.

Hermes's ruamel round-trip writer replaces the file atomically, writing through a symlinked `config.yaml` to its target and preserving comments, key order, quoting and untouched nodes. A rerun after a successful enablement writes nothing.

A missing `plugins.enabled` enables no user plugin, and Hermes grandfathers installed user plugins only while the key is missing; after setup writes it, other installed but not enabled user plugins stay disabled until the operator enables them. Setup never runs `hermes plugins enable`, whose dependency admission re-syncs the Hermes environment, which the plugin does not need and which fails on installs without `uv.lock`.

Setup runs only the local binary and stays offline. `cafleet setup --coding-agent hermes` requires `hermes` on `PATH`; its absence fails with the cause `binary hermes not found on PATH`. Plain `cafleet setup` with `hermes` absent from `PATH` installs the Hermes skills, records the `asset_installs` row, skips the extension and the plugin enablement and prints:

```
hermes: skipped the plugin (binary hermes not found on PATH); run 'cafleet setup --coding-agent hermes' after installing Hermes
```

#### Schema-version guard

Every fleet, member, message, event, input and permission command validates the current format before any other database read or write. The classifier runs the §8 Database schema compatibility diagnosis; setup, doctor and the invocation guards share it, and guard and setup work reuse the invocation's connection. Setup initializes a missing file, or an empty SQLite file with identity zero and no user tables, with the baseline atomically; every other guarded command refuses such a file with `no cafleet database; run 'cafleet setup'`. Every other populated, identified or malformed file, including one recorded by another cafleet version, fails before database mutation, asset installation or asset-install recording with `database format is incompatible at 'PATH'; choose an empty database path and run 'cafleet setup'.` Preserve database open/read causes, incompatible file bytes and existing sidecars. Doctor reports the classification rather than blocking, completing its other read-only diagnostics even when the database is unavailable.

`--help` renders at parse time and exits before any command body runs, so neither group-level help (`cafleet fleet --help`) nor subcommand help (`cafleet fleet create --help`) triggers the guard; both print help even under a missing database.

#### `cafleet fleet` — Fleet Management

##### `fleet create`

`cafleet fleet create --name NAME --coding-agent AGENT [--pane PANE_ID [--mux-server SERVER]] [--json]`

`fleet create` and `doctor`'s `multiplexer` row are the only surfaces that read the caller's multiplexer environment. Require the selected backend token, then resolve each input, first match wins:

| Input | Source |
|---|---|
| Backend | `TMUX`/`HERDR_ENV` auto-detection (§6.5 Backend selection); a detection failure exits 1 with `cafleet fleet create must be run inside a tmux or herdr session` |
| `mux_server` (tmux) | `--mux-server`; else everything before the last two commas of a non-empty `TMUX`, so a socket path containing `,` is kept whole; else `tmux display-message -p '#{socket_path}'` (the default server) |
| `mux_server` (herdr) | `--mux-server`; else a non-empty `HERDR_SESSION`; else the `herdr session list` row whose socket equals `HERDR_SOCKET_PATH`; else `default` |
| Director pane | `--pane`; else a non-empty `TMUX_PANE` (tmux) or `HERDR_PANE_ID` (herdr); a tmux caller without `TMUX_PANE` exits 1 with the detection-failure message |

The pane is then read on the chosen server — `tmux -S <server> display-message -p -t <pane> '#{session_name}|#{window_id}|#{pane_id}'` or `herdr --session <server> pane current --pane <pane>` — and its session, window and pane are recorded with the server as the Director's placement (§6.5 Multiplexer context). tmux prints `||` and exits 0 for a missing pane, so a reply with an empty field, or whose `pane_id` differs from the requested pane, means the pane was not found. Each failure exits 1 before any write:

| Condition | Error |
|---|---|
| tmux finds no server | `Error: no tmux server found; pass --mux-server <socket path>` |
| The pane is not on the chosen server | `Error: pane <pane-id> was not found on the <backend> server <server>; pass --pane and --mux-server for the Director's pane` |
| herdr without `--pane` and with `HERDR_PANE_ID` unset | `Error: cafleet fleet create cannot identify its herdr pane because HERDR_PANE_ID is not set; pass --pane <pane-id>` |
| `HERMES_SESSION_ID` is set and `--pane` is absent | `Error: cafleet fleet create cannot identify its pane inside a Hermes session; pass --pane <pane-id>` |

A Hermes terminal tool runs in the shared Hermes gateway and may carry another pane's `TMUX_PANE`, hence the Hermes refusal; Hermes's leading-`!` local shell runs in the client process with the pane's own environment (`! echo $TMUX_PANE` or `! echo $HERDR_PANE_ID`).

Registration follows the §6.2 Fleets bootstrap transaction. The Director has `name="Director"` and `description="Root Director for this fleet"`.

Run `fleet create` from the coding-agent session declared by `--coding-agent`; root-Director reception follows §6.3 `message listen` and §6.5 Push notifications.

##### `fleet list`

List non-deleted fleets by descending creation timestamp/fleet ID. Rows retain the Director reference so its ID remains discoverable.

##### `fleet show`

`cafleet fleet show FLEET_ID [--json]` includes soft-deleted rows. Unknown ID: `fleet 'X' not found.`, exit 1.

##### `fleet delete`

`cafleet fleet delete FLEET_ID [--json]` ends every active ordinary member that owns a concrete pane, then deregisters the fleet. It retains messages and is idempotent; an unknown ID fails with `fleet 'X' not found.`, exit 1. One operation-wide five-second deadline bounds the whole command, extended by the time each Hermes work settlement takes (§6.3 `member delete`).

1. **Lock.** Lock every active concrete placement in sorted key order and revalidate the placement set, retrying contention every 50 milliseconds; a set change releases every acquired guard and restarts within the same deadline. Contention until the deadline releases every acquired guard, leaves every pane and registry row unchanged and exits 1 with `fleet ID pane input remained busy for 5 seconds at member MEMBER_ID; fleet was not deleted.`
2. **End members.** Under the held locks, end each active ordinary member with a concrete pane, in lock-key order, through the `member delete` procedure: runner shutdown and recovery for a wrapped member, pane close, that member's deregistration, then optional layout restoration. A run-less ordinary row is ended as §6.3 `member delete` specifies for a run-less row. The root Director's pane stays open.
3. **Deregister the fleet.** One transaction revalidates the placement set, stamps deletion, deregisters the remaining active members (the root Director and members with a pending or absent placement) and removes their placements.

The first member that cannot be ended stops the command, and the fleet is not deleted. Members ended before it stay deregistered with closed panes; the failing member and every later member keep their registry rows and panes, so `member list` still shows them. The command exits 1 with `fleet ID was not deleted: member MEMBER_ID: CAUSE. Members already deleted: IDS.`, where `CAUSE` is the member deletion cause and `IDS` is a comma-separated id list or `none`. A placement-set change after cleanup began exits 1 with `fleet ID placement or runtime ownership changed during cleanup; fleet was not deleted.` Rerunning `fleet delete` resumes with the members that remain.

On success the text output is `Deleted fleet ID. Deregistered N members.` followed by one `  member MEMBER_ID: PANE_STATUS` line per ordinary member in ascending id. `N` counts every member the command deregistered, root Director included; `PANE_STATUS` takes the `member delete` values `PANE (killed)`, `(pending — no pane)` and `(no placement)`. JSON is ordered `fleet_id, deregistered_count, members`, with one `{member_id, pane_status}` entry per ordinary member in the same order.

#### `cafleet doctor` — Environment Diagnosis

A full-environment diagnosis that reports **every** check, with no early abort when the multiplexer is unavailable. `doctor` is exempt from the schema-version guard (§6.3 Schema-version guard), so a missing or incompatible database is reported, not fatal. Its optional flags are `--verbose`, whose help reads `Print one row per individual check instead of the per-component summary`, and `--json` (§6.3 JSON output (`--json`)).

| Invocation | stdout |
|---|---|
| `cafleet doctor` | Summary lines (§ Summary lines) |
| `cafleet doctor --verbose` | Per-check rows (§ Verbose rows), no summary |
| `cafleet doctor --json`, with or without `--verbose` | `{"summary":[…],"checks":[…]}` (§ Doctor JSON) |

Statuses are `ok`, `info`, `warn` and `error`. After printing, doctor exits 1 with
`Error: doctor found errors` when any check has status `error`; otherwise it
exits 0, `warn` included.

##### Check rows

Each check has a stable `name`, a `status` and a one-line `detail`. Checks run in table order; the rows from `<agent>.configuration` through `<agent>.installation` repeat for each backend in registry order, each backend emitting only its own preset and extension rows.

| Check name | `ok` | `info` | `warn` | `error` |
|---|---|---|---|---|
| `multiplexer` | `<backend> session=<session> window=<window_id> pane=<pane_id>` | — | — | The multiplexer resolution, availability or context-discovery error |
| `database` | `cafleet <VERSION> at <path>` | — | — | The schema-version guard error for an empty or incompatible database, or the open/inspection cause |
| `<agent>.configuration` | The resolved config path (§ Config-dir resolution) | — | — | The config-path validation error, or `HOME is not set` |
| `<agent>.executable` | `<binary> is on PATH` | `<binary> is optional and absent from PATH` | — | — |
| `opencode.preset[.<mode>]` | § OpenCode preset checks | § OpenCode preset checks | — | § OpenCode preset checks |
| `opencode.extension`, `hermes.extension`, `hermes.plugin_enabled`, `pi.extension` | § Extension checks | § Extension checks | — | § Extension checks |
| `<agent>.installation` | `recorded <version> at <installed_at>` when the version at the resolved path equals the CLI version | `unavailable: configuration failed: <cause>`; `unavailable: <database error>` for a database that is not current | `recorded <version> at <installed_at>; run 'cafleet setup --coding-agent <agent>'` for another version; `no installation recorded at <path>; run 'cafleet setup --coding-agent <agent>'` | `installation query failed: <cause>` |
| `member_runtime.<run_id>`, `member_runtime` | § Member runtime checks | § Member runtime checks | § Member runtime checks | § Member runtime checks |

A recorded version is compared by string equality. Installation records are
read only at the resolved path of a current database; a row recorded at
another path is not the resolved installation.

##### Summary lines

The default text output prints one `<status> <component>: <detail>` line per
component, with each LF in the detail escaped as `\n` and each CR as `\r`.
Components appear in check order: `multiplexer`, `database`, each backend in
registry order, then the `members` lines (§ Member runtime checks).

| Component | `ok` detail | Problem line |
|---|---|---|
| `multiplexer` | `<backend> (session <session>, pane <pane_id>)` | `error` with the check's cause |
| `database` | `cafleet <VERSION>` | `error` with the check's cause, which names `run 'cafleet setup'` |
| `<agent>` | `ready` | The backend rules below |

A backend line folds that backend's `configuration`, `executable`, OpenCode `preset`, extension and `installation` checks under the first matching rule:

1. A `configuration` check with status `error` prints `error <agent>: <cause>`.
2. An absent `executable` prints `info <agent>: not installed (<agent> is not on PATH)`; the other checks of an unused backend raise nothing.
3. Otherwise each non-`ok` check contributes a problem phrase, and the line takes the worst contributed status (`error` > `warn` > `info`). The detail joins the phrases with `; ` in check order, listing a phrase identical to an earlier one once (a missing Hermes home reads `info hermes: no Hermes home at <base> — run 'cafleet setup --coding-agent hermes'`), and ends once with ` — run 'cafleet setup --coding-agent <agent>'` when any phrase carries the setup fix.

| Check state | Status | Phrase | Setup fix |
|---|---|---|---|
| OpenCode preset missing or unreadable | `error` | `preset <path> is unreadable: <cause>` | yes |
| Extension missing, unreadable or differing | `error` | `extension <path> is missing or differs from the embedded copy` | yes |
| Executable record unusable | `error` | `extension executable record <path> is unusable: <cause>` | yes |
| Hermes config path unreadable | `error` | `cannot read hermes configuration through 'hermes config path': <cause>` | no |
| Hermes plugins unreadable | `error` | `cannot read hermes configuration through 'hermes config get': <cause>` | no |
| No Hermes home at the resolved base | `info` | `no Hermes home at <base>` | yes |
| Hermes plugin listed in `plugins.disabled` | `error` | `hermes plugins.disabled lists cafleet` | no |
| Hermes plugin not enabled | `error` | `hermes plugin cafleet is not enabled: <key>, …` | yes |
| Installation recorded at another version | `warn` | `assets from cafleet <version> (this is <cli version>)` | yes |
| No installation recorded | `warn` | `assets not installed at <path>` | yes |
| Installation unavailable because the database is not current | `info` | `assets unchecked (database unavailable)` | no |
| Installation query failed | `error` | `installation query failed: <cause>` | no |

For example:

```
ok claude: ready
warn codex: assets from cafleet 0.30.0 (this is 0.31.1) — run 'cafleet setup --coding-agent codex'
info opencode: not installed (opencode is not on PATH)
error hermes: hermes plugin cafleet is not enabled: plugins.enabled — run 'cafleet setup --coding-agent hermes'
```

##### Verbose rows

`--verbose` prints one `<status> <name>: <detail>` row per check in check
order, with the same LF/CR escaping. Member runtime rows follow § Member
runtime checks.

##### Doctor JSON

`--json` prints one object with `summary` and `checks` arrays:

```json
{"summary":[
  {"component":"multiplexer","status":"ok","detail":"herdr (session w2, pane w2:p2N)"},
  {"component":"members","status":"warn","detail":"20 lost runtime: 1-6, 9-22 — run 'cafleet member recover ID --from-member-id 2' or 'cafleet member delete ID'","state":"lost_runtime","member_ids":[1,2,3,4,5,6,9,10,11,12,13,14,15,16,17,18,19,20,21,22]}
 ],
 "checks":[
  {"name":"multiplexer","status":"ok","detail":"herdr session=w2 window=w2:tP pane=w2:p2N"},
  {"name":"member_runtime.9","status":"warn","detail":"member 9 lost runtime: …","member_id":9,"state":"lost_runtime"}
 ]}
```

- Summary entries are ordered `component, status, detail`; `members` entries add `state, member_ids`.
- Check entries are ordered `name, status, detail`; `member_runtime.<run_id>` and `member_runtime` entries add `member_id, state`.
- `summary` holds exactly the lines the text summary prints, and `checks` holds exactly the `--verbose` rows, `ended` rows included.
- Each detail equals its text form without the `<component>: ` or `<name>: ` prefix; the verbose member-row prefix `member <id> <label>: ` stays.

###### OpenCode preset checks

`doctor` has no selected launch policy, so it checks the preset `<preset base>/agents/<mode>.md` of every OpenCode launch-policy choice (§6.9 Launch policy and capability registry), in catalog order. The default choice's check is `opencode.preset`; every other choice's is `opencode.preset.<mode>`. After the shared `info` states of § Extension checks, a readable preset is `ok` with its path. Setup installs every preset, so a missing or unreadable one is `error` `<path>: <cause>; run 'cafleet setup --coding-agent opencode'`: the installation predates that preset or was altered.

An OpenCode installation recorded only at another base is not the resolved installation; doctor reports through the checks at the resolved base, `~/.config/opencode` with `OPENCODE_CONFIG_DIR` unset: `opencode.configuration` is `ok` with that base, `opencode.installation` is `warn` `no installation recorded at <path>; run 'cafleet setup --coding-agent opencode'`, and every preset check is `error`. Rerunning setup installs every preset at the resolved base and leaves the files at the other base in place.

###### Extension checks

Each extension check is read-only. An executable record is usable when it names an existing executable file. `opencode.extension` checks `<preset base>/plugins/cafleet.ts` and `<preset base>/plugins/cafleet.executable`; `hermes.extension` checks `<hermes home>/plugins/cafleet/`, located by the batch's `hermes config path`; `hermes.plugin_enabled` reads `disabled`, `enabled` and `entries.cafleet.allow_gateway_injection` from the batch's one bounded `hermes config get --json plugins` call under the *not set* and subtree rules (§ Hermes plugin enablement); `pi.extension` follows `pi.executable`, precedes `pi.installation` and checks `<pi base>/extensions/cafleet.ts` and `<pi base>/extensions/cafleet.executable`.

`hermes.extension` and `hermes.plugin_enabled` share one concurrent read batch, `hermes config path` and `hermes config get --json plugins`, run with null stdin and `HERMES_HOME` set to the resolved base under one two-second deadline. The batch starts before the multiplexer, database, OpenCode and member checks and overlaps them; the rows keep their check order. A `hermes` call against a missing home makes Hermes create one, so when the resolved Hermes base does not exist doctor runs no `hermes` process.

A preset or extension check reports the first matching state:

| Check | State | Status | Detail |
|---|---|---|---|
| Every preset and extension check | The backend configuration failed | `info` | `unavailable: configuration failed: <cause>` |
| Every preset and extension check | The backend executable is absent from `PATH` | `info` | `unavailable: executable is absent` |
| `hermes.extension`, `hermes.plugin_enabled` | The resolved Hermes base does not exist | `info` | `unavailable: no Hermes home at <base>` |
| `hermes.extension` | `hermes config path` fails or times out | `error` | `cannot read hermes configuration through 'hermes config path': <cause>` |
| `opencode.extension`, `hermes.extension`, `pi.extension` | An embedded file's installed copy is missing, unreadable or differs | `error` | `<path>: <cause or "differs from the embedded copy">; run 'cafleet setup --coding-agent <agent>'` |
| `opencode.extension`, `hermes.extension`, `pi.extension` | The executable record is missing, unreadable or names no executable file | `error` | `<record path>: <cause>; run 'cafleet setup --coding-agent <agent>'` |
| `opencode.extension`, `hermes.extension`, `pi.extension` | Otherwise | `ok` | The plugin path; for Hermes, the plugin directory; for pi, the extension path |
| `hermes.plugin_enabled` | The call fails other than *not set*, times out or prints a member of the wrong JSON type | `error` | `cannot read hermes configuration through 'hermes config get': <cause>` |
| `hermes.plugin_enabled` | `plugins.disabled` lists `cafleet` | `error` | `hermes plugins.disabled lists cafleet; remove it, then run 'cafleet setup --coding-agent hermes'` |
| `hermes.plugin_enabled` | `plugins.enabled` lacks `cafleet`, or `allow_gateway_injection` is not `true` | `error` | `hermes plugin cafleet is not enabled: <key>, …; run 'cafleet setup --coding-agent hermes'` |
| `hermes.plugin_enabled` | Otherwise | `ok` | `cafleet enabled with gateway injection` |

##### Member runtime checks

With a compatible current database, doctor classifies every recorded member run in ascending `run_id` order in two passes over a read-only connection, mutating no registry or runtime state:

1. **Registry pass.** One query reads every run's `run_id`, `member_id`, run `status`, `cleanup_state` and the member's registry `status`. It runs no probes and sits outside the time budget.
2. **Probe pass.** Only runs the registry pass leaves unclassified are probed, up to 16 at a time: the launch-descriptor, run-path, owner-lock and health probes. One five-second budget covers the pass, and each run is bounded by the smaller of 100 ms and the remaining budget when its probe starts. Results keep ascending `run_id` order.

Each run gets the first matching state:

| # | Condition | State | Label | Status | Next command |
|---|---|---|---|---|---|
| 1 | The run's diagnosis, including its own read of the run record, returned an error, or the owner-lock probe failed | `check_failed` | `could not be checked` | `error` | `cafleet doctor --verbose` |
| 2 | Run `status` is `exited`, `failed` or `lost`, `cleanup_state` is `complete` and the member is deregistered | `ended` | `ended` (hidden from the summary) | `info` | — |
| 3 | Run `status` is `exited`, `failed` or `lost`, `cleanup_state` is `complete` and the member is active | `agent_exited` | `agent exited` | `warn` | `cafleet member delete ID` |
| 4 | Any other run of a deregistered member | `deleted_with_leftover_runtime` | `deleted with leftover runtime` | `warn` | `cafleet member recover ID --from-member-id <director-id>` |
| 5 | The budget was exhausted before this run was probed | `not_checked` | `not checked (time limit)` | `info` | `cafleet member show ID` |
| 6 | A launch descriptor is left over, or the run paths fail validation | `interrupted_launch` | `interrupted launch` | `warn` | `cafleet member recover ID --from-member-id <director-id>` |
| 7 | A health reply was observed | `running` | `running` | `ok` | — |
| 8 | The health probe was skipped because the deadline passed | `not_checked` | `not checked (time limit)` | `info` | `cafleet member show ID` |
| 9 | The owner lock is held and no health reply arrived | `not_responding` | `not responding` | `warn` | `cafleet member recover ID --from-member-id <director-id>` |
| 10 | The owner lock is free and no health reply arrived | `lost_runtime` | `lost runtime` | `warn` | `cafleet member recover ID --from-member-id <director-id>` or `cafleet member delete ID` |

The registry pass decides rows 2–4; the probe pass decides rows 1 and 5–10. A probed run's diagnostic phrases are:

| Observation | Phrase |
|---|---|
| A leftover launch descriptor | `launch descriptor left over` |
| A free owner lock | `owner lock is free` |
| A held owner lock | `owner lock is held` |
| A health reply | `runtime answered the health probe` |
| No health reply | `runtime did not answer the health probe: <cause>` |
| Run paths that fail validation | `runtime files are invalid: <cause>` |

**Summary lines.** Members are grouped by member ID; a member with several runs is listed once per distinct state. One line is printed per occupied state, in the order `running`, `not responding`, `lost runtime`, `interrupted launch`, `agent exited`, `deleted with leftover runtime`, `could not be checked`, `not checked (time limit)`:

```
<status> members: <N> <label>: <ID ranges>[ — run '<command>'[ or '<command>']]
```

`<N>` counts the distinct member IDs in that state. The ID ranges list those IDs ascending and de-duplicated, joined with `, `; three or more consecutive IDs collapse to `a-b`, while one or two consecutive IDs stay separate (`7, 8`). The next command keeps `ID` literal, because the line covers several members. A `member recover` command fills in `--from-member-id` with the fleet Director ID when every listed member shares one fleet Director, and keeps `<director-id>` literal otherwise; a verbose row fills in its member's fleet Director ID. For example:

```
ok members: 4 running: 7, 8, 23, 24
warn members: 20 lost runtime: 1-6, 9-22 — run 'cafleet member recover ID --from-member-id 2' or 'cafleet member delete ID'
```

**Verbose rows.** Each run produces one `member_runtime.<run_id>` row:

```
<status> member_runtime.<run_id>: member <member_id> <label>: <phrase>[; <phrase>…][ — run '<command with the literal member ID>']
```

A probed run joins its diagnostic phrases with `; `. A registry-pass row for state `agent_exited` or `deleted_with_leftover_runtime` uses the phrase `run <status>, cleanup <cleanup_state>`, and an `ended` row uses `run <status>, cleanup complete`. A `not_checked` row uses `runtime diagnosis time budget exhausted`. A `check_failed` row is `error member_runtime.<run_id>: member <member_id> could not be checked: <cause>`, where `<cause>` is the message of the diagnosis or owner-lock probe error, with no other phrase and no command suffix. For example:

```
ok member_runtime.7: member 7 running: owner lock is held; runtime answered the health probe
warn member_runtime.9: member 9 lost runtime: owner lock is free; runtime did not answer the health probe: member runtime: ECONNREFUSED: Connection refused — run 'cafleet member recover 9 --from-member-id 2' or 'cafleet member delete 9'
info member_runtime.3: member 3 ended: run exited, cleanup complete
error member_runtime.5: member 5 could not be checked: database is locked
```

**Run-independent results.** Each condition below prints one `members` summary line in place of the per-state lines. Its JSON entries carry `member_ids` `[]` and, for the check entry, `member_id` `null`:

| Condition | Summary line | Per-run rows | Run-independent check row | JSON `state` |
|---|---|---|---|---|
| No runs, or only `ended` runs | `ok members: none running` | `ended` rows only | — | `running` |
| The database is not current | `info members: not checked (database unavailable)` | None | — | `not_checked` |
| The registry query fails; no run is known and the probe pass is skipped | `error members: could not be checked: <cause>` | None | `error member_runtime: member runtimes could not be read: <cause>` | `check_failed` |

A registry-query failure still prints every other check and exits 1 with `Error: doctor found errors`. The recovery contract (§6.9 Storage pressure and durability boundary) owns the probes and their recovery actions.

#### `cafleet message` — Message Broker

##### `message send`

`cafleet message send --from-member-id ID --to-member-id ID (TEXT | --file PATH) [--json]`

Resolve the complete text body, including a `--file -` stdin wait, before starting the five-second persistence/notification budget. Persist one point-to-point delivery, then evaluate delivery for it by recipient, first match wins:

| Recipient | Delivery evaluation | `delivery` | `input_id` |
|---|---|---|---|
| The sender itself (equal sender and recipient IDs), even an ordinary member with a healthy wrapped pane | Persists only in the broker | `{"status":"not_applicable","method":null,"reason":"self_send"}` | `null` |
| Another sender's root Director | §6.5 Push notifications, with no pane-delivery diagnostic and no multiplexer requirement | `{"status":"not_applicable","method":null,"reason":"director_stdout"}` | `null` |
| An ordinary member | Drain its pending messages oldest-first into its concrete pane within the remaining budget, the new message included | The projection of this invocation's evaluation of the new message | The receipt's ID when allocated |

§6.5 Push notifications specifies the line, the drain and under-lock revalidation. A committed send exits 0 with its stdout record; a queued, held or unavailable ordinary-member notification leaves `notification_sent=false`. `notification_sent` is `true` exactly when this invocation submitted the newly committed message, including after draining an older one. When another invocation delivered it, `notification_sent` stays `false` with no stderr diagnostic; the recipient still received the line. An exhausted attempt preserves any committed sent prefix and reports the failure with its current receipt context. The next send to that recipient or a `message wait` (§6.3) rotation retries messages still pending; the runner retries a preview it accepted into its queue (§6.9 Runner-owned deferred delivery).

JSON is ordered `message,notification_sent,input_id,delivery`. `input_id` is the nullable durable input ID of the new message's preview receipt, and `delivery` is the shared delivery projection (§6.9 Delivery reporting) for the new message, even when the invocation also retries an older one. Text output appends the delivery and input lines of §6.3 Runtime presentation after the envelope.

###### Delivery outcome

A notification failure prints `Message NEW_ID was persisted. ` followed by its diagnostic on stderr; stdout and exit 0 are unchanged. Every per-attempt failure uses this diagnostic:

```text
message MESSAGE_ID was not notified to member MEMBER_ID: CAUSE; the next send to member MEMBER_ID or the next message wait retries it.
```

| Failure | `CAUSE` |
|---|---|
| Recipient ordinary member has no recorded run; the message stays pending and the failed-attempt `delivery` has `method: null` | `member MEMBER_ID has no recorded run` |
| The pane input lock stays busy through the lock wait (§6.5 Pane input lock) | `pane input remained busy` |
| Runner lock, transport or submission failure | The runner error text with known run/input context |
| Database busy or deadline exceeded during the under-lock snapshot or notification stamp | The storage error text; names the current candidate, or the candidate from placement revalidation when selection has not completed |
| `message wait` attempt fails with database busy or deadline exceeded before selecting a candidate | The storage error text; names the selected rotation entry |

Other failures, including corrupt, incompatible or unreadable storage and a termination-span start failure, retain their causes after the persisted prefix. Storage failure before commit exits 1 with its cause.

##### `message poll`

`cafleet message poll MEMBER_ID [--json]` returns only incoming `input_required` records, descending by status timestamp/message ID. Unknown or inactive member: `Member ID not found`, exit 1.

##### `message wait`

`cafleet message wait MEMBER_ID [--timeout SECONDS] [--json]`

Timeout is an integer in `1..=1800`, default `300`. Validate an active subject and live fleet. Existing unread deliveries return immediately; otherwise check at one-second monotonic intervals. The invocation is a blocking receive for scripts and manual use and never ACKs messages.

Between inbox checks, retry pending deliveries for at most one ordinary member per interval with an in-memory round-robin cursor. Select concrete ordinary-member placements in the fleet, excluding its root Director, the wait's own subject, self-notifications, ACKed and already-notified rows. Each attempt drains that member's pending messages under the delivery procedure (§6.5 Push notifications) without consulting pane state. One invocation attempts each pending message at most once; a later send or wait invocation retries a failed message, and later messages for the same member remain eligible. Recheck the inbox before starting another attempt. Shared input locks and under-lock revalidation keep multiple waiters safe.

Start each receive check or notification attempt before the monotonic receive deadline. An attempt already in flight retains its original five-second budget for runtime locking, revalidation, submission and receipt settlement. A lock still busy at that attempt's bound records the not-notified diagnostic (§6.3 Delivery outcome) in `notification_errors`. Cap subprocess/database waits and staged input to the remaining original budget. Mandatory database finalization follows §6.5 Push notifications. A receive returns by its deadline plus at most one six-second in-flight attempt/finalization allowance. A snapshot blocked until receive expiry returns `timeout` with retained notification diagnostics; earlier transient contention can retry before expiry. Corruption and other fatal storage errors retain their causes.

The ordered JSON result is:

```json
{"member_id":41,"outcome":"timeout","messages":[],"notification_errors":[]}
```

`outcome` is `messages` with nonempty full poll records, or `timeout` with an empty array. Each diagnostic has ordered keys `{member_id, message_id, error}`. Keep one diagnostic per not-notified message, ordered by message ID.

Pending placement and a placement change under the lock are ordinary holds that keep entries unchanged. A drain that notifies a recorded message removes that message's entry. Reconcile diagnostics with each successful receive snapshot and the typed observations of its owned notification attempt. Remove an entry when they show its message ACKed, its recipient gone or the message notified by any invocation. After receive expiry, return the last successful snapshot reconciled with the completed attempt's observations. Subsequent external changes become visible to the next invocation; final reconciliation starts no additional I/O or tail budget.

Text is `Messages available for member ID.` plus normal messages, or `Timed out waiting for member ID.`; diagnostics go to stderr. Both outcomes exit 0. Deleted subject/fleet or corrupt/unreadable state exits 1 with its cause and retains earlier notification diagnostics on stderr, including when JSON output was requested. Ctrl+C outside a delivery attempt terminates the process normally; inside an attempt it takes effect when the attempt's termination-deferral span (§6.5 Termination during pane input) ends.

##### `message listen`

`cafleet message listen MEMBER_ID [--timeout SECONDS] [--codex-thread THREAD]`

Stream any active member's pending inbox as UTF-8 JSONL. Derive and pin its fleet at startup through the configured database and the ordinary schema-version guard. Require a live root Director with a concrete registered placement, without resolving a multiplexer or checking its executable/physical pane; the subject's own placement may be absent. Unknown/inactive startup subjects use `Member ID not found`, exit 1.

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

The inner message is exactly the full message record (§6.4 Persisted shape), preserving original text beyond CLI/pane limits. `closed.reason` is `timeout`, `member_inactive`, `fleet_deleted` or, only in forwarding mode (§ Codex forwarding), `session_exited`; fleet deletion takes precedence when both lifecycle closures are observed. Check normal closures before revalidating a continuing live fleet. Missing persisted rows, changed fleet identity, invalid stored enums/root relationships or missing root placement are fatal errors.

Each short read transaction validates identity and selects every `input_required` record for the pinned subject ordered by `(created_at ASC, message_id ASC)`, committing before output or sleep; a later commit with an earlier timestamp emits when first observed. A process-local set holds emitted IDs still pending in successful snapshots: an ID enters only after its complete record and LF flush succeed, a failed snapshot leaves the set unchanged, and a set member is not emitted again. A snapshot that emits nothing waits one second; one that emits rechecks immediately.

Listening leaves consumption and `notified_at` unchanged and performs no pane retry, lock or dispatch; stamped pending self-sends remain stream-eligible. Concurrent listeners are independent read-only observers with no consumer lock, and a restart replays every pending ID. ACK before a snapshot removes a message, while ACK racing with buffered output may leave a stale pending envelope.

Each snapshot has a fresh database deadline at most five seconds away, shortened by the remaining lease. Scoped busy waiting and an execution progress handler every 1,000 VM instructions bound lock/SQL work, checking the deadline before/after prepare and execution, before/after decoding each row and before commit. Expiry discards a partial snapshot; interrupted SQL maps to deadline exceeded only when that deadline expired. Every path restores busy handling and removes the listener-owned progress handler, preserving cleanup failures alongside the original cause. These cooperative boundaries cover row collection but give no hard real-time preemption of one allocation or SQL operation.

Output writes one serialized record at a time and applies backpressure before selecting more messages. Nonblocking filesystem/poll output preserves/restores descriptor flags on normal return, resumes partial writes at their byte offset and checks every write/flush, under a five-second deadline shortened by the lease. Broken pipe, write-zero, flush failure or output-deadline exhaustion exits 1 visibly, even after partial records; normal lease expiry between records writes `closed` with a separate bounded final write. Complete records survive a later error; an incomplete final line stays unconsumed and recovers through broker replay. Listener memory holds only the current snapshot, one record and the pending emitted IDs.

Database contention retries stay inside the snapshot budget; exhaustion or another fatal error ends the stream. The listener keeps its connection/PID across healthy events and ordinary signal behavior outside pane termination deferral; host cancellation may end it without `closed`. Native owners retain one handle, reconcile durable state after failure and establish old-process termination before replacement. Installed backend instructions own concrete native tool recipes and their evidence limits.

###### Codex forwarding

`--codex-thread THREAD` selects forwarding mode, which keeps the record, ordering, replay, lease and idle contracts and changes the output:

- **Stdout** carries only the `ready` record and the final `closed` record, so an unread PTY never stalls the five-second output deadline. Writing that `closed` record, with any reason including `session_exited`, is the listener's last action before exit; a write failure leaves the exit status unchanged.
- **Forwarded events.** Each `message` record, and each `closed` record whose reason is `timeout`, `member_inactive` or `fleet_deleted`, is forwarded in emission order. `ready` and `closed` `session_exited` are never forwarded.
- **Forwarding command:** `codex queue --thread THREAD --message <text>`, where the text is `[cafleet listen record] <compact record JSON>`.
- **Large records.** When that text exceeds 65,536 bytes, the forwarded text is the reference below instead:

  ```
  [cafleet listen record] {"event":"message_ref","message_id":<id>,"from_member_id":<id>,"bytes":<n>} — read it with: cafleet message show <id> --json
  ```

  `bytes` is the byte length of the text the reference replaces.
- **One at a time.** Each `codex queue` runs to exit, with a 10 s deadline, before the next record is forwarded.
- **Full queue.** Codex refuses a submission while the thread's queue holds 100 (`queue cannot contain more than 100 submissions`). A `codex queue` that exits non-zero with a stderr line containing `queue cannot contain more than` is a *full-queue refusal*, not a failure: the listener keeps the same record, waits 1 s, checks the pinned session (§ Liveness) and retries it. Later records wait behind it, so emission order holds and no record is dropped. The pinned session's exit during the wait ends the listener with `session_exited`.
- **Full-queue limit.** When one record meets full-queue refusals for 600 s without being accepted, the listener fails.
- **Binary resolution.** `codex` resolves through the listener's `PATH` with the absolute-entry search of §6.9 Native executable resolution.

| Condition | Behavior |
|---|---|
| `codex` not on `PATH` at startup | `Error: binary codex not found on PATH`, exit 1, before `ready` |
| No Codex session ancestor at startup | `Error: --codex-thread requires a codex ancestor process`, exit 1, before `ready` |
| `codex queue` exits non-zero, other than a full-queue refusal, while the pinned session is alive | stderr `failed to forward record to codex thread <THREAD>: <codex stderr>`, closed notice, exit 1 |
| Full-queue refusals for one record reach 600 s | stderr `codex thread <THREAD> queue stayed full for 600 s`, closed notice, exit 1 |
| `codex queue` exceeds 10 s while the pinned session is alive | kill it; stderr `codex queue for thread <THREAD> did not finish within 10 s`, closed notice, exit 1 |
| Any other failure after `ready` (database, schema guard, deadline) | its existing stderr diagnostic, closed notice, exit 1 |
| The pinned Codex process exits, including when the listener checks it first after a `codex queue` exits non-zero or exceeds 10 s | stdout `closed` with reason `session_exited`, exit 0; no diagnostic, no closed notice and nothing forwarded |

**Closed notice.** Before every non-zero exit after `ready`, the listener makes one best-effort attempt, bounded by 10 s, to forward `[cafleet listen closed] exit 1: <stderr diagnostic>` with `codex queue`; the attempt can itself meet the full queue and is not retried. When that attempt also fails, stderr keeps the original diagnostic and adds `failed to forward the closed notice to codex thread <THREAD>: <cause>`, and the exit status stays 1.

**Session pinning.** At startup the listener walks its ancestors, starting from its parent, reading `/proc/<pid>/stat` (ppid) and `/proc/<pid>/cmdline` on Linux and `ps -o ppid= -o args= -p <pid>` elsewhere. It pins the nearest ancestor whose argv[0] basename is exactly `codex`; shells, `bwrap`, node launchers and the sandbox helper (the same binary, run with argv[0] `codex-linux-sandbox`) are skipped. The executable path is not compared, because the helper shares it. A sandbox that unshares the PID namespace hides the ancestor, and startup fails with the missing-ancestor error.

**Liveness.** Each idle tick (at most 1 s) checks the pinned pid with `kill(pid, 0)`. On Linux it also compares the pinned process's start time from `/proc/<pid>/stat`, so a reused pid counts as exited. The pinned process's exit ends the listener with `session_exited`. The listener sets no `PR_SET_PDEATHSIG`, which fires when the forking thread exits rather than the multi-threaded Codex process.

##### `message ack`

`cafleet message ack MESSAGE_ID [--json]` transitions an existing unread message to completed, updating its status timestamp. The recipient/fleet derive from that row. ACK signifies consumption (§6.4 Persisted shape).

##### `message show`

`cafleet message show MESSAGE_ID [--json]` returns one full record. Existence is the read guard.

#### `cafleet member` — Member Lifecycle + Pane Interaction

##### Member targeting and key delivery

Require an active member and placement for pane operations; concrete pane IDs remain opaque. Pending placement is valid during creation and yields contextual prompt/exec/capture errors. Missing placement fails pane actions; list/show retain diagnostic nulls. Individual member deletion protects the root Director. Before pane I/O, prompt and exec reject the root Director (bodies at each command) and an ordinary member without a recorded run (§6.3 Error Messages). They submit to the member's runner, which owns the pane input lock for actual stages (§6.5 Pane input lock) and selects the command's submission plan, delivery fallback or queueing (§6.9 Delivery method and interaction priority); exit 0 establishes complete submission or queue acceptance only.

##### `member create`

Register a member **and** spawn its coding-agent pane. A fleet has exactly one root Director, so the command takes no identity or override flag: it first resolves the acting Director from `fleets.director_member_id`, before registration, and uses that id for backend inheritance and spawn-prompt substitution. Per-backend child argv and omitted permission defaults are in §6.7 Per-backend `build_spawn_argv` (exact, token-by-token).

| Argument | Required | Notes |
|---|---|---|
| `--fleet-id` | yes | The fleet the new member joins. |
| `--name` | yes | Display name; per-backend pane-title behavior: §6.7 Backend capabilities. |
| `--description` | yes | One-sentence purpose. |
| `--coding-agent` | no | A backend registry token; when omitted, the member inherits the spawning Director's placement backend. Availability failures exit 1 (below). |
| `--model` | no | Forwarded to the backend binary's own `--model` flag; opencode requires `<provider-id>/<model-id>`, and pi refuses a `:<thinking>` suffix. Per-backend formats and create-time validation: §6.7 Model selection. |
| `--permission-mode MODE` | no | Launch-only selection from the resolved backend’s policy catalog; omission selects the backend's default mode (§6.9 Launch policy and capability registry). |
| `--event-evidence POLICY` | no | `metadata` by default, or explicit `redacted` evidence under the privacy contract (§6.9 Persistence, notification and read interfaces). |
| `--event-redaction-file PATH` | for redacted | Absolute readable UTF-8 redaction configuration; invalid combinations/configuration fail with usage exit 2 before registration. |
| `--effort` | no | Reasoning-effort level forwarded to the backend binary, validated per backend before any side effect (levels, forwarding forms and rejection strings: §6.7 Reasoning effort). |
| positional `PROMPT` | one of | Inline spawn prompt (backend-neutral template). |
| `--file PATH` | one of | UTF-8 spawn-prompt file (`-` = stdin) under the shared body-input and transport contract (§6.3 Text bodies (positional `TEXT` / `--file`)). |
| `--json` | no | Output as JSON |

A missing backend binary exits 1 with `Error: binary <name> not found on PATH`. The opencode backend also requires the selected launch policy's agent preset `<preset base>/agents/<mode>.md` at the resolved preset path (§ Config-dir resolution); a missing or unreadable preset or an invalid config-location variable exits 1 (§6.3 Error Messages). The hermes backend also resolves its effective home through `hermes config path`, concurrently with the native version probe (§6.7 Hermes availability preflight). The pi backend also requires the CAFleet pi extension `<pi base>/extensions/cafleet.ts`; a missing or unreadable extension exits 1 (§6.7 Contract error strings), and a pi prompt beginning with `@` exits 2 before registration (§6.7 Per-backend `build_spawn_argv`).

The split reference — session, window and Director pane — and the multiplexer server come from the fleet Director's recorded placement (§6.5 Multiplexer context); each new member placement copies the Director's `mux_server`.

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

The spawn always creates the pane without stealing focus (tmux `split-window -d`): the Director's pane and active window stay active. In the default output, `pane` renders `(pending)` until the pane id is patched onto the placement.

###### Creation failure compensation

Member creation validates policy, evidence configuration and assignment, and records the installed native version (§6.9 Native version probe: a failed probe warns and creation continues with unknown coverage), then registers a pending placement and run. A known version below the backend's first boundary exits 1 before member/run registration or pane creation (§6.9 Profile selection by minimum version). The multiplexer starts a runner from a private descriptor; a `prepared` handshake precedes placement publication and `start`, and creation returns only after child spawn produces `started` within one handshake deadline. §6.9 Runner boundary and creation transaction owns descriptor cleanup and uncertain-launch compensation. Spawn-prompt placeholders are validated before registration, so a placeholder error exits 2 with nothing registered. The backend owns a newly split pane until the split returns (§6.5 Pane ownership during creation).

After registration, compensation ends with the descriptor/run/member rollback: remove the pending descriptor, then mark the run `failed` and deregister the member in one transaction. When compensation succeeds, the member is deregistered, its placement removed and the run `failed`.

| Failure point | Compensation order |
|---|---|
| Pending member/run registration | The registration transaction rolls back, leaving no new member, placement or run |
| Runtime artifact preparation or prompt rendering after registration | The descriptor/run/member rollback |
| Runner pane split | The backend closes a pane it already identified, then the rollback |
| After the split: handshake, placement publication, `start` or the started-state check | Shut down the owned runner and child when the runner already owns the run, close the created pane, then the rollback |

For a `hermes` run whose child started, compensation runs the Hermes work settlement (§6.7 Hermes work settlement) before the owned runner shutdown, under its own 20-second budget; an unverified settlement appends its cause as a cleanup diagnostic.

Compensation continues past a failed cleanup and never re-closes a pane the backend already tried to close. Creation keeps the primary cause and its exit category and appends each cleanup diagnostic on its own stderr line:

| Cleanup outcome | Appended diagnostic |
|---|---|
| Failed split with no confirmed id (no other pane is inferred or closed) | `pane ID unknown; pane cleanup unconfirmed` |
| Backend close failure | `cleanup failed for pane <id>: <detail>` |
| Unconfirmed owned-child shutdown | Its shutdown diagnostics, or `owned child cleanup remains unconfirmed` when it reports none |
| Failed descriptor removal, run update or deregistration | Its own cause; a recorded run with remaining diagnostics keeps `cleanup_state=incomplete` |
| Panic during launch | `native launch unwound; performing owned compensation`, after the same compensation |
| Interrupted pending creation | `pending creation was interrupted` |

Once the runner reports `started` under the matching identity, creation removes the descriptor and emits its result; placement publication alone keeps the rollback armed. Output keys keep their documented order and nullability; runtime fields follow §6.3 Runtime presentation.

##### `member delete`

For a member with a recorded run, request owned runner shutdown under that identity before pane/registry cleanup, retaining explicit unconfirmed-child diagnostics and recoverable runtime records. A run-less ordinary row skips runner shutdown, closes its recorded pane under the deletion lock and deregisters, requiring no runtime. Attempt remaining event replay before deregistration only within the remaining operation deadline. Lock a concrete placement before kill/deregistration (a member without a concrete pane needs no lock), retrying contention every 50 milliseconds against one five-second deadline; expiry exits 1 with `member ID's pane input remained busy for 5 seconds; member was not deleted.` and leaves state unchanged. Preserve pane/registry errors and layout cleanup ownership.

For a `hermes` run, after the pane lock is held and before runner shutdown, run the Hermes work settlement (§6.7 Hermes work settlement) under its own 20-second budget; the deletion deadline moves later by the settlement's duration, so lock acquisition stays bounded by the original five seconds and the remaining steps keep the time they had when settlement began. An unverified stop exits 1 (§6.3 Error Messages) and retains the runner's cleanup records and the member row; a retry re-runs the settlement.

Revalidate pending or absent placement and deregister atomically. For a concrete pane, retain input ownership while closing it outside a SQLite write transaction, then revalidate and finalize the registry. After successful close, mandatory registry finalization uses the later of the original deadline and one second after finalization begins; actual database failures report that the pane was closed and preserve the database cause. Optional layout work follows mandatory finalization within the original deadline. A failed or expired restoration emits a stderr diagnostic while preserving successful deregistration.

##### `member show`

`cafleet member show MEMBER_ID [--json]` reads any active registry entry, placed or placementless (root Director included), with no multiplexer requirement. Text is the compact one-line row; an ordinary member's row appends the runtime text line, and the root Director's stays its registry row. The `--json` payload is the detailed view, including `kind` (`director` / `member`, §5.4 Member kind discriminator), the placement sub-dict and the appended runtime summary (§6.3 Runtime presentation).

##### `member list`

`cafleet member list FLEET_ID [--json]` lists every **active** registry entry of the fleet (root Director, ordinary members and placementless rows) and takes no identity flag; an empty roster prints `0 members.`. Field projection and absent values: §6.3 Member list output; rows and activity: §6.2 Members. Text idle values render as `Ns` / `Nm` / `Nh`. Per-member `description` and `registered_at` are on §6.3 `member show`.

##### `member prompt`

`cafleet member prompt MEMBER_ID --from-member-id DIRECTOR_ID TEXT [--json]` validates a nonblank line without CR/LF or C0/DEL controls, otherwise exits 2 with `text must be one non-empty line without control characters.`

A root Director target exits 1 with `cannot prompt the Director's own pane`. The runner's submission plan is profile delivery (§6.3 Member targeting and key delivery); success returns ordered `{member_id, pane_id, text, input_id, delivery}`.

A submitted prompt prints `Sent prompt TEXT_REPR to member NAME (PANE).`; a queued prompt prints `Queued prompt for member ID (PANE).` Both add the delivery and input lines (§6.3 Runtime presentation).

Pre-write error, including a held runner reply: `prompt was not dispatched to member ID: CAUSE`. Partial uncertainty: `prompt delivery to member ID is uncertain: CAUSE. Inspect the pane before retrying.` These exit 1 without a success record.

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

Missing/extra positional arguments and a nonpositive/malformed member ID use the parser's native usage errors; key validation uses these bodies, with one-based `N` and JSON-quoted invalid input:

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
| Selected adapter differs from placement | `press backend does not match the recorded placement` |
| Selected live context differs from placement | `press target context does not match the recorded placement` |

A missing member or placement, a pending pane (§6.3 Error Messages) and a native process error report their own primary diagnostic. Binary-not-found, not-started and prelaunch expiration have zero writing invocations. No started failure, including a pane-gone response, is retried automatically. Request-bound permission decisions retain the separate `permission respond` contract and host authorization.

##### `member capture`

`cafleet member capture MEMBER_ID [--lines N] [--ansi] [--json]`

- `--lines`: Number of trailing lines to capture (default: **20**).
- `--ansi`: Preserve ANSI escapes in the raw capture. The default strips ANSI escapes and cleans carriage-return redraws.

Output shapes: §6.3 Output shapes; target resolution: §6.3 Member targeting and key delivery. A pending placement is a hard error (§6.3 Error Messages).

JSON stamps `captured_at` from the local UTC clock at the capture read boundary and sets `content_sha256` to the lowercase SHA-256 hex of the exact emitted `content`'s UTF-8 bytes, including Unicode and empty captures: the ANSI-stripped, carriage-return-defragmented string by default, or the ANSI-preserving string with `--ansi`, with no later normalization. Capture content is never stored in SQLite. Text capture prints content without adding a newline.

#### Runtime presentation

Member create and member show append `runtime` to their ordered `member_id,name,registered_at,placement` and `member_id,name,description,status,registered_at,kind,placement` keys respectively. Member creation records the member's run in its registration transaction, so every ordinary member CAFleet registers has a recorded run. `runtime` is `null` for the root Director and for an ordinary row without a run, a registry state CAFleet never writes, which send, prompt, exec, recover and delete handle as specified at each command. The runtime object has ordered keys `run_id,resolved_mode,status,reachability,owner_epoch,last_health_at,coverage,current_interaction_id,unresolved_input_ids,owner_kind,owner_pid,owner_start_identity,runner_pid,runner_start_identity,child_pid,child_start_identity,effective_policy`, preserving stored nullability. `effective_policy` is the stored effective policy record (§6.9 Effective policy record) or null. `unresolved_input_ids` lists, in input-ID order, the run's `in_progress`, `accepted` and `uncertain` inputs and its `ready` inputs carrying deferred metadata: `in_progress` and `uncertain` inputs need Director reconciliation, an accepted input needs none and stays listed until a settlement records its disposition, and a deferred ready input stays listed while the runner owns it in its queue. These are registry reads and make no live-health claim.

Wrapped create/show prints its `ID NAME backend=AGENT pane=PANE` or `ID NAME STATUS` row and appends `Runtime: run=RUN_ID mode=MODE status=STATUS coverage=COVERAGE review=STATE`, where `STATE` is the effective policy's `automatic_review` value, `-` when absent. `event show` uses the same runtime object and line. Prompt/exec/send JSON appends `delivery` after `input_id` (§6.9 Delivery reporting). Their text output appends `Delivery: status=STATUS method=METHOD reason=REASON`, using `-` for a missing method or reason, then `Input: INPUT_ID` when a durable ID exists. The `Sent prompt …` and `Dispatched native command …` lines print only for actual submissions; a fallback submission always prints `method=fallback` with reason `submission_plan_unavailable` or `idle_timeout`, and a queued result names `interaction_open`, `interaction_candidate`, `composer_busy` or `earlier_input`, states that the runner will retry and points to `cafleet input show INPUT_ID --json` for the eventual result. An uncertain error exits 1, names the receipt's recorded `uncertain_cause` and the selected method, as in `native command delivery to member 110 is uncertain: manual_input (method=profile)`, and includes known receipt identity as `input_id=INPUT_ID`; run/interaction context accompanies held/rejected input.

#### Runtime event and input commands

`cafleet event show EVENT_ID [--json]` returns ordered `event,notification_state,message_id,archive_reason,run,interaction`. The event is the complete versioned runtime payload (§6.9 Persistence, notification and read interfaces), run is the current runtime summary, and nullable interaction contains the stored interaction fields in the interaction field order below. Text is `Event: EVENT_ID run=RUN_ID seq=EVENT_SEQ kind=KIND notification=STATE message=MESSAGE_ID` followed by the runtime summary line and `Interaction: INTERACTION_ID classification=CLASS closed_at=TIMESTAMP close_reason=REASON` when linked.

`cafleet input show INPUT_ID [--json]` and `cafleet input reconcile INPUT_ID --from-member-id DIRECTOR_ID --disposition DISPOSITION --evidence-ref REFERENCE [--json]` return `{input}` with the durable row ordered `input_id,run_id,operation_key,kind,payload_digest,state,created_at,updated_at,message_id,disposition,evidence_ref,permission_response,delivery,deferred_at,defer_reason,uncertain_cause`. The nullable audit object and additional text line follow the permission contract (§6.3 Permission-response audit output). `delivery` is the ordered `status,method,reason` projection of the stored receipt (§6.9 Delivery reporting), or null for a permission response; `deferred_at` and `defer_reason` are null together unless the runner queued the input. `uncertain_cause` is the cause the runner recorded when the input became `uncertain` (§6.9 Uncertain cause), retained through settlement, or null when no cause was recorded. Text is `Input: INPUT_ID run=RUN_ID kind=KIND state=STATE disposition=DISPOSITION message=MESSAGE_ID`.

Reconciliation is Director-owned. CLI disposition tokens are `submitted`, `not-submitted` and `abandoned-uncertain`, persisted with underscores as defined by the receipt contract (§6.9 Durable input receipts and settlement). Require a nonempty evidence reference. A settled ordinary receipt with the same disposition returns its original row and evidence reference before live work; a conflicting disposition fails unchanged. Settlement sends no input and establishes no native command result. One original five-second budget covers the pane wait, current-owner synchronization, supplied-connection SQL and socket exchange. Owner synchronization, CAS settlement and the receipt-only exclusive ownership path for an unreachable writer follow §6.9 Durable input receipts and settlement; uncertainty persists while an active writer cannot be ruled out or the proof is unavailable.

All event/input IDs are positive database-assigned integers; malformed IDs/options fail with usage exit 2. Missing rows and invalid stored state fail explicitly. Event and input text render `-` for an unavailable optional value. Interaction fields are ordered `interaction_id,run_id,episode_seq,classification,evidence_digest,opened_at,evidence_json,closed_at,close_reason`.

##### `member recover`

`cafleet member recover MEMBER_ID --from-member-id DIRECTOR_ID [--json]` is Director-owned and reconciles a recorded run of an active or deregistered member. A member without a run fails explicitly before mutation. §6.9 Storage pressure and durability boundary owns identity, ownership, replay, archival and its bounded pass.

The exceptional form is `cafleet member recover MEMBER_ID --from-member-id DIRECTOR_ID --owner-terminated --expected-run-id RUN_ID --expected-owner-epoch EPOCH --evidence-ref REFERENCE [--json]`. Require all four options together, positive snapshot IDs/epoch and a nonempty operator evidence reference; incomplete or invalid input exits 2. A changed owner rejects the attestation, and positive live evidence preserves the live owner.

Ordered JSON keys are `member_id,run_id,outcome,replayed_count,inbox_count,archived_count,status,cleanup_diagnostics`. Outcome is `recovered`, `recovering`, `live_or_unreachable` or `operator_reconciliation_required`; diagnostics retain cleanup/attestation provenance. Counters measure work newly committed by this invocation under the runtime recovery count domains below. A matching completed terminal return validates supplied current run/epoch expectations and preserves existing history with zero counts. Text is `Recovery: member=MEMBER_ID run=RUN_ID outcome=OUTCOME status=STATUS replayed=COUNT inbox=COUNT archived=COUNT`, followed by each bounded diagnostic as `Cleanup: DIAGNOSTIC` in array order; a string diagnostic renders its contained text and a structured one compact JSON. Incomplete or unreachable outcomes remain distinct from confirmed recovery.

#### Permission commands

`cafleet permission inspect INTERACTION_ID --event-id EVENT_ID --from-member-id DIRECTOR_ID [--json]` reads the current request through its pinned runner. Require positive IDs, then an existing interaction (an unknown `INTERACTION_ID` is reported as not found before any event, run or database-identity check), then an active ordinary target, pinned active Director and live fleet; verify the event/message association and interaction ownership under the member's pane-input lock (§6.5 Pane input lock). One original five-second deadline, clipped to any inherited SQL bound, covers locking, current observation/manual progress, database work and the control exchange. Inspection sends zero programmatic child bytes and creates no receipt, audit or claim. Unknown answer capability is an inspection result; stale ownership or an unavailable owner is an operational error.

The inspection JSON is `{request}`. Its object has ordered keys:

```text
event_id,message_id,member_id,fleet_id,director_member_id,run_id,owner_epoch,interaction_id,episode_seq,classification,visibility,backend,native_version,answer_rule_id,answer_capability,capability_reason,supported_decisions,supported_choices,request_token,output_seq,evidence,missing_evidence
```

IDs/epoch/episode are positive integers and `output_seq` an unsigned integer; `message_id` is the verified inbox message ID. `classification` is the current interactive class or null; `visibility` is `visible`, `obscured`, `not_visible` or `unknown`. `native_version` and `answer_rule_id` are null when unavailable. `answer_rule_id` is the unique common rule of the currently supported decision plans (permission request) or choice plans (question), so one supported decision or choice yields its actual rule and differing rules yield null; respond derives its chosen nonnullable rule internally for the immutable audit. The runner's reply and evidence carry the run's backend and native version. `answer_capability` is `tested` or `unknown`; `capability_reason` is null when the current request is answerable, otherwise a bounded explanation. `tested` requires a non-null run `native_version`, a non-null, non-blank `answer_rule_id` of at most 256 bytes, a token, a null capability reason, and a visible permission request with a non-empty decision set or a visible question with a non-empty choice set.

| Field | Permission request | Question |
|---|---|---|
| `supported_decisions` | The currently tested subset of `allow,deny`, in that order | `[]` |
| `supported_choices` | `[]` | One `{choice_id,label,scope}` object (keys in that order) per tested choice plan, in the rule's choice order; `label` is the extracted choice label and `scope` the declared choice scope (`single_request`, `session` or `persistent`) |
| `missing_evidence` | The missing public facts below | `[]`; the operation-centred facts do not apply to a question |

A blocked or unsupported request returns empty arrays and a null token. A question without a tested choice plan, such as a question of another backend or one whose rule declares no answer, returns `answer_capability=unknown` with `capability_reason` `question has no tested answer`.

`request_token` is a nullable random lowercase 64-hex value issued by the current runner, bound to the request and invalidated under the response contract (§6.9 Request-bound permission responses). `evidence` is the complete selected current post-policy object. For a permission request, `missing_evidence` lists unavailable public facts in order from `operation,arguments,cwd,environment,targets,effects,choice_scope`. Operation is available only when its post-policy value is nonblank, unredacted and untruncated, so metadata-only, `[redacted]` or truncated evidence leaves it missing. The current detector supplies no independent structured arguments, cwd, environment, targets or effects, and none are parsed from operation text, so those facts stay missing. `choice_scope` is listed whenever the public post-policy evidence has no explicit scope fact, even with a valid token and a proved private `single_request` plan. The Director obtains necessary missing facts through authorized current capture/routed evidence before deciding. Token identity and private plan eligibility are independent of public evidence completeness and authorization. The token binds the full immutable decision-to-plan or choice-to-plan set, including each rule and stage identity; a null common-rule field grants no authority to select a replacement plan.

Inspection text is:

```text
Permission: interaction=INTERACTION_ID event=EVENT_ID member=MEMBER_ID run=RUN_ID epoch=EPOCH visibility=VISIBILITY answers=ANSWERS
Request: token=TOKEN output_seq=SEQUENCE rule=ANSWER_RULE
```

`ANSWERS` joins with commas the supported decisions of a permission request or the supported choice ids of a question; unavailable values and an empty set render as `-`. Append `Capability: REASON` when nonnull and `Missing evidence: FIELDS` (comma-joined) when the array is nonempty. Text includes no raw screen content.

`respond` requires this complete request snapshot and decision provenance:

```text
cafleet permission respond INTERACTION_ID --event-id EVENT_ID --from-member-id DIRECTOR_ID --expected-run-id RUN_ID --expected-owner-epoch EPOCH --request-token TOKEN (--decision allow|deny | --choice CHOICE_ID) --decision-source director-auto|user --director-mode auto|manual|unknown --mode-evidence-ref REFERENCE --decision-evidence-ref REFERENCE --operation-key KEY [--json]
```

Exactly one of `--decision` (for a permission request) and `--choice` (for a question) is required. `CHOICE_ID` is 1–64 bytes of lowercase hyphen-separated words (`^[a-z0-9]+(?:-[a-z0-9]+)*$`). Require positive IDs/epoch, the token's exact hex form, a nonblank operation key and the bounded evidence references (§6.9 Permission response storage). Source `director-auto` requires mode `auto`; source `user` records the user's decision or choice in any mode. Enum/shape/source-mode errors exit 2 before lock or byte effects. These fields are attributed claims: resolve Director mode from current host evidence independently of member launch policy and obtain user permission in other/unknown modes. Native and managed rules still authorize running the command.

Validate a retained operation key against its recorded database/run, response identity and canonical digest before the admission drain and any current-request eligibility check, pane lock or live-owner probe. A matching retained receipt returns its stored result with zero bytes, even after token revocation, interaction closure, deregistration or owner exit, or while the current request is blocked; conflicting reuse (a conflicting decision/digest, or another key bypassing the episode claim) fails. Validate historical audit relationships against recorded ownership while preserving closed/inactive history. Only an absent key or a positively recorded ready zero-byte attempt enters current live-request validation.

The response JSON is `{response}`, with ordered keys:

```text
status,input_id,member_id,run_id,owner_epoch,interaction_id,event_id,decision,choice,decision_source,stage,output_seq,reason
```

`status` is `accepted`, `held`, `rejected` or `uncertain`; `input_id` is null until a durable row exists. Decision, choice and source retain their command values; exactly one of `decision` and `choice` is non-null. `stage` is `zero_bytes`, `partial`, `submitted` or `unknown`: accepted has `submitted`, held/rejected `zero_bytes`, and uncertainty the observed partial/submitted stage or `unknown`. `output_seq` is an unsigned integer, null when no current sequence can be established. `reason` is a nullable bounded explanation.

Historical response projection uses only the durable row and immutable audit. Return the original input identity, decision or choice, source and audit `expected_owner_epoch` as `owner_epoch`, with `output_seq=null`; current owner/screen evidence supplies no replacement value. It performs no lock, live probe, control exchange or database mutation.

| Durable receipt | Retry status and stage |
|---|---|
| `accepted`, or `settled/submitted` | `accepted/submitted`. |
| `in_progress` or `uncertain` | `uncertain/unknown`; retain original transmission uncertainty and claim. |
| `settled/not_submitted` or `settled/abandoned_uncertain` | `rejected/zero_bytes` with a consumed-key reason; preserve the original disposition and claim. |
| `ready` | The separate exact-metadata live-validation path; retain its original epoch/token binding. |

An uncertain reply, live or retained, reports the receipt's recorded `uncertain_cause` (§6.9 Uncertain cause) as `reason`; a receipt without a recorded cause, such as an `in_progress` row left by runner loss, reports `prior transmission remains uncertain`. For a consumed key, `zero_bytes` describes this retry only and supplies no zero-byte evidence about the original attempt. A prior abandoned-uncertain claim remains possibly transmitted.

Response text is:

```text
Permission response: interaction=INTERACTION_ID run=RUN_ID decision=DECISION choice=CHOICE source=SOURCE status=STATUS input=INPUT_ID stage=STAGE output_seq=SEQUENCE
```

Render null values as `-` and append `Reason: REASON` when nonnull. Accepted means answer-key submission, independently of native approval and execution.

Exit 0 means accepted submission or its matching prior submitted receipt. A structured held/rejected/uncertain reply prints the response in the selected text/JSON form and exits 1 (§6.3 Error Messages), with stderr `Error: permission response STATUS for interaction INTERACTION_ID: REASON`; append ` (input_id=INPUT_ID)` when known. Pre-reply operational failures retain their ordinary error category and include known run/interaction/input context; usage failures exit 2.

A new attempt against a blocked current request fails before any receipt, audit, claim or byte. It exits 1 with the primary stderr line `Error: runner control: permission request is blocked: REASON`, followed by the known run/interaction/operation-key context. Respond checks the blocker at admission, after its admission drain, and again before each answer stage. `REASON` is the bounded explanation `permission inspect` returns as `capability_reason` for that state:

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

An answer of the wrong kind for the current interaction, or a choice without a tested plan, returns a structured `rejected` response with `zero_bytes` stage and exit 1, before any receipt, audit, claim or byte:

| Condition | `reason` |
|---|---|
| `--choice` on an interaction that is not a question | `choice answers apply only to question interactions` |
| `--decision` on a question | `decision answers apply only to permission requests` |
| `--choice` naming no tested plan of the current question | `choice CHOICE_ID has no tested answer for this question` |

The CLI takes semantic decisions and question choices. For a new attempt, the pinned runner validates the current stable permission request or question, token, answer capability, manual input and output state, atomically reserves its audit/claim/receipt, and sends only a tested single-request choice for a decision, or a tested declared dismissal choice for a question. Unknown capability preserves manual native input. `member prompt`, `member exec`, message previews, receipt settlement and recovery remain separate operations.

##### Permission-response audit output

`input show` and `input reconcile` append nullable `permission_response` after `evidence_ref`, null for other input kinds. For a response kind it contains ordered `input_id,event_id,interaction_id,expected_owner_epoch,request_token,decision,choice_id,director_member_id,director_mode,decision_source,mode_evidence_ref,decision_evidence_ref,answer_rule_id,decided_at`; every field is present and follows its SQL type/nullability, so exactly one of `decision` and `choice_id` is null.

For a response audit, the `Input:` line is followed by `Permission decision: interaction=INTERACTION_ID event=EVENT_ID decision=DECISION choice=CHOICE source=SOURCE mode=MODE director=DIRECTOR_ID epoch=EPOCH`, rendering the null one of `DECISION` and `CHOICE` as `-`. Settlement records disposition and sends zero bytes; it neither grants a native answer nor releases a possibly transmitted episode claim.

#### Error Messages

Application errors print to stderr even with `--json`, which selects only successful output; the permission-response exception (§6.3 Permission commands) also prints its structured held/rejected/uncertain response to stdout, alongside its stderr diagnostic, before exit 1. Input errors use the documented usage/application category. Dispatch, input-lock, compatibility and deletion-lock errors are specified at their action owners above. The executable prefixes the first stderr line of every application error with `Error: `, including messages this document quotes without it.

| Command | Situation | Error message | Exit |
|---|---|---|---|
| (any surface resolving a config dir) | A set config-path variable holds a non-absolute value (empty string and relative paths included); `doctor` reports it as the agent's configuration check instead | §6.3 Config-dir resolution validation | 1 |
| (any surface resolving a config dir) | A set config-path variable or `HOME` holds a value that is not valid UTF-8 (§6.3 Config-dir resolution) | `Error: <VAR> is not valid UTF-8` | 1 |
| (any command except `setup` and `doctor`) | A missing, empty or incompatible database | §6.3 Schema-version guard | 1 |
| (any command with output) | Writing stdout fails with a broken pipe, e.g. `cafleet member list 1` piped into `head -1`; the command stops writing and nothing goes to stderr | — | The command's own result |
| (any command with output) | Writing stdout fails with any other error | `Error: failed to write output: <cause>` | 1 |
| `member prompt` / `member exec` / `member press` / `member recover` / `permission respond` / `input reconcile` | `--from-member-id` is not the live subject fleet's root Director, checked after the schema-version guard and the subject's not-found check, before multiplexer, lock, socket or file access; stdout stays empty | §6.3 Director-only command guard | 1 |
| `fleet create` | No tmux server, the pane is not on the chosen server, a herdr caller without `HERDR_PANE_ID` or `--pane`, or a Hermes session without `--pane`; before any write | §6.3 `fleet create` | 1 |
| `member create` | `--coding-agent hermes` with an unreadable `hermes config path`, before registration or multiplexer effects | §6.7 Hermes availability preflight | 1 |
| `member create` | A non-default database and a spawn prompt without `{database_url}`, before registration (§6.3 Spawn-prompt substitution) | `Error: the fleet database is not the default; the spawn prompt must include {database_url} so the member passes --database-url` | 1 |
| `member delete` | A `hermes` run whose work settlement cannot be verified; the member row and runner cleanup records are kept | `Error: hermes work for member <ID> could not be verified as stopped: <cause>; member was not deleted.` | 1 |
| `setup` | An extension file or its executable record cannot be installed, or the Hermes `config path` read fails; no `asset_installs` row for the agent (§6.3 Asset installation, Hermes plugin enablement) | `failed to install extension into <target>: <error>` | 1 |
| `setup` | The Hermes plugin enablement fails; no `asset_installs` row for `hermes` (§6.3 Hermes plugin enablement) | `failed to enable hermes plugin cafleet in <config path>: <cause>` | 1 |
| `message listen --codex-thread` | `codex` is not on `PATH`, or no Codex ancestor process exists; before `ready` | §6.3 Codex forwarding | 1 |
| `member create` | `--coding-agent opencode` with the selected agent preset missing or unreadable at the resolved preset path, before registration or multiplexer effects. `<preset>` is the resolved `<preset base>/agents/<mode>.md` for the resolved launch policy, and `<cause>` is the read error | `Error: cannot read opencode agent preset at <preset>: <cause>; run 'cafleet setup --coding-agent opencode' first` | 1 |
| (any parser-validated input) | A missing positional subject id (missing-required-argument error naming the positional); a non-integer id or a `permission respond` `--choice` outside the choice-id shape (invalid-value error); neither or both of the positional body and `--file` (`message send` / `member create`) or of `--decision` and `--choice` (`permission respond`) (argument-group error). `permission respond` errors precede lock or byte effects | The parser's native error of that kind | 2 |
| `fleet show` / `fleet delete` / `member list` / `member create` | Unknown fleet ID; for `member create`, raised by the first fleet check, after the schema-version guard and before Director resolution | `Error: fleet '<fleet-id>' not found.` | 1 |
| `member create` | Into a soft-deleted fleet | `Error: fleet X is deleted` | 1 |
| `member create` | The fleet row has no `director_member_id` recorded (mid-bootstrap corruption) | `Error: fleet <fleet-id> has no root Director recorded; re-create the fleet with 'cafleet fleet create'.` | 1 |
| `member create` | The live-fleet guard: the fleet's root Director is not an active member | `Error: fleet <fleet-id>'s root Director (member <id>) is not active.` | 1 |
| `member create` | The live-fleet guard: the fleet's root Director has no concrete placement | `Error: fleet <fleet-id>'s root Director (member <id>) has no concrete placement.` | 1 |
| `member delete` | Against the root Director's id | `Error: cannot deregister the root Director; use 'cafleet fleet delete' instead` | 1 |
| `message ack` / `message show` | An unknown `MESSAGE_ID` | `Error: Message <message-id> not found` | 1 |
| `message ack` | A message not in the `input_required` state | `Error: Cannot ACK message in state <state>` | 1 |
| `message send` | The sender is unknown or inactive | `Error: Sender member not found or not active: <from-member-id>` | 1 |
| `message send` | The recipient is unknown or inactive | `Error: Destination member not found: <to-member-id>` | 1 |
| `message send` | Sender and recipient in different fleets | `Error: members <from-member-id> and <to-member-id> are not in the same fleet.` | 1 |
| `member capture` / `member prompt` / `member exec` / `member press` | The member has a pending placement | <code>Error: member &lt;id&gt; has no pane yet (pending placement) — nothing to &lt;capture&#124;prompt&#124;exec&#124;press&gt;.</code> | 1 |
| `member show` / `prompt` / `exec` / `capture` | An unknown or inactive target member id | `Error: Member <member-id> not found` | 1 |
| `member prompt` / `exec` / `capture` | A target with no placement row | ``Error: member <member-id> has no placement row; it was not spawned via `cafleet member create`.`` | 1 |
| `member prompt` / `member exec` | Ordinary member without a recorded run, before pane I/O | `Error: member <ID> has no recorded run` | 1 |
| `message send` / `member create` | Positional body empty or whitespace-only | `Error: text may not be empty.` | 2 |
| `message send` / `member create` | `--file <path>` to an empty (zero-byte or whitespace-only) file | `Error: --file <path>: file is empty.` | 1 |
| `message send` / `member create` | `--file -` with empty or whitespace-only stdin | `Error: --file -: stdin is empty.` | 1 |
| `message send` / `member create` | `--file <path>` to a non-existent path or non-regular file | `Error: --file <path>: file does not exist or is not a regular file.` | 1 |
| `message send` / `member create` | `--file <path>` to an unreadable file | `Error: --file <path>: file is not readable.` | 1 |
| `message send` / `member create` | `--file <path>` to a file containing invalid UTF-8 | `Error: --file <path>: file is not valid UTF-8.` | 1 |
| `member create` | A `--model` or `--effort` value the selected backend rejects, before any side effect | §6.7 *Model selection* and *Reasoning effort* | 2 |
| `member create` | An unknown `{placeholder}` in the prompt, before registration | `Error: Unknown placeholder '<name>' in custom prompt. Supported placeholders: {fleet_id}, {member_id}, {director_member_id}, {coding_agent}, {database_url}. Double literal braces ({{, }}) to keep them as text.` | 2 |
| `member create` | A malformed brace expression in the prompt, before registration | `Error: Malformed custom prompt: <detail>. Double literal braces ({{, }}) to keep them as text.` | 2 |
| `member create` | `--coding-agent` omitted and the spawning Director not found in the fleet; nothing spawned | `Error: Director (member <director-id>) not found.` | 1 |
| `member create` | `--coding-agent` omitted and the spawning Director has no placement row; nothing spawned | `Error: Director (member <director-id>) has no placement; supply --coding-agent.` | 1 |
| `permission inspect` | An unknown `INTERACTION_ID`, ordered per §6.3 Permission commands | `Error: Interaction <interaction-id> not found` | 1 |
| `permission respond` | `--choice` on a non-question, `--decision` on a question, or a choice without a tested plan; the structured `rejected` response prints on stdout first, with reasons per §6.3 Permission commands | `Error: permission response rejected for interaction <interaction-id>: <reason>` | 1 |
| `permission inspect` | The inspected run records a database identity other than the selected database's; the known interaction context follows | `Error: invalid stored value for member_runs.database_identity_json: selected database identity mismatch` | 1 |

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

`to_member_id` is always an integer. ACK changes `status_state` to `completed` and stamps `status_timestamp`; completion means consumption, independent of the assignment's progress.

#### Rendered shape

§6.3 Output shapes owns each command's wrapper. Poll returns an array of full records; show/ACK wrap one record as `message`; send adds `notification_sent`; wait returns full poll records inside its finite receive result; listen emits the full record inside a tagged `message` envelope, with separate `ready`/`closed` lifecycle records and explicit ACK consumption. Capture timestamping, hashing, mode selection and text output follow §6.3 `member capture`, and its JSON key order §6.3 Output shapes.

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

Text bodies use a fixed 200 codepoints plus `…`; `--json` preserves the complete exact text. Pane preview lines, their folding, inline limit and long-body `message show` pointer follow §6.5 Push notifications.

#### Runtime observation messages

A runtime notification is an ordinary inbox record whose `text` is the complete versioned event JSON. The pinned observed member is its sender and the Director its recipient; verify the durable event/message association through `event show` before acting. Its typed source distinguishes runner observation from model-authored speech. Version-2 permission-opening events carry a broker-built `director_action` (review instruction and inspection argv), validated against the stored event/Director association (§6.9 Request-bound permission responses). ACK consumes the message independently of the attributed decision, answer submission, event visibility and actual continuation. §6.9 Persistence, notification and read interfaces owns evidence policy, atomic association and archived recovery.

#### `strip_ansi` regex and CR-defrag

The CSI regex is exactly `\x1b\[[0-?]*[ -/]*[@-~]`: ESC, `[`, any run of parameter bytes (`0x30`–`0x3F`), any run of intermediate bytes (`0x20`–`0x2F`), then one final byte (`0x40`–`0x7E`). Only CSI sequences are stripped; OSC, DCS and single-character escapes deliberately remain. After stripping, split on `\n`; per line, trim the trailing run of `\r` (a CR with nothing written after it, such as a CRLF terminator's, overwrites nothing), then keep only the substring after the **last** remaining `\r` (CR-redraw defrag: a TUI redraw `prefix\rNEW` keeps `NEW`, a CRLF-terminated `row\r\n` keeps `row`); re-join with `\n`. An empty/falsy input returns unchanged.

#### Formatter helper semantics

Absent ordinary member/fleet cells use ASCII `-`. ISO-to-HMS takes the first eight characters after `T`; absent/non-string/no-T values render `-`. Idle renders null as `-`, seconds below 60, floor minutes below 3600 and floor hours afterward. Required fields fail if absent; nullable activity and pending placement are documented valid cases.

### 6.5 Multiplexer (tmux + herdr)

CAFleet hosts coding-agent sessions in tmux or herdr panes. Both backends are driven as CLI subprocesses, and wrapped input on both goes through the pinned PTY runner control socket. The adapters preserve pane ownership, layout, literal input and bounded subprocess behavior.

#### Backend matrix

| Behavior | tmux | herdr |
|---|---|---|
| Pane ID example | `%7` | `w1:p1` |
| Spawn working directory | Splitting pane inheritance | Explicit `--cwd` |
| Capture for `member capture` | `capture-pane -p -e` | `pane read --format ansi` |
| Delete layout | Native auto-fit | Best-effort rebalance scoped to the closed pane’s tab |

#### Backend selection

`fleet create` and the `cafleet doctor` `multiplexer` row are the only readers of the caller's multiplexer environment. Both use one resolver, where truthy `HERDR_ENV` signals herdr and a set `TMUX` signals tmux:

| `HERDR_ENV` | `TMUX` | Result |
|---|---|---|
| truthy | set | Error `ambiguous multiplexer environment: both HERDR_ENV and TMUX are set; unset the one that does not belong to this session` |
| truthy | unset | herdr |
| not truthy | set | tmux |
| not truthy | unset | Error `no supported multiplexer detected: neither HERDR_ENV nor TMUX is set; run cafleet inside a tmux or herdr session` |

`fleet create` records the backend, its server and the Director's pane context (§6.3 `fleet create`); `cafleet doctor` reports the detected backend and its identifiers (§6.3 `cafleet doctor` — Environment Diagnosis). Every other multiplexer-driving command — `member create`, `member delete`, `fleet delete`, `member prompt`, `member exec`, `member press`, `member capture`, runner recovery and Hermes work settlement — builds it from the fleet Director's placement row (backend, `mux_server`, session, window, pane) and targets the recorded server explicitly (§ Multiplexer context).

#### Error taxonomy

Both backends report failures through one multiplexer error type carrying the backend's own message text, so CLI boundaries handle both uniformly. A creation error also records its pane-cleanup outcome (below), and a key-dispatch error whether a writing process had started.

#### Pane ownership during creation

The split owns a newly created pane until it returns successfully; herdr takes ownership as soon as it extracts the pane id from the split response, and closes that pane itself when the subsequent `pane run` fails, returning the original run error. A creation error carries one of these pane-cleanup outcomes:

| Outcome | Meaning and caller action |
|---|---|
| Closed | The backend already closed the known pane, tolerating an already-missing pane; the caller performs the remaining database compensation without another close |
| Close failed | The backend tried to close the known pane and failed; the error keeps the pane id and the original error and appends `cleanup failed for pane <id>: <detail>`, and the caller performs the remaining compensation without retrying the close |
| Unknown | A failed or malformed split response left no confirmed pane id; the error appends `pane ID unknown; pane cleanup unconfirmed`, and the caller closes no pane |

On success, ownership transfers immediately to member creation, whose compensation follows the creation failure order (§6.3 Creation failure compensation) and closes only the pane owned by that operation. Layout equalization is best effort (§ herdr column equalization).

#### Subprocess output and deadlines

The shared subprocess runner captures stdout and stderr without truncation. With a timeout, it drains both pipes concurrently from spawn using nonblocking I/O: each loop checks the monotonic deadline and the direct child's exit status, then reads at most 64 KiB per stream, polling at most 20 ms or the remaining deadline, whichever is shorter. Interrupted operations retry against the same deadline. Completion requires the child's exit and EOF on both streams. Success returns stdout, and a nonzero exit reports stderr, as lossy UTF-8. Calls without a timeout retain the unbounded output-collection path.

At the deadline, including when the direct child has exited but a descendant still holds a pipe open, the runner kills and reaps the direct child, closes both read pipes and reports a timeout; descendant termination is outside this contract. FD setup, read, poll and child-status failures also release the pipes and kill/reap the child; cleanup failures accompany the primary error instead of replacing it. The deadline bounds observation and the start of cleanup; an unresponsive operating system can delay cleanup itself.

#### Pane spawn working directory

A member pane spawned by `cafleet member create` starts in the invoking process's working directory (the Director's pane cwd): tmux inherits it from the split pane, and herdr receives it verbatim as `--cwd` rather than relying on herdr's own inheritance, which spawns `/bin/sh` instead of the passwd login shell when `SHELL` is unset ([herdr discussion #1517](https://github.com/ogulcancelik/herdr/discussions/1517)). An unresolvable cwd fails the spawn with `cannot resolve the working directory for pane spawn: <cause>`; there is no fallback directory.

#### Delete-time pane layout

On tmux, `kill-pane` auto-fits natively. herdr reads the target pane's tab (`herdr pane get`), runs `herdr pane close`, then rebalances that tab (§ herdr column equalization) within the original operation deadline after registry finalization. A failed or expired restoration emits a stderr diagnostic; `member delete` retains its successful close and deregistration result.

#### Pane input lock

Every operation that writes pane input uses one process-shared OS file lock. Acquire the guard before placement revalidation. For wrapped prompt/exec/preview submissions the runner owns the lock: wrapped clients validate and submit without holding it while awaiting the runner, and the runner acquires it nonblockingly from its event loop under the attempt deadline, revalidates the concrete placement and retains it across an active staged write through durable receipt/stamp settlement (§6.9 Runner-owned deferred delivery). A merely queued request holds no lock. Explicit key requests hold it through their outer-pane dispatch and bounded cleanup. Close the non-cloned owned handle on every path.

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
| `message send` / `message wait` delivery | The §6.3 Delivery outcome diagnostic with `CAUSE` `pane input remained busy` |
| `member prompt`, `member exec` | Exit 1 with the runner input failure and recorded runtime context (§6.3 Runtime presentation) |
| `member press` | Exit 1 with the primary and zero-dispatch diagnostics (§6.3 `member press`) |

Process exit releases ownership. Read-only capture needs no input lock. Deletion locking follows §6.3 `member delete` and `fleet delete`.

#### Termination during pane input

Interrupting a host turn, including through Escape-first delivery fallback, can also end the turn's in-flight shell tool, such as a running `cafleet` command. The CLI therefore blocks `SIGINT`, `SIGTERM` and `SIGHUP` across every pane-input and lifecycle span; a signal received meanwhile takes its default action when the span ends. Spans nest and restore the outer signal mask.

| Command | Span |
|---|---|
| `message send`, each `message wait` attempt | From lock acquisition through the last `notified_at` stamp |
| `member prompt`, `member exec` | From the runner request through its reply |
| `member press` | Begin before acquiring the lock; retain it through the writing process and bounded cleanup, releasing the lock before restoring signals |
| `member create` | From the pane spawn through the registry commit or its compensation |
| `member delete`, `fleet delete` | From lock acquisition through deregistration and layout cleanup |

Every span is bounded by its own deadline, so a deferred signal waits at most one span. `message wait` outside an attempt keeps default signal behavior. Multiplexer client processes started inside a span inherit the blocked set and stay bounded by their timeouts; the member session, started by the multiplexer server, does not inherit it.

`SIGKILL` cannot be deferred; a kill inside a span leaves:

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

Accept only these productions, with the CLI's count/UTF-8 bounds and complete-vector validation. The dedicated key adapter sends only requested keys, using shell-free argv and no prompt phase or settling sleep.

Each backend exposes target validation against the recorded placement and key sending, which accepts only a validated vector (canonical names plus backend arguments) and performs the single bounded writing invocation. The press entry keeps its original connection and deadline and holds the validated lock through process cleanup.

Acquire the canonical pane-input lock shared with deletion and validate its retained descriptor/path binding. Build the adapter from the fleet Director's recorded placement (§ Backend selection) and, on that recorded server, verify the target's complete tuple against the recorded session/workspace and window: tmux reads `#{pane_id}|#{window_id}|#{session_name}` for the target and splits only the first two separators; herdr reads its pane-list JSON and requires exactly one matching workspace/tab/pane. Re-read the active member after metadata, compare its fixed fleet/kind/full placement and validate the held lock immediately before writing. A missing, ambiguous, malformed or changed target fails before the writing invocation.

Validation establishes the tuple within the current recorded server only: the placement records no socket/server-generation identity, so it supplies no cross-server fallback or proof of historical server/build equivalence. Keep one adapter/environment across every probe and the write.

Capture one inherited-or-new five-second absolute deadline before invocation database validation. Every press process — the availability, current-context and target-metadata probes and the write — runs under absolute `run_until` with that bound through spawn, collection and cleanup, and a nested runner clips it by any earlier retained bound. The lock wait reserves one second for metadata/dispatch/cleanup (not the prompt reservation). Every later check uses the remaining original budget, and insufficient launch/cleanup reserve starts no new writing process.

Within the original deadline, reserve 250 milliseconds for owned process cleanup and end collection/launch at that earlier bound. On an actual error, close read pipes, kill only the still-owned unreaped direct child and poll `try_wait` with sleeps at most 2 milliseconds while the original cleanup deadline has time remaining, checking that deadline after every interruption. Explicit cleanup and unwind fallback use the same deadline and perform no blocking wait. An explicit cleanup attempt prevents a second Drop retry cycle; unconfirmed termination/reap remains an explicit diagnostic: `cleanup unconfirmed for child PID: original command deadline expired before reap`. Preserve actual kill/status errors, the primary process error (a timed run error keeps its original cause) and the ordered cleanup diagnostics; unwind emits cleanup diagnostics to stderr. Relative and untimed execution serve no press process and follow § Subprocess output and deadlines. Synchronous calls have no hard scheduler/syscall return-time guarantee.

Both writing adapters acknowledge only successful process completion with empty/whitespace stdout. Unexpected non-whitespace stdout on success is unreadable acknowledgment with uncertain effects; herdr metadata retains its command-specific JSON parsing. Backend keyboard modes or key interception can affect actual PTY bytes even after acknowledgment. Use the CLI result/error contract (§6.3 `member press`), including its zero-write and no-retry cases, and observe actual native effects separately.

#### Prompt dispatch

Every ordinary member uses a pinned, healthy control socket and its runner's owned child PTY (§6.9 Input arbitration and the original native answer). The runner selects profile delivery without Escape, zero-byte queueing or Escape-first delivery fallback by §6.9 Delivery method and interaction priority. Uncertain reachability preserves manual terminal use while holding generated input.

Validate a nonblank literal line without CR/LF or C0/DEL controls before starting any process. No input path captures the pane or consults its state. Native shell input starts with `! ` followed by the actual validated command; composer controls begin the submitted input.

An OpenCode member (`opencode mini`, §6.7 Per-backend `build_spawn_argv`) receives pushed notifications and direct prompts as submitted user turns. Native `! ` input executes a shell command.

Deletion and prompt/exec operations start their five-second budgets after the invocation's schema-version guard. Resolve native command bodies before starting their input budget. Runner-owned staged writes use the original attempt deadline (§6.9 Runner-owned deferred delivery).

A started write with an ambiguous result retains uncertainty; failure after submission can mean the command ran. Inspect native output and durable receipts before any newly authorized retry. The lock excludes CAFleet writers while the user's keyboard and host UI remain independent; successful dispatch establishes submission only.

#### Push notifications

Active ordinary recipients with concrete panes receive previews through their recorded transport. The root Director receives through native `message listen` (§6.3) or manual broker reads, with zero automatic pane input. Self-sends persist without pane input. §6.3 `message send` owns `notification_sent`, exact output and errors; §6.3 `message wait` owns diagnostic revalidation and bounded retries.

Profile delivery submits unchanged the canonical wrapped preview, this exact line ending with the resume clause:

```text
[cafleet msg MESSAGE_ID from SENDER_ID TIMESTAMP] BODY [cafleet] Resume your work if something was still running.
```

Delivery fallback (§6.9 Delivery method and interaction priority), whose submission leads with Escape, uses the full line with both clauses:

```text
[cafleet msg MESSAGE_ID from SENDER_ID TIMESTAMP] BODY [cafleet] Resume your work if something was still running. [cafleet] If this delivery's Escape dismissed a pending prompt in your pane, that rejection did not come from the user; re-issue the tool call if you still need it.
```

Stored text remains exact. Both forms fold CRLF/LF/CR to `⏎`, tabs to `␉`, and other C0/DEL controls to visible hexadecimal escapes (`\xNN`). A folded body of at most 4000 Unicode scalar values appears inline; a longer body becomes `Body is N characters; run cafleet message show MESSAGE_ID --json to read it.`, where N counts stored text. This limit is independent of the 200-codepoint text-output limit (§6.4 Text-mode projection). The runner renders the form from the selected delivery method before its first byte.

One synchronous drain uses a five-second attempt budget and the runtime pane-lock protocol, committing short read snapshots before socket I/O.

1. Read recipient/message eligibility, excluding the root Director before preview selection or pane locking. An inactive/ineligible recipient, fleet mismatch or no pending row ends the attempt. An ordinary recipient without a recorded run fails this attempt with `member MEMBER_ID has no recorded run`; pending placement holds a recipient with a run.
2. Query durable key `preview:<run_id>:<message_id>` before live input eligibility. Conflicting reuse fails; in-progress or uncertain holds for inspection, a queued ready receipt returns its queued result, and a settled key returns its existing disposition without replay. An accepted receipt permits settlement without typing. Capture one expiration from the original deadline for every path that will submit, queue, settle or retire, including retained accepted/submitted history.
3. Revalidate activity, member kind, fleet, concrete placement, message status and notification stamp before runner submission; the runtime input protocol acquires and revalidates the shared pane lock for actual stages. Changed placement holds delivery.
4. A new input attempt requires a fresh pinned run identity/epoch and socket health; retained accepted history proceeds to settlement.
5. Select the oldest unattempted pending message by `(created_at,message_id)` and revalidate the recipient/placement. Each invocation attempts it at most once; query its durable receipt before current input eligibility.
6. A new attempt sends the canonical base preview to the runner, which reserves the receipt, then submits it immediately, queues it with zero bytes or holds it (§6.9 Delivery method and interaction priority). An immediate submission commits its durable in-progress row and selected method before child bytes, then records accepted or uncertain submission. Reuse the captured expiration for submission and any later retirement.
7. Settle accepted delivery in one transaction that stamps `notified_at` or verifies ACK and marks the receipt settled/submitted. The serialized service verifies that same durable settled/submitted receipt and excludes every active pending writer before removing an unleased cache entry with no available attempt permit. Retirement uses the remaining original expiration and sends zero native bytes. Failure after possible input preserves the actual receipt state and notification stamp for reconciliation.
8. Continue at step 5 while the budget and input state permit. The runner evaluates each new attempt afresh; a request queued behind an earlier queued input receives the same queued result.

A retirement failure after settlement preserves the committed sent prefix and `notification_sent=true`. Report `message MESSAGE_ID was notified to member MEMBER_ID; input INPUT_ID is settled/submitted; receipt retirement failed: CAUSE`, retaining known run/input/key context and the original cause. This warning uses the attempt error shape; durable settlement remains authoritative and grants no submission replay.

The lock owner retains the lock through final submission/settlement. Wrapped delivery follows the durable input contract (§6.9 Input arbitration and the original native answer); only absent receipts or certified ready/zero-byte state permit a new eligible attempt.

The notification diagnostic (§6.3 Delivery outcome) applies to failed input attempts and database contention, retaining actual causes and the committed message. Mandatory finalization uses the later of the original deadline and one second after finalization begins, while the transport stays within its original deadline. Other fatal storage errors retain their cause; a send preserves its committed result.

Delivery leaves `status_state` unchanged: an inline or pointer preview stays `input_required` until its recipient ACKs the message ID, and poll and ACK identify consumption independently of input delivery.

##### Held delivery

The runner queues a wrapped preview with zero bytes and a durable ready receipt, or selects delivery fallback, under the ordered rules of §6.9 Delivery method and interaction priority, and retries a queued preview itself (§6.9 Runner-owned deferred delivery). Wrapped delivery stays held, with no queue acceptance or write, for a degraded/recovering/lost observer, unavailable ownership or storage, an uncertain receipt/reachability, pending placement or a placement change under the lock. Queued and held delivery leave `notified_at` unset, report no successful notification, create no new permission event and grant no native answer.

A message stays pending for external retry while the lock stays busy (contention waits within the attempt budget), the pane is not yet placed, the budget runs out or an attempt fails; the next send to that ordinary recipient or a `message wait` (§6.3) rotation retries it. Transport and stamp failures after persistence remain diagnostics alongside a successful send result; no notification outcome erases a committed delivery.

##### Accepted trade-offs

These trade-offs apply to delivery fallback, which leads with Escape. A single Escape's effect depends on the backend, which may need more than one Escape to interrupt a running session. Delivery is correct whatever Escape does: when Escape does not interrupt, the line is still typed into the composer and the host submits or queues it, and both conditional trailing clauses stay accurate.

- Where Escape interrupts an ordinary member's working turn, the resume clause tells the recipient to continue.
- Where Escape dismisses an ordinary member's question, confirmation or approval prompt, the host can report a user rejection before the input arrives; Claude Code reports `The user doesn't want to proceed with this tool use.` The preview's dismissed-prompt clause marks that rejection as delivery-caused, and the member re-issues the required tool call or re-asks the question. `member prompt` and `member exec` input carries no clause; the Director checks the pane for an open prompt before that dispatch.
- Escape does not reliably clear typed composer text on every host; unsent text in an ordinary member's pane can be submitted together with the delivered line.
- A failed or interrupted notification is retried, so a recipient can see the same `[cafleet msg ID …]` line more than once; it polls and ACKs by ID.
- Two sends in quick succession from different senders each lead with Escape; the second can interrupt the turn the first delivery started.
- A wrapped runner suppresses fallback for detected and candidate interactions only (§6.9 Delivery method and interaction priority). On a run with unknown or partial detection coverage, fallback can dismiss an undetected permission request or question; a changed native UI can trigger fallback while its unsupported permission answer remains unavailable.
- OpenCode `mini` captures can expose only the status bar. Inspect retained session records for submitted input and native shell output when capture omits them; use broker poll and ACK to establish message consumption.

#### Multiplexer context

A multiplexer context is a plain value of three non-empty strings: `session` (tmux session name or herdr workspace ID), `window_id` (tmux window or herdr tab, e.g. `@N`) and `pane_id` (e.g. `%N`). `fleet create` reads it for the Director's pane on the chosen server and records it with that server as `mux_server`, which every later CAFleet invocation for the fleet targets explicitly:

| Backend | `mux_server` holds | Every CAFleet invocation after `fleet create` |
|---|---|---|
| tmux | The absolute server socket path | `tmux -S <mux_server> …` |
| herdr | The herdr session name (`default` for the default session) | `herdr --session <mux_server> …` |

Each member placement copies the Director's `mux_server`. A split takes the Director's recorded context, not a bare window ID: tmux splits the reference window, herdr splits relative to the reference pane. A missing or invalid created-pane ID is an unconfirmed cleanup, never a license to infer another target (§ Pane ownership during creation).

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
| Pane context (`fleet create` only) | <code>tmux display-message -p -t &lt;pane&gt; "#{session_name}&#124;#{window_id}&#124;#{pane_id}"</code>, split on the first two <code>&#124;</code> | `herdr pane current --pane <pane>`; `result.pane` supplies `workspace_id`, `tab_id`, `pane_id` | tmux: `unexpected tmux display-message output: <quoted>`; herdr: `herdr <context> missing '<key>' field`; an unknown pane fails per §6.3 `fleet create` |
| Split | `tmux split-window -t <window_id> -P -F "#{pane_id}" -d <command>...`, then `tmux select-layout -t <window_id> main-vertical` | <code>herdr pane split &lt;pane&gt; --direction right&#124;down --no-focus --cwd &lt;cwd&gt;</code>, then `herdr pane run <new pane> "<shell-quoted command>"` | An empty or failed split is an unknown pane-cleanup outcome (§ Pane ownership during creation); tmux ignores a layout error |
| Capture | `tmux capture-pane -p -e -t <pane> -S -<lines + 1000>` | `herdr pane read <pane> --format ansi --source recent-unwrapped --lines <lines + 1000>` | `lines <= 0` rejection (§ Capture windowing) |
| Close | `tmux kill-pane -t <pane>` | `herdr pane get <pane>`, `herdr pane close <pane>`, then the delete-time rebalance | An explicit ignore-missing teardown tolerates only a recognized pane-gone cause (tmux stderr `can't find pane` / `no such pane`, herdr error code `pane_not_found`) |

The herdr split targets the Director's pane rightward for the first member; otherwise it splits the highest-sorting pane ID in the Director's tab (other than the Director) downward and then equalizes the member column. Its `--cwd` follows § Pane spawn working directory. `pane run` submits one text line into the pane's shell, so the argv command is rendered as a single POSIX-shell-quoted string. Every herdr JSON reply is parsed as a `result` envelope; non-JSON output fails with `herdr returned non-JSON output: <quoted>` and a reply without a `result` object with `herdr output missing 'result' object: <quoted>`.

Process failures preserve their context in the multiplexer error:
`<backend> binary not found: <detail>`, `<backend> command was not started:
<detail>: <argv>`, `<backend> command timed out after <N>s: <argv>` (or
`<backend> command timed out at the pane input deadline: <argv>` for a
deadline-bounded call) and `<backend> command failed: <argv>` followed by a
`stderr: <trimmed stderr>` line. Other errors propagate.

#### herdr column equalization

herdr has no single reflow command, so it rebalances the member column arithmetically, emulating tmux `main-vertical`. A layout read `herdr pane layout --pane <anchor>` returns `result.layout` with `panes[].rect{x,y,width,height}` and `splits[].{direction,rect,ratio}` for the anchor pane's tab, whichever tab or pane holds focus, so the read needs no tab-ID comparison. The member column is every pane whose `rect.x` is not the minimum x (the Director column), sorted by `y`; its `down` splits at the column's x form a right-leaning chain where split *k* (top→bottom) separates member *k* from the members below.

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

The receive task is the synchronous caller-owned `message wait` process (§6.3 `message wait`); its round-robin cursor, attempted-message set and diagnostics live only in memory. Every failed attempt, including database busy or deadline exceeded before candidate selection, adds its message ID to the attempted set. Submission budgets and pane input locking follow §6.5 Multiplexer.

The persistent listener follows §6.3 `message listen` and performs no preview retry, pane lock, multiplexer resolution or termination-deferral span. Goal activation/pause/budget and pending decisions retain host authority.

### 6.7 Coding-agent backends

Every member pane runs a coding-agent binary selected from the backend registry and recorded per member in `member_placements.coding_agent`. This section specifies the backend interface, each backend's spawn argv, model and effort validation, permission posture, bundled presets and host evidence.

#### Role requirements

| Role | Tier | Required backend capability |
|---|---|---|
| Director | Eligible | A goal requirement (§ Goal requirement) and a listen requirement (§ Listen requirement), evaluated independently. Both are required. |
| Member | Eligible | A runner-compatible PTY lifecycle, an explicit detection-coverage declaration and a classified default launch mode, `auto` or `manual`, with approval-prompt coverage: the bundled profile for the evaluated native version detects every approval form the default mode renders and declares request-bound allow and deny answers for it, at every pane geometry outside its recorded layout limits. A default mode that renders no approval form meets this without request rules. |
| Member | Bundled support | A native version the version probe parses, selecting a bundled profile that declares detection, automatic review annotations, composer plans and request-bound answers independently from native evidence. Capabilities beyond approval-prompt coverage are optional, and an undeclared one is `unknown` coverage or unsupported. |

An *approval form* is a native permission request that blocks the member until answered. Question, trust, update and startup dialogs are separate interaction classes outside approval-prompt coverage, with or without answer plans. A *recorded layout limit* is a profile-documented rendering that no request rule matches, such as a pane narrower than the profile's `W_min`, a wrapped or clipped operation, or an uncaptured geometry; a form rendered inside one produces no `interaction_opened` event and is answered manually on the native surface.

Evaluate each role independently; a backend ships for the roles it meets. A verdict is `yes` only when recorded evidence establishes every Eligible-tier requirement of that role against the profile selected for the recorded native version; otherwise it is `no` with "not established" or the observed failure as evidence, and a Director `no` names each unmet requirement. Default-mode approval-prompt coverage counts toward the Member verdict; other bundled-support capabilities never change a verdict. Each requirement group's evidence holds for its own recorded native version; a newer installed version carries the verdict forward until the next re-check, which follows any change to the backend's bundled profile boundary or recorded evidence. A Member-eligible backend whose version probe fails or selects no profile still launches: it records `coverage=unknown`, and any approval form it shows is answered manually on the native surface. A backend with unknown detector/composer coverage, including such a run, permits manual terminal work and delivers ordinary generated input through delivery fallback. § Role eligibility records each backend's verdicts.

Every non-default launch mode is an explicit `--permission-mode` choice under §6.9 Launch policy and capability registry, which records each mode's `auto`/`manual` class. Preserve native guards, managed policy, explicit denies, sandbox boundaries and the host's activation, pause, completion and budget authority. The decision surface and native receive support are evaluated independently of Director eligibility: a Director meeting both requirements may use manual `poll` or finite `wait` when its reception mechanism is unavailable, and a tier-3 listen establishes no unsolicited wake of an idle session.

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

**Subscribe probe** (tier 1 or 2, on an installed native version). The candidate Director starts the receiver through its tier's mechanism while another member sends broker records:

1. One record while the session is idle: a new turn starts without user input and consumes it.
2. Two records while the session runs a foreground command of at least 60 s: the command is not interrupted, and each record arrives no later than the end of that turn, without input.
3. Three records back to back: all arrive, in order.
4. The receiver ends when its owning session ends.

**Stream probe** (tier 3, on an installed native version). The candidate Director starts `cafleet message listen` through its native background-execution facility and continues unrelated work in the same session. The facility must deliver each record the running listener emits without waiting for the process to exit, at the latest at the session's next turn boundary, whether the host pushes records into the session or the model reads the running process's accumulated output on demand. The probe passes when broker records another member sends at separate times during that work are each first observed in the listener's output through that facility and consumed without user input while the same listener process still runs. Output that reaches the session only when the process exits, including a completion notification for a finite receive, fails the probe. Mid-turn delivery and waking an idle session are not required.

A record discovered through a foreground `poll`, `wait` or `show` invalidates a subscribe or stream probe run. Each run records the mechanism, native version and check date.

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
| `claude` | Director | `listen` | `native` | `subscribe` | `yes` | One Monitor task running `cafleet message listen`: subscribe probe (idle wake, records during a 70 s foreground command delivered after it, burst of 3 in order), 2.1.312, checked 2026-11-06. After `member delete` the listener had exited with its session; the exit latency was not measured. |
| `claude` | Director | `verdict` | — | — | `yes` | `goal and listen rows` |
| `claude` | Member | — | — | — | `yes` | PTY lifecycle, coverage declaration and omitted `auto` automatic review: live runner acceptance on tmux and herdr (omitted-default launch, auto-approved and auto-refused cases), 2.1.302, checked 2026-10-29. Bundled support: the Claude profile from `2.1.302`. |
| `codex` | Director | `goal` | `native` | — | `yes` | [Codex goals](https://developers.openai.com/cookbook/examples/codex/using_goals_in_codex), 0.162.1, checked 2026-10-22. Native `/goal` stayed active through the 200-record burst, idle and forced-failure steps, 0.165.0, checked 2026-11-06. |
| `codex` | Director | `listen` | `native` | `subscribe` | `yes` | `cafleet message listen --codex-thread` forwarding through `codex queue` on the escalated path: subscribe probe, a `message_ref` for a 70,000-byte body, 200 burst records in order past the 100-submission cap, 10 idle minutes and a forced-failure closed notice, 0.165.0, checked 2026-11-06. The sandboxed path is unverified, and `member delete` left no listener. |
| `codex` | Director | `verdict` | — | — | `yes` | `goal and listen rows` |
| `codex` | Member | — | — | — | `yes` | PTY lifecycle, coverage declaration and omitted `--approve-for-me` automatic review: live runner acceptance on tmux and herdr (omitted-default launch and auto-refused cases, host-limited cases recorded as limits), 0.163.0, checked 2026-10-29. Bundled support: the Codex profile from `0.163.0`. |
| `opencode` | Director | `goal` | `extension` | — | `yes` | `cafleet` plugin goal extension under `opencode mini --standalone`: set, status, pause, resume, clear, complete, compaction, restart on the same session and the no-progress stop, 2.2.0 (minimum), checked 2026-11-06. A continuation in flight at pause was not separately observed. |
| `opencode` | Director | `listen` | `extension` | `subscribe` | `yes` | `cafleet` plugin `cafleet_listen` with queued `session.synthetic` delivery under `opencode mini --standalone`: subscribe probe (idle wake, 70 s busy command, burst of 10 in order) and client exit ending the child, 2.2.0 (minimum), checked 2026-11-06. |
| `opencode` | Director | `verdict` | — | — | `yes` | `goal and listen rows` |
| `opencode` | Member | — | — | — | `yes` | PTY lifecycle and coverage declaration: live runner acceptance on herdr only (`coverage=tested`), 2.1.4, checked 2026-10-29. Default-mode approval-prompt coverage: the omitted `cafleet-ask` preset's shell permission form, whose detection and allow and deny answers the OpenCode profile from `2.1.4` declares `tested` (§6.9 Bundled profiles and native screen fixtures). Bundled support: the OpenCode profile from `2.1.4`. |
| `hermes` | Director | `goal` | `native` | — | `yes` | Native `/goal` set, status, pause, resume and clear on the TUI client, 2026.9.24, checked 2026-11-06. |
| `hermes` | Director | `listen` | `extension` | `subscribe` | `yes` | `cafleet` plugin `cafleet_listen` with ordered gateway injection on the TUI client: subscribe probe (idle wake, 70 s busy command, bursts of 3 and 10 in order) and `/exit` ending the child, 2026.9.24, checked 2026-11-06. |
| `hermes` | Director | `verdict` | — | — | `yes` | `goal and listen rows` |
| `hermes` | Member | — | — | — | `yes` | PTY lifecycle, coverage declaration and default `smart` approval-prompt coverage (ESCALATE and override, allow-once and deny): live runner acceptance on tmux and herdr, 2026.9.24, checked 2026-11-02. Bundled support: the Hermes profile from `2026.9.24`. |
| `pi` | Director | `goal` | `extension` | — | `yes` | CAFleet pi extension `/cafleet-goal` and goal tools under plain `pi`: set, status, pause, resume, clear, complete, automatic continuation, the no-progress stop, an interrupt pausing, manual `/compact` and `/quit` then `pi --continue` on the same session, 1.5.0 (minimum), checked 2026-11-08. A threshold compaction inside a continuation chain and a continuation by the re-arm turn after restart were not separately observed. |
| `pi` | Director | `listen` | `extension` | `subscribe` | `yes` | CAFleet pi extension `cafleet_listen` with `followUp` delivery under plain `pi`: subscribe probe (idle wake, records during a 70 s foreground command delivered after it, burst of 3 in order), `/quit` and a pane kill ending the child and `/reload` delivering the runtime-replaced notice, 1.5.0 (minimum), checked 2026-11-08. |
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

A coding-agent backend is a stateless value selected per member at spawn time, with two read-only properties and five operations:

- **`name`** — the registry key: a stable lowercase string that MUST equal its `--coding-agent` token, the persisted `placement.coding_agent` lookup value and its registration key.
- **`binary_name`** — the executable resolved against `PATH`; equal to `name` for every backend.
- **`ensure_available()`** — fails on the first unmet spawn precondition, in order: the config directory resolves per §6.3 CLI *Config-dir resolution* (an invalid config-location variable fails first); the binary resolves on `PATH`; the backend's own check (opencode: the selected preset file is readable; hermes: the effective approvals configuration passes § Hermes availability preflight; pi: the CAFleet pi extension `<pi base>/extensions/cafleet.ts` is readable). A shared helper resolves `binary_name` to an absolute path per §6.9 Member PTY runtime *Native executable resolution*, failing with `binary {binary_name} not found on PATH`. Preconditions read the host through a **spawn probe** (binary lookup, home directory, `env_var(name) -> optional string`); the system probe reads the process environment.
- **`validate_model(model)`**, **`validate_effort(effort)`** — raise a value-error for a value malformed or unacceptable for this backend; `None` (flag omitted) is always valid. A backend without a reasoning-effort control rejects every non-`None` effort.
- **`validate_prompt(prompt)`** — raises a value-error for a spawn-prompt body this backend cannot carry as its prompt argument; only pi rejects a body (§ Per-backend `build_spawn_argv`), and every other backend accepts every prompt. `member create` checks the raw body, before placeholder substitution.
- **`build_spawn_argv(prompt, display_name, cwd, model, effort)`** — the full argv (binary + flags + prompt) for the multiplexer's window-split. Its leading token is `binary_name`; the launched `argv[0]` follows §6.9 Native executable resolution and later tokens are unchanged. `cwd` is the member's resolved absolute working directory (§6.5 Pane spawn working directory); only hermes emits it, and every other backend's argv is independent of it.

**Ordering invariant:** `member create` MUST call `validate_model` → `validate_effort` → `validate_prompt` → `ensure_available` → `build_spawn_argv`, so a malformed model, effort or prompt fails before any precondition check or side effect, as a usage error (exit 2) with the backend's message; broker value-errors (§7.2) exit 1. `validate_prompt` runs on the resolved prompt body before the Director placement lookup and registration.

**No-model byte-identity:** a `None` model emits no `--model` tokens (never an empty `--model ""`) and a `None` effort emits no effort tokens, so each argv is byte-identical to its omitted form. An invalid effort never reaches argv construction.

#### Registry resolution

The backend registry is the fixed set of supported backends in registry order, each keyed by its `name`. Resolution exactly matches the persisted `placement.coding_agent` token, with no fuzzy matching or default; a token with no entry fails with `invalid stored value for member_placements.coding_agent: VALUE`, exit 1.

#### Per-backend `build_spawn_argv` (exact, token-by-token)

The blocks below are the child argv with omitted permission selection. The leading token names the backend binary (launched `argv[0]`: §6.9 *Native executable resolution*). The multiplexer starts the CAFleet PTY runner, which preserves the literal child argv, cwd and required environment. Every backend honors the leading-`!` shell shortcut that `member exec` dispatches through, and every member can run its broker commands from the moment it is spawned (§ Backend capabilities). Every backend's `validate_model` and `validate_effort` follow § Model selection and § Reasoning effort.

**claude** — `ensure_available` resolves the claude config directory and checks `claude` on `PATH`. `display_name` becomes `--name` (§ Backend capabilities).

```
["claude", "--permission-mode", "auto", "--prompt-suggestions", "false", "--allowedTools", <rule>...,
 "--name", <display_name>]                           # explicit selection replaces "auto" with <mode>
  (+ ["--model", <model>]  if model is not None)
  (+ ["--effort", <effort>]  if effort is not None)
  (+ <prompt>)                                       # bare trailing positional
```

The `--allowedTools` values are the claude member's **spawn-time allow rules**, one argv token per rule, so the member depends on no `Bash(cafleet ...)` rule in the user's `settings.json`. Every claude member receives exactly `Bash(cafleet message *)`, covering its broker commands including finite `message wait` and `message listen`. `--allowedTools` takes several values, so it precedes `--name`, which ends the list and keeps the prompt positional. The rules are fixed at spawn: a later loss of allow rules in the user's settings does not affect a running member. These cases stay with the host:

- A `deny` or `ask` rule at any settings level, including managed policy, still blocks the command.
- The managed setting `allowManagedPermissionRulesOnly` makes Claude Code ignore `--allowedTools`.
- A blocking `PreToolUse` hook stops a tool call before permission rules are evaluated.
- Settings can change after spawn, so overdue member readiness or replies are inspected directly.
- Work commands (`mise`, `git commit`, package managers) stay under the user's own read/edit/command rules; a denied one is routed through `member exec`.

**codex** — `ensure_available` resolves the codex home and checks `codex` on `PATH`. `display_name` is ignored.

```
["codex", "--approve-for-me"]
  (+ ["--model", <model>]  if model is not None)
  (+ ["--config=model_reasoning_effort=<effort>"]  if effort is not None)  # ONE token
  (+ <prompt>)                                       # bare trailing positional
```

An explicit `on-request` or `never` replaces `"--approve-for-me"` with `"--ask-for-approval", <mode>, "--sandbox", "workspace-write"`; `"--approve-for-me"` selects the workspace-write sandbox itself, and Codex 0.163.0 rejects it together with `--sandbox`.

**opencode** — `validate_model` examples: `"openai/gpt-4"` accepted; `"a/b/c"` accepted as provider `a` / model `b/c`; `"a/"`, `"/b"`, `"abc"` rejected. `validate_effort` rejects every non-`None` value, so `build_spawn_argv` never emits effort tokens. `member create` resolves the launch policy before `ensure_available`, whose opencode check (§ Interface order) reads only the selected preset `<preset base>/agents/<mode>.md` for the resolved mode (§ opencode preset); only opencode has a required preset. `display_name` is ignored; the prompt is a two-token `--prompt <prompt>` pair, unlike claude/codex's bare positional.

```
["opencode", "mini", "--standalone", "--agent", <mode>]   # <mode> is cafleet or cafleet-ask
  (+ ["--model", <model>]  if model is not None)
  (+ ["--prompt", <prompt>])                         # prompt via flag — TWO tokens
```

The pane runs `opencode mini --standalone`, the long-lived minimal interactive interface with a private server per pane; `mini` takes no positional message. The `--agent` value is the only argv difference between the two launch policies, and `member create` separately probes the native version for profile selection. Spawn, preset loading, broker input and native shell input are validated against opencode 2.1.4; the OpenCode profile from `2.1.4` is the first boundary, so `member create` refuses an earlier probed version (§6.9 Profile selection by minimum version).

**hermes** — `ensure_available` resolves the Hermes home per §6.3 *Config-dir resolution* (`HERMES_HOME`, default `~/.hermes`), checks `hermes` on `PATH`, then runs § Hermes availability preflight. `display_name` is ignored; Hermes has no `--name` analog.

```
["hermes", "chat", "--cli", "--source", "tool", "--in", <cwd>]
  (+ ["--model", <model>]       if model is not None)
  (+ ["--reasoning", <effort>]  if effort is not None)
  (+ ["-q", <prompt>])                               # prompt via flag — TWO tokens
```

The argv carries no permission tokens (the operator configuration's `approvals.mode` governs the native session) and never `--provider` (the operator's configuration selects the provider). Every spawn starts a fresh conversation: the argv never contains `--resume`, `--continue` or `--create-if-missing`. `--source tool` keeps member sessions out of the operator's default session lists, and `-q` on a terminal seeds the interactive session with the prompt as its literal first turn.

**pi** — `ensure_available` resolves the pi base per §6.3 *Config-dir resolution* (`PI_CODING_AGENT_DIR`, default `~/.pi/agent`), checks `pi` on `PATH`, then reads the CAFleet pi extension `<pi base>/extensions/cafleet.ts` in every launch mode, because every pi member depends on its gate (§ pi member gate). `display_name` becomes `--name`; `cwd` is unused.

```
["pi", "--cafleet-gate", "cafleet", "--no-approve", "--tui-mode", "fullscreen", "--name", <display_name>]
                                                     # explicit selection replaces "cafleet" with <mode>
  (+ ["--model", <model>]     if model is not None)
  (+ ["--thinking", <effort>] if effort is not None)
  (+ ["--", <prompt>])                               # bare trailing positional after --
```

The permission argv is the `--cafleet-gate <mode>` pair, where `<mode>` is `cafleet` or `cafleet-ask`. `--no-approve` ignores trust-gated project-local configuration and resources, so no project-trust dialog blocks a member; CAFleet skills load from `<pi base>/skills`, and `AGENTS.md` and `CLAUDE.md` still load. `--tui-mode fullscreen` pins the layout the pi profile matches, whatever the operator's `tuiMode` setting. `--` keeps a prompt that begins with `-` positional. Every spawn starts a fresh session: the argv never contains `--continue`, `--resume`, `--session`, `--session-id` or `--fork`. Spawn is validated against pi 1.5.0; the pi profile from `1.5.0` is the first boundary, so `member create` refuses an earlier probed version (§6.9 Profile selection by minimum version).

**Prompt rule.** pi reads a positional argument beginning with `@` as a file attachment, so pi's `validate_prompt` rejects a prompt body whose first character is `@` with usage exit 2 and `Error: a pi spawn prompt begins with text, not '@' (pi reads a leading '@' argument as a file attachment).`, before registration: no member row, run or pane is created. Checking the raw body is equivalent to checking the rendered prompt, because a leading placeholder renders to digits, `default`, a backend token or `sqlite:///…`. `build_spawn_argv` asserts the invariant for every prompt it receives.

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
| `claude` | None | Yes, via `--name <member-name>` | Runs its spawn-time allow rules and every command matching `permissions.allow`; auto mode's classifier reviews the rest, and a refused command routes to the Director | A Claude Code permission profile; a permission posture, not a spawn dependency |
| `codex` | Kernel-enforced workspace-write sandbox | No; `member list` `pane_id` locates the pane | Runs commands its workspace/host rules permit; automatic review decides escalations, and a refused required command routes to the Director | `<codex home>/rules/cafleet.rules`, the `config.toml` settings below and a trusted working directory; a permission posture, not a spawn dependency |
| `opencode` | None | No; `member list` `pane_id` locates the pane | `cafleet-ask` (default): the member broker commands run and every other shell command opens a native permission form; `cafleet`: the same broker commands, every other command denied | The selected policy's preset `<preset base>/agents/<mode>.md`; a spawn precondition |
| `hermes` | None | No; `member list` `pane_id` locates the pane | `smart`: commands outside Hermes's dangerous-command patterns run unreviewed, and the guardian model reviews the rest and may open an approval prompt; `manual`: every pattern-detected command opens an approval prompt. Hardline floors, the sudo guard and `approvals.deny` block before any review | The operator configuration's `approvals.mode` |
| `pi` | None | Yes, via `--name <member-name>`; pi titles the terminal `π - <name> - <cwd basename>` | `cafleet-ask` (default): the member broker commands run and every other bash command opens a native permission form, the gate's approval form for a command without a control character (§ pi member gate); `cafleet`: the same broker commands, every other command denied | The CAFleet pi extension `<pi base>/extensions/cafleet.ts`; a spawn precondition |

The omitted claude and codex policies are native automatic approval modes (§6.9 Launch policy and capability registry). The omitted hermes policy `smart` is also an `auto` mode: an ESCALATE or a guardian DENY opens a native approval prompt that the Director answers allow once or deny through the permission commands (§6.9 Bundled profiles and native screen fixtures), and an unanswered prompt fails closed after the configuration's `approvals.timeout`. Every session that reads a Hermes configuration applies its `approvals.deny`, so an operator glob there that matches a Director-only command also blocks it in a Hermes Director's own session, reported as `BLOCKED: this command matches the user-defined deny rule '<glob>' …`. CAFleet needs no `approvals.deny` entry, because the runner guard covers members (§6.3 Director-only command guard). The omitted OpenCode and pi policy `cafleet-ask` is a `manual` mode whose form the Director answers; explicit `cafleet` is an `auto` mode with a fixed allow/deny policy that renders no approval form (§ opencode preset, § pi member gate).

#### Claude permission posture

Under the omitted `--permission-mode auto`, a member runs its spawn-time allow rules and every tool call matching `permissions.allow` without review; Claude Code's auto mode classifier reviews each other call, and a refused call returns to the member, which routes it to the Director like any denial. Native rules, hooks and managed policy remain authoritative (a `deny` or `ask` rule still applies), and auto mode must be available for the account and model; where it is not, the member can show native permission forms, and the runner reports `automatic_review_unavailable` when its profile recognizes the notice. The runner holds the auto-mode dialogs `Make auto mode your default permission mode?` (shown when the user settings pin a non-`auto` default) and `Teach auto mode about your environment?` (shown in auto mode) as `question` interactions, which the Director dismisses through `permission respond --choice` (§6.9 *Bundled profiles and native screen fixtures* states the choices and when each dialog stops reappearing).

Explicit native modes keep their own permission behavior. Under explicit `dontAsk`, a member runs without a prompt its spawn-time allow rules, Claude Code's built-in read-only commands, file reads inside its working directory and every tool call matching `permissions.allow`; every other call, a shell command or a file edit including one matching `permissions.ask`, is denied without a prompt. A write to one of Claude Code's protected paths, such as a project's `.git` or `.claude` directory, is denied whatever the allow list holds.

Members need no settings entry for their broker commands, which they receive as the spawn-time allow rule `Bash(cafleet message *)`. Under `auto`, the classifier reviews each Director command that no rule matches, and auto mode approves the Director's file edits inside the working directory. An ask rule outranks every allow rule, so `ask` entries for the `Bash(cafleet member prompt *)`, `Bash(cafleet member exec *)` and `Bash(cafleet member press *)` prefixes beside `Bash(cafleet *)` still make the operator confirm each explicit native pane-input dispatch of the Director. Inside a member, the Director-only commands refuse through the runner guard (§6.3 Director-only command guard). An `Edit` or `Write` allow entry applies to every session that reads the settings file. When `CLAUDE_CONFIG_DIR` relocates the config directory, a `Read` entry for the installed skills names that directory's `skills/**`.

#### Codex sandbox and configuration

The kernel-enforced workspace-write sandbox confines writes to the workspace, and every Codex launch policy runs under it. §6.9 Launch policy and capability registry maps each choice to its argv, approval behavior and inherited Codex configuration.

A codex spawn requires these `config.toml` settings:

| Setting | Required value | Why |
|---|---|---|
| `network_access` | `true` | The multiplexer socket counts as network access |
| `writable_roots` | Includes the CAFleet database directory | The member writes the broker database |
| `trust_level` | `"trusted"` under `[projects."<absolute workspace path>"]` | The working directory must be trusted before spawning |

#### codex rules file

The codex auto-approval rules for `cafleet` commands are a static file embedded in the binary (source `assets/presets/codex/cafleet.rules`) and installed by `setup`'s asset installation (§6.3) to `<codex home>/rules/cafleet.rules`, with `<codex home>` resolved per §6.3 *Config-dir resolution* (`CODEX_HOME`, default `~/.codex`), overwriting any existing target. Codex loads every `*.rules` file under its rules directory (decision precedence below), so operator customizations live in a separate rules file there. The checked-in `assets/presets/codex/cafleet.rules` is the content contract: exactly the one line `prefix_rule(pattern = ["cafleet"], decision = "allow")` followed by one trailing newline.

The broad allow matches every `cafleet` command for every fleet, since every ID rides past the matched prefix as a positional or trailing argument. Codex members have no spawn-time allow rules, so they need it to run broker commands; the runner guard refuses the Director-only commands inside a member (§6.3 Director-only command guard). Codex applies the strictest decision when several rules match (`forbidden` > `prompt` > `allow`), so an operator who wants to confirm the Director's commands adds `prompt` rules in a separate rules file.

A rules directory maintained without the installed file needs at least an `allow` rule for the `["cafleet", "message"]` prefix, covering the broker commands members run themselves (those a claude member receives as spawn-time allow rules). That minimum set suffices for members, not for a Director session: the rules settle only the commands they match, so every other `cafleet` command a Director runs follows that session's own approval policy inside the workspace-write sandbox, where an escalation goes to Codex's automatic reviewer under the omitted `approve-for-me`, to a native approval form under explicit `on-request`, and returns as a failure under explicit `never`. The installed file is a superset of the minimum set.

#### opencode preset

Each opencode launch policy names an agent definition of the same name, a
static file installed by §6.3 Asset installation:

| Launch policy | Spawn argument |
|---|---|
| `cafleet` | `--agent cafleet` |
| `cafleet-ask` (default) | `--agent cafleet-ask` |

The preset base is opencode's `--agent <mode>` discovery path: opencode 2.1.4 loads agent definitions from `~/.config/opencode` and from `OPENCODE_CONFIG_DIR`, so a relocated preset remains discoverable, and definitions with the same name can merge across those sources. `member create` forwards a set `OPENCODE_CONFIG_DIR` into the member environment (§6.3 *Member environment*), so the member's opencode loads the preset the precondition read.

The selected policy's preset is a spawn precondition (§ Interface) whose error (§ Contract error strings) names the selector as the targeted repair; plain `cafleet setup` also installs it.

Each preset's `shell` ruleset is its `"*"` base followed by four `"allow"` entries for the member broker commands: `cafleet message send *`, `cafleet message poll *`, `cafleet message ack *` and `cafleet message show *`, where `*` matches any character sequence, spaces included, so a member passes `--database-url <url>` after the subcommand. The allow entries overlap only the base, so their order carries no precedence. Every other `cafleet` command, Director-only commands and `message listen` / `message wait` included, falls to the base, and the runner guard refuses the Director-only commands inside the member (§6.3 Director-only command guard). Tool permission and assignment authority remain separate; the assigned role and Director instructions determine which allowed operations a member may perform.

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
| Interpreter or hook execution via a command the Director allows | `mise //:test` executes workspace-writable test code; `git commit` runs `.git/hooks` | An allowed command also runs the code and hooks it executes, under that one-time permission. |
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

To enter each command, Escape and backspaces clear the composer, the command is typed, Escape closes its completion menu, and Enter is pressed until the command takes effect, at most three times, one second apart, never after the client exits.

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

The permission argv stays independent of the pi base, so runtime observation re-derives it from the stored policy (§6.9 Effective policy record). pi 1.5.0 rejects a long option that no loaded extension registers, printing `Error: Unknown option: --cafleet-gate`: a gated member whose extension cannot register the flag (excluded from loading, a stale copy or a load failure) exits at startup, and the runner reports `runner_exited` with the native exit code instead of running the member ungated.

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
| Bash command not allowlisted, `cafleet` mode | `the CAFleet member gate runs only cafleet message send, poll, ack and show; route the command to the Director if it is required` |
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
  "cafleet message send *", "cafleet message poll *", "cafleet message ack *", "cafleet message show *",
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
| Allowed tooling | The gate constrains a cooperative member's tool calls and is not containment. A command the Director allows on the form or dispatches with `member exec` runs the workspace code it executes, which a member can edit (test code, `build.rs`, the `mise.toml` task definitions), and that code can write anywhere the user can, protected directories included |
| No sandbox | `read`, `grep`, `find` and `ls` reach any path, and a symbolic link inside the cwd can target a path outside it |
| `.env` files | The `.env` rule refuses only a `read`, `grep`, `find`, `ls`, `edit` or `write` call whose path names a `.env` file; a directory search (`grep` on `.`), an allowlisted broker `message send --file` naming a `.env` file, which copies its contents into a broker message, and a command the Director allows or dispatches still read `.env` contents |
| Protected directories | The rule covers only `edit` and `write`, and its path check follows no symbolic link, so a link elsewhere in the cwd that targets a protected directory stays writable; a command the Director allows or dispatches writes where its own tool writes, as `git` does under `.git/`, and workspace code it runs writes these directories like any other path |
| Project files | Project-root configuration files such as `opencode.json` and context files such as `AGENTS.md` and `CLAUDE.md` stay writable, because members edit them as ordinary work; a later session of any backend loads what a member writes there |
| Delivery fallback | Escape interrupts a running pi turn and cancels an open form. While coverage is `tested`, profile delivery queues instead of falling back to Escape; a `coverage=unknown` member's Escape-first delivery interrupts its run or dismisses the form |
| Multi-line commands | Every command holding a newline is blocked in both launch modes without a form, including `cafleet message send` with a multi-line positional body; a member sends a multi-line body with `--file PATH` from a file written inside the cwd |

#### Contract error strings

Model and effort rejections are in § Model selection and § Reasoning effort, and the hermes deletion error in § Hermes work settlement. The availability errors are:

| Condition | Error |
|---|---|
| PATH miss | `binary {binary_name} not found on PATH` |
| Selected opencode preset missing or unreadable | `cannot read opencode agent preset at {preset}: {cause}; run 'cafleet setup --coding-agent opencode' first` |
| CAFleet pi extension missing or unreadable | `cannot read the CAFleet pi extension at {path}: {cause}; run 'cafleet setup --coding-agent pi' first` |
| Invalid config-path variable (§6.3 *Config-dir resolution*) | `{var} must be an absolute path (got '{value}')` |
| Non-UTF-8 config-path variable or `HOME` (§6.3 *Config-dir resolution*) | `{var} is not valid UTF-8` |
| `hermes config path` cannot start, times out or exits nonzero | `cannot read hermes configuration through 'hermes config path': {cause}` |
| `hermes config path` prints no absolute file path | `cannot read hermes configuration through 'hermes config path': printed no absolute file path: {stdout}` |

#### Host evidence and native-input authorization

**claude**: A separate evaluator checks the session condition against the transcript; native background work defers evaluation and returns a host result. Director inbox records arrive through one native Monitor task (§ Role eligibility); the installed CAFleet skill's [Claude native recipe](https://github.com/himkt/cafleet/blob/main/assets/skills/cafleet/reference/coding-agents.md#claude-native-director-reception) owns discovery, invocation, renewal and evidence limits, and manual finite reception remains available when Monitor is unavailable or denied.

Native shell probes displayed command/stdout/stderr followed by a model turn; the trigger and any second permission check were not established. The Director authorizes each routed command under its host rules.

CAFleet input cancels no Claude tool approval or `AskUserQuestion` in the root Director's pane, which receives no automatic pane input, or a detected one in a runner-managed member's pane, where input queues while the interaction is open (§6.9 Delivery method and interaction priority). A runner-managed member's Escape-first input (delivery fallback for previews, `member prompt` and `member exec`) can cancel an open prompt the runner did not detect, and Claude Code then reports a user rejection before the input arrives. A rejection followed by a `[cafleet msg …]` line ending with the Escape-dismissal clause is delivery-caused: the member re-asks the question or re-issues the call. A profile-delivered preview ends with the resume clause only and sends no Escape, so a rejection before it stands. A rejection followed by Director prompt or native-command input follows that input, and the Director checks the pane for an open prompt before such dispatch.

**codex**: The objective belongs to the thread, with model-callable lifecycle tools and optional token-budget accounting. Director reception is `cafleet message listen <director-id> --codex-thread "$CODEX_THREAD_ID"`, started once through native `exec_command` (§6.3 Codex forwarding); Codex injects `CODEX_THREAD_ID` into every exec shell. Each forwarded record arrives as a queued user turn: it starts a turn on an idle session, waits for the end of a running turn otherwise, and interleaves with native goal continuation turns at turn boundaries. Where the sandbox cannot start the forwarder, for example when bwrap cannot create namespaces, it runs on the escalated path under the host's approval rules. The installed CAFleet skill's [Codex native recipe](https://github.com/himkt/cafleet/blob/main/assets/skills/cafleet/reference/coding-agents.md#codex-native-director-reception) owns the tool call, consumption and recovery.

A native shell probe displayed the actual command, exit code, duration and output. Installed protocol metadata describes unsandboxed dispatch while public CLI guidance describes current approval/sandbox settings; the TUI's effective boundary and protocol path were not established. Native input is a Director-authorized action, with no second member-tool permission check assumed.

**opencode**: OpenCode members perform supplied assignments with their selected provider/model, workspace reads/edits, permitted tools and broker communication; the Director coordinates next steps and routed native shell commands and owns explicit pane input dispatch. Leading-`!` input runs a native shell command in `mini`; its effective permission boundary and any second member-tool permission check remain unestablished, so the preset's member-tool permissions and the Director's command authorization stay in force. An OpenCode Director runs OpenCode 2.2.0 or later as `opencode mini --standalone` from the project directory with the default agent, so plugins load in its private server with the launcher's environment; the shared background service passes no client environment to plugins and is not a supported Director launch. The `cafleet` plugin supplies reception and the goal extension (§ Director extensions). OpenCode 2.2.0 gives every agent a base policy whose first rule allows every action, so the plugin's `cafleet` action resolves to allow on a default Director; a configuration whose broad `ask` or `deny` rule overrides that base allow restores it with `{ "action": "cafleet", "resource": "*", "effect": "allow" }`.

**hermes**: Hermes 2026.9.24 ships a user-operated `/goal` command with `status`, `show`, `pause`, `resume`, `clear`, `wait <pid>`, `unwait` and `gate` subcommands. A Hermes Director runs the TUI client (`hermes chat --tui`, or `display.interface: tui`), because turns injected through the gateway run without rendering in the classic CLI; members keep `--cli`. The Director receives inbox records through the `cafleet` plugin's `cafleet_listen` tool, which runs the listener inside the per-home gateway (§ Director extensions): each record becomes its own turn, and a record injected during a busy turn runs after that turn ends without interrupting it. Native `/goal` set, status, pause, resume and clear work in the TUI client, and continuation turns already queued before `/goal clear` still run after it. In the classic REPL and the TUI client, leading-`!` input runs a native shell command locally; the Director authorizes each routed command under its host rules. CAFleet makes no goal lifecycle calls and preserves activation, pause, completion and gate/wait authority.

**pi**: pi 1.5.0 has no native goal, no native facility that pushes external text into a session and no per-tool approval; the CAFleet pi extension supplies reception, the goal extension and the `cafleet_ask` decision surface (§ Director extensions), and the member gate (§ pi member gate). A pi Director runs plain `pi` (the fullscreen TUI) from the project directory, so the extension auto-loads; pi's bash tool inherits the pane environment, so `fleet create` registers the pane without `--pane`. A pi Director runs its own tool calls without approval, and the member gate is inert in its session. The installed CAFleet skill's [pi native recipe](https://github.com/himkt/cafleet/blob/main/assets/skills/cafleet/reference/coding-agents.md#pi-native-director-reception) owns the tool calls, consumption and recovery. Leading-`!` input runs a shell command as `user_bash` outside the member gate, stores its output for the next prompt and starts no turn, so after `member exec` the Director sends an explicit continuation; the Director authorizes each routed command under its host rules. Enter while a member works queues the text as a steering message.

#### Director extensions

The extensions §6.3 Asset installation installs supply Director reception for `hermes`, `opencode` and `pi`, and the `opencode` and `pi` goal extensions. Each runs only the `cafleet` executable its executable record names, never a path from a tool argument, as `<recorded cafleet> message listen <director_id>` without `--timeout`, adding `--database-url <url>` when the tool received a `database_url` (§6.3 Database selection). The plugins replace no receiver automatically; every replacement belongs to the Director's receiver recovery.

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
| Child environment | The gateway's environment with every `CAFLEET_*` name removed | The server's environment | The pi process's environment |
| Receiver end | Session finalize, plugin unload, 30 s with no client subscriber, a raising or overdue injection, or 600 s of refusals for one record kills the child | Plugin cleanup kills every child when the server ends | `session_shutdown`, for every reason, kills the child with `SIGTERM` |

A rejected `synthetic` call kills the child. A Hermes injection that raises or exceeds its 10 s deadline, or one record refused for 600 s, kills the child, logs the cause and attempts the closed notice below once through ordered injection. A child that exits unexpectedly, without a `closed` record and without the plugin stopping it, queues `[cafleet listen closed] exit <code>: <last stderr line>` into the owning session. A Hermes plugin unload for a still-attached session injects `[cafleet listen closed] the plugin unloaded: exit <code>: <last stderr line>` once through ordered injection after the child exits; a draining gateway refuses it, without retry. A child the plugin stops through session finalize, client detach, plugin cleanup or a Hermes unload for a detached session queues no closed notice, and each receiver attempts at most one. The OpenCode plugin decides the notice on the child's `close` event, after its stdio drains, so a final `closed` record is read first; a child `error` event, such as an asynchronous spawn failure, removes the receiver and queues the notice. An OpenCode child exiting by `SIGTERM` is an intentional stop by plugin cleanup or host shutdown (which can signal the child before cleanup runs) and queues no closed notice; a non-zero exit code or any other signal queues it. A `closed` record is delivered as a record and ends the receiver.

**Hermes plugin.** The plugin declares tool `cafleet_listen` and hook `on_session_finalize`. It depends on three private Hermes core seams outside the plugin API, which the tool resolves on every call:

| Seam | Use |
|---|---|
| `hermes_cli.plugin_session.session_key()` | Calling session key |
| `gateway.session_authorities.authority_for_home(...)` → `authority.sessions[...].subscribers` | Client attachment |
| The published gateway runner (`ctx._manager._gateway_message_injector[0]`), its `_gateway_loop` and `_dispatch_plugin_message_injection(session_key=, content=, plugin_id=, plugin_home=)` | Ordered injection |

`hermes plugins validate` flags the first two seams' imports as private core imports. A failed import, lookup or attachment read returns `{"ok": false, "error": "<seam>: <exception>"}` and starts no receiver; an empty session key returns `{"ok": false, "error": "cafleet_listen must be called from a session turn"}`. Ordered injection bypasses the consent check of `ctx.inject_message`, so before spawning the child the tool requires `ctx._gateway_injection_allowed()`, which holds when `plugins.entries.cafleet.allow_gateway_injection` is `true` (§6.3 Hermes plugin enablement), and otherwise returns `{"ok": false, "error": "plugins.entries.cafleet.allow_gateway_injection is not true; run 'cafleet setup --coding-agent hermes'"}`; it then verifies `director_id`. The child selects its database only through `--database-url`, so an omitted `database_url` selects the default database exactly as in the Director's shell. The plugin decodes child output as UTF-8, replacing invalid bytes, and treats a stdout read failure as the end of output. A watchdog checks every 5 s that the session has at least one subscriber; a read error kills the child. `on_session_finalize` resolves the finalized session's route through the session authority and stops the receiver with that session key, so a session id rotated by context compression still matches. The unload callback stops every receiver and confirms its exit, so plugin re-discovery leaves no untracked receiver; re-discovery leaves the gateway and its sessions running, so the unload closed notice, sent outside the callback after the receiver's delivery ends, tells a still-attached Director to re-arm reception.

**Ordered injection.** `ctx.inject_message` returns `True` once it schedules its dispatch coroutine on the gateway loop, before the record is committed, so consecutive calls can commit out of order. The plugin instead submits each record's `_dispatch_plugin_message_injection(...)` coroutine to the runner's `_gateway_loop` with `asyncio.run_coroutine_threadsafe` and waits up to 10 s for its result before reading the next line. A `False` result is a refused injection, treated as transient (for example the session authority's 32-row admission queue cap, a draining gateway or a briefly detached client): the plugin keeps the record, waits 1 s and retries it, and later records wait behind it. The retry checks no attachment, so a detached session keeps the 30 s detach grace and the watchdog alone ends its receiver.

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

Settings retain only the selected SQLite URL, built from `--database-url` (§6.3 Database selection). CAFleet reads no `CAFLEET_*` environment variable; it reads the backend config-location variables and `HOME` (§6.3 Config-dir resolution), and only `fleet create` and doctor's `multiplexer` row read the caller's multiplexer environment (§6.5 Backend selection). Spawned members receive, when the creator sets it, the selected backend's config-location variable (§6.3 *Member environment*); identity and the database are rendered into prompt literals (§6.3 Spawn-prompt substitution).

### 6.9 Member PTY runtime

#### Launch policy and capability registry

`cafleet member create --permission-mode <MODE>` selects a launch policy. Resolve the coding-agent backend first, validate `MODE` against that backend's typed launch-policy catalog, then allocate the member or pane. An omitted value selects the backend default below; an empty, unsupported or unavailable explicit value fails with usage exit `2`, naming the backend and supported choices. Selection is launch-only and is independent of Director policy; it introduces no fleet inheritance, environment default or live policy mutation.

| Backend | `auto` modes | `manual` modes | Explicit selection mapping |
|---|---|---|---|
| Claude | `auto` (default), `dontAsk`, `bypassPermissions` | `default`, `manual`, `acceptEdits`, `plan` | The native permission-mode value through `--permission-mode`; retain broker-tool allowance, prompt suggestions, name and model/effort boundaries. |
| Codex | `approve-for-me` (default), `never` | `on-request` | `on-request` and `never` map to `--ask-for-approval <mode> --sandbox workspace-write`; `approve-for-me` maps to the single `--approve-for-me` flag, which selects the workspace-write sandbox itself. |
| OpenCode | `cafleet` preset | `cafleet-ask` preset (default) | `--agent <mode>`, selecting the agent preset of that name; explicit `cafleet` records `mode_source=explicit`. |
| Hermes | `smart` (default) | `manual` | No permission argv for any native version; the choice is recorded only, and the operator configuration's `approvals.mode` governs the native session. |
| pi | `cafleet` | `cafleet-ask` (default) | `--cafleet-gate <mode>`, selecting the CAFleet member gate's mode (§6.7 pi member gate); explicit `cafleet` records `mode_source=explicit`. |

Each backend has one default (omitted) mode, marked in the table above. Every launch mode carries an `auto` or `manual` class, a documentation and catalog property: `--permission-mode` values stay backend-native, and the class changes no argv, run-record field or CLI contract.

| Class | Definition |
|---|---|
| `auto` | Native automatic review or a fixed allow/deny policy: a command the policy does not settle goes to the automatic reviewer, or is allowed or denied without a native prompt. An `auto` mode may still raise an approval prompt when its reviewer escalates. |
| `manual` | Native interactive approval: the backend asks a human on its native surface for operations its rules do not settle. |

Claude `dontAsk` denies tools that are not pre-approved without prompting, Claude `bypassPermissions` allows without prompting, and Codex `never` returns failures without asking, so each is a fixed policy. Claude `acceptEdits` and `plan` still prompt. When an approval prompt that the run's bundled profile covers appears in a member pane, the runner reports it as `interaction_opened`, and the Director answers it with a single-request allow or deny through the permission commands (§6.3 Permission commands).

The omitted Claude and Codex selections are each backend's native automatic approval mode: an automatic reviewer decides the operations that no native rule settles, and the runner reports its observable refusals and unavailability as automatic review events (§ Automatic review events). An omitted selection is not validated against the version catalog, so a member whose probed version is newer than every profile boundary still launches with the default; a native version that rejects `auto` or `--approve-for-me` exits immediately with `runner_exited` (below).

The bundled backend policy catalog is the single source of truth for each backend's launch-policy catalog, default, exact argv mapping and detector/composer/answer profiles; help and usage errors report its choices, and each backend accepts them for every native version. Claude accepts the native `--permission-mode` values, documented aliases included, verified from installed help and parser evidence: the verified Claude Code 2.1.302 and 2.1.312 accept canonical `default` and its `manual` alias, help displays `manual`, and the [official permission-mode reference](https://code.claude.com/docs/en/permission-modes) documents both. Preserve the explicitly requested spelling in argv and launch intent. The Codex choices are verified from the installed `codex --help` of Codex 0.163.0; `approve-for-me` names the native `--approve-for-me` flag, which routes approval requests through automatic review, and explicit `on-request` and `never` inherit the user's Codex configuration, including an `approvals_reviewer` setting that can answer escalations without a native form. The OpenCode choices select agent presets rather than a native permission flag: `cafleet` resolves every shell check to `allow` or `deny`, and `cafleet-ask` asks for every shell command outside the member broker commands, so OpenCode opens a native permission form. Hermes emits no argv because no Hermes launch flag selects an approval mode. The pi choices select the mode of the CAFleet member gate, because pi 1.5.0 has no native per-tool approval: `cafleet` runs the member broker commands and blocks every other bash command without a form, so it renders no approval form, and `cafleet-ask` opens the gate's approval form for every bash command outside the member broker commands.

Creation success establishes that the child was spawned, not that its native policy was accepted. A fast native exit after spawning, including policy rejection, produces `runner_exited` with the actual code/signal and a Director inbox event; the runner preserves the chosen mode rather than retrying with another one. An error from the spawn operation itself still fails creation and follows compensation. There is no post-spawn liveness window.

Record `requested_mode` as nullable text, `resolved_mode` as required text, `mode_source` as `default` or `explicit`, and the exact non-secret permission/sandbox argv in the run record. These fields state launch intent:

| Selection | `requested_mode` | `resolved_mode` | `mode_source` | `permission_argv_json` |
|---|---|---|---|---|
| Omitted Claude | `null` | `auto` | `default` | `["--permission-mode","auto"]` |
| Omitted Codex | `null` | `approve-for-me` | `default` | `["--approve-for-me"]` |
| Explicit Codex `never` | `never` | `never` | `explicit` | `["--ask-for-approval","never","--sandbox","workspace-write"]` |
| Omitted Hermes | `null` | `smart` | `default` | `[]` |
| Explicit Hermes `manual` | `manual` | `manual` | `explicit` | `[]` |
| Omitted pi | `null` | `cafleet-ask` | `default` | `["--cafleet-gate","cafleet-ask"]` |
| Explicit pi `cafleet` | `cafleet` | `cafleet` | `explicit` | `["--cafleet-gate","cafleet"]` |

`effective_policy_json` remains null unless a tested native observation supplies it (§ Effective policy record); an installed version, argv or selected profile alone proves no effective authorization decision.

Launch is independent of every profile capability; detection, annotation, composer, submission and answer capabilities resolve independently through the selected profile (§ Profile selection by minimum version), and answer profiles declare each semantic decision independently. A bundled tested answer declaration rests on measured single-request answer stages. Synthetic fixtures exercise rule and capability validation while unverified components remain unknown or unsupported. A failed probe or a backend without bundled profiles yields `coverage=unknown` and an initial coverage notification; it still permits manual terminal operation, records the selected mode and delivers ordinary programmatic input through delivery fallback, and automatic/fixed policies remain useful where screen detection is unknown. The `cafleet` OpenCode preset's returned denials continue through member evidence routing; returned tool responses are observable only when the native terminal exposes them.

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

Hermes replaces the one-token rule with its own first-line rule. The first stdout line must match `^Hermes Agent v\S+ \((?P<version>[^)]+)\)$`, and the captured value must parse as canonical SemVer; that value is the recorded `native_version`. A non-matching line, or a capture that is not canonical SemVer, is a probe failure:

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

One shared SemVer parser and precedence comparison serves profile validation, native-version admission, fixture validation and selection. Numeric components compare numerically, prerelease identifiers by SemVer precedence, and a release ranks above its prereleases. Build metadata contributes no precedence. Selection is independent of catalog order.

The run retains the actual probed `native_version`; the backend and that version determine the selected profile within one binary. A composer plan and an answer capability retain the actual run version and the cited fixture identities, while each fixture record keeps its original capture version; input validation compares the backend and native version with the run rather than requiring the run version to equal a capture version or boundary.

The runner pins its selected profile for its lifetime. Fresh creation and observer initialization verify that deterministic selection from the recorded backend and native version yields the recorded coverage, so a `tested` coverage requires a selected profile and a null native version requires `unknown`. Historical run, event and audit reads preserve their recorded identities; recovery reports a recorded `tested` coverage for which no profile is selected now explicitly rather than attaching another profile. A catalog update affects newly launched runners only.

The rules match screen patterns, so a changed native UI fails safe for detection: an unmatched form yields the `unknown` or `unknown_interaction` observation and offers no answer plan, and an unverified composer selects delivery fallback for ordinary input. An answer plan writes only after its request rule, choices and gates match the fresh observation.

Native evidence is the provenance of a profile interval. Catalog validation requires every cited fixture's backend to match its profile and its capture version to lie at or above the profile's `min_version` and, when the backend has a next boundary, below it. Each subsequent native release within the interval reuses the declaration without another exact-version capture. A new profile is added only when an observed native change requires different rules or plans, with `min_version` set to the first native version requiring the change.

##### Bundled profiles and native screen fixtures

The bundled catalog and the native screen fixtures recorded in its fixture registry are embedded in the CAFleet binary. Each profile's capabilities rest on accepted native captures taken on a native version within its interval. A bundled profile entry has required `backend`, `min_version`, `rules` and `capabilities`, and no other key; documentation and messages name a profile by its backend and boundary, as in "the OpenCode profile from `2.1.4`". Each fixture entry keeps its own `native_version` as the actual capture version. Each existing profile's `min_version` is the capture version of its earliest native fixtures; fixtures captured on later versions inside the interval add declarations to the same entry, and adding operation plans to an existing entry changes its contents without introducing a new boundary.

The embedded fixture registry uses `assets/profiles/native/<backend>.json` (§6.3 Asset installation) for each backend with catalogued native screens. A fixture entry has exactly `id,kind,backend,native_version,rows,cols,source_ref,sha256`; `source_ref` selects `native/<backend>.json`, and `sha256` is the lowercase SHA-256 hex of `digest::canonical_json` applied to that entry's screen object. The loader validates every catalog fixture entry, including screens that no profile capability cites.

Every decision answer plan selects a single-request choice, and every choice answer plan selects a declared dismissal choice of a question; other broadened, persistent or session-wide choices have no answer plan, and a focus on a choice without a choice entry satisfies no gate. Native lines a capture shows after an answer or a timeout are decision evidence for the Director's continuation check, not bundled rules. Codex 0.163.0 shows no startup notice or opt-in dialog for `approve-for-me`, so its profile declares no startup rule.

The Claude profile from `2.1.302` applies to Claude Code under every Claude launch policy:

| Capability | Status | Declaration |
|---|---|---|
| Detection | `tested` | Rule `claude-bash-permission`: the native `default` Bash permission form (`Bash command` heading, approval text, proceed question, focused choice and the `Yes`/`No` single-request choices) on the primary or alternate screen, with a plain or boxed operation, unwrapped or wrapped, within 24 rows from its heading to its `Esc to cancel · Tab to amend` line. Request rules `claude-bash-permission-auto-tip`, `claude-bash-permission-no-tip`, `claude-auto-ask-permission`, `claude-read-outside-permission`, `claude-read-file-permission`, `claude-edit-file-permission` and `claude-create-file-permission`, and question rules `claude-auto-default-nudge`, `claude-question-pending-notice`, `claude-auto-startup` and `claude-auto-env-setup-wizard`: described below |
| Allow answer | `tested` | One stage per request rule selecting its single-request allow choice as `allow-once` (answer table below) |
| Deny answer | `tested` | One stage per request rule selecting its single-request deny choice as `deny-once` (answer table below) |
| Question answers | `tested` | Choice `decline` of `claude-auto-default-nudge`: stage `0e` (Ctrl+N) while `accept` is focused, then `0d` (Enter) while `decline` is focused. Choices `not-now` (byte `32`) and `dont-show-again` (byte `33`) of `claude-auto-startup`, one ungated stage each |
| Empty-composer observation | `tested` | The idle prompt line with the terminal cursor immediately after the prompt prefix; a verified non-interaction boundary |
| Annotations | `tested` | `claude-auto-refusal`, `claude-auto-refusal-notice`, `claude-auto-unavailable` and `claude-auto-mode-indicator`, described below |
| Prompt submission | `tested` | Composer plan `prompt` (§ Executable native plans) |
| Native-exec submission | `tested` | Composer plan `exec` (§ Executable native plans) |

Every permission answer and both Teach-auto-mode choices select their choice by its number, independent of focus; the Make-auto-mode-default dialog has no numbered choices, so its `decline` plan gates each stage on the marked focus. The empty-composer observation reads the composer as empty only when the cursor sits at the end of the prompt prefix; a draft whose cursor was moved back to the prefix reads as empty.

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

Claude positions every row explicitly, so the terminal records no soft wrap, and a displayed row break cannot be told from a line break in the command itself. The rule therefore reports the operation as its displayed rows: it removes exactly the one-space margin from each operation row, keeps the boxed `│ ` gutter (also on a single boxed row), and joins the rows with line breaks. The evidence records the number of displayed rows as `operation_rows`, and an operation over more than one row extends the Director's review instruction (§6.9 Request-bound permission responses). Every Claude request and question rule has priority 100 and matches on the primary or alternate screen; every request rule also requires the cursor inside, spans at most 24 rows and, except the Edit file and Create file rules, extracts its operation through this shared operation matcher with its one-space `row_margin`.

The request identity follows the displayed rows. A resize that rewraps a multi-row operation, or moves it between one row and several, replaces its episode; a resize that leaves every operation row unchanged keeps it.

Claude Code 2.1.302 in `auto` shows a native permission form only through an explicit `permissions.ask` rule. That form differs from the `default` form: it carries a `│ Ask rule … overrides auto mode for this command.` row and a `/permissions to let auto mode decide` row, has no `This command requires approval` line, and offers `2. No` as its deny choice. Rule `claude-auto-ask-permission` spans from `^ Bash command$` to `^ Esc to cancel · Tab to amend$` and requires these line matchers:

- `^ │ Ask rule \S`
- `^ /permissions to let auto mode decide$`
- `^ Do you want to proceed\?\s*$`
- `^ ❯ [1-9]\. ` for the focus, `^ [❯ ] 1\. (?P<label>Yes)$` and `^ [❯ ] 2\. (?P<label>No)$` for the choices

Under a non-automatic mode such as an explicit `default`, Claude Code 2.1.302 can show another Bash form for a workspace file write. It replaces the `This command requires approval` line with a `Tip: auto mode handles these prompts for you — choose "switch to auto mode" below` row and a description row, and offers four choices. Rule `claude-bash-permission-auto-tip` spans from `^ Bash command$` to `^ Esc to cancel · Tab to amend$` and requires these line matchers:

- `^ Do you want to proceed\?$` and `^ ❯ [1-9]\. ` for the focus
- `^ [❯ ] 1\. (?P<label>Yes)$` and `^ [❯ ] 4\. (?P<label>No)$` for the single-request choices
- `^ [❯ ] 2\. Yes, and always allow access to \S.* from this project$` and `^ [❯ ] 3\. Yes, and switch to auto mode · auto mode handles these prompts for you$` for the broadened choices

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
| `claude-auto-startup` (offer) | `^  Teach auto mode about your environment\?\s*$` | `^  Enter to confirm · Esc to cancel\s*$` | heading, footer, focus `^  ❯ [1-9]\. `, `^  [❯ ] 1\. Yes\s*$`, and the choices `not-now` (`^  [❯ ] 2\. (?P<label>Not now)\s*$`) and `dont-show-again` (`^  [❯ ] 3\. (?P<label>Don't show again)\s*$`), both `persistent` |
| `claude-auto-env-setup-wizard` | `^   Teach auto mode about your environment\?\s*$` | `^   ←/→ to change · Enter to continue · Esc to cancel\s*$` | heading, footer, focus `^   ❯ \S`; no choices |

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

The second label names the user-settings default (`No, keep manual mode` for `default`, `No, keep accept edits` for `acceptEdits`). Focus starts on `Yes` and is marked only by the `❯` glyph and a foreground colour; Ctrl+N and Ctrl+P (also the arrow keys) move and wrap it, Enter confirms the focused choice, typed text is discarded and Escape declines. `Yes` writes `permissions.defaultMode` `auto` into the user settings and switches the session; `No` and Escape set `hasSeenAutoDefaultNudge` `true`. The hidden terminal cursor sits at column 3 of the focused choice row. Question rule `claude-auto-default-nudge` (cursor inside, at most 10 rows) spans from `^ Make auto mode your default permission mode\?\s*$` to `^   (?:❯ |  )No, keep \S.*$` and declares the interaction frame guard rejecting `^❯\u{a0}`, so a quoted dialog above a live composer never matches:

- `heading` `^ Make auto mode your default permission mode\?\s*$`
- `focus` `^   ❯ \S` with `focus_style` `marker` (§ Focused-choice resolution)
- choice `accept` (`^   (?:❯ |  )(?P<label>Yes, set auto mode as my default permission mode)\s*$`) and choice `decline` (`^   (?:❯ |  )(?P<label>No, keep \S.*?)\s*$`), both `persistent`

The `decline` label carries the kept default's name and is part of the fingerprint; the explanatory paragraph has no matcher, and the rule declares no operation. Answer plan `claude-auto-default-decline` for choice `decline` has two focus-gated stages: `0e` (Ctrl+N) while `accept` is focused moves the focus to `No, keep …`, then `0d` (Enter) while `decline` is focused closes the dialog as declined. Ctrl+N wraps, so a focus already on `decline` fails the first gate before any byte. `accept` is declared only so a gate can name it and has no plan. In the captures the plan closed the dialog and changed only `hasSeenAutoDefaultNudge`.

When a draft reaches the composer before the dialog mounts, Claude hides the dialog and shows the notice row `Claude has a question for you — it shows once you send or clear what you're typing.` at column 0 above the composer. Question rule `claude-question-pending-notice` (cursor any, one row) matches that row, `^Claude has a question for you — it shows once you send or clear what you're typing\.\s*$`, and declares the Claude annotation rules' frame guard; model text quoting the words renders with the `● ` or two-space transcript margin and does not match. It declares no choices, operation or answer plan. As a candidate interaction the notice stops a profile submission after its literal stage and before its submit key (§ Measured submission plans and settle intervals) and queues later input. Two native outcomes remove it and close its episode as `not_visible`:

| User action on the native surface | Native outcome in the 2.1.312 captures |
|---|---|
| Clear the draft | The notice disappears and the dialog is drawn in its place at once, opening its own `question` episode |
| Send the draft | For a draft typed during a turn, the first Enter submits it and the notice disappears; a draft typed at startup before any turn needs a second Enter. The dialog does not reappear in that process and `hasSeenAutoDefaultNudge` stays unset, so it returns at the next launch |

Claude Code 2.1.309 asks before a file tool reads outside the working directories, also in `auto`. The dialog shows the read operation between dashed rules, an explanatory paragraph, the question and four choices, and opens focused on choice 1. Once the native configuration's `.claude.json` holds `hasSeenAutoModeOutsideReadPrompt` `true`, later reads outside the working directories in `auto` proceed without asking; when Claude writes the flag is not established. Rule `claude-read-outside-permission` spans from `^ Read outside the working directories\s*$` to `^ Esc to cancel · Tab to amend\s*$` and requires `^ Read outside the working directories\s*$` for the heading, `^ Allow this read outside the working directories\?\s*$` for the question, `^ ❯ [1-9]\. ` for the focus and one line matcher per choice:

| Choice matcher | Scope |
|---|---|
| `^ [❯ ] 1\. Yes, and keep allowing any reads outside the working directories\s*$` | Changes a persistent setting |
| `^ [❯ ] 2\. No, and block reads outside the working directories from now on\s*$` | Changes a persistent setting |
| `^ [❯ ] 3\. (?P<label>No, and ask again next time)\s*$` | This read only, as `deny-once` |
| `^ [❯ ] 4\. (?P<label>Yes, but ask again next time)\s*$` | This read only, as `allow-once` |

The operation is the displayed `Read(<path>)` rows. A long path continues on a further row with the one-space margin, so the evidence reports it with `operation_rows` above 1. The explanatory paragraph has no matcher. The rule rests on 2.1.309 captures at 120 columns; an earlier version that never renders the dialog never matches it.

| Limit | Behavior |
|---|---|
| Panes narrower than 120 columns | Only 120-column renderings were captured; a narrower rendering whose rows differ from the matchers is an `unknown` frame, and the dialog is answered on the native surface |
| `Tab to amend` | CAFleet offers no amend action |
| Panes too short to show the heading row | The region has no start row, so the frame is `unknown` until a resize redraws the dialog in full |
| Other file tools | In the 2.1.309 captures, Edit first read its target and showed this same dialog, Write ran without a dialog under the auto mode classifier, and the session offered no Glob or Grep tool. An outside-working-directory dialog with other literals is an `unknown` frame with no event |

In `default` mode, Claude Code 2.1.312 asks before a file tool touches a path outside the working directories. Each form has a heading, dashed `╌` rules around the operation, a question, three numbered choices and the footer ` Esc to cancel · Tab to amend`; the cursor sits on the focused choice row. Three request rules, ending at `^ Esc to cancel · Tab to amend\s*$`, claim them. They share the focus matcher `^ ❯ [1-9]\. `, the single-request choices `allow-once` (`^ [❯ ] 1\. (?P<label>Yes)\s*$`) and `deny-once` (`^ [❯ ] 3\. (?P<label>No)\s*$`), and a choice-2 matcher anchored on the label's start, because choice 2 can wrap onto a six-space continuation row:

| Rule | Heading | Operation | Question | Choice 2 (no plan) |
|---|---|---|---|---|
| `claude-read-file-permission` | ` Read file` | The `Read(<absolute path>)` rows, through the shared operation matcher | ` Do you want to proceed?` | `Yes, allow reading from <dir> during this session` |
| `claude-edit-file-permission` | ` Edit file` | The path row below the heading, relative to the working directory | ` Do you want to make this edit to <file>?` | `Yes, and switch to accept edits …` (session) |
| `claude-create-file-permission` | ` Create file` | The path row below the heading, relative to the working directory | ` Do you want to create <file>?` | `Yes, and switch to accept edits …` (session) |

The Edit form shows the diff and the Create form the new content between the dashed rules; neither is extracted, and the Director reviews it from the capture excerpt. Edit first shows the Read form for its target unless reads of that path are allowed.

When auto mode cannot be offered (`permissions.disableAutoMode` `disable`, an unavailable plan or model, fast mode, an ask rule or hook, or `plan` mode), Claude Code 2.1.312 drops the Tip row and the `switch to auto mode` choice from its Bash form and shows three choices. Rule `claude-bash-permission-no-tip` (from `^ Bash command\s*$` to `^ Esc to cancel · Tab to amend\s*$`) claims both of its variants:

| Variant | Shown for | Rows after the description row |
|---|---|---|
| Approval | A command inside the working directories | The operation with the one-space margin, ` This command requires approval`, a blank row, the question and choice 2 ` Yes, and don’t ask again for: <command prefix> *` (U+2019 apostrophe) |
| Access | A command touching a path outside the working directories | The operation in the boxed `│ ` gutter, then the question with no blank row, and choice 2 ` Yes, and always allow access to <dir> from this project` |

Besides the shared operation matcher (which keeps the boxed gutter, as for `claude-bash-permission`), the rule requires `^ Do you want to proceed\?\s*$`, the focus `^ ❯ [1-9]\. `, `allow-once` `^ [❯ ] 1\. (?P<label>Yes)\s*$`, the choice-2 matcher `remember` (either label, no choice entry) and `deny-once` `^ [❯ ] 3\. (?P<label>No)\s*$`. The approval line has no matcher, and the model-authored description row has none either. `claude-bash-permission` and `claude-bash-permission-auto-tip` require `4. No`, and `claude-auto-ask-permission` requires its `│ Ask rule` row and `2. No`, so the four Bash rules match disjoint forms.

When Claude Code 2.1.312 issues several tool calls in one turn, it asks for them one form at a time and right-aligns a request counter `N of M` at the end of each form's heading row (` Read file` … `1 of 2`). The `claude-read-file-permission`, `claude-edit-file-permission` and `claude-create-file-permission` rules and every rule anchored on the ` Bash command` heading accept an optional `(?:\s+\d+ of \d+)?` before the end of their region start pattern and their heading matcher; the Edit and Create rules also accept it on the heading line of their operation matcher. The counter is not evidence, not part of the operation and not part of the request identity, so a batched form classifies with the same rule and evidence as its single form. The answer plans stay the single-stage digits:

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

Claude renders an automatic refusal in one of two native forms, depending on the user's Verbose output setting, and the profile claims both. Every Claude annotation rule (priority 40, primary or alternate screen, cursor any) declares the same frame guard: require `^❯\u{a0}` (the live composer prefix) and reject `^  Showing detailed transcript · ctrl\+o to toggle` (the transcript viewer's footer).

| Rule | Region | Captured screen |
|---|---|---|
| `claude-auto-refusal` | Start `^● [A-Z][A-Za-z]*\(`, end `^  ⎿ \u{a0}Denied by auto mode classifier ∙ \[`, at most 8 rows | The verbose-view transcript block `● Bash(<command>)` followed by `  ⎿  Denied by auto mode classifier ∙ [<category>] ∙ see https://code.claude.com/docs/s/claude-code-auto-mode`, kept in the live transcript |
| `claude-auto-refusal-notice` | Start `^ +[a-z][a-z_]* denied by auto mode · \[[^\]]+\] · /permissions`, end the composer border `^─{3,}(?: \S.* ─)?$`, at most 2 rows | The default-view status-row notice `<tool> denied by auto mode · [<category>] · /permissions` above the composer border, shown for about 8–10 seconds |
| `claude-auto-unavailable` | Start `^ +auto mode disabled by settings\b`, end the composer border, at most 2 rows | The transient notice `auto mode disabled by settings`, induced by `permissions.disableAutoMode`; the footer then shows `⏸ manual mode on` |
| `claude-auto-mode-indicator` | Start and end `^  ⏵⏵ auto mode on \(shift\+tab to cycle\)`, one row | The footer badge on every live `auto` frame; its `mode` capture records `observed_mode` `auto mode` |

`claude-auto-refusal` matches its region with one region-scoped matcher that captures the refused tool, the operation between the parentheses and the category, so the operation evidence is the refused command. Its first row follows `● Bash(` and carries no margin, so the rule declares no `row_margin`, and a wrapped operation reports its continuation rows with their two-space margin. `claude-auto-refusal-notice` captures `<tool> denied by auto mode · [<category>]` as its operation, because the default view never shows the refused command. `claude-auto-unavailable` captures its notice with a null operation. The native result prefix carries a U+00A0 after `⎿`, while a model quote of the same words carries two ASCII spaces, so quotes do not match.

The Codex profile from `0.163.0` applies to Codex CLI under every Codex launch policy; the request and answer rules come from captures under `--ask-for-approval on-request --sandbox workspace-write`:

| Capability | Status | Declaration |
|---|---|---|
| Detection | `tested` | The native command-approval form in the alternate screen, from its `Would you like to run the following command?` heading to its `Press enter to confirm or esc to cancel` line within 24 rows: the `Environment:` and `Reason:` lines, the single-line `$ ` command, the focused `›` choice and the `1. Yes, proceed (y)` / `3. No, and tell Codex what to do differently (esc)` single-request choices, with the cursor inside the form |
| Allow answer | `tested` | One stage, byte `79` (`y`), selecting `Yes, proceed` as `allow-once` |
| Deny answer | `tested` | One stage, byte `33` (digit `3`), selecting `No, and tell Codex what to do differently` as `deny-once` |
| Empty-composer observation | `tested` | The idle `› ` composer line with the terminal cursor immediately after the prefix, its placeholder text shown; a verified non-interaction boundary |
| Annotations | `tested` | `codex-auto-review-refusal` and `codex-auto-review-indicator`, described below |
| Prompt submission | `tested` | Composer plan `prompt` (§ Executable native plans) |
| Native-exec submission | `tested` | Composer plan `exec` (§ Executable native plans) |

The rules anchor on the live form's heading and closing lines rather than on absolute rows, and the reason line is model-authored text that the rule requires but excludes from the request identity. The deny answer uses the choice's digit because that choice displays only `esc` as its shortcut, and CAFleet never sends a lone Escape to a held form. Choice 2 approves a command prefix persistently. The native captures showed the decision on the native surface: after allow, Codex printed `✔ You approved codex to run <command> this time` and ran the command; after deny, it printed `✗ You canceled the request to run <command>` and `■ Conversation interrupted`, and ran nothing. Under an `approvals_reviewer` setting (§ Launch policy and capability registry), detection observes no request. The detection covers a single-line command; a command that wraps in a narrow pane is a recorded layout limit (§6.7 Role requirements), and ordinary programmatic input is not suppressed and may reach the form through delivery fallback.

Codex 0.163.0 in `approve-for-me` routes every escalation to its automatic reviewer and shows no native permission form, so the profile adds no request rule. Both Codex annotation rules (priority 40, alternate screen, cursor any) declare the same frame guard: require `^(?:› Ask Codex to do anything|  \? for shortcuts)` (the live composer or its hint line) and reject `^  k keep & next · esc dismiss & close` (the f2 warnings overlay).

- `codex-auto-review-refusal` spans from `^✗ Request denied for codex to run \S` to the next blank row `^\s*$` within 6 rows. Its region matcher `\A✗ Request denied for codex to run (?P<operation>[^\n]*\S(?:\n  [^\n]*\S)*)\s*\n\s*\z` captures the refused command, and its `row_margin` is two spaces. The line stays in the transcript at column 0, followed by `• Failed (exit 1) <command>`; model quotes render indented or `• `-prefixed, never at column 0, so they do not match. Codex wraps a long command inside a token with a two-space continuation margin, and the fingerprint strips that margin before removing the row break, so a resize keeps the fingerprint.
- `codex-auto-review-indicator` matches the one row `^  Permissions: +Workspace \(Approve for me\)\s*$` in the `/status` block. Its `mode` capture records `observed_mode` `Workspace (Approve for me)`.

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

Three breakpoints separate the layouts: `W_split` = 76, the widest split width; `W_compact` = 55, the widest compact width; and `W_min` = 36, the narrowest width that still renders the three choices on one row. The supported range is every width at or above `W_min`. The two-option layout (`Allow once` / `Reject`) follows the same rules. At every width at or above `W_min`, a form whose command is unclipped and that fits within `max_rows` matches exactly one of the two request rules, and at any width at most one matches, because their heading margins differ. Both have the same choices, answers, focus resolution, `native_request_id` and sensitivity.

Each request rule's `focus` matcher matches the choice row with or without its trailing control text, and its `deny` matcher accepts a choice row that ends at `Reject`. Its region matcher `controls` requires either a choice-and-control row to end the region, or a control-only row to end it with the choice row as the last non-blank row above it. A control-only row carries no label and takes no part in focus resolution.

The focus matcher's full capture is part of the request identity, and the layouts capture different text. A resize across `W_split` or `W_compact` therefore starts a new request generation, as a focus change does: an earlier request token stops validating, and a response in flight fails before its next byte, or ends `uncertain` once a byte was written.

Each operation matcher is region-scoped. It captures from after the `$ ` prefix on the first command row through each following row that continues the command, and its extraction removes the scrollbar glyph `█` beside the command box from every row (§ Operation extraction). A wide or split continuation row starts with the two-space continuation margin and then a non-space character, and the extraction removes that margin from every row after the first. A compact continuation row starts with a non-space character at column 0 and has no margin to remove. The capture is anchored at both ends: after the last captured row, only rows that are blank or hold only `█` may precede the choice row, and any other row fails the match. The `operation` evidence reports the displayed rows joined with `\n` and `operation_rows` equals the displayed row count; a one-row command extracts the same text with `operation_rows` 1. The semantic fingerprint keeps the row breaks, so a resize that moves a row break closes the episode and opens a new one.

A command taller than the form's command box scrolls inside the box: in the 57-column capture the box keeps its height, clips the command and shows a partial thumb `▀` on the first track row with the rest of the track blank. Each request rule therefore requires its region matcher `unclipped`, which holds only when the full-track thumb `█` ends every row from `# Shell command` through the last command row. A clipped command matches no request, so the Director never approves a command it cannot read in full.

| Rule | Element | Pattern |
|---|---|---|
| `opencode-shell-permission` | Region | Start `^  △ Permission required\s*$`, end `  ⇆ select  enter confirm  esc reject\s*$`, 24 rows |
| `opencode-shell-permission` | `heading`, `shell` (line) | `^  △ Permission required\s*$`, <code>^   # Shell command(?:\s&#124;$)</code> |
| `opencode-shell-permission` | `operation` (region) | `(?m)^  \$ (?P<operation>\S[^\n]*(?:\n  \S[^\n]*)*)(?:\n *(?:█ *)?)*\n   Allow once   `, with `row_margin` two spaces, `margin_rows` `continuation` and `row_trailer` `█` |
| `opencode-shell-permission` | `unclipped` (region) | `(?m)^   # Shell command +█ *\n(?: *█ *\n)*  \$ [^\n]*█ *(?:\n  \S[^\n]*█ *)*(?:\n *(?:█ *)?)*\n   Allow once   ` |
| `opencode-shell-permission` | `controls` (region) | <code>(?m)^   Allow once   (?:Always allow   )?Reject(?: +⇆ select  enter confirm  esc reject *&#124; *(?:\n *)*\n  ⇆ select  enter confirm  esc reject *)\z</code> |
| `opencode-shell-permission` | `focus`, `allow`, `deny` (line) | `^   Allow once   (?:Always allow   )?Reject(?: +⇆ select  enter confirm  esc reject)?\s*$` with `focus_style` `background`, `^   (?P<label>Allow once)   `, <code>(?:Allow once&#124;Always allow)   (?P&lt;label&gt;Reject)(?: &#124;$)</code> |
| `opencode-shell-permission-compact` | Region | Start `^△ Permission required\s*$`, end `pgup/pgdn scroll\s*$`, 24 rows |
| `opencode-shell-permission-compact` | `heading`, `shell` (line) | `^△ Permission required\s*$`, <code>^# Shell command(?:\s&#124;$)</code> |
| `opencode-shell-permission-compact` | `operation` (region) | `(?m)^\$ (?P<operation>\S[^\n]*(?:\n\S[^\n]*)*)(?:\n *(?:█ *)?)*\n Allow once   `, with `row_trailer` `█` and no `row_margin` |
| `opencode-shell-permission-compact` | `unclipped` (region) | `(?m)^# Shell command +█ *\n(?: *█ *\n)*\$ [^\n]*█ *(?:\n\S[^\n]*█ *)*(?:\n *(?:█ *)?)*\n Allow once   ` |
| `opencode-shell-permission-compact` | `controls` (region) | <code>(?m)^ Allow once   (?:Always allow   )?Reject(?: +pgup/pgdn scroll *&#124; *(?:\n *)*\npgup/pgdn scroll *)\z</code> |
| `opencode-shell-permission-compact` | `focus`, `allow`, `deny` (line) | `^ Allow once   (?:Always allow   )?Reject(?: +pgup/pgdn scroll)?\s*$` with `focus_style` `background`, `^ (?P<label>Allow once)   `, <code>(?:Allow once&#124;Always allow)   (?P&lt;label&gt;Reject)(?: &#124;$)</code> |

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
| `opencode-reject-reason` | Region | Start `^  △ Reject permission\s*$`, end `  enter confirm  esc cancel\s*$`, 24 rows |
| `opencode-reject-reason` | `heading` (line), `empty_reason` (region) | `^  △ Reject permission\s*$`; <code>(?m)^  △ Reject permission *(?:\n *)*\n  Tell OpenCode what to do differently *(?:\n *)*\n  Tell OpenCode what to do differently(?: {2,}enter confirm  esc cancel *&#124; *(?:\n *)*\n  enter confirm  esc cancel *)\z</code> |
| `opencode-reject-reason-compact` | Region | Start `^△ Reject permission\s*$`, end `^enter reject esc back\s*$`, 24 rows |
| `opencode-reject-reason-compact` | `heading` (line), `empty_reason` (region) | `^△ Reject permission\s*$`; `(?m)^△ Reject permission *(?:\n *)*\nTell OpenCode what to do differently *(?:\n *)*\nenter reject esc back *\z` |

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

The Hermes profile from `2026.9.24` applies to `hermes chat --cli` under either Hermes launch choice; its request rules and answers come from captures under the operator configuration's `approvals.mode: smart`. An approval prompt shown under `approvals.mode: manual` is detected only when its screen matches a request rule; otherwise it is an `unknown` frame answered manually:

| Capability | Status | Declaration |
|---|---|---|
| Detection | `tested` | Request rule `hermes-approval-escalate` for the guardian ESCALATE form (choices `1. Allow once`, `2. Allow for this session`, `3. Add to permanent allowlist`, `4. Deny`) and `hermes-approval-override` for the guardian-DENY owner-override form (`1. Allow once`, `2. Deny`), on the primary screen from the box's `╭` row to the `↑/↓ to select` footer. The operation is the box's command rows. `smart` reached no `execute_code` or action gate and no other native form in the captures. |
| Allow answer | `tested` | One stage `1` (`31`) on both forms, as `allow-once` |
| Deny answer | `tested` | One stage `4` (`34`) on the ESCALATE form and `2` (`32`) on the override form, as `deny-once` |
| Empty-composer observation | `tested` | `hermes-idle-composer`: the classic REPL row `❯ ` with the terminal cursor immediately after it; the rotating placeholder text follows the cursor |
| Prompt submission | `tested` | Composer plan `prompt`, submit `0d` |
| Native-exec submission | `tested` | Composer plan `exec`, submit `0d` |
| Annotations | `unknown` | None declared |

A digit selects and confirms its choice whatever the focus, so the answer stages have no focus gate. The focused choice carries the `❯ ` marker, and choices carry no background colour. Recorded layout limits: the ESCALATE form is detected at 37 columns and above, below which `3. Add to permanent allowlist` wraps; the override form is detected at 28 columns and above, below which the box border wraps. A command too long for the box adds `5. Show full command` and is answered manually.

The Director answers single-request choices only: allow once or deny, on both the ESCALATE and the override prompt. The session and always choices have no choice entry. When the guardian model is unavailable, Hermes shows the same human prompt, detected by the same rule. An unanswered prompt fails closed after `approvals.timeout` (default 300 seconds): Hermes prints its timeout notice and returns a BLOCKED result to the model, the form disappears, and the runner closes the episode with the `not_visible` close reason. A later bound response then fails request-token revalidation.

The pi profile from `1.5.0` applies to pi under every pi launch mode; only `cafleet-ask` shows an approval form, the CAFleet member gate's form (§6.7 pi member gate), and the argv pins `--tui-mode fullscreen`:

| Capability | Status | Declaration |
|---|---|---|
| Detection | `tested` | Request rule `pi-gate-approval`: the gate's form on the alternate screen, from the `CAFleet member gate: allow this bash command?` title row to pi's select hint row within 24 rows, with the `$ ` operation row, the focused choice and the `Allow once` and `Deny` choice rows. The operation is the single `$ ` row |
| Allow answer | `tested` | Answer rule `pi-gate-allow-once`: one stage, Enter (`0d`) gated on focus `allow-once`, as `allow-once` |
| Deny answer | `tested` | Answer rule `pi-gate-deny-once`: two stages, `j` (`6a`) gated on focus `allow-once`, then Enter (`0d`) gated on focus `deny-once`, as `deny-once` |
| Empty-composer observation | `tested` | `pi-idle-composer`: the idle editor between its two full-width `─` border rows, with the terminal cursor in the editor; a verified non-interaction boundary |
| Prompt submission | `tested` | Composer plan `prompt`, submit `0d` (§ Executable native plans) |
| Native-exec submission | `tested` | Composer plan `exec` for `! COMMAND`, submit `0d` (§ Executable native plans) |
| Annotations | `unknown` | None declared |

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

The operation is the single row after `$ `: the gate blocks a command holding a control character, a newline included, before the form opens (§6.7 pi member gate), so the form shows one command line. A command line too long for its row wraps onto a further row, which the operation matcher does not accept. The `operation` evidence reports that command line.

`pi-idle-composer` matches the empty editor row between the two rules with the cursor on it. A draft fills the editor row, a multi-line draft adds rows, and a running turn titles the upper rule `── ⠋ Working ──`, so none of them matches. The transcript draws a `! COMMAND` result between `─` rules too, but the cursor stays in the editor below it.

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

Each screen object has exactly these fields, of which only `cell_backgrounds` is optional:

| Field | Requirement |
|---|---|
| `rows`, `cols` | Positive measured geometry, equal to the catalog fixture entry |
| `alternate_screen`, `screen_seq`, `cursor_row`, `cursor_col` | Measured screen state; `cursor_row` is below `rows`, and `cursor_col` is at most `cols`, where `cursor_col` equal to `cols` records the terminal's pending-wrap position after the last column |
| `lines` | Exactly `rows` captured row texts, in viewport order |
| `masked_rows` | Unique zero-based row indices, each `{"row":N,"reason":"reviewed_privacy"}`; the corresponding `lines` entry is the empty string |
| `cell_backgrounds` | Optional runs `{"row":N,"start_col":C,"end_col":E,"bg":COLOUR}`, one per maximal run of cells in an unmasked row that share one non-default background, `end_col` exclusive; `COLOUR` is `{"kind":"indexed","index":I}` or `{"kind":"rgb","r":R,"g":G,"b":B}`. An absent key means every cell has the default background. |

The capture-to-fixture conversion inserts the converted screen under its catalog fixture `id` in `assets/profiles/native/<backend>.json`. It derives `cell_backgrounds` from the capture grid's per-cell `bg`, and fixture replay sets each run's background before writing its row text, so a fixture reproduces the focus styling a `focus_style` matcher reads and the separator backgrounds that sidebar clipping reads (§ Diff sidebar clipping).

Masking applies only to rows removed by the reviewed capture privacy policy, such as home paths and account, model or plan text. Every other row keeps its captured text exactly, including trailing spaces. The detection region of every rule citing the fixture lies entirely in unmasked rows.

These checks are the bundled catalog's build guard: a CAFleet build whose bundled backend files fail them does not pass its test suite. The guard reports these validation errors:

| Condition | Error |
|---|---|
| The source file is not embedded | `bundled fixture bytes are not embedded` |
| File `schema_version` differs from `1`, `provenance` differs from `native`, or `backend` differs from the entry | `bundled fixture file identity differs from its catalog entry` |
| The entry's `id` is absent from `screens` | `bundled fixture screen is missing` |
| A `screens` key has no catalog fixture entry | `bundled fixture file holds an uncited screen` |
| The canonical screen digest differs from the entry's `sha256` | `bundled fixture digest mismatch for <ID>: catalog <SHA>, computed <SHA>` |
| Screen `rows` or `cols` differ from the entry | `bundled fixture identity differs from its catalog entry` |

Each file and screen parses with exactly its fields above. Every row declared in `masked_rows` is in range and empty. Every `cell_backgrounds` run lies inside the geometry and outside masked rows, has `start_col` below `end_col`, overlaps no other run and carries a valid non-default colour. JSON formatting leaves the canonical screen digest unchanged.

Synthetic fixtures, which the build guard rejects, remain available to explicitly selected test catalogs.

A rule's cursor condition treats a pending-wrap cursor as standing on the last cell of its row, both in fixtures and on live screens. A region spanning the full width therefore counts a cursor at column `cols` on one of its rows as inside the region.

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

The operation evidence joins the processed rows with `\n`, and `operation_rows` is their count. The margin-then-trim order keeps a whitespace-only margined row, such as a blank line inside a multi-line Claude command, a match. An annotation rule reports its captured rows as displayed, continuation margins included, and uses `row_margin` only to normalize its fingerprint (§ Annotation detection).

##### Diff sidebar clipping

A rule's region may declare `panel_separator`, a single non-whitespace glyph. When any rule of the profile declares it, the detector derives one clipped line set per screen:

1. A row is a panel row when, at some column `C` with `40 <= C < cols - 1`, the cell holds the separator glyph, its background differs from the cell at `C - 1` and equals the cell at `C + 1`.
2. Every panel row of the screen must share one `C`; otherwise no row is clipped.
3. Each panel row keeps cells `0..C-1`; other rows are unchanged. A clipped row ends a logical line and is never joined to the next row.

A rule that declares `panel_separator` matches against the clipped set, and its region rows, matcher rows, label cells and cursor test use the same row indexes; a rule without it sees the unclipped rows. A boxed operation's `│ ` gutter shares its row's background, so it is never a separator. The separator column depends on the pane width, so it is found per frame. Catalog loading fails with `panel separator requires one non-whitespace glyph` for an empty, multi-glyph or whitespace `panel_separator`.

##### Executable native plans

Each operation's eligibility for the run's selected profile is independent (§ Profile selection by minimum version): empty-composer observation, executable prompt submission and executable native-shell submission each have their own coverage, `tested` exactly when the selected profile has a validated executable definition for that operation and otherwise `unknown`, with ordinary input for that operation using delivery fallback. Observing an empty composer supplies input-state evidence; executable submission additionally requires its measured literal/submit plan, so an observation definition without a submission plan keeps its observation coverage and yields no executable composer capability. Synthetic plans run only in explicitly selected test services, and a catalog selected explicitly by test services describes its fixture coverage.

`capabilities.answers` holds two entry forms, distinguished by the `decision` key:

| Form | Keys | Request rule class | Selected choice scope | Unique by |
|---|---|---|---|---|
| Decision answer | `decision,request_rule_id,answer_rule_id,choice_id,focus_matcher_id,stages,fixture_ids` | `permission_request` | `single_request` | `request_rule_id` and `decision` |
| Choice answer | `request_rule_id,answer_rule_id,choice_id,focus_matcher_id,stages,fixture_ids` | `question` | Any declared scope (`single_request`, `session`, `persistent`) | `request_rule_id` and `choice_id` |

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
- a `permission_continuation` rule declares no operation, choices or answers and cites a native fixture.

###### Focused-choice resolution

A focus matcher declares `focus_style` `background` or `marker`; a focus-gated plan whose focus matcher declares neither fails with `a focus-gated plan needs a focus_style matcher`. Under `marker`, the focused choice is the one choice whose label matcher matched the same row as the focus matcher; no such choice, or several, resolve to no choice.

A focus matcher with `focus_style: "background"` resolves the focused choice from cell styling, without depending on the theme:

1. Locate each choice label's cells through its line's cell boundaries.
2. Take the choice row background as the background that covers more than half of the choice line's cells. When no background does, no choice resolves.
3. Resolve the focused choice as the one choice whose label cells all carry a single background that differs from the row background, provided no other choice label carries a differing background.

The majority rule holds in every layout: the OpenCode compact layout has no left margin and its highlight covers the focused label plus one cell on each side, so with a focused first or last choice neither edge cell carries the row background.

A focus on an option without a choice entry, on several options or on none resolves to no choice, and a gate that does not hold is never written. The focus value compared in the observed request, the answer match and the write-time binding is the resolved focused choice id (or none) together with a capture that depends on the focus style. Under `background` the capture is the focus matcher's capture; the focus text alone does not change when a styling-only focus moves, so the resolved choice id makes a styling-only focus change start a new request generation. Under `marker` the capture is the choice-label row layout, the row each choice label matched, which stays constant while only the marker moves (the focus matcher's own text changes), so a between-stage focus gate compares an unchanged capture and only a change in the choice rows' layout changes it.

The composer object at `capabilities.composer` has required `rule_ids,fixture_ids` and `submission`; every bundled profile declares `submission`, and an explicitly selected test catalog may omit it to exercise missing-plan fallback. `submission` has exactly the required operation definitions `prompt,exec`, each an object with exactly `prefix_hex,submit_hex,fixture_ids`. Prompt has an empty prefix; exec has prefix `2120`, the UTF-8 bytes of `! `; `submit_hex` is the measured native submit key, never an assumed Enter mapping. Prompt and exec carry independent provenance. Every plan's fixture references follow the profile's interval provenance (§ Profile selection by minimum version) and establish the complete composer/work boundary for that operation. Unknown fields and incomplete or incompatible provenance fail validation.

| Plan property | Inclusive limit or required representation |
|---|---|
| Answer stages | 1–16 ordered stages. |
| Escape | No stage contains the byte `1b`, for every backend. |
| Answer bytes | 1–256 decoded bytes per stage; at most 4096 decoded bytes per plan. |
| Composer submit key | 1–32 decoded bytes. |
| Composer prefix | Zero bytes for prompt; exactly two bytes for exec as specified above. |
| Encoded native ordinary submission | At most 65536 bytes for prefix, unchanged literal UTF-8 and submit key together; the complete control JSON must independently satisfy the control-frame limit (§ Input arbitration and the original native answer). |
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

A positive short write, a failure after any child byte, or a timer-stage cancellation after a positive write produces `uncertain` with the input ID and last stage; an Escape byte counts as a side effect. The receipt records its `uncertain_cause` (§ Uncertain cause). A zero-byte transient write may retry only after fresh validation, and an uncertain operation is reconciled before any new attempt.

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

Store runtime artifacts in `.cafleet-runs/<database_namespace>/<run_id>/` beside the canonical selected database. The database identity is the canonical path, filesystem device/inode and the stored `runtime_database_identity` instance ID (§8); `database_namespace` is the first 26 base32 characters of its SHA-256 digest. A private namespace manifest stores the complete identity; a prefix collision with a different manifest fails explicitly rather than sharing artifacts.

Directories use `0700`, descriptor/spool files `0600`, and a control socket is accessible only to its owner. Create each run directory exclusively and reconcile an existing directory before reuse. Reject symlinks, ownership mismatches, malformed descriptors and socket paths exceeding the platform limit against the complete resulting path before pane creation. The descriptor contains schema version, complete database identity, fleet/member/Director/run identities, ownership epoch, a random launch nonce, literal child argv, absolute cwd, required environment overrides, evidence policy and initial terminal dimensions. The child inherits the native configuration; CAFleet neither copies the entire environment nor creates per-member native profiles.

Replacing a database file creates a different filesystem identity and namespace even when restored bytes retain the instance ID. Runners validate the selected path's file/instance identity before input and storage mutations; replacement fences the old run from the new database, holds input and initiates owned-child cleanup. Retain the old namespace for explicit reconciliation; a new creator never adopts its socket or outbox.

The descriptor and control socket are local coordination mechanisms, not an authorization boundary against the same operating-system user; matching identities and the nonce reject stale endpoints. The creator's guarded cleanup deletes the descriptor on every outcome (success, handshake failure, timeout, rollback and uncertain post-`start` timeout) and reports a failed unlink explicitly. Failed run records retain only the non-secret `permission_argv_json`, never the assignment-bearing prompt element or raw environment. After a creator crash, `member recover` removes the descriptor only after identity/ownership validation, and read-only `doctor` reports the leftover and its recovery action. The runner removes its loaded descriptor after preparation when safe, independently of the creator's idempotent cleanup.

During verified ordinary recovery, retain the original ownership lock and pin a leftover launch descriptor by its opened file identity and bounded content, validating the recorded namespace/directory, private regular file, immutable database/fleet/member/Director/run identity and nonce. A positive historical descriptor epoch at or below the captured current epoch remains valid across takeover and retry; native launch loading still requires current creator ownership and the exact current epoch. Expected absence is idempotent while the pathname remains absent.

Before unlink, revalidate the fixed current run/placement, original lock/directory, named file type/permissions/device/inode/content and original deadline; report complete cleanup only after removing the pinned descriptor. Preserve a replacement or symlink with an explicit identity error; an unlink failure is reported explicitly and leaves recovery incomplete and retryable. Diagnostics exclude assignment and environment content. Exceptional database-only reconciliation leaves surviving unverified descriptors untouched.

Prepare connection setup waits, before any request write and within the original handshake deadline shared by all retries, only for an absent endpoint, a same-user socket awaiting mode 0600 or a transient bound-but-not-listening socket. A wrong type, owner or symlink, including a dangling symlink, fails explicitly; current-run health keeps strict endpoint checks. A possible prepare write, malformed reply or lost reply returns its explicit error or uncertainty without reconnecting.

Creation follows this ordered protocol:

1. Validate assignment input/placeholders, backend policy eligibility, runtime paths and backend/multiplexer prerequisites. Preserve one assignment substitution and one literal prompt argv element.
2. Register the pending member and allocate its run row in one transaction. Write the private descriptor and create the pane with the absolute current CAFleet executable and the internal runner argv.
3. The runner validates the descriptor, acquires the precreated identity-pinned ownership lock, records its process identity, opens the PTY, initializes terminal state and event machinery, and binds its control endpoint. It reports `prepared` without spawning the native CLI.
4. The creator connects, validates the nonce/run identity, publishes the concrete pane placement with `status=prepared` and only then sends the control `start` frame. The runner revalidates placement, spawns the child and records `running` before reporting `started` with child PID/start identity.
5. Return creation success only after `started`, meaning child spawn succeeded without a native-policy/liveness guarantee; include the run identity and detection coverage with the member result. A child that immediately exits is reported by `runner_exited`, including its native status and Director notification. Use a ten-second monotonic handshake budget and preserve the rollback/cleanup diagnostics of any phase that fails.

Until `start`, the runner exits if its creator disconnects or the handshake expires. Failure after child spawn kills and reaps that owned child, closes the known pane, deregisters the pending member and retains a failed run record. A failed cleanup retains the original failure plus explicit orphan/pane diagnostics. A timeout after `start` is an uncertain launch: compensation inspects the existing run and stops its owned child, never retrying with a second child. Registration and pane ownership remain guarded until the handshake is complete.

#### Terminal transport and lifecycle

The runner relays terminal input and child output as bytes, with the outer terminal in raw mode under a restoration guard. Preserve interactive control sequences, UTF-8 fragments, bracketed paste and alternate-screen transitions. Size the PTY from the outer terminal, propagate resize to both PTY and parser, and preserve the native child's cwd, intended environment and exit status. Child bytes and runner diagnostics use separate logical channels, so diagnostics never become child input or detector evidence.

Forward outer-terminal input so a burst spanning several reads reaches the child together. One read of the raw-mode outer terminal returns at most the line discipline's 4096-byte buffer, and a longer terminal reply that reaches the child a tick late can be parsed as keys:

| Concern | Contract |
|---|---|
| Read | Each tick repeats the non-blocking outer read until it would block, reaches EOF or fills the 65536-byte input queue |
| Write | The tick then writes the whole queue to the PTY master in one write; unaccepted bytes stay queued, in order, for the next tick |
| EOF | Bytes read before EOF are forwarded before outer EOF is reported |
| Pacing | A tick that moved input bytes, by reading outer bytes or writing a positive count to the PTY master, is followed immediately by the next tick; every other tick, including one whose queued input the master did not accept, keeps the loop's idle sleep |
| Framing | None: forwarded bytes pass unchanged and in order, with no holding or timeout. Only bytes classified as manual input advance `human_generation` (§ Terminal reply classification), so a complete terminal reply is forwarded but leaves it unchanged |

The PTY-reader path updates terminal state before publishing the same bytes to display and passes immutable observations to the classifier. Track cursor/cell state, primary/alternate screens, OSC title and a monotonic output sequence. Keep scrollback out of live-prompt matching. Terminal queries and replies continue through the terminal transport, and parser callbacks never synthesize a second reply to a query the outer terminal already handled.

Terminal transport, classification and event storage are separate: database waits, Director receiver loss and event-spool retries never stop draining child output or accepting a human's native answer. Use bounded byte buffers and ordinary terminal backpressure for a slow outer display; a broken outer terminal triggers owned-child cleanup. Classification may coalesce intermediate working-screen snapshots, but it processes every potential interaction transition from the ordered parser stream before such coalescing. This is an observation contract for output actually rendered/read, not a guarantee to capture a dialog opened and closed between observable states.

| Condition | Required lifecycle behavior |
|---|---|
| Native child exits | Record its actual code/signal immediately, then apply the shutdown clock below and close the episode with `process_exited`. Restore the outer terminal independently of storage completion. |
| Runner receives termination or outer EOF/HUP | Apply the shutdown clock below and record the real termination cause. |
| Runner fails while child is live | Mark observation degraded and apply the shutdown clock below; return failure rather than a fabricated native success. |
| Runner is killed without cleanup | Closing the PTY may cause native hangup, but descendants may survive. Mark reachability suspect until exclusive recovery establishes loss; inspect verified PID/start identity before signaling. |
| Member/fleet deletion | Acquire the pane-input locks in the order §6.3 `member delete` and `fleet delete` specify, revalidate placement/run, request runner shutdown and finish pane/registry cleanup as those commands specify. Report any unconfirmed child cleanup explicitly. |
| Restart or replacement | Create a fresh member/run after explicit reconciliation. Keep earlier episodes/events; a process generation never inherits an earlier pending request or causes automatic command re-execution. |

One shutdown clock starts at normal native-child exit, runner failure or termination request, and every wait shares its absolute deadlines:

| Boundary | Action |
|---|---|
| Start | Stop programmatic input and send TERM to verified surviving descendants in the owned process group. During the first two seconds, drain available PTY bytes. |
| 2 s | KILL verified survivors and reap, and close the master even if a descendant retains the slave. |
| 3 s | Nonblocking outer-display flushing ends; discard only undelivered display-tail bytes, recording their count and a truncation diagnostic. |
| 4 s | Event-worker flush/SQLite attempts end, including transaction completion and retry delays, with no additional grace budget. Retain complete uncommitted outbox frames for explicit recovery and record any unsaved in-memory gap. |

Run every runner SQLite attempt under a bounded database budget from the remaining absolute work/shutdown deadline: it replaces the connection's default five-second busy wait, cancels long-running statements within that deadline and restores the prior handler and connection hooks after each attempt.

Restore terminal settings and release ownership after stopping/joining the writer, regardless of EOF, display or database completion. Retain actual child code/signal separately from `cleanup_state=complete|incomplete` and bounded diagnostic reasons for truncated output, pending events or unconfirmed descendants. When cleanup/restoration is complete, return the native status (`128 + signal` for a signal exit). With incomplete cleanup, preserve a native nonzero return; use runner exit `1` if the native status was zero or unavailable, and report its distinct recorded native status. Hard-kill or restoration failure remains an explicit host-recovery condition.

Maintain liveness counters only in process and expose them through `health`; persist `last_health_at` on lifecycle transitions and explicit input/recovery checks, not as a per-second heartbeat. An old recorded timestamp is historical evidence, not a staleness threshold. `member show` remains a registry read, returns the recorded fields and requires neither a socket nor a multiplexer. `doctor` reports registry metadata, pending content reconciliation and the results of nonblocking identity-pinned lock and bounded socket probes. Outbox content inspection belongs to the current lock/epoch owner; diagnosis leaves registry records, events and reserved sequences unchanged. Input dispatch requires a fresh matching health/ownership check; its normal bounded transaction can record `reachability=suspect` on failure or `reachable` on success. A failed probe holds input but closes no interaction, changes no terminal status and allocates no event sequence.

The creator precreates `owner.lock`, records its device/inode and runtime-directory identity, and records the current owner PID/start identity. Every ownership transfer advances `owner_epoch` with a CAS, including creator-to-runner transfer after the runner verifies/acquires that inode; the runner's `prepared` reply reports its new epoch for subsequent control frames. The runner holds the ownership lock through every writer/input thread and bounded shutdown. Validate the open descriptor's identity against both registry and current pathname; reopening a replacement lock is not acquisition of the original lock. A deleted directory/lock or changed inode requires the distinct missing-path recovery branch below.

Normal recovery acquires the identity-matching lock before a CAS epoch takeover. Every event mutation validates complete database identity, owner kind and epoch; input additionally validates its ownership/lock identities immediately before child writes. A delayed live runner retaining the original lock prevents normal takeover. The missing-path branch also requires verified original-owner termination before fencing; missing files alone establish no exclusive ownership or safe input. Every takeover records the recovering process identity and advances the epoch, rejecting stale writes/input.

Only the current lock/epoch owner allocates event sequences. Confirmed loss and its events are produced by the explicit recovery procedure below after exclusive takeover, ordered after complete outbox replay. Concurrent reconcilers contend for the same lock and CAS rather than allocating competing loss events. PID/start identity assists child reconciliation; it never substitutes for ownership.

##### Terminal reply classification

The runner classifies every positive outer-terminal read for manual-input accounting only: classification holds, reorders, drops and synthesizes no byte and retains only parse state and a pending byte count. `human_generation` is the cumulative count of bytes resolved as manual input. The staged settle check, drain proofs, permission-response authority and `member press` accounting all use it, so a terminal reply affects none of them. Tick pacing still counts every byte read.

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

The paste state follows a manual `CSI 200 ~`: every byte counts as manual, and `CSI 201 ~` counts as manual and returns to ground. A sequence split across reads stays pending, within one tick or across ticks.

A pending prefix resolves as manual input once 100 milliseconds pass since its last byte:

- The runner resolves expired prefixes at the start of each tick's outer read and of every drain round, before reading, and advances `human_generation` by the resolved manual count.
- While a prefix is pending, the transport is not quiescent: a drain returns `Pending` with blocker `terminal reply pending`, an earlier drain proof fails validation with `input transport is no longer quiescent`, and no programmatic stage, permission answer or deferred write proceeds.
- Outer EOF resolves a pending prefix as manual before EOF is reported.
- A lone Escape key is pending for at most 100 milliseconds and then counts as manual, so a staged delivery whose settle sees that key stops with `manual_input` once the bound resolves it.
- The bound runs inside the deadline of the write attempt it delays (the runner's five-second ordinary-input attempt budget for a deferred write, or the invoking command's five-second deadline for a prompt, exec or permission answer), adding no deadline of its own.
- Each new byte of a prefix restarts the bound; the 64-byte CSI and 4096-byte string body limits cap how long a continuously fed prefix stays pending.

Human input whose bytes exactly equal an allowlisted reply and arrive within the bound counts as a reply. The realistic member of this accepted collision class is a modified F3 key (`CSI 1 ; Pmod R`), matching the cursor-position-report form; the others need an exact multi-key sequence typed within 100 milliseconds: Alt-`[` then `I`/`O` (a focus report), Alt-`]` … Ctrl-G (an OSC reply), and Alt-Shift-P or Alt-`_` … Alt-`\` (a DCS or APC reply). A colliding key still reaches the child unchanged; it only fails to interrupt a staged delivery or revoke automatic authority.

#### Classification and episode identity

Each detector consumes a reconstructed screen, title, cursor/input-region evidence, output sequence and known backend version, and returns a typed observation with its rule identifier, matched region, extracted visible operation/choices, a normalized semantic fingerprint and a confidence/evidence designation. Rules compile at startup, and an invalid rule definition fails. Versioned profiles are bundled locally in CAFleet, never loaded from remote rules or herdr manifests.

| Classification | Evidence needed | Meaning |
|---|---|---|
| `permission_request` | A backend-specific live permission heading/operation and its choice/input region | An observed request awaiting a native decision; title text alone is insufficient. |
| `permission_denied` | A tested permission-specific returned-result region | A displayed refusal, not an arbitrary nonzero exit and not proof the user refused. |
| `permission_continuation` | A tested screen the native agent shows for a pending request after one of its choices is selected, such as a rejection-reason dialog | A continuation of the active permission episode; it never opens an episode (§ Continuation screens). |
| `question` | A tested general input/question form | Human input is required without establishing a tool-permission request. |
| `trust` / `update` | A tested startup trust or update form | A distinct native interaction, preserving its own consequences. |
| `unknown_interaction` | A live interactive form without enough evidence for a more precise type | Queue ordinary programmatic input with zero bytes and request inspection. |
| `working` / `composer_empty` / `unknown` | The corresponding tested activity/composer rule, or lack of one | Input eligibility is evaluated separately; absence of permission text is not an empty composer. |

The persisted episode classes are exactly the §8 `member_interactions.classification` values, with `trust` and `update` distinct; every other classification creates no interaction row. `permission_denied` is a returned-result observation recorded only through `permission_denied_observed`, with a nullable episode link. Each frame has exactly one of these primary observations; transcript results and notices of automatic review are annotations evaluated beside it (§ Annotation detection).

A title such as `Action Required`, a quoted approval transcript, a transcript viewer and an old permission line cannot independently create `permission_request`. Profiles must bind controls to the same live region, distinguish general questions/startup forms, and remain explicit about wrapping, layout changes and competing rules. Conflicting strongest classifications yield `unknown_interaction` and preserve the screen. A run with no selected profile, or whose profile declares no detection, advertises unknown coverage; missing matches never imply that all requests were observed.

An episode is a CAFleet observation identity, `(run_id, episode_seq)`, not a native request ID. Allocate `episode_seq` monotonically within a run when a stable live interaction first appears. A candidate is *stable* after two compatible observations at least 100 ms apart, rechecked after 100 ms even if no bytes arrive; an episode opens only for a stable candidate. Gate programmatic input immediately on the first candidate, before this notification debounce.

The fingerprint contains backend, interaction class, visible operation and stable choice labels. Exclude spinner/counter/cursor decoration. The visible operation includes its displayed row breaks, so a resize that rewraps the operation changes the fingerprint and replaces the episode. Redraws and resize matching the fingerprint refresh the stored latest interaction evidence without another opening notification, but only when the post-policy evidence differs in a field other than `screen_seq` and the `evidence_digest` derived from it; a redraw that changes only the output sequence schedules no refresh and leaves the stored evidence and any issued request token unchanged. A different operation/choice/class candidate holds input immediately and replaces the active episode only once stable; the old episode stays open until then. An incompatible candidate restarts stability, while return to the original fingerprint cancels the pending replacement. After a verified native noninteraction boundary, an identical request gets a new episode. If two native requests are visually indistinguishable with no observable boundary, preserve one episode and report that identity limitation; never claim one-to-one native request coverage.

| Episode transition | Event and interpretation |
|---|---|
| Stable candidate appears | `interaction_opened`; record its classification and observed evidence. |
| Partial redraw or temporary overlay | Keep the episode open, gate input and update local visibility. A two-second uncertain interval emits one `interaction_obscured` event. |
| Verified noninteraction boundary stable for 250 ms | `interaction_closed` with `not_visible`; record the observation rather than inferring a decision. |
| Different interaction replaces the active form | Once the replacement is stable, close with `superseded` and open the next episode in order. |
| Child exits or exclusive recovery confirms observer loss | Close with `process_exited` or `monitor_lost`; a suspect/unreachable observer leaves the episode open and approval outcome remains unknown. |
| Returned permission refusal appears | Emit `permission_denied_observed` once per stable result. Link an episode only through native correlation evidence. |
| Continuation screen of the active permission episode | Keep the episode open and `Visible` with ordinary input queued for the interaction. Emit no event, start no obscured timer and keep the stored request evidence. |

A returned refusal resets its deduplication identity only after a profile-verified noninteraction boundary lasting 250 ms. Temporary uncertainty resets pending stability while retaining the previous deduplication identity; a generic working screen establishes a boundary only when its profile verifies the result region's disappearance.

Equivalent redraw evidence uses one bounded coalesced worker slot, with a bounded local snapshot during an in-flight attempt. Apply the evidence policy before queuing, bind the refresh to the current owner/run/open episode and output sequence, and coalesce only newer evidence for that same active episode. Persist it in an ownership-checked transaction under an absolute deadline; equal sequence with identical evidence is idempotent, while regressing, conflicting, closed-episode or stale-episode updates fail explicitly.

Order each refresh after its opening and previously accepted semantic transitions and before later ones. A transient event-sequence barrier prevents delayed evidence from overwriting a later closure or visibility transition; preserve a newer coalesced revision when an earlier attempt completes. Refresh only the interaction's latest evidence/digest, preserving original opening event/message bytes and allocating no event, episode, sequence or inbox notification. Failed refreshes retain bounded pending evidence and expose degraded/gap state while manual terminal I/O continues; recovery preserves the last committed evidence and reports the gap rather than reconstructing lost redraws.

A closing transition keeps the stored request evidence; the closing frame's post-policy evidence appears only in the `interaction_closed` event payload.

Keep decision outcome (`approved`, `denied`, `cancelled`) and operation outcome separate from episode visibility. Screen disappearance, Enter, Escape, a timeout, a message ACK and resumed animation establish none of those outcomes by themselves. Machine decision outcome is `unknown` unless explicitly documented native evidence establishes it; human/Director reports remain attributed reports, not authenticated native events.

##### Continuation screens

A `permission_continuation` rule declares `continues_rule_id`, the permission rule it continues, and `continues_choice_id`, the choice whose selection opens it. Showing no operation, it cannot identify a request by itself; episode continuity and succession bind it to one:

| Concern | Contract |
|---|---|
| Episode | While a permission episode of its `continues_rule_id` is active, a continuation observation keeps that episode open. Without such an active episode it is an `unknown` screen. |
| Request binding | Inside a response, a continuation qualifies for the next stage only as the first fresh quiescent observation after the complete write of the stage gated on its `continues_choice_id`, with the episode, `human_generation` and runner ownership unchanged and no observation gap or degradation in between. The runner then rebinds the in-flight selection to the continuation's rule id and match fingerprint within the same episode. |
| Inspection | `permission inspect` on a continuation offers no request token or answer capability, so a continuation reached by human input is completed manually. |

##### Annotation detection

An annotation is a transcript result or notice, not a live form. The detector evaluates annotation rules on every frame alongside the primary observation and keeps them out of its priority and conflict resolution: it returns the primary observation exactly as without annotations, plus the list of annotation blocks, each with its rule id, matched region, post-policy evidence and in-memory fingerprint. Annotations create no `member_interactions` row and leave input eligibility, episode visibility and the primary observation unchanged. They are evaluated only while semantic classification runs, so a degraded runner's observation gap covers them.

| Annotation kind | Meaning | Effect |
|---|---|---|
| `automatic_refusal` | The automatic reviewer refused an operation and returned the refusal to the model | `automatic_refusal_observed` (§ Automatic refusal event) |
| `automatic_review_unavailable` | Automatic review is unavailable or degraded, for example unavailable for the account or model or fallen back to prompting | `automatic_review_unavailable` (§ Unavailable automatic review event) and `automatic_review=unavailable` in the effective policy record |
| `automatic_review_available` | A positive automatic-mode indicator, such as a mode badge | `automatic_review=available` in the effective policy record; no event |

Observation kinds parse and render each annotation kind by those exact strings. Profiles declare annotation rules in the required capability group `annotations`, either `null` or `{ "rule_ids": [...], "fixture_ids": [...] }`, under the provenance rules of every other capability group (§ Profile selection by minimum version): nonempty rule ids and known fixtures. Every bundled profile declares the key. The catalog compiles only when:

- The `annotations` group accepts only annotation kinds, and the `detection`, `composer` and continuation groups reject them; an annotation rule outside the `annotations` group is invalid.
- An annotation rule has empty `extract.choices`, null `extract.native_request_id` and `verified_noninteraction` `false`. `automatic_refusal` requires an `operation` capture, and the other two kinds require a null one.
- An annotation rule declares `frame`, `{ "require_patterns": [...], "reject_patterns": [...] }`, with at least one compilable require pattern and zero or more compilable reject patterns. An interaction rule (`permission_request`, `question`, `trust`, `update`, `unknown_interaction`) may declare `frame` with zero or more require patterns and at least one compilable reject pattern; one without a reject pattern fails with `interaction frame requires a reject pattern`. Other rules reject the field. A frame failing an interaction rule's guard yields no observation from that rule.
- Each automatic-mode fixture produces exactly its expected primary observation and its expected annotation set, and no annotation fixture produces a `permission_denied` primary observation.

Annotation rules use the region contract of the other rules, with these differences:

| Concern | Contract |
|---|---|
| Screen | Each rule declares the screen its captures show; scrollback is excluded, as for every rule. |
| Blocks | The start and end patterns delimit one refusal or notice block, and the matchers require the native result decoration, so model text quoting the same words does not match. Every qualifying region yields a block, in row order and non-overlapping, so two refusals visible together are two blocks; other rules still require exactly one qualifying region. |
| Live-transcript frame guard | Each `frame.require_patterns` entry must match a live-screen line of the same frame and no `frame.reject_patterns` entry may match one. Require patterns name lines present only in the live TUI, such as its composer or footer line; reject patterns name the transcript viewer's header, and a frame failing the guard yields no blocks for that rule. |
| Fingerprint | Backend, rule id and the displayed operation or notice text, normalized by first stripping the operation extract's `row_margin` from each continuation row, then removing every row break and collapsing each whitespace run to one space, so a resize that rewraps a block, even inside a token, keeps its fingerprint. The event's `operation` evidence still reports the displayed rows, continuation margins included. |

The annotation tracker keeps a set of fingerprint states per annotation rule:

1. A fingerprint present in a frame's blocks is emitted once stable (§ Classification and episode identity); several fingerprints are tracked and emitted independently.
2. An emitted fingerprint is not emitted again until its state resets.
3. A state resets only after the fingerprint has been absent for 250 ms of continuous frames whose primary observation is a verified noninteraction and whose frame guard holds. A frame showing a permission form, another interaction, an overlay, an unknown screen, a guard failure or a redraw does not advance the reset, so a block hidden by any of them keeps its state and emits nothing when it reappears. A working screen advances the reset only when the profile classifies it as a verified noninteraction; Claude Code 2.1.302 and Codex 0.163.0 keep their idle composer visible while working (Codex also while its reviewer runs), so their working frames do.
4. `automatic_review_unavailable` is emitted at most once per run per rule id unless an `automatic_review_available` annotation was observed after it.
5. `automatic_review_available` emits no event; it only updates the effective policy record.

The `permission_denied` reset uses the verified noninteraction boundary alone because a primary refusal frame and an idle-composer frame exclude each other. An annotation stays visible above the idle composer, so its reset also requires the annotation's absence. An automatic refusal is a separate kind from `permission_denied_observed` because the automatic reviewer decided it and the Director procedure differs; as a primary rule and an annotation, the two never match the same block.

The event is an alert, not a one-to-one count of native refusals. These limits apply:

| Backend | Limit |
|---|---|
| Every backend | A second refusal of the same normalized operation emits nothing while the first is visible or before an idle frame shows the first gone. A refusal rendered and removed within 100 ms is not observed. |
| Claude Code 2.1.302 | In the default view the notice names no command, so refusals of the same tool and category collapse into one event, and a notice that expires while a form hides it is never observed. The default-view notice was captured at 120 columns only; a narrower pane can truncate its ` · /permissions` suffix, and such a notice is not observed. |
| Claude Code 2.1.302 | The native-quote guard rests on the U+00A0 that the native result prefix carries. |
| Claude Code 2.1.302 | `claude-auto-refusal` strips no row margin, so a command token wider than the pane, wrapped inside the token, changes its fingerprint after a resize and can emit once more. |
| Claude Code 2.1.302 | Only the captured unavailable notice `auto mode disabled by settings` is observed. The binary also carries `auto mode is unavailable for your plan`, `auto mode unavailable while fast mode is on`, `auto mode unavailable for this model` and `auto mode is unavailable right now`, which no capture shows and no rule claims. |
| Codex 0.163.0 | Commands longer than about 70 characters display truncated, so refusals sharing the displayed prefix share a fingerprint. The reviewer's reason appears only in the f2 warnings overlay and the ctrl+t view. |
| Codex 0.163.0 | After three consecutive automatic denials Codex interrupts the turn (`■ Conversation interrupted`); the reason, `Automatic approval review rejected too many approval requests for this turn …`, appears only in the f2 warnings overlay. The Director receives one `automatic_refusal_observed` per denial, and the member stops without reporting. |
| Codex 0.163.0 | No unavailable-review screen appears on the live screen, so no rule claims `automatic_review_unavailable`, and the positive indicator appears only on demand in the `/status` block. |

The member's routed refusal evidence remains the record in each case.

#### Persistence, notification and read interfaces

The baseline creates the runtime records with the broker tables (§8). The running CLI never opens a file of another cafleet version for writes, migrates, renames or deletes it, and shows none of its fleets, messages or runtime history.

The §8 DDL fixes the runtime tables' exact columns, types, enums, nullability, foreign keys, uniqueness and partial unique indexes, and typed conversion enforces the same enums. Terminal `member_runs.status` values are `exited`, `failed` and `lost`. Nullable fields represent genuinely unavailable evidence or phases not reached. A missing required identity, invalid enum, malformed payload or cross-run interaction reference fails the write/read explicitly. Member history is soft-deregistered; runs, interactions, runtime events, input operations and permission responses are retained for the lifetime of the database file, and a permission response claim is removed only by claim release (§ Permission response storage). Timestamps use UTC RFC 3339, and ordering uses the run/event sequences rather than wall-clock uniqueness.

| Record | Semantics beyond the DDL |
|---|---|
| `member_runs` | `owner_epoch` starts at 1, and `event_seq_reserved_through` and `observation_gap_count` start at zero. `runtime_path` is canonical, each start identity is a verified platform-specific process-start token, and `recovery_origin_status` preserves the original lifecycle status while recovering. |
| `member_interactions` | `evidence_digest` is post-policy, and `evidence_json` is the latest bounded evidence. |
| `member_runtime_events` | `payload_json` is versioned, and unique `(run_id, event_seq)` supports idempotent retries. |
| `member_input_operations` | `ready` certifies a zero-byte attempt, and a ready row with deferred metadata is a runner-queued ordinary input; `uncertain_cause` is a value from § Uncertain cause. Settled rows are retained as replay-rejection records for the lifetime of the database file. |
| `member_permission_responses`, `member_permission_response_claims` | Immutable audit provenance, and a claim for the one possibly transmitted answer per episode (§ Permission response storage). |

Event payloads carry `schema_version=2`; readers accept only version 2 and return existing records and their associated message bytes unchanged on replay. `cleanup_state` begins `incomplete` and becomes complete only after successful shutdown cleanup, so a live/crashed run makes no prior completion claim.

`coverage` is `tested` when the profile selected for the run's backend and native version declares detection, and `unknown` otherwise (§6.9 Profile selection by minimum version); it describes the selected profile's declared detection, not universal prompt detection. `launch_nonce` is a random 32-byte value encoded as hex and stored with the run so later same-user control clients can present it; it prevents accidental reuse of stale endpoints and is not a privileged authorization credential. Validate member/fleet/Director ownership against broker records within each mutation transaction; inbox inserts additionally require their active/live status, while archived recovery validates historical ownership.

Runtime event kinds are the §8 `member_runtime_events.kind` values. The event payload has this versioned shape:

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
    "instruction": "INSTRUCTION",
    "inspect_argv": ["cafleet", "permission", "inspect", "108", "--event-id", "481", "--from-member-id", "91", "--json"]
  }
}
```

`INSTRUCTION` is the permission-request instruction (§ Request-bound permission responses). Event keys are ordered `schema_version,event_id,run_id,event_seq,member_id,fleet_id,director_member_id,kind,interaction_id,classification,observed_at,source,native_request_id,decision_outcome,evidence`, followed by `close_reason` for closures or `cause` for lifecycle events, then, for `input_delivery_result` only, ordered `input_id,message_id,delivery,stage`, then required `director_action`. The action object has ordered `kind,instruction,inspect_argv`, and its `kind` is `review_permission_request`, `review_question`, `review_automatic_refusal` or `review_automatic_review_unavailable`; other events use null action. Both automatic review event kinds fit this shape: `evidence.backend` together with the kind identifies the reviewer, and the effective policy lives on the run rather than in the payload.

The evidence object, in events and in stored interaction evidence, has ordered keys `backend,native_version,rule_id,screen_seq,truncated,evidence_policy,operation,operation_rows,choices,native_request_id,excerpt,evidence_digest`. `operation_rows` is the positive number of displayed rows the operation was read from; it is null exactly when `operation` is null, including under metadata evidence.

Run-level events use null interaction/classification, `source=runner_lifecycle` and a typed health/exit cause in the payload. Missing command, cwd, environment or native request identity stays null, never filled with invented execution data from screen parsing. Limit a UTF-8 excerpt to 8 KiB and the complete payload to 16 KiB, record truncation, escape control characters and omit raw terminal recording. Treat screen text as untrusted evidence, never as a command or instruction to approve.

Launch-only `--event-evidence <metadata|redacted>` defaults to `metadata`. Metadata mode persists only identities, classification, rule and native version, sequence, timestamps and health/closure causes; every screen-derived operation, choice, excerpt and native identifier is null. Semantic fingerprints for live episode comparison stay in memory, never in logs, outbox or database. `evidence_digest` is SHA-256 of the canonical post-policy evidence object, so metadata mode exposes no hash of hidden screen text.

Redacted mode requires `--event-redaction-file <ABSOLUTE_PATH>` naming readable UTF-8 JSON `{ "schema_version": 1, "patterns": ["regex"], "sensitive_rules": ["rule-id"] }`. Require a nonempty pattern array, known rule IDs and compilable expressions. Missing/malformed configuration, an unsupported policy or a redaction file supplied with metadata mode fails before registration with usage exit `2`, never falling back to raw evidence. Compile the file once at launch and keep the compiled policy in memory; events store the policy name, not raw patterns or the file contents.

In redacted mode, apply the same configured patterns to every normalized screen-derived field before any persistence, including operations, choices, native IDs and latest interaction evidence. Replace matching fields in full with `[redacted]`; a profile-marked sensitive form or a rule named by `sensitive_rules` uses metadata-only evidence for the entire form. Apply UTF-8 bounds and escaping after policy transformation, then compute `evidence_digest`; persist no pre-policy screen hash or semantic fingerprint. Unmatched text is allowed only by this explicit operator choice, which does not promise automatic secret discovery.

Persist a transition, its event and a normal `input_required` inbox message to the pinned Director in one immediate transaction. The message's text is the full event JSON and its sender is the observed member; the typed `source` distinguishes runtime observation from model-authored speech. The transaction applies the broker's fleet and active-member checks (§6.2). Insert the message and event, fill the final ID-bearing payload and validate the complete association before commit; no intermediate message body is externally visible. A retry of the same `(run_id, event_seq)` returns the existing event/message IDs and verifies identical payload identity; conflicting reuse fails.

Before persistence, the outbox stores an observed transition keyed by `(run_id, event_seq)` and nullable `episode_seq`, with kind, original observation time and immutable evidence. The transaction allocates or resolves interaction/event/message IDs and produces the final public payload. Idempotence compares these original transition fields, excluding generated IDs and commit timestamps.

The runner uses the broker library directly and never depends on its child invoking `cafleet message send`. A persisted notification remains pending through Director absence or listener restart. Live events use `notification_state=inbox`; active ownership and the sender/recipient fleet checks remain mandatory. After closure, ordinary observation stops. Exclusive recovery may replay historical frames as `notification_state=archived`, `message_id=null`, with `archive_reason=member_inactive|fleet_deleted|director_inactive`, retaining the event/interaction history without inserting a message from an inactive sender or claiming Director notification.

`event show` returns the complete durable event with its notification/archive state and current interaction/run state, and `input show` returns a durable programmatic-input receipt (§ CLI presentation contract). Runtime events use the ordinary broker message envelope and ACK semantics.

##### Automatic review events

Both events follow the runtime-event contract above: an atomic event plus an `input_required` Director inbox message, the outbox, idempotent replay, archived recovery and the evidence policy. Neither creates an interaction row or changes input eligibility. Each payload has null `interaction_id`, `classification` and `native_request_id`, `source` `runner_screen`, `decision_outcome` `"unknown"` and `director_action.inspect_argv` `["cafleet","event","show","<EVENT_ID>","--json"]`, built from the stored event ID. `evidence` is the ordered evidence object, and its `excerpt` follows the run's evidence policy:

| Field | `automatic_refusal_observed` | `automatic_review_unavailable` |
|---|---|---|
| `director_action.kind` | `review_automatic_refusal` | `review_automatic_review_unavailable` |
| `evidence.rule_id` | The refusal rule | The notice rule |
| `evidence.operation`, `evidence.operation_rows` | The run's evidence policy (null under `metadata`) | `null` |

##### Automatic refusal event

The `director_action.instruction` is: "Verify this event. The member's automatic reviewer refused the operation and no native request is waiting. Dispatch evidence comes only from the member's routed request with its exact command, cwd, environment and refusal; this event and screen captures corroborate it. With no routed request, dispatch nothing and ask the member whether the refused operation is still needed when it next reports or goes idle. For a routed request, decide in the Director session's effective auto mode whether to authorize a separate execution through the routed-command path or to refuse; otherwise obtain the user's permission first. Inspect the actual result and the member's continuation afterward." When the evidence has `operation_rows` greater than 1, the instruction appends the same displayed-rows sentence as the permission-request instruction (§ Request-bound permission responses).

The event has no linked interaction, so `permission inspect` and `permission respond` naming it fail their event/interaction association check before any byte.

##### Unavailable automatic review event

The `director_action.instruction` is: "Verify this event. Automatic review is unavailable for this member, so native permission forms may appear and each needs a decision, and refusals may stop its work. Supervise the member as interactive, and tell the user when the selected automatic mode cannot apply." The member keeps running under the mode the native host actually applies; CAFleet never restarts it in another mode.

##### Input delivery result event

The runner emits one `input_delivery_result` event to the run's Director for a deferred input's first submitted, not-submitted or uncertain result. It follows the runtime-event contract above, creates no interaction row and carries no literal payload.

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

Only a change of `automatic_review` or `observed_mode` writes the record. Readers validate the shape and fail on a malformed record. A null record means no tested observation has been made; it is never filled from argv or the selected mode.

| Write | Transaction | Failure behavior |
|---|---|---|
| With `automatic_review_unavailable` | Part of the event's outbox frame and its immediate event transaction, so live commit and recovery replay apply it idempotently with the event | The event-transaction behavior |
| `automatic_review_available` update | Submitted to the event worker in observation order and committed through the ownership-checked transaction (owner epoch, database identity and historical ownership), additionally requiring `member_runs.status` `running` or `degraded` | A stale owner fails with `stale ownership token` and any other status fails as a `member_runs.status` invariant violation; the worker records a local diagnostic, drops that write without retry and continues |

The available update is not spooled: a runner lost before it commits loses only that observation, and the next indicator observation records it again.

#### CLI presentation contract

§6.3 CLI *Runtime presentation*, *Runtime event and input commands* and *`member recover`* fix the exact keys, order, nullability and text layouts of the runtime outputs of member create/show, `event show`, `input show`/`reconcile`, `member recover` and wrapped prompt/exec. The registry-only runtime fields let the Director bind a recovery attestation to the inspected owner snapshot without probing a live socket, and printed submission or settlement establishes no native execution result.

#### Storage pressure and durability boundary

An event worker owns SQLite and a private append-only per-run outbox. The parser/control loop submits immutable transitions through a bounded channel of 64 records. The worker fsyncs length-delimited, checksummed, redacted records before attempting SQLite. Retain committed frames while any complete frame remains uncommitted; truncate and fsync the file only after independently validating each complete frame's committed event/message association. Under exclusive ownership, diagnose and remove an incomplete final tail at the last valid boundary before further append; preserve complete corrupt bytes and fail explicitly. Replay valid complete records with their original run/sequence, and compact once the remaining frames commit so pressure can clear.

The current owner reserves event-sequence ranges of 256 by advancing `event_seq_reserved_through` in a bounded ownership-checked transaction before using them. The classifier consumes the reserved range in memory; if the next range is unavailable it enters the defined observation-gap state while terminal I/O continues. Unused reserved values are valid gaps. Recovery allocates new sequences above this durable high-water mark and all committed/complete-spool sequences, so missing artifacts cannot cause reuse of a previously issued sequence.

The Director-owned explicit `member recover` command (§6.3 `member recover`) operates on the recorded run of an active or deregistered member, never starts an agent, and returns recovery counts, final lifecycle/notification state and cleanup diagnostics. A five-second invocation deadline bounds each recovery pass. Start one cleanup clock at entry to the common recovery coordinator, before snapshot, locking, process/health evidence and replay; its two/three/four-second boundaries share that origin and are clipped to the supplied original deadline, including a deletion caller's earlier deadline. Incomplete replay retains the spool and `recovering` state for the next pass. `doctor` diagnoses recovery needs without mutating them. Member/fleet deletion invokes the same recovery component after successful runner shutdown and before deregistration when its remaining deadline permits; otherwise retain the namespace for a later explicit pass.

Every recovery counter counts, once after commit, durable work newly committed by this invocation; a duplicate lookup, skipped frame, reserved sequence or failed/rolled-back transaction contributes zero.

| Counter | Commit domain |
|---|---|
| `replayed_count` | Distinct original `(run_id,event_seq)` transitions from this invocation's input spool whose event transaction first commits in this invocation; already committed originals and identical duplicate frames add nothing. |
| `inbox_count` | Runtime event/message associations, from replayed originals or newly synthesized recovery transitions. |
| `archived_count` | Runtime events with archived disposition, from replayed originals or newly synthesized recovery transitions. |

An incomplete report retains its already committed work; a later invocation counts only its additional commits. A transition synthesized in this invocation counts in the inbox/archive domain, not as a replay; if an interrupted invocation fsynced it without committing, its successful replay from the next invocation's input spool counts in that next replay count. The domains are independent: two unique uncommitted inputs, an identical duplicate, an already committed prefix and two newly synthesized inbox transitions yield `replayed_count=2,inbox_count=4,archived_count=0`. Database-only missing-path recovery has zero replay count; its newly committed closure/loss events count toward inbox/archive.

Before returning an existing completed terminal result, validate supplied expected run/epoch against the currently inspected registry. A matching no-work return preserves existing status, diagnostics and attestation provenance, reports zero counts and advances no ownership, observing terminal history even if the recorded current reconciler remains alive. A successful exceptional takeover advances the epoch, so repeating its original pre-takeover expected epoch is stale; any later owner change also invalidates an earlier expected epoch.

Recovery follows this sequence:

1. Validate complete database/run identity, acquire the member's pane-input lock when a concrete placement remains, then open/verify the recorded runtime directory and lock inode without creating a replacement. A matching held lock returns `live_or_unreachable`. A creator-owned run still in startup also requires creator/pinned-process reconciliation even if its precreated lock is free. A missing path, changed inode or unregistered startup owner follows the special branch below, not a free-lock result.
2. After acquiring the matching ownership lock, CAS the run's owner epoch to the next integer, record the recovering PID/start identity, set `owner_kind=reconciler`, preserve `recovery_origin_status` on the first pass and set `status=recovering`. A later pass takes a fresh epoch and retains the same original status. This fences prior event producers and input clients.
3. Under that exclusive ownership, validate the entire complete frame sequence before replay: checksums and sequence identities match, and `event_seq` is nondecreasing in physical order. Reconcile identical same-sequence originals idempotently; conflicting same-sequence originals or any physical sequence regression are complete corruption, which blocks the pass and preserves original spool bytes. Replay valid originals in their recorded ascending order, updating interactions before later closures. An incomplete final suffix retains its separate tail diagnostic and exclusive-owner handling. Choose inbox or archived insertion from current member/fleet/Director status within each transaction.
4. After all complete original frames are reconciled, reserve new sequences above the durable reserved high-water mark, committed events and every valid complete outbox frame. Derive the terminal outcome from the original status and any replayed terminal lifecycle record. If neither establishes an exit/failure/loss, append a pending interaction's `monitor_lost` closure and then `runner_lost` through the same fsynced outbox/idempotent transaction path. Otherwise finish the recorded terminal outcome without inventing a loss; a replayed child exit supersedes the stale pre-crash running status. Repeated passes detect existing closure/loss records instead of regenerating them.
5. Persist the final `lost`, `exited` or `failed` status, ended/cleanup evidence and notification disposition, then truncate/fsync the outbox after validating all complete frames are committed and release ownership. Reconcile verified descendants under the bounded cleanup rules; unknown ownership remains a diagnostic. Retain archived events permanently under database retention and delete the outbox only when all complete frames are durably represented.

Only the lock owner reads/compacts/writes the outbox, including in normal event-worker operation, so an external pass cannot race the live writer.

For a missing directory/lock or identity mismatch, a recorded owner still alive under its PID/start identity returns `live_or_unreachable`; unknown process identity returns `operator_reconciliation_required`. Preserve suspect state and the open interaction in both cases. An owner may hold an unlinked original lock, so creating/acquiring a new pathname supplies no death proof. Startup without a registered runner requires reconciling the creator, pinned pane and any started process before treating ownership as stopped.

For unreadable owner identity, expose the exceptional Director-owned `--owner-terminated` form (§6.3 `member recover`), whose `--evidence-ref` names the operator's recorded attestation that the inspected owner and startup processes have terminated. Without that attestation, unreadable identity continues to return `operator_reconciliation_required`.

The Director obtains the run ID, owner epoch and PID/start identity from the inspected registry record and binds the operator evidence to that snapshot. Revalidate database/run identity, epoch and owner tuple after obtaining the SQLite write lock; a changed run or owner rejects the stale attestation without rebinding it. A matching live-process identity, held identity-matching lock or matching live health reply rejects the termination claim even with the flag. For an unchanged owner with unreadable identity and no positive live evidence, record `owner_termination_attestation` in `cleanup_diagnostics_json` with the original run/epoch, owner PID/start token, evidence reference, UTC timestamp and `source=operator_via_director`, then execute the database-only branch in the same CAS transaction. Preserve that provenance separately from machine-verified termination. This attestation enables bookkeeping closure of the identified owner only; it authorizes no child creation, input, approval answer or replay.

Once the recorded owner's termination is machine-verified or accepted through that attributed attestation, perform database-only reconciliation in one bounded immediate transaction, rereading the fixed database/run identity, placement, epoch, owner tuple and supplied attestation after obtaining the SQLite write lock. A completed terminal result uses the expectation-checked no-work return above; a concurrent live reconciler on unfinished recovery causes deferral. For unfinished recovery, advance the epoch and record the recovering process identity, retain separately attributed provenance, increment the gap count once and add `runtime_path_missing` to cleanup diagnostics.

Derive the terminal outcome from durable terminal records or the preserved original terminal status, retaining the exact recorded exit code/signal and existing event/message bytes. When a terminal outcome is established, finish that outcome with no new lifecycle event. Otherwise reserve sequences above the durable high-water mark, close any persisted interaction with `monitor_lost` before `runner_lost`, and finish as `lost`. In the same transaction, settle every deferred `ready` receipt as `not_submitted` with reason `runner_lost`, then publish each durable first deferred result that has no `input_delivery_result` event yet on sequences reserved above the high-water mark, deduplicated by run and input (§ Input delivery result event). Choose inbox or archived insertion from current lifecycle state within the same transaction. This branch performs no outbox replay and leaves surviving mismatched artifacts untouched; missing observations remain an explicit gap. Commit the terminal outcome and all new bookkeeping/records atomically. A failed transaction leaves the original state for retry.

Cap the outbox at 16 MiB per run. Retry database contention with bounded exponential delays from 100 ms to two seconds, each clipped to the same absolute database/work/shutdown budget rather than added to the connection's default timeout. Keep terminal transport and socket health service independent. A full channel/outbox or filesystem failure changes the runner to `degraded`, holds all programmatic input, emits one local health diagnostic and retains the failed transition plus an explicit observation-gap counter. Stop further semantic classification rather than silently discarding events, continuing manual terminal I/O, and resume with a fresh screen and an explicit gap event after the writer recovers. `runner_degraded` carries the gap interval/count, and events potentially missed during that gap remain unverified.

Durability begins at a successful SQLite commit; a spooled record is recoverable pending delivery, not a delivered notification. A hard crash before spool fsync may lose an observation; runner-loss reconciliation identifies that coverage gap. If both database and filesystem writes fail, use the control health response and local diagnostic and preserve the actual error; memory stays bounded, and notification is claimed only after a successful commit.

#### Input arbitration and the original native answer

For a runner-managed member, every CAFleet-generated preview, `member prompt` and `member exec` goes through the pinned runner control socket instead of multiplexer pane input. The client validates and submits without holding the outer pane input lock while awaiting the runner. The runner owns that lock for wrapped writes: it acquires it nonblockingly from its event loop under the attempt deadline, revalidates the concrete placement and holds it across an active staged write, so explicit press, deletion and other programmatic writers serialize; it releases the lock while a request is only queued. Its own terminal-state loop serializes the final check and child writes.

Control frames are length-prefixed JSON with protocol version, run/nonce/owner epoch, operation key, kind and literal payload. A four-byte unsigned big-endian prefix encodes the serialized UTF-8 JSON body length; the inclusive 65536-byte (64 KiB) body limit excludes that prefix, and an oversized declared length fails immediately after the header is read. Requests and replies require the top-level integer `protocol_version: 1`; missing or unsupported versions and truncated frames fail explicitly. § Input control frames and deadlines lists the complete set of request kinds and `receipt` actions. Submission replies distinguish `accepted`, `queued`, `held`, `rejected` and `uncertain`, carrying a nullable input ID, output-sequence evidence, byte/submission stage and the delivery projection; the ID is null for a zero-byte attempt with no durable row, and `queued` preserves zero-byte stage evidence. Conflicting reuse of an operation key with another payload digest fails; store no literal prompt/command in the receipt table.

##### Input control frames and deadlines

New input, permission and receipt requests require top-level `protocol_version,kind,identity,expires_at_mono_ns`. `identity` has exactly `database_identity_json,fleet_id,member_id,director_member_id,run_id,owner_epoch,launch_nonce`, binding the complete selected database and recorded owner tuple. Submissions validate recorded database/run/nonce and historical associations before durable key lookup; current epoch/owner and active eligibility checks apply only to a new or live ready attempt. IDs/epoch are positive signed 64-bit integers, sequences and byte counts unsigned 64-bit integers, and `expires_at_mono_ns` a positive unsigned 64-bit CLOCK_MONOTONIC nanosecond expiration. Each `kind` adds required fields:

| Kind | Additional exact fields |
|---|---|
| `submit_prompt` | `operation_key,input_kind,message_id,literal`; input_kind is preview or prompt. |
| `submit_exec` | `operation_key,input_kind,message_id,literal`; input_kind is exec. |
| `permission_inspect` | `interaction_id,event_id,director_member_id`. |
| `permission_respond` | `interaction_id,event_id,director_member_id,expected_run_id,expected_owner_epoch,request_token,decision,decision_source,director_mode,mode_evidence_ref,decision_evidence_ref,operation_key`. |
| `receipt`, action `query` | `action,operation_key`. |
| `receipt`, action `retire` | `action,input_id`. |
| `receipt`, action `reconcile` | `action,input_id,expected_state,disposition,evidence_ref`. |

Preview requires a positive message ID; prompt/exec require null `message_id`. `literal` is the unchanged message/prompt/command text; the exec plan supplies its own `! ` prefix. Require nonblank operation keys, validate the ordinary single-line/control-character contract and enforce matching outer kind/input_kind. Permission IDs/expected epoch must agree with identity and the stored association. Reconcile is the Director-owned input coordinator's private action; its enum values match the durable state/disposition domains. Reject unknown, duplicate, missing or wrongly typed fields before effects. Lifecycle requests carry `protocol_version`, `kind` and `identity` and no `expires_at_mono_ns`: the handshake's `prepare` adds `remaining_ms` and `start` adds `pane_id`, and `health` and `shutdown` add nothing.

Replies retain exactly `protocol_version` plus one of `result` or textual `error`. Input results have exactly `kind` plus the following field: submission/`submission`, inspection/`request`, permission_response/`response`, or receipt/`receipt`. Submission contains exactly `status,input_id,stage,output_seq,reason,delivery`, where `delivery` is the ordered `status,method,reason` projection (§ Delivery reporting); permission response and inspection carry the complete ordered public objects fixed by the CLI contract. A receipt object has exactly `status,input_id,submission`: missing has null ID/submission; present has a positive ID and submission object; retired or settled has a positive ID and null submission. Result kind must match the requested operation. Clients validate and reload durable input/audit rows from their supplied database connection; socket receipt metadata never replaces durable identity.

The original invocation captures one expiration and carries it through framing, transit, queues, SQL and every native stage. Convert using paired clock samples `Instant_before,CLOCK_MONOTONIC,Instant_after`, validating sample order and checked arithmetic. Sender expiration adds the remaining original deadline measured at Instant_after to the earlier monotonic sample. Receiver deadline adds remaining expiration measured at the later monotonic sample to Instant_before. Scheduling delay can shorten this budget and grants no extension; use no wall clock. Clock failure, overflow or expiration fails before new reservation/native bytes while preserving prior committed or possibly transmitted state.

The service also caps new-operation admission at five seconds from the first observed frame-header byte, or any earlier shutdown deadline; delayed framing and queueing consume that cap. Client exchange, lock acquisition, supplied-connection SQL scopes and stage advancement retain their earlier original/inherited deadline. Validate the declared frame size (above) before allocation and the complete serialized size before sending.

##### Bounded input draining

Before a programmatic input attempt, drain actual human and child input through the runner's manual queue, terminal parser, ordered observer and outer display. A drain returns either `Pending` or `Quiescent` with the actual cumulative `human_generation`, the count of bytes classified as manual input (§ Terminal reply classification), and terminal `output_seq`. Quiescence describes transport and observation health; the input coordinator separately validates composer/request eligibility, ownership, reservation and the original deadline before a write.

Each call executes at most eight rounds under the original deadline. A round attempts at most one human read, one manual-queue PTY write, one child read, one display-queue write and one display flush, in that order. Skip an operation only for unavailable capacity or remaining quota, an empty queue or previously observed stream `EOF`. Let B be the positive configured `bytes_per_tick`; each actual read buffer is bounded by the minimum of 16384 bytes, the remaining stream quota and available queue capacity.

| Work per drain call | Inclusive bound |
|---|---|
| Human reads, child reads, manual PTY writes, display writes | Eight calls and B positive bytes for each; child bytes include previously deferred child bytes. |
| Display flushes | Eight calls. |
| Final observation-health samples | One sample, taken only for a candidate completed quiet round. |
| Rounds | Eight; unresolved work returns `Pending`. |

`Quiescent` requires both actual reads to return `WouldBlock` in the same completed round, an empty manual queue, consumed deferred child bytes, successful parser/observer processing and positive free display capacity. The final health sample must be present, nondegraded, have `pending_count=0` and `pending_evidence=false`. An absent health adapter supplies no proof; a healthy `Hold(Unknown)` state may establish physical quiescence while remaining ineligible for programmatic input. `Quiescent` also requires no pending terminal-reply prefix: each round first resolves expired prefixes, and a round that would otherwise certify quiescence while a prefix is pending returns `Pending` with blocker `terminal reply pending` and installs no proof. The eighth round may certify these facts; quota exhaustion, a skipped read, `Interrupted` or `EOF` supplies `Pending` instead. A continuously readable source terminates at the round or byte bound. Deferred child bytes precede subsequent reads and consume the same child quota.

In both ordinary progress and input draining, classify every actual positive human read immediately and count its manual bytes with checked cumulative unsigned 64-bit arithmetic; the counts and literal queued bytes survive a later PTY, child, display, parser or observer failure. The service delivers the observed before/after count delta to manual-input invalidation after successful and failed progress alike, before propagating any error; a simultaneous notification failure accompanies the primary error. Nonrecoverable errors propagate, and a zero-length write fails with `terminal write made no progress`. `Interrupted` requires a later fresh read pair; `EOF` records the stream state and leaves shutdown to the normal service path.

Clear stored drain proof at entry, check the original deadline before I/O and before each endpoint/callback, and recheck after completed work, retaining the bytes/counts of successful completed I/O before reporting expiry. In-memory bookkeeping and observation of a completed read may finish after expiry; an observed expiration permits no later endpoint call or proof installation. Synchronous I/O and callbacks have no preemption guarantee, so finite work bounds and clock checks impose no hard syscall or callback latency cap. Every call keeps the original deadline, with no internal sleep or deadline renewal.

Retain every completed child read for display and preserve the exact prefix processed by the terminal parser and ordered observer. If a positive child chunk fails to complete that observation, an observation-incomplete fence persists for the `Transport` lifetime, through ordinary progress, resize and display flushing: later drains continue bounded manual and display progress and return `Pending` without proof, even when later health is healthy or the caller supplies a new deadline. Fully completed observation remains complete when only a later deadline or display operation fails. Partly processed chunks are neither replayed nor treated as observed suffixes; the runtime exposes no repair/reset operation for this fence.

Only a successful final check installs an opaque, single-use proof bound to its transport instance, drain turn and actual health/generation/sequence facts. The visible result fields describe evidence and grant no write authority. Taking the stored proof consumes it once, and validating it while a terminal-reply prefix is pending fails with `input transport is no longer quiescent`. Ordinary progress, resize, prefetch, display mutation/discard and a new drain invalidate it before their validation or I/O, including later error paths; pure generation/snapshot reads preserve it. A new transport begins with generation 0 and no proof. Programmatic writing consumes actual coordinator-owned authority, never a caller-constructed result.

##### Durable input receipts and settlement

Preview keys are exactly `preview:<run_id>:<message_id>` and are independent of CLI invocation or connection. Every submission first checks the durable operation row, including retired dispositions. Before a later send/wait retries an unstamped message, it queries the receipt. Durable `accepted` permits stamping without typing; `in_progress` or `uncertain` holds for inspection, a queued `ready` row returns its queued result, and a settled key returns its disposition without replay. Only absence of a row under the same verified run/database identity, or a recorded `ready` zero-byte state without runner queue ownership, permits reevaluating input eligibility. A held/rejected zero-byte attempt creates no side-effect receipt; a row already reserved moves to `ready` only after positively recording that zero bytes were written.

An ordinary `receipt query` reads the durable operation for its fixed run/key and reports its durable status with the last real retained stage and output sequence. `missing` requires an absent durable row and absent matching cache entry. A durable row remains `present` after cache retirement; a cache entry without its durable reservation is an identity error. Validate the retained immutable input identity before using its stage. The receipt projection table in § Delivery reporting maps each durable receipt to its `submission.status` and its stage without a cache entry. With a matching retained entry, return its actual stage and output sequence alongside that durable status; otherwise the output sequence is null. Query preserves pending writers, clients, permits, receipts and native bytes. Its results describe observations: new write authority and zero-byte retirement proof still require their separate validated guards. A settled `not_submitted` or `abandoned_uncertain` disposition alone supplies no observed byte stage; its absent-cache result remains `unknown` and leaves the original transmission uncertainty intact.

Before the first child byte, commit `in_progress` to `member_input_operations` under the current ownership epoch and the bounded database budget. If that commit fails, write no child bytes. After submission, persist `accepted` or `uncertain` before promising that durable outcome; failure to persist after a possible write leaves the durable row in progress and returns uncertainty. The live runner may later reconcile its retained stage evidence into the same row.

Retire only the in-memory receipt after validating durable settlement, the current full run/owner and original run lock, exclusion of every active writer, and absence of a leased guard or available attempt permit for that input. Cache absence is idempotent only after the same durable and ownership checks. For a preview, atomically stamp `notified_at` (or verify the message is already ACKed) and set its input row to `settled/submitted`; a later `receipt retire` verifies that transaction before freeing memory. For prompt/exec, the durable accepted input row itself is the queryable submission record: the creator or runner can settle it as submitted and retire the cache after verifying that stored row. Printed stdout only presents its input ID and supplies no durable acknowledgement. Settled operation keys/digests/dispositions persist for the lifetime of the database file, so a delayed request still returns the prior disposition and never types again.

Bound unretired side-effect receipts, including queued deferred inputs, to 4096: while that unresolved set is full, new side-effect attempts fail with an explicit zero-byte capacity error; settled receipts leave the set. Keep uncertain receipts until the Director records an evidence-based disposition through `input reconcile` (§6.3 Runtime event and input commands), which only records disposition, settles the key and permits cache retirement; it grants no native permission and schedules no replay. `input show` exposes that durable record. Even a settled not-submitted key is consumed; a later deliberate action uses a new operation key after reconciliation.

Reconciliation runs under the original deadline clipped to any inherited SQL bound, and every transaction it commits is a supplied-connection immediate transaction. It pins the original input row in its supplied-connection snapshot: `input_id`, `run_id`, `operation_key`, `kind`, `payload_digest`, `message_id`, `created_at`, the complete immutable permission-response audit where present, and expected state remain fixed through live preference and exclusive settlement. For a recorded concrete placement, acquire its genuine pane/input lock and retain that same guard through current-owner synchronization and any receipt-only exclusive ownership proof. Synchronize with the pinned runner's receipt handler so an active byte-writing operation cannot be settled concurrently; an unreachable owner requires the exclusive proof below. Compare-and-swap the original expected operation state in the settlement transaction; a changed row requires a separate invocation. Settlement records the observed input disposition, not the native command's execution outcome.

For an ordinary or permission receipt already settled with the requested disposition, reconciliation returns the unchanged validated durable row and its original `evidence_ref` before live locks, health probes or exchange. A conflicting disposition fails with the row unchanged. This historical return grants no new input authority. For a live nonsettled receipt, validate the full fixed identity and original ownership lock inside the transaction of the expected-state settlement CAS, after the SQL wait. After a new reconciliation, require the matching `settled` acknowledgment and a validated durable reread, then exactly one `retire` exchange with a matching input ID under the same pane guard and original expiration. A post-commit failure retains the known input context and original cause while the durable settlement remains authoritative. A changed expected state requires a separately reconciled attempt.

Receipt-only reconciliation for ordinary and permission inputs accepts the fixed placement state from that original snapshot. A present placement requires its matching, continuously held pane/input guard. An absent placement requires an absent guard, valid historical member/fleet/Director relationships and a deregistered member or deleted fleet; an active member in a nondeleted fleet requires its recorded placement. Revalidate the exact placement or its exact absence, complete fixed run/input identity and historical relationships inside the settlement transaction after preceding waits. A changed placement, identity or expected input state invalidates the attempt.

Both placement states require the recorded identity-matching original run lock and positive exclusion of the previous owner and relevant startup processes. A held original lock, matching live process/health, unresolved termination or competing live reconciler preserves uncertainty. Missing or replaced runtime paths supply no original-lock proof. Placement absence, a failed live call and prior exceptional bookkeeping each grant no takeover authority; retain the original live failure when typed exclusion cannot be established. Valid absent-placement history proceeds directly to the same exclusive proof.

Its receipt-only CAS advances only the ownership tuple: increment `owner_epoch`, set `owner_kind=reconciler` and record the current reconciler PID/start identity. Commit that CAS, expected-state settlement and any matching claim removal authorized by a genuine zero-byte proof atomically in one transaction; any failure rolls all those mutations back. Preserve lifecycle status, recovery origin, exit fields, cleanup state/diagnostics, events, reservations, spool and immutable input/audit history. Retain the exclusive guard through commit and release it on every outcome. This path performs no observation recovery, child operation or native input; ordinary member recovery retains its completed-terminal no-work behavior.

A durable ready row is positive zero-byte evidence only under actual writer exclusion and exact associations. Owner death or an attributed not-submitted disposition alone supplies no such proof; recovered in-progress, accepted and uncertain records keep their transmission uncertainty. Proven-zero settlement and matching claim release commit atomically within the same proof lifetime. A ready response from a different epoch/token requires a fresh inspection and separate deliberate key; its immutable metadata is never rebound.

The live input guard distinguishes native-write authority, receipt-only writer exclusion and current-plan ready rearm. A production zero-byte proof borrows its still-held guard, binds the complete immutable input/audit and current ownership, and carries a non-Send marker; its constructors and fields remain private. Receipt-only exclusion authorizes bookkeeping without a write permit or current visible prompt. With a retained entry, proof requires its actual zero-byte stage and cumulative count zero; without one, only a validated durable ready row under genuine writer exclusion supplies it. A projected retry stage or a settled not-submitted label supplies no replacement observation.

Rearm additionally requires the unchanged old token/epoch/metadata and the actual current request, selected answer plan, `human_generation` and healthy complete observation drain. Revalidate owner/audit/claim under the bounded immediate transaction, CAS ready to in-progress once, then require fresh normal pre-stage checks before any native byte. Retain the input ID, original audit, decision timestamp, key and digest. Preserve its matching claim, or restore that exact pair only when a released ready row has no competing claim. The exclusion-only proof grants no rearm authority. Each explicit ready invocation captures one expiration; an in-progress stage continuation retains its original attempt expiration.

Permission receipts use the ordinary query, reconciliation, receipt-only exclusion, zero-byte proof and retirement rules above. A proven-zero not-submitted settlement deletes the matching claim in the same transaction; the proof comes only from actual retained zero-byte in-progress evidence or exact durable ready history. Possibly transmitted, absent-cache, submitted and abandoned-uncertain history retains its claim. Retirement leaves durable audit, key and claim history. The native startup probe reports unknown, so unavailable exclusion preserves the original live failure.

A process loss, ownership change or unavailable receipt permits no automatic replay of an attempted submission. Keep notification unstamped, retain its durable uncertainty and reconcile actual output before a separate deliberate action. Explicit prompt/exec use generated keys and printed durable input IDs with the same rule.

Native-exec success means submission, not execution success. Queued and held messages keep `notified_at` unset, and only confirmed submission or ACK settlement (§6.3 `message ack`) stamps them. Queueing supplies no new permission event and never closes the native request.

Screen observation cannot create an atomic native request API: unread child output may race with a write, an unknown CLI can redraw unexpectedly, and a person can type concurrently. The runner therefore guarantees zero ordinary-input bytes for detected and candidate interactions and refuses uncertain states; tested composer profiles bound the remaining race for profile delivery without asserting universal prompt preservation. Release evidence must include this race and preserve uncertainty instead of authorizing a second submission.

Human keystrokes from the outer terminal pass to the child as native input, including the human's intentional approval, denial or cancellation. A Director may submit a request-bound allow or deny through the permission-response path below after applying its session decision policy; ordinary preview/prompt/exec input stays queued with zero bytes during the request. Keep attributed decisions, key submission, native outcomes and actual continuation separately observable. Unknown answer capability or uncertain request identity preserves manual original-surface operation.

##### Delivery method and interaction priority

These ordered rules select the delivery of every wrapped preview, `member prompt` and `member exec`:

1. Validate target/run identity, reachability, writer ownership, storage and receipt history. A failed invariant or an uncertain prior attempt is an explicit operational failure, never a reason to send fallback bytes. Unknown detection coverage alone is valid capability absence, not an observer-health failure.
2. A candidate or open detected interaction queues ordinary input with zero bytes. An obscured interaction remains preserved, and `permission_request`, `question`, `trust`, `update` and `unknown_interaction` are all blocking interactions.
3. A verified empty composer with the operation's executable plan uses profile delivery without Escape (§ Measured submission plans and settle intervals). This includes a working member whose composer row is empty: the native CLI queues the submitted input behind its current turn without interrupting it.
4. With an executable plan and a busy composer (one not verified empty: a draft is present or the observer cannot classify the composer), queue the input with zero bytes under defer reason `composer_busy`. The runner drains it by profile delivery as soon as it observes a verified empty composer. A queued head that has waited 3 minutes only on a busy composer is delivered by fallback with reason `idle_timeout`. The bound starts when the head first waits solely on that composer and restarts whenever an interaction blocks it; time spent behind an interaction or earlier input never counts. A detector that stops matching a newer native UI therefore delays delivery rather than stranding it.
5. Otherwise use delivery fallback: Escape, settle 500 ms, literal prefix and text, settle 500 ms, Enter (`0d`). This covers missing operation plans, missing profile coverage and failed version probes.

| Selection | Bytes | `delivery.method` | Selection reason |
|---|---|---|---|
| Queued behind an interaction, a busy composer or earlier input | Zero | `null` until a later attempt selects one | `interaction_open`, `interaction_candidate`, `composer_busy` or `earlier_input` |
| Profile delivery | Literal stage, 500 ms settle, measured submit key | `profile` | `null` |
| Fallback | Escape, settle, literal stage, settle, `0d` | `fallback` | `submission_plan_unavailable` (rule 5) or `idle_timeout` (rule 4 bound) |

The Director interrupts a working member explicitly with `cafleet member press MEMBER_ID --from-member-id DIRECTOR_ID Escape`; ordinary delivery sends a member whose profile has an executable plan no Escape before the 3-minute bound.

Fallback runs through the owned child PTY and the same input coordinator and receipts, never a second multiplexer input writer, and preserves the caller's literal and the exec `! ` prefix. Revalidate the higher-priority interaction rule before Escape and every later stage: a newly detected interaction before the first byte requeues the input; one after a positive write stops the remaining stages with uncertainty.

Fallback may interrupt work, mix text with a draft, and dismiss an undetected permission request or question on an unknown-coverage run (§6.5 Accepted trade-offs). The guarantee is suppression for detected and candidate interactions, not detection of every native form. Semantic permission answers continue to require their independently tested request-bound plans and never use fallback. Human keystrokes and request-bound permission responses keep their separate paths and can resolve the interaction that blocks ordinary input; closing that interaction grants no permission and supplies no native execution result.

##### Runner-owned deferred delivery

The runner owns a FIFO of ordinary input requests, including previews. It accepts a blocked request into the queue only after allocating its durable input ID and committing a `ready` receipt with `deferred_at` and `defer_reason` under its operation key, digest and target identity. The literal or control payload stays in runner memory; receipt tables, result events and diagnostics retain metadata only.

Only one ordinary request writes at a time, in enqueue order, and a request queued behind another receives the same explicit queued result. Queue processing is scheduled from the runner event loop:

| Trigger | Runner action |
|---|---|
| `interaction_closed` after its event transaction commits | Drain pending output and revalidate the current state; a superseding interaction keeps the queue blocked, and closure due to exit or monitor loss dispatches nothing |
| A candidate clears | Retry the queued head after rechecking the current interaction state |
| The active input completes | Retry the queued head after rechecking the current interaction state |
| A temporary zero-byte obstacle during an active attempt | Retry on a 100-millisecond timer within the same five-second attempt deadline |

No external `message send`, `message wait` or Director wake is required to resume the accepted queue. Immediately before writing a queued preview, recheck that its message remains unconsumed and its target active; an ACK or target removal cancels its zero-byte receipt as not submitted and advances the queue rather than typing an already consumed preview.

A queued request outlives the originating CLI connection and command deadline. Each actual drain attempt has its own five-second budget, and waiting for an interaction consumes none of it. A client timeout after queue acceptance reports the known input ID and directs the caller to inspect the receipt instead of generating a new operation key. The timer retry runs even without child output or broker commands, preserving the input ID, FIFO position and `ready` receipt without renewing the deadline. A reappearing candidate or open interaction pauses dispatch and ends the zero-byte attempt; the next closure or clearance starts another bounded attempt.

| Deferred-attempt condition | Receipt and queue outcome | Result publication |
|---|---|---|
| Busy pane lock or temporary storage/observer unavailability before the deadline, with zero bytes | Retain the queued head and retry on the timer | Input inspection remains `queued` with its original defer reason |
| Five-second deadline expires with zero bytes | Settle `not_submitted`, clear the unused selected method, and advance after the terminal transaction commits | `input_delivery_result` with reason `pane_input_busy`, `storage_unavailable`, `observer_unavailable` or `delivery_budget_exhausted`, according to the last obstacle |
| Target/run/placement identity fails revalidation or a queued preview is consumed, with zero bytes | Settle `not_submitted` immediately and advance after commit | The result with reason `target_inactive`, `target_changed` or `message_consumed` and diagnostic context |
| Any attempt may have written a child byte | Record `uncertain`, stop automatic ordinary dispatch and retain its method and selection reason | The uncertain result; receipt reconciliation (`input reconcile`) is required before advancing |

If storage prevents the terminal transaction, discard the zero-byte payload and retain a metadata-only pending finalization at the FIFO head. Retry that transaction on the 100 ms timer without sending bytes or starting another input; commit the receipt when storage recovers, then advance, and its result event follows from that durable receipt. While publication is unavailable, write a runner-stderr diagnostic naming the input ID and persistence failure; the Director receives the durable result after recovery. Ownership loss transfers finalization to the fenced recovery path, and the old writer neither updates nor dispatches again. An accepted request therefore becomes queued, submitted, not submitted or uncertain; `held` remains a result for work the runner did not accept into its queue.

Orderly shutdown cancels queued zero-byte requests as `settled/not_submitted` with reason `runner_stopped`. After runner loss, recovery may settle a deferred `ready` receipt as not submitted with reason `runner_lost` once ownership fencing establishes that its in-memory payload and writer are gone. Explicit payloads are never restored from disk; a later deliberate submission is a new request. A preview settled not submitted remains an unnotified broker message and may be retried explicitly under a new run's preview key.

##### Delivery reporting

`message send`, `member prompt` and `member exec` JSON append a `delivery` object with ordered `status,method,reason` (§6.3 Runtime presentation). `method` is `profile`, `fallback` or null before selection. The shared projection is:

| Status | Meaning | CLI behavior |
|---|---|---|
| `submitted` | Complete literal and submit stages were written | Exit 0; send has `notification_sent=true`, exec has `dispatched=true` |
| `queued` | The runner accepted durable zero-byte deferred work | Exit 0; send has `notification_sent=false`, exec has `dispatched=false`; the input ID and reason are returned |
| `held` | No queue acceptance or write, due to unavailable health, ownership or storage | Send preserves its committed message and prints the reason; explicit prompt/exec exits 1 with its delivery context |
| `uncertain` | Some stage may have written; reconciliation is required | Exit 1 with input identity and stage diagnostics, including the recorded `uncertain_cause` (§ Uncertain cause) and the selected method |
| `not_submitted` | Work was cancelled with established zero-byte evidence | Visible through input inspection and, for deferred work, its result event |
| `not_applicable` | A self-send or root-Director target uses broker consumption without automatic pane delivery | Send retains `notification_sent=false`, null method and input ID, and no warning |

A held `message send` reports one of these reasons:

| Held reason | Cause |
|---|---|
| `observer_unavailable` | The wrapped recipient's runner cannot currently observe its native screen, so it accepted no queue work |
| `notification_held` | No deliverable pane, a recipient deregistered after the commit, a placement or run change during the attempt, or an unresolved earlier preview receipt |
| `notification_failed` | The ordinary recipient has no recorded run, the pane stayed busy, runner exchange or storage failed, or the attempt deadline expired |

Self-sends and sends to a root Director project as §6.3 `message send` specifies; self-sends stay excluded from automatic preview selection and the runner queue.

Preview forms, body folding, inline limits and long-body pointers follow §6.5 Push notifications.

Ordinary inputs (`preview`, `prompt` and `exec`) use SHA-256 over `digest::canonical_json` of `run_id,kind,message_id,literal`. The preview client sends the canonical base preview, the method-independent wrapped line ending at the resume clause, in `SubmitInput.literal`. The runner reloads the immutable broker message, validates its run/recipient/key relationship, reconstructs the same canonical base and requires byte equality before reserving or reusing the receipt. Prompt and exec use their submitted literal and a null `message_id`. A preview preimage is:

```json
{"run_id":42,"kind":"preview","message_id":99,"literal":"CANONICAL_BASE_PREVIEW"}
```

Method, appended clause, native prefix/key bytes and timing are outside the digest preimage. The runner selects and renders afresh only while zero bytes are established; once a stage writes, its method and rendered bytes stay pinned through submission or uncertainty. Queries and duplicate requests validate canonical identity before current composer state, so a later method change neither changes the digest nor authorizes retyping an existing input.

Permission responses use the canonical payload digest of § Permission response storage; typed permission reservation, retained-operation lookup and input reads validate it against the immutable audit row and its bound run, interaction, event and Director (§ Request-bound permission responses).

`defer_reason` records why a request first entered the queue and stays paired with its original `deferred_at`. `delivery_reason` records the eventual selected-method reason or a terminal zero-byte cancellation reason. The owned transaction that changes an ordinary receipt to `in_progress` commits its `delivery_method` and `delivery_reason` before the first byte: profile delivery has a null reason, and fallback has `submission_plan_unavailable` or `idle_timeout`, also for an immediate fallback. A zero-byte requeue clears unexecuted selection metadata before any later reselection; a terminal zero-byte cancellation commits a null method and its outcome reason. After any possible positive write, the selected method and reason persist through accepted, uncertain and settled states.

Every result consumer projects these durable fields rather than recomputing a reason from the current catalog or composer. Receipt outcome, eventual method and reason and preview notification settlement commit together, and the deferred-result event follows from that durable receipt (§ Input delivery result event).

| Ordinary receipt state/disposition | `submission.status` | Stage without a cache entry | `delivery.status` | Method and reason |
|---|---|---|---|---|
| `ready` with deferred metadata | `queued` | `zero_bytes` | `queued` | Null method; original `defer_reason` |
| `ready` without deferred metadata | `held` | `zero_bytes` | `held` | No active queue ownership; null method and `zero_bytes_ready` |
| `in_progress` or `uncertain` | `uncertain` | `unknown` | `uncertain` | Recorded selection; no automatic dispatch or replay |
| `accepted` or `settled/submitted` | `accepted` | `submitted` | `submitted` | Recorded selection |
| `settled/not_submitted` | `rejected` | `unknown` | `not_submitted` | Null method and recorded outcome reason for a zero-byte cancellation; recorded selection for a reconciled attempt that may have written bytes |
| `settled/abandoned_uncertain` | `rejected` | `unknown` | `uncertain` | Recorded selection; abandonment ends retries without establishing zero bytes |

`submission.status` is the runner's outcome vocabulary for the attempt (§ Durable input receipts and settlement); `delivery.status` is the separate delivery projection. An ordinary receipt whose method and reason were never recorded takes its status from the same mapping with a null method and reason `method_unrecorded`, except the `ready` row without deferred metadata, which keeps `zero_bytes_ready`; no method or cancellation reason is invented. Permission-response receipts expose `delivery:null` and null deferred metadata; their semantic decision/submission/settlement projection remains authoritative. Unknown stored combinations fail validation.

An asynchronous completion leaves the originating CLI output unchanged; `input show` exposes the persisted method, and the first submitted, not-submitted or uncertain result of a deferred input emits one `input_delivery_result` event (§ Input delivery result event).

###### Uncertain cause

Every runner transition of an input from `in_progress` to `uncertain` records exactly one `uncertain_cause` in the same update, and storage refuses that transition without a cause. The cause attributes the uncertainty, separately from `delivery_reason`, which keeps the selection reason unchanged after a positive write.

| Value | Recorded when |
|---|---|
| `manual_accounting_failed` | Manual-input accounting failed when a staged stage stops |
| `manual_input` | `human_generation` changed since the literal stage when a staged stage stops |
| `interaction_candidate` | A staged stage stops on a blocking interaction whose eligibility is held as a candidate |
| `interaction_open` | A staged stage stops on any other blocking interaction: a visible or obscured interaction, an interaction hold or a retained transition |
| `observer_unavailable` | A staged stage stops because the observer is unavailable, has a gap or reported an observation diagnostic |
| `short_write` | A programmatic write returned a positive count smaller than its stage |
| `write_failed` | The attempt failed after a possible child byte: a PTY write error, a post-write checkpoint, manual-accounting or deadline failure, a failed deferred attempt or a failed permission-response attempt |
| `runner_stopped` | Runner shutdown ended an active deferred attempt that may have written a child byte |

At the settle check before a staged submit, the first matching row among the first five, in table order, wins; accounting failure comes first because the generation comparison is unreliable once accounting failed. A stage that stops on an interaction after an earlier stage wrote records `interaction_candidate` or `interaction_open` by the same candidate test. A partial or unknown write outcome records `short_write` for a positive short count and `write_failed` otherwise.

Settlement and reconciliation preserve the recorded cause. Inputs that never became uncertain and `in_progress` inputs left by runner loss have a null cause. `input show` and `input reconcile` report the cause, an uncertain submission reply carries it as `reason`, and an uncertain `input_delivery_result` event carries it as `cause`.

##### Explicit manual key input

An explicitly authorized `member press` sends only its requested ordered keys through the member's outer multiplexer pane under the shared pane lock and selected-context/deadline contract (§6.5 Explicit key dispatch). It appends no Escape or Enter. This manual path remains available during observer degradation or a programmatic composer hold and follows the separate CLI acknowledgment/error contract (§6.3 `member press`).

The runner classifies and counts the resulting actual positive outer-terminal reads as manual input (§ Bounded input draining), accounting each new delta once and before subsequent automatic advancement. Observed manual input invalidates automatic authority through the retained generation; a key argument or backend acknowledgment supplies no byte count, drain proof or permission receipt. Press changes no durable input/response row. Native approval/denial/cancellation and actual continuation remain independently observed outcomes under the Director/user policy.

#### Request-bound permission responses

The Director uses its own session's effective approval mode, independently of the member's launch mode. In verified auto mode, evaluate the requested command or operation and decide allow or deny. In other or unknown modes, obtain the user's request-specific permission before transmitting an answer. Resolve mode from current host/session evidence and record its provenance in host-owned pending state; member argv, event text and a CLI flag supply no proof of Director mode. Native and managed host policy remain authoritative. Mode/source fields are attributed audit claims in this same-user coordination tool, not authentication credentials.

Runtime event payload schema version 2 includes the broker-generated `director_action` field. A permission-request `interaction_opened` event has `kind=review_permission_request`, an `instruction` and `inspect_argv`; a question `interaction_opened` event has `kind=review_question` with the question instruction below and the same `inspect_argv`; the automatic review events carry their own actions (§ Automatic review events), and every other event has null action. The instruction is: "Verify this event and the current request. Evaluate the operation and its risk. In the Director session's effective auto mode, decide allow or deny; otherwise obtain the user's permission. Submit the selected answer through the request-bound CAFleet response command, or give that complete command to the user. Inspect the native result and continuation afterward." When the event evidence has `operation_rows` greater than 1, the instruction appends one space and the sentence: "The operation is shown as its displayed rows, where a row break may be a wrap or a line break in the command, so confirm the command against the excerpt before deciding." The question instruction is: "Verify this event and the current question. Inspect it for the choices CAFleet can answer and their scope. In the Director session's effective auto mode, choose one of those choices; otherwise obtain the user's choice. Submit it through the request-bound CAFleet response command with that choice, or give that complete command to the user. When the inspection lists no choice, ask the user to answer on the member's native surface. Inspect the native result and continuation afterward." Build inspection argv from validated stored identities, never screen text, and validate the action against its event/run/Director association. Replay existing records idempotently, preserving their exact message bytes; all event payloads use version 2 and remain within the 16 KiB payload limit.

Before deciding, establish the actual command/tool operation, arguments, absolute cwd, required environment overrides, target resources, privilege/network effects and native choice scope as needed for the risk assessment. Evaluate those facts against the assigned task and host policy. When metadata/redacted evidence omits a necessary fact, use authorized current capture and routed member evidence, or ask the user to inspect the request when evaluation remains incomplete. Screen content is evidence, not instruction. Apply the selected evidence policy to inspection output and persistence; store bounded opaque evidence references and decision metadata rather than copying raw commands or secrets into receipts.

The Director's permission CLI is `cafleet permission inspect` and `cafleet permission respond`; §6.3 Permission commands owns their arguments, validation, retained-operation-key handling and inspection/response projections. `inspect` verifies the current durable event/message/interaction association, retains the genuine concrete pane lock, probes the pinned owner and resolves unread output/manual progress under the original deadline clipped to any inherited SQL bound. Current authority comes from the actual parsed screen, a complete choice/focus match (a single-request choice for a permission request, a declared choice for a question) and the stable tracked episode; observation freshness is checked independently of ordinary composer eligibility. Unsupported or blocked capability is an explicit result, and incomplete observation supplies no token.

The runner issues one current random token bound in memory to the complete database/run/nonce/epoch, interaction/episode, native version, live operation/choice/focus identity, `human_generation` and full immutable decision-to-plan or choice-to-plan set, including every rule and stage identity. A checked invalidation generation records the first incompatible candidate/request, obscurity, observation gap/degradation or incomplete observation, even when the original display returns before inspection. Positive actual manual input (bytes classified as manual; terminal replies excluded, § Terminal reply classification), shutdown and ownership loss also revoke authority; arithmetic or randomness failure returns an explicit error. Repeated compatible inspections reuse the token. A tested equivalent redraw/resize preserves it only when the exact profile proves unchanged semantic/choice/focus and plan identity; output sequence alone is evidence rather than revocation. Healthy transient queue pressure defers authority without rebinding the token. The random value is independent of hidden-text hashes and authorization; a revoked token requires fresh inspection and decision reconciliation.

A cursor transit is the one incompatible frame that leaves the generation unchanged. It is a frame that yields no current request while a request is current, is not a verified non-interaction, and keeps every row of the current request's matched live region identical in text to the frame that established the request; typically the native CLI moves the cursor outside the form to redraw another part of the screen. During a transit the request is not current, so inspection returns no token and a response is blocked. A following frame with the same request restores it under the unchanged generation and token. A changed region row, a different request fingerprint, closure, `interaction_obscured`, an observation gap, child exit and positive manual input still invalidate immediately.

A gated plan changes the request between its stages, so the runner accepts exactly one change after each complete nonfinal stage: the transition the next stage's gate names. For a focus gate, a fresh quiescent observation qualifies when its episode, rule, semantic fingerprint, choices, native correlation and `human_generation` are unchanged and its resolved focus equals the gate. For a screen gate, the continuation binding of § Continuation screens applies. The runner rebinds the in-flight selection to the qualifying observation, with its new request generation, and writes the next stage, minting no token because the reservation already consumed the request token.

The next stage waits for its gate within the responding invocation's deadline, and an observation that still shows the previous stage's screen and focus keeps waiting. These end the response with the failure reporting below (`uncertain` once a byte was written), and the native surface stays for manual input with no further byte:

- a focus on any other option;
- a screen other than the expected continuation after a stage that opens one;
- the episode closing before the final stage;
- any other identity change, manual input or a degraded observation;
- the deadline expiring.

Inspection exposes the selected current post-policy evidence only through its §6.3 projection, independently of private request/match/plan state: neither operation text nor private plan eligibility substitutes for a disclosed structured fact, and a proved `single_request` choice establishes answer eligibility whether or not public evidence states the choice scope. Keep transient raw semantics, focus and plan bytes in memory; persist no token or pre-policy digest. Respond retains the requested decision's actual rule in its immutable audit/digest.

Use versioned control operations `permission_inspect` and `permission_respond` and the receipt query/retire boundary (§ Durable input receipts and settlement). A five-second command deadline covers locking, database work and socket exchange. Before the first byte, the serialized runner resolves pending manual input and unread output, revalidates the current visible stable permission request/token and tested answer capability, and commits the response receipt, audit and episode claim atomically under the current ownership epoch. The capability binds to the response without a profile identifier: its backend and native version equal the run's, its answer rule equals the response metadata's `answer_rule_id`, its request rule equals the event evidence `rule_id`, and its plan key equals the requested decision or choice with a non-empty fixture list. The request token, expected owner epoch and event/message association bind the response to the inspection that minted the token. A decision on a question, a choice on another class, a choice without a tested plan of the current question, other interaction classes, uncertain state, stale identity or unavailable storage receive zero-byte rejection. Write only the profile's answer plan for that key, rechecking relevant state and each stage's gate before that stage; partial writes or ambiguous state retain durable uncertainty. The CLI accepts semantic allow/deny decisions and declared question choice ids, not arbitrary key sequences.

Track answer progress as cumulative actual positive byte counts across ordered stages. A complete nonfinal stage retains `partial` progress and durable `in_progress`; only complete final-stage submission establishes `submitted`, followed by durable `accepted` before returning acceptance. A stage syscall returning WouldBlock or Interrupted with zero bytes may yield within the same attempt and original expiration. Retain the untouched next-stage index and any cumulative `partial` progress, then retry only that stage after fresh actual drain, current token/request/choice/focus/plan, `human_generation`, owner and immediate-transaction validation through a newly consumed guard. Completed stages stay completed, and this continuation is neither a new invocation nor a ready-state rearm.

Record a syscall result before yielding. A recorded zero-byte WouldBlock or Interrupted preserves the actual prior stage/count and releases its lease while consuming that guard; a yielding operation remains `in_progress` and supplies no zero-byte retirement proof. Dropping a consumed guard before I/O preserves its actual no-I/O state, while possible I/O without a recorded result retains uncertainty and every known earlier byte count. A positive short write, incompatible state, unrecorded possible I/O or expiration stops the remaining plan. Preserve cumulative progress, the original cause and any known input identity; persist uncertainty only within the original budget, retaining durable `in_progress` when bookkeeping fails.

##### Permission response storage

§8 holds the exact DDL of the `permission_response` input kind and the `member_permission_responses` and `member_permission_response_claims` tables.

Audit fields use the displayed column order. Every field is required except `decision` and `choice_id`, of which exactly one is present: `decision` for a permission request, `choice_id` (the `CHOICE_ID` form of §6.3 Permission commands) for a question. `decided_at` is the UTC RFC 3339 time when the runner records the attributed decision, independently of native decision time. Tokens are random 32-byte values encoded as lowercase hex; persist no screen-derived fingerprint or pre-policy hash. Evidence references contain 1–2048 UTF-8 bytes, include a non-whitespace character and contain no C0 or DEL control character; they identify host evidence without embedding commands, screen text or secrets.

In the same immediate transaction, require input kind `permission_response`, an active pinned Director, an inbox-linked opening event of class `permission_request` for a decision or `question` for a choice, and the matching unclosed interaction/run. Validate the expected epoch, event/message association and answer capability binding before inserting the input, immutable audit and claim. Other input kinds have no audit row. The composite claim foreign key binds its input to the same interaction; same-run/event and recorded ownership relationships are also checked on reads while preserving closed/inactive history; active/current eligibility applies to new attempts only.

The canonical payload digest covers `run_id,owner_epoch,interaction_id,event_id,director_member_id,request_token,decision,choice_id,director_mode,decision_source,mode_evidence_ref,decision_evidence_ref,answer_rule_id`, with JSON null for the absent one of `decision` and `choice_id`. The `owner_epoch` value is the request's `expected_owner_epoch`. Encode this metadata as canonical JSON with sorted keys before SHA-256; `input_id`, timestamps, operation key and literal native keys are excluded. Validate a matching key against this digest before any new action.

Retain a claim after possible submission, accepted submission, uncertain settlement or `abandoned_uncertain`. Release it only in the synchronized transaction that proves zero bytes, no active writer and `ready` or settled `not_submitted`; a disposition label alone supplies no zero-byte evidence. Audit rows and consumed operation keys remain retained after claim release.

Permission inputs reuse the ordinary receipt states, settlement dispositions, 4096 unresolved limit, same-key and conflicting-payload rules and synchronized retirement. The Director records the operation key before dispatch. A different key cannot bypass a submitted, active, uncertain or abandoned-uncertain episode claim; a later deliberate attempt uses fresh inspection and a new key. Recovery sends no answer keys. Key submission and recorded `decision=allow` establish neither native approval nor command success.

§6.3 Permission commands fixes the inspection, response and audit output, exit codes and failure output. When asking the user to execute a response, present the concrete command with actual snapshot values; stale values fail before bytes.

Answer-profile capability belongs to the run's selected profile, rests on native evidence (§ Profile selection by minimum version) and is independently tested for allow and deny, and for each question choice. It specifies request form/rule, single-request choice semantics for a decision or the declared choice for a question, focus/selection requirements, exact key stages and output revalidation. Use real native evidence rather than invented Enter/Escape mappings. Persistent/session-wide permission changes have no decision answer, and trust/update forms have no answer. Unsupported answer profiles preserve manual native input. Lacking an atomic native request API, serialize available input/output, reject observed mismatches and retain uncertainty for remaining races and indistinguishable requests.

#### Director consumption and continuation

The Director consumes runtime events through `message listen`, `poll --json` and finite `wait`. Runtime events reach the Director's stdout receiver and durable inbox; they provide no composer injection, Escape-first wake, goal activation or host interrupt guarantee (latency targets: § Native acceptance and coverage). Native model consumption latency remains controlled by the Director host and may be unbounded while the host is blocked or paused.

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

For a permission refusal returned to the member, the evidence route is the member's routed request: exact attempted command, absolute cwd, required environment overrides and actual refusal. A screen event can alert the Director earlier or supplement that route; it supplies no missing dispatch evidence. For a blocked request, follow the `review_permission_request` instruction (§ Request-bound permission responses); the native result it inspects is the native output/exit/error and the member's actual continuation. Previews, prompts and native commands the runner queued during the request drain automatically after the interaction closes (§ Runner-owned deferred delivery); a deferred input's eventual result arrives as an `input_delivery_result` event. Messages that were held rather than queued remain pending for the next send or a finite `message wait <DIRECTOR_ID>` rotation. A different command submitted through native exec is a separate authorized action, never proof that the original request was answered. Reconcile uncertain outcomes before retries, and preserve host/managed permission authority throughout.

The Director handles the automatic review events (§ Automatic review events) through the same wake, `event show` verification, ACK and host-owned pending-action record:

| Event | Director procedure |
|---|---|
| `interaction_opened` in automatic mode | The permission-request procedure above, including `permission respond`. |
| `interaction_opened` for a `question` | Follow the `review_question` instruction (§ Request-bound permission responses), submitting the choice with `permission respond --choice`. |
| `automatic_refusal_observed` with a routed request | Dispatch evidence comes only from the routed request fields the `review_automatic_refusal` instruction names (§ Automatic refusal event); the event and captures corroborate it and supply no missing field. In verified auto mode decide to authorize through the routed-command dispatch (`member exec`) or to refuse with a broker reply; otherwise obtain the user's permission first, then inspect the actual result and continuation. |
| `automatic_refusal_observed` without a routed request | Dispatch nothing, ACK and keep the event as an informational pending item. When the member next reports, routes a request or reaches an idle composer without addressing it, ask it through the broker whether the refused operation is still needed; the item closes when a routed request arrives or the member answers that it is not needed. |
| Several `automatic_refusal_observed` events, then an idle member that sent no report | Codex 0.163.0 interrupts the turn after three consecutive automatic denials, so the member stops without routing a request (§ Annotation detection). Treat the events as one pending item and ask the member through the broker whether the refused operations are still needed; the broker message resumes it. |
| `automatic_review_unavailable` | Record the member as interactive in host-owned supervision state, expect native permission forms from it and tell the user when the selected automatic mode cannot apply. No answer is sent. |

A routed request that arrives with no event follows the refusal route above.

#### Native acceptance and coverage

Acceptance evidence for every declared detector, composer and answer capability consists of synthetic terminal replay and real native PTYs on each registered multiplexer. A run without a selected profile, and a capability its selected profile does not declare, retain explicit `unknown` coverage and manual operation; fixed-policy launch modes use the version-selected profile like every other mode.

The release minimum is a real interactive permission request under Claude's native `default` policy and under Codex's native `on-request` policy with `workspace-write`, each on both tmux and herdr. Acceptance records pin exact installed CLI versions to their corresponding bundled profiles, and their permitted disposable fixtures actually reach the native approval surface. Every minimum case demonstrates blocked-model-independent detection, a committed actionable event/inbox record, Director consumption and a preserved original request. Separate fresh requests cover verified Director-auto allow and deny, user-authorized responses under non-auto/unknown modes, manual original-surface input and a user-executed complete bound command. The records show zero response bytes before required user permission. Missing native availability, unknown coverage or an unobserved required case fails overall feature acceptance; unknown coverage is only the compatibility path for other versions/backends. Additional profiles retain their own declared acceptance gates.

Automatic-mode acceptance covers Claude `auto` and Codex `approve-for-me` on both tmux and herdr, with automatic review available for the account and model: the omitted-default launch, an auto-approved fixture, an auto-refused fixture with its event and Director handling, and, where the native host shows one, an escalated form answered allow and deny. A later-version case repeats the launch, auto-approved, auto-refused and escalated cases on a native version above its profile's `min_version`, recording the selected profile and each UI difference that failed to match. A fixture that does not produce its intended screen is a recorded limit for that backend, not a pass.

Minimum-version delivery acceptance covers every bundled backend on each registered multiplexer, using isolated native sessions and databases and benign marker commands:

| Case | Required evidence |
|---|---|
| Threshold selection | Equal/between/above/below boundaries, catalog permutation, multiple backends, numeric component ordering, prerelease ordering, build-metadata ties, malformed thresholds and failed probes |
| Complete profile reuse | Detection, annotations, both composer operations and each available semantic answer resolve the same profile and retain actual run/capture identities |
| Native operation plans | Prompt preview and direct prompt create a native user turn; native exec runs a harmless marker command and exposes its result, with literal-to-submit timing recorded |
| Fallback operation | Missing plan and failed-probe cases show the rule-5 fallback sequence (§ Delivery method and interaction priority) and explicit fallback results |
| Busy composer | A busy composer with an executable plan follows rule 4 (`composer_busy` with zero bytes, profile drain, `idle_timeout` after 3 minutes) without any earlier Escape; a working member with an empty composer row receives profile delivery |
| Interaction deferral | Permission and question remain open with no Escape/literal bytes; answering on the original native surface yields closure and exactly one queued delivery without another send/wait |
| Adversarial transition | Candidate/obscured/superseding interaction, human input during either settle, observer/owner loss and partial stage writes preserve no-write or uncertain outcomes |
| Durable retry | A queued canonical preview drains by fallback and retains one ID/digest after composer changes and repeated queries; duplicate keys, conflicting digests, concurrent sender/drain, lost replies and stamp failures avoid duplicate dispatch |
| Deferred failure and liveness | After closure with a busy pane lock and no further output/commands, the runner timer either delivers when the lock frees or publishes not submitted at the deadline; temporary storage/observer failure and terminal-commit recovery preserve FIFO/result visibility |
| Schema and reporting | Fresh-baseline initialization, canonical digest validation, referential integrity, sequence preservation, self-send/no-pane output and durable reason/state projection for ordinary/history/permission-response receipts |

Measurements establish submitted native input separately from model consumption, permission decisions and command completion.

Live acceptance records carry the source/installed CAFleet version, native CLI version, native configuration/mode, detector and answer profiles, Director mode/provenance, attributed decision/evidence references, request identity/token, receipt/stage, multiplexer, terminal size, cwd and redacted evidence. They include a permitted operation, an ordinary command failure after allow, an actual permission refusal, an approval that blocks the model, both supported answer routes and observed continuation. The exercised cases cover stale tokens/epochs/events, same/different operation keys, conflicting decisions, partial writes/lost replies, redraw, fragmented ANSI/UTF-8, primary/alternate screens, resize, title-only prompts, question/trust/update screens, transcript quotes, identical requests, concurrent human/output/input, slow display, database locks, outbox pressure, startup compensation, runner hard kill and deletion. Unavailable native modes or unobserved cases remain explicit coverage limits and cannot pass that profile's release gate.

Acceptance additionally covers the transport, lifecycle and persistence cases below. With a live receiver and available storage, the latency target is two seconds from a stable prompt to inbox commit and another two seconds from commit to receiver output.

- Exact child argv for the selected native policy, with native/managed restrictions and sandbox boundaries preserved.
- Equal run IDs across sibling databases, and replacement at an unchanged path.
- A held write transaction and a long statement, showing that restoration and exit are independent of the connection's default timeout.
- A child exiting while its descendant holds the slave, exit during a stalled display and a locked database, and deadline-bounded terminal restoration.
- Timeout followed by the original runner's recovery, and concurrent recovery attempts.
- A command containing a known secret, which is absent from every database/message/outbox field in metadata mode and with a matching redaction pattern, including interaction updates.
- Hard kill with complete uncommitted frames, replay of an opening before loss closure, concurrent recovery, interruption during recovery, and deregistration before replay with archived rather than fabricated inbox records.
- A live unlinked lock, verified death, unknown identity without attestation, matching/stale attestations, positive live evidence rejecting an attestation and concurrent database-only recovery.
- Cross-invocation accepted-then-failed-stamp recovery, queued-then-drained delivery after interaction closure, disconnected partial writes, receipt retirement followed by delayed replay, and a lost explicit-command stdout result.
- Every registered backend's omitted-policy argv and each adapter-declared explicit policy.

## 7. Cross-cutting concerns

### 7.1 Configuration & environment

The database URL resolves per §6.3 Database selection, and config directories resolve lazily per selected backend (§6.3 Config-dir resolution). Validate supplied values and required state at their boundary.

### 7.2 Error handling & exit-code policy

Usage/parser/model/effort/line-validation errors exit 2. Application, database and transport errors exit 1 and print per §6.3 Error Messages. A successful finite timeout exits 0. Preserve primary causes alongside explicit cleanup failures. Missing required state or unknown enum/column values propagate instead of substituting meaningless defaults.

### 7.3 Output / JSON / truncation

JSON uses explicit ordered presenters and unescaped UTF-8 (§6.3 JSON output (`--json`)); message-body truncation follows §6.3 Message Body Truncation.

### 7.4 Logging & stdout discipline

Successful command records stay on stdout. Post-commit notification diagnostics and creation compensation failures go to stderr. A committed message retains successful output even when its preview cannot be delivered; an uncertain native command prints no success record.

### 7.5 Time discipline

Use UTC for durable timestamps, monotonic clocks for budgets and deadlines. Cap every bounded process/database operation to remaining time. Every guard/owned child releases resources on ordinary errors and process exit.

### 7.6 Packaging & distribution

Ship one `cafleet` binary per release target. Archives are `cafleet-v<version>-<target>.tar.gz` containing the binary; the release tag is the bare package version. The compile-time version owns `--version`, asset stamps and doctor's installation checks. Embed the `assets/` tree (§6.3 Asset installation) offline. Rust builds/release packaging require no frontend output. The documentation site retains its independent Node/pnpm tooling.

### 7.7 Agent-operation contract — one-shot command isolation

Agents run each CAFleet process as the only command in its shell-tool invocation, so a host's single-command permission rules match it; the shipped CAFleet skill owns the operating procedure.

## 8. Database schema

The binary embeds one complete schema baseline, `assets/migrations/V1__baseline.sql` (§6.3 Asset installation), whose application identity is `PRAGMA application_id` `1128351289` (`0x43414639`, ASCII `CAF9`). Each cafleet version owns its default database file, `cafleet_<VERSION>.db`, where `<VERSION>` is the package version, so each version starts from a fresh file; baseline initialization records that version in the `cafleet_database` row. The fresh baseline creates exactly the application tables declared in the schema, and its ledger holds exactly the row `(1, 'baseline')`. Compatibility diagnosis verifies, in order, the application identity, that baseline ledger, every required table’s column shape with required indexes/foreign keys, and a recorded version equal to the running binary's. Any other identified or populated file, including one recorded by another cafleet version or one whose ledger holds a row beyond the baseline, is an incompatible format whose bytes are preserved, and `cafleet doctor` reports the version's default file as absent until `cafleet setup` creates it. Unrelated auxiliary tables, indexes and triggers are compatible while those required definitions remain intact.

| Table | Responsibility |
|---|---|
| `fleets` | Fleet identity, creation/deletion and root Director reference |
| `members` | Member registration and active/deregistered state |
| `member_placements` | Multiplexer and coding-agent association |
| `messages` | Point-to-point delivery, acknowledgement and preview stamp |
| `runtime_database_identity` | Singleton database-instance identity for runtime namespaces |
| `cafleet_database` | Singleton record of the cafleet version that initialized the file |
| `member_runs` | Launch intent, ownership, lifecycle, recorded coverage and observed effective policy |
| `member_interactions` | Observed interactive episodes and visibility closure |
| `member_runtime_events` | Sequenced events with atomic Director inbox association or archival |
| `member_input_operations` | Durable submission receipts, delivery method/deferral metadata, payload digests and settled replay protection |
| `member_permission_responses` | Immutable attributed decisions and question choices tied to input/event/interaction identity |
| `member_permission_response_claims` | One possibly transmitted answer per observed episode |
| `asset_installs` | Installed asset versions by backend and config path |
| `refinery_schema_history` | Baseline ledger |

Exact baseline DDL:

```sql
PRAGMA application_id = 1128351289;

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
    mux_server TEXT NOT NULL,
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
    event_evidence TEXT NOT NULL CHECK(event_evidence IN ('metadata','redacted')),
    coverage TEXT NOT NULL CHECK(coverage IN ('tested','unknown')),
    status TEXT NOT NULL CHECK(status IN ('starting','prepared','running','degraded','stopping','recovering','exited','failed','lost')),
    created_at TEXT NOT NULL,
    native_version TEXT,
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
    native_session_json TEXT CHECK(native_session_json IS NULL OR (json_valid(native_session_json) AND json_type(native_session_json) = 'object')),
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
    kind TEXT NOT NULL CHECK(kind IN ('coverage_unknown','interaction_opened','interaction_obscured','interaction_closed','permission_denied_observed','automatic_refusal_observed','automatic_review_unavailable','input_delivery_result','runner_degraded','runner_lost','runner_exited')),
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
    kind TEXT NOT NULL CHECK(kind IN ('preview','prompt','exec','permission_response')),
    payload_digest TEXT NOT NULL CHECK(length(payload_digest) = 64 AND payload_digest NOT GLOB '*[^0-9a-f]*'),
    state TEXT NOT NULL CHECK(state IN ('ready','in_progress','accepted','uncertain','settled')),
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    message_id INTEGER REFERENCES messages(message_id) ON DELETE RESTRICT,
    disposition TEXT CHECK(disposition IS NULL OR disposition IN ('submitted','not_submitted','abandoned_uncertain')),
    evidence_ref TEXT,
    delivery_method TEXT CHECK(delivery_method IS NULL OR delivery_method IN ('profile','fallback')),
    delivery_reason TEXT CHECK(delivery_reason IS NULL OR length(trim(delivery_reason)) > 0),
    deferred_at TEXT,
    defer_reason TEXT CHECK((deferred_at IS NULL AND defer_reason IS NULL)
        OR (deferred_at IS NOT NULL AND defer_reason IS NOT NULL
            AND kind IN ('preview','prompt','exec'))),
    uncertain_cause TEXT CHECK(uncertain_cause IS NULL
        OR (uncertain_cause IN ('manual_input','manual_accounting_failed','interaction_candidate',
                                'interaction_open','observer_unavailable','short_write','write_failed',
                                'runner_stopped')
            AND state IN ('uncertain','settled'))),
    UNIQUE(run_id, operation_key),
    CHECK(kind != 'preview' OR message_id IS NOT NULL),
    CHECK((state = 'settled' AND disposition IS NOT NULL) OR (state != 'settled' AND disposition IS NULL))
);

CREATE TABLE member_permission_responses (
    input_id INTEGER PRIMARY KEY REFERENCES member_input_operations(input_id) ON DELETE RESTRICT,
    event_id INTEGER NOT NULL REFERENCES member_runtime_events(event_id) ON DELETE RESTRICT,
    interaction_id INTEGER NOT NULL REFERENCES member_interactions(interaction_id) ON DELETE RESTRICT,
    expected_owner_epoch INTEGER NOT NULL CHECK(expected_owner_epoch > 0),
    request_token TEXT NOT NULL CHECK(length(request_token) = 64 AND request_token NOT GLOB '*[^0-9a-f]*'),
    decision TEXT CHECK(decision IS NULL OR decision IN ('allow','deny')),
    choice_id TEXT CHECK(choice_id IS NULL OR (length(choice_id) BETWEEN 1 AND 64
        AND choice_id NOT GLOB '*[^a-z0-9-]*')),
    director_member_id INTEGER NOT NULL REFERENCES members(member_id) ON DELETE RESTRICT,
    director_mode TEXT NOT NULL CHECK(director_mode IN ('auto','manual','unknown')),
    decision_source TEXT NOT NULL CHECK(decision_source IN ('director-auto','user')),
    mode_evidence_ref TEXT NOT NULL CHECK(length(CAST(mode_evidence_ref AS BLOB)) BETWEEN 1 AND 2048),
    decision_evidence_ref TEXT NOT NULL CHECK(length(CAST(decision_evidence_ref AS BLOB)) BETWEEN 1 AND 2048),
    answer_rule_id TEXT NOT NULL CHECK(length(trim(answer_rule_id)) > 0),
    decided_at TEXT NOT NULL CHECK(length(trim(decided_at)) > 0),
    UNIQUE(interaction_id, input_id),
    CHECK(decision_source != 'director-auto' OR director_mode = 'auto'),
    CHECK((decision IS NULL) != (choice_id IS NULL))
);

CREATE TABLE member_permission_response_claims (
    interaction_id INTEGER PRIMARY KEY REFERENCES member_interactions(interaction_id) ON DELETE RESTRICT,
    input_id INTEGER NOT NULL UNIQUE REFERENCES member_input_operations(input_id) ON DELETE RESTRICT,
    FOREIGN KEY(interaction_id, input_id)
        REFERENCES member_permission_responses(interaction_id, input_id) ON DELETE RESTRICT
);
```

Baseline column semantics:

| Table | Semantics |
|---|---|
| `fleets` | `director_member_id` is backfilled inside the bootstrap transaction. A live fleet has null `deleted_at` and an active valid Director with placement |
| `members` | `status` decodes as `active` or `deregistered`; unknown values fail. Deregistration is soft and keeps the row |
| `member_placements` | `mux_pane_id` is null while member creation is pending. `mux_server` is the server every invocation for the fleet targets (§6.5 Multiplexer context); member placements copy the Director's |
| `messages` | `status_state` decodes as `input_required` or `completed`. `notified_at` is internal preview state that the public message record omits; a self-send stamps it on insertion and stays pending for reception and explicit ACK |
| `asset_installs` | Each successful backend asset installation upserts its CLI version and timestamp at `(coding_agent, path)`, independently of member registration |

A concrete placement selects the shared pane input lock, and duplicate entries targeting the same backend/session/pane share it. Inbox streaming ignores `notified_at`, and native handles and emitted-ID tracking belong to the receiver process with no durable consumer state.

Runtime records keep their run/member/fleet/Director references after soft deregistration, so history survives member and fleet closure.

The binary appends `INSERT INTO cafleet_database(singleton_id, cafleet_version) VALUES(1, '<VERSION>');` to the baseline SQL, with `<VERSION>` its package version, and applies both in one transaction, so a failed initialization leaves no identity, table, ledger or version row.

`native_session_json` records a `hermes` run's native session identity for work settlement (§6.7 Hermes work settlement) and is null for every other backend and for a run whose identity is not yet captured.

The DDL above owns every runtime table column, type, enum, nullability constraint, foreign key, uniqueness invariant and partial unique index. Section 6.9 specifies their runtime semantics, including database-instance identity, settled receipt retention, transactional same-run/event/Director response checks, privacy, zero-byte claim release and immutable audit semantics.

## 9. Testing strategy

The test suite covers current schema/identity and incompatible-file preservation; atomic registration and creation compensation; point-to-point/ACK/pending notification behavior; finite receive deadlines, round-robin retries and diagnostic clearing; process-shared exclusion, bounded lock waits and process-death release; minimum-version profile selection, staged profile submission with its settle interval, terminal-reply classification of outer-terminal bytes and its pending-prefix bound, uncertain-cause recording at every runner uncertain transition, delivery fallback, runner-owned queueing and delivery reporting for wrapped members; budget exhaustion, zero automatic Director input and no pane capture on any input path; stream ordering/replay, exact bodies, isolation, read-only state, ACK races, scoped SQL/collection deadlines and handler cleanup, bounded partial/broken output and native receiver lifecycle; termination deferral across pane-input spans; literal native command visibility and partial uncertainty; explicit continuation; backend/multiplexer smoke coverage; links, embedding and release builds; and general timed pipe draining and owned-child cleanup.
