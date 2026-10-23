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
| `member ping` | Submit a fixed inbox-poll prompt when idle | `MEMBER_ID` | yes |
| `member capture` | Read a pane tail | `MEMBER_ID` | yes |
| `message send` | Persist a point-to-point message | Sender/recipient flags | yes |
| `message poll` / `message wait` | Read incoming deliveries / receive finitely | `MEMBER_ID` | yes |
| `message ack` / `message show` | Consume / read one message | `MESSAGE_ID` | yes |

## Environment variables

| Variable | Field | Default | Meaning |
|---|---|---|---|
| `CAFLEET_DATABASE_URL` | `database_url` | `sqlite:///` + `~/.local/share/cafleet/cafleet_v8.db` | Selected SQLite file path |
| `CAFLEET_MULTIPLEXER` | `multiplexer` | Auto-detect | Explicit supported multiplexer override |
| `CAFLEET_MAX_TEXT_LEN` | `max_text_len` | `200` | Text-body and pane-preview codepoint limit |

The default expands HOME once. An explicit URL has the form `sqlite:///<path>`; resolve a relative file path against the invoking working directory before canonicalizing the database file for shared pane-lock identity. A literal tilde in an override is not expanded. Count values accept nonnegative integers and invalid values fail with `<NAME> must be a non-negative integer (got '<value>')`.

## Global Options

`cafleet --version` prints `cafleet <version>` and exits 0 before dispatch.

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

### Member list output {#member-list-output}

Roster records include `member_id`, `name`, `kind`, `placement`, `last_sent`, `last_recv`, `last_ack` and `idle`. `kind` is `director` when the fleet reference matches, otherwise `member`. Missing activity is null; placement may be null, or its pane ID pending. Text uses `-` for absent values and `(pending)` for a pending pane.

## JSON output (`--json`) {#json-output}

Use a trailing per-subcommand `--json`. It emits compact single-line JSON with UTF-8 and complete untruncated message text. Application errors remain stderr text; parse errors exit 2. Commands marked no reject the flag, including the root group.

## Positional subject ids {#positional-subject-ids}

Place `FLEET_ID`, `MEMBER_ID` or `MESSAGE_ID` immediately after the subcommand. The subject row supplies related fleet/recipient state. IDs are complete DB-assigned integers, without prefix resolution or environment defaults. A missing or malformed subject uses the parser's native error, exit 2.

Spawned members read literal `FLEET ID:`, `YOUR MEMBER ID:`, `DIRECTOR MEMBER ID:` and `CODING AGENT:` lines. Pass these literal IDs on each invocation.

## Sender and recipient (`--from-member-id`, `--to-member-id`) {#from-to-member-id}

Send requires integer sender and recipient flags. Both must be active in the same live fleet. The sender row determines the fleet; IDs are coordination identifiers within the local trust model.

## `permissions.allow` coverage

Use prefix rules such as `Bash(cafleet message *)` to cover ordinary broker operations, including `cafleet message wait ID --timeout 300 --json`. Director capture is covered by `Bash(cafleet fleet capture *)` or a broader authorized prefix. Literal IDs and trailing JSON flags preserve prefix matching.

Keep `Bash(cafleet member prompt *)` and `Bash(cafleet member exec *)` under `permissions.ask`, because their bodies carry operator-controlled input. Ask rules outrank allow rules. The [permission profiles](coding-agent-backends.md#claude-profile-entries) describe each role's needs.

## Message Body Truncation

CLI text and inline preview bodies use `CAFLEET_MAX_TEXT_LEN` Unicode codepoints followed by `…` when truncated. Full JSON and pane captures preserve their complete content. [Message envelope](message-envelope.md#rendered-shape) owns the compact rendering.

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


## Shared diagnosis and connection reuse {#diagnosis-reuse}

Use the invocation’s shared classifier and connection for guard/setup work. Doctor completes read-only sections even when the database is unavailable. Compatibility includes identity, ledger version and expected table/column shapes.

## Schema-version guard {#schema-version-guard}

The fresh baseline creates exactly the application tables declared in the schema. Compatibility diagnosis verifies current application identity, ledger version, every required table’s column shape, and required indexes/foreign keys. Unrelated auxiliary tables, indexes and triggers are compatible while those required definitions remain intact.

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
| No agent has a row at its currently-resolved path (zero rows, or — on a hand-tampered at-head database — a dropped `asset_installs` table) | `Error: no assets install is recorded at the resolved paths; run 'cafleet setup' to install` | 1 |
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
every resolvable agent shows the `–` state. No superseded footnotes render,
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
[Spawn-time allow rules](coding-agent-backends.md#spawn-time-allow-rules).

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
no path resolved. `schema_version` is `null` when the ledger is absent.
When the recorded rows are not read (a non-`ok` database report, or a
missing `asset_installs` table), resolved agents use `"not_installed"`
and path errors remain
`"error"`. Recorded fields are null and `superseded` is empty. Exit-code semantics are identical to text mode.


## `cafleet message` — Message Broker

### `message send`

`cafleet message send --from-member-id ID --to-member-id ID (TEXT | --file PATH) [--json]`

Persist one point-to-point delivery, then attempt at most one oldest eligible ordinary-member preview. A committed/persisted send remains successful with exit 0; held/unavailable notifications preserve the stdout record and `notification_sent=false`. The Director receives by broker reads; self-sends have settled notification state.

`notification_sent` refers to the newly committed message; delivery of an older preview leaves it false. [Push notifications](multiplexer-backends.md#push-notifications) specifies selection and under-lock revalidation.

#### Delivery outcome {#message-send-delivery}

On the first attempted unknown/draft capture, stderr is `Message NEW_ID was persisted. member ID is STATE; preview PREVIEW_ID was not sent.` Other post-commit notification/stamp failures also name the persisted ID without suppressing successful stdout. Storage failure before commit exits 1 with its cause. Consume an existing committed ID once; notification failure supplies no reason to create another delivery.

### `message poll`

`cafleet message poll MEMBER_ID [--json]` returns only incoming `input_required` records, descending by status timestamp/message ID. Unknown or inactive member: `Member ID not found`, exit 1.

### `message wait`

`cafleet message wait MEMBER_ID [--timeout SECONDS] [--json]`

Timeout is an integer in `1..=1800`, default `300`. Validate an active subject and live fleet. Existing unread deliveries return immediately; otherwise check at one-second monotonic intervals. The invocation owns finite receiving and never ACKs messages.

Between inbox checks, retry at most one ordinary-member preview per interval with an in-memory round-robin cursor. Exclude the Director, self-notifications, ACKed and already-notified rows. Recheck the inbox before starting another attempt. Shared input locks and under-lock revalidation keep multiple waiters safe.

Each attempt has a five-second total budget for capture, typing, settle and stamp. Cap subprocess/database busy waits to its remaining budget. Start no new I/O after the monotonic receive deadline; return by that deadline plus at most one in-flight five-second attempt/cleanup allowance. Stop between stages when time expires and report uncertainty if bytes may have arrived.

The ordered JSON result is:

```json
{"member_id":41,"outcome":"timeout","messages":[],"notification_errors":[]}
```

`outcome` is `messages` with nonempty full poll records, or `timeout` with an empty array. Each diagnostic is `{member_id, message_id, delivery, error}`, where delivery is `not_sent` or `uncertain`. Keep only the latest error per member in memory.

Unknown/draft diagnostics are `member ID is STATE; preview MESSAGE_ID was not sent.` Working, approval, pending placement and lock contention are ordinary holds. Revalidate diagnostics at every receive check and before return; remove entries after ACK, member departure or successful notification by any invocation. A later error replaces the previous one.

Text is `Messages available for member ID.` plus normal messages, or `Timed out waiting for member ID.`; diagnostics go to stderr. Both outcomes exit 0. Deleted subject/fleet or corrupt/unreadable state exits 1 with its cause. Ctrl+C terminates the finite process normally.

The Director ACKs every consumed delivery before the next wait, including questions or deferred approvals; unfinished actions remain in host-owned task state. Use the default for a host-native background task, retain its live task/session handle and collect the terminal result. A foreground-only mechanism uses an explicit timeout of at most 30 seconds. On timeout inspect roster, panes and artifacts, then receive again while delegated work remains. Keep active receive ownership until the reply or explicit user handoff.

### `message ack`

`cafleet message ack MESSAGE_ID [--json]` transitions an existing unread message to completed, updating its status timestamp. The recipient/fleet derive from that row. ACK signifies consumption.

### `message show`

`cafleet message show MESSAGE_ID [--json]` returns one full record. Existence is the read guard.

## `cafleet member` — Member Lifecycle + Pane Interaction {#cafleet-member}

### Member targeting and key delivery

Require an active member and placement for pane operations; concrete pane IDs remain opaque. Pending placement is valid during creation and yields contextual prompt/exec/capture errors, or a ping skip. Missing placement fails pane actions; list/show retain diagnostic nulls. Exec rejects the root Director; individual member deletion protects it.

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
| positional `PROMPT` | one of | Inline spawn prompt (backend-neutral template). Exactly one of `PROMPT` / `--file`. |
| `--file PATH` | one of | Path to a UTF-8 file whose contents are the spawn prompt (`-` = stdin). Follow the shared [body-input and transport contract](#text-body-input). |
| `--json` | no | Output as JSON |



#### Spawn command per backend

The per-backend spawn argv and auto-approval flags live in
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



### `member delete` {#member-delete}

Lock the concrete placement before kill/deregistration. A member without a concrete pane needs no lock. Retry contention every 50 milliseconds against one five-second deadline; expiry exits 1 with `member ID's pane input remained busy for 5 seconds; member was not deleted.` and leaves state unchanged. Preserve pane/registry errors and layout cleanup ownership.

### `member show` {#member-show}

`cafleet member show MEMBER_ID [--json]` — the positional `MEMBER_ID` names
any active registry entry — placed or placementless (root Director included).

Registry read — no multiplexer requirement. Text is the compact one-line row;
the detailed view — `kind` ([`director` / `member`](data-model.md#members)), and the
placement sub-dict — is the `--json` payload (see
[Output shapes](#output-shapes)).


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

Acquire the shared input lock, revalidate activity/placement and require idle. Submit literal text, settle and Enter. Exit 0 means submission only, with `{member_id, pane_id, text}`.

A held lock uses `member ID's pane is receiving another input; retry after it is idle.` Readiness rejection is `member ID is STATE; prompt was not dispatched.` Pre-write error: `prompt was not dispatched to member ID: CAUSE`. Partial uncertainty: `prompt delivery to member ID is uncertain: CAUSE. Inspect the pane before retrying.` These exit 1 without a success record.

### `member exec` {#member-exec}

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

### `member ping` {#member-ping}

`cafleet member ping MEMBER_ID [--json]` is an explicit Director action. Lock/revalidate the target, require idle, then submit exactly `cafleet message poll ID --json` with settle/Enter.

Successful JSON is `{member_id, pane_id, skipped:false, reason:null}`; text is `Pinged member NAME (PANE) — poll keystroke dispatched.`

Pending placement, lock contention and readiness yield exit 0 with `skipped:true`. Reasons are `pending_placement`, `busy`, `working`, `awaiting_user`, `draft`, `unknown`; pane_id is null only for pending placement. Pending text: `Member NAME has no pane yet (pending placement) — ping skipped; it will poll its inbox on spawn.` Other skip text: `Member NAME (PANE) is REASON — ping skipped.`

Capture/transport failures exit 1 with their cause; possible partial input is `poll-trigger delivery to member ID is uncertain: CAUSE. Inspect the pane before retrying.` Finite receiving creates no ping.

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


## Error Messages

Application errors print as stderr text even with `--json`; the flag selects successful output. Input errors use the documented usage/application category. Dispatch, readiness, compatibility and deletion-lock errors are specified at their action owners above.

| Command | Situation | Error message | Exit | Notes |
|---|---|---|---|---|
| (any surface resolving a config dir) | A set config-path variable holds a non-absolute value (empty string and relative paths included) | [Config-path validation](#config-dir-resolution) | 1 | See [Config-dir resolution](#config-dir-resolution); `doctor` renders it as a per-agent issue instead |
| (any non-setup command) | The recorded schema version is behind the embedded head | [Schema-version guard](#schema-version-guard) | 1 | See [Schema-version guard](#schema-version-guard) |
| (any non-setup command) | No schema ledger and no app tables (missing or empty DB file) | [Schema-version guard](#schema-version-guard) | 1 | See [Schema-version guard](#schema-version-guard) |
| (any non-setup command) | No schema ledger but app tables present | [Schema-version guard](#schema-version-guard) | 1 | See [Schema-version guard](#schema-version-guard) |
| (any non-setup command) | The recorded schema version is ahead of the embedded head | [Schema-version guard](#schema-version-guard) | 1 | See [Schema-version guard](#schema-version-guard) |
| (any fleet-scoped command) | No agent has an `asset_installs` row at its currently-resolved path | [Stale-assets guard](#stale-assets-guard) | 1 | See [Stale-assets guard](#stale-assets-guard) |
| (any fleet-scoped command) | An `asset_installs` row at a resolved path differs from the runtime CLI version | [Stale-assets guard](#stale-assets-guard) | 1 | See [Stale-assets guard](#stale-assets-guard) |
| `member create` | `--coding-agent opencode` with no agent preset at the resolved preset path | `Error: opencode agent preset not found at <preset>; run 'cafleet setup --coding-agent opencode' first` | 1 | `<preset>` is the resolved `<preset base>/agents/cafleet.md` — see [Config-dir resolution](#config-dir-resolution) |
| `setup` | The `asset_installs` table is missing as the assets half starts | `the database schema is missing or outdated; run 'cafleet setup' first` | 1 | An assets-half failure, after a db-half failure or an externally broken schema |
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
| `member capture` / `member prompt` / `member exec` | The member has a pending placement | <code>Error: member &lt;id&gt; has no pane yet (pending placement) — nothing to &lt;capture&#124;prompt&#124;exec&gt;.</code> | 1 | `member ping` instead skips and exits 0 — see [member ping](#member-ping) |
| `member show` / `prompt` / `exec` / `ping` / `capture` | An unknown or inactive target member id | `Error: Member <member-id> not found` | 1 | — |
| `member prompt` / `exec` / `ping` / `capture` | A target with no placement row | ``Error: member <member-id> has no placement row; it was not spawned via `cafleet member create`.`` | 1 | — |
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
