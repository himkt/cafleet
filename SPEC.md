# CAFleet — Reimplementation Specification

This is the single, self-contained, authoritative specification for
reimplementing the `cafleet` CLI (message broker + member registry for coding
agents). It is
**language- and stack-agnostic**: it defines the *interface and observable
behavior* that any reimplementation must reproduce, independent of the language
or libraries used to build it. It fixes the cross-cutting decisions once,
resolves type disagreements centrally, and carries the full per-module
behavioral contract inline — there is no external detail document to consult.

The implementation is **documented as the eight sections** below, each specified
in full in [§6](#6-module-specifications).

| Module | Scope |
|---|---|
| Persistence & Schema | data models, connection factory, migration-managed SQLite schema |
| Broker | synchronous data-access layer |
| CLI | the whole `cafleet` command tree |
| Output | text/JSON formatting, truncation, ANSI strip |
| Multiplexer | tmux + herdr integration, keystroke injection |
| Monitor | heartbeat supervision loop |
| Coding agents | claude/codex/opencode backends |
| WebUI + Config | HTTP API + `CAFLEET_*` settings |

Where this document states both a high-level invariant and a detailed rule, the
detailed rule governs; they are written to agree.

---

## 1. Overview & goals

CAFleet is a message broker and member registry for coding agents. A single
SQLite database holds fleets, members, their tmux placements, messages,
and a per-fleet monitor runtime row. The `cafleet` CLI is the primary surface:
it creates fleets, spawns coding-agent members into tmux panes, routes messages
between them by keystroke-injecting inline previews, and runs a heartbeat loop
that periodically wakes the fleet's dedicated monitor member to health-check
the team. An admin WebUI exposes a read-mostly JSON API over the same broker.

**Goal:** specify the **redesigned** `cafleet` command surface end-to-end so any
implementation can reproduce it. The contract is the *interface and observable
behavior* of the surface defined here, not the internal byte-for-byte mechanics.
This is a deliberate, greenfield redesign: it is **not** behavior-preserving with
respect to any earlier `cafleet` (see **Non-goals** below).

What is part of the contract (must be reproduced):

- **CLI surface:** every command and subcommand name, option name, option type,
  default, required-ness, documented-vs-hidden status, and exit code, exactly as
  fixed by §6.3 and §10.
- **Configuration surface:** every `CAFLEET_*` environment variable, its type,
  and its default (§9).
- **Persistence surface:** the SQLite schema at the migration head — table
  names, columns, types, nullability, defaults, foreign-key rules, indexes,
  and status/enum string values.
- **HTTP surface:** every route, method, request/response shape, header
  contract, and status code of the WebUI API.
- **Observable semantics:** the message status lifecycle, the soft-delete +
  cascade rules, the monitor claim/heartbeat/clear protocol, the message
  routing and notification behavior (including the unicast partial-failure
  surfacing, §6.3), and the stdout-vs-stderr stream
  choice for each emitted line.

What is **not** required (the relaxation):

- **Byte-level output identity is not required.** A reimplementation must
  produce output that is *semantically equivalent and structurally faithful*
  (same fields, same JSON key set and ordering, same human-readable layout
  intent), but it need not match every byte of a particular host language's
  rendering. Where an implementation does something idiosyncratic purely as an
  artifact of its language (e.g. a particular `repr()` rendering, an exact
  exception message suffix), reproducing the *intent* is sufficient.

**Non-goals:**

- **Cross-implementation database interoperability is a non-goal.** The schema
  is the migration head defined in §8; databases produced by other
  implementations are not expected to interoperate.
- **Reference-parity is a non-goal.** The surface here is the contract; there is
  no separate "reference" surface to match.

Points the per-module sections leave implicit are clarified in
[§11](#11-decisions--clarifications).

---

## 2. Architecture stance

The reimplementation adopts a **synchronous-core + async-server** shape. The
CLI, broker, monitor, multiplexer, and coding-agent layers are all synchronous;
only the WebUI HTTP server may be asynchronous, and it calls the synchronous
broker from blocking tasks.

The rationale for the split:

- CLI invocations stay free of an async runtime — no async runtime spin-up for a one-shot
  command like `cafleet message send`.
- SQLite's per-connection write lock serializes monitor claims without async
  complication.
- Only the long-lived server pays for an async runtime, if the target language
  even has one.

The concurrency model is an implementation choice and is not itself part of the
contract. The one hard requirement is that the monitor's "SQLite write lock
serializes claims" assumption (§6.2) is preserved, whatever threading or
concurrency model is used.

---

## 3. Module layout

The implementation is organized as a set of modules, one per concern, plus a
single CLI entry point. The CLI embeds the WebUI as a library launched by its
`server` subcommand. Whether these are separate compilation units, packages,
namespaces, or directories is a target-language choice; the **dependency
structure** below is the contract.

```
cafleet
├── config          config half: Settings singleton, CAFLEET_* env
├── db              connection factory, embedded migration chain
├── multiplexer     Multiplexer interface, tmux + herdr backends, resolver, keystrokes
├── coding-agent    coding-agent interface + claude/codex/opencode
├── output          render + formatter layers
├── broker          typed data-access layer and notification traits
├── runtime         concrete process/probe/notifier adapters
├── monitor         heartbeat loop
├── webui           server half: HTTP app, /api router, SPA fallback
└── cli             command tree + handlers; the cafleet entry point
```

**Entry points:** exactly one user-facing binary/command — `cafleet`. There is
no separate server binary; `cafleet server` constructs the WebUI app and serves
it.

---

## 4. Architecture & module dependency graph

The diagram shows the process and notification boundary; the edge list defines
the remaining dependencies. Arrows point from callers to their dependencies.

```text
CLI / HTTP presenters ─────────────► broker ──► db
        │                              ▲
        ▼                              │ notification traits
     runtime adapters ─────────────────┘
        │
        ├──► multiplexer ──► injected CommandRunner
        └──► coding-agent ─► injected SpawnProbe
```

Edges (who depends on whom):

- **config** — leaf. No internal deps.
- **db** — depends on `config` for the database URL. Owns the connection
  factory and embedded migration chain.
- **output** — pure string/structure transforms, using configuration-derived
  text limits where required.
- **multiplexer / coding-agent** — backend protocols, using injectable runner
  and probe interfaces. Inline-preview truncation remains broker-side.
- **broker** — depends on DB/domain types and pure formatting helpers. It owns
  the notification trait and policy, not a concrete multiplexer or process
  launcher. It neither starts subprocesses nor imports HTTP or CLI handlers.
  The existing monitor liveness semantics, including the signal-0 probe, stay
  unchanged by this boundary refactor.
- **runtime** — owns `SystemRunner`, `SystemProbe`, and the concrete notifier
  adapter. It implements the broker's notification trait using multiplexer and
  coding-agent interfaces, and may consume configuration. It imports neither
  CLI handlers nor HTTP handlers.
- **monitor** — uses broker monitor operations and injected multiplexer
  operations for pane discovery and wake delivery.
- **webui** — uses broker types, runtime adapters, and configuration. HTTP
  presenters construct the existing wire payloads; webui never imports cli.
- **cli** — composes broker, runtime adapters, output, multiplexer,
  coding-agent, monitor, config, db, and webui for the single `cafleet` entry
  point. CLI presenters retain their existing output and error contracts.

**Reconciled overlap points** (specced once, here, then referenced):

1. **Broker → runtime notifier → multiplexer inline preview.** The broker's `_try_notify_recipient`
   (§6.2) looks up the recipient's `mux_pane_id`, skips self-sends and
   paneless recipients, **truncates** `text` to `settings.max_text_len` with a
   `…` suffix, then calls its notification trait. The runtime adapter delegates to
   `send_inline_preview` (§6.5), which keystrokes the 2-line `[cafleet msg …]` payload Esc-first. The multiplexer call returns a
   result carrying the **raw backend error** on failure (§6.5); the broker
   never rolls back the persisted message on a failed keystroke and never
   retries it. The unicast CLI surfaces an attempted-and-failed notification
   as an exit-1 partial failure (§6.3); broadcast discards individual preview
   errors and only its `delivered` count reflects them (§6.2). Truncation
   happens broker-side; the keystroke mechanics are multiplexer-side.
2. **CLI ↔ multiplexer ↔ coding-agent member-create.** `cafleet member create`
   (§6.3) sequences: resolve backend → `validate_model` → `validate_effort` →
   resolve the prompt
   body via the shared positional-`PROMPT` / `--file` reader → `ensure_available`
   → broker `register_member` (placement with `mux_pane_id` unset) → substitute
   `{fleet_id}` / `{member_id}` / `{director_member_id}` / `{coding_agent}`
   placeholders (§6.3) → `build_spawn_argv` (§6.7) →
   multiplexer `split_window` (§6.5), forwarding `CAFLEET_DATABASE_URL` (when
   set) into the new pane's environment (§7.1) → broker
   `update_placement_pane_id`. A rollback ladder attempts deregistration on any
   post-register failure.
3. **Monitor loop ↔ broker monitor DB ops.** The loop (§6.6) owns the
   OS-facing half (signal handling, sleep, the single keystroke); all DB
   mutation and lookup (`claim`/`heartbeat`/`clear`/`record_monitor_wake`/
   `list_fleet_wake_targets`/`active_monitor_member_id`) is the broker's
   (§6.2). The single-instance /
   split-brain guard lives entirely in the broker's runtime-row protocol; the
   loop only consumes its boolean signals.

---

## 5. Shared domain model

These are the unified entity shapes every module agrees on. Cross-module type
disagreements are **resolved here**; module sections must not contradict this.
Field types below are described abstractly (integer, string, optional string,
boolean); map them to the target language's natural types.

### 5.1 Timestamps (resolved)

Every timestamp column (`created_at`, `registered_at`, `deregistered_at`,
`status_timestamp`, `deleted_at`, `last_ping_at`, `started_at`, `last_tick_at`)
is stored as an **ISO-8601 string** in UTC with an explicit `+00:00` offset and
**fixed-width 6-digit microsecond precision**
(`YYYY-MM-DDTHH:MM:SS.ffffff+00:00`).

- **Storage type:** string (TEXT in SQLite).
- **Production:** the current UTC time formatted as
  `YYYY-MM-DDTHH:MM:SS.ffffff+00:00`. The fractional part is always exactly
  six zero-padded digits — including when the microsecond value is zero — so
  every produced timestamp has the same width and rows interleave and sort
  identically.
- **Parsing:** the reader stays lenient for externally supplied values (CLI
  inputs): it accepts a missing fractional part and any UTC offset spelling.
  Leniency applies to parsing only; production always emits the fixed-width
  form above.
- **Comparison/ordering:** done **lexicographically on the string** in SQL
  (`ORDER BY status_timestamp DESC`) and in any max-over-timestamps (e.g.
  idle-seconds computation). This is correct only because all timestamps share
  the one canonical format. Do **not** parse-then-compare for ordering; preserve
  string comparison.
- **Arithmetic:** for age/idle math only, parse the ISO-8601 string and compute
  seconds (float, integer-truncated where the consuming surface specifies an
  integer).

### 5.2 Core entities

Column types, nullability, defaults, and FK rules are detailed in §6.1 and §8.
The unified shapes:

**Fleet**

| Field | Type | Notes |
|---|---|---|
| `fleet_id` | integer | PK, AUTOINCREMENT |
| `name` | optional string | |
| `created_at` | string | ISO timestamp |
| `deleted_at` | optional string | soft-delete marker |
| `director_member_id` | optional integer | FK→members, ON DELETE RESTRICT; null only mid-bootstrap |

**Member**

| Field | Type | Notes |
|---|---|---|
| `member_id` | integer | PK, AUTOINCREMENT |
| `fleet_id` | integer | FK→fleets, ON DELETE RESTRICT |
| `name` | string | |
| `description` | string | |
| `status` | enum string | see 5.3 |
| `registered_at` | string | ISO timestamp |
| `deregistered_at` | optional string | |
| `member_card_json` | string | A2A card JSON |

**MemberPlacement** (1:1 with Member; `member_id` is PK = FK, not autoincrement)

| Field | Type | Notes |
|---|---|---|
| `member_id` | integer | FK→members, ON DELETE CASCADE |
| `mux_session` | string | backend-neutral multiplexer session |
| `mux_window_id` | string | backend-neutral window/tab id |
| `mux_pane_id` | optional string | opaque backend pane id (tmux `%N`, herdr `w1:p1`); unset until `split_window` resolves it |
| `backend` | string | DDL default `"tmux"`; the resolved `mux.name` (`"tmux"`/`"herdr"`) that produced the pane ids |
| `coding_agent` | string | NOT NULL, no DDL default |
| `created_at` | string | ISO timestamp |

**Message** (the message record)

| Field | Type | Notes |
|---|---|---|
| `message_id` | integer | PK, AUTOINCREMENT |
| `owner_member_id` | integer | FK→members, ON DELETE RESTRICT (the member whose inbox owns the row) |
| `from_member_id` | integer | NO FK |
| `to_member_id` | optional integer | NO FK; nullable — `NULL` on `broadcast_summary` rows (see 5.5) |
| `type` | enum string | see 5.3 |
| `created_at` | string | ISO timestamp |
| `status_state` | enum string | see 5.3 |
| `status_timestamp` | string | ISO timestamp |
| `origin_message_id` | optional integer | NO FK; broadcast deliveries point at the summary; summary points at itself |
| `text` | string | never truncated at persistence |

**MonitorRuntime** (1:1 with Fleet; `fleet_id` is PK = FK, not autoincrement)

- `fleet_id`: integer, FK→fleets, ON DELETE RESTRICT
- `pid`: optional integer, the claiming process
- `started_at`, `last_tick_at`: optional timestamp strings
- `tick_seconds`: DDL default 5
- `last_wake_at`: nullable; the UTC ISO timestamp of the last successfully delivered wake, durable across loop restarts; preserved by the runtime clear
- `wake_interval_seconds`: nullable; the live mirror of the running loop's wake interval — stamped at every claim/reclaim, re-read per tick, overwritten by `PATCH /api/monitor`, preserved by the runtime clear; `NULL` only in rows that predate the column and were never re-claimed
- `wake_requested_at`: nullable; the UTC ISO timestamp of the latest pending forced-wake request (`POST /api/monitor/wake`) — `NULL` when none is pending; repeat requests overwrite the timestamp (coalesce into a single wake); cleared by a delivered wake (`record_monitor_wake`) and by the reclaim reset in `claim_monitor_runtime`

A missing runtime row differs from null fields on an existing row. Clear
nulls pid/start/tick, preserving wake timestamps and intervals; the stopped
HTTP projection masks process timestamps. A null wake interval differs from
zero, which disables scheduled wakes while permitting forced wakes.


**AssetInstalls** (composite TEXT PK `(coding_agent, path)`, not autoincrement, not FK-linked)

- `coding_agent`: PK part; one of `"claude"` / `"codex"` / `"opencode"`
- `path`: PK part; the agent's resolved identity path (§6.3 *Config-dir resolution*), stored absolute exactly as resolved
- `cafleet_version`: the CLI's compile-time version string at install time (§7.6)
- `installed_at`: UTC ISO-8601 with microsecond precision

Writes are upserts on the composite key (`ON CONFLICT(coding_agent, path) DO UPDATE SET cafleet_version, installed_at`). Rows are written by the assets half of `cafleet setup` — one row per installed agent at its resolved identity path — after that agent's skills and preset (where one exists) install successfully, so a row attests skills + preset; the db half never touches the rows. Consumers partition an agent's rows by comparing `path` against the currently-resolved identity path: the row at the resolved path (at most one, by the primary key) is **current**; every other row of that agent is **superseded**. Current rows feed the stale-assets guard on every fleet-scoped command group and the `cafleet doctor` coding-agents section; superseded rows surface only as informational doctor footnotes.

### 5.3 Enums (literal string contracts)

All values are persisted/compared as exact lowercase strings.

- **MemberStatus:** `"active"` | `"deregistered"`.
- **MessageType:** `"unicast"` | `"broadcast_summary"`. Broadcast fan-out emits ONE
  `broadcast_summary` (owned by the sender) + N `unicast` deliveries. There is no
  distinct "broadcast delivery" type — deliveries reuse `unicast`.
- **MessageStatus:** `"input_required"` | `"completed"`.
  - `unicast` is born `input_required`; `broadcast_summary` is born `completed`.
  - ack: `input_required` → `completed` (recipient only) — the only transition.
- **CodingAgentName:** `"claude"` | `"codex"` | `"opencode"`.

### 5.4 Member kind discriminator (resolved cross-module)

The member "kind" is a **three-value** discriminator — `director` (derived:
`member_id == fleets.director_member_id`), `monitor` (derived: the member's
`member_card_json` carries the application-level marker `$.cafleet.kind ==
"monitor"`), or `member` (every other active member) — derived at read time
from the fleet's `director_member_id` back-reference plus the member card.
`director_member_id` is checked first, so a fleet's root Director can never
read as `monitor` regardless of its card contents. No dedicated column backs
the `monitor` value — the marker is plain JSON written by `register_member`
only when the caller
requests the monitor role (§6.2, § *`member create` — spawn orchestration*).
`get_member`, `list_members`, `list_roster`, and the WebUI roster all derive
`kind` through this same rule.

### 5.5 Nullable `to_member_id` (resolved)

A `broadcast_summary` row has no single recipient, so `to_member_id` is
**nullable** and `broadcast_message` writes **`NULL`** on the summary row. A
`unicast` message always carries a real recipient id. Consumers that branch on
the recipient — e.g. the WebUI's name resolution over message endpoints
(§6.8) — test `to_member_id IS NULL` / `is None`, never a truthiness check.
Model `to_member_id` as an **optional/nullable integer**; there is no `0`
sentinel.

### 5.6 Typed records and wire presenters

Broker queries decode rows into typed records. CLI and HTTP presenters build
wire JSON, preserving field names, order, nulls, and existing envelopes.
Invalid stored enums and missing required message names are integrity errors.
Concrete subprocess and notification adapters live in `runtime/`.

## 6. Module specifications

Each subsection gives the scope, the load-bearing public surface, the critical
invariants, and the full behavioral detail — every function, behavior, and
contract error string.

### 6.1 Persistence & Schema

**Scope:** the nine data models (§5.2), the connection factory, and the
migration-managed SQLite schema. This module owns **no** CRUD/query logic and no
HTTP surface; all reads/writes/joins live in the broker (§6.2). Schema
management is detailed in §8; this section covers the connection factory, the
per-connection PRAGMAs, and the structural invariants.

#### Connection factory & engine semantics

A single lazily-constructed connection factory (a pool / session factory), built
once on first use and cached as a process-wide singleton. There is no
teardown/dispose path in normal operation; the `setup` db-migration driver
disposes its own short-lived engine when it finishes.

- **Lazy singletons, fail-loud.** The factory builds once and caches. There is
  **no fallback** if construction fails — a bad `database_url` raises at
  connect time and propagates. Do not substitute an in-memory database or any
  other default on error.
- **URL scheme validation.** The configured database URL must use the `sqlite`
  scheme (`sqlite:///<path>`); a URL with any other scheme **fails loudly** at
  connect time. The default URL is already `sqlite://…`. A user-supplied
  `CAFLEET_DATABASE_URL` is otherwise passed through verbatim (§7.1). The same
  validation is applied independently by the `setup` db-migration driver.
- **Cross-thread sharing.** The connection is shared across threads — disable
  SQLite's same-thread check where the runtime enforces one. This is part of
  the contract.

#### Per-connection PRAGMAs (mandatory)

A connection-init hook runs on **every** new SQLite connection — including
ad-hoc connections opened by tests or by the `setup` db-migration driver, not
only those from the singleton pool. On each connection:

1. If the underlying connection is not a SQLite connection, return immediately
   (a short-circuit for non-SQLite backends).
2. Otherwise run, in order:
   - `PRAGMA foreign_keys=ON`
   - `PRAGMA busy_timeout=5000`

`busy_timeout=5000` is **5000 milliseconds** — a connection waits up to 5
seconds for a lock before raising a busy error. `foreign_keys=ON` is
**mandatory**: SQLite defaults FK enforcement OFF per connection, so every
RESTRICT/CASCADE rule in §5.2 is inert unless this PRAGMA runs on the very
connection performing the delete. Omitting either PRAGMA is a correctness bug,
not a no-op. Both must run on every connection the reimplementation opens.

#### Structural invariants

- **AUTOINCREMENT on exactly three tables** — `fleets`, `members`, `messages` —
  guaranteeing monotonically increasing ids that are never reused. The 1:1
  state tables deliberately do not use it; each reuses its parent's id as PK/FK.
- **Create-order / forward-reference quirk.** `fleets.fleet_id` and
  `members.fleet_id` form a mutual reference. The initial migration creates
  the member table **first**, relying on SQLite tolerating a foreign key to a
  not-yet-created table at table-creation time (`members.fleet_id` forward-
  references the still-uncreated `fleets`). The create order must be preserved;
  it is valid only because FK enforcement is per-connection and is engaged later
  by `foreign_keys=ON`, not at table-creation time. No FK declares an `ON
  UPDATE` clause (all default to NO ACTION). §8 fixes the migration chain
  that establishes this.
- **DDL-level (server-side) defaults**, applied by SQLite when the column is
  omitted from an INSERT (distinct from values the application writes
  explicitly): `monitor_runtime.tick_seconds` → `5`.
  `member_placements.coding_agent` carries no DDL default — every writer
  passes an explicit value. `monitor_runtime.last_wake_at` carries no DDL
  default (nullable, unset until the first successful wake),
  `monitor_runtime.wake_interval_seconds` likewise carries no DDL default
  (nullable; stamped by every claim), and
  `monitor_runtime.wake_requested_at` carries no DDL default (nullable;
  set by `POST /api/monitor/wake`, cleared by a delivered wake and by the
  reclaim reset).
- **No-FK message columns.** `messages.from_member_id`, `messages.to_member_id`, and
  `messages.origin_message_id` are plain integer columns with **no** FK constraint;
  only `messages.owner_member_id` is FK-constrained (ON DELETE RESTRICT).
- **Soft delete** lives in `fleets.deleted_at` (a non-null timestamp marks the
  fleet deleted); this layer never physically removes rows. `messages.text` always
  stores the full untruncated body — message text is never truncated at
  persistence.
- All timestamp columns are stored as ISO-8601 text (§5.1).

### 6.2 Broker

**Scope:** the synchronous data-access layer shared by CLI and WebUI; the only
module that reads/writes the operational tables (fleets, members, placements,
messages, monitor schedule/runtime, message queries). Owns
transaction boundaries, the member-kind predicates, soft-delete + cascade, the
message status lifecycle, and the monitor single-instance claim/heartbeat/clear. It
delegates attempted inline-preview delivery through its notification trait;
the runtime adapter owns the concrete multiplexer call (§4). The broker does
not start subprocesses or depend on HTTP/CLI handlers. Its existing
process-liveness probe (signal-0) retains the monitor claim semantics.

#### Session semantics

- **read_session** — opens a read-only connection with no transaction wrapper;
  used by every query/read function.
- **write_session** — opens a connection inside a single transaction that
  commits on clean exit and rolls back on any exception. **Every mutating
  function wraps all of its writes in exactly one `write_session` block** — its
  mutations all commit together or all roll back.
- **`delete_fleet_monitor_rows`** takes an existing transaction as its first
  argument and participates in the *caller's* transaction (atomic cascade).
  Every other function opens its own session.
- "Exactly one row" reads (EXISTS / aggregate / single-row lookups) assume
  exactly one row and fail loudly if the invariant breaks. Do not coerce a
  missing row to a default.

#### Kind constants and intervals

How the broker surfaces the kind (the `is_root` SQL flag joined with the
member card's `$.cafleet.kind` marker, shared by `get_member`,
`list_members`, and `list_roster`) is detailed in §5.4.

- Liveness staleness (the monitor runtime's own heartbeat, unrelated to any
  per-member cadence): `MONITOR_STALE_FACTOR = 3`,
  `MONITOR_STALE_FLOOR_SECONDS = 15` → `stale_after = max(3·tick_seconds, 15)`.
- Root Director identity strings written by `create_fleet`: name `Director`,
  description `Root Director for this fleet`. Monitor member identity strings
  written by `create_fleet`: name `monitor`, description `Monitor member for
  this fleet`.

#### Fleets

- **`create_fleet(name, director_context, coding_agent, spawn_monitor)`** —
  bootstraps the fleet, root Director, and monitor rows in one write_session,
  with owned-pane compensation across the DB/multiplexer boundary. Order: stamp `created_at`; insert the fleet with
  `director_member_id = NULL`; insert the Director member row (`name="Director"`,
  `description="Root Director for this fleet"`, `status="active"`, card
  `{name, description, skills:[]}` with **no** `cafleet.kind`); insert the
  Director's placement (the root Director is pane-bound and keeps its own
  placement row) carrying the multiplexer identity (`mux_session`
  / `mux_window_id` / `mux_pane_id`), the `backend` (the resolved `mux.name`),
  and `coding_agent`; insert the monitor member row (`name="monitor"`,
  `description="Monitor member for this fleet"`, `status="active"`, card
  carrying the monitor marker `cafleet: {kind: "monitor"}`); back-fill the
  fleet's `director_member_id`; invoke the caller-supplied `spawn_monitor`
  callback with the three allocated ids (`fleet_id`, `director_member_id`,
  the monitor's `member_id`) — the CLI's callback performs the prompt
  substitution and pane split, transfers the successful split's pane ownership
  immediately to its CLI guard, and returns the id without intervening fallible
  work; insert the
  monitor placement row with the returned pane id (same session/window
  context as the Director; `coding_agent` = the fleet's coding agent);
  commit. A callback error triggers explicit transaction rollback; on success,
  no fleet, Director, monitor, or placement rows persist. Report rollback
  failure as a secondary diagnostic and do not claim full cancellation.
  A Herdr run failure with known id closes the pane in the backend before the
  callback error reaches the broker. A placement-insert/commit failure after
  callback success closes the DB transaction before the CLI kills its owned
  pane (§6.3 *Creation ownership and compensation*); no new broker API forces
  pane-first compensation in this case. The transaction holds
  SQLite's write lock across the callback's subprocess call, backstopped by
  the connection's `busy_timeout=5000` PRAGMA. Returns
  `{fleet_id, name, created_at, director:{…}, monitor:{…}}`.
- **`list_fleets()`** — one record `{fleet_id, director_member_id, name,
  created_at, member_count}` per non-soft-deleted fleet (`deleted_at IS NULL`);
  `member_count` counts only **active** members (0 for empty fleets). Ordering:
  **`created_at DESC, fleet_id DESC`** (newer id first when timestamps tie).
- **`get_fleet(fleet_id)`** — single-row lookup by id; **includes soft-deleted
  fleets** (no `deleted_at` filter) and exposes `deleted_at` so callers
  distinguish missing (None) from soft-deleted. Returns `{fleet_id, name,
  created_at, deleted_at, director_member_id}` or None.
- **`delete_fleet(fleet_id)`** — soft-delete + cascade-deregister in one
  write_session; **idempotent**. If the fleet row does not exist → application
  error `fleet '{fleet_id}' not found.`. Set `deleted_at = now` where
  `deleted_at IS NULL`; if zero rows updated, short-circuit return
  `{deregistered_count: 0}`. Else flip all active members to `deregistered`
  (stamping `deregistered_at`), hard-delete their placements, delete the fleet's
  monitor rows. **Messages are never deleted.** Returns `{deregistered_count}`.

#### Members — registration & lookups

- **`register_member(fleet_id, name, description, skills, placement,
  monitor=False)`** —
  pre-transaction validation, then one write_session. Pre-checks: `get_fleet`;
  if None → usage error `Fleet '{fleet_id}' not found.`; if `deleted_at` set →
  usage error `fleet {fleet_id} is deleted`. Build the card `{name, description,
  skills: skills or []}`, adding `cafleet: {kind: "monitor"}` only when
  `monitor` is true — an ordinary registration's card carries no kind
  marker. The `placement` dict carries no director id — the fleet row is the
  single source of the Director identity. Registration uses an `IMMEDIATE`
  transaction to serialize writers. Monitor uniqueness is enforced both by
  an in-transaction recheck and the partial unique index (§8), including for
  direct broker callers. The monitor-first policy for ordinary pane creation
  remains CLI-side: ordinary broker registration does not require an active
  monitor.
  Inside the transaction:
  - **Root-Director invariant guard** (only when `placement` is given): the
    fleet's `director_member_id` must reference an active member of the fleet;
    violation → application error `fleet {fleet_id}'s root Director (member
    {id}) is not active.` — a loud invariant failure, not a usage error, since
    the value is not user input. Nested teams stay impossible by
    construction: no caller supplies a director id.
  - When `monitor` is true, check `active_monitor_member_id` inside the
    transaction before inserting the member or placement. A conflicting row
    raises `ActiveMonitorExists { fleet_id, member_id }`, identifying the
    existing monitor. A unique-constraint violation is converted to this
    variant only for `idx_members_one_active_monitor_per_fleet`; unrelated
    SQL errors retain their original classification. The CLI maps the variant
    to application error (exit 1) `fleet {fleet_id} already has an active
    monitor member (member {member_id})`, without `register failed:`. A losing
    registration adds no member or placement and cannot proceed to pane
    creation, including when two CLI prechecks both saw an empty slot.
  - Insert the member row; if `placement` given, insert it. There is no
    per-member monitor enrollment — supervision cadence lives entirely on the
    fleet-scoped `monitor_runtime` row (§6.2 *Monitor — runtime claim /
    heartbeat / clear + liveness*), and the monitor role marker on the card is
    the only per-member monitor state.
- **`active_monitor_member_id(fleet_id)`** — the fleet's single active member
  whose card carries the monitor marker (`json_extract(member_card_json,
  '$.cafleet.kind') = 'monitor'` and `status = 'active'`), or `None`. This
  predicate exactly matches the unique index (§8). Consumed by registration's
  transaction check, the CLI's two `member create` monitor-role guards (§6.3),
  and the monitor loop's
  pane resolution (§6.6).
- **`get_member(member_id, fleet_id)`** — **active only**. Returns `{member_id,
  name, description, status, registered_at, kind, skills, placement}` where
  `skills` is the card's `skills` list (usually `[]`) and `kind` is one of the
  three values of §5.4: `director` (derived: `member_id ==
  fleets.director_member_id`), `monitor` (derived: the card's
  `$.cafleet.kind == "monitor"` marker), or `member`; `placement` is None if
  absent.
- **`deregister_member(member_id)`** — soft-delete one member + drop placement.
  If the member is the root Director of any fleet → **application
  error (exit 1)** `cannot deregister the root Director; use 'cafleet fleet
  delete' instead`. The root-Director guard raises a single
  **application** error (exit 1) here on the broker side and identically on the
  `cafleet member delete` CLI side (§6.3) — one error model for the same
  string and condition. Flip `active → deregistered` (stamp `deregistered_at`);
  if a row was flipped, hard-delete its placement. Returns
  `true` iff a row was flipped.
- **`update_placement_pane_id(member_id, pane_id)`** — set `mux_pane_id` for the
  member's placement; None if no placement row; else returns the placement
  projection. Called after the multiplexer resolves a spawned pane's real id.
- **`verify_member_fleet(member_id, fleet_id)`** — EXISTS check; **status-
  agnostic** (deregistered members still pass).
- **`get_member_names(member_ids)`** — returns `BTreeMap<i64, String>`, ordered
  by member id and **status-agnostic**, including deregistered members. Empty
  input executes no SQL. Deduplicate ids before querying; bind at most 500
  unique ids per `IN` query, for at most `ceil(unique_ids / 500)` queries.
  Unknown ids are absent from the map. Construct only the placeholder list
  dynamically; all ids are bound parameters. Batching preserves the existing lookup results.

#### Members — roster

- **`list_members(fleet_id)`** — every **active** registry row of the fleet:
  active rows LEFT OUTER
  JOIN `member_placements`, joined against `fleets` for the `is_root` flag that
  directly produces `kind` (§5.4). Order by `member_id ASC`. The activity query
  supplies `last_sent = MAX(created_at)` over **all** messages whose sender is
  the member, including broadcast summaries; `last_recv = MAX(created_at)`
  over unicast rows owned by the member; and `last_ack = MAX(status_timestamp)`
  over completed unicast rows owned by the member. ACK changes the latter
  without changing the send/receive creation timestamps.
  Compute `idle` against one `now` for the whole list: choose the lexicographic
  maximum non-null string from **all three** timestamps, then parse only that
  selected string with the existing lenient RFC3339 reader. All null, or an
  unparseable selected value, yields null; do not fall back to an older valid
  timestamp. Otherwise use `max(0, (now - parsed).num_seconds())`, retaining
  whole-second truncation and the existing offset/fraction parsing. Clamp
  only the final idle result; leave stored timestamps and other age outputs
  unchanged. The final zero clamp leaves the aggregate selection and
  three-timestamp maximum unchanged.
  Returns `{member_id, name,
  kind, placement, last_sent, last_recv, last_ack, idle}` per row — `kind` is
  the same three values as `get_member` (§5.4), `placement` is null for
  placementless rows. Backs `member list`.
- **`list_roster(fleet_id, *, include_message_holders=False)`** — every **active**
  registry row of the fleet: active rows LEFT OUTER
  JOIN `member_placements`, joined against `fleets` for the `is_root` flag that
  contributes to `kind` (§5.4), plus the member card's monitor marker. With
  `include_message_holders=True` (the WebUI
  roster), deregistered members that still own messages (a message exists with
  `owner_member_id = member_id`) are also returned, so
  the audit-relevant deregistered set stays visible. Returns `{member_id, name,
  description, status, registered_at, placement}` per row plus `kind` (the
  same three values as `get_member`), with
  `placement` null for placementless rows. Backs `GET /api/members`
  (`include_message_holders=True`); it is not a CLI surface. Order by
  `member_id ASC`. This lean query is separate from the activity
  query: it performs no message activity aggregates, retaining only the
  owner-message `EXISTS` needed for holder inclusion. Keep kind precedence,
  placement null versus pending pane, and the existing wire projection.

#### Messaging

- **`send_message(from_member_id, to, text)`** — one unicast message + at most
  one attempted notify, one write_session. Coerce `to` to int; on failure → value error
  `Invalid destination format: {to}`. If the sender is not an active member →
  value error `Sender member not found or not active: {from_member_id}`. The
  sender's fleet is **derived from the sender row** — no caller-supplied fleet
  exists. Find
  the destination among active members; absent → value error `Destination member
  not found: {to_id}`; in a different fleet than the sender → value error
  `members {from_member_id} and {to_id} are not in the same
  fleet.`. Build the unicast message (`owner_member_id = to_id`,
  `from_member_id = from_member_id`, `to_member_id = to_id`, `type = "unicast"`,
  `status_state = "input_required"`, `origin_message_id = null`), insert, then
  attempt the inline-preview notification via `_try_notify_recipient`. The
  persisted row holds the **full untruncated text**. Returns the send outcome:
  the unchanged `{message, notification_sent}` payload plus a separate
  `notification_error` — the retained raw multiplexer error string, present
  only when a pane notification was attempted and failed. `notification_error`
  is out-of-band caller metadata and is never inserted into the payload. The
  four cases:

  | Recipient/notification state | `notification_sent` | `notification_error` | Row state |
  |---|---|---|---|
  | Self-send | `false` | absent | `input_required` |
  | Recipient has no pane id | `false` | absent | `input_required` |
  | Pane notification succeeds | `true` | absent | `input_required` |
  | Pane notification is attempted and fails | `false` | the raw error | `input_required` |

  `send_message` still returns success for the attempted-failure row because
  the durable insert — the operation the broker owns — succeeded. The unicast
  CLI alone interprets `notification_error` as a sender-facing partial failure
  (§6.3); broadcast discards individual preview errors (below); the WebUI send
  handler ignores the field (§6.8).
- **`broadcast_message(from_member_id, text)`** — fan out one unicast
  delivery per active peer plus one `broadcast_summary` owned by the
  sender. Sender not active → value error `Sender member not found or not
  active: {from_member_id}`. The fleet is **derived from the sender row**.
  Recipients = active members in that fleet, **excluding
  the sender** (the Director **is** included); let `N` = the count of these
  recipients. Build the
  summary (`owner_member_id = member_id`, `from_member_id = member_id`, **`to_member_id =
  NULL`**, `type = "broadcast_summary"`, `status_state = "completed"`, `text =
  "Broadcast sent to {N} recipients"`), insert it, set its `origin_message_id` to
  its own `message_id` (self-referential), then insert each delivery with
  `origin_message_id = summary.message_id`. **After all deliveries are inserted (still
  inside the same write_session), call `_try_notify_recipient` once per delivery
  and set `delivered` = the count of those calls whose attempted preview
  succeeded** (a paneless or self-recipient delivery contributes 0; an
  attempted preview's individual error is discarded — only the count reflects
  it, and broadcast keeps exit 0 with no per-recipient failure schema).
  Returns a **single-element list** `[{message: <summary>,
  recipients: N, delivered}]` — `recipients` is the real recipient count `N` and
  `delivered` is the preview success count; the two diverge when any
  preview fails to land. The two values are kept as **separate fields** and never
  conflated; the CLI surfaces both (§6.3).
- **`_try_notify_recipient`** — the single inline-preview attempt, classified
  three ways: **skipped** (recipient == sender, or a paneless recipient — no
  attempt is made), **delivered** (the attempted keystroke landed), or
  **failed** (the attempted keystroke or the multiplexer resolution failed —
  the raw error string is retained). Multiplexer resolution failure (an
  unavailable or ambiguous environment, §6.5) is deferred: it is retained and
  exposed only when a pane preview is actually attempted after the insert, so
  it can never preempt the insert and never turns a self-send or no-pane skip
  into an error. On a non-skip, **truncate** the preview text to
  `settings.max_text_len` codepoints (+ a single U+2026 `…` suffix when over
  the limit) and call the multiplexer's inline-preview keystroke exactly once.
  Truncation is broker-side. The notification outcome never rolls back the
  insert and triggers no retry; it flows only into `notification_sent` +
  `notification_error` (unicast) or the broadcast `delivered` count.
- **`poll_messages(member_id)`** — un-acked deliveries for an existing member.
  If the member is not an active registry row → value error
  `Member {member_id} not found`. Then: `owner_member_id = member_id` AND
  `status_state = "input_required"`, `broadcast_summary` excluded, ordered
  `status_timestamp DESC`.
- **`ack_message(message_id)`** — transitions a message in one
  write_session; the recipient and fleet are **derived from the message row**
  — existence and state are the only guards. Load; absent → value error
  `Message {message_id} not found`.
  If `status_state` is not
  `input_required` → value error `Cannot ACK message in state {status_state}`.
  Set `status_state = "completed"` and `status_timestamp = now`.
  `input_required` is the only state a message may transition from.

#### Queries

- **`list_inbox(member_id)`** — all messages where `owner_member_id = member_id`, any
  state, `broadcast_summary` excluded, ordered `status_timestamp DESC, message_id DESC`.
- **`list_sent(member_id)`** — all messages where `from_member_id = member_id`, any
  state, `broadcast_summary` excluded, ordered `status_timestamp DESC, message_id DESC`.
- **`list_timeline(fleet_id, limit=200)`** — delivery rows (`g.type = 'unicast'`)
  joined to their **owning member's** row through `owner_member_id`, filtered to
  that member's `fleet_id`. SQL orders by `status_timestamp DESC, message_id DESC`
  and applies `LIMIT` after the delivery filter. The cap counts delivery rows,
  so it can cut through a broadcast group. Summary rows remain stored and
  available through `get_message` / `message show` and broadcast results; their
  initial `completed` state is not a recipient ACK.
- **`get_message(message_id)`** — single-message lookup; the fleet is
  **derived from the message row** — existence is the only guard. Load;
  absent → value error `Message {message_id} not found`.

#### Monitor — wake targets

- **`list_fleet_wake_targets(fleet_id)`** — one row per **active, non-Director,
  non-monitor** member of the fleet (the roster the fleet-level wake names;
  neither the root Director nor the monitor member is ever a target of the
  wake it hosts). Each row: `{member_id, name,
  coding_agent, pending_count, oldest_pending_ts}`, ordered by `member_id`
  ascending — the order the wake payload's entries render in (§6.5).
  `pending_count` counts messages with `owner_member_id = member_id`,
  `status_state = "input_required"`, `type != "broadcast_summary"`, and
  `oldest_pending_ts` is `MIN(status_timestamp)` over the same predicate set (a
  correlated scalar subquery; `None` when the member has no pending delivery).
  Feeds both the wake payload (§6.5, §6.6) and the WebUI `GET
  /api/monitor` per-member rows (§6.8, via `monitor_members_payload` below).
- **`fleet_wake_director(fleet_id)`** — the fleet's Director descriptor for
  the wake's trailing `Director:` segment: `{member_id, name, coding_agent,
  pending_count}`, `pending_count` computed by the same predicate as
  `list_fleet_wake_targets`. A live fleet always has its Director registered
  with a placement, so a missing row is a loud error, not a skip. Feeds only
  the wake payload (§6.5, §6.6) — the WebUI's `GET /api/monitor` has no
  Director row.

Supervision cadence lives entirely on the fleet-scoped `monitor_runtime` row
below — there is no per-member schedule state. Anything a member needs from
the Director travels as a plain per-event `send_message` on the ordinary
messaging path — no monitor-specific delivery state exists.

#### Monitor — runtime claim / heartbeat / clear + liveness

The `monitor_runtime` table holds **exactly one row per fleet** (PK = fleet_id)
— the single-instance slot.

- **Liveness predicate** `_is_live(row, now)`: `false` if `pid` or
  `last_tick_at` is null; `stale_after = max(3·tick_seconds, 15)`; if `now −
  last_tick_at > stale_after` → `false`; then probe the process with a signal-0
  (`kill(pid, 0)`): no-such-process → `false`, permission-denied (owned by
  another user) → `true`, success → `true`. Heartbeat freshness is
  authoritative; the process probe corroborates.
- **`claim_monitor_runtime(fleet_id, pid, tick_seconds, wake_interval_seconds,
  when)`** — atomically
  claim the slot (the SQLite write lock serializes concurrent claims). No row →
  insert (with `last_wake_at` and `wake_requested_at` null) and return `true`;
  row exists and **live**
  → return `false`; row exists but **stale** → overwrite `pid` / `started_at` /
  `tick_seconds` / `wake_interval_seconds`, reset `wake_requested_at = NULL`,
  and return `true` (reclaim) —
  **`last_wake_at` is left
  untouched by a reclaim**, so an immediately-restarted loop honors the
  remaining wake cadence instead of firing an instant wake, while the
  `wake_requested_at` reset guarantees a pending request never survives into
  a later loop instance.
  `wake_interval_seconds` is stamped in both the insert and the reclaim
  overwrite, exactly like `tick_seconds`.
- **`heartbeat_monitor_runtime(fleet_id, pid, when)`** — update `last_tick_at =
  when` **only where the current pid equals the caller's pid**; returns `true`
  iff exactly one row matched. **Ownership-checked** — `false` when the slot was
  reclaimed; that `false` is the displaced monitor's self-terminate signal.
- **`record_monitor_wake(fleet_id, when)`** — update `last_wake_at = when,
  wake_requested_at = NULL`
  for the fleet's runtime row: one unconditional `UPDATE` keyed on
  `fleet_id` — no pid parameter, no ownership check, no return value beyond
  success. Called only after a successful wake keystroke into the monitor
  member's own pane (§6.6). This is the one write that clears a pending wake
  request exactly when a wake actually fired — a scheduled wake also clears
  any pending request, because the wake the operator asked for has happened.
- **`set_monitor_wake_interval(fleet_id, wake_interval_seconds)`** — one
  ownership-free `UPDATE monitor_runtime SET wake_interval_seconds = ?1 WHERE
  fleet_id = ?2`, returning `true` iff exactly one row changed. `false` ⇔ no
  row — the fleet's monitor has never run (rows are removed only by `fleet
  delete`, so "no row" is exactly "never run"). Consumed by the WebUI
  `PATCH /api/monitor` (§6.8); the running loop obeys the new value within
  one tick (§6.6).
- **`request_monitor_wake(fleet_id, when)`** — one ownership-free
  `UPDATE monitor_runtime SET wake_requested_at = ?1 WHERE fleet_id = ?2`,
  returning `true` iff exactly one row changed; `false` ⇔ no row — mirroring
  `set_monitor_wake_interval`. Repeat calls overwrite the timestamp, so
  requests coalesce into a single wake. Consumed by the WebUI
  `POST /api/monitor/wake` (§6.8); the running loop honors the request on
  its next tick (§6.6).
- **`clear_monitor_runtime(fleet_id, pid)`** — null the slot's `pid` /
  `started_at` / `last_tick_at` **only where the current pid equals the
  caller's pid**. **Ownership-checked** → a non-owner clear is a no-op, so a
  self-terminating loser never wipes the winner's row. **`last_wake_at` and
  `wake_interval_seconds` are preserved** (never nulled by a clear) — the
  wake cadence survives a clean stop/restart cycle. `wake_requested_at` is
  likewise untouched by the clear — the claim-time reclaim reset is the
  single guard against stale requests.
- **`read_monitor_runtime(fleet_id)`** — `{fleet_id, pid, started_at,
  last_tick_at, tick_seconds, wake_interval_seconds, last_wake_at,
  wake_requested_at}` or None.
- **`monitor_is_live(fleet_id, now)`** — `false` if no row, else `_is_live`. An
  advisory pre-check for `cafleet monitor`; the atomic claim is authoritative.
- **`monitor_runtime_payload(fleet_id, now)`** — the runtime-liveness dict
  consumed by the WebUI `GET /api/monitor`: `{running, pid,
  tick_seconds, wake_interval_seconds, last_tick_at, last_tick_age_seconds,
  started_at, last_wake_at,
  last_wake_age_seconds}`, with the process fields — including `last_wake_at` /
  `last_wake_age_seconds` — null when the monitor is not live (no row, or a
  stale/cleared heartbeat), matching the `last_tick_at` masking rule.
  `wake_interval_seconds` follows the `tick_seconds` preservation rule
  instead: the row's value when a row exists (even stale or cleared; `null`
  when the row predates the column and was never re-stamped), else `null`.
- **`monitor_members_payload(fleet_id, now)`** — the per-member rows consumed
  by the WebUI `GET /api/monitor`: one dict per `list_fleet_wake_targets` row —
  `{member_id, name, pending_count, oldest_pending_ts,
  oldest_pending_age_seconds}` (`coding_agent` is dropped at this wire layer —
  it feeds only the wake payload, §6.5) — with
  `oldest_pending_age_seconds` computed against the single supplied `now`
  (whole seconds, integer-truncated; `None` when the source timestamp is
  `None`).
- **`delete_fleet_monitor_rows(session, fleet_id)`** (in caller's
  transaction) — in-transaction cascade delete: deletes the fleet's
  `monitor_runtime` row.

#### Soft-delete + cascade summary

- Members and fleets are **never row-deleted** — they flip to
  `status="deregistered"` / `deleted_at` set.
- Placements and the fleet's `monitor_runtime` row are hard-deleted by their
  explicit lifecycle owners.
- **Messages are never deleted** — audit history is permanent.
- Deregistered members remain visible via `verify_member_fleet`,
  `get_member_names` (both status-agnostic), and
  `list_roster(include_message_holders=True)` (when they still own messages); they
  are hidden from `get_member` and `list_members` (active-only).

#### Contract error strings → exception class → exit code

Usage-class → exit 2; application-class → exit 1; value errors are
raised by messaging/queries and translated by the caller (CLI → exit 1, WebUI →
HTTP status). The exit-code policy is
§7.2; the strings below are the broker's contract.

| Function | Class | Message |
|---|---|---|
| `register_member` | usage | `Fleet '{fleet_id}' not found.` |
| `register_member` | usage | `fleet {fleet_id} is deleted` |
| `register_member` | application | `fleet {fleet_id}'s root Director (member {id}) is not active.` |
| `register_member` | `ActiveMonitorExists`, mapped to application | `fleet {fleet_id} already has an active monitor member (member {member_id})` |
| `deregister_member` | application | `cannot deregister the root Director; use 'cafleet fleet delete' instead` |
| `delete_fleet` | application | `fleet '{fleet_id}' not found.` |
| `send_message` | value | `Invalid destination format: {to}` |
| `send_message` | value | `Sender member not found or not active: {from_member_id}` |
| `send_message` | value | `Destination member not found: {to_id}` |
| `send_message` | value | `members {from_member_id} and {to_id} are not in the same fleet.` |
| `broadcast_message` | value | `Sender member not found or not active: {from_member_id}` |
| `poll_messages` | value | `Member {member_id} not found` |
| `ack_message` | value | `Message {message_id} not found` |
| `ack_message` | value | `Cannot ACK message in state {status_state}` |
| `get_message` | value | `Message {message_id} not found` |

### 6.3 CLI

**Scope:** the entire `cafleet` command tree (16 subcommands across 3 groups +
4 top-level commands, `monitor` two-form — §1, §10), the shared argument
rules, and the `member
create` spawn orchestration + rollback ladder. Orchestration glue only — it
wires broker/multiplexer/output/coding-agent. The command/option checklist is
§10; this section gives the per-command semantics. Exit codes are §7.2;
application errors (exit 1) and usage errors (exit 2) are printed as `Error:
<message>` to stderr (usage errors additionally print a usage line).

Framework-generated parse errors — usage banners, missing-required-argument,
invalid-value / invalid-integer, unknown-argument, and unexpected-argument
renderings — are **clap's native renderings**. For these, the contract is the
exit code (2) and the identity of the offending flag or argument, not the exact
text. Cafleet-authored strings (every error string quoted in this document)
remain byte-exact.

#### Global options & top-level group

The top-level command is `cafleet`, group help `CAFleet — CLI for the message
broker and member registry.`. One option lives before any subcommand:

- `--version` — prints `cafleet <version>` and exits 0, short-circuiting before
  subcommand dispatch, so no subcommand argument validation runs.

Any other pre-subcommand option — including `--json` — is the parser's
unknown-argument usage error (clap's native rendering, exit 2).

#### Positional subject ids

The id a command acts on — its **subject** — is a **required positional
argument**; ids that describe a relationship rather than the subject stay as
flags. The positional subjects:

- `FLEET_ID` — on `fleet show`, `fleet delete`, `member list`, and both
  `monitor` forms (the fleet is the subject of the listing / the supervision
  loop / the batch scan).
- `MEMBER_ID` — on `member show` / `delete` / `prompt` / `ping` / `capture`
  (the target) and `message poll` (the requester).
- `MESSAGE_ID` — on `message ack` and `message show`.

Every positional subject is an integer: a non-integer value is clap's native
invalid-value usage error, a missing subject its native
missing-required-argument error (both exit 2). There is **no environment
default** — a spawned member reads its ids from the literal `FLEET ID:` /
`YOUR MEMBER ID:` / `DIRECTOR MEMBER ID:` lines the CLI rendered into its
spawn prompt (the placeholder substitution below). The fleet is never restated
where it is derivable: member ids are globally unique, so the member row names
its fleet, and every message row names its recipient and (via its endpoints)
its fleet (§6.2).

The **relationship flags** — ids that label a role rather than name the
subject:

- `--fleet-id` — **only** on `member create` (integer, required): the fleet
  the new member joins; the subject of the command is the member being
  created.
- `--from-member-id` / `--to-member-id` — required integers naming both
  parties of a two-party command: `--from-member-id` is the sender on `message
  send` and `message broadcast`; `--to-member-id` is the recipient on `message
  send`. Help texts: `Sender's member ID` / `Recipient member ID`.

Every other subcommand rejects `--fleet-id` with the parser's unknown-option
error (exit 2).

#### The shared `--json` flag

- `--json` — boolean, default `false`, help `Output in JSON format.`; a shared
  per-subcommand flag, canonically written **trailing**, after all other
  arguments. On every `message` subcommand; `member create` / `delete` /
  `show` / `list` / `prompt` / `ping` / `capture`; `fleet create` / `list` /
  `show` / `delete`; `monitor scan`; and `doctor`. Emits compact single-line
  JSON instead of
  text. JSON is always the **complete, untruncated machine form** — full
  envelopes, full message bodies; text output is always the human/pane form,
  truncated per §6.4. `--json` is the **only** output switch in the tree.

#### Shared positional-`TEXT` / `--file` body input {#text-body-input}

`message send`, `message broadcast`, and `member create` take their text body
as a **positional argument** (`TEXT`; named `PROMPT` on `member create`) with
`--file PATH` as the alternative, resolved through **one shared reader**.
Exactly one of the positional and `--file` must be supplied, enforced as a
clap-native required argument group (the positional and `--file` conflict;
supplying neither or both is clap's native rendering, exit 2 — the contract is
the exit code and the offending arguments, not the exact text). Resolution:

- Positional `<s>` → the body is `s` verbatim; empty or whitespace-only →
  usage error (exit 2) `text may not be empty.`.
- `--file -` → the whole body is read from stdin (read to EOF, decoded
  UTF-8); empty or whitespace-only stdin → application error (exit 1)
  `--file -: stdin is empty.`.
- `--file <path>` → the file is read as **raw bytes and decoded UTF-8 with
  no universal-newline translation** (CRLF/CR survive byte-for-byte); an
  absolute path is used as-is, a relative path resolves against CWD. Error
  surfaces (all application errors, exit 1, keyed on `--file`, riding the
  read-bytes exception surface with **no** `is_file()` pre-check so a permission
  failure lands correctly): missing / non-regular file → `--file <path>:
  file does not exist or is not a regular file.`; unreadable → `--file
  <path>: file is not readable.`; invalid UTF-8 → `--file <path>: file is
  not valid UTF-8.`; empty or whitespace-only file → `--file <path>: file
  is empty.`.

The body is returned **verbatim** (no stripping). Empty-body rejection is
**uniform** across all three commands and across inline / file / stdin. Long or
multi-line bodies use `--file` (or `-` stdin) to bypass the shell's
`ARG_MAX` limit.

#### Shared `message` handler sequence

Every `message` leaf handler (which returns a broker result) follows one
shared sequence, configured per command by a **required** text renderer. The
broker derives the fleet and recipient from the subject row (§6.2) — no
fleet-gate runs CLI-side. Per invocation, in order:

1. **Handler call.**
2. **Emit branch** — if the subcommand's `--json` flag was passed, emit the
   broker result as compact JSON — the complete, untruncated typed-column
   envelopes; else route the result through message truncation (§6.4,
   `max_text_len`) and call the text renderer.
3. **Exception wrap** — re-raise an application/usage error unchanged; wrap any
   other exception as an application error (exit 1) carrying its message.

#### `doctor`

Only the shared `--json` flag. A full-environment diagnosis that renders
**all** sections even when the multiplexer is unavailable or the database is
missing or stale — no early abort. Diagnosis order: multiplexer, database,
coding agents. `doctor` is exempt from the schema-version and stale-assets
guards — it reports instead of blocking.

**Text layout.** The first output line of the whole report is `cafleet
<version>`. Each section is led by a single-width verdict glyph (`✓` U+2713 /
`✗` U+2717) plus the section name; detail lines are indented two spaces
beneath. A worked example (at-head schema, one stale agent, one
superseded record):

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

**Multiplexer section.** Resolves the active backend via
`resolve_multiplexer()`, ensures it is available (`ensure_available()`),
discovers the pane context (`context_discovery()`), and reads the backend's
presence env var (`TMUX` for tmux, `HERDR_ENV` for herdr). `✓` with the five
detail lines (`backend`, `session`, `window_id`, `pane_id`, `presence`). On
any multiplexer or environment failure (no supported multiplexer, ambiguous
environment, binary not on `PATH`, pane not discoverable): `✗ multiplexer`
with the resolver's error message as the single detail line, and the report
continues. One issue.

**Database section.** One detail line; the five states (`<M>` recorded
version, `<N>` embedded head):

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

**Coding agents section.** A light box-drawing framed table
(`┌ ─ ┬ ┐ │ ├ ┼ ┤ └ ┴ ┘`), header separator only, no per-row rules. Column
alignment uses **display width** (Unicode display-width, e.g. via a
unicode-width facility), never byte length — the glyphs and `→` are
single-width, but the rule is general. One row per agent in the fixed order
`claude`, `codex`, `opencode`.

| Column | Content |
|---|---|
| `coding agent` | The agent name. |
| `path` | The resolved identity path (§6.3 *Config-dir resolution*), with `~` abbreviation when under `$HOME`. On a resolution error: the raw invalid variable value. |
| `source` | The winning origin: `$<VAR>` (e.g. `$CLAUDE_CONFIG_DIR`) or `default`. |
| `setup` | The state below, keyed on the **resolved path only**. |

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

**Footer and exit code.** Last line: `no issues found`, `1 issue found`, or
`<N> issues found` (proper pluralization). Exit code: 0 when no issues, 1
otherwise — the `–` state and footnotes never count. No failure exits before
output; every failure is a rendered issue.

**JSON mode.** Mirrors the sections with `ok` booleans, unabbreviated
absolute paths, and the issue count. `source` holds the winning env-var
**name** (no `$`) or the literal `"default"`; `state` is `"ok" | "stale" |
"not_installed" | "error"` (`"error"` is the per-agent resolution-error
state; `"not_installed"` never contributes to `issues`). Every agent row
carries the same keys. `error` is the validation message for `"error"`, and otherwise null,
without an `Error: ` prefix.
Section `error` fields likewise hold the detail text when `ok` is false,
else `null`.

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
and path errors remain
`"error"`. Recorded fields are null and `superseded` is empty. `installed_at` values are printed **verbatim** (microsecond
precision, exactly as stored). Exit-code semantics are identical to text
mode.

#### `fleet` group

Does **not** follow the shared `message` handler sequence. All four `fleet`
subcommands take the shared `--json` flag and emit JSON when it is set.
`fleet show` and `fleet delete` take the positional `FLEET_ID` subject (§6.3
*Positional subject ids*).

- **create** — `--name` (string, **required**), `--coding-agent`
  (choice over the coding-agent names, **required**; the monitor member
  inherits this backend by construction — there is no
  `--monitor-coding-agent`), `--monitor-file PATH` (**required**; a UTF-8
  file whose contents are the monitor's spawn prompt, `-` = stdin; the same
  body semantics as `member create --file`, with the rejection strings
  naming the flag label `--monitor-file`; no inline positional form),
  `--monitor-model MODEL` (optional; validated by the `--coding-agent`
  backend exactly as `member create --model`; omitted → the backend's own
  default model), `--json` (shared). Omitting any required flag → clap's
  native missing-required-argument error naming the flag, exit 2.
  One invocation creates fleet/Director/monitor rows transactionally and
  compensates an owned pane on failure. Ladder: (1) multiplexer
  preconditions — on a `MultiplexerError` → application error `cafleet
  fleet create must be run inside a tmux or herdr session` (exit 1, no DB
  writes); (2) resolve the monitor prompt body from `--monitor-file`;
  (3) backend checks before any write — backend lookup, `validate_model`
  on `--monitor-model`, `ensure_available`; (4) broker `create_fleet`
  (§6.2) with a callback that substitutes the four identity placeholders
  (substitution errors → the two `member create` primary strings, exit 2,
  transaction rollback attempted with any failure reported) and spawns the monitor pane detached
  (`display_name="monitor"`, the `--monitor-model` value, no effort,
  `CAFLEET_DATABASE_URL` as the only forwarded environment variable;
  `split_window` failure → application error with primary reason
  `tmux split-window failed: <detail>`, exit 1). On split success the callback
  immediately arms the CLI pane guard and returns the id. Herdr run failure
  inside the callback is compensated backend-side before DB rollback;
  placement-insert/commit failure after callback success rolls back and
  closes the DB transaction first, then kills the CLI-owned pane. Cleanup
  diagnostics follow the primary reason; `Rolled back fleet creation.` is
  a confirmed-compensation suffix, never an unconditional failure claim.
  If the pane id was not obtained, report unknown/unconfirmed pane cleanup
  and never guess a pane to kill. After commit success, disarm every creation
  guard before the existing text/JSON `emit` boundary. Full failure ordering
  and rollback-failure reporting are in § *Creation ownership and compensation*.

- **list** — `--json` (shared). Empty → `No fleets found.`; else a header plus
  one formatted row per fleet (five columns: FLEET_ID / DIRECTOR / NAME / MEMBERS
  left-padded 40 / 40 / 20 / 8, then a trailing unpadded CREATED_AT; nullable
  cells fall back to empty strings).
- **show** — positional `FLEET_ID` + the shared `--json`. Not found →
  application error `fleet '<fleet_id>' not found.`. Text: `fleet_id`, `name`,
  `created_at`, plus a `deleted_at:` line when soft-deleted (soft-deleted rows
  are returned intentionally).
- **delete** — positional `FLEET_ID` + the shared `--json`. Text: `Deleted
  fleet <fleet_id>. Deregistered <n> members.`; JSON: the broker result
  `{"deregistered_count": <n>}`. Idempotent (an already-deleted
  fleet reports 0 members).

#### `message` group

All five follow the shared handler sequence above and take the shared
`--json` flag. Subjects and relationship flags: positional `MEMBER_ID` (the
requester) on `poll`; positional `MESSAGE_ID` on `ack` / `show`;
`--from-member-id` (integer, required — the sender) on `send` / `broadcast`;
`--to-member-id` (integer, required — the recipient) on `send`. The broker
derives everything else from those rows (§6.2): the fleet from the sender row
on `send` / `broadcast`, the recipient and fleet from the message row on
`ack` / `show`.

- **send** — `--from-member-id`, `--to-member-id`, and the shared body input
  (positional `TEXT` or `--file PATH`; §6.3
  [text-body input](#text-body-input)). A sender/recipient pair from
  different fleets → the broker's cross-fleet error (§6.2)
  `members <from> and <to> are not in the same fleet.` (exit 1). Text:
  `Message sent.\n` + the formatted message.

  **Partial failure.** Before the success emit, the handler checks the send
  outcome's `notification_error` (§6.2). When present — the row was persisted
  but the attempted pane notification failed — it raises an application error
  (exit 1) with this cafleet-authored message (the top-level handler supplies
  the `Error: ` prefix):

  ```
  Message <message-id> was persisted, but pane notification failed: <raw backend error>. Do not resend this message. Recover the recipient pane, then run 'cafleet member ping <recipient-id>' or have the recipient run 'cafleet message poll <recipient-id>'.
  ```

  `<raw backend error>` is inserted verbatim and may contain the backend
  command, its payload argv, and a newline-delimited stderr detail (§6.5); the
  formatter adds no separate copy of the sent message body. stdout stays
  empty; `--json` follows the global error contract — it selects successful
  command output only and creates no JSON error envelope, so both modes emit
  the same stderr text. The intentional skips (self-send, no-pane recipient)
  keep their exit-0 success output with `notification_sent: false`, and a
  successful attempted notification keeps the byte-identical success contract
  in both modes. No layer retries the notification.
- **broadcast** — `--from-member-id` and the shared body input (positional
  `TEXT` or `--file PATH`; §6.3 [text-body input](#text-body-input)). The
  result is a list; text is `broadcast
  id=<message_id> recipients=<N> delivered=<k>`, where `<N>` is the result's
  `recipients` (the real recipient count, matching `Broadcast sent to {N}
  recipients`) and `<k>` is the result's `delivered` (the count of attempted
  inline previews that landed; individual preview errors are discarded, §6.2).
  The two diverge when any preview fails to deliver;
  they are reported as **separate fields**, not conflated (the broker computes
  both, §6.2). In JSON mode the result object carries both `recipients` and
  `delivered`.
- **poll** — positional `MEMBER_ID`; an unknown or inactive requester → the
  broker's existence error `Member <member_id> not found` (exit 1); indexed
  message list; empty `No messages found.`.
- **ack** — positional `MESSAGE_ID`; existence + `input_required` state are
  the only guards (§6.2). Text: `Message acknowledged.\n` + the formatted
  message.
- **show** — positional `MESSAGE_ID`; existence is the only guard. Text is
  the formatted message.

#### `member` group — shared resolution helpers

These helpers back the `member` subcommands. The target member is named by
the positional `MEMBER_ID` subject (§6.3 *Positional subject ids*); the fleet
is derived from the member row.

- **Require-pane** — given a placement and an action label
  (`capture`/`prompt`), no pane id → application error `member
  <member_id> has no pane yet (pending placement) — nothing to <action>.`.
  `member ping` does not use it — a pending placement takes ping's skip path.
- **Load-member** — fetch the member by id: not found →
  `Member <member_id> not found`; other fetch failure → `failed to fetch member:
  <error>`; absent placement → application error ``member <member_id> has no
  placement row; it was not spawned via `cafleet member create`.``, unless the
  caller opts into tolerating a missing placement (`member show` and `member
  delete` do — a placementless target resolves successfully). Does **not**
  check pane presence (`member delete` tolerates a pending
  placement). Callers re-fetch by the canonical
  member id.
- **Registration compensation** — an owned registration guard attempts
  deregistration and placement removal. Preserve the primary error's category
  and message; report failure as `cleanup failed for member <id>: <detail>`
  after it on stderr. A `Rolled back registration of <new_member_id>.` suffix
  describes confirmed compensation only, never a failed or unconfirmed
  cleanup. Explicit `finish`/`rollback` disarms the guard, so it does not
  deregister twice; see § *Creation ownership and compensation*.
- **Resolve-coding-agent** — explicit `--coding-agent` wins; else (flag
  omitted) inherit the Director's placement coding agent, with three
  error surfaces (Director fetch failure / not found / no placement), each
  prefixed `cannot resolve the member's coding agent:` and ending
  `Re-run with an explicit --coding-agent.`.

#### `member create` — spawn orchestration & rollback ladder

The one genuinely distinct lifecycle op: register **and** spawn a pane. It
takes **no identity flag** — the acting Director is auto-resolved from the
fleet row. Arguments: `--fleet-id` (integer, required — the fleet the new
member joins, §6.3 *Positional subject ids*), `--name` (string, required),
`--description` (string,
required), `--coding-agent` (choice, optional — omitted → inherit the
Director's placement backend; the help default text reads `inherits the
Director's backend`),
`--model` (string, optional), `--effort` (string, optional — reasoning-effort
level, validated per backend; help text `Reasoning-effort level (claude, codex
only).`), `--role` (choice, optional — the sole accepted value is `monitor`;
any other value is the parser's native invalid-value error, exit 2; omitted →
the member registers as an ordinary member), the shared body input (positional
`PROMPT` or `--file PATH`,
exactly one; §6.3 [text-body input](#text-body-input)), and the shared
`--json`.
A `member create` without `--role` registers an ordinary member; with
`--role monitor` it registers the fleet's monitor member — the mid-run
recovery path for re-spawning a dead monitor; the bootstrap monitor is
spawned by `fleet create` (§6.3 `fleet` group). The member `kind`
union is `"director" | "monitor" | "member"` (§5.4), with `director` reserved
for the fleet's single root Director bootstrapped by `fleet create` — no
`member create` invocation, with or without `--role`, can produce a
`director` kind. Sequence:

1. Read `fleet_id`; **auto-resolve the Director** from `broker.get_fleet`,
   first thing: fleet missing → usage error (exit 2) `Fleet '<fleet-id>' not
   found.`; soft-deleted → application error (exit 1) `fleet <fleet-id> is
   deleted`; `director_member_id` NULL (mid-bootstrap corruption) → application
   error (exit 1) `fleet <fleet-id> has no root Director recorded; re-create
   the fleet with 'cafleet fleet create'.`. The resolved id feeds the monitor
   backend inheritance and the spawn-prompt substitution; no override flag
   exists. Then resolve the coding agent; look up the backend.
2. **Model and effort validation** — validate `--model`, then `--effort`
   (`validate_model` then `validate_effort`); a failure → usage error (exit 2)
   with the backend's message, **before any registration or tmux side effect**.
3. **Monitor-role guards** — evaluated CLI-side for early diagnostics,
   before any registration or pane effect, one-per-fleet guard first: `--role monitor` into a fleet that already has an active
   monitor member (via the broker's `active_monitor_member_id`, §6.2) →
   application error `fleet <fleet-id> already has an active monitor member
   (member <member-id>)`; no `--role` (an ordinary member) into a fleet with
   no active monitor member → application error `fleet <fleet-id> has no
   active monitor member; spawn one with --role monitor first`.
4. **Resolve the body** — via the shared positional-`PROMPT` / `--file` reader
   (§6.3 [text-body input](#text-body-input)): exactly-one enforced at parse
   time (clap-native, exit 2), `-` stdin, abs /
   CWD-relative path, UTF-8, uniform empty-body rejection. An empty inline
   body is a usage error (exit 2); a file / stdin surface is an
   application error (exit 1). Resolved **before any registration or tmux side
   effect**; substitution (step 7) is deferred until the new member id exists.
5. **Preconditions** — ensure tmux available, the backend binary on PATH, and
   discover the tmux context; any tmux/runtime error → application error (exit
   1).
6. **Register the member** — with a placement carrying the tmux session, tmux
   window id, an unset pane id, and the coding agent (no director id — the
   fleet row is the single source), passing `--role`'s presence through as
   `register_member`'s `monitor` boolean (§6.2) so a monitor registration's
   card carries the `$.cafleet.kind == "monitor"` marker. Registration's
   `IMMEDIATE` transaction rechecks the active monitor, and the DB unique
   index backstops all writers (§6.2). Map `ActiveMonitorExists` to the same
   one-per-fleet application error as step 3, with no member, placement, or
   pane added by the losing registration. Re-raise an application error
   verbatim (preserves the root-Director invariant guard); wrap any other exception as
   `register failed: <error>`. Capture the new member id.
7. **Substitute placeholders** (below) — run the Rust spawn-placeholder mini-formatter over the resolved body
   (step 4), substituting `{fleet_id}` / `{member_id}` (the new member id from
   step 6) / `{director_member_id}` (the auto-resolved Director) /
   `{coding_agent}`. An unknown-placeholder or
   malformed-brace error is a usage error (exit 2); on it,
   use the registration guard to deregister and remove placement, then
   return the original usage error and exit 2, appending any cleanup failure
   after the original cause. No pane guard exists yet.
8. **Build the spawn argv** from the backend (the rendered prompt from step 7,
   display name, model, effort).
9. **Split the pane** — split the window to obtain the pane id. The only
   forwarded env var is `CAFLEET_DATABASE_URL` (when set); identity travels in
   the rendered prompt (step 7), not the environment. The backend owns the
   pane until successful return, then immediately transfers it to the CLI
   pane guard. A split failure retains the primary reason
   `tmux split-window failed: <error>` and compensates the registration; the
   CLI never repeats a backend's recorded pane-cleanup attempt.
10. **Patch the pane id** — record it on the placement. On exception, handle
    the failed SQL, then kill the owned pane and deregister, preserving primary
    reason `placement update failed: <error>`. If the placement vanished, use
    the same compensation order with reason `placement row vanished before
    pane-id patch`. Use `kill_pane(id, true)`, not `send_exit`.
11. **Emit** — once the placement is confirmed, disarm all creation guards,
    attach the placement view, then use the existing JSON (complete result)
    or compact spawned-member text formatter (`format_member`, §6.4).

#### Creation ownership and compensation

A backend owns a newly allocated pane until successful return, then the CLI
owns it. A known-pane failure attempts cleanup exactly once; an unknown pane
id is reported without guessing. Fleet failure rolls back and closes its DB
connection before CLI pane cleanup. Member placement failure cleans the pane
then deregisters the member. Preserve the primary error and append cleanup
failures; successful creation disarms cleanup.

#### `member delete`

The pane-teardown + registry-soft-delete op. Arguments: positional
`MEMBER_ID` (the **target**) + the shared `--json`. The tmux precondition
fires only on the
pane-teardown path (live pane id) — a placementless or pending-placement
delete is a pure registry operation and succeeds outside tmux.

1. **Root-Director guard, before any pane mutation** — fetch the fleet; if the
   target is the fleet's Director → application error (exit 1) `cannot deregister
   the root Director; use 'cafleet fleet delete' instead` (the same string and
   exit code the broker's `deregister_member` guard raises, §6.2).
2. Load the member **tolerating a missing placement**; re-fetch
   the canonical id and read the pane id (absent when placementless or
   pending).
3. **No placement row** — registry soft-delete via the broker (a failure →
   application error `deregister failed: <error>`). Success: header `Member
   deleted.`, pane status `(no placement)`, exit 0. No multiplexer requirement.
4. **Pending placement** (no pane yet) — registry soft-delete via
   the broker (a failure → application error `deregister failed: <error>`).
   Success: header `Member deleted.`, pane status `(pending — no pane)`, exit 0.
   No multiplexer requirement.
5. **Has pane** — ensure the resolved multiplexer available, then kill the pane
   immediately (tolerating a missing pane); a multiplexer error → application
   error `kill_pane failed for pane <pane_id>: <error>. The <backend> server may
   be unreachable. Verify with 'cafleet doctor', then re-run the command.`
   (`<backend>` is the resolved `mux.name`, `tmux`/`herdr`). Then deregister;
   header `Member deleted.`, pane status `<pane_id> (killed)`, exit 0.

Success text: the header line plus indented `member_id:` / `pane_id:` lines;
JSON: `{member_id, pane_status}`.

#### `member show`

Registry read — no tmux requirement and no requester gate. Arguments:
positional `MEMBER_ID` (the **target**; any active
registry entry, placed or placementless, the root Director
included) + the shared `--json`. Load the target
tolerating a missing placement: unknown / inactive → application
error `Member <member_id> not found`. JSON emits the broker `get_member` dict
unchanged (§6.2) — the detailed view; text renders via `format_member_detail`
(§6.4) — the compact `<member_id> <name> <status>` line.

#### `member list`

Arguments: positional `FLEET_ID` (the fleet is the subject of the listing) +
the shared `--json`; no
identity flag. Lists every **active registry entry** of the fleet via
`list_members` (§6.2) — the root Director, ordinary members, and placementless
rows. Empty case `0 members.`; else the header is
`<N> members:` and the table renders one row per member with `member_id`,
`name`, `kind` (the three `get_member` values, §5.4), `backend` (the placement's
`coding_agent`), `pane_id` (`(pending)` when unset), and the humanized `idle`
columns (§6.4 `format_member_list`); a placementless row renders `-` in the
`backend` and `pane_id` cells. JSON emits
the raw `list_members` rows (`member_id`, `name`, `kind`, `placement` — null
when placementless — `last_sent`, `last_recv`, `last_ack`, `idle`).

#### `member prompt`

Arguments: positional `MEMBER_ID` (first), **positional** `TEXT` (string,
required, second), `--shell` (boolean flag, default
`false`), and the shared `--json`. `TEXT` has no `--file` alternative — its
body is a one-line keystroke by contract. A newline/CR → usage error
`text may not contain newlines.`; empty after trim → usage error `text may not
be empty.`; then trim. Ensure tmux, load the member, require a pane (`prompt`).
Dispatch via the multiplexer's `send_prompt` (§6.5): the plain form delivers
the text Esc-safeguarded as a submitted user turn; the `--shell` form delivers
`! <text>` with the same Esc safeguard via the coding agent's `!` shell shortcut
(a tmux error →
application error `send failed: <error>`). The flag performs no content
inspection — plain-form text beginning with `!` is delivered verbatim. JSON:
`{member_id, pane_id, text, shell}`; text: `Sent prompt <quoted-text> to
member <name> (<pane_id>).`, or with `--shell` `Sent shell prompt
<quoted-text> to member <name> (<pane_id>).` (the text rendered with
human-readable quoting/escaping — reproducing the quoted intent is
sufficient).

#### `member ping`

Re-pokes a member's inbox. Arguments: positional `MEMBER_ID` (the
**target**) + the shared `--json`.
Ensure tmux and load the target (a missing placement row is
still the hard error of the shared loader). A **pending placement** (a
placement row with no pane id) takes the **skip path**: no keystroke is sent
and the command exits 0 — text `Member <name> has no pane yet (pending
placement) — ping skipped; it will poll its inbox on spawn.`, JSON
`{"member_id": <id>, "pane_id": null, "skipped": true}`. With a pane, inject
the inbox-poll keystroke via the multiplexer's
`send_poll_trigger`, which is **best-effort** (§6.5) — it returns a boolean and
never raises. The keystroked payload carries a resume clause: `cafleet message
poll <member_id> — then resume your work if
something was still running.`. A returned `false` (non-delivery) → application
error `send failed: tmux send-keys did not deliver the poll-trigger keystroke
to pane <pane_id>.`. Because `send_poll_trigger` swallows its own `TmuxError`
and returns `false`, the only reachable failure surface is the non-delivery
message above. JSON: `{member_id, pane_id, skipped}` — the `skipped` key is
present on **both** success paths (`false` on a dispatched ping); text: `Pinged
member <name> (<pane_id>) — poll keystroke dispatched.`.

#### `member capture`

Pane read. Arguments: positional `MEMBER_ID` (the **target**), `--lines`
(integer, default
**20**, shown in help), `--ansi` (boolean, default `false` — preserve ANSI
escapes; stripping is the default), plus the shared `--json`. Ensure tmux,
load the member (the shared
`member`-group loader), require a pane (`capture`) — the same guards as the
rest of the `member` group. Capture the last N lines
(a tmux error → application error `capture failed: <error>`). When `--ansi`
is not set, strip ANSI. JSON begins `{member_id, pane_id, lines, content,
...}`; text emits the content **only**, with no trailing newline, **preserving
ANSI even on a non-TTY sink** when `--ansi` is set. JSON adds `captured_at`,
stamped from local UTC at the capture read boundary, and
`content_sha256 = sha256(content.encode("utf-8"))`, in key order after
`content`. The hash is mode-exact: no-ANSI hashes the stripped,
carriage-return-defragmented emitted string; ANSI hashes the preserving
emitted string. Text output stays byte-identical. Capture content is not
stored.

#### `monitor`

A two-form top-level command: the bare positional form runs the supervision
loop; the `scan` subcommand is a one-shot batch capture. The loop positionals
and the subcommand are mutually exclusive (args-conflict-with-subcommands
parsing): `cafleet monitor <FLEET_ID>` parses the loop form whenever no
subcommand is given, and `cafleet monitor scan <FLEET_ID>` dispatches the
subcommand (`FLEET_ID` is an integer and `scan` is not, so the two forms
cannot collide).

**The loop form** — `cafleet monitor FLEET_ID [--tick N] [--interval N]` —
with the positional `FLEET_ID` subject. A
`_require_live_fleet` guard fetches the fleet; missing or soft-deleted
→ application error `fleet <fleet_id> not found`.
`--tick` (integer ≥1, default 5, shown in help) and `--interval`
(a non-negative 64-bit integer, parser-enforced `0..=i64::MAX` — a negative
or above-`i64::MAX` value fails the parser's standard invalid-value error,
exit 2; optional; when omitted, falls back to
`CAFLEET_MONITOR_WAKE_INTERVAL` §7.1, default 600). `--interval 0` disables
the wake while the loop keeps heartbeating every tick. The
startup-resolved interval is stamped into the fleet's `monitor_runtime` row
by the claim and re-read on every tick (§6.6), so a `PATCH /api/monitor`
edit (§6.8) changes a running loop's cadence within one tick. Requires a
live fleet, then tmux. Runs the monitor loop in-process (blocking). The fleet's
monitor member hosts that command as a backend-resolved long-lived execution
in its own pane immediately after the pane boots (the pane is spawned by the
`cafleet fleet create` bootstrap, before any ordinary member; `cafleet member
create --role monitor` is the mid-run re-spawn path).
Immediately after the successful runtime claim, before the first tick, the
loop prints the startup line the monitor member confirms before sending the
`monitor live` gate signal to the Director — the signal that unblocks the
Director's first ordinary
`cafleet member create`: `monitor loop started (fleet <fleet_id>, tick <tick>s, pid <pid>)`.

##### `monitor scan`

Shared capture uses `CaptureSnapshot::from_raw(raw, ansi, now)` for both
member capture and scan. ANSI false applies the existing strip/CR normalization;
true uses raw content. Hash the final string's UTF-8 bytes to lowercase SHA256
hex, retaining the existing timestamp format and line-input/windowing contract.
Text member capture adds no newline. Scan carries typed success/error results;
only its text presenter builds headings, and JSON keeps the exact existing
key order and error-null behavior. No capture content is stored in SQLite.

`cafleet monitor scan FLEET_ID [--lines N] [--ansi] [--json]` — capture the
Director's pane plus every active member's pane once, print, exit. No loop
runs, no `monitor_runtime` row is claimed, and the command performs no DB
writes; capture content is never stored in SQLite. Options: `--lines`
(integer ≥1, default **20**, shown in help — trailing lines captured per
pane), `--ansi` (boolean, default `false` — preserve ANSI escapes in every
captured content; stripping is the default, as in `member capture`), plus the
shared `--json`.

Guards, in order (the same as the loop form): require a live fleet (missing
or soft-deleted → application error `fleet <fleet_id> not found`), then
resolve the multiplexer (`ensure_available` failure → exit 1).

**Roster.** The Director's row first, then every other active member owning a
placement row, ascending by `member_id`. A member with no placement row (not
spawned via `member create`) is excluded, mirroring the wake roster's join; a
placement row with a `NULL` pane (pending placement) stays in the roster as
an annotated entry. A fleet with no members scans the Director's pane only.

**Per-entry capture**, in roster order: a `NULL` pane → annotated entry
`pane not available (pending placement)`; a `capture_pane` error (dead pane,
backend failure — including the Director's own pane) → annotated entry
`capture failed: <error>`; success → content (ANSI-stripped unless `--ansi`),
`captured_at` stamped from local UTC at that entry's read boundary, and
`content_sha256 = sha256(content.encode("utf-8"))` over the exact emitted
content (mode-exact, as in `member capture`). The scan always completes: an
annotated entry never aborts the remaining captures, and a scan whose every
entry is annotated still exits 0.

**Text mode** — one section per roster entry, separated by one blank line.
`<name>` is the raw DB value (stdout is not a keystroke path, so no
sanitization). `kind` is `director` or `member`:

```
=== <member-id> (<name>; kind=<kind>; coding_agent=<coding_agent>; pane=<pane-id>; captured_at=<ts>) ===
<content>
```

An annotated entry drops `captured_at` from the header and carries the
annotation as its body; the pane token is `—` when no pane exists, and a
failed capture keeps its real pane id:

```
=== <member-id> (<name>; kind=<kind>; coding_agent=<coding_agent>; pane=—) ===
pane not available (pending placement)
```

**JSON mode** — a top-level array, same order, one object per entry mirroring
`member capture`'s keys plus `name` / `kind` / `coding_agent` / `error`, in
this pinned key order: `member_id`, `name`, `kind`, `coding_agent`,
`pane_id`, `lines`, `content`, `captured_at`, `content_sha256`, `error`. On a
successful entry `error` is `null`. On an annotated entry `content`,
`captured_at`, and `content_sha256` are `null`; `error` carries the exact
annotation string from text mode; `pane_id` is `null` for a pending placement
and the real pane id for a failed capture. `lines` always echoes the
requested depth.

Exit codes: `0` completed scan (annotated entries included), `1` unknown or
soft-deleted fleet or multiplexer unreachable, `2` usage errors.

#### `server`

Options: `--host` (string, default `settings.broker_host` = `127.0.0.1`, shown
in help), `--port` (integer, default `settings.broker_port` = `8000`, shown in
help). Serves the WebUI app on host/port. The schema-version guard
(§ *Schema-version guard*) runs before the server starts (wired into
`server::run`); port-in-use and all other server
errors propagate unwrapped.

#### `setup`

`setup` is a plain **command** (no subcommands) — the single onboarding
and schema-management entry point, and the migrations-apply path
(idempotent; safe to re-run). Command help: `Migrate the database schema
and install the coding-agent assets (skills and presets).` It takes no
positional arguments — a bare `cafleet setup <word>` fails with clap's native
unexpected-argument error, while a word following the flag (`cafleet setup
--coding-agent claude <word>`) is greedily consumed as another flag value and
fails with clap's native invalid-value error unless it names an agent (both
exit 2).

| Flag | Required | Notes |
|---|---|---|
| `--coding-agent AGENT...` | no | Multi-value (space-delimited) and repeatable — `--coding-agent claude codex` and `--coding-agent claude --coding-agent codex` are valid and equivalent. A choice over `claude` / `codex` / `opencode`; an unknown value fails with clap's native invalid-value error (exit 2). Duplicates are deduplicated. Help: `Install the named agent's assets (space-delimited, repeatable; default: all agents).` |

Reads the CLI's own version and runs two independent halves, **in order** (db
first, then assets):

- **DB half** — initialize or migrate the registry via the db-migration driver
  (§8): force a sync SQLite URL, create the DB file's parent directory, and
  apply the bundled migrations up to the head revision (idempotent). On an
  application error, print `db half failed: <message>` and record the
  failure. The assets half still runs after a DB-half failure. Its pre-flight
  fails if `asset_installs` is missing, but an old database with that table can
  accept assets updates even when duplicate monitors prevent migration.
  Report only the halves that actually failed; the DB failure still makes
  the overall command exit 1.
- **Assets half** — installs, from the data embedded in the binary at build
  time (§7.6) with **no network access**, each selected agent's skills plus
  its bundled preset (where one exists) at the directories resolved per
  § *Config-dir resolution* (each target's agent directories are created as
  needed), upserting one `asset_installs` row keyed
  `(coding_agent, identity path)` per installed agent after that agent's
  install succeeds. Selection:

  | Invocation | Assets-half behavior |
  |---|---|
  | `--coding-agent` given (one or more values) | Install exactly the named agents, in the fixed order `claude`, `codex`, `opencode`, each at its resolved paths; upsert the `(agent, resolved identity path)` row after that agent's skills and preset (where one exists) install successfully. |
  | No flag | Install all three agents, in the fixed order — identical to `--coding-agent claude codex opencode`. |

  On an application error, print `assets half failed: <message>` and record
  the failure. A config-path validation failure (§ *Config-dir resolution*)
  fails the assets half with the pinned error as `<message>` wherever the
  half resolves an agent's identity path — a targeted agent in the selector
  form, and every agent in the no-flag form, which resolves all three
  identity paths.
- If anything that ran failed → application error `<failed halves joined by '
  and '> half failed` (exit 1; db listed first, matching run order — e.g. `db
  and assets half failed`).

##### Config-dir resolution

Backend config directories resolve through the backends' native
config-location environment variables, falling back to the defaults when
unset:

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
variable. `OPENCODE_CONFIG` (a config **file** variable) is ignored.

**Validation.** A set variable must hold an absolute path. Any other value —
the empty string, a relative path, a literal unexpanded `~/…` — fails at
resolution time with the application error (exit 1):

```
<VAR> must be an absolute path (got '<value>')
```

Validation is lazy: a variable is read and validated only when a site
actually resolves that backend's directory. `cafleet setup --coding-agent
claude` with an invalid `CODEX_HOME` succeeds because the selector resolves
only the targeted agent's directory; plain `cafleet setup` resolves all
three identity paths, so an invalid variable fails its assets half. The
spawn preconditions themselves
read none of the three variables for claude and codex (PATH-check-only;
opencode's resolves the preset base) — but the stale-assets guard fronting
every fleet-scoped command, `member create` included, resolves all three
identity paths (§ *Stale-assets guard*). One exception to strict lazy
failure: `doctor` catches per-agent resolution errors and renders them as
issues instead of aborting (§ `doctor`).

Resolution also reports the winning origin — the supplying variable name or
the default — which feeds `doctor`'s `source` column and JSON. The three
variables are **not** configuration fields (§7.1): they are backend-native
variables read at point of use through an injected env lookup, so tests run
against fakes.

**Recorded-path identity.** Every surface that keys on "the agent's resolved
path" uses one canonical path per agent — the resolved **base** directory —
stored absolute, exactly as resolved (no canonicalization beyond the
absolute-path validation):

| Agent | Recorded / status-keyed path |
|---|---|
| claude | The resolved claude config dir (`$CLAUDE_CONFIG_DIR` or `~/.claude`) |
| codex | The resolved codex home (`$CODEX_HOME` or `~/.codex`) |
| opencode | The resolved preset base (`$OPENCODE_CONFIG_DIR` or `~/.opencode`) — the only opencode root that can vary; the skills base is fixed and carries no identity |

##### Shared helpers (the assets half)

**resolve-targets** selects the explicitly named agents, or all three in the
fixed order `claude`, `codex`, `opencode`. Each backend sequentially replaces
the two embedded skills `cafleet`, `cafleet-design-doc`, removes
`<skills_dir>/cafleet-research`, then replaces its preset where present.
Each target is deleted before writing its replacement. Success output follows
each operation; the installed version is recorded after all operations succeed.
Skills resolve to `<claude base>/skills`, `<codex home>/skills`, and the fixed
`~/.config/opencode/skills`. Presets map embedded
`presets/codex/cafleet.rules` to `<codex home>/rules/cafleet.rules`, and
`presets/opencode/cafleet.md` to `<preset base>/agents/cafleet.md`.

#### Shared diagnosis and connection reuse

Schema and asset diagnosis return typed facts; CLI presenters supply messages.
Guards and command work share the invocation connection. Setup attempts both
halves, reuses an open connection, and diagnoses again after migration.
Doctor reports all sections and reads installed versions only at schema head.

#### Schema-version guard

Every non-setup command — the `fleet`, `member`, and `message` groups (at
the top of the group callback, before any subcommand body runs), the
`monitor` command (both forms, before the command body), and `server` —
runs a schema-version prologue before
its command body and before the stale-assets guard. Exempt: `setup` (must
remain runnable to repair) and `doctor` (reports instead of blocking). The
guard connects and classifies the database via the `recorded_version` /
`has_foreign_tables` helpers shared with `setup` and `doctor`; `<M>` is the
recorded version, `<N>` the embedded head:

| Database state | Guard result (application error, exit 1) |
|---|---|
| Recorded version == head | Proceed silently. |
| Recorded version < head | `database schema is outdated (schema <M>, head <N>); run 'cafleet setup'` |
| No ledger, no app tables (missing or empty DB file) | `no cafleet database; run 'cafleet setup'` |
| No ledger, app tables present | `database has tables but no schema history — not a cafleet database?` |
| Recorded version > head | `database schema <M> is newer than this cafleet (head <N>); upgrade cafleet` |

The table strings are the application-error payloads; the CLI renders each
with its uniform `Error: ` prefix. `Connection::open` creates an empty DB
file when missing, so the guard detects "missing" post-hoc as the
no-ledger/no-tables state — matching doctor's `Missing` classification.
Connection-level failures (unreadable file, bad URL scheme) keep their
existing `failed to open database at '<path>': <e>` / scheme errors — those
are environment errors, not schema states. The guard's wording mirrors
doctor's report lines (`schema <M>, head is <N> — run: cafleet setup` stays
doctor's rendering; the guard uses the phrasings above). With this guard in
front, the stale-assets guard runs only against an at-head schema — a
missing or outdated schema never surfaces a raw SQLite error from a guarded
command.

#### Stale-assets guard

Every fleet-scoped surface — the `fleet`, `member`, and `message` groups (at
the top of the group callback, before any subcommand body runs) and the
`monitor` command (both forms, before the command body) — resolves, after
the schema-version guard passes, each
agent's identity path (§6.3 *Config-dir resolution*) and validates the
recorded assets installs at those paths:

1. If a config-path variable fails validation, exit 1 with the pinned
   validation error (§6.3 *Config-dir resolution*).

2. If no agent has a row at its currently-resolved path (zero rows, or — on
   a hand-tampered at-head database — a dropped `asset_installs` table,
   which the kept `asset_installs_table_exists` pre-check classifies as the
   no-rows case), exit 1 with:
   ```
   Error: no assets install is recorded at the resolved paths; run 'cafleet setup' to install
   ```

3. If a row at a resolved path has a `cafleet_version` differing from the
   runtime CLI version (simple string inequality — a downgrade also
   triggers), exit 1 with the stale agents listed in ascending
   `coding_agent` order:
   ```
   Error: stale assets detected (<agent>=<recorded>[, ...]; CLI <runtime>); run 'cafleet setup' to reinstall
   ```

5. Otherwise proceed silently.

Staleness checks use version records at currently-resolved paths; agents without
a matching row contribute nothing to staleness. Plain `cafleet setup` installs
agents at their resolved paths.
Exempt surfaces: `setup` (must remain runnable to
repair), `doctor`
(reports instead of blocking), and `server` (human-facing WebUI, not
fleet-scoped — though the schema-version guard above does cover it).

**Help interaction.** `--help` renders at parse time and exits before any
command body runs, so neither group-level help (`cafleet fleet --help`) nor
subcommand help (`cafleet fleet create --help`) triggers either guard — both
always print help, even under a missing database or a missing or stale
install.

#### Spawn-prompt resolution (used by `member create`)

The spawn prompt is supplied through the shared body input (positional
`PROMPT` or `--file PATH`, §6.3 [text-body input](#text-body-input)): exactly
one is required, `-`
reads stdin, a relative `--file` path resolves against CWD, decoded UTF-8
with no newline translation. There is **no** built-in default template — a
bare `member create` with neither the positional nor `--file` is clap's
native missing-required-argument-group error (exit 2).

**Placeholder substitution.** After the body is resolved, `member create` runs
the Rust spawn-placeholder mini-formatter over it, substituting `{fleet_id}`, `{member_id}` (the spawned
member's own id), `{director_member_id}`, and `{coding_agent}` (the resolved
backend). A custom prompt keeps a literal brace by doubling it (`{{` / `}}`).
Only these four exact names and doubled braces are supported; Python format
specifications, conversions, and attribute/index access are not interpreted.
Error surfaces (both usage errors, exit 2): an unknown placeholder → `Unknown
placeholder '<key>' in custom prompt. Supported placeholders: {fleet_id},
{member_id}, {director_member_id}, {coding_agent}. Double literal braces ({{, }})
to keep them as text.`; a malformed brace expression → `Malformed custom prompt:
<detail>. Double literal braces ({{, }}) to keep them as text.`. Substitution is
applied **only** by `member create`; the two message-body commands
(`message send`, `message broadcast`) call the shared
reader alone and never run `.format`. This substitution is the **sole**
identity-delivery mechanism for a spawned member — no identity environment
variable is injected into the pane (§7.1).

### 6.4 Output & Formatting

**Scope:** every line of human/machine output for members, messages, fleets,
and the monitor. Pure string/structure transformation — no I/O, no DB,
no network; the only external input is `settings.max_text_len` (default `200`).
Two consumers depend on these exact shapes: the CLI (which prints them) and the
WebUI (which reuses the JSON serialization but bypasses truncation). This module
sets no exit codes. (`doctor` output is produced by the CLI, §6.3, not here.)

The text-vs-JSON selection is the CLI's: every JSON-capable subcommand takes
the one shared per-subcommand `--json` flag (§6.3) — the single output
switch. Text output is the human/pane form, always truncated; JSON is the
complete, untruncated machine form. The `message` group branches on the flag
inside the shared handler sequence, while the `member`, `fleet`, and
`doctor`
handlers branch on it per-handler with their own emit sites (§7.3). The single
absent glyph below and the compact-JSON rules apply to every path.

#### Two-layer architecture

- **Render layer** — projects a typed-column message into the slim text-mode
  "wire" shape, truncates oversized text by codepoint count, serializes to
  compact JSON, and strips
  ANSI from captured pane buffers.
- **Formatter layer** — consumes those shapes (or the raw dicts) and produces
  exact multi-line, column-aligned, ANSI-free terminal strings.

The split is load-bearing: formatters call render functions internally (e.g.
`format_message` calls `render_message` for compact mode), but render functions never
call formatters.

**Render functions:** `strip_ansi(text)`; `format_json(data)`;
`truncate_text(value, limit)`; `truncate_message_text(result)`
(in-place); `render_message(message)` → `{id, from, ts, text, kind?, origin?}`
(the text-mode projection consumed by `format_message`). Truncation and the
compact projection run only on the CLI's text
branch (§6.3); the JSON branch emits the broker result verbatim — the
complete typed-column envelopes.

**Formatter functions:** `format_message`; `format_indexed_list`
(joins formatted items with one blank line between, `empty_msg` when empty —
not numbered); `format_member_detail`; `format_fleet_create`; `format_member`;
`format_member_list`. Private contract helpers: an ISO→`HH:MM:SS` extractor;
an idle-seconds humanizer.

#### Truncation rules

- Truncation counts and slices **by Unicode codepoint**, never by byte. A value
  longer than the effective limit returns its first `limit` codepoints plus a
  one-codepoint `…` (U+2026) suffix — so the result is `limit + 1` codepoints.
- `truncate_text` passes the value through unchanged when the
  value is null or its codepoint length is `<= limit`. Null returns null.
- The effective limit is `truncate_text`'s explicit `limit` argument when given,
  else `max_text_len` (default `200`, from config) — the **only** config
  dependency. `truncate_message_text` takes no `limit` and always uses
  `max_text_len`.

#### Compact-JSON rules

`format_json` emits compact JSON: **no whitespace** between tokens (item
separator `,`, key/value separator `:`); **non-ASCII kept raw** as UTF-8 (e.g.
`…` stays 3 bytes), never escaped to `\uXXXX`; **insertion-order keys** (the
render functions build their output maps in a fixed key order, which is part of
the contract). The WebUI bypasses `truncate_*` but its JSON serialization must
still obey these three rules.

#### The `unicast` suppression sentinel

`render_message` adds a `kind` key **only** when the message's `type` is not the
literal `"unicast"`. `unicast` is the default/suppressed type; only non-`unicast`
types (e.g. `broadcast_summary`) surface a `kind`.

#### The single absent glyph

Every formatter uses **one** absent/empty placeholder — the **ASCII
hyphen-minus `-`** (U+002D). It marks an absent or empty cell everywhere: the
ISO→HMS helper (null/unparseable timestamp) and the idle humanizer (null).
There is no EM DASH `—`
absent-glyph: it is portable, has no Unicode dependency, and the golden-output
tests assert this single glyph.

#### `strip_ansi` regex and CR-defrag

The CSI regex is exactly `\x1b\[[0-?]*[ -/]*[@-~]`: ESC, `[`, any run of
parameter bytes (`0x30`–`0x3F`), any run of intermediate bytes (`0x20`–`0x2F`),
then one final byte (`0x40`–`0x7E`). This matches **CSI sequences only** — OSC,
DCS, and single-character escapes are deliberately **not** stripped. After
stripping CSI matches, split on `\n` and, per line, keep only the substring after
the **last** `\r` (CR-redraw defrag: a TUI redraw `prefix\rNEW` keeps only
`NEW`), then re-join with `\n`. An empty/falsy input returns unchanged.

#### Mutation contract

`truncate_message_text` **mutates its input in place** and returns the same
object; `render_message` is **non-mutating** (it builds a new structure). In a
language with no aliasing concern, preserve the *observable* result.

#### Field access / optionality

Every field is read with required access unless marked optional; required access
**fails loud** on a missing key by design. The truthiness guards on `text` /
`origin_message_id` mean empty string and `0` are also suppressed, not just null.

- **Message** (`render_message` / `format_message` / `truncate_message_text`): `message_id`
  (req), `from_member_id` (req), `status_timestamp` (req),
  `text` (req key; guarded by truthiness),
  `type` (req; `"unicast"` suppresses `kind`), `origin_message_id`
  (optional; `origin` key only when **truthy**). Envelope: a message may be wrapped
  `{message: {…}}`; `format_message`
  unwraps when the inner value is a dict.
- **Member detail** (`format_member_detail`): `member_id` (req), `name` (req),
  `status` (req). The detailed view (description, kind, skills, placement) is
  the broker `get_member` dict, emitted by `--json` (§6.3).
- **Fleet-create** (`format_fleet_create`): `fleet_id` (req), `director` (req
  nested) → `member_id` (req), `monitor` (req nested, after `director`, the
  identical member shape) → `member_id` (req).
- **Member-create** (`format_member`): `member_id` (req), `name` (req),
  `placement` (req) → `coding_agent` (req), `mux_pane_id` (req key; `(pending)`
  when falsy).
- **Member-list row**: `member_id`, `name`, `kind`, `placement` (optional,
  null for a placementless row; when present → `{coding_agent, mux_pane_id (→
  "(pending)")}` feed the `backend` / `pane_id` cells, `-` cells when null),
  `last_sent`, `last_recv`, `last_ack` (ISO str | null), `idle` (int seconds |
  null).
- **Roster row (the WebUI `GET /api/members` roster)**: `member_id`, `name`,
  `description`, `status`, `registered_at`, `kind` (the three `get_member`
  values, §5.4), `placement` (null when placementless); serialized directly by the
  WebUI, not by a formatter. The monitor runtime/member payloads (§6.2) are
  likewise serialized directly by the WebUI (§6.8), not by a formatter.

The `(pending)` fallback for `mux_pane_id` appears in the member render
and both list rows.

The `backend` display label (in the
`format_member` render and the roster/list column headers) maps to the
placement's **`coding_agent`** value (`claude`/`codex`/`opencode`) — it names the
coding-agent backend, a distinct axis from the placement's `backend` column
(the multiplexer, `tmux`/`herdr`). The placement projection carries the new
`backend` column, but these formatters do not render it; only `cafleet doctor`
surfaces the resolved multiplexer backend (§6.3).

#### Exact text layouts

`format_message` — line 1 by concatenation: `[<id> | from:<from> |
<ts>]`, with ` | kind:<kind>` inserted before `]` when a `kind` is present and
` | origin:<origin>` inserted (after kind) when an `origin` is present; if the
rendered `text` is truthy a second line holds the body. This compact form is
the **only** text form; the full envelope is `--json` (§6.3).

`format_member_detail` — `<member_id> <name> <status>` (single spaces, no
labels). The detailed view is `--json` (the broker `get_member` dict, §6.3).

`format_fleet_create` — `<fleet_id> director=<director.member_id>
monitor=<monitor.member_id>`.

`format_member` — `<member_id> <name> backend=<coding_agent> pane=<pane>`
(`pane` = `mux_pane_id` or `(pending)`).

`format_member_list` — empty → `0 members.`; else a header `<count> member<s>:`
(trailing `s` only when `count > 1`; `1 member:` exactly), a column header and
separator, then one row per member. Each row begins with a two-space indent and
columns separated by two spaces, left-justified to fixed widths (longer values
are **not** truncated): `member_id` 9, `name` 13, `kind` 8, `backend` (the
placement's `coding_agent`; `-` when placementless) 8, `mux_pane_id`
(→`(pending)` when unset; `-` when placementless) 7, then
the humanized `idle` with no padding (last column). `member_id` is stringified.

#### Private helper semantics

- **ISO→HMS** — returns the `HH:MM:SS` portion: the substring after `T`,
  truncated to its first 8 characters (fractional seconds/offsets dropped).
  Returns ASCII `-` when null, has no `T`, or is not a string. A shorter time
  portion yields a shorter (unpadded) string — slice, do not validate or pad.
- **idle humanizer** — null → `-`; `< 60` → `<n>s`; `< 3600` → `<n // 60>m`;
  else `<n // 3600>h` (integer floor division).

Both absent-cell helpers above use the single ASCII `-` glyph (§6.4 *The
single absent glyph*). The conditional fields `kind`, `origin`, and the body
line are gated on truthiness — omitted, never emitted empty.

### 6.5 Multiplexer (tmux + herdr)

**Scope:** the `Multiplexer` interface, the frozen `MultiplexerContext`, the
optional `AgentStateAware` capability, the
`MultiplexerError` exception taxonomy, the `MULTIPLEXERS` registry with the
`resolve_multiplexer()` resolver, and the two shipped backends `TmuxMultiplexer`
and `HerdrMultiplexer`. Each backend owns all subprocess invocation and
keystroke injection for its multiplexer. The `MULTIPLEXERS` registry maps
`"tmux"` and `"herdr"` each to a single shared stateless backend instance. Every
method invokes its multiplexer binary as an **argv list without a shell** (no
shell interpolation — load-bearing for the literal `send-keys -l` payloads on
tmux). The exact argv each method builds is given verbatim; preserve subcommand,
flags, and ordering.

**Error taxonomy.** A shared base `MultiplexerError(Exception)`;
`TmuxError(MultiplexerError)` and `HerdrError(MultiplexerError)` are the
backend-specific subclasses. Every CLI boundary that converts a backend failure
to an application error catches `MultiplexerError`, so both backends' failures
are handled uniformly while each backend keeps its own message text.

**Backend resolution — `resolve_multiplexer() -> Multiplexer`.** Every call site
resolves its backend through this function rather than a hardcoded
`MULTIPLEXERS["tmux"]`. Precedence:

1. **Explicit override.** `settings.multiplexer` (from `CAFLEET_MULTIPLEXER`)
   non-`None` must be a registry key; otherwise raise `MultiplexerError` with
   `CAFLEET_MULTIPLEXER=<value!r> is not a supported multiplexer (expected one
   of: <sorted keys>)`.
2. **Auto-detect.** `HERDR_ENV` truthy ⇒ herdr present; `TMUX` set ⇒ tmux
   present.
3. **Ambiguity is a hard error.** Both present ⇒ raise `MultiplexerError`
   `ambiguous multiplexer environment: both HERDR_ENV and TMUX are set; set
   CAFLEET_MULTIPLEXER to 'tmux' or 'herdr' to disambiguate`. Neither present ⇒
   raise `MultiplexerError` `no supported multiplexer detected: neither HERDR_ENV
   nor TMUX is set; run cafleet inside a tmux or herdr session, or set
   CAFLEET_MULTIPLEXER`. Exactly one present ⇒ that backend.

An unset `CAFLEET_MULTIPLEXER` (auto-detect) is the default; the override is
the deterministic escape hatch.

#### Interface signature note — `split_window`

`Multiplexer.split_window(*, reference: MultiplexerContext, env, command) -> str`
takes the full reference context rather than a bare window id: tmux splits a
*window* and uses `reference.window_id`; herdr splits a *pane* and uses
`reference.pane_id`. The `member create` and `fleet create` orchestrators
(§6.3) hold the Director's `MultiplexerContext` and pass it directly. The
backend owns a created pane until successful return; the CLI takes ownership
then. Failed creation carries `PaneCleanup::Attempted` or
`PaneCleanup::Unknown` metadata when applicable, per §6.3 *Creation ownership
and compensation*. A missing/invalid id is an unconfirmed cleanup, not a
license to infer another target.

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
- **`send_exit(*, target_pane_id, ignore_missing=False)`** — keystrokes `/exit`
  + Enter via the literal-then-Enter core, **Esc-first=YES**; tolerates a missing
  pane when `ignore_missing`, including when the pane disappears before the
  leading Esc.
- **`send_poll_trigger(*, target_pane_id, member_id) -> bool`** —
  best-effort. tmux missing → `false`; payload `cafleet message poll
  <member_id> — then resume your work if something was
  still running.`; literal-then-Enter, `timeout=5`s,
  **Esc-first=YES**, any error → `false`. Used only by `member ping`.
- **`send_wake_trigger(*, target_pane_id, fleet_id, members, director) ->
  bool`** —
  best-effort; the **sole** keystroke the monitor loop fires, targeted at the
  fleet's **monitor member's own pane** (the loop never keystrokes any other
  pane — not the Director's, not an ordinary member's). `members` is the wake
  roster (§6.2 `list_fleet_wake_targets`, excluding the Director and the
  monitor member); `director` is a single descriptor carrying the Director's
  own `member_id`, `name`, `coding_agent`, and `pending_count`, rendered as
  the wake's trailing `Director:` segment. Each roster entry has `member_id`,
  `name`, validated
  `coding_agent`, and `pending_count`; an entry — roster or Director — whose
  `coding_agent` is not a
  supported backend name raises `member <id> has invalid coding_agent
  '<agent>'` — no keystroke, no cadence commit, and the loop surfaces the
  error and exits (distinct from a `false` return — backend missing or
  keystroke failed — which the loop retries next tick, §6.6). Names and agent
  values pass the single-line sanitizer. Render each roster
  entry as `<member_id> (<name>; coding_agent=<agent>; unacked=<pending_count>)`,
  joined by `, `, ordered by `member_id` ascending; the Director renders in the
  identical field grammar as its own trailing segment.

  The tmux/herdr payload is byte-identical and is a **pure, unconditional
  trigger** — the member roster, the Director segment, and two fixed protocol
  sentences; nothing
  else:

  ```
  [cafleet] tick: fleet <fleet_id> — health-check your <N> members: <entries>. Director: <director_id> (<director_name>; coding_agent=<director_agent>; unacked=<director_pending_count>). Follow your monitor role protocol. Resume your work if something was still running.
  ```

  `N == 1` uses the singular noun (`health-check your 1 member: …`); `N == 0`
  drops the `<entries>` segment and the clause reads `no members to
  health-check.`, with the `Director:` segment still present in both forms.
  The wake
  fires whenever the interval has elapsed and the
  monitor member's own pane is alive — **including when the fleet has no
  ordinary members**. The wake keystroke is literal-then-Enter with `timeout=5`s and
  **Esc-first=YES** (a wake landing on a pending permission prompt clears it
  instead of answering it); any error returns false. It contains no backtick,
  command-substitution sequence, or pipe.
- **`send_inline_preview(*, target_pane_id, message_id, sender_id, ts, text)`**
  — result-returning; the broker's inline-preview path (the broker truncates
  `text` first). A missing tmux binary (the existing `binary_exists` precheck —
  not `ensure_available`, whose extra environment/session validation is not
  part of inline delivery) fails with exactly `tmux binary not found on PATH`;
  cosmetic CR/LF strip on `text`
  (`\r\n`/`\n`/`\r` each → `⏎` U+23CE, **no** tab/backtick/command-substitution
  sanitization here); two-line payload (single `\n` separator intentionally
  kept):
  ```
  [cafleet msg <message_id> from <sender_id> <ts>]
  <sanitized_text>
  ```
  literal-then-Enter, `timeout=5`s, **Esc-first=YES**. A subprocess failure
  from whichever Escape, payload, or Enter operation failed propagates as the
  raw `MultiplexerError` with the existing subprocess-runner formatting (the
  failed argv and trimmed stderr) — no boolean wrapper. Under
  `send-keys -l` the `\n` is a soft line break inside one keystroke; the single
  trailing Enter submits the whole 2-line payload as one recipient turn.
- **`send_prompt(*, target_pane_id, text, shell=False)`** — fail-fast. Strip
  surrounding whitespace; empty after strip → `send_prompt: text may not
  be empty`; the **original** text with a newline or CR → `send_prompt:
  text may not contain newlines`. literal-then-Enter with `payload = "! " +
  stripped_text` when `shell` else `stripped_text`, and `esc_first=true` — both
  forms share the same Esc safeguard and failure semantics; `shell` changes only
  the `! ` payload prefix.
- **`capture_pane(*, target_pane_id, lines=20) -> str`** — fail-fast. `lines <=
  0` → `capture_pane: lines must be positive, got <lines>`. Run `tmux
  capture-pane -p -t <target_pane_id> -S -<lines + 1000>` (the fixed
  1000-line over-fetch margin, so a blank tail deeper than the requested window
  still leaves drawn lines to keep), split the raw output on
  `"\n"` **only** (not a general line-splitter — must not also split on `\r`, to
  preserve the CLI's CR-defrag), drop the trailing run of visually-blank lines —
  a line is blank when it is whitespace-only after per-line CSI stripping (the
  emptiness check only; kept lines keep their original bytes, TUI-painted empty
  rows carry ANSI sequences) — then return the last `lines` remaining lines
  joined with `"\n"` (no trailing newline; an all-blank buffer captures as the
  empty string). Interior blank lines are preserved — only the trailing blank
  run is dropped, so a small `lines` window shows the pane's drawn bottom rather
  than the blank area under the cursor.
- **`list_pane_ids() -> set`** — fail-fast. `tmux list-panes -a -F "#{pane_id}"`
  with `timeout=5`s; split on whitespace; return the pane-id set. One call
  resolves liveness for every member in a monitor tick.
- **`kill_pane(*, target_pane_id, ignore_missing=False)`** — fail-fast. `tmux
  kill-pane -t <target_pane_id>` through the pane-gone-tolerant runner.

#### Fail-fast vs. best-effort split

- **Fail-fast** (surface failures): `ensure_available`, `context_discovery`,
  `split_window`, `select_layout`, `send_exit`, `send_prompt`,
  `capture_pane`, `list_pane_ids`, `kill_pane` (modulo `ignore_missing`
  pane-gone tolerance on `kill_pane` / `send_exit`).
- **Best-effort boolean** (NEVER raise; `false` on any failure):
  `send_poll_trigger`, `send_wake_trigger`. Each guards
  "tmux missing → `false`" then wraps the keystroke so any error → `false`. The
  boolean is consumed as the monitor's `woke` and the ping outcome.
- **Result-returning** (the raw error is the contract): `send_inline_preview`.
  It applies the exact missing-binary precheck string, then propagates any
  Escape/payload/Enter failure as the raw `MultiplexerError` — no boolean
  wrapper. The broker consumes the result as the unicast
  `notification_sent` + `notification_error` pair and the broadcast
  `delivered` count (§6.2).

#### `MultiplexerContext` (frozen value type)

Immutable, three non-nullable string fields, no defaults, constructed only by
`context_discovery`: `session` (tmux session name), `window_id` (e.g. `@N`),
`pane_id` (e.g. `%N`).

#### Keystroke core, delays, and the Esc-first matrix

The shared literal-then-Enter primitive (used by `send_exit`,
`send_poll_trigger`, `send_wake_trigger`, `send_inline_preview`, and
`send_prompt`) takes `target_pane_id`, `payload`, optional
`timeout`, `ignore_missing` (default false), `esc_first` (default false):

1. **If `esc_first`:** run `tmux send-keys -t <target_pane_id> Escape`, then
   sleep `_ESC_SETTLE_DELAY` (`0.1`s). The leading `Escape` dismisses a pending
   permission prompt so the trailing `Enter` cannot blind-confirm it.
2. Run `tmux send-keys -t <target_pane_id> -l <payload>` — `-l` types the literal
   payload (single argv element, never shell-interpolated).
3. Sleep `_SUBMIT_DELAY` (`1.0`s) — **unconditionally**, so the Enter clears
   codex's post-paste Enter-suppression window and opencode slash-autocomplete
   settles.
4. Run `tmux send-keys -t <target_pane_id> Enter` — submits.

An embedded `\n` in `payload` is a **soft** newline within the single keystroke
sequence — it does NOT fragment into a second submit. Esc-first matrix:
`send_poll_trigger` **YES**, `send_inline_preview` **YES**, `send_wake_trigger`
**YES**, `send_exit` **YES**, and both `send_prompt` forms **YES**.

#### Subprocess core, timeout, and pane-gone tolerance

The subprocess runner invokes tmux as an argv list (no shell), treats a non-zero
exit as a failure, and returns stdout on success. Failure-message intents:
binary-not-found → `tmux binary not found: <detail>`; timeout → `tmux command
timed out after <timeout>s: <space-joined argv>`; non-zero exit → `tmux command
failed: <space-joined argv>\nstderr: <trimmed stderr>`. A **per-call timeout** of
`5`s is passed by `list_pane_ids` and the three keystroke helpers. Other calls remain unbounded. **Pane-gone tolerance:** the tolerant runner swallows a
tmux error only when **both** `ignore_missing` is true **and** the message text
(case-insensitive) contains `"can't find pane"` or `"no such pane"`; any other
failure re-raises even under `ignore_missing`. Whatever error shape a port uses
MUST keep the message/stderr text inspectable for this substring match.

The shared runner's timed path drains stdout and stderr concurrently from spawn
with nonblocking descriptors. Each iteration checks the monotonic deadline and
the direct child's status, reads at most 64 KiB from each stream, and polls for
at most the lesser of 20 ms and the remaining deadline. Interrupted operations
retry with the same deadline. Completion requires child exit and EOF on both
streams. Output is not truncated: success returns lossy UTF-8 stdout and nonzero
exit returns lossy UTF-8 stderr. The untimed path collects output without a
deadline.

Deadline expiry kills and reaps the direct child, closes both read descriptors,
and returns the timeout category. A descendant holding a pipe open after the
direct child exits remains subject to that deadline; descendant termination is
not guaranteed. FD configuration, read, poll, or child-status errors also trigger
direct-child kill/reap and descriptor release. Cleanup failures accompany the
primary cause and do not replace it. The deadline bounds observation and cleanup
initiation, not wall-clock return when the operating system does not respond.

#### Wake-field sanitizer — payload contract

Applied to each member name and `coding_agent` value before interpolation into
the `send_wake_trigger` payload. An absent/unregistered coding agent aborts the
wake without cadence commit. Replacement chain, **order matters**: `\r\n` → `⏎`
(U+23CE); `\n` → `⏎`; `\r` → `⏎`; `\t` → `⏎`; `` ` `` → `ˋ` (U+02CB); `$(` →
`$﹙` (`$` followed by U+FE59). CR/LF/tab → U+23CE preserves the single-line
guarantee; backtick → U+02CB and `$(` → `$`+U+FE59 preserve the no-backtick /
no-command-substitution guarantee. These are exact Unicode scalar values and are
part of the keystroked payload contract, not cosmetic — distinct from the
CR/LF-only cosmetic strip in `send_inline_preview`.

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
- **`send_exit(*, target_pane_id, ignore_missing=False)`** — `herdr pane
  send-keys <id> esc`, then `herdr pane run <id> "/exit"`; pane-not-found on
  either keystroke is tolerated exactly when `ignore_missing` is true, while
  other errors propagate.
- **`send_poll_trigger(...) -> bool`** — best-effort. `herdr pane send-keys <id>
  esc` (the Esc safeguard, with the same short settle delay), then `herdr pane
  run <id> "cafleet message poll <member_id>
  — then resume your work if something was still running."`.
- **`send_wake_trigger(...) -> bool`** — best-effort. `herdr pane send-keys <id>
  esc` (the Esc safeguard — the target is the monitor member's own pane, which
  can be
  parked on a permission prompt), then `herdr pane run <id> "<payload>"`. The
  `<payload>` — its single-line `[cafleet] tick:` text, its per-member
  `<member_id> (<name>; coding_agent=<agent>; unacked=<pending_count>)`
  entry list, and its trailing `Director:` segment — is **byte-identical** to the tmux `send_wake_trigger` payload
  above, carrying no backtick, no command-substitution sequence, and no pipe.
- **`send_inline_preview(...)`** — result-returning. A missing herdr binary
  (the existing `binary_exists` precheck — not `ensure_available`) fails with
  exactly `herdr binary not found on PATH`. Then `herdr pane send-keys
  <id> esc`, then `herdr pane send-text <id> "<2-line payload>"` (raw, no Enter —
  the embedded newline is literal), then a sleep of `_SUBMIT_DELAY` (`1.0`s),
  then a single `herdr pane send-keys <id> enter`, keeping the tmux contract of
  "one submit for the whole 2-line payload". A failure from whichever of those
  operations failed propagates as the raw `HerdrError` with the existing
  `_run()` formatting — no boolean wrapper.
- **`send_prompt(*, target_pane_id, text, shell=False)`** — `herdr pane
  send-keys <id> esc`, then `herdr pane run <id> "<payload>"`, where `<payload>`
  is `! <text>` for the shell form and `<text>` for the plain form. Both forms
  mirror `send_poll_trigger`'s esc-then-run shape and differ only in the prefix.
- **`capture_pane(*, target_pane_id, lines=20) -> str`** — fail-fast. `lines <=
  0` → `capture_pane: lines must be positive, got <lines>`. Run `herdr pane read
  <id> --source recent-unwrapped --lines <lines + 1000>` (the same fixed
  1000-line over-fetch margin as the tmux backend), then apply the same
  windowing: split on `"\n"` only, drop the trailing run of
  visually-blank lines (whitespace-only after per-line CSI stripping; kept
  lines keep their original bytes), and return the last `lines` remaining lines
  joined with `"\n"` (no trailing newline; the last-N window is enforced
  client-side because the daemon may return more rows than requested).

**`_SUBMIT_DELAY` (`1.0`s).** herdr `pane run` submits text **and** Enter
atomically, so the run-based paths (`send_poll_trigger`, `send_wake_trigger`,
`send_prompt`, `send_exit`) carry no submit delay. `send_inline_preview`
is the one herdr path built from a separate `pane send-text` + `pane send-keys
enter` pair, and it sleeps `_SUBMIT_DELAY` between them: codex classifies the
fast-injected payload as a paste and absorbs an Enter arriving within its
post-paste suppression window, which would otherwise leave the preview stuck in
the recipient's composer. The Esc safeguard maps to a discrete `herdr pane
send-keys <id> esc` before the payload on every keystroke path:
`send_poll_trigger`, `send_wake_trigger`, `send_inline_preview`, `send_exit`,
and both `send_prompt` forms.

#### `AgentStateAware` capability (herdr only)

A **separate optional** `@runtime_checkable` Protocol, kept off the base
`Multiplexer` interface so tmux need not implement anything new:

- **`agent_status(*, target_pane_id) -> str | None`** — the pane's current native
  agent state (`working`/`blocked`/`done`/`idle`/`unknown`), or `None` when no
  agent is detected. herdr realization: `herdr pane get` / `pane read --source
  detection`.

`HerdrMultiplexer` implements it; `TmuxMultiplexer` does **not** implement
`AgentStateAware` (an `isinstance(mux, AgentStateAware)` guard is therefore
false on the tmux backend). No DB column backs the native status, and the
monitor loop (§6.6) does not consume this capability — the loop's wake is
unconditional and interval-driven, on both backends.

### 6.6 Monitor heartbeat loop

**Scope:** the in-process supervision scheduler. The fleet's **monitor
member** — a dedicated watcher spawned by the `cafleet fleet create`
bootstrap, before any ordinary `cafleet member create` (re-spawned mid-run
via `cafleet member create --role monitor` after a monitor death) — hosts the
blocking `run_monitor_loop` command as a backend-resolved long-lived execution
in its own pane. It fires one unconditional, fleet-level wake into the
**monitor member's own pane** once per
wake interval, naming every ordinary member and the Director with their
pending-delivery counts, pointing at the monitor role protocol, and resuming
the monitor member's own work
if something was still running. The module owns the OS-facing half — the pure
due-check, one scan pass, the foreground driver with signal handling and
runtime-row cleanup, and the scan-cadence and default-wake-interval constants.
It performs no DB internals (the broker's) and no multiplexer internals; it
orchestrates calls into both. The wake is unconditional and interval-driven
on every backend: there is no per-member due computation and no consumption
of `AgentStateAware` native status.

#### Public surface

- **`wake_due(last_wake_at, started_at, wake_interval_seconds, now) -> bool`**
  — pure due-check for the fleet-level wake; no DB/multiplexer access. The
  interval parameter is a non-negative 64-bit integer. A
  present `last_wake_at` always wins as the baseline: parsable → due iff
  `now − last_wake_at`, in whole seconds, is `>= wake_interval_seconds`;
  unparsable → immediately due. A `NULL` `last_wake_at` falls back to
  `started_at` as the baseline: parsable → due iff `now − started_at`, in
  whole seconds, is `>= wake_interval_seconds`; `NULL` or unparsable →
  immediately due.
- **`monitor_tick(fleet_id, now) -> CONTINUE | STOP`** —
  one scan pass. Takes no interval parameter — each pass re-reads
  `wake_interval_seconds` from the fleet's runtime row.
- **`run_monitor_loop(fleet_id, tick_seconds, wake_interval_seconds)`** —
  foreground driver: claim slot → install signal handlers → `tick → sleep`
  until signalled → clear slot on exit. `wake_interval_seconds` is used only
  to stamp the claim; the ticks read the stored value.
- **`CONTINUE` / `STOP`** — tick-result markers distinguishing "keep looping"
  from "self-terminate".
- **`DEFAULT_TICK_SECONDS = 5`** — default scan cadence (seconds).
- **`DEFAULT_WAKE_INTERVAL_SECONDS = 600`** — default wake interval
  (seconds), re-exported from `settings.monitor_wake_interval` (§7.1) so the
  loop imports policy from one place.
- Re-exports `MONITOR_STALE_FACTOR` (3), `MONITOR_STALE_FLOOR_SECONDS` (15) —
  the runtime-liveness policy tunables, whose single home is the broker.

The stop flag, the sleep helper, the signal handler, and the marker type are
implementation-private; only the functions, the markers, and the constants
above are public.

#### `monitor_tick(fleet_id, now)`

One scan pass, steps in order:

1. **Ownership-checked heartbeat.** Call the broker's heartbeat with `(fleet_id,
   this-pid, now-as-ISO)`. Returns false (zero-row update — this process was
   displaced and another reclaimed the slot) → return `STOP`. This is the
   split-brain loser's exit.
2. **Fleet liveness.** Fetch the fleet; absent **or** `deleted_at` set → return
   `STOP`.
3. **Read the runtime row.** The heartbeat just matched, so the row exists;
   take its `wake_interval_seconds` — the owning loop stamped the value at
   claim, so `NULL` here is corrupt state and fails loudly — and its
   `wake_requested_at`: `forced = wake_requested_at is non-null` (an
   operator requested an immediate wake via `POST /api/monitor/wake`, §6.8).
4. **Wake-interval gate.** `wake_interval_seconds == 0` → return `CONTINUE`
   (a heartbeat-only tick: no due-check, no monitor-member resolution, no
   multiplexer call). Evaluated per tick against the value just read.
   **Skipped when `forced`** — an explicit operator action bypasses a
   disabled schedule.
5. **Compute due-ness.** Call
   `wake_due(last_wake_at, started_at, wake_interval_seconds, now)` on the
   row's values. Not due →
   return `CONTINUE` with no multiplexer call. **Skipped when `forced`** —
   an explicit operator action bypasses a not-yet-due schedule.
6. **Resolve the monitor member's pane.** Call the broker's
   `active_monitor_member_id(fleet_id)` (§6.2) and, when it resolves, read
   that member's placement `mux_pane_id`. No active monitor member, or one
   with no pane →
   return `CONTINUE`
   (nothing recorded — the fleet stays due, and a pending request stays
   pending, retrying next tick).
7. **Fetch pane liveness once.** A single `list_pane_ids` call against the
   resolved backend (§6.5). The monitor member's pane absent from the live set →
   return `CONTINUE` (nothing recorded — the fleet stays due, and a pending
   request stays pending, retrying next tick).
8. **Wake the monitor member.** Fetch the wake roster via the broker's
   `list_fleet_wake_targets(fleet_id)` (§6.2 — every active, non-Director,
   non-monitor
   member with its `coding_agent` and `pending_count`) and the Director's own
   descriptor via the broker's `fleet_wake_director(fleet_id)` (§6.2), then call the
   multiplexer's wake trigger against the monitor member's own pane (the loop's
   **only** keystroke), passing `fleet_id`, the roster, and the Director
   descriptor; it returns a
   boolean `woke`. An entry — roster or Director — with an invalid
   `coding_agent` aborts the wake
   without a cadence commit (§6.5).
   - If `woke` is true: call the broker's `record_monitor_wake` with
     `now-as-ISO` — one write that stamps `last_wake_at` and clears
     `wake_requested_at`, so a forced wake resets the schedule baseline and
     a scheduled wake consumes any pending request — then emit one stdout
     heartbeat line. When the wake fired with a pending request (`forced`):
     ```
     {now as canonical ISO-8601, §5.1} tick -> forced wake monitor {monitor_member_id} ({N} members)
     ```
     otherwise the scheduled form:
     ```
     {now as canonical ISO-8601, §5.1} tick -> wake monitor {monitor_member_id} ({N} members)
     ```
     `N` is the roster size (may be `0`).
   - If `woke` is false: do not record the wake and do not echo — the wake
     stays due and any pending request stays pending, so the next tick
     retries (no wake-storm, no silent skip).
9. Return `CONTINUE`.

**Critical ordering invariant:** `record_monitor_wake` and the heartbeat echo
are both gated behind `woke == true`. Preserve this gating exactly. The wake
fires **even when the fleet has no ordinary members** (`N == 0`) — the
Director is itself a supervision target, carried in the wake's trailing
`Director:` segment even on an empty roster, and a fleet with no ordinary
members yet is a transient bootstrap state, not a steady state.

The per-tick re-read is what makes the interval externally editable: an
`UPDATE` to `wake_interval_seconds` (the WebUI `PATCH /api/monitor`, §6.8)
changes the running loop's cadence on its next tick — within `tick_seconds` —
with no restart, gated against the existing `last_wake_at` / `started_at`
baseline. The same re-read serves the forced wake: a `wake_requested_at`
write (the WebUI `POST /api/monitor/wake`, §6.8) is honored on the next
tick, so a forced wake lands within `tick_seconds` of the request.

#### `run_monitor_loop(fleet_id, tick_seconds, wake_interval_seconds)`

Foreground driver. The fleet's monitor-runtime row is the **only** coordination
artifact (no PID file); identity throughout is the OS process id.

1. Reset the shared stop flag to false. Capture `pid = this-pid`.
2. **Claim the slot** via the broker's atomic claim `(fleet_id, pid,
   tick_seconds, wake_interval_seconds, now-as-ISO)` — the driver's only use
   of its `wake_interval_seconds` parameter. On refusal (returns false) →
   application error
   (exit 1) `monitor already running for fleet {fleet_id}`. There is no silent
   fallback. A reclaim leaves `last_wake_at` untouched (§6.2), so the wake
   cadence survives a crash/restart cycle; it re-stamps `started_at`, so a
   fleet that never received its first wake waits a fresh full
   `wake_interval_seconds` from the restart. On success, print the startup line
   `monitor loop started (fleet <fleet_id>, tick <tick>s, pid <pid>)` to
   stdout before the first tick — the line the monitor member confirms
   before sending the `monitor live` gate signal that unblocks the Director's
   first ordinary `cafleet member create`.
3. **Install signal handlers** for SIGTERM and SIGINT; each flips the shared stop
   flag to true (the handler is minimal — just a flag flip).
4. **Loop** while the stop flag is false: if `monitor_tick(fleet_id, now)`
   (each pass stamps `now` fresh as tz-aware UTC)
   returns `STOP` → break; else call `interruptible_sleep(tick_seconds)`.
5. **Cleanup (always, in a finally block):** the broker's ownership-checked clear
   `(fleet_id, pid)` — nulls the slot's `pid` / `started_at` / `last_tick_at`
   only if this pid still owns the slot (`last_wake_at` is preserved, §6.2), so
   a displaced loser's clear is a no-op.

**Stop paths:** (a) a signal sets the stop flag → loop exits → finally clears;
(b) `monitor_tick` returns `STOP` → break → finally clears; (c) a hard kill runs
no cleanup — the row's heartbeat goes stale and the broker's later liveness check
reports it dead. `cafleet fleet delete` also ends a still-running loop — its
next tick sees the soft-deleted fleet and self-terminates via step 2 of
`monitor_tick`.

#### Monitor resource ownership

After claiming a runtime row, retain each signal registration and clean up on
normal stop or failure: unregister handlers and clear only the owned
`(fleet_id, pid)` row. Preserve wake settings and ledger fields. Append cleanup
failure to a primary error; report cleanup failure when work succeeded.

#### Interruptible sleep & signals

`interruptible_sleep(seconds)` sleeps up to `seconds`, waking early once a stop
signal arrives: compute a **monotonic** deadline, then loop — while the stop flag
is false, `remaining = deadline − monotonic-now`; `remaining <= 0` → return; else
sleep `min(0.2, remaining)`. The 0.2-second poll cap bounds signal-response
latency to ≤200 ms regardless of `tick_seconds`. The SIGTERM/SIGINT handler sets
the shared stop flag and does nothing else; the flag is shared between the
handler (writer) and the loop condition + `interruptible_sleep` (readers). Every
`now` is tz-aware UTC, serialized to the canonical ISO-8601 string at the DB
boundary.

### 6.7 Coding-agent backends

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
- **`build_spawn_argv(prompt, display_name, model, effort)`** — returns the
  full argv vector (binary + flags + prompt) for the multiplexer's
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
constructed, with exactly three entries: `"claude"`, `"codex"`, `"opencode"`.
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
["claude", "--permission-mode", "dontAsk", "--name", <display_name>]
  (+ ["--model", <model>]  if model is not None)
  (+ ["--effort", <effort>]  if effort is not None)
  (+ <prompt>)                                       # bare trailing positional
```

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
preset). `display_name` is silently ignored; the prompt is passed
as a `--prompt <prompt>` flag pair (two tokens), unlike claude/codex's bare
positional.

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
    justification = "cafleet member prompt keystrokes arbitrary text or shell commands into a member pane",
)
```

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

You are a CAFleet member spawned by the Director. The bash ruleset in your frontmatter is deny-by-default: only the explicitly allowlisted commands — `cafleet` (except `cafleet member prompt`), read-only `gh` queries plus the PR comment/review endpoints, non-destructive `git` subcommands, file-inspection utilities, and the project's cargo-backed mise tasks — run; every other command is denied with no prompt (every check resolves to allow or deny). When a denied command is genuinely needed, route it to the Director per the prompt-routing protocol. Read and edit are workspace-scoped with `.env` files denied. Refer to your Director's spawn-prompt instructions for the task.
````

The body is a single physical paragraph (no internal hard line breaks after the
heading and its blank line); the file ends with exactly one trailing newline.

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

### 6.8 WebUI + Config

**Scope (two concerns):** (a) the HTTP app factory `create_app`, the `/api/*`
router, the `X-Fleet-Id` header dependency, the SPA-fallback static server, and
the `cafleet server` launcher; (b) the global `Settings` singleton from the
`CAFLEET_*` env block — consumed CLI-wide, not webui-local. The contract below
is stack-neutral. The config
env-var table is §7.1.

#### App factory (`create_app`)

Returns the configured HTTP application:

1. Constructs the HTTP application. The app exposes **no framework metadata
   surface** — no OpenAPI/schema or interactive-docs endpoints and no
   app-version string. The one canonical version is the binary's compile-time
   version (§7.6), read by `--version` and the stale-assets guard.
2. **Registers the `/api/*` router before the static file server.**
   This ordering is load-bearing: unmatched `/api/*` paths must produce a JSON
   404 from the router, never be swallowed by the SPA fallback.
3. Serves the SPA from the **admin WebUI dist embedded in the binary at build
   time** (§7.6). A missing dist cannot occur at runtime — the build fails
   without the dist — so there is no unmounted state.

#### SPA static file server

Wraps the embedded dist and a reserved-prefix set `("ui", "api")`. Delegates to the
static handler; returns any non-404 result unchanged. On a 404: if the **first
path segment** (the path with the leading `/` stripped — split on the first `/`,
take segment 0) is in the reserved set, re-raise the genuine 404; otherwise serve
`index.html` (the SPA entry). So `GET /anything/else` with no asset returns
`index.html` (200); `GET /ui/...` or `/api/...` with no asset returns a genuine
non-HTML 404.

#### Frontend route and resource ownership

Hash routes select the fleet and member. Each fleet client captures its fleet
id; each resource aborts and invalidates requests when its identity changes.
Refreshes coalesce while a request runs. Keep existing data during refresh,
show errors with Retry, and publish only current-resource results. Missing
fleets return to the picker; transport failures retain the route.

#### `X-Fleet-Id` header dependency

Every data endpoint (everything except `GET /api/fleets`) resolves the fleet via
a header dependency reading `X-Fleet-Id` (case-insensitive). Resolution order and
exact error details (each serialized as `{"detail": <string>}`):

1. Missing or **empty** → `400`, detail `X-Fleet-Id header required`. (An empty
   string counts as missing; a whitespace-only value passes this check and fails
   the next.)
2. Not parseable to an integer (including whitespace-only) → `400`, detail
   `X-Fleet-Id must be an integer`.
3. Fleet does not exist → `404`, detail `Fleet not found`.
4. Otherwise return the integer fleet id.

#### Wire renames & response wrapping

When projecting broker message rows to the wire (inbox / sent / timeline):
`status_state` → `status`, `text` → `body`. `GET /api/fleets` returns a **bare JSON
array**; every other list endpoint wraps in an object (member rows under
`members`, message rows under `messages`). All HTTP errors serialize as
`{"detail": <string>}`; a request-validation failure returns `422` with the
same `{"detail": <string>}` shape (a single human-readable string).

#### The 9 routes

- **`GET /api/fleets`** — unscoped (no `X-Fleet-Id`). Returns the broker fleet
  list **directly as a bare array**.
- **`GET /api/members`** — fleet-scoped. Returns the roster via
  `list_roster(include_message_holders=True)` (§6.2) — every active registry row
  plus deregistered members still owning messages — each row carrying the
  three-value `kind` (§5.4): `"director"` for the fleet's root Director
  (exactly one per fleet), `"monitor"` for the fleet's monitor member (at most
  one active per fleet, derived from the member card's `$.cafleet.kind`
  marker), `"member"` for every other row. Response
  `{"members": [ <member dict>, … ]}`.
- **`GET /api/monitor`** — fleet-scoped. Returns the flat runtime keys
  `{running, pid, tick_seconds, wake_interval_seconds, last_tick_at,
  last_tick_age_seconds,
  started_at, last_wake_at, last_wake_age_seconds}` plus a top-level `members`
  key. Read the runtime row and
  the live-check (current UTC). If absent **or** not live: `running=false`,
  `pid=null`, `tick_seconds` and `wake_interval_seconds` = the row's values
  when a row exists else `null` (`wake_interval_seconds` is also `null` when
  the row predates the column and was never re-stamped),
  `last_tick_at`/`last_tick_age_seconds`/`started_at`/`last_wake_at`/
  `last_wake_age_seconds` all `null` — **a stale row
  never leaks a lingering pid or start time**. When live: `running=true` with the
  live `pid`, `tick_seconds`, `wake_interval_seconds` (a stamped row is
  non-null), `last_tick_at`, `started_at`, `last_wake_at`, and
  computed `last_tick_age_seconds` / `last_wake_age_seconds` (each null when
  its source timestamp is null; else whole-seconds
  now − parsed timestamp, **integer-truncated**). `members` carries the
  shared `monitor_members_payload` rows (§6.2) — one element per active,
  non-Director, non-monitor member, `{member_id, name, pending_count, oldest_pending_ts,
  oldest_pending_age_seconds}` — computed with the same
  `now` as the runtime fields; the flat runtime keys are unchanged by this
  addition. There is no `GET`/`PATCH` per-member monitor endpoint — supervision
  cadence has no per-member configuration surface.
- **`PATCH /api/monitor`** — fleet-scoped. Body `{wake_interval_seconds: int}`:
  a JSON integer in `0..=i64::MAX`, validated as an exact 64-bit integer plus
  `>= 0` — floats, stringified integers, negatives, and numbers above
  `i64::MAX` are rejected, not coerced. Resolution order: header errors, then
  body validation — an unparsable body → `422`, detail
  `invalid JSON body: <parse error>`; a missing or invalid field → `422`,
  detail `wake_interval_seconds must be a non-negative integer` — then the
  fleet check (`404`, `Fleet not found`), then the row update via
  `set_monitor_wake_interval` (§6.2): no row → `404`, detail
  `monitor has never run for this fleet`. The body parse precedes the fleet
  check, matching `POST /api/messages/send`. Success → `200`
  `{wake_interval_seconds: <the stored value>}`. The running loop obeys the
  new value within one tick (§6.6); the next `cafleet monitor` start re-stamps
  the column from the CLI/env resolution.
- **`POST /api/monitor/wake`** — fleet-scoped. No request body; any body is
  ignored. Resolution order: header errors (the shared dependency), then the
  fleet check (`404`, `Fleet not found`), then a `monitor_is_live` gate
  (§6.2): not live — no runtime row, a cleared slot, or a stale heartbeat —
  → `404`, detail `monitor is not running for this fleet`; then
  `request_monitor_wake` (§6.2) with the current UTC time: `false` (the row
  vanished between the check and the write, e.g. a concurrent `fleet
  delete`) → the same `404`, detail `monitor is not running for this fleet`.
  Success → `200` `{wake_requested_at: <the stored UTC ISO timestamp>}`.
  The 404 gate is **liveness**, not row existence as in `PATCH
  /api/monitor` — a wake request needs a live consumer; against a dead loop
  it would silently never fire, whereas the interval is a durable setting.
  The check-then-write pair is not transactional; the race is benign because
  the claim-time reclaim reset clears any request left by a previous loop
  instance. Repeat requests overwrite the timestamp (coalesce into a single
  wake); the running loop honors the request on its next tick (§6.6).
  `GET /api/monitor`'s payload is unchanged — it does not expose
  `wake_requested_at`.
- **`GET /api/members/{member_id}/inbox`** — fleet-scoped. Member not in fleet →
  `404`, detail `Member not found`; else `{"messages": [ <FormattedMessage>, …
  ]}` over the member's inbox.
- **`GET /api/members/{member_id}/sent`** — fleet-scoped. Same as inbox over sent
  messages; same `404` detail `Member not found`.
- **`GET /api/timeline`** — fleet-scoped through the owning member, no per-member
  check. `{"messages": […]}` contains only `unicast` delivery rows, hard-capped
  in SQL at **200** rows ordered by `status_timestamp DESC, message_id DESC`.
  No pagination or group-level limit is introduced. Summary rows neither occupy
  this cap nor count as ACKs; response fields and wrapping stay unchanged.
- **`POST /api/messages/send`** — fleet-scoped. Body `{from_member_id: int,
  to_member_id: int | "*", text: string}`. `to_member_id` deserializes as **either
  a JSON integer or the exact JSON string `"*"`** (broadcast); anything else
  (e.g. a stringified integer `"5"`) is rejected, not coerced. If `from_member_id`
  is not in the fleet → `400`, detail `from_member not in fleet`. If `"*"`:
  broadcast, return `{message_id: <summary message_id>, status: <summary
  status_state>}`. Else: recipient not an **active** member in the fleet →
  `404`, detail `Member not found`; otherwise send and return `{message_id,
  status}`. Both branches:
  `{message_id: int, status: string}` (`status` = the broker message's `status_state`).
  The unicast branch consumes only the `message` object of the broker send
  outcome and intentionally ignores its `notification_error` (§6.2) — the
  `200` response after persistence is unchanged whatever the notification
  outcome.
  The SPA always submits `from_member_id = director.member_id` (the fleet's
  root Director); the endpoint itself is sender-agnostic.

**`FormattedMessage`** (one element of any `messages` array): `{message_id,
from_member_id, from_member_name, to_member_id, to_member_name, type, status,
created_at, status_timestamp, origin_message_id, body}`. Names are resolved by a
single bulk lookup over the union of all `from_member_id`/`to_member_id` values,
using checked keyed lookup — a missing required id/name returns an integrity
error (→ 500 with a detail string), never a panic or a silent fallback. `status` is the renamed `status_state`; `body` the renamed
`text`; `type` the raw row type. Empty input → empty array.

The TypeScript wire model uses `type` to distinguish `unicast` deliveries
(non-null recipient id/name) from `broadcast_summary` rows (null recipient
id/name). Inbox, sent, and timeline consumers accept delivery rows. This models
the existing wire fields; it does not add a discriminator to the HTTP response.

##### Timeline grouping and ACK display

Group unicast deliveries by `origin_message_id`, leaving null-origin messages
standalone. Exclude summaries. Order groups by their earliest creation time;
count recipients and ACKs only among fetched deliveries. The 200-row cap can
split a broadcast, so the UI labels these as displayed-delivery counts.

#### `cafleet server` launcher

Runs the WebUI app under the **built-in HTTP server** (in-process — no
external server program). `--host` (default `settings.broker_host`)
and `--port` (default `settings.broker_port`, integer) both read their defaults
from settings at command-definition time and are shown in `--help`. Serves the
app in a single process **with no auto-reload**. Because the defaults
come from settings, `CAFLEET_BROKER_HOST` / `CAFLEET_BROKER_PORT` are honored
indirectly. The schema-version guard (§6.3) runs before the
server starts. This is the only entry point to the HTTP server.

#### Configuration

All configuration detail — the env-var table, defaults, single explicit binding
per field (no prefix magic), loud failure on bad numerics, and the `max_text_len`
truncation scope — is specified in §7.1.

#### Invariants

1. **Router before static** — `/api/*` 404s serialize as JSON, never the SPA
   `index.html`.
2. **Reserved-prefix hard-404** — first path segment `ui` or `api` never falls
   back to `index.html`.
3. **Stale monitor never leaks process fields** — when not live, `pid` /
   `started_at` / `last_tick_at` / `last_tick_age_seconds` / `last_wake_at` /
   `last_wake_age_seconds` are all `null` even if
   a stale row exists; only `tick_seconds` and `wake_interval_seconds`
   survive from a stale row.
4. **Field renames are wire contract** — `status_state → status`, `text → body`.
5. **`list_fleets` returns a bare array**; every other list wraps in
   `{"members"|"messages": [...]}`.

---

## 7. Cross-cutting concerns

### 7.1 Configuration & environment

Owned by the `config` module (§6.8). A process-wide singleton, reachable from
every module. Each field binds to exactly one named env var (the exact
`CAFLEET_*` name below) with the documented default; there is **no** prefix
binding, so an unrelated `CAFLEET_*` variable never binds by accident.

| Field | Env var | Type | Default |
|---|---|---|---|
| `database_url` | `CAFLEET_DATABASE_URL` | string | `sqlite:///` + `~/.local/share/cafleet/cafleet_v6.db` (home expanded **at startup**) |
| `broker_host` | `CAFLEET_BROKER_HOST` | string | `"127.0.0.1"` |
| `broker_port` | `CAFLEET_BROKER_PORT` | integer (16-bit port) | `8000` |
| `max_text_len` | `CAFLEET_MAX_TEXT_LEN` | non-negative integer | `200` |
| `multiplexer` | `CAFLEET_MULTIPLEXER` | optional string | `None` (auto-detect) |
| `monitor_wake_interval` | `CAFLEET_MONITOR_WAKE_INTERVAL` | non-negative 64-bit integer | `600` |

- **`multiplexer`** is the explicit backend override consumed by
  `resolve_multiplexer()` (§6.5). `None` (unset) means auto-detect from
  `HERDR_ENV` / `TMUX`.
  A set value must name a registry key (`tmux`/`herdr`) or resolution raises.
- **`monitor_wake_interval`** is the `cafleet monitor` wake interval in
  seconds, overridden per-invocation by `--interval` (§6.3). `0` disables the
  wake while the loop keeps heartbeating every tick. The value is a
  non-negative 64-bit integer; anything else fails loudly at startup with
  `CAFLEET_MONITOR_WAKE_INTERVAL must be a non-negative integer (got '<raw>')`.
  The startup-resolved value is stamped into
  `monitor_runtime.wake_interval_seconds` at each `cafleet monitor` start
  (§6.6);
  dispatch cadence is
  persisted in `monitor_runtime.last_wake_at` (§6.2), durable across loop
  restarts.
- **Default DB URL** expands `~` to `$HOME` **only for the factory default**; a
  user-supplied `CAFLEET_DATABASE_URL` is passed through verbatim (no `~`
  expansion, so a user value must already be absolute). Net default on home
  `/home/u`: `sqlite:////home/u/.local/share/cafleet/cafleet_v6.db` (four slashes).
- A non-integer `broker_port`/`max_text_len` must **fail loudly at startup** (a
  hard validation error, not a silent default).
- `max_text_len` truncates only CLI echo + the broker inline-preview keystroke.
  It is **never** applied by the WebUI API (raw broker results) and never
  truncates the persisted `Message.text` column.

**Spawned-pane environment.** The only environment variable forwarded into a
spawned member pane is `CAFLEET_DATABASE_URL` (by `member create`'s
`split_window env={…}`, and only when set). No identity environment variable
exists at runtime — identity reaches a spawned member exclusively through
`member create`'s `{fleet_id}` / `{member_id}` / `{director_member_id}` /
`{coding_agent}` placeholder substitution (§6.3).

### 7.2 Error handling & exit-code policy

A unified, two-tier error model maps the exception taxonomy to exit codes. Use
per-module error types that carry an **exit-code class**, and a single top-level
printer that writes `Error: <message>` to stderr.

| Error class | Exit | Meaning | Mapping |
|---|---|---|---|
| usage error | **2** | argument/parse/usage mistakes: missing required option; unknown option; invalid integer; integer-range violations; mutually-exclusive-option violations; the spawn-prompt placeholder errors; explicit usage errors | a usage-class error; prints `Error: <msg>` (+ usage line). Parser-native parse errors already exit 2. |
| application error | **1** | application/runtime errors: runtime conflicts (one-monitor rule, not-enrolled, not-found-on-delete), the root-Director-deregistration guard, and the spawn rollback ladder | an app-class error; prints `Error: <msg>`. |
| value-error (broker/messaging/queries) | translated by caller | callable from CLI **and** WebUI; CLI wraps to exit 1, WebUI maps to HTTP status | distinct error variants. |
| HTTP error | — | serialized `{"detail": <string>}` | HTTP error responses with the same status + body. |

The root-Director-deregistration guard raises a single **application error
(exit 1)** on both the broker side and the `member delete` CLI side.

**Fail-fast points (never silently fall back):**

- Positional subject ids have **no environment default** (§6.3) and **must
  not** default to an arbitrary fleet, member, or message; a spawned member
  reads its ids from its spawn prompt.
- The broker derives fleet and recipient from the subject row (§6.2); an
  unknown subject fails loudly (`Member {member_id} not found` / `Message
  {message_id} not found`), never silently proceeds.
- `doctor` reads the resolved backend's presence env var (`TMUX` / `HERDR_ENV`)
  via `os.environ.get(presence_var, "")`; an empty value is legitimate under an
  explicit `CAFLEET_MULTIPLEXER` override, so the fail-fast lives upstream in
  `resolve_multiplexer()` + `ensure_available()`, not in this read.
- `register_member`'s root-Director invariant guard raises rather than
  silently registering against an inactive Director.
- opencode's `ensure_available` raises on a missing preset file rather than
  writing one.
- Broker "exactly one row" invariants raise if the assumption breaks — keep
  fail-loud.

Exact error strings are catalogued per module in §6; the **strings** are part of
the contract and should be reproduced (the relaxation in §1 concerns incidental
formatting artifacts, not these deliberate user-facing messages). Notable
cross-module string — the "must be run inside a … session" text exists in two
distinct forms with **two different provenances**:

- **Pane-command path** — the resolved backend's `ensure_available` raises its own
  backend-specific text (tmux: `cafleet member commands must be run inside a tmux
  session`, §6.5; herdr: its own `HerdrError` text); the CLI surfaces that text
  as-is (it does not hardcode it). Before `ensure_available` runs,
  `resolve_multiplexer()` may itself raise a `MultiplexerError` when no backend is
  detected or the environment is ambiguous (§6.5).
- **`fleet create` path** — the CLI **catches** the multiplexer's
  `MultiplexerError`, **discards** its message, and raises its own hardcoded
  command-specific string `cafleet fleet create must be run inside a tmux or herdr
  session` (§6.3, exit 1). This one is genuinely CLI-hardcoded; do not expect it
  to echo the backend's own wording.

### 7.3 Output / JSON / truncation

Output formatting is specified in §6.4. The cross-cutting choices: the CLI selects
text-vs-JSON with the shared per-subcommand `--json` flag (§6.3) — text is
always the truncated human form, JSON always the complete untruncated machine
form; the WebUI bypasses `truncate_*`
(raw broker results) but its JSON serialization still preserves key order and raw
UTF-8 (no ASCII escaping).

### 7.4 Logging & stdout discipline

- The monitor loop emits one heartbeat line per delivered wake to **stdout**
  (`{iso} tick -> wake monitor {monitor_member_id} ({N} members)`).
- Creation rollback diagnostics go to **stderr**, following the primary cause.
  Preserve the stream
  choice (stdout vs. stderr) — it is part of the observable contract.

### 7.5 Time discipline

Every "now" is timezone-aware UTC; every DB-boundary write serializes to the
canonical ISO-8601 string. See §5.1 — string comparison for ordering, parse only
for age math.

### 7.6 Packaging & distribution

`cafleet` ships as a single binary. Install with Homebrew
(`brew install himkt/tap/cafleet`) or download an archive from GitHub Releases.
The release workflow also publishes Homebrew bottles; Linux release targets
use musl. Skills, presets, migrations, and WebUI files are embedded, not
separate runtime downloads.

- **Release targets:** `aarch64-apple-darwin`, `x86_64-unknown-linux-musl`,
  `aarch64-unknown-linux-musl`.
- **Tag = bare version:** a release is tagged with the bare version string
  (e.g. `0.22.0`) — no `v` prefix.
- **Assets:** one archive per target, named
  `cafleet-v<version>-<target>.tar.gz`, each containing exactly one file: the
  `cafleet` binary.
- **Version string:** the binary's compile-time package version is the single
  canonical version. It feeds `--version` (`cafleet <version>`), the
  stale-assets guard comparison (§6.3), and the
  `asset_installs.cafleet_version` rows (§5.2).
- **Embedded data:** the skills tree, the presets (§6.7), the migration chain
  (§8), and the admin WebUI dist (§6.8) are embedded in the binary at build
  time; `cafleet setup` installs assets offline from the embedded data (§6.3)
  with no network access.

### 7.7 Agent-operation contract — one-shot command isolation

How a coding agent invokes the CLI is part of the operating contract. The
normative rule ships in the core cafleet skill and is backend-neutral —
identical for the claude, codex, and opencode backends:

- Every one-shot `cafleet` process is the only command in its shell-tool
  invocation; a sequence of CAFleet operations runs as separate shell-tool
  calls.
- A one-shot CAFleet command is never placed beside another command via a
  newline, `;`, `&&`, a pipe, shell `&`, or any other setup/follow-up command.
  A compound invocation keeps the agent's shell tool occupied after the
  CAFleet process exits, so the pane cannot consume an inbound inline-preview
  keystroke (§6.5) and a notification aimed at it can fail after the message
  was persisted.
- Leading `NAME=value` assignments immediately preceding the CAFleet
  executable are allowed — they set the CAFleet process environment without
  starting another process. An `env` helper process is not a substitute.
- Shell redirection does not authorize another process; a long body uses the
  positional argument or `--file <path>` (§6.3), not a pipe.

**Permission-error diagnostic.** A CAFleet command that fails with an
operating-system permission error — `Operation not permitted` /
`Permission denied`, commonly surfacing as a multiplexer socket or
pane-command failure — signals that the invocation likely ran outside the
coding agent's command auto-approval scope: a compound invocation does not
match single-command allow rules, so the shell tool executes it under the
agent's restricted sandbox or permission set. The response is to re-run the
CAFleet command as its own isolated invocation, honoring the no-resend rule
whenever a persisted message id was already reported; retrying the compound
form is forbidden.

The sole exception is the long-lived `cafleet monitor` process (§6.6): its
invocation still contains only that monitor process, but it may use the
background or managed-execution mechanism resolved by the member's
coding-agent overlay (part of the installed skill assets). The core rule
defers that launch syntax to the overlay and does not duplicate it.

The companion recovery contract for a `message send` partial failure — the
persisted id proves the send committed, the sender never resends the body, and
recovery is an isolated `cafleet member ping <recipient-id>` or a
recipient-side isolated `cafleet message poll <recipient-id>` followed by a
normal ACK — is pinned in §6.3 and also ships in the core skill's Send
guidance. No layer retries the notification.

---

## 8. Database schema

**Schema** = the six application tables of §5.2 (`fleets`, `members`,
`messages`, `member_placements`, `monitor_runtime`, `asset_installs`) plus the
migration ledger table `refinery_schema_history` (the bookkeeping table
recording one row per applied migration: version, name, applied-on timestamp,
checksum). The schema
is created and evolved by a **chain of embedded SQL migrations** — numbered
files `V<N>__<slug>.sql`, contiguous from 1, compiled into the binary;
applying the chain in place preserves existing data (§11; the one exception
is `V6`, which restarts `asset_installs` empty — see the chain below). Column types,
defaults, FK rules, AUTOINCREMENT, and the create-order quirk are in §6.1.

**Indexes (non-unique), at head:**

- `idx_members_fleet_status` on `members(fleet_id, status)`
- `idx_messages_owner_member_status_ts` on `messages(owner_member_id, status_timestamp)`
- `idx_messages_from_member_status_ts` on `messages(from_member_id, status_timestamp)`

**Partial unique index, at head:**

- `idx_members_one_active_monitor_per_fleet` on `members(fleet_id)` where
  `status = 'active' AND json_extract(member_card_json, '$.cafleet.kind') = 'monitor'`.
  This is the `active_monitor_member_id` predicate. It applies to INSERT and
  UPDATE of status, card, or fleet id. Ordinary members and deregistered
  monitors are excluded, and fleets are independent. Root Director card
  generation omits the monitor marker; Director-first display-kind resolution
  (§5.4) is unchanged and does not override this index predicate.

**The migration chain.** Head is **`V8`**, contiguous from 1 with exactly one
baseline:

1. `V1__baseline.sql` — the baseline, creating the schema in this order:
   `members` (+ `idx_members_fleet_status`) — created **first** because every
   other FK-bearing table references it; `members.fleet_id` forward-references
   the still-uncreated `fleets` (§6.1), AUTOINCREMENT; `fleets`, AUTOINCREMENT;
   `asset_installs` — TEXT PK `coding_agent`, no AUTOINCREMENT, no FK
   constraint, columns `coding_agent` TEXT PK, `cafleet_version` TEXT NOT NULL,
   `installed_at` TEXT NOT NULL (recreated with the composite
   `(coding_agent, path)` key at `V6`, below — §5.2 carries the head-shape
   semantics); `member_placements` — PK=FK `member_id`,
   not AUTOINCREMENT, `backend` DDL default `"tmux"`, `coding_agent` NOT NULL
   with no DDL default; the per-member monitor schedule table — PK=FK
   `member_id` ON DELETE CASCADE, `interval_seconds` default 60, `enabled`
   default 1, not AUTOINCREMENT (dropped at `V4`, below; its exact DDL is the
   `V1` migration file's content); `monitor_runtime` — PK=FK
   `fleet_id` ON DELETE RESTRICT, `tick_seconds` default 5, not AUTOINCREMENT;
   `messages` (+ `idx_messages_owner_member_status_ts`,
   `idx_messages_from_member_status_ts`), AUTOINCREMENT.
2. `V2__drop_director_monitor_enrollment.sql` — data migration: deletes the
   rows owned by any fleet's `director_member_id` from the per-member monitor
   schedule table (the root Director was never a supervision watch target).
3. `V3__strip_monitoring_member_kind.sql` — data migration: removes the whole
   `$.cafleet` object from `member_card_json`
   (`json_remove(member_card_json, '$.cafleet')`) on every row whose
   `$.cafleet.kind` carries the retired per-member enrollment marker (the
   migration file names the literal) — `cafleet.kind` was the sole key under
   `$.cafleet`, so the whole object goes. The marker `register_member` writes
   for the fleet's monitor member, `$.cafleet.kind == "monitor"` (§5.4,
   §6.2), is a different literal value, so `V3` never touches a monitor
   member's card.
4. `V4__fleet_level_wake_schedule.sql` — drops the per-member monitor
   schedule table (supervision cadence lives solely on `monitor_runtime`) and
   runs `ALTER TABLE monitor_runtime ADD COLUMN last_wake_at TEXT` (nullable,
   no DDL default).
5. `V5__monitor_wake_interval.sql` — runs
   `ALTER TABLE monitor_runtime ADD COLUMN wake_interval_seconds INTEGER`
   (nullable, no DDL default).
6. `V6__path_aware_asset_installs.sql` — drops and recreates `asset_installs`
   with the composite `(coding_agent, path)` key. A `PRIMARY KEY` change has
   no in-place `ALTER TABLE` form in SQLite, and the table has no FK parents,
   so drop-and-recreate is the legitimate path; the table restarts empty —
   the old rows' install locations were `$HOME`-dependent hard-coded
   defaults, and a plain-SQL migration cannot know `$HOME` to backfill a
   truthful `path` value, so an empty restart is the only non-fabricated
   option (every machine re-runs `setup` per agent after upgrading). Head
   `asset_installs` schema:
   ```sql
   DROP TABLE asset_installs;

   CREATE TABLE asset_installs (
       coding_agent TEXT NOT NULL,
       path TEXT NOT NULL,
       cafleet_version TEXT NOT NULL,
       installed_at TEXT NOT NULL,
       PRIMARY KEY (coding_agent, path)
   );
   ```
7. `V7__monitor_wake_request.sql` — runs
   `ALTER TABLE monitor_runtime ADD COLUMN wake_requested_at TEXT`
   (nullable, no DDL default). Head `monitor_runtime` schema:
   ```sql
   CREATE TABLE monitor_runtime (
       fleet_id INTEGER NOT NULL PRIMARY KEY REFERENCES fleets (fleet_id) ON DELETE RESTRICT,
       pid INTEGER,
       started_at TEXT,
       last_tick_at TEXT,
       tick_seconds INTEGER NOT NULL DEFAULT 5,
       last_wake_at TEXT,
       wake_interval_seconds INTEGER,
       wake_requested_at TEXT
   );
   ```
8. `V8__unique_active_monitor.sql` — adds the partial unique index:
   ```sql
   CREATE UNIQUE INDEX idx_members_one_active_monitor_per_fleet
   ON members(fleet_id)
   WHERE status = 'active'
     AND json_extract(member_card_json, '$.cafleet.kind') = 'monitor';
   ```
   Existing duplicate active monitors make this DDL fail. No survivor is
   selected automatically, and existing migration files remain unchanged.

There are no CHECK constraints. Future schema changes are hand-written
numbered files `V<N>__<slug>.sql` appended to the chain; the chain stays
contiguous from 1 with exactly one baseline.

A fresh DB migrated to head starts with **no rows in any application table**
(`V2`/`V3` are no-ops on an empty database). `asset_installs` rows are
written at install time, not by the schema.

**`setup` db-migration driver** (the db half of `setup`, §6.3). Procedure: (1)
validate the URL scheme is `sqlite` (§6.1); (2) extract the DB file path — if
empty → application error `database URL has no file path`; (3) create the
file's parent directory; (4) inspect the DB: existing tables but no
`refinery_schema_history` → the **unversioned-DB refusal**, application error
`DB has existing tables but no refinery_schema_history. Refusing to migrate an
unversioned database.`; a recorded version greater than the embedded chain's
head → the **ahead-of-head refusal**, application error `DB schema is at
version <M> which is unknown to this version of cafleet. Refusing to downgrade
automatically.`; (5) already at head (recorded version == head) → print
`Already at head (<N>); nothing to do.` and stop; (6) before applying pending
migrations, if `members` exists, query duplicate fleets using the partial
index's exact predicate, with fleet ids and member ids in ascending order.
A duplicate fails the DB half with
`active monitor duplicates prevent migration: fleet <id>: members <ids>; ...`.
Preserve all member/placement rows, panes, and schema history: diagnosis makes
no changes and does not choose a survivor. A new database without `members`
skips this check; (7) apply all pending migrations in refinery's grouped
transaction. If index creation fails (including a duplicate introduced after
step 6), roll back the entire pending group and its ledger writes. Re-query
for duplicates after failure: if that succeeds and finds duplicates, report
the same diagnostic; otherwise preserve the original migration error. On
success print `Created <db_file> and applied migrations
to head (<N>).` when no version was recorded before the run (a fresh or
table-less DB), else `Upgraded from <M> to <N>.`. `<M>` / `<N>` are the
integer migration versions (the head is `8`). The driver's
connection is closed when the command finishes (success or failure).

**Duplicate-monitor recovery.** Use the operator procedure in
[Storage](docs/docs/concepts/storage.md#duplicate-monitor-recovery).

## 9. Testing strategy

- **Unit:**
  - *Broker* against an **in-memory SQLite** (`:memory:`) with the same pragmas
    (`foreign_keys=ON`); assert FK cascade/restrict, the status lifecycle, the
    nested-team guard, and the error strings/types.
  - *Output* — golden tests: every `format_*`/`render_*` against fixed inputs,
    asserting the layout (column alignment, the single ASCII `-` absent glyph,
    codepoint truncation with `…`, compact-JSON key order).
  - *Multiplexer* — inject a **fake command runner** (no real tmux) and assert
    exact argv lists, the Esc-first/`-l`/Enter ordering, the two sleeps, the
    sanitizer substitutions, and the per-method failure contracts (best-effort
    boolean, the result-returning inline preview with its exact missing-binary
    strings, and fail-fast raising).
  - *Coding-agent* — assert each `build_spawn_argv` argv, the opencode model
    validation, and each backend's `ensure_available` preconditions (PATH
    check; opencode's preset-existence check at the resolved preset path)
    against a temp HOME and an injected env lookup.
  - *Monitor* — `wake_due` is pure (table-test interval gating,
    `last_wake_at` precedence, the `started_at` baseline, and corrupt
    stamps); `monitor_tick` against a fake broker+multiplexer asserting the
    zero-interval heartbeat-only tick, the `woke`-gated `record_monitor_wake`,
    and the `STOP` paths.
  - *Config* — env-var parsing, the default-URL home expansion, and loud failure
    on non-integer port/len.
- **Integration:**
  - End-to-end DB lifecycle: `create_fleet → register_member → send_message →
    poll → ack`, asserting persisted rows and soft-delete cascade.
  - WebUI: spin the app over an in-memory/temp DB; assert each route's status
    codes, the wire renames, the bare-array vs. wrapped shapes, the `X-Fleet-Id`
    errors, and the SPA/reserved-prefix fallback.
  - Monitor claim/heartbeat/clear concurrency: two "processes" (distinct fake
    pids) racing `claim_monitor_runtime`; assert single-winner and the displaced
    loser self-terminates (`heartbeat` returns false) + no-op clear.
- **CLI conformance:** drive the built `cafleet` against a temp DB and compare
  stdout/stderr/exit-code to **this specification** for every command in §10 —
  both text and `--json` modes, both success and each error path. Compare at the
  level of *structure and semantics* (same fields, same JSON shape, same exit
  code), not necessarily byte-for-byte. The §6 contract strings and §10 command
  checklist are the golden reference for intent.

---

## 10. CLI command checklist

The full command surface — **16 subcommands across 3 groups + 4 top-level
commands**, one of which (`monitor`) is two-form: the bare loop plus its
`scan` subcommand.
Each must be reproduced with identical positional/option names, types,
defaults, required-ness, output shapes, and exit codes. Per-command
argument semantics are in §6.3.

**Global:** `--version` (`cafleet <version>`, exit 0, short-circuits before
subcommand dispatch).
The shared trailing `--json` flag (§6.3) is listed per row below.

**Top-level:**

- [ ] `cafleet setup` (`--coding-agent AGENT...` multi-value/repeatable choice; no positional arguments; runs the db half then the assets half per the selector semantics — the named agents, or all three on the no-flag form)
- [ ] `cafleet doctor` (`--json`; the three-section diagnosis — multiplexer, database, coding agents — no early abort, exit 1 iff any issue)
- [ ] `cafleet server` (`--host`=settings.broker_host, `--port`=settings.broker_port)
- [ ] `cafleet monitor FLEET_ID` (the loop form; `--tick`≥1=5, `--interval`≥0=`CAFLEET_MONITOR_WAKE_INTERVAL` (default 600); prints the startup line after a successful runtime claim)
- [ ] `cafleet monitor scan FLEET_ID` (`--lines`≥1=**20**, `--ansi`, `--json`; one-shot batch capture — Director first, then members ascending; annotated entries still exit 0)

**`fleet`:**

- [ ] `cafleet fleet create` (`--name`, `--coding-agent`, `--monitor-file PATH` required, `--monitor-model` optional, `--json`; atomic fleet + Director + monitor bootstrap)
- [ ] `cafleet fleet list` (`--json`)
- [ ] `cafleet fleet show FLEET_ID` (`--json`)
- [ ] `cafleet fleet delete FLEET_ID` (`--json`)

**`member`:**

- [ ] `cafleet member create` (no identity flag — Director auto-resolved; `--fleet-id` required, `--name`, `--description`, `--coding-agent`, `--model`, `--effort`, `--role` (optional, sole accepted value `monitor`), positional `PROMPT` / `--file PATH` xor-required, `--json`)
- [ ] `cafleet member delete MEMBER_ID` (`--json`; pane path kills immediately and always exits 0; placementless target → registry soft-delete, exit 0)
- [ ] `cafleet member show MEMBER_ID` (`--json`)
- [ ] `cafleet member list FLEET_ID` (`--json`)
- [ ] `cafleet member prompt MEMBER_ID TEXT` (`--shell`, `--json`)
- [ ] `cafleet member ping MEMBER_ID` (`--json`; pending placement skips the keystroke and exits 0, `skipped` key on both JSON paths)
- [ ] `cafleet member capture MEMBER_ID` (`--lines`=**20**, `--ansi`, `--json`)

**`message`:**

- [ ] `cafleet message send` (`--from-member-id` sender, `--to-member-id` recipient, positional `TEXT` / `--file PATH` xor-required, `--json`)
- [ ] `cafleet message broadcast` (`--from-member-id`, positional `TEXT` / `--file PATH` xor-required, `--json`)
- [ ] `cafleet message poll MEMBER_ID` (`--json`)
- [ ] `cafleet message ack MESSAGE_ID` (`--json`)
- [ ] `cafleet message show MESSAGE_ID` (`--json`)

The subject of each command rides as its positional id (§6.3 *Positional
subject ids*); the only id flags are `member create`'s `--fleet-id` and the
`message send` / `broadcast` sender/recipient pair. `setup`, `doctor`, and
`server` take no id at all.

---

## 11. Decisions & clarifications

### Architecture

The concurrency model is an implementation choice (§2). The only requirement is
that the monitor's "SQLite write lock serializes claims" assumption (§6.2) is
preserved.

### Output fidelity

Fidelity is structural and semantic, not byte-for-byte (§1). The host-language
artifacts that need only preserve *intent* (not exact bytes): the `repr()`-style
quoting in `member prompt` echo, the OS-error message suffix in a preset-install
failure, and an exception's exact internal-repr fragment.

### Surface-redesign decisions

The decisions that shape this surface:

- **`member` is the single member-lifecycle surface.** `member` owns member registration, teardown, introspection (`show`, `list`), and keystroke interaction (`create`/`delete`/`show`/`list`/`capture`/`prompt`/`ping`). There is no separate `agent` group.
- **The subject id is positional; relationship ids are flags** (§6.3): the
  fleet and recipient are derived from the subject row (§6.2) rather than
  restated, with **no environment default** for any id.
- **`--json` is the single output switch** (§6.3/§6.4): text is always the
  truncated human form, JSON always the complete untruncated machine form.
- **One error/exit model** (§7.2): usage → exit 2, application/runtime → exit 1.
- **Migration-managed schema** (§8): an embedded chain of numbered SQL
  migrations with the applied versions recorded in `refinery_schema_history`;
  no cross-implementation DB interoperability. Re-running `cafleet setup` (the
  db half runs first) on a database created by this chain applies any pending
  migrations in place and preserves all existing rows, message history
  included; it refuses to auto-downgrade an ahead-of-head database and
  refuses an unversioned database with existing tables. Upgrade path: after
  installing a new release binary, the first fleet-scoped command errors with
  the schema-outdated message (when the release adds migrations) or the
  stale-assets message, and instructs the operator to run `cafleet setup`.
- **Schema-version guard** (§6.3): every non-setup command (the `fleet` /
  `member` / `message` group callbacks, the `monitor` command, and `server`)
  classifies the database against the embedded head before its command body
  and hard-errors with `cafleet setup` (or upgrade-cafleet) guidance instead
  of a raw SQLite error (exit 1); exempt: `setup`, `doctor`.
- **Stale-assets guard** (§6.3): every fleet-scoped surface (the `fleet` /
  `member` / `message` group callbacks and the `monitor` command) validates,
  after the schema-version guard passes,
  each agent's `asset_installs` row at its currently-resolved identity path
  against the runtime CLI version before any subcommand body runs;
  all-uninstalled/stale-at-resolved-path → hard error (exit 1); superseded
  rows at other paths never block; exempt: `setup`, `doctor`, `server`.
- **Nullable `to_member_id`** (§5.5): `NULL` on `broadcast_summary` rows; no `0`
  sentinel.
- **Prompt-substitution identity delivery** (§6.3/§7.1): identity reaches a
  spawned member as literals rendered by `member create`'s `{fleet_id}` /
  `{member_id}` / `{director_member_id}` / `{coding_agent}` placeholder
  substitution; no identity environment variable is injected.
- **Single absent glyph** (§6.4): ASCII `-` everywhere.

### Per-module clarifications

Choices left unconstrained by the contract (each underlying behavior is fully
specified in the cited section):

- **CLI (§6.3):** the `--coding-agent` choice sets (on `fleet create`,
  `member create`, and `setup`) may be
  hardcoded to `claude`/`codex`/`opencode` or data-driven off the registry —
  an implementation choice.
- **Multiplexer (§6.5):** `env` argument ordering in `split_window` is not
  behaviorally significant (tmux treats `-e` flags as a set).
- **Coding agents (§6.7):** the backend registry may be a name→backend map or a
  backend enum — an implementation choice.

### Cross-module consistency notes

- **Timestamps** unified in §5.1 (string storage + comparison; parse for math).
- **Member kind** unified in §5.4 (a three-value discriminator, derived from
  `fleets.director_member_id` plus the member card's `$.cafleet.kind ==
  "monitor"` marker).
- **Policy tunables** (the runtime-liveness stale factor/floor 3/15, and the
  default wake interval 600) have a single home in the broker/config modules,
  re-exported by the monitor module.
- **`settings` singleton** is config-module-owned and reachable from every
  module, not webui-local.
