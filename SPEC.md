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
in full in [§6](#6-module-specifications). Note that this documentation grouping
differs from the **architectural decomposition** in [§3](#3-module-layout) /
[§4](#4-architecture--module-dependency-graph): the dependency graph keeps
`config` as its own leaf module and `cli` as its own unit (nine modules total),
whereas the table below documents `config` inside the WebUI section (§6.8) and
gives `cli` its own section (§6.3). `config` is an independent module regardless
of where it is documented (§3/§4/§11) — the "WebUI + Config" row is a doc-layout
choice, not a merge.

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
and a monitor schedule. The `cafleet` CLI is the primary surface: it creates
fleets, spawns coding-agent members into tmux panes, routes messages between
them by keystroke-injecting inline previews, and runs a heartbeat loop that
keeps a dedicated *monitoring member* periodically woken. An admin WebUI exposes
a read-mostly JSON API over the same broker.

**Goal:** specify the **redesigned** `cafleet` command surface end-to-end so any
implementation can reproduce it. The contract is the *interface and observable
behavior* of the surface defined here, not the internal byte-for-byte mechanics.
This is a deliberate, greenfield redesign: it is **not** behavior-preserving with
respect to any earlier `cafleet`, and reference-parity with a prior
implementation is **not** a goal.

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
  routing and best-effort notification behavior, and the stdout-vs-stderr stream
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

This mirrors the reference implementation's sync-CLI / async-server split:

- CLI invocations stay runtime-free — no async runtime spin-up for a one-shot
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
├── broker          data-access layer
├── monitor         heartbeat loop
├── webui           server half: HTTP app, /api router, SPA fallback
└── cli             command tree + handlers; the cafleet entry point
```

**Entry points:** exactly one user-facing binary/command — `cafleet`. There is
no separate server binary; `cafleet server` constructs the WebUI app and serves
it. The reference implementation's server target maps to "construct the WebUI
application object and serve it".

---

## 4. Architecture & module dependency graph

The ASCII diagram below is an at-a-glance sketch; the **edge list that follows it
is the authoritative dependency contract**. Where the diagram's arrows are hard
to trace, defer to the edge list.

```
                 config  ◄─────────────────────┐ (leaf; CAFLEET_* settings)
                  ▲   ▲   ▲                     │
        ┌─────────┘   │   └──────────┐          │
       db          output          webui        │
        ▲             ▲              ▲   │       │
        │             │              │   └──► broker
        │             │              │          ▲
   multiplexer    coding-agent       │          │ (db + multiplexer + config)
        ▲   ▲          ▲             │          │
        │   └──────────┼─────────┐   │          │
        │           monitor ─────┘───┼──────────┤ (broker + multiplexer)
        │              ▲             │          │
        └────────────  cli  ─────────┴──────────┘
                 (broker, output, multiplexer,
                  coding-agent, monitor, config,
                  db, webui)  → cafleet entry point
```

Edges (who depends on whom):

- **config** — leaf. No internal deps.
- **db** — depends on `config` (reads `settings.database_url`). Owns the
  connection factory and the embedded migration chain.
- **output** — depends on `config` (reads `settings.max_text_len`). Pure
  string/structure transforms otherwise.
- **multiplexer** — leaf (process invocation only). Truncation for inline
  previews is done by the *broker* before calling `send_inline_preview`, so the
  multiplexer needs no config.
- **coding-agent** — leaf (process/PATH checks + the opencode preset-existence
  check).
- **broker** — depends on `db` (models + connection), `multiplexer`
  (`MultiplexerContext` argument to `create_fleet`; `send_inline_preview` for
  inline previews), and `config` (`max_text_len` for preview truncation).
- **monitor** — depends on `broker` (monitor DB ops + `get_fleet`) and
  `multiplexer` (`list_pane_ids`, `send_wake_trigger`).
- **webui** — depends on `broker` and `config`. Treats broker results as
  pass-through payloads (renaming two keys, dropping two).
- **cli** — depends on all of the above; it is the orchestration glue that
  wires broker ↔ multiplexer ↔ coding-agent ↔ output, and embeds `webui` for
  `cafleet server`.

**Reconciled overlap points** (specced once, here, then referenced):

1. **Broker ↔ multiplexer inline preview.** The broker's `_try_notify_recipient`
   (§6.2) looks up the recipient's `mux_pane_id`, skips self-sends and
   paneless recipients, **truncates** `text` to `settings.max_text_len` with a
   `…` suffix, then calls `send_inline_preview` (§6.5) which keystrokes the
   2-line `[cafleet msg …]` payload Esc-first. The multiplexer call is
   **best-effort**: it returns a boolean, never raises, and the broker never
   rolls back the persisted message on a failed keystroke. Truncation happens
   broker-side; the keystroke mechanics are multiplexer-side.
2. **CLI ↔ multiplexer ↔ coding-agent member-create.** `cafleet member create`
   (§6.3) sequences: resolve backend → `validate_model` → `validate_effort` →
   resolve the prompt
   body via the shared `--text` / `--text-file` reader → `ensure_available`
   → broker `register_member` (placement with `mux_pane_id` unset) → substitute
   `{fleet_id}` / `{member_id}` / `{director_member_id}` / `{coding_agent}`
   placeholders (§6.3) → `build_spawn_argv` (§6.7) →
   multiplexer `split_window` (§6.5), forwarding `CAFLEET_DATABASE_URL` (when
   set) into the new pane's environment (§7.1) → broker
   `update_placement_pane_id`. A rollback ladder deregisters the member on any
   post-register failure.
3. **Monitor loop ↔ broker monitor DB ops.** The loop (§6.6) owns the
   OS-facing half (signal handling, sleep, the single keystroke); all DB
   mutation (`claim`/`heartbeat`/`clear`/`record_pings`/`list_monitor_targets`)
   is the broker's (§6.2). The single-instance / split-brain guard lives
   entirely in the broker's runtime-row protocol; the loop only consumes its
   boolean signals.

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
  seconds (float, or integer-truncated where the reference truncates).

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
| `member_card_json` | string | A2A card JSON; carries `cafleet.kind` |

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

**MonitorConfig** (1:1 with Member; `member_id` is PK = FK, not autoincrement)

| Field | Type | Notes |
|---|---|---|
| `member_id` | integer | FK→members, ON DELETE CASCADE |
| `interval_seconds` | integer | DDL default 60; enrollment writes 180 (Director) / 720 (member) |
| `last_ping_at` | optional string | |
| `enabled` | boolean | stored INTEGER 0/1; exposed as boolean at the broker boundary |
| `last_stall_check_at` | optional string | last successfully dispatched stall-check wake |

**MonitorRuntime** (1:1 with Fleet; `fleet_id` is PK = FK, not autoincrement)

| Field | Type | Notes |
|---|---|---|
| `fleet_id` | integer | FK→fleets, ON DELETE RESTRICT |
| `pid` | optional integer | |
| `started_at` | optional string | |
| `last_tick_at` | optional string | |
| `tick_seconds` | integer | DDL default 5 |

**AssetInstalls** (`coding_agent` TEXT PK, not autoincrement, not FK-linked)

| Field | Type | Notes |
|---|---|---|
| `coding_agent` | string | PK; one of `"claude"` / `"codex"` / `"opencode"` |
| `cafleet_version` | string | the CLI's compile-time version string at install time (§7.6) |
| `installed_at` | string | UTC ISO-8601 with microsecond precision |

Writes are upserts. Rows are written by the assets half of `cafleet setup` — one row per target agent (`claude`, `codex`, `opencode`, minus any `--skip`ped agent) — after that agent's skills and preset (where one exists) install successfully, so a row attests skills + preset; the db half never touches the rows. The rows feed the stale-assets guard on every fleet-scoped command group and the `cafleet doctor` assets report.

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

The member "kind" lives in `member_card_json` at JSON path `$.cafleet.kind`.
Two distinct representations coexist; **they are not the same enum** and must
not be unified:

- **Raw card values** (§6.1/§6.2/§6.7): `"monitoring-member"` or **absent**
  (ordinary member/Director). Constant:
  `MONITORING_MEMBER_KIND = "monitoring-member"`.
- **The broker projection** (§6.2) is one **three-value** `kind` — `director`
  (derived: `member_id == fleets.director_member_id`), `monitor` (the card
  marks a monitoring member), else
  `member` — produced by the single `derive_member_kind` collapse over the
  SQL-supplied `is_root` flag and card kind, and shared by `get_member`,
  `list_members`, `list_roster`, and the WebUI roster. There is no parallel
  two-value discriminator.

### 5.5 Nullable `to_member_id` (resolved)

A `broadcast_summary` row has no single recipient, so `to_member_id` is
**nullable** and `broadcast_message` writes **`NULL`** on the summary row. A
`unicast` message always carries a real recipient id. `get_message` (§6.2) and
`format_message` (§6.4, verbose mode) test `to_member_id IS NULL` / `is None` to
decide whether to surface the `to:` endpoint, rather than a truthiness check.
Model `to_member_id` as an **optional/nullable integer**; there is no `0`
sentinel.

### 5.6 Result shapes vs. typed entities

The reference broker returns dictionaries; the output and webui layers are
duck-typed on them. How the boundary is modeled — typed entities (as in §5.2)
for broker results, versus a generic JSON/value type for the output render
walkers (which must handle heterogeneous nested shapes and conditional key
emission) — is an implementation choice and is not part of the contract. The
contract for which fields may be absent is the optional-vs-required field-access
tables in §6.4.

---

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
- **Cross-thread sharing.** The connection is shared across threads (the
  reference disables SQLite's same-thread check). This is part of the contract.
- Post-commit object usability (the reference keeps loaded attributes valid
  after commit) is a reference-ORM artifact and is not part of the contract.

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
  explicitly): `monitor_config.interval_seconds` → `60`,
  `monitor_config.enabled` → `1` (stored as INTEGER 0/1, a boolean-as-int),
  `monitor_runtime.tick_seconds` → `5`. `member_placements.coding_agent`
  carries no DDL default — every writer passes an explicit value.
- **No-FK message columns.** `messages.from_member_id`, `messages.to_member_id`, and
  `messages.origin_message_id` are plain integer columns with **no** FK constraint;
  only `messages.owner_member_id` is FK-constrained (ON DELETE RESTRICT).
- **JSON-path access.** `member_card_json` is queried via SQLite JSON path
  extraction at `$.cafleet.kind`; the reimplementation must support JSON-path
  extraction over that column.
- **Soft delete** lives in `fleets.deleted_at` (a non-null timestamp marks the
  fleet deleted); this layer never physically removes rows. `messages.text` always
  stores the full untruncated body — message text is never truncated at
  persistence.
- All timestamp columns are stored as ISO-8601 text (§5.1);
  `monitor_config.enabled` is stored as an integer 0/1 used as a boolean.

### 6.2 Broker

**Scope:** the synchronous data-access layer shared by CLI and WebUI; the only
module that reads/writes the operational tables (fleets, members, placements,
messages, monitor schedule/runtime, message queries). Owns
transaction boundaries, the member-kind predicates, soft-delete + cascade, the
message status lifecycle, and the monitor single-instance claim/heartbeat/clear. It
performs no OS side effects except one best-effort inline-preview keystroke
during message delivery (§6.5) and one process-liveness probe (signal-0).

#### Session semantics

- **read_session** — opens a read-only connection with no transaction wrapper;
  used by every query/read function.
- **write_session** — opens a connection inside a single transaction that
  commits on clean exit and rolls back on any exception. **Every mutating
  function wraps all of its writes in exactly one `write_session` block** — its
  mutations all commit together or all roll back.
- Three functions — `enroll_member`, `delete_fleet_monitor_rows`,
  `delete_member_monitor_row` — take an existing transaction as their first
  argument and participate in the *caller's* transaction (atomic registration /
  atomic cascade). Every other function opens its own session.
- "Exactly one row" reads (EXISTS / aggregate / single-row lookups) assume
  exactly one row and fail loudly if the invariant breaks. Do not coerce a
  missing row to a default.

#### Kind constants and intervals

How the broker surfaces the kind (the single
three-value `derive_member_kind` collapse shared by `get_member` and
`list_members`) is detailed in §5.4. An absent / null / empty / malformed-JSON /
non-object `cafleet` card value collapses to the ordinary kind — a deliberate,
documented non-match, not an error mask.

- `MONITORING_MEMBER_KIND = "monitoring-member"`.
- Enrollment intervals: the root Director is enrolled at **180 seconds**
  (`DIRECTOR_PING_INTERVAL_SECONDS`) by `create_fleet`; ordinary pane-bound
  members at **720 seconds** (`MEMBER_PING_INTERVAL_SECONDS`) by
  `register_member`. The monitoring member is **never**
  enrolled.
- Liveness staleness: `MONITOR_STALE_FACTOR = 3`,
  `MONITOR_STALE_FLOOR_SECONDS = 15` → `stale_after = max(3·tick_seconds, 15)`.
- Root Director identity strings written by `create_fleet`: name `Director`,
  description `Root Director for this fleet`.

#### Fleets

- **`create_fleet(name, director_context, coding_agent)`** — atomically
  bootstraps a fleet and its root Director in one
  write_session. Order: stamp `created_at`; insert the fleet with
  `director_member_id = NULL`; insert the Director member row (`name="Director"`,
  `description="Root Director for this fleet"`, `status="active"`, card
  `{name, description, skills:[]}` with **no** `cafleet.kind`); insert the
  Director's placement (the root Director is pane-bound and keeps its own
  placement row) carrying the multiplexer identity (`mux_session`
  / `mux_window_id` / `mux_pane_id`), the `backend` (the resolved `mux.name`),
  and `coding_agent`;
  enroll the Director at 180s; back-fill the fleet's `director_member_id`.
  Returns `{fleet_id, name, created_at, director:{…}}`.
- **`list_fleets()`** — one record `{fleet_id, director_member_id, name,
  created_at, member_count}` per non-soft-deleted fleet (`deleted_at IS NULL`);
  `member_count` counts only **active** members (0 for empty fleets). Ordering:
  **`created_at DESC, fleet_id ASC`**.
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

- **`register_member(fleet_id, name, description, skills, placement, kind)`** —
  pre-transaction validation, then one write_session. Pre-checks: `get_fleet`;
  if None → usage error `Fleet '{fleet_id}' not found.`; if `deleted_at` set →
  usage error `fleet {fleet_id} is deleted`. Build the card `{name, description,
  skills: skills or []}`, adding `cafleet:{kind}` when `kind` is given. The
  `placement` dict carries no director id — the fleet row is the single source
  of the Director identity. Inside the transaction:
  - **Monitoring-member guard** (only when `kind == "monitoring-member"`): if
    `placement` is None → application error `a monitoring member must be
    pane-bound; register it via 'cafleet member create --role monitor'
    (placement required).`; if the fleet already has an active monitoring member
    → application error `fleet {fleet_id} already has an active monitoring
    member (member {existing}); only one is allowed.`. **This is the single
    one-monitor-per-fleet enforcement site** — the CLI passes `kind` through
    unchecked.
  - **Root-Director invariant guard** (only when `placement` is given): the
    fleet's `director_member_id` must reference an active member of the fleet;
    violation → application error `fleet {fleet_id}'s root Director (member
    {id}) is not active.` — a loud invariant failure, not a usage error, since
    the value is not user input. Nested teams stay impossible by
    construction: no caller supplies a director id.
  - Insert the member row; if `placement` given, insert it; then, **only if
    `kind` is not the monitoring-member kind**, enroll the member
    at 720s.
- **`get_member(member_id, fleet_id)`** — **active only**. Returns `{member_id,
  name, description, status, registered_at, kind, skills, placement}` where
  `skills` is the card's `skills` list (usually `[]`) and `kind` is one of three
  values: `director` (derived: `member_id == fleets.director_member_id`),
  `monitor` (the card marks
  a monitoring member), else `member`; `placement` is None if absent.
- **`deregister_member(member_id)`** — soft-delete one member + drop placement +
  monitor row. If the member is the root Director of any fleet → **application
  error (exit 1)** `cannot deregister the root Director; use 'cafleet fleet
  delete' instead`. The root-Director guard raises a single
  **application** error (exit 1) here on the broker side and identically on the
  `cafleet member delete` CLI side (§6.3) — one error model for the same
  string and condition. Flip `active → deregistered` (stamp `deregistered_at`);
  if a row was flipped, hard-delete its placement and monitor_config row. Returns
  `true` iff a row was flipped.
- **`update_placement_pane_id(member_id, pane_id)`** — set `mux_pane_id` for the
  member's placement; None if no placement row; else returns the placement
  projection. Called after the multiplexer resolves a spawned pane's real id.
- **`verify_member_fleet(member_id, fleet_id)`** — EXISTS check; **status-
  agnostic** (deregistered members still pass).
- **`get_member_names(member_ids)`** — empty input → `{}` with no query; else a
  map id→name; **status-agnostic**.

#### Members — roster

- **`list_members(fleet_id)`** — every **active** registry row of the fleet:
  active rows LEFT OUTER
  JOIN `member_placements`, joined against `fleets` for the `is_root` flag, the
  card kind derived in SQL via `json_extract`, both collapsed by the single
  `derive_member_kind` path (§5.4); plus three
  correlated per-member aggregates over messages, all filtered to `type !=
  "broadcast_summary"`: `last_sent` (max `status_timestamp` where `from_member_id
  = member_id`), `last_recv` (where `owner_member_id = member_id`), `last_ack` (where
  `owner_member_id = member_id` AND `status_state = "completed"`). Then `idle` against
  a single `now`: take the non-null of `(last_sent, last_recv)`; none → `idle =
  null`; else `most_recent` = lexicographic max of the ISO timestamps, `idle =
  max(0, floor(now − most_recent))` in seconds. Returns `{member_id, name,
  kind, placement, last_sent, last_recv, last_ack, idle}` per row — `kind` is
  the same three values as `get_member`, `placement` is null for placementless
  rows. Backs `member list`.
- **`list_roster(fleet_id, *, include_message_holders=False)`** — every **active**
  registry row of the fleet: active rows LEFT OUTER
  JOIN `member_placements`, joined against `fleets` for the `is_root` flag, the
  card kind derived in SQL via `json_extract`, both collapsed by the single
  `derive_member_kind` path (§5.4). With `include_message_holders=True` (the WebUI
  roster), deregistered members that still own messages (a message exists with
  `owner_member_id = member_id OR from_member_id = member_id`) are also returned, so
  the audit-relevant deregistered set stays visible. Returns `{member_id, name,
  description, status, registered_at, placement}` per row plus `kind` (the same
  three values as `get_member`), with
  `placement` null for placementless rows. Backs `GET /api/members`
  (`include_message_holders=True`); it is not a CLI surface.

#### Messaging

- **`send_message(fleet_id, member_id, to, text)`** — one unicast message + best-
  effort notify, one write_session. Coerce `to` to int; on failure → value error
  `Invalid destination format: {to}`. If the sender is not active in the fleet →
  value error `Sender member not found or not active in fleet: {member_id}`. Find
  the destination among active members; absent → value error `Destination member
  not found: {to_id}`; in a different fleet → value error `Destination member not
  in fleet: {to_id}`. Build the unicast message (`owner_member_id = to_id`,
  `from_member_id = member_id`, `to_member_id = to_id`, `type = "unicast"`,
  `status_state = "input_required"`, `origin_message_id = null`), insert, then
  `notification_sent = _try_notify_recipient(...)`. The persisted row holds the
  **full untruncated text**. Returns `{message, notification_sent}`.
- **`broadcast_message(fleet_id, member_id, text)`** — fan out one unicast
  delivery per active peer plus one `broadcast_summary` owned by the
  sender. Sender not active → value error `Sender member not found or not active
  in fleet: {member_id}`. Recipients = active members in the fleet, **excluding
  the sender** (the monitoring member and the
  Director **are** included); let `N` = the count of these recipients. Build the
  summary (`owner_member_id = member_id`, `from_member_id = member_id`, **`to_member_id =
  NULL`**, `type = "broadcast_summary"`, `status_state = "completed"`, `text =
  "Broadcast sent to {N} recipients"`), insert it, set its `origin_message_id` to
  its own `message_id` (self-referential), then insert each delivery with
  `origin_message_id = summary.message_id`. **After all deliveries are inserted (still
  inside the same write_session), call `_try_notify_recipient` once per delivery
  and set `delivered` = the count of those calls that returned `true`** (the sum
  of successful best-effort inline previews; a paneless or self-recipient
  delivery contributes 0). Returns a **single-element list** `[{message: <summary>,
  recipients: N, delivered}]` — `recipients` is the real recipient count `N` and
  `delivered` is the best-effort-preview success count; the two diverge when any
  preview fails to land. The two values are kept as **separate fields** and never
  conflated; the CLI surfaces both (§6.3).
- **`_try_notify_recipient`** — best-effort inline preview, returns whether the
  keystroke landed. recipient == sender → `false`; paneless recipient → `false`;
  else **truncate** the preview text to `settings.max_text_len` codepoints (+ a
  single U+2026 `…` suffix when over the limit) and call the multiplexer's
  inline-preview keystroke, returning its boolean. Truncation is broker-side.
  The notification never rolls back the insert; the boolean flows only into
  `notification_sent` (unicast) or the broadcast `delivered` count.
- **`poll_messages(member_id)`** — un-acked deliveries: `owner_member_id = member_id` AND
  `status_state = "input_required"`, `broadcast_summary` excluded, ordered
  `status_timestamp DESC`.
- **`ack_message(member_id, message_id)`** — transitions a message in one
  write_session. Load; absent → value error `Message {message_id} not found`.
  If the caller is not the recipient (`owner_member_id`) → permission error
  `Only the recipient can ACK a message`. If `status_state` is not
  `input_required` → value error `Cannot ACK message in state {status_state}`.
  Set `status_state = "completed"` and `status_timestamp = now`.
  `input_required` is the only state a message may transition from.

#### Queries

- **`list_inbox(member_id)`** — all messages where `owner_member_id = member_id`, any
  state, `broadcast_summary` excluded, ordered `status_timestamp DESC`.
- **`list_sent(member_id)`** — all messages where `from_member_id = member_id`, any
  state, `broadcast_summary` excluded, ordered `status_timestamp DESC`.
- **`list_timeline(fleet_id, limit=200)`** — messages joined to their **sender's**
  member row, filtered to the sender's `fleet_id`, `broadcast_summary` excluded,
  ordered `status_timestamp DESC`, capped at `limit`.
- **`get_message(fleet_id, message_id)`** — fleet-gated. Load; absent → value error
  `Message {message_id} not found`. Build the endpoint set `[from_member_id]`,
  appending `to_member_id` only when it is **non-null** (so a `broadcast_summary`
  row's `NULL` recipient is dropped). If no endpoint member belongs to `fleet_id`
  → value error `Message
  {message_id} not found` (**same message** — the out-of-fleet gate is hidden as
  not-found).

#### Monitor — schedule CRUD & ping recording

- **`enroll_member(session, member_id, interval)`** (in caller's transaction) —
  inserts a monitor_config row (`interval_seconds = interval`, `enabled =
  true`), atomically with the member/placement insert.
- **`find_monitoring_member(fleet_id)`** — locates the monitoring member **by
  card kind** (not a monitor_config row — it is the unenrolled watcher); must be
  active in the fleet **and pane-bound** (a null pane is treated as absent).
  Returns `{member_id, name, pane_id}` or None.
- **`get_monitor_config(fleet_id, member_id)`** — the four schedule fields of
  §5.2, with `enabled` as a boolean; None if not enrolled / not in fleet.
- **`list_monitor_configs(fleet_id)`** — every enrolled member's config in the
  fleet, `enabled` as boolean.
- **`update_monitor_config(fleet_id, member_id, interval_seconds=None,
  enabled=None)`** — if not enrolled → application error `member {member_id} is
  not enrolled in monitoring for fleet {fleet_id}.`. **Partial update** — only
  supplied fields change (`enabled` stored as 0/1). Disabling always clears
  `last_stall_check_at`.
- **`record_pings(member_ids, when)`** — empty list → no-op (no transaction);
  else set `last_ping_at = when` for all listed configs.
- **`list_monitor_targets(fleet_id)`** — one row per **active, enrolled** member
  (the watched set; the monitoring member is excluded by the monitor_config
  join). Each row: `{member_id, name, is_director, pane_id, coding_agent,
  interval_seconds, last_ping_at, enabled, last_stall_check_at, pending_count,
  oldest_pending_ts}`, where
  `pending_count` counts messages with `owner_member_id = member_id`,
  `status_state = "input_required"`, `type != "broadcast_summary"`, and
  `oldest_pending_ts` is `MIN(status_timestamp)` over the same predicate set
  (a second correlated scalar subquery; `None` when the member has no pending
  delivery).

#### Monitor — no broker stall state

The broker exposes **no stall-episode API**: capture classification, quiet
confirmation across two stall-check wakes, the at-most-one fixed ping, and
Director escalation are all instruction-level behavior of the monitoring
member (§6.6), remembered in its own conversation notes. Anything needing
Director attention travels as a plain per-event `send_message` on the ordinary
messaging path — no monitor-specific delivery state exists.

Lifecycle cleanup batches placement-pending/dead rows before due filtering and
clears `last_stall_check_at`. Soft deregistration explicitly deletes the
member's monitor row.

#### Monitor — runtime claim / heartbeat / clear + liveness

The `monitor_runtime` table holds **exactly one row per fleet** (PK = fleet_id)
— the single-instance slot.

- **Liveness predicate** `_is_live(row, now)`: `false` if `pid` or
  `last_tick_at` is null; `stale_after = max(3·tick_seconds, 15)`; if `now −
  last_tick_at > stale_after` → `false`; then probe the process with a signal-0
  (`kill(pid, 0)`): no-such-process → `false`, permission-denied (owned by
  another user) → `true`, success → `true`. Heartbeat freshness is
  authoritative; the process probe corroborates.
- **`claim_monitor_runtime(fleet_id, pid, tick_seconds, when)`** — atomically
  claim the slot (the SQLite write lock serializes concurrent claims). No row →
  insert and return `true`; row exists and **live** → return `false`; row exists
  but **stale** → overwrite and return `true` (reclaim).
- **`heartbeat_monitor_runtime(fleet_id, pid, when)`** — update `last_tick_at =
  when` **only where the current pid equals the caller's pid**; returns `true`
  iff exactly one row matched. **Ownership-checked** — `false` when the slot was
  reclaimed; that `false` is the displaced monitor's self-terminate signal.
- **`clear_monitor_runtime(fleet_id, pid)`** — null the slot's `pid` /
  `started_at` / `last_tick_at` **only where the current pid equals the
  caller's pid**. **Ownership-checked** → a non-owner clear is a no-op, so a
  self-terminating loser never wipes the winner's row.
- **`read_monitor_runtime(fleet_id)`** — `{fleet_id, pid, started_at,
  last_tick_at, tick_seconds}` or None.
- **`monitor_is_live(fleet_id, now)`** — `false` if no row, else `_is_live`. An
  advisory pre-check for `monitor start`; the atomic claim is authoritative.
- **`monitor_runtime_payload(fleet_id, now)`** — the runtime-liveness dict
  consumed by the WebUI `GET /api/monitor`: `{running, pid,
  tick_seconds, last_tick_at, last_tick_age_seconds, started_at}`, with the
  process fields null when the monitor is not live (no row, or a stale/cleared
  heartbeat).
- **`monitor_members_payload(fleet_id, now)`** — the per-member rows consumed
  by the WebUI `GET /api/monitor`: one dict per
  `list_monitor_targets` row — `{member_id, name, role, interval_seconds,
  last_ping_at, last_ping_age_seconds, enabled, pending_count,
  oldest_pending_ts, oldest_pending_age_seconds}` — with `role` =
  `"director"`/`"member"` from `is_director`, and `last_ping_age_seconds` /
  `oldest_pending_age_seconds` computed against the single supplied `now`
  (whole seconds, integer-truncated; `None` when the source timestamp is
  `None`).
- **`delete_fleet_monitor_rows(session, fleet_id)`** /
  **`delete_member_monitor_row(session, member_id)`** (in caller's transaction) —
  in-transaction cascade deletes: the fleet variant deletes the fleet's
  monitor_config rows (by fleet membership) and the monitor runtime row; the
  member variant explicitly deletes the single member's monitor_config row.

#### Soft-delete + cascade summary

- Members and fleets are **never row-deleted** — they flip to
  `status="deregistered"` / `deleted_at` set.
- Placements and monitor rows (`monitor_config`, `monitor_runtime`) are
  hard-deleted by their explicit lifecycle owners.
- **Messages are never deleted** — audit history is permanent.
- Deregistered members remain visible via `verify_member_fleet`,
  `get_member_names` (both status-agnostic), and
  `list_roster(include_message_holders=True)` (when they still own messages); they
  are hidden from `get_member` and `list_members` (active-only).

#### Contract error strings → exception class → exit code

Usage-class → exit 2; application-class → exit 1; value/permission errors are
raised by messaging/queries and translated by the caller (CLI → exit 1, WebUI →
HTTP status); permission errors gate authorization. The exit-code policy is
§7.2; the strings below are the broker's contract.

| Function | Class | Message |
|---|---|---|
| `register_member` | usage | `Fleet '{fleet_id}' not found.` |
| `register_member` | usage | `fleet {fleet_id} is deleted` |
| `register_member` | application | `a monitoring member must be pane-bound; register it via 'cafleet member create --role monitor' (placement required).` |
| `register_member` | application | `fleet {fleet_id} already has an active monitoring member (member {existing}); only one is allowed.` |
| `register_member` | application | `fleet {fleet_id}'s root Director (member {id}) is not active.` |
| `deregister_member` | application | `cannot deregister the root Director; use 'cafleet fleet delete' instead` |
| `delete_fleet` | application | `fleet '{fleet_id}' not found.` |
| `update_monitor_config` | application | `member {member_id} is not enrolled in monitoring for fleet {fleet_id}.` |
| `send_message` | value | `Invalid destination format: {to}` |
| `send_message` | value | `Sender member not found or not active in fleet: {member_id}` |
| `send_message` | value | `Destination member not found: {to_id}` |
| `send_message` | value | `Destination member not in fleet: {to_id}` |
| `broadcast_message` | value | `Sender member not found or not active in fleet: {member_id}` |
| `ack_message` | value | `Message {message_id} not found` |
| `ack_message` | value | `Cannot ACK message in state {status_state}` |
| `ack_message` | permission | `Only the recipient can ACK a message` |
| `get_message` | value | `Message {message_id} not found` (missing and out-of-fleet) |

### 6.3 CLI

**Scope:** the entire `cafleet` command tree (17 commands across 4 groups + 3
top-level commands — §1, §10), the shared option guards, and the `member create`
spawn orchestration + rollback ladder. Orchestration glue only — it wires
broker/multiplexer/output/coding-agent. The command/option checklist is §10; this
section gives the per-command semantics. Exit codes are §7.2; application errors
(exit 1) and usage errors (exit 2) are printed as `Error: <message>` to stderr
(usage errors additionally print a usage line).

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
  subcommand dispatch, so it **bypasses** the `--fleet-id` requirement.

Any other pre-subcommand option — including `--json` — is the parser's
unknown-argument usage error (clap's native rendering, exit 2).

#### The `--fleet-id` required option

`--fleet-id` is a **required integer option** on every subcommand that operates
within a fleet, enforced by a shared option callback (declared optional at the
parser, `expose_value=False`, storing the value on the shared context object):
a missing `--fleet-id` is an **application error (exit 1)** `--fleet-id <int> is
required for this subcommand. Create a fleet with 'cafleet fleet create' and
pass its id.`; a non-integer value is a parse-time usage error (exit 2). Help
text: `Fleet ID (integer); required for this subcommand.`. There is **no
environment default** — the guard never defaults to an arbitrary fleet, and a
spawned member reads its fleet id from the `FLEET ID:` line the CLI rendered
into its spawn prompt (the placeholder substitution below).

Subcommands taking `--fleet-id`: all of `member *`, `message *`,
`monitor *`, plus `fleet show` and `fleet delete`. Commands that do not operate
within a single existing fleet — `setup`, `doctor`, `server`, `fleet create`,
`fleet list` — do not declare it and reject `--fleet-id` with the parser's
unknown-option error (exit 2).

#### Shared flags & the identity options

- `--full` — boolean, default `false`. On `member show`, `member create`,
  `fleet create`, and every `message` subcommand.
- `--json` — boolean, default `false`, dest `json_output`, help `Output in
  JSON format.`; a shared per-subcommand flag (declaration `json_flag` in
  `cli/_helpers.py`), canonically written **trailing**, after all other flags.
  On every `message` subcommand; `member create` / `delete` / `show` / `list` /
  `prompt` / `ping`; `monitor capture`;
  `fleet create` / `list` / `show`; and `doctor`. Emits compact single-line
  JSON instead of text; composes with `--full` (truncation is applied to the
  result before the json-vs-text fork); `--quiet` is a text-only shortcut,
  ignored in the JSON branch.
- `--member-id` — required integer naming **the member in question**, one
  meaning everywhere: the requester on `message poll` / `ack` /
  `show`, and the target on `member delete` / `show` / `prompt` / `ping` and
  `monitor capture`. Help text: `Member ID (the
  member in question)`. Shared declaration `member_id_option` in
  `cli/_helpers.py`.
- `--from-member-id` / `--to-member-id` — required integers naming both parties
  of a two-party command: `--from-member-id` is the sender on `message send`
  and `message broadcast`; `--to-member-id` is the
  recipient on `message send`. Help texts:
  `Sender's member ID` / `Recipient member ID`. Shared declarations
  `from_member_id_option` / `to_member_id_option` in `cli/_helpers.py`.

#### Shared `--text` / `--text-file` body input {#text-body-input}

`message send`, `message broadcast`, and `member create`
resolve their text body through **one shared reader** taking the `--text`
(string) and `--text-file` (string path) pair. Both options are declared with
**no** parser-level `required`; the reader enforces exactly-one-of. Resolution,
in order:

- Neither given → usage error (exit 2) `Provide exactly one of --text or
  --text-file.`.
- Both given → usage error (exit 2) `--text and --text-file are mutually
  exclusive.`.
- `--text <s>` → the body is `s` verbatim; empty or whitespace-only → usage
  error (exit 2) `text may not be empty.`.
- `--text-file -` → the whole body is read from stdin (read to EOF, decoded
  UTF-8); empty or whitespace-only stdin → application error (exit 1)
  `--text-file -: stdin is empty.`.
- `--text-file <path>` → the file is read as **raw bytes and decoded UTF-8 with
  no universal-newline translation** (CRLF/CR survive byte-for-byte); an
  absolute path is used as-is, a relative path resolves against CWD. Error
  surfaces (all application errors, exit 1, keyed on `--text-file`, riding the
  read-bytes exception surface with **no** `is_file()` pre-check so a permission
  failure lands correctly): missing / non-regular file → `--text-file <path>:
  file does not exist or is not a regular file.`; unreadable → `--text-file
  <path>: file is not readable.`; invalid UTF-8 → `--text-file <path>: file is
  not valid UTF-8.`; empty or whitespace-only file → `--text-file <path>: file
  is empty.`.

The body is returned **verbatim** (no stripping). Empty-body rejection is
**uniform** across all three commands and across inline / file / stdin. Long or
multi-line bodies use `--text-file` (or `-` stdin) to bypass the shell's
`ARG_MAX` limit.

#### Shared `message` handler sequence

Every `message` leaf handler (which returns a
broker result) follows one shared sequence, configured per command by a
**required** text renderer and one switch, `requires_member_fleet`. Per
invocation, in order:

1. **Fleet read** — read `fleet_id` from context (the required `--fleet-id`
   option populated it).
2. **Fleet-gate** (only when `requires_member_fleet`) — read the acting member
   id; if absent, a programmer-error application error; if the broker reports
   the member is not in the fleet → application error (exit 1)
   `member <member_id> is not in fleet <fleet_id>.`. **Runs before the handler
   body.**
3. **Handler call.**
4. **Render** — route the result through message truncation + message-list rendering
   (with `full`).
5. **Emit branch** — if the subcommand's `--json` flag was passed, emit compact
   JSON; else call the text renderer with `full`.
6. **Exception wrap** — re-raise an application/usage error unchanged; wrap any
   other exception as an application error (exit 1) carrying its message.

#### `doctor`

Only the shared `--json` flag. Resolves the active backend via `resolve_multiplexer()`
(re-wrapping a `MultiplexerError` as an application error, exit 1), ensures it is
available (`ensure_available()`), discovers the pane context
(`context_discovery()`), and reads the backend's presence env var (`TMUX` for
tmux, `HERDR_ENV` for herdr). Emits a JSON object under a `multiplexer` key
carrying `backend`, `session`, `window_id`, `pane_id`, `presence_var`, and
`presence_value` (or the equivalent text block), followed by the assets-install
report.

**Assets-install report.** Read all `asset_installs` rows (or detect a missing
table). In text mode, append an `assets:` block after the `multiplexer:` block:

```
assets:
  cli_version: 0.6.0
  claude:      0.6.0 (2026-07-04T00:12:09.123456+00:00) ok
  codex:       0.5.0 (2026-06-20T10:00:00.987654+00:00) STALE
```

One line per recorded `asset_installs` row, format `<agent>:  <version>
(<installed_at verbatim>) ok|STALE` where `ok` means the recorded version equals
the runtime CLI version, `STALE` means it differs. `installed_at` is printed
**verbatim** (microsecond precision, exactly as stored). When no rows exist (or
the table is missing):

```
assets:
  (no assets install recorded; run 'cafleet setup')
```

In JSON mode, a `"assets"` key sibling to `"multiplexer"`:

```json
{
  "multiplexer": { "backend": "tmux", "session": "...", "window_id": "...", "pane_id": "...", "presence_var": "TMUX", "presence_value": "..." },
  "assets": {
    "cli_version": "0.6.0",
    "installs": [
      {"coding_agent": "claude", "cafleet_version": "0.6.0", "installed_at": "2026-07-04T00:12:09.123456+00:00", "current": true},
      {"coding_agent": "codex", "cafleet_version": "0.5.0", "installed_at": "2026-06-20T10:00:00.987654+00:00", "current": false}
    ]
  }
}
```

`installs` is an empty array when no rows exist. `current` is `true` when the
recorded `cafleet_version` equals the runtime CLI version, else `false`. Rows
in `installs` are ordered by ascending `coding_agent`. `doctor` is exempt from
the stale-assets guard — it reports instead of blocking.

#### `fleet` group

Does **not** follow the shared `message` handler sequence. `fleet create`,
`fleet list`, and
`fleet show` take the shared `--json` flag and emit JSON when it is set.

`fleet create` and `fleet list` do **not** take `--fleet-id`; `fleet show` and
`fleet delete` take the **required `--fleet-id` option** like every other
fleet-scoped command (§6.3 `--fleet-id`).

- **create** — `--name` (string, **required**; no `--name` → clap's native
  missing-required-argument error naming `--name`, exit 2), `--coding-agent`
  (choice over the coding-agent names, **required**; omitted → clap's native
  missing-required-argument error naming `--coding-agent`, exit 2), `--json`
  (shared), `--full` (documented).
  Requires a supported multiplexer: on a `MultiplexerError`
  → application error `cafleet fleet create must be run inside a tmux or herdr
  session` (exit 1, no DB writes).
- **list** — `--json` (shared). Empty → `No fleets found.`; else a header plus
  one formatted row per fleet (five columns: FLEET_ID / DIRECTOR / NAME / MEMBERS
  left-padded 40 / 40 / 20 / 8, then a trailing unpadded CREATED_AT; nullable
  cells fall back to empty strings).
- **show** — `--fleet-id` (integer, required) + the shared `--json`. Not found →
  application error `fleet '<fleet_id>' not found.`. Text: `fleet_id`, `name`,
  `created_at`, plus a `deleted_at:` line when soft-deleted (soft-deleted rows
  are returned intentionally).
- **delete** — `--fleet-id` (integer, required). Prints `Deleted
  fleet <fleet_id>. Deregistered <n> members.`; idempotent (an already-deleted
  fleet reports 0 members).

#### `message` group

All five follow the shared handler sequence above. Common: the acting member id —
`--from-member-id` (integer, required — the sender) on `send` / `broadcast`,
`--member-id` (integer, required) on `poll` / `ack` / `show`;
`--message-id` (integer, required) on `ack`/`show`; `--full` (documented)
on all; `--quiet` (documented boolean, default `false` — success output is the
bare `message_id`) on `send` and `ack`.

- **send** — also `--to-member-id` (integer, required — the recipient) and the
  shared `--text` / `--text-file` body pair (exactly one required; §6.3
  [text-body input](#text-body-input)). Fleet-gated; truncates message text.
  Prints `Message sent.\n` + the formatted message.
- **broadcast** — also the shared `--text` / `--text-file` body pair (exactly
  one required; §6.3 [text-body input](#text-body-input)). **Not** fleet-gated; the
  result is a list; `--full` → the formatted first message envelope; else `broadcast
  id=<message_id> recipients=<N> delivered=<k>`, where `<N>` is the result's
  `recipients` (the real recipient count, matching `Broadcast sent to {N}
  recipients`) and `<k>` is the result's `delivered` (the count of best-effort
  inline previews that landed). The two diverge when any preview fails to deliver;
  they are reported as **separate fields**, not conflated (the broker computes
  both, §6.2). In JSON mode the result object carries both `recipients` and
  `delivered`.
- **poll** — fleet-gated; indexed message list; empty `No messages found.`.
- **ack** — fleet-gated; prefix `Message acknowledged.\n` + the formatted message.
- **show** — fetches the message within the fleet; text is the formatted message.

#### `member` group — shared resolution helpers

These helpers back the `member` subcommands. The target member is named by
`--member-id` (§1).

- **Require-pane** — given a placement and an action label
  (`capture`/`prompt`), no pane id → application error `member
  <member_id> has no pane yet (pending placement) — nothing to <action>.`.
  `member ping` does not use it — a pending placement takes ping's skip path.
- **Load-authorized-member** — fetch the member within the fleet: not found →
  `Member <member_id> not found`; other fetch failure → `failed to fetch member:
  <error>`; absent placement → application error ``member <member_id> has no
  placement row; it was not spawned via `cafleet member create`.``, unless the
  caller opts into tolerating a missing placement (`member show` and `member
  delete` do — a placementless target resolves successfully). Does **not**
  check pane presence (`member delete` tolerates a pending
  placement). Callers re-fetch by the canonical
  member id.
- **Deregister-with-warning** — best-effort deregister; on failure print a
  `WARNING: rollback deregister failed …` line to **stderr**, do not raise.
- **Rollback-register** — deregister-with-warning, then raise an application
  error `<reason>. Rolled back registration of <new_member_id>.`.
- **Resolve-coding-agent** — explicit `--coding-agent` wins; else (any role,
  flag omitted) inherit the Director's placement coding agent, with three
  error surfaces (Director fetch failure / not found / no placement), each
  prefixed `cannot resolve the member's coding agent:` and ending
  `Re-run with an explicit --coding-agent.`.

#### `member create` — spawn orchestration & rollback ladder

The one genuinely distinct lifecycle op: register **and** spawn a pane. It
takes **no identity flag** — the acting Director is auto-resolved from the
fleet row. Options: `--name` (string, required), `--description` (string,
required), `--coding-agent` (choice, optional — omitted → inherit the
Director's placement backend; the help default text reads `inherits the
Director's backend`),
`--model` (string, optional), `--effort` (string, optional — reasoning-effort
level, validated per backend; help text `Reasoning-effort level (claude, codex
only).`), `--role` (choice over `member`/`monitor`,
default `member`, shown in help), the shared `--text` / `--text-file` body pair
(exactly one required; §6.3 [text-body input](#text-body-input)), and `--full`.
Sequence:

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
3. **Resolve the body** — via the shared `--text` / `--text-file` reader (§6.3
   [text-body input](#text-body-input)): exactly-one-required, `-` stdin, abs /
   CWD-relative path, UTF-8, uniform empty-body rejection. A mutual-exclusivity
   or empty-inline error is a usage error (exit 2); a file / stdin surface is an
   application error (exit 1). Resolved **before any registration or tmux side
   effect**; substitution (step 6) is deferred until the new member id exists.
4. **Preconditions** — ensure tmux available, the backend binary on PATH, and
   discover the tmux context; any tmux/runtime error → application error (exit
   1).
5. **Register the member** — with a placement carrying the tmux session, tmux
   window id, an unset pane id, and the coding agent (no director id — the
   fleet row is the single source); kind = the monitoring-member kind when role
   is `monitor`, else unset. Re-raise an application error verbatim (preserves
   the one-monitoring-member message and the root-Director invariant guard);
   wrap any other exception as `register failed: <error>`. Capture the new
   member id.
6. **Substitute placeholders** (below) — run `str.format` over the resolved body
   (step 3), substituting `{fleet_id}` / `{member_id}` (the new member id from
   step 5) / `{director_member_id}` (the auto-resolved Director) /
   `{coding_agent}`. An unknown-placeholder or
   malformed-brace error is a usage error (exit 2); on it,
   **deregister-with-warning, then re-raise the original error unwrapped** —
   preserving both the exact message and its exit code.
7. **Build the spawn argv** from the backend (the rendered prompt from step 6,
   display name, model, effort).
8. **Split the pane** — split the window to obtain the pane id. The only
   forwarded env var is `CAFLEET_DATABASE_URL` (when set); identity travels in
   the rendered prompt (step 6), not the environment. tmux error →
   rollback-register, reason `tmux split-window failed: <error>`.
9. **Patch the pane id** — record it on the placement. On exception: best-effort
   send `/exit` (tolerating a missing pane), then rollback-register, reason
   `placement update failed: <error>`. If the placement row vanished: same
   best-effort `/exit`, then rollback-register, reason `placement row vanished
   before pane-id patch`.
10. **Emit** — attach the placement view; emit JSON or the spawned-member text
    formatter (`format_member`, §6.4, honoring `--full`).

The ladder contract: any post-register failure deregisters the member so no
orphan row survives; the best-effort cleanup never masks the original error.

#### `member delete`

The pane-teardown + registry-soft-delete op. Options: `--member-id`
(integer, required — the **target**). The tmux precondition fires only on the
pane-teardown path (live pane id) — a placementless or pending-placement
delete is a pure registry operation and succeeds outside tmux.

1. **Root-Director guard, before any pane mutation** — fetch the fleet; if the
   target is the fleet's Director → application error (exit 1) `cannot deregister
   the root Director; use 'cafleet fleet delete' instead` (the same string and
   exit code the broker's `deregister_member` guard raises, §6.2).
2. Load the authorized member **tolerating a missing placement**; re-fetch
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

Registry read — no tmux requirement and no requester gate. Options:
`--member-id` (integer, required — the **target**; any active in-fleet
registry entry, placed or placementless, the root Director
included), `--full` (documented; affects **text mode only**). Load the target
tolerating a missing placement: cross-fleet / unknown / inactive → application
error `Member <member_id> not found`. JSON emits the broker `get_member` dict
unchanged (§6.2) regardless of `--full`; text renders via `format_member_detail`
(§6.4) — compact `<member_id> <name> <status>` by default, the labeled verbose
block (`kind`, `skills`, placement sub-block) with `--full`.

#### `member list`

No options beyond the required `--fleet-id` and the shared `--json` flag; no
identity flag. Lists every **active registry entry** of the fleet via
`list_members` (§6.2) — the root Director, the monitoring member, ordinary
members, and placementless rows. Empty case `0 members.`; else the header is
`<N> members:` and the table renders one row per member with `member_id`,
`name`, `kind` (the three `get_member` values), `backend` (the placement's
`coding_agent`), `pane_id` (`(pending)` when unset), and the humanized `idle`
columns (§6.4 `format_member_list`); a placementless row renders `-` in the
`backend` and `pane_id` cells. JSON emits
the raw `list_members` rows (`member_id`, `name`, `kind`, `placement` — null
when placementless — `last_sent`, `last_recv`, `last_ack`, `idle`).

#### `member prompt`

Options: `--member-id` (integer, required), `--shell` (boolean flag, default
`false`), **positional** `text` (string, required). A newline/CR → usage error
`text may not contain newlines.`; empty after trim → usage error `text may not
be empty.`; then trim. Ensure tmux, load the member, require a pane (`prompt`).
Dispatch via the multiplexer's `send_prompt` (§6.5): the plain form delivers
the text Esc-safeguarded as a submitted user turn; the `--shell` form delivers
`! <text>` un-escaped via the coding agent's `!` shell shortcut (a tmux error →
application error `send failed: <error>`). The flag performs no content
inspection — plain-form text beginning with `!` is delivered verbatim. JSON:
`{member_id, pane_id, text, shell}`; text: `Sent prompt <quoted-text> to
member <name> (<pane_id>).`, or with `--shell` `Sent shell prompt
<quoted-text> to member <name> (<pane_id>).` (the text rendered with
human-readable quoting/escaping — reproducing the quoted intent is
sufficient).

#### `member ping`

Re-pokes a member's inbox. Options: `--member-id` (integer, required — the
**target**), `--quiet` (boolean, default `false` — success output is the
bare member id). Ensure tmux and load the target (a missing placement row is
still the hard error of the shared loader). A **pending placement** (a
placement row with no pane id) takes the **skip path**: no keystroke is sent
and the command exits 0 — text `Member <name> has no pane yet (pending
placement) — ping skipped; it will poll its inbox on spawn.`, JSON
`{"member_id": <id>, "pane_id": null, "skipped": true}`, `--quiet` the bare
member id. With a pane, inject the inbox-poll keystroke via the multiplexer's
`send_poll_trigger`, which is **best-effort** (§6.5) — it returns a boolean and
never raises. A returned `false` (non-delivery) → application error `send
failed: tmux send-keys did not deliver the poll-trigger keystroke to pane
<pane_id>.`. Because `send_poll_trigger` swallows its own `TmuxError` and
returns `false`, the only reachable failure surface is the non-delivery message
above. JSON: `{member_id, pane_id, skipped}` — the `skipped` key is present on
**both** success paths (`false` on a dispatched ping); text: `Pinged member
<name> (<pane_id>) — poll keystroke dispatched.`.

#### `monitor` group

A shared `_require_live_fleet` guard fetches the fleet; missing or soft-deleted
→ application error `fleet <fleet_id> not found`.

- **start** — `--tick` (integer ≥1, default 5, shown in help). Requires a live
  fleet, then tmux. No monitoring member → a warn-but-run line to **stderr**:
  `Warning: fleet <fleet_id> has no monitoring member; the monitor heartbeat
  will wake no member. Spawn one first with 'cafleet member create --role
  monitor'.`. Then run the monitor loop in-process (blocking). Immediately
  after the successful runtime claim, before the first tick, the loop prints
  the startup line backing the `ready: monitor live` handshake:
  `monitor loop started (fleet <fleet_id>, tick <tick>s, pid <pid>)`.
- **capture** — `--member-id` (integer, required), `--lines` (integer, default
  **20**, shown in help), `--ansi` / `--no-ansi` (boolean pair, default
  `false`), plus the shared `--json`. Ensure tmux, load the member (the shared
  `member`-group loader), require a pane (`capture`). Capture the last N lines
  (a tmux error → application error `capture failed: <error>`). When `--ansi`
  is not set, strip ANSI. JSON begins `{member_id, pane_id, lines, content,
  ...}`; text emits the content with no trailing newline, **preserving ANSI
  even on a non-TTY sink** when `--ansi` is set. JSON adds `captured_at`,
  stamped from local UTC at the capture read boundary, and
  `content_sha256 = sha256(content.encode("utf-8"))`, in key order after
  `content`. The hash is mode-exact: no-ANSI hashes the stripped,
  carriage-return-defragmented emitted string; ANSI hashes the preserving
  emitted string. Text output stays byte-identical. Capture content is not
  stored.

#### `server`

Options: `--host` (string, default `settings.broker_host` = `127.0.0.1`, shown
in help), `--port` (integer, default `settings.broker_port` = `8000`, shown in
help). Serves the WebUI app on host/port; port-in-use and all other server
errors propagate unwrapped.

#### `setup`

`setup` is a plain **command** (no subcommands) — the single onboarding
and schema-management entry point. Command help: `Migrate the database schema
and install the coding-agent assets (skills and presets).` It takes no
positional arguments — `cafleet setup <word>` fails with clap's native
unexpected-argument error — and does not accept
`--fleet-id`.

| Flag | Required | Notes |
|---|---|---|
| `--skip AGENT` | no | Repeatable. A choice over `claude` / `codex` / `opencode`; an unknown value fails with clap's native invalid-value error (exit 2). Duplicates are deduplicated. Help: `Skip the named agent's assets install (repeatable).` |

Reads the CLI's own version and runs two independent halves, **in order** (db
first, then assets):

- **DB half** — initialize or migrate the registry via the db-migration driver
  (§8): force a sync SQLite URL, create the DB file's parent directory, and
  apply the bundled migrations up to the head revision (idempotent). On an
  application error, print `db half failed: <message>` and record the
  failure. If the db half failed, the assets half fails its schema pre-flight and
  both halves are reported failed.
- **Assets half** — targets are the fixed list `claude`, `codex`, `opencode`
  (in that order) minus the `--skip`ped agents (no home auto-detection: each
  target's agent directories are created as needed). Installs, from the data
  embedded in the binary at build time (§7.6) with **no network access**, the
  skills plus each target's bundled preset (where one
  exists), upserting one `asset_installs` row per target after that target's
  install succeeds. On an application error, print `assets half failed:
  <message>` and record the failure. When all three agents are skipped, the
  half is skipped entirely: the command echoes `assets half skipped (all
  agents skipped)` and the half counts as **not-run** — it cannot contribute a
  failure.
- If anything that ran failed → application error `<failed halves joined by '
  and '> half failed` (exit 1; db listed first, matching run order — e.g. `db
  and assets half failed`).

##### Schema-only invocation

`cafleet setup --skip claude --skip codex --skip opencode` is the documented
contributor/CI path: it is deterministic (independent of which agent homes
exist) and runs the db half only (the assets half is skipped and cannot
contribute a failure). It never records `asset_installs` rows.

##### Shared helpers (the assets half)

**resolve-targets** (the fixed list `claude`, `codex`, `opencode` minus the
`--skip`ped agents, in that fixed order; all three skipped → the assets half
is skipped entirely, per above);
**install-skills** (per target, create the target's skills dir as needed, then
copy each embedded skill dir — exactly the three skill dirs `cafleet`,
`cafleet-design-doc`, `cafleet-research` — into it, removing any existing copy
first; a filesystem error → `failed to install skills into <skills_dir>: <error>`;
success prints
`<agent>: installed cafleet, cafleet-design-doc, cafleet-research (v<version>)
-> <skills dir>`);
**install-preset** (per target with a preset — codex: embedded source
`presets/codex/cafleet.rules` → `~/.codex/rules/cafleet.rules`; opencode:
`presets/opencode/cafleet.md` → `~/.opencode/agents/cafleet.md`; claude has
none: create the target's parent directory chain recursively, remove any
existing target in this explicit check order — a symlink → unlink; else a
directory → recursive delete; else if it exists → unlink (the symlink check
comes first because the directory check follows symlinks and the recursive
delete refuses them) — then
copy the embedded source in; a filesystem error → `failed to install preset into
<target>: <error>`; success prints `<agent>: installed preset (v<version>) ->
<target>`). A target's `asset_installs` row is upserted only after both its
skills and its preset (where one exists) install successfully. Known skills
dirs: `claude` → `~/.claude/skills`, `codex` → `~/.codex/skills`, `opencode` →
`~/.config/opencode/skills`.

Assets-half pre-flight: the `asset_installs` table must exist, else the half
fails with `the database schema is missing or outdated; run 'cafleet setup'
first`. (The db half always runs first within the same command, so this fires
only after a db-half failure or an externally broken schema.)

An install failure aborts the loop; rows recorded before the failure remain.

#### Stale-assets guard

Every fleet-scoped command group — `fleet`, `member`, `message`, and `monitor`
— validates the recorded assets installs at the top of its group callback,
before any subcommand body runs:

1. If the DB file, the `asset_installs` table, or all rows are missing, exit 1
   with:
   ```
   Error: no assets install is recorded; run 'cafleet setup' first
   ```

2. If any recorded `cafleet_version` differs from the runtime CLI version
   (simple string inequality — a downgrade also triggers), exit 1 with the
   stale agents listed in ascending `coding_agent` order:
   ```
   Error: stale assets detected (<agent>=<recorded>[, ...]; CLI <runtime>); run 'cafleet setup' to reinstall
   ```

3. Otherwise proceed silently.

Agents with no recorded row (an agent that was never installed) are not
checked. Exempt surfaces: `setup` (must remain runnable to repair), `doctor`
(reports instead of blocking), and `server` (human-facing WebUI, not
fleet-scoped).

**Help interaction.** `--help` renders at parse time and exits before any
command body runs, so neither group-level help (`cafleet fleet --help`) nor
subcommand help (`cafleet fleet create --help`) triggers the guard — both
always print help, even under a missing or stale install.

#### Spawn-prompt resolution (used by `member create`)

The spawn prompt is supplied through the shared `--text` / `--text-file` body
input (§6.3 [text-body input](#text-body-input)): exactly one is required, `-`
reads stdin, a relative `--text-file` path resolves against CWD, decoded UTF-8
with no newline translation. There is **no** built-in default template and **no**
positional prompt argument — a bare `member create` with neither flag is the
shared usage error `Provide exactly one of --text or --text-file.` (exit 2).

**Placeholder substitution.** After the body is resolved, `member create` runs
`str.format` over it, substituting `{fleet_id}`, `{member_id}` (the spawned
member's own id), `{director_member_id}`, and `{coding_agent}` (the resolved
backend). A custom prompt keeps a literal brace by doubling it (`{{` / `}}`).
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

The text-vs-JSON selection is the CLI's: `--full` and `--json` are **documented**
flags (Q5 hidden-flag cleanup). Every JSON-capable subcommand takes the one
shared per-subcommand `--json` flag (§6.3): the `message` group branches on it
inside the shared handler sequence, while the `member`, `monitor`, `fleet`, and
`doctor`
handlers branch on it per-handler with their own emit sites (§7.3). The single
absent glyph below and the compact-JSON rules apply to every path.

#### Two-layer architecture

- **Render layer** — projects raw broker results into slim "wire" shapes,
  truncates oversized text by codepoint count, walks/transforms nested
  structures without mutating originals, serializes to compact JSON, and strips
  ANSI from captured pane buffers.
- **Formatter layer** — consumes those shapes (or the raw dicts) and produces
  exact multi-line, column-aligned, ANSI-free terminal strings.

The split is load-bearing: formatters call render functions internally (e.g.
`format_message` calls `render_message` for compact mode), but render functions never
call formatters.

**Render functions:** `strip_ansi(text)`; `format_json(data)`;
`truncate_text(value, full, limit)`; `truncate_message_text(result, full)`
(in-place); `render_message(message, full)` → `{id, from, ts, text, kind?, origin?}`;
`render_messages_in_result(result, full)` (non-mutating, unwraps `{message: …}`
envelopes and flat message dicts).

**Formatter functions:** `format_message`; `format_indexed_list`
(joins formatted items with one blank line between, `empty_msg` when empty —
not numbered); `format_member_detail`; `format_fleet_create`; `format_member`;
`format_member_list`. Private contract helpers: an ISO→`HH:MM:SS` extractor;
an idle-seconds humanizer.

#### Truncation rules

- Truncation counts and slices **by Unicode codepoint**, never by byte. A value
  longer than the effective limit returns its first `limit` codepoints plus a
  one-codepoint `…` (U+2026) suffix — so the result is `limit + 1` codepoints.
- `truncate_text` passes the value through unchanged when `full` is set, the
  value is null, or its codepoint length is `<= limit`. Null returns null.
- The effective limit is `truncate_text`'s explicit `limit` argument when given,
  else `max_text_len` (default `200`, from config) — the **only** config
  dependency. `truncate_message_text` takes no `limit` and always uses
  `max_text_len`.
- The **member-description limit is a hardcoded literal `60`**, independent of
  `max_text_len`; `format_member_detail` verbose applies it.

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

`truncate_message_text` **mutates its input in place** and returns the same object.
`render_messages_in_result` is **non-mutating** (it
builds new structures, shallow-copying any envelope). Preserve the distinction; in
a language with no aliasing concern, preserve the *observable* result.

#### Field access / optionality

Every field is read with required access unless marked optional; required access
**fails loud** on a missing key by design. The truthiness guards on `text` /
`origin_message_id` mean empty string and `0` are also suppressed, not just null.

- **Message** (`render_message` / `format_message` / `truncate_message_text`): `message_id`
  (req), `from_member_id` (req), `status_timestamp` (req, compact),
  `text` (req key for compact, optional for verbose; guarded by truthiness),
  `type` (req; `"unicast"` suppresses `kind`), `status_state` (req, verbose),
  `to_member_id` (optional, nullable; verbose `to:` line only when **non-null** —
  a `broadcast_summary` row's `NULL` recipient is skipped), `origin_message_id`
  (optional; `origin` key only when **truthy**). Envelope: a message may be wrapped
  `{message: {…}}`; `format_message`
  unwraps when the inner value is a dict; the render walker unwraps when it is a
  dict containing `message_id`.
- **Member detail** (`format_member_detail`): `member_id` (req), `name` (req),
  `description` (req, truncated to 60), `status` (req), `kind` (req, verbose),
  `skills` (req, verbose; a compact JSON array, `-` when empty), `placement`
  (optional; verbose renders the placement sub-block when present,
  `placement:   none` otherwise, with `-` for a null field inside it).
- **Fleet-create** (`format_fleet_create`): `fleet_id` (req), `director` (req
  nested) → `member_id` (req), `name`/`placement` (req, verbose);
  `director.placement` (verbose) → `mux_session`/`mux_window_id`/`mux_pane_id`
  (req); `name` (req key, verbose, empty string
  when falsy); `created_at` (req, verbose).
- **Member-create** (`format_member`): `member_id` (req), `name` (req),
  `placement` (req) → `coding_agent` (req), `mux_pane_id` (req key; `(pending)`
  when falsy in compact), `mux_window_id` (req, verbose).
- **Member-list row**: `member_id`, `name`, `kind`, `placement` (optional,
  null for a placementless row; when present → `{coding_agent, mux_pane_id (→
  "(pending)")}` feed the `backend` / `pane_id` cells, `-` cells when null),
  `last_sent`, `last_recv`, `last_ack` (ISO str | null), `idle` (int seconds |
  null).
- **Roster row (the WebUI `GET /api/members` roster)**: `member_id`, `name`,
  `description`, `status`, `registered_at`, `kind` (the three `get_member`
  values), `placement` (null when placementless); serialized directly by the
  WebUI, not by a formatter. The monitor runtime/member payloads (§6.2) are
  likewise serialized directly by the WebUI (§6.8), not by a formatter.

The `(pending)` fallback for `mux_pane_id` appears in the compact member render
and both list rows, but **not** in the verbose `format_member` block.

The `backend:` display label (in `format_member_detail`'s verbose placement block, the
`format_member` renders, and the roster/list column headers) maps to the
placement's **`coding_agent`** value (`claude`/`codex`/`opencode`) — it names the
coding-agent backend, a distinct axis from the placement's `backend` column
(the multiplexer, `tmux`/`herdr`). The placement projection carries the new
`backend` column, but these formatters do not render it; only `cafleet doctor`
surfaces the resolved multiplexer backend (§6.3).

#### Exact text layouts

`format_message` — **compact** line 1 by concatenation: `[<id> | from:<from> |
<ts>]`, with ` | kind:<kind>` inserted before `]` when a `kind` is present and
` | origin:<origin>` inserted (after kind) when an `origin` is present; if the
rendered `text` is truthy a second line holds the body. **Verbose** — aligned
lines: `  id:    <message_id>`, `  state: <status_state>`, `  from:  <from_member_id>`,
then `  to:    <to_member_id>` **only when `to_member_id` is non-null**, then `  type:
 <type>` **always**, then `  text:  <text>` **only when `text` is truthy**.

`format_member_detail` — **compact**: `<member_id> <name> <status>` (single spaces, no
labels). **Verbose** (description truncated to 60): `  member_id:    <member_id>`,
`  name:        <name>`, `  description: <description>`, `  status:      <status>`,
`  kind:        <kind>`, `  skills:      <skills>` (a compact JSON array, `-`
when empty), then the placement block — `  placement:   none` when no placement
row exists, else `  placement:` followed by the indented
`    backend:` / `    session:` / `    window_id:` /
`    pane_id:` / `    created_at:` lines, each null field rendering `-`.

`format_fleet_create` — **compact**: `<fleet_id> director=<director.member_id>`.
**Verbose** — 6 lines; first two are bare
stringified values with no key prefix; `pane` joins the three placement fields
with `:`:

```
<fleet_id>
<director.member_id>
name:             <name or "">
created_at:       <created_at>
director_name:    <director.name>
pane:             <mux_session>:<mux_window_id>:<mux_pane_id>
```

`format_member` — **compact** (`pane` = `mux_pane_id` or `(pending)`):
`<member_id> <name> backend=<coding_agent> pane=<pane>`. **Verbose** — 6 lines
(verbose `pane_id` is the raw `mux_pane_id`, no `(pending)`):

```
Member registered and spawned.
  member_id: <member_id>
  name:      <name>
  backend:   <coding_agent>
  pane_id:   <mux_pane_id>
  window_id: <mux_window_id>
```

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
single absent glyph*). The conditional fields `kind`, `origin`, and the verbose
`text:` line are gated on truthiness — omitted, never emitted empty. The verbose
`to:` line is instead gated on **non-null** (`to_member_id is not None`): a
broadcast-summary row's NULL recipient omits it; a unicast's real id always
shows.

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

**Error taxonomy.** A shared base `MultiplexerError(Exception)` in `base.py`;
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

An unset `CAFLEET_MULTIPLEXER` (auto-detect) is a legitimate default — absence is
a valid, well-defined state, not a fallback for a missing value; the override is
the deterministic escape hatch.

#### Interface signature note — `split_window`

`Multiplexer.split_window(*, reference: MultiplexerContext, env, command) -> str`
takes the full reference context rather than a bare window id: tmux splits a
*window* and uses `reference.window_id`; herdr splits a *pane* and uses
`reference.pane_id`. The sole call site (`cli/member.py`) already holds the
Director's `MultiplexerContext` and passes it directly.

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
  + Enter via the literal-then-Enter core, **no Esc-first**; tolerates a missing
  pane when `ignore_missing`.
- **`send_poll_trigger(*, target_pane_id, fleet_id, member_id) -> bool`** —
  best-effort. tmux missing → `false`; payload `cafleet message poll --fleet-id
  <fleet_id> --member-id <member_id>`; literal-then-Enter, `timeout=5`s,
  **Esc-first=YES**, any error → `false`. Used only by `member ping`.
- **`send_wake_trigger(*, target_pane_id, due_members, director) -> bool`** —
  best-effort; the **sole** keystroke the monitor loop fires. Each due entry has
  `member_id`, `name`, `is_director`, validated `coding_agent`, and ordered
  deduped `wake_reasons`; the Director descriptor has `member_id` and
  `coding_agent`. Names and agent values pass the single-line sanitizer.
  Render each entry as `<role> <id> (<name>; coding_agent=<agent>)
  [<reasons>]`.

  The tmux/herdr payload is byte-identical and is a **pure trigger** — the due
  list, the Director descriptor, and one pointer sentence naming the
  monitoring member's role protocol; no protocol clauses:

  ```
  [monitor] wake: <N> <member|members> due — <entries>. Director: <id> (coding_agent=<agent>). Follow your monitor role protocol.
  ```

  The wake keystroke is literal-then-Enter with `timeout=5`s and
  **Esc-first=NO**; any error returns false. It contains no backtick,
  command-substitution sequence, or pipe.
- **`send_inline_preview(*, target_pane_id, message_id, sender_id, ts, text) ->
  bool`** — best-effort; the broker's inline-preview path (the broker truncates
  `text` first). tmux missing → `false`; cosmetic CR/LF strip on `text`
  (`\r\n`/`\n`/`\r` each → `⏎` U+23CE, **no** tab/backtick/command-substitution
  sanitization here); two-line payload (single `\n` separator intentionally
  kept):
  ```
  [cafleet msg <message_id> from <sender_id> <ts>]
  <sanitized_text>
  ```
  literal-then-Enter, `timeout=5`s, **Esc-first=YES**, any error → `false`. Under
  `send-keys -l` the `\n` is a soft line break inside one keystroke; the single
  trailing Enter submits the whole 2-line payload as one recipient turn.
- **`send_prompt(*, target_pane_id, text, shell=False)`** — fail-fast. Strip
  surrounding whitespace; empty after strip → `send_prompt: text may not
  be empty`; the **original** text with a newline or CR → `send_prompt:
  text may not contain newlines`. literal-then-Enter with `payload = "! " +
  stripped_text` when `shell` else `stripped_text`, and `esc_first=not shell` —
  the plain form Esc-safeguards the submitted user turn; the shell form omits
  the Esc (honors the coding-agent `!` shortcut).
- **`capture_pane(*, target_pane_id, lines=20) -> str`** — fail-fast. `lines <=
  0` → `capture_pane: lines must be positive, got <lines>`. Run `tmux
  capture-pane -p -t <target_pane_id> -S -<lines>`, split the raw output on
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
  `send_poll_trigger`, `send_wake_trigger`, `send_inline_preview`. Each guards
  "tmux missing → `false`" then wraps the keystroke so any error → `false`. The
  boolean is consumed as the broker's `notification_sent` (unicast) /
  the broadcast `delivered` count and the monitor's `woke`.

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
**NO**, `send_exit` **NO**, `send_prompt` plain form **YES** / shell form
**NO** (`esc_first=not shell`).

#### Subprocess core, timeout, and pane-gone tolerance

The subprocess runner invokes tmux as an argv list (no shell), treats a non-zero
exit as a failure, and returns stdout on success. Failure-message intents:
binary-not-found → `tmux binary not found: <detail>`; timeout → `tmux command
timed out after <timeout>s: <space-joined argv>`; non-zero exit → `tmux command
failed: <space-joined argv>\nstderr: <trimmed stderr>`. A **per-call timeout** of
`5`s is passed only by `list_pane_ids` and the three keystroke helpers; every
other call is unbounded. **Pane-gone tolerance:** the tolerant runner swallows a
tmux error only when **both** `ignore_missing` is true **and** the message text
(case-insensitive) contains `"can't find pane"` or `"no such pane"`; any other
failure re-raises even under `ignore_missing`. Whatever error shape a port uses
MUST keep the message/stderr text inspectable for this substring match.

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
  otherwise an argument containing spaces would be re-split).
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
- **`send_exit(*, target_pane_id, ignore_missing=False)`** — `herdr pane run
  <id> "/exit"`.
- **`send_poll_trigger(...) -> bool`** — best-effort. `herdr pane send-keys <id>
  esc` (the Esc safeguard, with the same short settle delay), then `herdr pane
  run <id> "cafleet message poll --fleet-id <fleet_id> --member-id <member_id>"`.
- **`send_wake_trigger(...) -> bool`** — best-effort. `herdr pane run <id>
  "<payload>"` (no Esc — an Esc would self-interrupt the monitoring member). The
  `<payload>` — its single-line text and its per-member `[<wake_reasons joined by
  ",">]` due-list suffix — is **byte-identical** to the tmux `send_wake_trigger`
  payload above, carrying no backtick, no command-substitution sequence, and no
  pipe.
- **`send_inline_preview(...) -> bool`** — best-effort. `herdr pane send-keys
  <id> esc`, then `herdr pane send-text <id> "<2-line payload>"` (raw, no Enter —
  the embedded newline is literal), then a sleep of `_SUBMIT_DELAY` (`1.0`s),
  then a single `herdr pane send-keys <id> enter`, keeping the tmux contract of
  "one submit for the whole 2-line payload".
- **`send_prompt(*, target_pane_id, text, shell=False)`** — shell form:
  `herdr pane run <id> "! <text>"` (no Esc); plain form: `herdr pane send-keys
  <id> esc`, then `herdr pane run <id> "<text>"` (mirroring
  `send_poll_trigger`'s esc-then-run shape).
- **`capture_pane(*, target_pane_id, lines=20) -> str`** — fail-fast. `lines <=
  0` → `capture_pane: lines must be positive, got <lines>`. Run `herdr pane read
  <id> --source recent-unwrapped --lines <lines>`, then apply the same
  windowing as the tmux backend: split on `"\n"` only, drop the trailing run of
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
the recipient's composer. The `esc_first` safeguard maps to a discrete `herdr
pane send-keys <id> esc` before the payload on exactly the paths that use it
today (`send_poll_trigger`, `send_inline_preview`, and `send_prompt`'s plain
form).

#### `AgentStateAware` capability (herdr only)

A **separate optional** `@runtime_checkable` Protocol, kept off the base
`Multiplexer` interface so tmux need not implement anything new:

- **`agent_status(*, target_pane_id) -> str | None`** — the pane's current native
  agent state (`working`/`blocked`/`done`/`idle`/`unknown`), or `None` when no
  agent is detected. herdr realization: `herdr pane get` / `pane read --source
  detection`.

`HerdrMultiplexer` implements it; `TmuxMultiplexer` does **not** implement
`AgentStateAware` (an `isinstance(mux, AgentStateAware)` guard is therefore
false on the tmux backend). The monitor loop consumes this capability (§6.6).

### 6.6 Monitor heartbeat loop

**Scope:** the in-process supervision scheduler. A coding agent launches
`run_monitor_loop` as a background task; it keeps a fleet's dedicated
*monitoring member* periodically woken so the watcher re-inspects the Director
and ordinary members. Every fixed tick performs one scan and at most one synchronized wake;
`unacked` is annotation-only. The module owns the OS-facing half — the pure due-check,
one scan pass, the foreground driver with signal handling and runtime-row
cleanup, the scan-cadence constant, and the re-export of the four policy
tunables. It performs no DB internals (the broker's) and no multiplexer
internals; it orchestrates calls into both. It resolves its backend via
`resolve_multiplexer()` (§6.5) and, on a backend that implements `AgentStateAware`
(herdr only), augments the interval due-check with a native-status due trigger
(see below); `should_ping` itself stays interval-only.

#### Public surface

- **`should_ping(target, now) -> bool`** — pure due-check for one watched member;
  no DB/multiplexer access.
- **`monitor_tick(fleet_id, now) -> CONTINUE | STOP`** — one scan pass.
- **`run_monitor_loop(fleet_id, tick_seconds)`** — foreground driver: claim slot
  → install signal handlers → `tick → sleep` until signalled → clear slot on
  exit.
- **`CONTINUE` / `STOP`** — tick-result markers distinguishing "keep looping"
  from "self-terminate".
- **`DEFAULT_TICK_SECONDS = 5`** — default scan cadence (seconds).
- Re-exports `DIRECTOR_PING_INTERVAL_SECONDS` (180),
  `MEMBER_PING_INTERVAL_SECONDS` (720), `MONITOR_STALE_FACTOR` (3),
  `MONITOR_STALE_FLOOR_SECONDS` (15) — policy tunables whose single home is the
  broker, re-exported so the loop imports policy from one place.

The stop flag, the sleep helper, the signal handler, and the marker type are
implementation-private; only the three functions, the markers, and the five
constants are public.

#### `should_ping(target, now)`

Pure function of one watched-member scan row (`member_id`, `name`, `is_director`,
`pane_id` optional, `interval_seconds`, `last_ping_at` optional ISO string,
`enabled`, `pending_count`, `oldest_pending_ts` optional ISO string,
`pane_alive`) and a tz-aware UTC `now`. Branch
conditions, in short-circuit order:

1. `enabled` false → false.
2. `pane_id` absent **or** `pane_alive` false → false (unplaced or dead/missing
   pane is always skipped).
3. `last_ping_at` set: `elapsed = (now − parse(last_ping_at))` in float seconds;
   if `elapsed < interval_seconds` → false (not yet due).
4. Otherwise → true. A never-pinged (`last_ping_at` absent) live, enabled member
   is **immediately due** — the elapsed check is skipped entirely.

`is_director` is **not** consulted (retained only for status labeling);
`pending_count` and `oldest_pending_ts` are **not** consulted (due-ness is
interval-driven). The monitoring member never appears as a `target` — it is
the unenrolled watcher.

#### `monitor_tick(fleet_id, now)`

One scan pass, steps in order:

1. **Ownership-checked heartbeat.** Call the broker's heartbeat with `(fleet_id,
   this-pid, now-as-ISO)`. Returns false (zero-row update — this process was
   displaced and another reclaimed the slot) → return `STOP`. This is the
   split-brain loser's exit.
2. **Fleet liveness.** Fetch the fleet; absent **or** `deleted_at` set → return
   `STOP`.
3. **Locate the watcher.** Ask the broker for the fleet's monitoring member (may
   be absent); shape `{member_id, name, pane_id}`.
4. **Fetch pane liveness once.** Resolve the backend via `resolve_multiplexer()`
   (§6.5); a **single** `list_pane_ids` call resolves liveness for every member
   this tick.
5. **Compute the due set.** For each watched `target` (root Director + ordinary
   members; never the monitoring member): set `target.pane_alive = (target.pane_id
   ∈ live_panes)`, then if `should_ping(target, now)` add it to the due set with an
   `interval` wake-reason. Each due target carries `wake_reasons: list[str]`,
   ordered and deduped, drawn from `{"interval", "status:done", "stall-check",
   "unacked"}`:
   - **Lifecycle reconciliation first.** Batch-clear disabled,
     placement-pending, and dead rows before due filtering (clearing
     `last_stall_check_at`).
   - **Stall-check trigger.** When `monitor_stall_interval > 0`, union each
     enabled live row whose persisted `last_stall_check_at` is null or a full
     interval old. Zero disables this branch and direct monitor pings.
   - **Native `done` trigger (`AgentStateAware` backend, herdr only).** Additionally
     point-read each **enabled** watched live member's `agent_status`, and union into
     the due set any member whose status **transitioned into** `done` since the loop's
     last-seen status for it (see *Native agent-state due trigger* below), with a
     `status:done` reason. A transition into `blocked` is recorded but **never** flags
     a wake. On a non-`AgentStateAware` backend (tmux) this branch is skipped
     entirely, so members come due by interval and stall-check only.
   - **Unacked annotation last.** Iterate only rows already in the due set.
     Append `unacked` when `oldest_pending_ts` is at least that member's
     interval old. It never adds a row; representative order is
     `[interval,stall-check,status:done,unacked]`.
6. **Wake the watcher iff due and watcher live.** If the due list is non-empty
   **and** the watcher is present **and** its `pane_id` is in the live set: call
   the multiplexer's wake trigger against the watcher's own pane (the loop's
   **only** keystroke), passing the due members (each with its `wake_reasons`) and
   the fleet's `director_member_id`; it returns a boolean `woke`.
   - If `woke` is true:
     - call the broker's `record_pings` with `now-as-ISO` and **only** the due
       members whose `wake_reasons` include `interval` or `status:done` (a
       stall-check-only member is excluded), advancing their
       `last_ping_at` **only** on a
       successful wake, so a just-flagged member is not due again next tick;
     - persist `last_stall_check_at = now` for every due member tagged
       `stall-check`;
     - emit one stdout heartbeat line per due member, appending that member's joined
       `wake_reasons` as a ` [<reasons joined by ",">]` suffix before ` -> wake
       monitor`, with this **exact** format:
       ```
       {now as canonical ISO-8601, §5.1} due member {member_id} ({name}) [{reasons}] -> wake monitor
       ```
       `name` is emitted **raw** (sanitization applies only to the keystroke
       payload). The only native-status reason that can appear is `status:done`,
       since a `blocked` transition never flags a wake.
   - If `woke` is false: do not record pings, do not persist stall-check
     dispatch timestamps, and do not echo — the due members stay flagged, so
     the next tick retries (no wake-storm, no silent skip).
   - No live watcher to wake: nothing is recorded.
7. Return `CONTINUE`.

**Critical ordering invariant:** `record_pings`, durable stall-dispatch commits,
and the heartbeat echo are all gated behind `woke == true`. Preserve this
gating exactly. Every target and Director descriptor carries validated
`coding_agent`; an absent/unknown value fails closed without cadence commit.

#### Native agent-state due trigger (herdr only)

Augments — never replaces — the interval and stall-check triggers. The single
long-running loop process owns an **in-memory** `_last_member_status: dict[member_id,
last_status]` that persists across ticks (no DB column). `_WAKE_ON_STATUS =
("done",)` is the sole wake-on-status set. Each tick, when the resolved backend
passes `isinstance(mux, AgentStateAware)`:

1. Point-read `agent_status(target_pane_id=…)` for each **enabled** watched member
   whose pane is live (a monitor-disabled member is skipped, matching `should_ping`).
2. A **transition into `done`** — the new status is `done` and differs from the
   loop's last-seen status for that member — flags the member due with a `status:done`
   wake-reason. Comparing against the last-seen status means a single `done` episode
   wakes the watcher **only once**.
3. A **transition into `blocked`** is **recorded but never flags a wake.** `blocked`
   means the member is awaiting a user answer; waking about it would only have the
   watcher classify the pane `awaiting_user` and take no action (§ classification
   rubric), at pure token cost plus a risk it misjudges and pings — the destructive
   path this design closes. The `blocked` read is still committed to `last_status`
   (step 4) so the episode is tracked and a later `blocked → working` recovery is
   detected as a transition; it produces no due flag and no wake-reason.
4. Commit each **non-flagged** member's read (every status other than a fresh
   `done` transition, including every `blocked` read) to the `last_status` map
   **immediately** (so a recovery read like `blocked → working` is always recorded
   and a later `done` is still detected as a transition). Return only the
   **`done`-flagged** members' reads to the caller, which commits them to
   `last_status` **only after a successful wake** (`woke == True`, alongside
   `record_pings`). On a failed/no-wake tick a flagged member's status is **not**
   committed, so its `done` episode stays un-consumed and re-flags next tick (no
   silent skip) — mirroring the interval branch's `record_pings` gating.

On the tmux backend the `isinstance` guard is false, so this branch never runs
and members come due by interval and stall-check only. `should_ping` and the
broker's due computation are untouched — they keep computing interval-due-ness
only, with no knowledge of native status.

#### Stall-detection cadence

Independent of the interval trigger and driven by `settings.monitor_stall_interval`
(`CAFLEET_MONITOR_STALL_INTERVAL`, default `240`; `0` disables). Dispatch
cadence is durable in each row's `last_stall_check_at`. A null timestamp is due
immediately; otherwise the full interval must elapse. Successful synchronized
wake persists `now`; failure persists nothing. Immediate loop restart therefore
honors the remaining interval. A stall-check-only member never advances
`last_ping_at`. Quiet-baseline confirmation across stall-check wakes is the
monitoring member's own in-context judgment (§6.2), not loop state.

#### Unacked-delivery annotation

No independent unacked trigger, re-fire cadence, or process map exists. After
interval, stall-check, and native done have built the synchronized due set, the
loop visits only those rows and appends `unacked` last when
`oldest_pending_ts` is non-null and at least `interval_seconds` old. A younger
delivery is omitted; ACK removes the hint on later normal wakes. A stale
delivery without another trigger yields no row and no wake.

#### `run_monitor_loop(fleet_id, tick_seconds)`

Foreground driver. The fleet's monitor-runtime row is the **only** coordination
artifact (no PID file); identity throughout is the OS process id.

1. Reset the shared stop flag to false and clear only the native-status
   transition cache; durable dispatch state (`last_stall_check_at`) remains in
   SQLite. Capture `pid = this-pid`.
2. **Claim the slot** via the broker's atomic claim `(fleet_id, pid,
   tick_seconds, now-as-ISO)`. On refusal (returns false) → application error
   (exit 1) `monitor already running for fleet {fleet_id}`. There is no silent
   fallback. On success, print the startup line
   `monitor loop started (fleet <fleet_id>, tick <tick>s, pid <pid>)` to
   stdout before the first tick — the line the monitoring member confirms
   before sending `ready: monitor live`.
3. **Install signal handlers** for SIGTERM and SIGINT; each flips the shared stop
   flag to true (the handler is minimal — just a flag flip).
4. **Loop** while the stop flag is false: if `monitor_tick(fleet_id, now)` (each
   pass stamps `now` fresh as tz-aware UTC) returns `STOP` → break; else call
   `interruptible_sleep(tick_seconds)`.
5. **Cleanup (always, in a finally block):** the broker's ownership-checked clear
   `(fleet_id, pid)` — nulls the slot's process fields only if this pid still
   owns the slot, so a displaced loser's clear is a no-op.

**Stop paths:** (a) a signal sets the stop flag → loop exits → finally clears;
(b) `monitor_tick` returns `STOP` → break → finally clears; (c) a hard kill runs
no cleanup — the row's heartbeat goes stale and the broker's later liveness check
reports it dead.

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
the preset file exists at `~/.opencode/agents/cafleet.md` (see § opencode
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
`~/.codex/rules/cafleet.rules` (expanding `~`) by the assets half of `setup`
(§6.3), overwriting any existing target. The file is not a spawn precondition —
codex's `ensure_available` is PATH-check-only — and codex loads every `*.rules`
file under `~/.codex/rules/`, applying the strictest matching decision, so
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
`~/.opencode/agents/cafleet.md` (expanding `~`) by the assets half of `setup`
(§6.3), overwriting any existing target. **Two opencode base directories serve
two distinct purposes** and are not interchangeable: the agent preset lives
under `~/.opencode/`, which is opencode's mandated `--agent cafleet` discovery
path; `setup`'s skills install (§6.3) targets `~/.config/opencode/`, cafleet's
own skills-install target for the opencode agent. Both paths are correct for
their purpose — keep each as written.

The preset is a spawn precondition (the spawn argv references `--agent
cafleet`): opencode's `ensure_available` verifies the file exists at the
install target and raises `opencode agent preset not found at {preset}; run
'cafleet setup' first` when it does not.

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
      "mise //:uv-sync": "allow",
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
  'cafleet setup' first`

### 6.8 WebUI + Config

**Scope (two concerns):** (a) the HTTP app factory `create_app`, the `/api/*`
router, the `X-Fleet-Id` header dependency, the SPA-fallback static server, and
the `cafleet server` launcher; (b) the global `Settings` singleton from the
`CAFLEET_*` env block — consumed CLI-wide, not webui-local. The reference builds
a specific HTTP framework's app; the contract below is stack-neutral. The config
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
`status_state` → `status`, `text` → `body`. The monitor-config projection renames
nothing but **drops** `member_id` and `last_stall_check_at`, exposing
`{interval_seconds, last_ping_at, enabled}`. `GET /api/fleets` returns a **bare JSON
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
  three-value `kind` (§5.4) and a `monitor` field set to the projected monitor
  config when an enrolled config exists, else `null`.
  Response `{"members": [ <member dict> + "monitor": <MonitorConfig>|null, … ]}`.
  Projected `MonitorConfig`: `{interval_seconds, last_ping_at, enabled}`
  (`member_id` dropped).
- **`GET /api/monitor`** — fleet-scoped. Returns the flat runtime keys
  `{running, pid, tick_seconds, last_tick_at, last_tick_age_seconds,
  started_at}` plus a top-level `members` key. Read the runtime row and
  the live-check (current UTC). If absent **or** not live: `running=false`,
  `pid=null`, `tick_seconds` = the row's value when a row exists else `null`,
  `last_tick_at`/`last_tick_age_seconds`/`started_at` all `null` — **a stale row
  never leaks a lingering pid or start time**. When live: `running=true` with the
  live `pid`, `tick_seconds`, `last_tick_at`, `started_at`, and a computed
  `last_tick_age_seconds` (null when `last_tick_at` is null; else whole-seconds
  now − parsed `last_tick_at`, **integer-truncated**). `members` carries the
  shared `monitor_members_payload` rows (§6.2, including `pending_count`,
  `oldest_pending_ts`, `oldest_pending_age_seconds`), computed with the same
  `now` as the runtime fields; the flat runtime keys are unchanged by this
  addition.
- **`GET /api/members/{member_id}/monitor`** — fleet-scoped. Absent config → `404`,
  detail `Member not enrolled`; else the projected `MonitorConfig` (single
  object).
- **`PATCH /api/members/{member_id}/monitor`** — fleet-scoped. Body
  `{interval_seconds?: int, enabled?: bool}` (both optional). A present
  `interval_seconds` must be **≥ 1**; `< 1` → `422`, `{"detail": <string>}`. A
  `null`/`null` patch is a valid no-op. Pre-check the config; absent → `404`,
  detail `Member not enrolled`. Then update; if the member was deregistered
  between the pre-check and the update (TOCTOU), the raised error is caught and
  **collapsed to `404` detail `Member not enrolled`** (not 500). Returns the
  projected updated config.
- **`GET /api/members/{member_id}/inbox`** — fleet-scoped. Member not in fleet →
  `404`, detail `Member not found`; else `{"messages": [ <FormattedMessage>, …
  ]}` over the member's inbox.
- **`GET /api/members/{member_id}/sent`** — fleet-scoped. Same as inbox over sent
  messages; same `404` detail `Member not found`.
- **`GET /api/timeline`** — fleet-scoped, no per-member check. `{"messages": […]}`
  over the fleet's messages, hard-capped at the **200** most recent
  (`status_timestamp DESC`).
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
  The SPA always submits `from_member_id = director.member_id` (the fleet's
  root Director); the endpoint itself is sender-agnostic.

**`FormattedMessage`** (one element of any `messages` array): `{message_id,
from_member_id, from_member_name, to_member_id, to_member_name, type, status,
created_at, status_timestamp, origin_message_id, body}`. Names are resolved by a
single bulk lookup over the union of all `from_member_id`/`to_member_id` values,
using **direct keyed access** — a missing id is a hard failure (→ 500), never a
silent fallback. `status` is the renamed `status_state`; `body` the renamed
`text`; `type` the raw row type. Empty input → empty array.

#### `cafleet server` launcher

Runs the WebUI app under the **built-in HTTP server** (in-process — no
external server program). `--host` (default `settings.broker_host`)
and `--port` (default `settings.broker_port`, integer) both read their defaults
from settings at command-definition time and are shown in `--help`. Serves the
app in a single process **with no auto-reload**. Because the defaults
come from settings, `CAFLEET_BROKER_HOST` / `CAFLEET_BROKER_PORT` are honored
indirectly. This is the only entry point to the HTTP server.

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
   `started_at` / `last_tick_at` / `last_tick_age_seconds` are all `null` even if
   a stale row exists; only `tick_seconds` survives from a stale row.
4. **Field renames are wire contract** — `status_state → status`, `text → body`,
   `member_id` / `last_stall_check_at` dropped from the monitor projection.
5. **`list_fleets` returns a bare array**; every other list wraps in
   `{"members"|"messages": [...]}`.
6. **PATCH TOCTOU collapses to 404**, not 500.

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
| `monitor_stall_interval` | `CAFLEET_MONITOR_STALL_INTERVAL` | non-negative integer | `240` |

- **`multiplexer`** is the explicit backend override consumed by
  `resolve_multiplexer()` (§6.5). `None` (unset) means auto-detect from
  `HERDR_ENV` / `TMUX` — a legitimate default, not a fallback for a missing value.
  A set value must name a registry key (`tmux`/`herdr`) or resolution raises.
- **`monitor_stall_interval`** is the per-member stall-check cadence (seconds)
  driven by the monitor loop, independent of the `monitor_config.interval_seconds`
  ping intervals. A watched member is stall-check due every `monitor_stall_interval`
  seconds; `0` disables stall-check wakes entirely (no `stall-check` wake-reason
  tag is ever emitted, and therefore no monitor-direct ping fires). Dispatch
  cadence is persisted in `monitor_config.last_stall_check_at`. `unacked`
  remains only an annotation on another normal trigger (§6.6).
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
| application error | **1** | application/runtime errors: runtime conflicts (one-monitor rule, not-enrolled, not-found-on-delete), the root-Director-deregistration guard, the spawn rollback ladder, and the missing-`--fleet-id` callback error | an app-class error; prints `Error: <msg>`. |
| value-error / permission-error (broker/messaging/queries) | translated by caller | callable from CLI **and** WebUI; CLI wraps to exit 1, WebUI maps to HTTP status | distinct error variants; permission-error gates authorization (recipient-acks). |
| HTTP error | — | serialized `{"detail": <string>}` | HTTP error responses with the same status + body. |

The root-Director-deregistration guard raises a single **application error
(exit 1)** on both the broker side and the `member delete` CLI side.

**Fail-fast points (never silently fall back):**

- `--fleet-id` is a required option enforced by its shared callback (missing →
  exit 1, §6.3); it has **no environment default** and **must not** default to
  an arbitrary fleet.
- The `message` fleet-gate runs **before** the handler body.
- `doctor` reads the resolved backend's presence env var (`TMUX` / `HERDR_ENV`)
  via `os.environ.get(presence_var, "")`; an empty value is legitimate under an
  explicit `CAFLEET_MULTIPLEXER` override, so the fail-fast lives upstream in
  `resolve_multiplexer()` + `ensure_available()`, not in this read.
- `derive_member_kind` collapses a malformed card kind to the ordinary kind (a
  deliberate non-match).
- `register_member` monitoring-member-without-placement raises.
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
text-vs-JSON (the shared per-subcommand `--json` flag, §6.3) and full-vs-compact
(documented `--full`); the WebUI bypasses `truncate_*`
(raw broker results) but its JSON serialization still preserves key order and raw
UTF-8 (no ASCII escaping).

### 7.4 Logging & stdout discipline

- The monitor loop emits per-due-member heartbeat lines to **stdout**
  (`{iso} due member {id} ({name}) [{reasons}] -> wake monitor`), `name` raw
  (unsanitized), the `[{reasons}]` suffix listing that member's joined wake reasons.
- `member create` rollback diagnostics and `monitor start`'s "no monitoring
  member" warning go to **stderr**. Preserve the stream choice (stdout vs.
  stderr) — it is part of the observable contract.

### 7.5 Time discipline

Every "now" is timezone-aware UTC; every DB-boundary write serializes to the
canonical ISO-8601 string. See §5.1 — string comparison for ordering, parse only
for age math.

### 7.6 Packaging & distribution

`cafleet` ships as a **single static binary** distributed on GitHub Releases.

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

---

## 8. Database schema

**Schema** = the seven application tables of §5.2 plus the migration ledger
table `refinery_schema_history` (the bookkeeping table recording one row per
applied migration: version, name, applied-on timestamp, checksum). The schema
is created and evolved by a **chain of embedded SQL migrations** — numbered
files `V<N>__<slug>.sql`, contiguous from 1, compiled into the binary;
applying the chain in place preserves existing data (§11). Column types,
defaults, FK rules, AUTOINCREMENT, and the create-order quirk are in §6.1.

**Indexes (non-unique), at head:**

- `idx_members_fleet_status` on `members(fleet_id, status)`
- `idx_messages_owner_member_status_ts` on `messages(owner_member_id, status_timestamp)`
- `idx_messages_from_member_status_ts` on `messages(from_member_id, status_timestamp)`

**The migration chain.** One migration at cutover: the baseline
`V1__baseline.sql`, which creates the full §5.2 head schema in one step, in
this order:

1. `members` (+ `idx_members_fleet_status`) — created **first** because every
   other FK-bearing table references it; `members.fleet_id` forward-references
   the still-uncreated `fleets` (§6.1). AUTOINCREMENT.
2. `fleets` — AUTOINCREMENT.
3. `asset_installs` — TEXT PK `coding_agent`, no AUTOINCREMENT, no FK
   constraint. Columns: `coding_agent` TEXT PK, `cafleet_version` TEXT NOT
   NULL, `installed_at` TEXT NOT NULL. Upsert semantics; rows written by the
   assets half of `setup` — one row per target agent (the fixed list `claude`,
   `codex`, `opencode` minus any `--skip`ped agent) — after that target's
   skills and preset (where one exists) install successfully — the row attests
   skills + preset; never written by the db half. Feeds the stale-assets guard
   and `doctor`.
4. `member_placements` — PK=FK `member_id`, not AUTOINCREMENT; `backend` DDL
   default `"tmux"`; `coding_agent` NOT NULL with no DDL default.
5. `monitor_config` — PK=FK `member_id` ON DELETE CASCADE, `interval_seconds`
   default 60, `enabled` default 1; not AUTOINCREMENT.
6. `monitor_runtime` — PK=FK `fleet_id` ON DELETE RESTRICT, `tick_seconds`
   default 5; not AUTOINCREMENT.
7. `messages` (+ `idx_messages_owner_member_status_ts`, `idx_messages_from_member_status_ts`)
   — AUTOINCREMENT.

There are no CHECK constraints. Future schema changes are hand-written
numbered files `V<N>__<slug>.sql` appended to the chain; the chain stays
contiguous from 1 with exactly one baseline.

A fresh DB starts with **no rows in any application table** (only
`refinery_schema_history` holds its per-migration rows); monitor enrollment is written
at runtime (the Director at 180s by `create_fleet`, pane-bound members at 720s
by `register_member`, §6.2), never seeded by the schema. `asset_installs` rows
are written at install time, not by the schema.

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
`Already at head (<N>); nothing to do.` and stop; (6) otherwise apply the
pending migrations to head and print `Created <db_file> and applied migrations
to head (<N>).` when no version was recorded before the run (a fresh or
table-less DB), else `Upgraded from <M> to <N>.`. `<M>` / `<N>` are the
integer migration versions (the head is `1` at cutover). The driver's
connection is closed when the command finishes (success or failure).

---

## 9. Testing strategy

- **Unit:**
  - *Broker* against an **in-memory SQLite** (`:memory:`) with the same pragmas
    (`foreign_keys=ON`); assert FK cascade/restrict, the status lifecycle, the
    one-monitor and nested-team guards, and the error strings/types.
  - *Output* — golden tests: every `format_*`/`render_*` against fixed inputs,
    asserting the layout (column alignment, the single ASCII `-` absent glyph,
    codepoint truncation with `…`, compact-JSON key order).
  - *Multiplexer* — inject a **fake command runner** (no real tmux) and assert
    exact argv lists, the Esc-first/`-l`/Enter ordering, the two sleeps, the
    sanitizer substitutions, and best-effort-vs-raising contracts.
  - *Coding-agent* — assert each `build_spawn_argv` argv, the opencode model
    validation, and each backend's `ensure_available` preconditions (PATH
    check; opencode's preset-existence check) against a temp HOME.
  - *Monitor* — `should_ping` is pure (table-test interval/enabled/pane states);
    `monitor_tick` against a fake broker+multiplexer asserting the `woke`-gated
    `record_pings` and the `STOP` paths.
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

The full command surface — **17 commands across 4 groups + 3 top-level commands**.
Each must be reproduced with identical option names, types, defaults,
required-ness, documented-vs-hidden status, output shapes, and exit codes. Every
interaction flag is now **documented** (there are no hidden flags). Per-command
option semantics are in §6.3.

**Global:** `--version` (`cafleet <version>`, exit 0, bypasses `--fleet-id`).
The shared trailing `--json` flag (§6.3) is listed per row below.

**Top-level:**

- [ ] `cafleet setup` (`--skip AGENT` repeatable choice; no positional arguments; runs the db half then the assets half for the fixed target list claude/codex/opencode minus the skipped agents)
- [ ] `cafleet doctor` (`--json`; emits tmux block + assets-install report)
- [ ] `cafleet server` (`--host`=settings.broker_host, `--port`=settings.broker_port)

**`fleet`:**

- [ ] `cafleet fleet create` (`--name`, `--coding-agent` required, `--json`, `--full`)
- [ ] `cafleet fleet list` (`--json`)
- [ ] `cafleet fleet show` (`--fleet-id`, `--json`)
- [ ] `cafleet fleet delete` (`--fleet-id`)

**`member`:**

- [ ] `cafleet member create` (no identity flag — Director auto-resolved; `--name`, `--description`, `--coding-agent`, `--model`, `--effort`, `--role`=member, `--text` / `--text-file` xor-required, `--full`, `--json`)
- [ ] `cafleet member delete` (`--member-id` target, `--json`; pane path kills immediately and always exits 0; placementless target → registry soft-delete, exit 0)
- [ ] `cafleet member show` (`--member-id` target, `--full`, `--json`)
- [ ] `cafleet member list` (`--json`)
- [ ] `cafleet member prompt` (`--member-id`, `--shell`, positional `text`, `--json`)
- [ ] `cafleet member ping` (`--member-id`, `--quiet`, `--json`; pending placement skips the keystroke and exits 0, `skipped` key on both JSON paths)

**`message`:**

- [ ] `cafleet message send` (`--from-member-id` sender, `--to-member-id` recipient, `--text` / `--text-file` xor-required, `--full`, `--json`)
- [ ] `cafleet message broadcast` (`--from-member-id`, `--text` / `--text-file` xor-required, `--full`, `--json`)
- [ ] `cafleet message poll` (`--member-id`, `--full`, `--json`)
- [ ] `cafleet message ack` (`--member-id`, `--message-id`, `--full`, `--json`)
- [ ] `cafleet message show` (`--member-id`, `--message-id`, `--full`, `--json`)

**`monitor`:**

- [ ] `cafleet monitor start` (`--tick`≥1=5; prints the startup line after a successful runtime claim)
- [ ] `cafleet monitor capture` (`--member-id`, `--lines`=**20**, `--ansi`/`--no-ansi`, `--json`)

Every `member *`, `message *`, and `monitor *` command, plus `fleet
show` and `fleet delete`, takes the **required `--fleet-id` option** (integer);
a missing `--fleet-id` is the shared callback's application error (exit 1,
§6.3). It is omitted from the per-command rows above to avoid repetition.
`setup`, `doctor`, `server`, `fleet create`, and `fleet list`
do **not** take `--fleet-id`.

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

The decisions that shape this surface (full rationale in the design doc):

- **`member` is the single member-lifecycle surface.** `member` owns member registration, teardown, introspection (`show`, `list`), and keystroke interaction (`create`/`delete`/`show`/`list`/`capture`/`prompt`/`ping`). There is no separate `agent` group.
- **`--fleet-id` is a required option with no environment default** (§6.3); a
  missing value is the shared callback's exit-1 error.
- **One error/exit model** (§7.2): usage → exit 2, application/runtime → exit 1.
- **Migration-managed schema** (§8): an embedded chain of numbered SQL
  migrations with the applied versions recorded in `refinery_schema_history`;
  no cross-implementation DB interoperability. Re-running `cafleet setup` (the
  db half runs first) on a database created by this chain applies any pending
  migrations in place and preserves all existing rows, message history
  included; it refuses to auto-downgrade an ahead-of-head database and
  refuses an unversioned database with existing tables. Upgrade path: after
  installing a new release binary, the first fleet-scoped command errors with
  the stale-assets message and instructs the operator to run `cafleet setup`
  to reinstall.
- **Stale-assets guard** (§6.3): every fleet-scoped group callback validates
  recorded `asset_installs` versions against the runtime CLI version before any
  subcommand body runs; missing/stale → hard error (exit 1); exempt: `setup`,
  `doctor`, `server`.
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

- **CLI (§6.3):** the `--coding-agent`/`--role`/`--skip` choice sets may be
  hardcoded to `claude`/`codex`/`opencode` or data-driven off the registry —
  an implementation choice.
- **Multiplexer (§6.5):** `env` argument ordering in `split_window` is not
  behaviorally significant (tmux treats `-e` flags as a set).
- **Coding agents (§6.7):** the backend registry may be a name→backend map or a
  backend enum — an implementation choice.

### Cross-module consistency notes

- **Timestamps** unified in §5.1 (string storage + comparison; parse for math).
- **Member kind** unified in §5.4 (three distinct representations, not one enum).
- **`enabled`** stored INTEGER 0/1, exposed as boolean at the broker boundary
  (§6.1/§6.2/§6.6/§6.8).
- **Policy tunables** (180/720/3/15) have a single home in the broker module,
  re-exported by the monitor module.
- **`settings` singleton** is config-module-owned and reachable from every
  module, not webui-local.
