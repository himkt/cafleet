# CLI options

The unified `cafleet` CLI uses literal subject IDs, synchronous SQLite access and explicit pane actions. Run each command as one isolated shell-tool invocation.

## Subcommand summary

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

## Environment variables

| Variable | Field | Default | Meaning |
|---|---|---|---|
| `CAFLEET_DATABASE_URL` | `database_url` | `sqlite:///` + `~/.local/share/cafleet/cafleet_v8.db` | Selected SQLite file path |
| `CAFLEET_MULTIPLEXER` | `multiplexer` | Auto-detect | Explicit supported multiplexer override |
| `CAFLEET_MAX_TEXT_LEN` | `max_text_len` | `200` | CLI text-output body codepoint limit |

The default expands HOME once. An explicit URL has the form `sqlite:///<path>`; resolve a relative file path against the invoking working directory before canonicalizing the database file for shared pane-lock identity. A literal tilde in an override is not expanded. Count values accept nonnegative integers and invalid values fail with `<NAME> must be a non-negative integer (got '<value>')`.

## Global Options

`cafleet --version` prints `cafleet <version>` and exits 0 before dispatch.

## Help text {#help-text}

`--help` describes every visible command, subcommand and option with a one-line description. Command group descriptions state the purpose given in the [subcommand summary](#subcommand-summary). Help derives choice lists from the same registries that validate the values:

| Option | Help shows | Source |
|---|---|---|
| `member create --permission-mode MODE` | Each backend's accepted modes and its omitted default, in backend registry order, as `BACKEND: MODE, MODE (default MODE)` joined by `; ` | The backend [policy catalog](member-runtime.md#launch-policy-and-capability-registry) |
| `permission respond --decision`, `--decision-source`, `--director-mode` | Their possible values | The stored enum values of the [permission response storage](member-runtime.md#permission-response-storage) |
| `input reconcile --disposition`, `member create --event-evidence` | Their possible values | Their command enums |

An invalid value for an option with possible values exits 2 with a usage error that lists those values. `member create` resolves its backend at run time, so a `--permission-mode` value outside that backend's catalog keeps its run-time usage error `--permission-mode for BACKEND must be one of CHOICES (got 'VALUE')`, also with exit 2. For opencode, help reads `opencode: cafleet, cafleet-ask (default cafleet)` and the error reads `--permission-mode for opencode must be one of cafleet, cafleet-ask (got 'VALUE')`.

## Output shapes {#output-shapes}

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

### Member list output {#member-list-output}

Roster records include `member_id`, `name`, `kind`, `placement`, `last_sent`, `last_recv`, `last_ack` and `idle`. `kind` is `director` when the fleet reference matches, otherwise `member`. Missing activity is null; placement may be null, or its pane ID pending. Text uses `-` for absent values and `(pending)` for a pending pane.

## JSON output (`--json`) {#json-output}

Place `--json` among the subcommand options, before a `--` separator that introduces literal key arguments. It emits compact single-line JSON with UTF-8 and complete untruncated message text. Application errors remain stderr text, with the [structured permission-response exception](#permission-commands); parse errors exit 2. Commands marked no reject the flag, including the root group.

## Positional subject ids {#positional-subject-ids}

Place the command’s subject ID immediately after its subcommand; event, input and permission subjects use their positive durable event, input and interaction IDs. The subject row supplies related fleet/recipient state. IDs are complete DB-assigned integers, without prefix resolution or environment defaults. A missing or malformed subject uses the parser's native error, exit 2.

Spawned members read literal `FLEET ID:`, `YOUR MEMBER ID:`, `DIRECTOR MEMBER ID:` and `CODING AGENT:` lines. Pass these literal IDs on each invocation.

## Sender and recipient (`--from-member-id`, `--to-member-id`) {#from-to-member-id}

Send requires integer sender and recipient flags. Both must be active in the same live fleet. The sender row determines the fleet; IDs are coordination identifiers within the local trust model.

## `permissions.allow` coverage

Use prefix rules such as `Bash(cafleet message *)` to cover broker operations, including `cafleet message listen ID --timeout 300` and finite `cafleet message wait`. Verify native Monitor authorization separately: a Bash-rule check establishes no Monitor permission. Director capture is covered by `Bash(cafleet fleet capture *)` or a broader authorized prefix. Literal IDs and trailing JSON flags preserve prefix matching.

Keep commands that send explicit native pane input under `permissions.ask`, including the `Bash(cafleet member prompt *)`, `Bash(cafleet member exec *)` and `Bash(cafleet member press *)` prefixes. Ask rules outrank allow rules. Authorize `cafleet permission respond` under the Director host’s effective policy after its request-specific decision; mode/source flags bypass no ask, deny or managed rule. Ordinary member role authority covers broker reception and evidence routing, while the Director owns permission responses. The [permission profiles](coding-agent-backends.md#claude-profile-entries) describe each role's needs.

## Message Body Truncation

CLI text bodies use `CAFLEET_MAX_TEXT_LEN` Unicode codepoints followed by `…` when truncated. Full JSON and pane captures preserve their complete content. Pane notification lines follow their own [inline-body limit](multiplexer-backends.md#push-notifications). [Message envelope](message-envelope.md#rendered-shape) owns the compact rendering.

## Text bodies (positional `TEXT` / `--file`) {#text-body-input}

Send and member creation require exactly one positional body or `--file PATH`; `--file -` reads UTF-8 stdin. Preserve exact nonblank body text. Missing/both sources or a blank positional body exit 2; blank file/stdin sources and file-read errors exit 1 with the authored diagnostics below. File read/UTF-8 failures retain the actual cause. Compact role-by-path spawn prompts remain necessary because resolved prompts enter backend/multiplexer argv.

`member prompt` takes one positional line. Native exec has the [separate command contract](#member-exec).

## `cafleet setup` — Onboarding and Schema Management {#cafleet-setup}

`cafleet setup [--coding-agent AGENT...]` validates the selected database format before either half writes. Initialize an empty zero-identity file with the embedded baseline in one transaction; migrate known current-family versions. An ahead ledger version or incompatible shape rejects both setup write paths before database mutation, asset installation or asset-install recording. Independent half-error reporting applies after this compatibility gate. A format failure prevents asset installation and content writes. Compatible setup installs embedded assets offline and records versions. Backend selection is multi-value/repeatable, deduplicated and defaults to the supported backend registry.

The database half prints `Created PATH and applied migrations to head (N).`, `Upgraded from M to N.` or `Already at head (N); nothing to do.` according to state. Report actual errors as `db half failed: CAUSE` or `assets half failed: CAUSE`; failed halves exit 1. Assets require a compatible usable current schema.

### Config-dir resolution {#config-dir-resolution}

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


### Assets half {#assets-half}

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


## Shared diagnosis and connection reuse {#diagnosis-reuse}

Use the invocation’s shared classifier and connection for guard/setup work. Doctor completes read-only sections even when the database is unavailable. Compatibility includes identity, ledger version and expected table/column shapes.

## Schema-version guard {#schema-version-guard}

The fresh baseline creates exactly the application tables declared in the schema. Compatibility diagnosis verifies current application identity, ledger version, every required table’s column shape, and required indexes/foreign keys. Known versions use their recorded migration shape; newer versions are diagnosed before any head-shape comparison. Unrelated auxiliary tables, indexes and triggers are compatible while those required definitions remain intact.

Every fleet/member/message command validates current format before writes and asset checks. Identity is `application_id=1128351288`; malformed or incompatible files fail with `database format is incompatible at 'PATH'; choose an empty database path and run 'cafleet setup'.` Preserve file bytes and existing sidecars. Empty state reports `no cafleet database; run 'cafleet setup'`. Behind state reports `database schema is outdated (schema M, head N); run 'cafleet setup'`; ahead reports `database schema M is newer than this cafleet (head N); upgrade cafleet`. Database-open/read errors preserve their cause. Setup initializes/repairs compatible known versions; doctor reports rather than blocks its other diagnostics.

## Stale-assets guard {#stale-assets-guard}

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


## `cafleet fleet` — Fleet Management

### `fleet create`

`cafleet fleet create --name NAME --coding-agent AGENT [--json]`

Require the selected backend token and a valid tmux/herdr context. Outside that context fail with `cafleet fleet create must be run inside a tmux or herdr session`. Insert fleet, current Director member and placement in one transaction and backfill `director_member_id` before commit. The Director has `name="Director"` and `description="Root Director for this fleet"`. Creation splits no pane and launches no process. An ordinary member can be created immediately.

The calling pane becomes the Director's concrete placement. Run `fleet create` from the coding-agent session declared by `--coding-agent`, then arm native [message reception](#message-listen) before depending on member replies. Root-Director messages remain in the durable inbox for native or manual reads; automatic pane previews target ordinary members.

### `fleet list`

List non-deleted fleets by descending creation timestamp/fleet ID. Rows retain the Director reference so its ID remains discoverable.

### `fleet show`

`cafleet fleet show FLEET_ID [--json]` includes soft-deleted rows. Unknown ID: `fleet 'X' not found.`, exit 1.

### `fleet delete`

`cafleet fleet delete FLEET_ID [--json]` locks active concrete placements in sorted key order, revalidates the placement set, then stamps deletion and deregisters active members/removes placements in one transaction. It retains messages and is idempotent. Close owned member panes individually first.

One operation-wide five-second deadline bounds lock acquisition, with 50-millisecond retries and set-change restart. Contention leaves state unchanged and exits 1 with `fleet ID pane input remained busy for 5 seconds at member MEMBER_ID; fleet was not deleted.`

### `fleet capture`

`cafleet fleet capture FLEET_ID [--lines N] [--ansi] [--json]` checks fleet existence and soft-deletion before resolving the multiplexer. An absent or deleted fleet exits 1 with `Error: fleet ID not found`. Capture reads the current active roster once, Director first then placement-owning members by ascending ID. The default depth is 20. Per-pane failures, including a pending Director placement, annotate entries and leave the rest available. It writes no broker state.

Text sections begin `=== ID (NAME; kind=KIND; coding_agent=AGENT; pane=PANE; captured_at=TS) ===`, separated by a blank line. A failed/pending pane omits capture time and carries its diagnostic.

JSON entries are ordered `member_id, name, kind, coding_agent, pane_id, lines, content, captured_at, content_sha256, error`. Kind is director/member. On error, content/time/hash are null; pane_id is null only without a concrete pane. Successful hash/timestamp semantics match [member capture](#member-capture).

## `cafleet doctor` — Environment Diagnosis {#cafleet-doctor}

A full-environment diagnosis that renders **all** sections even when the
multiplexer is unavailable or the database is missing or stale — no early
abort. Diagnosis order: multiplexer, database, coding agents, member
permissions. `doctor` is
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
✓ member permissions
  claude: no setting blocks the member broker commands
1 issue found
```

### Multiplexer section

`✓` with the five detail lines (`backend`, `session`, `window_id`,
`pane_id`, `presence`). On any multiplexer or environment failure (no
supported multiplexer, ambiguous environment, binary not on `PATH`, pane not
discoverable): `✗ multiplexer` with the resolver's error message as the
single detail line, and the report continues. One issue.

### Database section

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

#### OpenCode preset checks {#doctor-opencode-preset}

`doctor` has no selected launch policy, so it checks the preset of every
OpenCode [launch-policy choice](member-runtime.md#launch-policy-and-capability-registry),
in catalog order. The default choice keeps the check name `opencode.preset`;
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

### Member permissions section {#member-permissions}

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
[Spawn-time allow rules](coding-agent-backends.md#spawn-time-allow-rules).

### Member runtime checks {#doctor-member-runtime}

With a compatible at-head database, doctor diagnoses every recorded member run in ascending `run_id` order without mutating registry or runtime state. Each run produces one entry named `member_runtime.<run_id>`:

| Outcome | Status | Detail |
|---|---|---|
| The read-only diagnosis completes | `info` | The run's diagnostics joined by `; `, such as a leftover launch descriptor, the owner-lock observation and the health-probe result |
| The diagnosis fails | `error` | The diagnosis cause; it counts as an issue |

One five-second budget covers all runs, and each run's diagnosis is bounded by the smaller of 100 ms and the remaining budget. When the budget is spent, doctor adds one `member_runtime` entry with status `info` and detail `runtime diagnosis budget exhausted; inspect remaining runs explicitly`, and skips the remaining runs. A database that is not at head produces no runtime entries. The [recovery contract](member-runtime.md#storage-pressure-and-durability-boundary) owns the probes and their recovery actions.

### Footer and exit code

Last line: `no issues found`, `1 issue found`, or `<N> issues found` (proper
pluralization). Exit code: 0 when no issues, 1 otherwise — the `–` state and
footnotes never count. Every failure is a rendered issue; no failure exits
before output.

### `--json`

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


## `cafleet message` — Message Broker

### `message send`

`cafleet message send --from-member-id ID --to-member-id ID (TEXT | --file PATH) [--json]`

Resolve the complete text body before starting the five-second persistence/notification budget, including when `--file -` waits for stdin. Persist one point-to-point delivery, then drain an ordinary recipient's pending messages oldest-first into its concrete pane within the remaining budget, the new message included. Root-Director messages use native or manual broker reads: sends succeed with `notification_sent=false`, no pane-delivery diagnostic and no multiplexer requirement. Self-sends persist with no pane input. A committed/persisted send remains successful with exit 0; held/unavailable ordinary-member notifications preserve the stdout record and `notification_sent=false`.

`notification_sent` is `true` exactly when this invocation delivered the newly committed message, including when it was drained after an older one. When another invocation delivered the new message, `notification_sent` stays `false` and stderr carries no diagnostic; the recipient still received the line. Messages left pending when the budget no longer covers another submission stay pending with no diagnostic; the next send to that recipient or a [message wait](#message-wait) rotation retries them. [Push notifications](multiplexer-backends.md#push-notifications) specifies the line, the drain and under-lock revalidation.

#### Delivery outcome {#message-send-delivery}

A notification failure prints `Message NEW_ID was persisted. ` followed by its diagnostic on stderr, without suppressing successful stdout. Every per-attempt failure uses this diagnostic:

```text
message MESSAGE_ID was not notified to member MEMBER_ID: CAUSE; the next send to member MEMBER_ID or the next message wait retries it.
```

| Failure | `CAUSE` |
|---|---|
| The pane input lock stays busy through the [lock wait](multiplexer-backends.md#pane-input-lock) | `pane input remained busy` |
| Lock acquisition error | `pane input: <lock error>` |
| Multiplexer resolution or availability error | The multiplexer error text |
| Budget cannot cover literal input, settle and Enter before the first line | `insufficient pane input budget for literal input, settle and Enter` |
| Transport error before or during typing | The transport error text |
| Database busy or deadline exceeded during the under-lock snapshot or notification stamp | The storage error text; names the current candidate, or the candidate from placement revalidation when selection has not completed |
| `message wait` attempt fails with database busy or deadline exceeded before selecting a candidate | The storage error text; names the selected rotation entry |

A lock wait that ends after another sender delivered every pending message reports nothing. Other failures, including corrupt, incompatible or unreadable storage and a termination-span start failure, retain their causes after the persisted prefix. Storage failure before commit exits 1 with its cause. Consume an existing committed ID once; notification failure supplies no reason to create another delivery.

After a failed delivery the sender sends exactly one follow-up to the same recipient with the body `message N was not delivered to your pane; poll your inbox`. Its drain retries `N` first. When the follow-up also fails, the sender stops.

### `message poll`

`cafleet message poll MEMBER_ID [--json]` returns only incoming `input_required` records, descending by status timestamp/message ID. Unknown or inactive member: `Member ID not found`, exit 1.

### `message wait`

`cafleet message wait MEMBER_ID [--timeout SECONDS] [--json]`

Timeout is an integer in `1..=1800`, default `300`. Validate an active subject and live fleet. Existing unread deliveries return immediately; otherwise check at one-second monotonic intervals. The invocation is a blocking receive for scripts and manual use and never ACKs messages.

Between inbox checks, retry pending deliveries for at most one ordinary member per interval with an in-memory round-robin cursor. Select concrete ordinary-member placements in the fleet, excluding its root Director, the wait's own subject, self-notifications, ACKed and already-notified rows. Each attempt drains that member's pending messages under the [delivery procedure](multiplexer-backends.md#push-notifications) without consulting pane state. One invocation attempts each pending message at most once; a failed message is retried by a later send or wait invocation, while later messages for the same member remain eligible. Recheck the inbox before starting another attempt. Shared input locks and under-lock revalidation keep multiple waiters safe.

Start each receive check or notification attempt before the monotonic receive deadline. An attempt already in flight retains its original five-second budget for the lock wait, revalidation, Escape, typing, settle and Enter. A busy pane lock makes the attempt wait up to 1.5 seconds; a lock still busy then records the [not-notified diagnostic](#message-send-delivery) in `notification_errors`. Cap subprocess/database busy waits to its remaining budget and start input only with time reserved for settle and Enter. Mandatory database finalization uses the later of the original deadline and one second after finalization begins, keeping pane transport within its original budget. A receive returns by its deadline plus at most one six-second in-flight attempt/finalization allowance. A snapshot blocked until receive expiry returns `timeout` with retained notification diagnostics; earlier transient contention can retry before expiry. Corruption and other fatal storage errors retain their causes.

The ordered JSON result is:

```json
{"member_id":41,"outcome":"timeout","messages":[],"notification_errors":[]}
```

`outcome` is `messages` with nonempty full poll records, or `timeout` with an empty array. Each diagnostic has ordered keys `{member_id, message_id, error}`. Keep one diagnostic per not-notified message, ordered by message ID.

Pending placement and a placement change under the lock are ordinary holds that keep entries unchanged. A drain that notifies a recorded message removes that message's entry. Reconcile diagnostics with each successful receive snapshot and the typed observations of its owned notification attempt. Remove entries when these observations show that message ACKed, its recipient gone or the message notified by any invocation. After receive expiry, return the last successful snapshot reconciled with the completed attempt's observations. Subsequent external changes become visible to the next invocation; final reconciliation starts no additional I/O or tail budget.

Text is `Messages available for member ID.` plus normal messages, or `Timed out waiting for member ID.`; diagnostics go to stderr. Both outcomes exit 0. Deleted subject/fleet or corrupt/unreadable state exits 1 with its cause and retains earlier notification diagnostics on stderr, including when JSON output was requested. Ctrl+C outside a delivery attempt terminates the process normally; inside an attempt it takes effect when the attempt's [termination-deferral span](multiplexer-backends.md#termination-during-pane-input) ends.

A script or operator uses `message wait` for finite manual reception and ordinary-member retry recovery, then polls and ACKs consumed messages. Native Director reception retains the existing listener process between events.

### `message listen`

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

The inner message is exactly the [full message record](message-envelope.md#persisted-shape), preserving original text beyond CLI/pane limits. `closed.reason` is `timeout`, `member_inactive` or `fleet_deleted`; fleet deletion takes precedence when both lifecycle closures are observed. Check normal closures before revalidating a continuing live fleet. Missing persisted rows, changed fleet identity, invalid stored enums/root relationships or missing root placement are fatal errors.

Each short read transaction validates identity and selects every `input_required` record for the pinned subject ordered by `(created_at ASC, message_id ASC)`. Commit before output or sleep. A later commit with an earlier timestamp emits when first observed. A process-local ID set suppresses already-emitted pending IDs; insert only after the complete record and LF flush successfully. Retain only IDs still pending in successful snapshots; a failed snapshot leaves the set unchanged. A snapshot containing only previously emitted pending IDs waits one second, while newly emitted records permit immediate recheck.

Listening leaves consumption and `notified_at` unchanged and performs no pane retry, lock or dispatch. Stamped pending self-sends remain stream-eligible. Concurrent listeners are independent read-only observers with no consumer lock; restart replays all pending IDs. ACK-before-snapshot removes a message, while ACK racing with buffered output may leave a stale pending envelope. Reconcile known/replayed IDs with `poll --json` or `show --json` before acting, record deferred actions and explicitly ACK consumption. Completed records need no second action or ACK.

Each snapshot has a fresh database deadline at most five seconds away, shortened by the remaining lease. Scoped busy waiting and an execution progress handler every 1,000 VM instructions bound lock/SQL work. Check that deadline before/after prepare and execution, before/after decoding each row and before commit. Expiry discards partial snapshots; interrupted SQL maps to deadline exceeded only when that deadline expired. Restore busy handling and remove the listener-owned progress handler on every path, preserving cleanup failures alongside the original cause. These cooperative boundaries include row collection without claiming hard real-time preemption of one allocation or SQL operation.

Output owns one serialized record at a time and applies backpressure before selecting more messages. Nonblocking filesystem/poll output preserves/restores descriptor flags on normal return, resumes partial writes from their byte offset and checks every write/flush. Its five-second deadline is shortened by the lease. Broken pipe, write-zero, flush failure or output deadline exhaustion terminates visibly with exit 1, even after partial records. Normal lease expiry between records uses a separate bounded final write for `closed`. Complete records survive a later error; an incomplete final line remains unconsumed and recovers through broker replay. Only the current snapshot, one record and pending emitted IDs occupy listener memory.

Database contention retries stay inside the snapshot budget; exhaustion or another fatal error terminates the stream. The listener retains its connection/PID across healthy events and ordinary signal behavior outside pane termination deferral. Host cancellation may end it without `closed`. Native owners retain one handle, reconcile durable state after failure and establish old-process termination before replacement. The [manual lifecycle](../how-to/mixed-backend-team.md#manual-lifecycle) explains operation; installed backend instructions own concrete native tool recipes and their evidence limits.

### `message ack`

`cafleet message ack MESSAGE_ID [--json]` transitions an existing unread message to completed, updating its status timestamp. The recipient/fleet derive from that row. ACK signifies consumption.

### `message show`

`cafleet message show MESSAGE_ID [--json]` returns one full record. Existence is the read guard.

## `cafleet member` — Member Lifecycle + Pane Interaction {#cafleet-member}

### Member targeting and key delivery

Require an active member and placement for pane operations; concrete pane IDs remain opaque. Pending placement is valid during creation and yields contextual prompt/exec/capture errors. Missing placement fails pane actions; list/show retain diagnostic nulls. Prompt and exec reject the root Director before pane I/O; individual member deletion protects it.

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
| `--coding-agent` | no | One of `claude`, `codex`, or `opencode`; when omitted, the member — every role — inherits the spawning Director's placement backend. Exits 1 with `Error: binary <name> not found on PATH` when the binary is missing. The opencode backend additionally requires the selected launch policy's agent preset, `<preset base>/agents/<mode>.md`, at the resolved preset path (via `OPENCODE_CONFIG_DIR` — see [Config-dir resolution](#config-dir-resolution)); a missing or unreadable selected preset or an invalid variable exits 1 per [Error Messages](#error-messages). |
| `--model` | no | Model forwarded to the backend binary's own `--model` flag. The opencode backend additionally requires `<provider-id>/<model-id>`; per-backend formats and create-time validation are in [Model selection](coding-agent-backends.md#model-selection). |
| `--permission-mode MODE` | no | Launch-only selection from the resolved backend’s [policy catalog](member-runtime.md#launch-policy-and-capability-registry); omission preserves its default. For opencode the choices are `cafleet` (default) and `cafleet-ask`, each selecting the agent preset of that name. |
| `--event-evidence POLICY` | no | `metadata` by default, or explicit `redacted` evidence under the [privacy contract](member-runtime.md#persistence-notification-and-read-interfaces). |
| `--event-redaction-file PATH` | for redacted | Absolute readable UTF-8 redaction configuration; invalid combinations/configuration fail with usage exit 2 before registration. |
| `--effort` | no | Reasoning-effort level forwarded to the backend binary, validated per backend before any side effect. Accepted levels, forwarding forms, and rejection strings are in [Reasoning effort](coding-agent-backends.md#reasoning-effort). |
| positional `PROMPT` | one of | Inline spawn prompt (backend-neutral template). Exactly one of `PROMPT` / `--file`. |
| `--file PATH` | one of | Path to a UTF-8 file whose contents are the spawn prompt (`-` = stdin). Follow the shared [body-input and transport contract](#text-body-input). |
| `--json` | no | Output as JSON |



#### Spawn command per backend

The per-backend child argv and omitted permission defaults live in
[Coding-agent backends](coding-agent-backends.md#spawn-argv).

#### Spawn-prompt substitution

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

#### Creation failure compensation {#creation-failure-compensation}

Member creation validates policy, evidence configuration and assignment, and records the installed native version through the [native version probe](member-runtime.md#native-version-probe), then registers a pending placement and run. A failed probe adds the stderr line `Warning: native version unavailable for BACKEND: CAUSE; coverage unknown.` and creation continues with unknown coverage. The multiplexer starts a runner from a private descriptor; a `prepared` handshake precedes placement publication and `start`, and creation returns only after child spawn produces `started` within one handshake deadline. The [runner creation contract](member-runtime.md#runner-boundary-and-creation-transaction) owns descriptor cleanup and uncertain-launch compensation. The CLI owns a registration guard after successful
registration and acquires a pane guard only when `split_window` succeeds.
The backend owns the pane before that return; see
[pane ownership](multiplexer-backends.md#pane-creation-ownership).

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



### `member delete` {#member-delete}

For wrapped members, request owned runner shutdown under the recorded identity before pane/registry cleanup, retaining explicit unconfirmed-child diagnostics and recoverable runtime records. Attempt remaining event replay before deregistration only within the remaining operation deadline. Lock the concrete placement before kill/deregistration. A member without a concrete pane needs no lock. Retry contention every 50 milliseconds against one five-second deadline; expiry exits 1 with `member ID's pane input remained busy for 5 seconds; member was not deleted.` and leaves state unchanged. Preserve pane/registry errors and layout cleanup ownership.

Revalidate pending or absent placement and deregister atomically. For a concrete pane, retain input ownership while closing it outside a SQLite write transaction, then revalidate and finalize the registry. After successful close, mandatory registry finalization uses the later of the original deadline and one second after finalization begins, allowing short reader/writer contention to clear; actual database failures report that the pane was closed and preserve the database cause. Optional layout work follows mandatory finalization within the original deadline. A failed or expired restoration emits a stderr diagnostic while preserving successful deregistration.

### `member show` {#member-show}

`cafleet member show MEMBER_ID [--json]` — the positional `MEMBER_ID` names
any active registry entry — placed or placementless (root Director included).

Registry read — no multiplexer requirement. Text is the compact one-line row;
the detailed view — including `kind` ([`director` / `member`](data-model.md#members)) and the
placement sub-dict and appended runtime summary — is the `--json` payload (see
[Runtime presentation](#runtime-presentation)). A wrapped result appends the runtime text line; an unwrapped result retains its existing row.


### `member list` {#member-list}

`cafleet member list FLEET_ID [--json]` — the positional `FLEET_ID` names the
fleet whose roster is listed; no identity flag. Lists every **active**
registry entry of the fleet —
the root Director, ordinary members, and placementless rows. An empty roster
prints `0 members.`.

Field projection and absent values are in [Member list output](#member-list-output).

Rows and activity follow the [data-model query contract](data-model.md#query-and-activity-contracts).
Text idle values render as `Ns` / `Nm` / `Nh`.
Per-member detail such as `description` and `registered_at` lives on
[`member show`](#member-show).


### `member prompt` {#member-prompt}

`cafleet member prompt MEMBER_ID TEXT [--json]` validates a nonblank line without CR/LF or C0/DEL controls, otherwise exits 2 with `text must be one non-empty line without control characters.`

A root Director target exits 1 with `cannot prompt the Director's own pane` before any pane I/O, because Escape would interrupt the Director's own running turn. Otherwise acquire the shared input lock within its [bounded wait](multiplexer-backends.md#pane-input-lock) and revalidate activity/placement. For a wrapped member, submit literal text and the native submit key through the verified runner composer profile, gated by the [input arbitration contract](member-runtime.md#input-arbitration-and-the-original-native-answer). Explicitly unwrapped members retain Escape, settle, literal text, settle and Enter; that legacy path can interrupt work or dismiss a prompt. Exit 0 means submission only, with ordered `{member_id, pane_id, text, input_id}`; unwrapped input has a null ID.

A lock still busy when the wait ends uses `member ID's pane input remained busy; prompt was not dispatched.` Pre-write error, including an Escape failure: `prompt was not dispatched to member ID: CAUSE`. Partial uncertainty: `prompt delivery to member ID is uncertain: CAUSE. Inspect the pane before retrying.` These exit 1 without a success record.

### `member exec` {#member-exec}

`cafleet member exec MEMBER_ID COMMAND [--json]` or `cafleet member exec MEMBER_ID --file PATH [--json]`; `--file -` reads stdin. Exactly one source is required. File/stdin is UTF-8 and strips at most one terminal LF or CRLF. Preserve inline bytes exactly.

Require a nonblank line without CR/LF, NUL or other C0/DEL controls, including tabs. Invalid input exits 2 before pane I/O with `command must be one non-empty line without control characters.` A multiline task supplies a one-line explicit script invocation.

Resolve an active ordinary member and concrete placement, reject the root Director, lock input within its [bounded wait](multiplexer-backends.md#pane-input-lock) and reload. Wrapped input uses the tested native-shell composer profile, literal `! ` plus the command, and the native submit key through the runner. Explicitly unwrapped input retains Escape, settle, literal native shell input, settle and Enter. The backend owns working directory, environment, permissions, output and exit rendering.

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

### `member press` {#member-press}

`cafleet member press MEMBER_ID KEY... [--json]` requests only the specified ordered keys. Require a positive signed 64-bit member ID and one through 64 keys, each at most 32 UTF-8 bytes, using the [common key grammar and backend mappings](multiplexer-backends.md#explicit-key-dispatch). Preserve literal character case and render named keys canonically. Validate the complete vector before any backend probe or writing invocation.

Use the ordinary `--` option separator before literal punctuation that could be read as an option; put `--json` before that separator. For example, `cafleet member press 42 -- '-' '+' ';'` requests three keys. It supplies no Escape prelude, Enter suffix, prompt text or shell-command submission.

Resolve an active ordinary member and its concrete placement, preserving the Director-target restriction and the [shared input lock, selected-context validation and original budget](multiplexer-backends.md#explicit-key-dispatch). Wrapped members receive the request through their outer pane as [manual native input](member-runtime.md#explicit-manual-key-input). Success means backend acknowledgment of this ordered request; actual PTY bytes, native approval and completion require their own observation.

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

### `member capture` {#member-capture}

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


## Runtime presentation {#runtime-presentation}

Member create preserves ordered `member_id,name,registered_at,placement` and appends `runtime`; member show preserves ordered `member_id,name,description,status,registered_at,kind,placement` and appends `runtime`. An explicitly unwrapped member has `runtime: null`. The runtime object has ordered keys `run_id,resolved_mode,status,reachability,owner_epoch,last_health_at,coverage,current_interaction_id,unresolved_input_ids,owner_kind,owner_pid,owner_start_identity,runner_pid,runner_start_identity,child_pid,child_start_identity`, preserving stored nullability. `unresolved_input_ids` lists the run's inputs in `in_progress`, `accepted` or `uncertain`, in input-ID order; an accepted input needs no reconciliation and stays listed until a settlement records its disposition, while `in_progress` and `uncertain` inputs need Director reconciliation. These are registry reads and make no live-health claim.

Wrapped create/show preserves its existing text row and appends `Runtime: run=RUN_ID mode=MODE status=STATUS coverage=COVERAGE`. Wrapped prompt/exec preserves its existing success line and appends `Input: INPUT_ID` when a durable ID exists; JSON appends nullable `input_id` after existing keys. An uncertain error retains its existing exit category and includes known receipt identity in the cause as `input_id=INPUT_ID`; run/interaction context accompanies held/rejected input.

## Runtime event and input commands

`cafleet event show EVENT_ID [--json]` returns ordered `event,notification_state,message_id,archive_reason,run,interaction`. The event is the complete [versioned runtime payload](member-runtime.md#persistence-notification-and-read-interfaces), run is the current runtime summary, and nullable interaction contains the stored interaction fields in the interaction field order below. Text is `Event: EVENT_ID run=RUN_ID seq=EVENT_SEQ kind=KIND notification=STATE message=MESSAGE_ID` followed by the runtime summary line and `Interaction: INTERACTION_ID classification=CLASS closed_at=TIMESTAMP close_reason=REASON` when linked. Unavailable optional text values render `-`.

`cafleet input show INPUT_ID [--json]` and `cafleet input reconcile INPUT_ID --disposition DISPOSITION --evidence-ref REFERENCE [--json]` return `{input}` with the durable row ordered `input_id,run_id,operation_key,kind,payload_digest,state,created_at,updated_at,message_id,disposition,evidence_ref,permission_response`. The nullable audit object and additional text line follow the [permission contract](#permission-response-audit-output). Text is `Input: INPUT_ID run=RUN_ID kind=KIND state=STATE disposition=DISPOSITION message=MESSAGE_ID`, with `-` for absent optional fields.

Reconciliation is Director-owned. CLI disposition tokens are `submitted`, `not-submitted` and `abandoned-uncertain`, persisted with underscores as defined by the [receipt contract](member-runtime.md#input-arbitration-and-the-original-native-answer). Require a nonempty evidence reference, synchronize with the current owner before CAS settlement and preserve uncertainty while an active writer cannot be ruled out. A settled ordinary receipt with the same disposition returns its original row and evidence reference before live work; a conflicting disposition fails unchanged. Settlement sends no input and establishes no native command result. One original five-second budget covers the pane wait, current-owner synchronization, supplied-connection SQL and socket exchange. If the writer is unreachable, use the [receipt-only exclusive ownership contract](member-runtime.md#input-arbitration-and-the-original-native-answer), preserving the fixed placement state, any original pane guard, the original identity-matching run lock and lifecycle history; unavailable proof retains uncertainty. Ordinary member recovery keeps its completed-terminal no-work behavior.

All event/input IDs are positive database-assigned integers; malformed IDs/options fail with usage exit 2. Missing rows and invalid stored state fail explicitly. Interaction fields are ordered `interaction_id,run_id,episode_seq,classification,evidence_digest,opened_at,evidence_json,closed_at,close_reason`.

### `member recover` {#member-recover}

`cafleet member recover MEMBER_ID [--json]` is Director-owned and reconciles a recorded run of an active or deregistered member. A member without a run fails explicitly before mutation. The [recovery contract](member-runtime.md#storage-pressure-and-durability-boundary) owns identity, ownership, replay, archival and its bounded pass.

The exceptional form is `cafleet member recover MEMBER_ID --owner-terminated --expected-run-id RUN_ID --expected-owner-epoch EPOCH --evidence-ref REFERENCE [--json]`. Require all four options together, positive snapshot IDs/epoch and a nonempty operator evidence reference; incomplete or invalid input exits 2. A changed owner rejects the attestation, and positive live evidence preserves the live owner.

Ordered JSON keys are `member_id,run_id,outcome,replayed_count,inbox_count,archived_count,status,cleanup_diagnostics`. Outcome is `recovered`, `recovering`, `live_or_unreachable` or `operator_reconciliation_required`; diagnostics retain cleanup/attestation provenance. Counters measure work newly committed by this invocation under the [runtime recovery count domains](member-runtime.md#storage-pressure-and-durability-boundary). A matching completed terminal return validates supplied current run/epoch expectations and preserves existing history with zero counts. Text is `Recovery: member=MEMBER_ID run=RUN_ID outcome=OUTCOME status=STATUS replayed=COUNT inbox=COUNT archived=COUNT`, followed by each bounded diagnostic as `Cleanup: DIAGNOSTIC` in array order. A string diagnostic renders its contained text; a structured diagnostic renders compact JSON. Incomplete or unreachable outcomes remain distinct from confirmed recovery.

## Permission commands {#permission-commands}

`cafleet permission inspect INTERACTION_ID --event-id EVENT_ID --from-member-id DIRECTOR_ID [--json]` reads the current request through its pinned runner. Require positive IDs, then an existing interaction (an unknown `INTERACTION_ID` is reported as not found before any event, run or database-identity check), then an active ordinary target, pinned active Director and live fleet; verify the event/message association and interaction ownership under the existing pane lock. One original five-second deadline, clipped to any inherited SQL bound, covers locking, current observation/manual progress, database work and the control exchange. Inspection sends zero programmatic child bytes and creates no receipt, audit or claim. Unknown answer capability is an inspection result; stale ownership or an unavailable owner is an operational error.

The inspection JSON is `{request}`. Its object has ordered keys:

```text
event_id,message_id,member_id,fleet_id,director_member_id,run_id,owner_epoch,interaction_id,episode_seq,classification,visibility,backend,native_version,profile_version,answer_profile_version,answer_rule_id,answer_capability,capability_reason,supported_decisions,request_token,output_seq,evidence,missing_evidence
```

IDs/epoch/episode are positive integers; `output_seq` is an unsigned integer. `message_id` is the verified inbox message ID. `classification` is the current interactive class or null, and `visibility` is `visible`, `obscured`, `not_visible` or `unknown`. Native/detector/answer-profile identifiers are nullable when unavailable. `answer_profile_version` is shared by the currently supported decision plans. `answer_rule_id` is their common rule when unique, including the actual rule for one supported decision; differing supported rules yield null. Respond derives its chosen nonnullable rule internally for the immutable audit. `answer_capability` is `tested` or `unknown`; `capability_reason` is null when the current request is answerable and otherwise a bounded explanation. `supported_decisions` contains the currently tested subset of `allow,deny`, in that order; a blocked or unsupported request returns an empty array and null token.

`request_token` is a nullable random lowercase 64-hex value issued by the current runner, bound to the request and invalidated under the [response contract](member-runtime.md#request-bound-permission-responses). `evidence` is the complete selected current post-policy object. `missing_evidence` lists unavailable public facts in order from `operation,arguments,cwd,environment,targets,effects,choice_scope`. Treat operation as available only when its post-policy value is nonblank, unredacted and untruncated; metadata-only, `[redacted]` or truncated evidence leaves it missing. The current detector supplies no independent structured arguments, cwd, environment, targets or effects, so those facts remain missing rather than being parsed from operation text. Include `choice_scope` whenever the public post-policy evidence has no explicit scope fact, including scope-free evidence accompanied by a valid token and a proved private `single_request` plan. The Director obtains necessary missing facts through authorized current capture/routed evidence before deciding. Token identity and private plan eligibility are independent of public evidence completeness and authorization. The token binds the full immutable decision-to-plan set, including each profile, rule and stage identity; a null common-rule field supplies no authority to select a replacement plan.

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

Require positive IDs/epoch, the token's exact hex form, a nonblank operation key and the [bounded evidence references](member-runtime.md#permission-response-storage). Source `director-auto` requires mode `auto`; source `user` records the user's decision in any mode. Enum/shape/source-mode errors exit 2 before lock or byte effects. These fields are attributed claims: resolve Director mode from current host evidence independently of member launch policy and obtain user permission in other/unknown modes. Native and managed rules still authorize running the command.

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

### Permission-response audit output

`input show` and `input reconcile` append nullable `permission_response` after `evidence_ref`. For a response kind it contains ordered `input_id,event_id,interaction_id,expected_owner_epoch,request_token,decision,director_member_id,director_mode,decision_source,mode_evidence_ref,decision_evidence_ref,answer_profile_version,answer_rule_id,decided_at`; every field is required and follows its SQL type/nullability. Other input kinds return null.

Preserve the existing input text line and append `Permission decision: interaction=INTERACTION_ID event=EVENT_ID decision=DECISION source=SOURCE mode=MODE director=DIRECTOR_ID epoch=EPOCH` for a response audit. Settlement records disposition and sends zero bytes; it neither grants a native answer nor releases a possibly transmitted episode claim.

## Error Messages

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
| `member create` | `--coding-agent opencode --model` violating the `<provider-id>/<model-id>` format | [Backend model error](coding-agent-backends.md#model-selection) | 2 | Fires before any side effect |
| `member create` | `--effort` with a level unknown to the claude backend | [Backend effort errors](coding-agent-backends.md#reasoning-effort) | 2 | Fires before any side effect |
| `member create` | `--coding-agent codex --effort` with an unknown level | [Backend effort errors](coding-agent-backends.md#reasoning-effort) | 2 | Fires before any side effect |
| `member create` | `--coding-agent opencode --effort` with any value | [Backend effort errors](coding-agent-backends.md#reasoning-effort) | 2 | Fires before any side effect |
| `member create` | An unknown `{placeholder}` in the prompt | `Error: Unknown placeholder '<name>' in custom prompt. Supported placeholders: {fleet_id}, {member_id}, {director_member_id}, {coding_agent}. Double literal braces ({{, }}) to keep them as text.` | 2 | `member create` attempts deregistration; cleanup failures follow the primary error |
| `member create` | A malformed brace expression in the prompt | `Error: Malformed custom prompt: <detail>. Double literal braces ({{, }}) to keep them as text.` | 2 | `member create` attempts deregistration; cleanup failures follow the primary error |
| `member create` | `--coding-agent` omitted and the spawning Director not found in the fleet | `Error: cannot resolve the member's coding agent: Director <director-id> not found in fleet <fleet-id>. Re-run with an explicit --coding-agent.` | 1 | Nothing spawned |
| `member create` | `--coding-agent` omitted and the spawning Director has no placement row | `Error: cannot resolve the member's coding agent: Director <director-id> has no placement row recording its backend. Re-run with an explicit --coding-agent.` | 1 | Nothing spawned |
| `permission inspect` | An unknown `INTERACTION_ID` | `Error: Interaction <interaction-id> not found` | 1 | Reported before any event, run or database-identity check |
| `permission inspect` | The inspected run records a database identity other than the selected database's | `Error: invalid stored value for member_runs.database_identity_json: selected database identity mismatch` | 1 | Followed by the known interaction context |
