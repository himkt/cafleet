# CAFleet — Reimplementation Specification

This is the single, self-contained, authoritative specification for
reimplementing the `cafleet` CLI (message broker + coding-agent registry). It is
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
| Persistence & Schema | data models, connection factory, single-baseline SQLite schema |
| Broker | synchronous data-access layer |
| CLI | the whole `cafleet` command tree |
| Output | text/JSON formatting, truncation, ANSI strip |
| Multiplexer | tmux integration, keystroke injection |
| Monitor | heartbeat supervision loop |
| Coding agents | claude/codex/opencode backends |
| WebUI + Config | HTTP API + `CAFLEET_*` settings |

Where this document states both a high-level invariant and a detailed rule, the
detailed rule governs; they are written to agree.

---

## 1. Overview & goals

CAFleet is a message broker and agent registry for coding agents. A single
SQLite database holds fleets, agents, their tmux placements, messaging tasks,
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
  and its default — including `CAFLEET_FLEET_ID` (§6.3/§7.1) and the three
  identity variables injected into spawned panes (§6.3/§7.1).
- **Persistence surface:** the single-baseline SQLite schema — table names,
  columns, types, nullability, defaults, foreign-key rules, indexes, and
  status/enum string values.
- **HTTP surface:** every route, method, request/response shape, header
  contract, and status code of the WebUI API.
- **Observable semantics:** the task status lifecycle, the soft-delete +
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
  is the single baseline defined in §8; databases are not migrated from, nor
  expected to interoperate with, any earlier schema.
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

The behavioral requirements below are independent of this choice: any
implementation that preserves the interface and observable semantics of §1 is
conformant, regardless of its threading or concurrency model.

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
├── db              connection factory, single-baseline schema
├── multiplexer     Multiplexer interface, tmux backend, keystrokes
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

The overlay-coverage check (dev tooling invoked by
`mise //cafleet:lint-overlay`) is **not** part of the runtime modules. It
validates markdown skill files, not runtime behavior, and may be kept as a
standalone lint in any language.

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
  connection factory and the single-baseline schema.
- **output** — depends on `config` (reads `settings.max_text_len`). Pure
  string/structure transforms otherwise.
- **multiplexer** — leaf (process invocation only). Truncation for inline
  previews is done by the *broker* before calling `send_inline_preview`, so the
  multiplexer needs no config.
- **coding-agent** — leaf (process/PATH checks + serialization for the opencode
  preset).
- **broker** — depends on `db` (models + connection), `multiplexer`
  (`MultiplexerContext` argument to `create_fleet`; `send_inline_preview` for
  inline previews), and `config` (`max_text_len` for preview truncation).
- **monitor** — depends on `broker` (monitor DB ops + `get_fleet`) and
  `multiplexer` (`list_pane_ids`, `send_wake_trigger`).
- **webui** — depends on `broker` and `config`. Treats broker results as
  pass-through payloads (renaming two keys, dropping one).
- **cli** — depends on all of the above; it is the orchestration glue that
  wires broker ↔ multiplexer ↔ coding-agent ↔ output, and embeds `webui` for
  `cafleet server`.

**Reconciled overlap points** (specced once, here, then referenced):

1. **Broker ↔ multiplexer inline preview.** The broker's `_try_notify_recipient`
   (§6.2) looks up the recipient's `tmux_pane_id`, skips self-sends and
   paneless recipients, **truncates** `text` to `settings.max_text_len` with a
   `…` suffix, then calls `send_inline_preview` (§6.5) which keystrokes the
   2-line `[cafleet msg …]` payload Esc-first. The multiplexer call is
   **best-effort**: it returns a boolean, never raises, and the broker never
   rolls back the persisted task on a failed keystroke. Truncation happens
   broker-side; the keystroke mechanics are multiplexer-side.
2. **CLI ↔ multiplexer ↔ coding-agent agent-spawn.** `cafleet agent spawn`
   (§6.3) sequences: resolve backend → `validate_model` → `ensure_available`
   → broker `register_agent` (placement with `tmux_pane_id` unset) → resolve the
   prompt via the shared `--text` / `--text-file` reader, then substitute
   `{fleet_id}` / `{agent_id}` / `{director_agent_id}` / `{coding_agent}`
   placeholders (§6.3) → `build_spawn_argv` (§6.7) →
   multiplexer `split_window` (§6.5), injecting `CAFLEET_DATABASE_URL`,
   `CAFLEET_FLEET_ID`, `CAFLEET_AGENT_ID`, and `CAFLEET_DIRECTOR_AGENT_ID` into
   the new pane's environment (§7.1) → broker `update_placement_pane_id`. A
   rollback ladder deregisters the agent on any post-register failure.
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
microsecond precision (the reference's `datetime.now(UTC).isoformat()` form).

- **Storage type:** string (TEXT in SQLite).
- **Production:** the current UTC time formatted as ISO-8601 with the `+00:00`
  offset and microsecond precision, so rows written by any implementation
  interleave and sort identically.
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
| `label` | optional string | |
| `created_at` | string | ISO timestamp |
| `deleted_at` | optional string | soft-delete marker |
| `director_agent_id` | optional integer | FK→agents, ON DELETE RESTRICT; null only mid-bootstrap |

**Agent**

| Field | Type | Notes |
|---|---|---|
| `agent_id` | integer | PK, AUTOINCREMENT |
| `fleet_id` | integer | FK→fleets, ON DELETE RESTRICT |
| `name` | string | |
| `description` | string | |
| `status` | enum string | see 5.3 |
| `registered_at` | string | ISO timestamp |
| `deregistered_at` | optional string | |
| `agent_card_json` | string | A2A card JSON; carries `cafleet.kind` |

**AgentPlacement** (1:1 with Agent; `agent_id` is PK = FK, not autoincrement)

| Field | Type | Notes |
|---|---|---|
| `agent_id` | integer | FK→agents, ON DELETE CASCADE |
| `director_agent_id` | optional integer | FK→agents, ON DELETE RESTRICT; null ⇒ root Director's own placement |
| `tmux_session` | string | |
| `tmux_window_id` | string | |
| `tmux_pane_id` | optional string | unset until `split_window` resolves it |
| `coding_agent` | string | DDL default `"claude"` |
| `created_at` | string | ISO timestamp |

**Task** (the message/task record)

| Field | Type | Notes |
|---|---|---|
| `task_id` | integer | PK, AUTOINCREMENT |
| `context_id` | integer | FK→agents, ON DELETE RESTRICT (recipient/owner context) |
| `from_agent_id` | integer | NO FK |
| `to_agent_id` | optional integer | NO FK; nullable — `NULL` on `broadcast_summary` rows (see 5.5) |
| `type` | enum string | see 5.3 |
| `created_at` | string | ISO timestamp |
| `status_state` | enum string | see 5.3 |
| `status_timestamp` | string | ISO timestamp |
| `origin_task_id` | optional integer | NO FK; broadcast deliveries point at the summary; summary points at itself |
| `text` | string | never truncated at persistence |

**MonitorConfig** (1:1 with Agent; `agent_id` is PK = FK, not autoincrement)

| Field | Type | Notes |
|---|---|---|
| `agent_id` | integer | FK→agents, ON DELETE CASCADE |
| `interval_seconds` | integer | DDL default 60; enrollment writes 180 (Director) / 720 (member) |
| `last_ping_at` | optional string | |
| `enabled` | boolean | stored INTEGER 0/1; exposed as boolean at the broker boundary |

**MonitorRuntime** (1:1 with Fleet; `fleet_id` is PK = FK, not autoincrement)

| Field | Type | Notes |
|---|---|---|
| `fleet_id` | integer | FK→fleets, ON DELETE RESTRICT |
| `pid` | optional integer | |
| `started_at` | optional string | |
| `last_tick_at` | optional string | |
| `tick_seconds` | integer | DDL default 5 |

### 5.3 Enums (literal string contracts)

All values are persisted/compared as exact lowercase strings.

- **AgentStatus:** `"active"` | `"deregistered"`.
- **TaskType:** `"unicast"` | `"broadcast_summary"`. Broadcast fan-out emits ONE
  `broadcast_summary` (owned by the sender) + N `unicast` deliveries. There is no
  distinct "broadcast delivery" type — deliveries reuse `unicast`.
- **TaskStatus:** `"input_required"` | `"completed"` | `"canceled"` (NOTE:
  `"canceled"` — one `l`).
  - `unicast` is born `input_required`; `broadcast_summary` is born `completed`.
  - ack: `input_required` → `completed` (recipient only).
  - cancel: `input_required` → `canceled` (sender only).
  - transitions are legal ONLY from `input_required`.
- **CodingAgentName:** `"claude"` | `"codex"` | `"opencode"`.

### 5.4 Agent kind discriminator (resolved cross-module)

The agent "kind" lives in `agent_card_json` at JSON path `$.cafleet.kind`. Three
distinct representations coexist; **they are not the same enum** and must not be
unified:

- **Raw card values** (§6.1/§6.2/§6.7): `"builtin-administrator"`,
  `"monitoring-member"`, or **absent** (ordinary user/Director). Constants:
  `ADMINISTRATOR_KIND = "builtin-administrator"`,
  `MONITORING_MEMBER_KIND = "monitoring-member"`.
- **Broker projection** (§6.2 `get_agent`/`list_agents`/`list_fleet_agents`):
  collapses to a **two-value** discriminator — the literal `ADMINISTRATOR_KIND`
  when the card marks an administrator, else the literal `"user"`. The richer
  `monitoring-member` kind is **not** surfaced by these projections; it is only
  consulted internally via `is_monitoring_member` / JSON-path guards.
- **Internal predicates:** `is_administrator(card)` / `is_monitoring_member(card)`
  parse the JSON and compare `$.cafleet.kind`; both return false on
  absent/empty/malformed-JSON (a deliberate non-match, not an error mask).

### 5.5 Nullable `to_agent_id` (resolved)

A `broadcast_summary` row has no single recipient, so `to_agent_id` is
**nullable** and `broadcast_message` writes **`NULL`** on the summary row. A
`unicast` task always carries a real recipient id. `get_task` (§6.2) and
`format_task` (§6.4, verbose mode) test `to_agent_id IS NULL` / `is None` to
decide whether to surface the `to:` endpoint, rather than a truthiness check.
Model `to_agent_id` as an **optional/nullable integer**; there is no `0`
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

**Scope:** the six data models (§5.2), the connection factory, and the
single-baseline SQLite schema. This module owns **no** CRUD/query logic and no
HTTP surface; all reads/writes/joins live in the broker (§6.2). The baseline
schema is detailed in §8; this section covers the connection factory, the
per-connection PRAGMAs, and the structural invariants.

#### Connection factory & engine semantics

A single lazily-constructed connection factory (a pool / session factory), built
once on first use and cached as a process-wide singleton. There is no
teardown/dispose path in normal operation; the `setup` schema-create driver
disposes its own short-lived engine when it finishes.

- **Lazy singletons, fail-loud.** The factory builds once and caches. There is
  **no fallback** if construction fails — a bad `database_url` raises at
  connect time and propagates. Do not substitute an in-memory database or any
  other default on error.
- **URL drivername normalization.** Before building the engine, the configured
  database URL has its drivername **force-set to `sqlite`** (e.g. an async
  `sqlite+aiosqlite://…` is rewritten to sync `sqlite://…`). The default URL is
  already `sqlite://…`, so this is a no-op in the common case. The same
  normalization is applied independently by the `setup` schema-create driver. An
  async-driver suffix is stripped to the sync driver, never rejected.
- **Cross-thread sharing.** The connection is shared across threads (the
  reference disables SQLite's same-thread check). This is part of the contract.
- Post-commit object usability (the reference keeps loaded attributes valid
  after commit) is a reference-ORM artifact and is not part of the contract.

#### Per-connection PRAGMAs (mandatory)

A connection-init hook runs on **every** new SQLite connection — including
ad-hoc connections opened by tests or by the `setup` schema-create driver, not
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

- **AUTOINCREMENT on exactly three tables** — `fleets`, `agents`, `tasks` —
  guaranteeing monotonically increasing ids that are never reused. The three 1:1
  child tables (`agent_placements`, `monitor_config`, `monitor_runtime`)
  deliberately **do not** use it: each reuses its parent's id (`agent_id` /
  `fleet_id`) as both PK and FK.
- **Create-order / forward-reference quirk.** `fleets.fleet_id` and
  `agents.fleet_id` form a mutual reference. The baseline schema creates
  `agents` **first**, relying on SQLite tolerating a foreign key to a
  not-yet-created table at table-creation time (`agents.fleet_id` forward-
  references the still-uncreated `fleets`). The create order must be preserved;
  it is valid only because FK enforcement is per-connection and is engaged later
  by `foreign_keys=ON`, not at table-creation time. No FK declares an `ON
  UPDATE` clause (all default to NO ACTION). §8 fixes the single ordered baseline
  `CREATE` that establishes this.
- **DDL-level (server-side) defaults**, applied by SQLite when the column is
  omitted from an INSERT (distinct from values the application writes
  explicitly): `agent_placements.coding_agent` → `"claude"`,
  `monitor_config.interval_seconds` → `60`, `monitor_config.enabled` → `1`
  (stored as INTEGER 0/1, a boolean-as-int), `monitor_runtime.tick_seconds` →
  `5`.
- **No-FK task columns.** `tasks.from_agent_id`, `tasks.to_agent_id`, and
  `tasks.origin_task_id` are plain integer columns with **no** FK constraint;
  only `tasks.context_id` is FK-constrained (ON DELETE RESTRICT).
- **JSON-path access.** `agent_card_json` is queried via SQLite JSON path
  extraction at `$.cafleet.kind`; the reimplementation must support JSON-path
  extraction over that column.
- **Soft delete** lives in `fleets.deleted_at` (a non-null timestamp marks the
  fleet deleted); this layer never physically removes rows. `tasks.text` always
  stores the full untruncated body — message text is never truncated at
  persistence.
- All timestamp columns are stored as ISO-8601 text (§5.1);
  `monitor_config.enabled` is stored as an integer 0/1 used as a boolean.

### 6.2 Broker

**Scope:** the synchronous data-access layer shared by CLI and WebUI; the only
module that reads/writes the operational tables (fleets, agents, placements,
members, messaging tasks, monitor schedule/runtime, task queries). Owns
transaction boundaries, the agent-kind predicates, soft-delete + cascade, the
task status lifecycle, and the monitor single-instance claim/heartbeat/clear. It
performs no OS side effects except one best-effort inline-preview keystroke
during message delivery (§6.5) and one process-liveness probe (signal-0).

#### Session semantics

- **read_session** — opens a read-only connection with no transaction wrapper;
  used by every query/read function.
- **write_session** — opens a connection inside a single transaction that
  commits on clean exit and rolls back on any exception. **Every mutating
  function wraps all of its writes in exactly one `write_session` block** — its
  mutations all commit together or all roll back.
- Three functions — `enroll_agent`, `delete_fleet_monitor_rows`,
  `delete_agent_monitor_row` — take an existing transaction as their first
  argument and participate in the *caller's* transaction (atomic registration /
  atomic cascade). Every other function opens its own session.
- "Exactly one row" reads (EXISTS / aggregate / single-row lookups) assume
  exactly one row and fail loudly if the invariant breaks. Do not coerce a
  missing row to a default.

#### Kind predicates, constants, and intervals

`is_administrator(card)` and `is_monitoring_member(card)` parse
`agent_card_json`, read `$.cafleet.kind`, and compare to their kind constant.
Absent / null / empty / malformed-JSON / non-object `cafleet` value → non-match
(`false`) — a deliberate, documented non-match, not an error mask. These
collapse to the two-value broker projection (`ADMINISTRATOR_KIND` vs `"user"`)
in `get_agent` / `list_agents` / `list_fleet_agents` per §5.4;
`monitoring-member` is never surfaced by those projections.

- `ADMINISTRATOR_KIND = "builtin-administrator"`,
  `MONITORING_MEMBER_KIND = "monitoring-member"`.
- Enrollment intervals: the root Director is enrolled at **180 seconds**
  (`DIRECTOR_PING_INTERVAL_SECONDS`) by `create_fleet`; ordinary pane-bound
  members at **720 seconds** (`MEMBER_PING_INTERVAL_SECONDS`) by
  `register_agent`. The monitoring member and the Administrator are **never**
  enrolled.
- Liveness staleness: `MONITOR_STALE_FACTOR = 3`,
  `MONITOR_STALE_FLOOR_SECONDS = 15` → `stale_after = max(3·tick_seconds, 15)`.
- Root Director identity strings written by `create_fleet`: name `Director`,
  description `Root Director for this fleet`.

#### Fleets

- **`create_fleet(label, director_context, coding_agent)`** — atomically
  bootstraps a fleet, its root Director, and a built-in Administrator in one
  write_session. Order: stamp `created_at`; insert the fleet with
  `director_agent_id = NULL`; insert the Director agent (`name="Director"`,
  `description="Root Director for this fleet"`, `status="active"`, card
  `{name, description, skills:[]}` with **no** `cafleet.kind`); insert the
  Director's placement with **`director_agent_id = NULL`** (the sentinel marking
  the root Director's own placement) plus the tmux identity and `coding_agent`;
  enroll the Director at 180s; back-fill the fleet's `director_agent_id`; insert
  the Administrator agent (`status="active"`, card with
  `cafleet:{kind:"builtin-administrator"}`, description `Built-in administrator
  agent for fleet {fleet_id}`) with **no placement** and **not enrolled**.
  Returns `{fleet_id, label, created_at, administrator_agent_id, director:{…}}`.
- **`list_fleets()`** — one record `{fleet_id, director_agent_id, label,
  created_at, agent_count}` per non-soft-deleted fleet (`deleted_at IS NULL`);
  `agent_count` counts only **active** agents (0 for empty fleets). Ordering:
  **`created_at DESC, fleet_id ASC`**.
- **`get_fleet(fleet_id)`** — single-row lookup by id; **includes soft-deleted
  fleets** (no `deleted_at` filter) and exposes `deleted_at` so callers
  distinguish missing (None) from soft-deleted. Returns `{fleet_id, label,
  created_at, deleted_at, director_agent_id}` or None.
- **`delete_fleet(fleet_id)`** — soft-delete + cascade-deregister in one
  write_session; **idempotent**. If the fleet row does not exist → application
  error `fleet '{fleet_id}' not found.`. Set `deleted_at = now` where
  `deleted_at IS NULL`; if zero rows updated, short-circuit return
  `{deregistered_count: 0}`. Else flip all active agents to `deregistered`
  (stamping `deregistered_at`), hard-delete their placements, delete the fleet's
  monitor rows. **Tasks are never deleted.** Returns `{deregistered_count}`.

#### Agents

- **`register_agent(fleet_id, name, description, skills, placement, kind)`** —
  pre-transaction validation, then one write_session. Pre-checks: `get_fleet`;
  if None → usage error `Fleet '{fleet_id}' not found.`; if `deleted_at` set →
  usage error `fleet {fleet_id} is deleted`. Build the card `{name, description,
  skills: skills or []}`, adding `cafleet:{kind}` when `kind` is given. Inside
  the transaction:
  - **Monitoring-member guard** (only when `kind == "monitoring-member"`): if
    `placement` is None → application error `a monitoring member must be
    pane-bound; register it via 'cafleet agent spawn --role monitor'
    (placement required).`; if the fleet already has an active monitoring member
    → application error `fleet {fleet_id} already has an active monitoring
    member (agent {existing}); only one is allowed.`. **This is the single
    one-monitor-per-fleet enforcement site** — the CLI passes `kind` through
    unchecked.
  - **Placement validation** (only when `placement` is given): let `director_id
    = placement.director_agent_id`; look up that agent active in this fleet; if
    absent → usage error `Director agent '{director_id}' not found or not active
    in fleet '{fleet_id}'.`; if that director is an Administrator → application
    error `Administrator cannot be a director`; if `director_id !=
    fleet.director_agent_id` → usage error `nested teams are not supported;
    placement director_agent_id {director_id} must equal the fleet root Director
    {root_director_id}.` (**nested teams forbidden**).
  - Insert the agent; if `placement` given, insert it; then, **only if `kind` is
    neither monitoring-member nor administrator**, enroll the agent at 720s.
- **`get_agent(agent_id, fleet_id)`** — **active only**. Returns `{agent_id,
  name, description, status, registered_at, kind, placement}` where `kind` is
  `ADMINISTRATOR_KIND` if the card marks an administrator else `"user"`; None if
  absent.
- **`list_agents(fleet_id)`** — `{agent_id, name, description, status,
  registered_at}` per **active** agent; `status` is hardcoded `"active"`.
- **`deregister_agent(agent_id)`** — soft-delete one agent + drop placement +
  monitor row. If the agent is the root Director of any fleet → **application
  error (exit 1)** `cannot deregister the root Director; use 'cafleet fleet
  delete' instead`; if it is an Administrator → application error `Administrator
  cannot be deregistered`. The root-Director guard raises a single
  **application** error (exit 1) here on the broker side and identically on the
  `cafleet agent deregister` CLI side (§6.3) — one error model for the same
  string and condition. Flip `active → deregistered` (stamp `deregistered_at`);
  if a row was flipped, hard-delete its placement and monitor_config row. Returns
  `true` iff a row was flipped.
- **`update_placement_pane_id(agent_id, pane_id)`** — set `tmux_pane_id` for the
  agent's placement; None if no placement row; else returns the placement
  projection. Called after the multiplexer resolves a spawned pane's real id.
- **`verify_agent_fleet(agent_id, fleet_id)`** — EXISTS check; **status-
  agnostic** (deregistered agents still pass).
- **`list_fleet_agents(fleet_id)`** — active agents **plus** deregistered agents
  that still own tasks (a task exists with `context_id = agent_id OR
  from_agent_id = agent_id`), so the audit-relevant deregistered set stays
  visible. Returns `{…, kind}` collapsed to `ADMINISTRATOR_KIND` vs `"user"`.
- **`get_agent_names(agent_ids)`** — empty input → `{}` with no query; else a
  map id→name; **status-agnostic**.

#### Members

A "member" is an agent joined to its placement, excluding the root Director.

- **`list_members(fleet_id)`** — joins agents to placements where the agent is
  **active in the fleet** AND the placement's **`director_agent_id IS NOT
  NULL`** (this excludes the root Director's own NULL-director placement while
  including every member). Returns `{agent_id, name, description, status,
  registered_at, placement}` per row.
- **`list_members_with_activity(fleet_id)`** — `list_members` plus three
  correlated per-member aggregates over tasks, all filtered to `type !=
  "broadcast_summary"`: `last_sent` (max `status_timestamp` where `from_agent_id
  = agent_id`), `last_recv` (where `context_id = agent_id`), `last_ack` (where
  `context_id = agent_id` AND `status_state = "completed"`). Then `idle` against
  a single `now`: take the non-null of `(last_sent, last_recv)`; none → `idle =
  null`; else `most_recent` = lexicographic max of the ISO timestamps, `idle =
  max(0, floor(now − most_recent))` in seconds.

#### Messaging

- **`send_message(fleet_id, agent_id, to, text)`** — one unicast task + best-
  effort notify, one write_session. Coerce `to` to int; on failure → value error
  `Invalid destination format: {to}`. If the sender is not active in the fleet →
  value error `Sender agent not found or not active in fleet: {agent_id}`. Find
  the destination among active agents; absent → value error `Destination agent
  not found: {to_id}`; in a different fleet → value error `Destination agent not
  in fleet: {to_id}`. Build the unicast task (`context_id = to_id`,
  `from_agent_id = agent_id`, `to_agent_id = to_id`, `type = "unicast"`,
  `status_state = "input_required"`, `origin_task_id = null`), insert, then
  `notification_sent = _try_notify_recipient(...)`. The persisted row holds the
  **full untruncated text**. Returns `{task, notification_sent}`.
- **`broadcast_message(fleet_id, agent_id, text)`** — fan out one unicast
  delivery per active non-admin peer plus one `broadcast_summary` owned by the
  sender. Sender not active → value error `Sender agent not found or not active
  in fleet: {agent_id}`. Recipients = active agents in the fleet, **excluding
  the sender** and **excluding Administrators** (the monitoring member and the
  Director **are** included); let `N` = the count of these recipients. Build the
  summary (`context_id = agent_id`, `from_agent_id = agent_id`, **`to_agent_id =
  NULL`**, `type = "broadcast_summary"`, `status_state = "completed"`, `text =
  "Broadcast sent to {N} recipients"`), insert it, set its `origin_task_id` to
  its own `task_id` (self-referential), then insert each delivery with
  `origin_task_id = summary.task_id`. **After all deliveries are inserted (still
  inside the same write_session), call `_try_notify_recipient` once per delivery
  and set `delivered` = the count of those calls that returned `true`** (the sum
  of successful best-effort inline previews; a paneless or self-recipient
  delivery contributes 0). Returns a **single-element list** `[{task: <summary>,
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
- **`poll_tasks(agent_id)`** — un-acked deliveries: `context_id = agent_id` AND
  `status_state = "input_required"`, `broadcast_summary` excluded, ordered
  `status_timestamp DESC`.
- **`ack_task` / `cancel_task(agent_id, task_id)`** — both transition a task in
  one write_session. Load; absent → value error `Task {task_id} not found`. If
  the caller is not the authorized party → permission error. If `status_state`
  is not `input_required` → value error `Cannot {verb} task in state
  {status_state}` (verb `ACK` / `cancel`). Set the new state and
  `status_timestamp = now`. **ack**: authorized = recipient (`context_id`); new
  state `completed`; permission error `Only the recipient can ACK a task`.
  **cancel**: authorized = sender (`from_agent_id`); new state `canceled`;
  permission error `Only the sender can cancel a task`. `input_required` is the
  only state a task may transition from.

#### Queries

- **`list_inbox(agent_id)`** — all tasks where `context_id = agent_id`, any
  state, `broadcast_summary` excluded, ordered `status_timestamp DESC`.
- **`list_sent(agent_id)`** — all tasks where `from_agent_id = agent_id`, any
  state, `broadcast_summary` excluded, ordered `status_timestamp DESC`.
- **`list_timeline(fleet_id, limit=200)`** — tasks joined to their **sender's**
  agent row, filtered to the sender's `fleet_id`, `broadcast_summary` excluded,
  ordered `status_timestamp DESC`, capped at `limit`.
- **`get_task(fleet_id, task_id)`** — fleet-gated. Load; absent → value error
  `Task {task_id} not found`. Build the endpoint set `[from_agent_id]`,
  appending `to_agent_id` only when it is **non-null** (so a `broadcast_summary`
  row's `NULL` recipient is dropped). If no endpoint agent belongs to `fleet_id`
  → value error `Task
  {task_id} not found` (**same message** — the out-of-fleet gate is hidden as
  not-found).

#### Monitor — schedule CRUD & ping recording

- **`enroll_agent(session, agent_id, interval)`** (in caller's transaction) —
  inserts a monitor_config row (`interval_seconds = interval`, `enabled =
  true`), atomically with the agent/placement insert.
- **`find_monitoring_member(fleet_id)`** — locates the monitoring member **by
  card kind** (not a monitor_config row — it is the unenrolled watcher); must be
  active in the fleet **and pane-bound** (a null pane is treated as absent).
  Returns `{agent_id, name, pane_id}` or None.
- **`get_monitor_config(fleet_id, agent_id)`** — `{agent_id, interval_seconds,
  last_ping_at, enabled}` with `enabled` as a boolean; None if not enrolled / not
  in fleet.
- **`list_monitor_configs(fleet_id)`** — every enrolled agent's config in the
  fleet, `enabled` as boolean.
- **`update_monitor_config(fleet_id, agent_id, interval_seconds=None,
  enabled=None)`** — if not enrolled → application error `agent {agent_id} is
  not enrolled in monitoring for fleet {fleet_id}.`. **Partial update** — only
  the supplied (non-null) fields change (`enabled` stored as 0/1). Returns the
  updated config.
- **`record_pings(agent_ids, when)`** — empty list → no-op (no transaction);
  else set `last_ping_at = when` for all listed configs. **`record_ping`** is a
  thin wrapper over `record_pings([agent_id], when)`.
- **`list_monitor_targets(fleet_id)`** — one row per **active, enrolled** agent
  (the watched set; the monitoring member is excluded by the monitor_config
  join). Each row: `{agent_id, name, is_director, pane_id, interval_seconds,
  last_ping_at, enabled, pending_count}`, where `pending_count` counts tasks
  with `context_id = agent_id`, `status_state = "input_required"`, `type !=
  "broadcast_summary"`.

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
- **`heartbeat_monitor_runtime(fleet_id, pid, when)`** — update `pid` and
  `last_tick_at = when` **only where the current pid equals the caller's pid**;
  returns `true` iff exactly one row matched. **Ownership-checked** — `false`
  when the slot was reclaimed; that `false` is the displaced monitor's
  self-terminate signal.
- **`clear_monitor_runtime(fleet_id, pid)`** — null the slot's `pid` /
  `started_at` / `last_tick_at` **only where the current pid equals the
  caller's pid**. **Ownership-checked** → a non-owner clear is a no-op, so a
  self-terminating loser never wipes the winner's row.
- **`read_monitor_runtime(fleet_id)`** — `{fleet_id, pid, started_at,
  last_tick_at, tick_seconds}` or None.
- **`monitor_is_live(fleet_id, now)`** — `false` if no row, else `_is_live`. An
  advisory pre-check for `monitor start`; the atomic claim is authoritative.
- **`delete_fleet_monitor_rows(session, fleet_id)`** /
  **`delete_agent_monitor_row(session, agent_id)`** (in caller's transaction) —
  in-transaction cascade deletes: the fleet variant deletes the fleet's
  monitor_config rows (by agent membership) and its monitor_runtime row; the
  agent variant deletes the single agent's monitor_config row.

#### Soft-delete + cascade summary

- Agents and fleets are **never row-deleted** — they flip to
  `status="deregistered"` / `deleted_at` set.
- Placements and monitor rows (monitor_config, monitor_runtime) **are
  hard-deleted** on cascade.
- **Tasks are never deleted** — audit history is permanent.
- Deregistered agents remain visible via `verify_agent_fleet`,
  `get_agent_names` (both status-agnostic), and `list_fleet_agents` (when they
  still own tasks); they are hidden from `get_agent`, `list_agents`,
  `list_members` (active-only).

#### Contract error strings → exception class → exit code

Usage-class → exit 2; application-class → exit 1; value/permission errors are
raised by messaging/queries and translated by the caller (CLI → exit 1, WebUI →
HTTP status); permission errors gate authorization. The exit-code policy is
§7.2; the strings below are the broker's contract.

| Function | Class | Message |
|---|---|---|
| `register_agent` | usage | `Fleet '{fleet_id}' not found.` |
| `register_agent` | usage | `fleet {fleet_id} is deleted` |
| `register_agent` | application | `a monitoring member must be pane-bound; register it via 'cafleet agent spawn --role monitor' (placement required).` |
| `register_agent` | application | `fleet {fleet_id} already has an active monitoring member (agent {existing}); only one is allowed.` |
| `register_agent` | usage | `Director agent '{director_id}' not found or not active in fleet '{fleet_id}'.` |
| `register_agent` | application | `Administrator cannot be a director` |
| `register_agent` | usage | `nested teams are not supported; placement director_agent_id {director_id} must equal the fleet root Director {root_director_id}.` |
| `deregister_agent` | application | `cannot deregister the root Director; use 'cafleet fleet delete' instead` |
| `deregister_agent` | application | `Administrator cannot be deregistered` |
| `delete_fleet` | application | `fleet '{fleet_id}' not found.` |
| `update_monitor_config` | application | `agent {agent_id} is not enrolled in monitoring for fleet {fleet_id}.` |
| `send_message` | value | `Invalid destination format: {to}` |
| `send_message` | value | `Sender agent not found or not active in fleet: {agent_id}` |
| `send_message` | value | `Destination agent not found: {to_id}` |
| `send_message` | value | `Destination agent not in fleet: {to_id}` |
| `broadcast_message` | value | `Sender agent not found or not active in fleet: {agent_id}` |
| `ack_task` / `cancel_task` | value | `Task {task_id} not found` |
| `ack_task` / `cancel_task` | value | `Cannot {verb} task in state {status_state}` |
| `ack_task` | permission | `Only the recipient can ACK a task` |
| `cancel_task` | permission | `Only the sender can cancel a task` |
| `get_task` | value | `Task {task_id} not found` (missing and out-of-fleet) |

### 6.3 CLI

**Scope:** the entire `cafleet` command tree (24 commands across 5 groups + 3
top-level commands — §1, §10), the shared option guards, and the `agent spawn`
spawn orchestration + rollback ladder. Orchestration glue only — it wires
broker/multiplexer/output/coding-agent. The command/option checklist is §10; this
section gives the per-command semantics. Exit codes are §7.2; application errors
(exit 1) and usage errors (exit 2) are printed as `Error: <message>` to stderr
(usage errors additionally print a usage line).

#### Global options & top-level group

The top-level command is `cafleet`, group help `CAFleet — CLI for the message
broker and agent registry.`. Two options live before any subcommand:

- `--json` — a global boolean flag, default `false`, stored on a shared context
  object every handler can read (distinct from the local `--json` some
  subcommands also declare).
- `--version` — prints `cafleet <version>` and exits 0, short-circuiting before
  subcommand dispatch, so it **bypasses** the `--fleet-id` requirement.

#### The `--fleet-id` required option and `CAFLEET_FLEET_ID`

`--fleet-id` is a **plain required integer option** on every subcommand that
operates within a fleet. There is no optional-at-parser flag and no hand-written
callback: a missing `--fleet-id` is a **parser-native missing-required-option
error (exit 2)**; a non-integer value is a parse-time usage error (exit 2). Help
text: `Fleet ID (integer); required for this subcommand.`. The guard never
defaults to an arbitrary fleet.

- **`CAFLEET_FLEET_ID` env default.** When set, it supplies the default for
  `--fleet-id` so it need not be retyped each call; an explicit `--fleet-id`
  overrides it. A non-integer `CAFLEET_FLEET_ID` fails loudly at parse time
  (exit 2). This is the **only** flag the spawned-pane identity env vars auto-bind
  (§7.1) — `CAFLEET_AGENT_ID` / `CAFLEET_DIRECTOR_AGENT_ID` bind no flag default.

Subcommands taking `--fleet-id`: all of `agent *`, `pane *`, `message *`,
`monitor *`, plus `fleet show` and `fleet delete`. Commands that do not operate
within a single existing fleet — `setup`, `doctor`, `server`, `fleet create`,
`fleet list` — do not declare it and reject `--fleet-id` with the parser's
unknown-option error (exit 2).

#### Shared documented flags & the agent-id option

- `--full` — boolean, default `false`, **documented**. On
  `agent list`/`show`/`spawn`, `fleet create`, and every `message` subcommand.
  There is no `--quiet` flag on any command.
- `--agent-id` — required integer. Its polarity is overloaded by design (§1):
  the **calling agent (requester)** on `agent show` and every `message *`
  subcommand; the **target agent** on `agent deregister` and every `pane *`
  subcommand. There is no `--member-id`; every pane-interaction subcommand
  targets its pane-bound agent via `--agent-id`.

#### Shared `--text` / `--text-file` body input {#text-body-input}

`message send`, `message broadcast`, `pane wake --message`, and `agent spawn`
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
**uniform** across all four commands and across inline / file / stdin. Long or
multi-line bodies use `--text-file` (or `-` stdin) to bypass the shell's
`ARG_MAX` limit.

#### The `client_command` wrapper (agent + message groups)

The `agent` and `message` groups route every leaf handler (which returns a
broker result) through one shared wrapper, configured per command by four
switches: `requires_agent_fleet`; a text renderer (optional);
`truncates_task_text` (route through task truncation + task-list rendering, call
the renderer with `full`); `renders_agent_card` (route through
agent-card rendering, call the renderer with `full` only). `truncates_task_text`
and `renders_agent_card` are mutually exclusive (configuring both is a
construction-time programmer error). Per invocation, in order:

1. **Fleet read** — read `fleet_id` from context (the required `--fleet-id`
   option, defaulted from `CAFLEET_FLEET_ID` when set, populated it).
2. **Fleet-gate** (only when `requires_agent_fleet`) — read `agent_id`; if
   absent, a programmer-error application error; if the broker reports the agent
   is not a fleet member → application error (exit 1) `agent <agent_id> is not a
   member of fleet <fleet_id>.`. **Runs before the handler body.**
3. **Handler call.**
4. **Render branch** — truncate-task / render-agent / pass-through.
5. **Emit branch** — if global `--json`, emit compact JSON; else if a text
   renderer is configured, call it (`truncates_task_text` passes `full`); else
   emit JSON.
6. **Exception wrap** — re-raise an application/usage error unchanged; wrap any
   other exception as an application error (exit 1) carrying its message.

#### `agent` group

The single mental model for registry + lifecycle: `register | list | show |
deregister | spawn`. A "member" is just an agent with a placement, so this group
is the single home for both registry CRUD and pane-bound lifecycle (`spawn` and
`deregister` are detailed in their own subsections below).

- **register** — `--name` (string, required), `--description` (string,
  required), `--skills` (string, optional); when present `--skills` is parsed as
  JSON, a failure → application error `Invalid JSON in --skills: <error>`. No
  fleet-gate.
- **list** — `--full` (documented), `--activity` (documented). Renders an indexed
  agent list of the fleet's active agents; when an agent has a placement, its row
  shows a placement/pane column. Empty case `No agents found.`.
- **show** — `--agent-id` (integer, required, fleet-gated requester), `--id`
  (integer, required, agent to show), `--full` (documented). Target not found →
  application error `Agent <id> not found`.
- **deregister** — `--agent-id` (integer, required, fleet-gated **target**),
  `--force` / `-f`. If the target has a pane it is torn down before the
  soft-delete. Full behavior — the no-pane, has-pane, `--force`, root-Director,
  and Administrator paths — is in the `agent deregister` subsection below.
- **spawn** — register **and** spawn a pane in one op. Options, the spawn
  sequence, and the rollback ladder are in the `agent spawn` subsection below.

#### `doctor`

Only global `--json`. Ensures tmux is available (wrapping a tmux error as an
application error), discovers the tmux context (re-wrapping as an application
error, exit 1), and reads the `TMUX_PANE` environment variable by **direct
access** — an unset variable is an intentional loud failure, not a default.
Emits the four pane identifiers (session name, window id, pane id, raw
`TMUX_PANE`) as a JSON object under a `tmux` key (`session_name`, `window_id`,
`pane_id`, `tmux_pane_env`) or a four-line text block.

#### `fleet` group

Does **not** use `client_command`. Each subcommand with a local `--json` flag
emits JSON when **either** the local flag **or** global `--json` is set.

`fleet create` and `fleet list` do **not** take `--fleet-id`; `fleet show` and
`fleet delete` take the **required `--fleet-id` option**, defaulting from
`CAFLEET_FLEET_ID` like every other fleet-scoped command (§6.3 `--fleet-id`).

- **create** — `--label` (string, optional), `--coding-agent` (choice over the
  coding-agent names, default `claude`, shown in help), `--json` (local),
  `--full` (documented). Requires tmux: on a tmux error → application error
  `cafleet fleet create must be run inside a tmux session` (exit 1, no DB
  writes).
- **list** — `--json` (local). Empty → `No fleets found.`; else a header plus
  one formatted row per fleet (column widths 40 / 20 / 8; nullable cells fall
  back to empty strings).
- **show** — `--fleet-id` (integer, required) + local `--json`. Not found →
  application error `fleet '<fleet_id>' not found.`. Text: `fleet_id`, `label`,
  `created_at`, plus a `deleted_at:` line when soft-deleted (soft-deleted rows
  are returned intentionally).
- **delete** — `--fleet-id` (integer, required); no `--force`. Prints `Deleted
  fleet <fleet_id>. Deregistered <n> agents.`; idempotent (an already-deleted
  fleet reports 0 agents).

#### `message` group

All six route through `client_command`. Common: `--agent-id` (integer, required
— the calling agent/requester) on all; `--task-id` (integer, required) on
`ack`/`cancel`/`show`; `--full` (documented) on all. No `--quiet` flag.

- **send** — also `--to` (integer, required) and the shared `--text` /
  `--text-file` body pair (exactly one required; §6.3 [text-body
  input](#text-body-input)). Fleet-gated; truncates task text. Prints `Message
  sent.\n` + the formatted task.
- **broadcast** — also the shared `--text` / `--text-file` body pair (exactly
  one required; §6.3 [text-body input](#text-body-input)). **Not** fleet-gated; the
  result is a list; `--full` → the formatted first task envelope; else `broadcast
  id=<task_id> recipients=<N> delivered=<k>`, where `<N>` is the result's
  `recipients` (the real recipient count, matching `Broadcast sent to {N}
  recipients`) and `<k>` is the result's `delivered` (the count of best-effort
  inline previews that landed). The two diverge when any preview fails to deliver;
  they are reported as **separate fields**, not conflated (the broker computes
  both, §6.2). In JSON mode the result object carries both `recipients` and
  `delivered`.
- **poll** — fleet-gated; indexed task list; empty `No messages found.`.
- **ack** — fleet-gated; prefix `Message acknowledged.\n` + the formatted task.
- **cancel** — fleet-gated; prefix `Task canceled.\n` + the formatted task.
- **show** — fetches the task within the fleet; text is the formatted task.

#### `agent`/`pane` group — shared resolution helpers

These helpers back both `agent spawn`/`agent deregister` and every `pane`
subcommand. The polarity of `--agent-id` here is the **target** agent (§1).

- **Require-pane** — given a placement and an action label
  (`capture`/`exec`/`wake`), no pane id → application error `agent
  <agent_id> has no pane yet (pending placement) — nothing to <action>.`.
- **Load-authorized-agent** — fetch the agent within the fleet: not found →
  `Agent <agent_id> not found`; other fetch failure → `failed to fetch agent:
  <error>`; absent placement → a caller-supplied "placement missing" message
  (default ``agent <agent_id> has no placement row; it was not spawned via
  `cafleet agent spawn`.``). Does **not** check pane presence (only `agent
  deregister` tolerates a pending placement). Callers re-fetch by the canonical
  agent id.
- **Deregister-with-warning** — best-effort deregister; on failure print a
  `WARNING: rollback deregister failed …` line to **stderr**, do not raise.
- **Rollback-register** — deregister-with-warning, then raise an application
  error `<reason>. Rolled back registration of <new_agent_id>.`.
- **Resolve-coding-agent** — explicit `--coding-agent` wins; else a non-monitor
  role → `claude`; else (monitor role, no flag) inherit the Director's placement
  coding agent, with three error surfaces (Director fetch failure / not found /
  no placement), each ending `Re-run with an explicit --coding-agent.`.

#### `agent spawn` — spawn orchestration & rollback ladder

The one genuinely distinct lifecycle op: register **and** spawn a pane.
Options: `--agent-id` (integer, required — the Director),
`--name` (string, required), `--description` (string, required), `--coding-agent`
(choice, optional — resolved when absent), `--model` (string, optional), `--role`
(choice over `member`/`monitor`, default `member`, shown in help), the shared
`--text` / `--text-file` body pair (exactly one required; §6.3 [text-body
input](#text-body-input)), and `--full` (documented). Sequence:

1. **Resolve the body** — via the shared `--text` / `--text-file` reader (§6.3
   [text-body input](#text-body-input)): exactly-one-required, `-` stdin, abs /
   CWD-relative path, UTF-8, uniform empty-body rejection. A mutual-exclusivity
   or empty-inline error is a usage error (exit 2); a file / stdin surface is an
   application error (exit 1). Resolved **before any registration or tmux side
   effect**; substitution (step 6) is deferred until the new agent id exists.
2. Read `fleet_id`; resolve the coding agent; look up the backend.
3. **Model validation** — validate `--model`; a failure → usage error (exit 2)
   with the backend's message, **before any registration or tmux side effect**.
4. **Preconditions** — ensure tmux available, the backend binary on PATH, and
   discover the tmux context; any tmux/runtime error → application error (exit
   1).
5. **Register the agent** — with a placement carrying the Director id, tmux
   session, tmux window id, an unset pane id, and the coding agent; kind = the
   monitoring-member kind when role is `monitor`, else unset. Re-raise an
   application error verbatim (preserves the one-monitoring-member message); wrap
   any other exception as `register failed: <error>`. Capture the new agent id.
6. **Substitute placeholders** (below) — run `str.format` over the resolved body
   (step 1), substituting `{fleet_id}` / `{agent_id}` (the new agent id from step
   5) / `{director_agent_id}` / `{coding_agent}`. An unknown-placeholder or
   malformed-brace error is a usage error (exit 2); on it,
   **deregister-with-warning, then re-raise the original error unwrapped** —
   preserving both the exact message and its exit code.
7. **Build the spawn argv** from the backend (verbatim prompt, display name,
   model).
8. **Split the pane** — forward identity into the new pane's environment, then
   split the window to obtain the pane id. The forwarded env vars are
   `CAFLEET_DATABASE_URL` (when set), `CAFLEET_FLEET_ID`, `CAFLEET_AGENT_ID` (the
   spawned agent's own id, captured in step 5), and `CAFLEET_DIRECTOR_AGENT_ID`
   (the `--agent-id` Director) — the env-var identity-delivery mechanism (§7.1).
   tmux error → rollback-register, reason `tmux split-window failed: <error>`.
9. **Patch the pane id** — record it on the placement. On exception: best-effort
   send `/exit` (tolerating a missing pane), then rollback-register, reason
   `placement update failed: <error>`. If the placement row vanished: same
   best-effort `/exit`, then rollback-register, reason `placement row vanished
   before pane-id patch`.
10. **Emit** — attach the placement view; emit JSON or the spawned-agent text
    formatter (`format_member`, §6.4, honoring `--full`).

The ladder contract: any post-register failure deregisters the agent so no
orphan row survives; the best-effort cleanup never masks the original error.

#### `agent deregister`

The merged registry-soft-delete + pane-teardown op. Options: `--agent-id`
(integer, required — the **target**), `--force` / `-f` (boolean, default
`false`). One unified error model — every failure path is
an **application error (exit 1)**; there is no exit-2 timeout path.

1. Ensure tmux available.
2. **Root-Director guard, before any pane mutation** — fetch the fleet; if the
   target is the fleet's Director → application error (exit 1) `cannot deregister
   the root Director; use 'cafleet fleet delete' instead` (the same string and
   exit code the broker's `deregister_agent` guard raises, §6.2).
3. Load the authorized agent with placement-missing message ``agent <agent_id>
   has no placement; it was not spawned via `cafleet agent spawn`.``; re-fetch
   the canonical id and read the pane id.
4. **No pane** (never spawned, or pending placement) — registry soft-delete via
   the broker. Nothing deregistered → application error `agent <agent_id> not
   found or already deregistered.`; an Administrator target → the broker's
   application error `Administrator cannot be deregistered`. Success: header
   `Agent deregistered successfully.`, pane status `(pending — no pane)`, exit 0.
5. **`--force`** (has pane) — kill the pane immediately, skipping the graceful
   wait (tolerating a missing pane); a tmux error → application error `kill_pane
   failed for pane <pane_id>: <error>. The tmux server may be unreachable. Verify
   with 'cafleet doctor', then re-run the command.`. Then deregister; header
   `Agent deregistered (--force).`, pane status `<pane_id> (killed)`, exit 0.
6. **Default path** (has pane) — send the backend exit keystroke (tolerating a
   missing pane; a tmux error → application error carrying the `send_exit failed
   …` recovery message), then wait up to 15 s for the pane to disappear, polling
   every 0.5 s (a tmux error during the wait → application error `tmux call failed
   while waiting for pane <pane_id> to close: <error>`).
   - **Pane gone** — deregister; header `Agent deregistered successfully.`, pane
     status `<pane_id> (closed)`, exit 0.
   - **Timeout** — capture the pane's last 80 lines (a capture error prints a
     stderr warning and yields an empty tail); print to **stderr** the block:
     `Error: pane <pane_id> did not close within 15.0s after the exit keystroke.`,
     then `--- pane <pane_id> tail (last 80 lines) ---`, the tail, `---`, and a
     `Recovery: …` hint; pane status `<pane_id> (timeout)`; emit a JSON object
     (`agent_id`, `pane_status`) in JSON mode; **application error (exit 1)**.

Success text: the header line plus an indented pane-status line; JSON:
`{agent_id, pane_status}`.

#### `agent list` activity rows

`agent list` takes `--activity` (documented boolean, default `false`). Lists the
fleet's active agents (or active agents-with-activity when `--activity`). The
base list shows a placement/pane column for each placed agent; the `--activity`
rows add the per-agent send/recv/ack/idle aggregates. JSON emits the raw rows.

#### `pane` group

The single home for keystroke interaction with a pane-bound agent: `capture |
exec | wake`. Every subcommand targets its agent via **`--agent-id`**
(the **target**, §1), uses the shared resolution helpers above, and is documented
(no hidden interaction flags remain).

#### `pane capture`

Options: `--agent-id` (integer, required), `--lines` (integer, default **20**,
shown in help; `--lines` is the single spelling, with no alias), `--ansi` /
`--no-ansi` (documented boolean pair, default `false`).
Ensure tmux, load the agent, require a pane (`capture`). Capture the last N lines
(a tmux error → application error `capture failed: <error>`). When `--ansi` is
not set, strip ANSI. JSON: `{agent_id, pane_id, lines, content}`; text emits the
content with no trailing newline, **preserving ANSI even on a non-TTY sink** when
`--ansi` is set.

#### `pane exec`

Options: `--agent-id` (integer, required), **positional** `command` (string,
required). A newline/CR → usage error `command may not contain newlines.`; empty
after trim → usage error `command may not be empty.`; then trim. Ensure tmux,
load the agent, require a pane (`exec`). Dispatch via the coding agent's `!`
shell shortcut (a tmux error → application error `send failed: <error>`). JSON:
`{agent_id, pane_id, command}`; text: `Sent bash command <quoted-command> to
agent <name> (<pane_id>).` (the command rendered with human-readable
quoting/escaping — reproducing the quoted intent is sufficient).

#### `pane wake`

One command, two mutually-exclusive modes for waking a pane-bound agent.
Options: `--agent-id` (integer, required — the **target**), `--poll-only`
(boolean) and `--message` (boolean) selecting the mode, plus `--from` (integer)
and the shared `--text` / `--text-file` body pair (§6.3 [text-body
input](#text-body-input)) for `--message`. Exactly one of `--poll-only` /
`--message` is required; supplying both, or `--message` without `--from` or
without a body (`--text` / `--text-file`), is a usage error (exit 2). There is no
`--quiet`.

- **`--poll-only`** — Ensure tmux, load the target, require a pane (`wake`).
  Inject the inbox-poll keystroke via the multiplexer's
  `send_poll_trigger`, which is **best-effort** (§6.5) — it returns a boolean and
  never raises. A returned `false` (non-delivery) → application error `send
  failed: tmux send-keys did not deliver the poll-trigger keystroke to pane
  <pane_id>.`. Because `send_poll_trigger` swallows its own `TmuxError` and
  returns `false`, the only reachable failure surface is the non-delivery message
  above. JSON: `{agent_id, pane_id}`; text: `Woke agent <name> (<pane_id>) — poll
  keystroke dispatched.`.
- **`--message`** — requires `--from <sender-agent-id>` (the dispatching
  Director) and a body via the shared `--text` / `--text-file` pair (§6.3
  [text-body input](#text-body-input)); the **target** is `--agent-id` and the
  **sender** is `--from` (the `--agent-id` target polarity of §1). Ensure tmux.
  The body is resolved by the shared reader (empty/whitespace inline `--text` →
  usage error `text may not be empty.`; file/stdin surfaces per §6.3). Resolve the
  target (fleet-isolation only; re-fetch the canonical id). Send the message from
  `--from` to the target; a sender-not-active value error → application error
  carrying that message. JSON: `{agent_id, pane_id, task_id, notification_sent}`
  (a boolean, not a count). Text, by outcome: notification sent → `Woke <name>
  (<pane_id>) — task <task_id> queued, Esc-safeguarded preview dispatched.`; no
  pane → `Woke <name> — no pane; task <task_id> queued.`; otherwise → `Woke
  <name> (<pane_id>) — task <task_id> queued; inline preview not delivered.`.

#### `monitor` group

A shared `_require_live_fleet` guard fetches the fleet; missing or soft-deleted
→ application error `fleet <fleet_id> not found`.

- **start** — `--tick` (integer ≥1, default 5, shown in help). Requires a live
  fleet, then tmux. No monitoring member → a warn-but-run line to **stderr**:
  `Warning: fleet <fleet_id> has no monitoring member; the monitor heartbeat
  will wake no agent. Spawn one first with 'cafleet agent spawn --role
  monitor'.`. Then run the monitor loop in-process (blocking).
- **status** — requires a live fleet; reads the runtime row at the current UTC
  time. Not running / no row → a not-running payload (`running` false; `pid`,
  `last_tick_at`, `last_tick_age_seconds`, `started_at` null; `tick_seconds`
  from the row when present, else null). Else a live payload with
  `last_tick_age_seconds`. Per-agent list from the monitor targets, each
  carrying `agent_id`, `name`, `interval_seconds`, `last_ping_at`,
  `last_ping_age_seconds`, `enabled`, `pending_count`, and a `role` of
  `director`/`member`. Payload `{runtime, agents}`.
- **config** — `--agent-id` (integer, required), `--interval` (integer ≥1,
  optional), `--enable` / `--disable` (boolean, default `false`). `--enable`
  with `--disable` → usage error `--enable and --disable are mutually
  exclusive.`. When both `--interval` and the enabled value are unset (read-only
  mode), fetch the config; not enrolled → application error `agent <agent_id> is
  not enrolled in monitoring for fleet <fleet_id>.`. Else update.

#### `server`

Options: `--host` (string, default `settings.broker_host` = `127.0.0.1`, shown
in help), `--port` (integer, default `settings.broker_port` = `8000`, shown in
help). Serves the WebUI app on host/port; port-in-use and all other server
errors propagate unwrapped.

#### `setup`

Options: `--agent` (choice over `claude`/`codex`/`opencode`, repeatable, default
empty). Reads the CLI's own version. `setup` is the single schema-management
entry point; there is no separate `db init` command. **Both halves always run
independently** (a skills-half failure does not
abort the DB half):

- **Skills half** — resolve install targets, then download and install the
  skills release. On an application error, print `skills half failed: <message>`
  and record the failure.
- **DB half** — create the single-baseline schema to head via the schema-create
  driver (§8): force a sync SQLite URL, create the DB file's parent directory,
  and create the baseline schema fresh. On an application error, print `db half
  failed: <message>` and record the failure.
- If anything failed → application error `<failed halves joined by ' and '> half
  failed` (exit 1).

Helpers: **resolve-targets** (given `--agent`, dedupe preserving order; else
auto-detect each known coding-agent home whose parent dir exists; none →
application error `no coding-agent homes detected (looked for ~/.claude,
~/.codex, ~/.config/opencode); install a coding agent first, or pass --agent`);
**resolve-download-url** (GET the GitHub release for the tag matching the CLI
version, 30 s timeout; 404 → `no release found for version <version>`; other
HTTP/network error → `could not reach the GitHub API (<reason>)`; find asset
`cafleet-skills-v<version>.zip`; parse failure → `could not parse the GitHub API
response`; missing asset → `asset <asset_name> not found in release
<version>`); **download-and-extract** (download to a temp zip; **reject any
member whose path is absolute or contains a `..` component** with `archive
member '<member>' has an unsafe path; rejecting the archive`; a
malformed/unreadable archive → `release asset is malformed`; validate the
extracted `skills/` dir contains exactly the three skill dirs `cafleet`,
`cafleet-design-doc`, `cafleet-research`, else `release asset is malformed`);
**install-skills** (per target, copy each skill dir into the agent's skills dir,
removing any existing copy first; a filesystem error → `failed to install skills
into <skills_dir>: <error>`; success prints `<agent>: installed <skill dirs
joined by ', '> (v<version>) -> <skills dir>`). Known skills dirs: `claude` →
`~/.claude/skills`, `codex` → `~/.codex/skills`, `opencode` →
`~/.config/opencode/skills`.

#### Spawn-prompt resolution (used by `agent spawn`)

The spawn prompt is supplied through the shared `--text` / `--text-file` body
input (§6.3 [text-body input](#text-body-input)): exactly one is required, `-`
reads stdin, a relative `--text-file` path resolves against CWD, decoded UTF-8
with no newline translation. There is **no** built-in default template and **no**
positional prompt argument — a bare `agent spawn` with neither flag is the shared
usage error `Provide exactly one of --text or --text-file.` (exit 2).

**Placeholder substitution.** After the body is resolved, `agent spawn` runs
`str.format` over it, substituting `{fleet_id}`, `{agent_id}` (the spawned
agent's own id), `{director_agent_id}`, and `{coding_agent}` (the resolved
backend). A custom prompt keeps a literal brace by doubling it (`{{` / `}}`).
Error surfaces (both usage errors, exit 2): an unknown placeholder → `Unknown
placeholder '<key>' in custom prompt. Supported placeholders: {fleet_id},
{agent_id}, {director_agent_id}, {coding_agent}. Double literal braces ({{, }})
to keep them as text.`; a malformed brace expression → `Malformed custom prompt:
<detail>. Double literal braces ({{, }}) to keep them as text.`. Substitution is
applied **only** by `agent spawn`; the three message-body commands
(`message send`, `message broadcast`, `pane wake --message`) call the shared
reader alone and never run `.format`.

Identity **also** reaches the spawned agent as the environment variables injected
at `split_window` (§7.1): `CAFLEET_FLEET_ID`, `CAFLEET_AGENT_ID` (the spawned
agent's own id), and `CAFLEET_DIRECTOR_AGENT_ID` (its Director). Only
`CAFLEET_FLEET_ID` auto-defaults `--fleet-id` (§6.3); the agent reads
`$CAFLEET_AGENT_ID` / `$CAFLEET_DIRECTOR_AGENT_ID` and passes them explicitly on
its calls.

### 6.4 Output & Formatting

**Scope:** every line of human/machine output for agents, tasks, fleets,
members, and the monitor. Pure string/structure transformation — no I/O, no DB,
no network; the only external input is `settings.max_text_len` (default `200`).
Two consumers depend on these exact shapes: the CLI (which prints them) and the
WebUI (which reuses the JSON serialization but bypasses truncation). This module
sets no exit codes. (`doctor` output is produced by the CLI, §6.3, not here.)

The text-vs-JSON selection is the CLI's: `--full` and `--json` are **documented**
flags (Q5 hidden-flag cleanup). The `--json` emit path is **not** unified — the
`agent`/`message` groups emit JSON via the global `--json` through
`client_command`, while the `fleet` group keeps its own local `--json` flag OR-ed
with the global one and its own emit path (§7.3). The single absent glyph below
and the compact-JSON rules apply to both paths.

#### Two-layer architecture

- **Render layer** — projects raw broker results into slim "wire" shapes,
  truncates oversized text by codepoint count, walks/transforms nested
  structures without mutating originals, serializes to compact JSON, and strips
  ANSI from captured pane buffers.
- **Formatter layer** — consumes those shapes (or the raw dicts) and produces
  exact multi-line, column-aligned, ANSI-free terminal strings.

The split is load-bearing: formatters call render functions internally (e.g.
`format_task` calls `render_task` for compact mode), but render functions never
call formatters.

**Render functions:** `strip_ansi(text)`; `format_json(data)`;
`truncate_text(value, full, limit)`; `truncate_task_text(result, full, limit)`
(in-place); `render_task(task, full)` → `{id, from, ts, text, kind?, origin?}`;
`render_tasks_in_result(result, full)` (non-mutating, unwraps `{task: …}`
envelopes and flat task dicts); `render_agent(agent, full)` → `{id, name,
description, status, coding_agent?}` (description truncated to **60**);
`render_agents_in_result(result, full)` (non-mutating).

**Formatter functions:** `format_register`; `format_task`; `format_indexed_list`
(joins formatted items with one blank line between, `empty_msg` when empty —
not numbered); `format_agent`; `format_fleet_create`; `format_member`;
`format_member_list`; `format_member_list_activity`; `format_monitor_status`;
`format_monitor_config`. Private contract helpers: an agent-id stringifier; an
ISO→`HH:MM:SS` extractor; an idle-seconds humanizer; a ping-age humanizer.

#### Truncation rules

- Truncation counts and slices **by Unicode codepoint**, never by byte. A value
  longer than the effective limit returns its first `limit` codepoints plus a
  one-codepoint `…` (U+2026) suffix — so the result is `limit + 1` codepoints.
- `truncate_text` passes the value through unchanged when `full` is set, the
  value is null, or its codepoint length is `<= limit`. Null returns null.
- The effective limit is the explicit `limit` argument when given, else
  `max_text_len` (default `200`, from config) — the **only** config dependency.
- The **agent-description limit is a hardcoded literal `60`**, independent of
  `max_text_len`; both `render_agent` and `format_agent` verbose apply it.

#### Compact-JSON rules

`format_json` emits compact JSON: **no whitespace** between tokens (item
separator `,`, key/value separator `:`); **non-ASCII kept raw** as UTF-8 (e.g.
`…` stays 3 bytes), never escaped to `\uXXXX`; **insertion-order keys** (the
render functions build their output maps in a fixed key order, which is part of
the contract). The WebUI bypasses `truncate_*` but its JSON serialization must
still obey these three rules.

#### The `unicast` suppression sentinel

`render_task` adds a `kind` key **only** when the task's `type` is not the
literal `"unicast"`. `unicast` is the default/suppressed type; only non-`unicast`
types (e.g. `broadcast_summary`) surface a `kind`.

#### The single absent glyph

Every formatter uses **one** absent/empty placeholder — the **ASCII
hyphen-minus `-`** (U+002D). It marks an absent or empty cell everywhere: the
ISO→HMS helper (null/unparseable timestamp), the idle humanizer (null), the
ping-age humanizer (null age, in `format_monitor_status`'s `last_ping` column),
and `format_monitor_config` (null `last_ping_at`). There is no EM DASH `—`
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

`truncate_task_text` **mutates its input in place** and returns the same object.
`render_tasks_in_result` and `render_agents_in_result` are **non-mutating** (they
build new structures, shallow-copying any envelope). Preserve the distinction; in
a language with no aliasing concern, preserve the *observable* result.

#### Field access / optionality

Every field is read with required access unless marked optional; required access
**fails loud** on a missing key by design. The truthiness guards on `text` /
`origin_task_id` mean empty string and `0` are also suppressed, not just null.

- **Task** (`render_task` / `format_task` / `truncate_task_text`): `task_id`
  (req), `from_agent_id` (req), `status_timestamp` (req, compact),
  `text` (req key for compact, optional for verbose; guarded by truthiness),
  `type` (req; `"unicast"` suppresses `kind`), `status_state` (req, verbose),
  `to_agent_id` (optional, nullable; verbose `to:` line only when **non-null** —
  a `broadcast_summary` row's `NULL` recipient is skipped), `origin_task_id`
  (optional; `origin` key only when **truthy**). Envelope: a task may be wrapped
  `{task: {…}}`; `format_task`
  unwraps when the inner value is a dict; the render walker unwraps when it is a
  dict containing `task_id`.
- **Agent** (`render_agent` / `format_agent`): `agent_id` (req), `name` (req),
  `description` (req, truncated to 60), `status` (req), `placement` (optional;
  `coding_agent` emitted only if placement present and contains it).
- **Register** (`format_register`): `agent_id`, `name` (both req).
- **Fleet-create** (`format_fleet_create`): `fleet_id` (req), `director` (req
  nested) → `agent_id` (req), `name`/`placement` (req, verbose);
  `director.placement` (verbose) → `tmux_session`/`tmux_window_id`/`tmux_pane_id`
  (req); `administrator_agent_id` (req); `label` (req key, verbose, empty string
  when falsy); `created_at` (req, verbose).
- **Member-create** (`format_member`): `agent_id` (req), `name` (req),
  `placement` (req) → `coding_agent` (req), `tmux_pane_id` (req key; `(pending)`
  when falsy in compact), `tmux_window_id` (req, verbose).
- **Member-list row**: `agent_id`, `name`, `status`, `placement` →
  `{coding_agent, tmux_session, tmux_window_id, tmux_pane_id (→ "(pending)"),
  created_at}`.
- **Member-list-activity row**: `agent_id`, `name`, `status`, `last_sent`,
  `last_recv`, `last_ack` (ISO str | null), `idle` (int seconds | null).
- **Monitor-status payload**: `{runtime, agents}`. `runtime.running` (bool, req);
  when true also `pid`, `last_tick_age_seconds`, `tick_seconds`, `started_at`.
  Each agent: `agent_id`, `name`, `role`, `interval_seconds`,
  `last_ping_age_seconds` (int | null), `enabled` (bool), `pending_count`.
- **Monitor-config** (`format_monitor_config`): `agent_id`, `interval_seconds`,
  `enabled` (bool), `last_ping_at` (str | null; `-` when null).

The `(pending)` fallback for `tmux_pane_id` appears in the compact member render
and both list rows, but **not** in the verbose `format_member` block.

#### Exact text layouts

`format_register` — 3 lines (`agent_id:` + two spaces; `name:` + six spaces, so
values align):

```
Agent registered successfully!
  agent_id:  <agent_id>
  name:      <name>
```

`format_task` — **compact** line 1 by concatenation: `[<id> | from:<from> |
<ts>]`, with ` | kind:<kind>` inserted before `]` when a `kind` is present and
` | origin:<origin>` inserted (after kind) when an `origin` is present; if the
rendered `text` is truthy a second line holds the body. **Verbose** — aligned
lines: `  id:    <task_id>`, `  state: <status_state>`, `  from:  <from_agent_id>`,
then `  to:    <to_agent_id>` **only when `to_agent_id` is non-null**, then `  type:
 <type>` **always**, then `  text:  <text>` **only when `text` is truthy**.

`format_agent` — **compact**: `<agent_id> <name> <status>` (single spaces, no
labels). **Verbose** (description truncated to 60): `  agent_id:    <agent_id>`,
`  name:        <name>`, `  description: <description>`, `  status:      <status>`.

`format_fleet_create` — **compact**: `<fleet_id> director=<director.agent_id>
admin=<administrator_agent_id>`. **Verbose** — 7 lines; first two are bare
stringified values with no key prefix; `pane` joins the three placement fields
with `:`:

```
<fleet_id>
<director.agent_id>
label:            <label or "">
created_at:       <created_at>
director_name:    <director.name>
pane:             <tmux_session>:<tmux_window_id>:<tmux_pane_id>
administrator:    <administrator_agent_id>
```

`format_member` — **compact** (`pane` = `tmux_pane_id` or `(pending)`):
`<agent_id> <name> backend=<coding_agent> pane=<pane>`. **Verbose** — 6 lines
(verbose `pane_id` is the raw `tmux_pane_id`, no `(pending)`):

```
Member registered and spawned.
  agent_id:  <agent_id>
  name:      <name>
  backend:   <coding_agent>
  pane_id:   <tmux_pane_id>
  window_id: <tmux_window_id>
```

`format_member_list` — empty → `0 members.`; else a header `<count> member<s>:`
(trailing `s` only when `count > 1`; `1 member:` exactly), a column header and
separator, then one row per member. Each row begins with a two-space indent and
columns separated by two spaces, left-justified to fixed widths (longer values
are **not** truncated): `agent_id` 14, `name` 8, `status` 6, `coding_agent` 7,
`tmux_session` 7, `tmux_window_id` 9, `tmux_pane_id` (→`(pending)`) 7, then
`created_at` with no padding (last column). `agent_id` is stringified.

`format_member_list_activity` — empty → `0 members.`; same pluralized header.
Per row, left-justified: `agent_id` 14, `name` 8, `status` 6, then the HMS
extraction of `last_sent`/`last_recv`/`last_ack` each width 9, then the humanized
`idle` with no padding.

`format_monitor_status` — line 1 when running: `monitor: running (pid <pid>,
last tick <last_tick_age_seconds>s ago, tick <tick_seconds>s, started
<started_at>)`; else `monitor: stopped`. If `agents` is non-empty, append a
column header and separator, then one row per agent, left-justified: `agent_id`
8, `name` 11, `role` 8, then `<interval_seconds>s` width 8, the humanized
ping-age width 9 (ASCII `-` when null), then `yes`/`no` for `enabled` width 7,
then `pending_count` with no padding.

`format_monitor_config` — one line: `agent <agent_id>: interval
<interval_seconds>s, <state>, last_ping <last_ping>` where `<state>` is
`enabled`/`disabled` and `<last_ping>` is `last_ping_at` or ASCII `-` when null.

#### Private helper semantics

- **ISO→HMS** — returns the `HH:MM:SS` portion: the substring after `T`,
  truncated to its first 8 characters (fractional seconds/offsets dropped).
  Returns ASCII `-` when null, has no `T`, or is not a string. A shorter time
  portion yields a shorter (unpadded) string — slice, do not validate or pad.
- **idle humanizer** — null → `-`; `< 60` → `<n>s`; `< 3600` → `<n // 60>m`;
  else `<n // 3600>h` (integer floor division).
- **ping-age humanizer** — null → ASCII `-`; else `<n>s ago`.

All four absent-cell helpers above use the single ASCII `-` glyph (§6.4 *The
single absent glyph*). All conditional fields (`kind`, `origin`, the verbose
`to:`/`text:` lines, the `coding_agent` key) are gated on truthiness — omitted,
never emitted empty.

### 6.5 Multiplexer & tmux

**Scope:** the `Multiplexer` interface, the frozen `MultiplexerContext`, the
`poll_until_pane_gone` helper, and the single `TmuxMultiplexer` backend that owns
all `tmux` subprocess invocation and keystroke injection. A `MULTIPLEXERS`
registry maps `"tmux"` to a single shared stateless backend instance. Every
method invokes tmux as an **argv list without a shell** (no shell interpolation
— load-bearing for the literal `send-keys -l` payloads). The exact tmux argv each
method builds is given verbatim; preserve subcommand, flags, and ordering.

#### Method surface

- **`name`** — the registry key literal `"tmux"`.
- **`ensure_available()`** — fail-fast. Raises if `tmux` is not on `PATH` →
  `tmux binary not found on PATH`; or if `TMUX` is unset/empty → `cafleet
  tmux-pane commands must be run inside a tmux session`.
- **`context_discovery() -> MultiplexerContext`** — resolves the **calling
  shell's** pane via `$TMUX_PANE` (not the active window). Read `TMUX_PANE`;
  missing/empty → `TMUX_PANE is not set; not running inside a tmux pane`. Invoke
  `tmux display-message -p -t <TMUX_PANE> "#{session_name}|#{window_id}|#{pane_id}"`,
  strip, split on `|` into **exactly 3** parts (max-split 2); wrong count →
  `unexpected tmux display-message output: <quoted-output>`. Return the context.
- **`split_window(*, target_window_id, env, command) -> str`** — spawns a new
  **detached** pane and returns its id. Base argv `tmux split-window -t
  <target_window_id> -P -F "#{pane_id}" -d` (the `-d` detach is unconditional;
  `-P -F "#{pane_id}"` prints the new pane id); for each `(k, v)` in `env` append
  `-e <k>=<v>`; append the `command` argv elements. Run, take the printed pane
  id, then call `select_layout(target_window_id)` (default layout
  `main-vertical`, **swallowing** any error from it), and return the pane id.
  `select_layout` runs `tmux select-layout -t <target_window_id> <layout>` and is
  internal to the tmux backend (not on the interface).
- **`send_exit(*, target_pane_id, ignore_missing=False)`** — keystrokes `/exit`
  + Enter via the literal-then-Enter core, **no Esc-first**; tolerates a missing
  pane when `ignore_missing`.
- **`send_poll_trigger(*, target_pane_id, fleet_id, agent_id) -> bool`** —
  best-effort. tmux missing → `false`; payload `cafleet message poll --fleet-id
  <fleet_id> --agent-id <agent_id>`; literal-then-Enter, `timeout=5`s,
  **Esc-first=YES**, any error → `false`. Used only by `pane wake --poll-only`.
- **`send_wake_trigger(*, target_pane_id, due_agents, director_agent_id) ->
  bool`** — best-effort; the **sole** keystroke the monitor loop fires. Each due
  entry has `agent_id`, `name`, `is_director`. tmux missing → `false`; `noun =
  "agent"` if one due else `"agents"`; build `due_list` by joining with `", "`,
  each `<"director" if is_director else "member"> <agent_id> (<sanitized
  name>)`; single-line payload (note the em-dash, `{N}` = count):
  ```
  [monitor] wake: {N} {noun} due — {due_list}. Capture each named pane read-only, with the Director pane ({director_agent_id}) always inspected; judge each active/idle and progressing/stalled; re-engage the Director via cafleet pane wake --message when it is idle with un-acked work or any due agent looks stalled.
  ```
  literal-then-Enter, `timeout=5`s, **Esc-first=NO** (an Esc would self-interrupt
  the monitoring member); any error → `false`. The payload carries no backtick
  and no command-substitution sequence.
- **`send_inline_preview(*, target_pane_id, task_id, sender_id, ts, text) ->
  bool`** — best-effort; the broker's inline-preview path (the broker truncates
  `text` first). tmux missing → `false`; cosmetic CR/LF strip on `text`
  (`\r\n`/`\n`/`\r` each → `⏎` U+23CE, **no** tab/backtick/command-substitution
  sanitization here); two-line payload (single `\n` separator intentionally
  kept):
  ```
  [cafleet msg <task_id> from <sender_id> <ts>]
  <sanitized_text>
  ```
  literal-then-Enter, `timeout=5`s, **Esc-first=YES**, any error → `false`. Under
  `send-keys -l` the `\n` is a soft line break inside one keystroke; the single
  trailing Enter submits the whole 2-line payload as one recipient turn.
- **`send_bash_command(*, target_pane_id, command)`** — fail-fast. Strip
  surrounding whitespace; empty after strip → `send_bash_command: command may not
  be empty`; the **original** command with a newline → `send_bash_command:
  command may not contain newlines`. literal-then-Enter with `payload = "! " +
  normalized_command`, **no Esc-first** (honors the coding-agent `!` shortcut).
- **`capture_pane(*, target_pane_id, lines=20) -> str`** — fail-fast. `lines <=
  0` → `capture_pane: lines must be positive, got <lines>`. Run `tmux
  capture-pane -p -t <target_pane_id> -S -<lines>`, split the raw output on
  `"\n"` **only** (not a general line-splitter — must not also split on `\r`, to
  preserve the CLI's CR-defrag), return the last `lines + 1` elements joined with
  `"\n"` (tmux terminates output with `\n`, so this restores the final newline).
- **`pane_exists(*, target_pane_id) -> bool`** — fail-fast. Whether
  `target_pane_id` is in `list_pane_ids()`.
- **`list_pane_ids() -> set`** — fail-fast. `tmux list-panes -a -F "#{pane_id}"`
  with `timeout=5`s; split on whitespace; return the pane-id set. One call
  resolves liveness for every agent in a monitor tick.
- **`kill_pane(*, target_pane_id, ignore_missing=False)`** — fail-fast. `tmux
  kill-pane -t <target_pane_id>` through the pane-gone-tolerant runner.
- **`wait_for_pane_gone(*, target_pane_id, timeout=15.0, interval=0.5) ->
  bool`** — delegates to `poll_until_pane_gone` with a `pane_exists` closure;
  `true` if the pane disappeared before timeout, `false` on timeout.

#### Fail-fast vs. best-effort split

- **Fail-fast** (surface failures): `ensure_available`, `context_discovery`,
  `split_window`, `select_layout`, `send_exit`, `send_bash_command`,
  `capture_pane`, `pane_exists`, `list_pane_ids`, `kill_pane`,
  `wait_for_pane_gone` (modulo `ignore_missing` pane-gone tolerance on
  `kill_pane` / `send_exit`).
- **Best-effort boolean** (NEVER raise; `false` on any failure):
  `send_poll_trigger`, `send_wake_trigger`, `send_inline_preview`. Each guards
  "tmux missing → `false`" then wraps the keystroke so any error → `false`. The
  boolean is consumed as the broker's `notification_sent` /
  `notifications_sent_count` and the monitor's `woke`.

#### `MultiplexerContext` (frozen value type)

Immutable, three non-nullable string fields, no defaults, constructed only by
`context_discovery`: `session` (tmux session name), `window_id` (e.g. `@N`),
`pane_id` (e.g. `%N`).

#### `poll_until_pane_gone` helper

Backend-generic. Takes a no-arg predicate `pane_exists_fn` (may raise —
propagate), plus `timeout` and `interval` seconds. Using a **monotonic** clock:
compute `deadline = monotonic_now() + timeout`; loop — if `not pane_exists_fn()`
return `true`; if `monotonic_now() >= deadline` return `false`; sleep
`interval`. Checks existence **first** (so `timeout=0` against an already-gone
pane returns `true`), then the deadline, then sleeps.

#### Keystroke core, delays, and the Esc-first matrix

The shared literal-then-Enter primitive (used by `send_exit`,
`send_poll_trigger`, `send_wake_trigger`, `send_inline_preview`, and
`send_bash_command`) takes `target_pane_id`, `payload`, optional
`timeout`, `ignore_missing` (default false), `esc_first` (default false):

1. **If `esc_first`:** run `tmux send-keys -t <target_pane_id> Escape`, then
   sleep `_ESC_SETTLE_DELAY` (`0.1`s). The leading `Escape` dismisses a pending
   permission prompt so the trailing `Enter` cannot blind-confirm it.
2. Run `tmux send-keys -t <target_pane_id> -l <payload>` — `-l` types the literal
   payload (single argv element, never shell-interpolated).
3. Sleep `_SUBMIT_DELAY` (`0.12`s) — **unconditionally**, so codex
   bracketed-paste finalizes and opencode slash-autocomplete settles.
4. Run `tmux send-keys -t <target_pane_id> Enter` — submits.

An embedded `\n` in `payload` is a **soft** newline within the single keystroke
sequence — it does NOT fragment into a second submit. Esc-first matrix:
`send_poll_trigger` **YES**, `send_inline_preview` **YES**, `send_wake_trigger`
**NO**, `send_exit` **NO**, `send_bash_command` **NO**.

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

#### `_sanitize_wake_name` — payload contract

Applied to each user-controlled agent name before interpolation into the
`send_wake_trigger` payload. Replacement chain, **order matters**: `\r\n` → `⏎`
(U+23CE); `\n` → `⏎`; `\r` → `⏎`; `\t` → `⏎`; `` ` `` → `ˋ` (U+02CB); `$(` →
`$﹙` (`$` followed by U+FE59). CR/LF/tab → U+23CE preserves the single-line
guarantee; backtick → U+02CB and `$(` → `$`+U+FE59 preserve the no-backtick /
no-command-substitution guarantee. These are exact Unicode scalar values and are
part of the keystroked payload contract, not cosmetic — distinct from the
CR/LF-only cosmetic strip in `send_inline_preview`.

### 6.6 Monitor heartbeat loop

**Scope:** the in-process supervision scheduler. A coding agent launches
`run_monitor_loop` as a background task; it keeps a fleet's dedicated
*monitoring member* periodically woken so the watcher re-inspects the Director
and ordinary members. The module owns the OS-facing half — the pure due-check,
one scan pass, the foreground driver with signal handling and runtime-row
cleanup, the scan-cadence constant, and the re-export of the four policy
tunables. It performs no DB internals (the broker's) and no multiplexer
internals; it orchestrates calls into both.

#### Public surface

- **`should_ping(target, now) -> bool`** — pure due-check for one watched agent;
  no DB/multiplexer access.
- **`monitor_tick(fleet_id, now) -> Continue | Stop`** — one scan pass.
- **`run_monitor_loop(fleet_id, tick_seconds)`** — foreground driver: claim slot
  → install signal handlers → `tick → sleep` until signalled → clear slot on
  exit.
- **`Continue` / `Stop`** — tick-result markers distinguishing "keep looping"
  from "self-terminate".
- **`DEFAULT_TICK_SECONDS = 5`** — default scan cadence (seconds).
- Re-exports `DIRECTOR_PING_INTERVAL_SECONDS` (180),
  `MEMBER_PING_INTERVAL_SECONDS` (720), `MONITOR_STALE_FACTOR` (3),
  `MONITOR_STALE_FLOOR_SECONDS` (15) — policy tunables whose single home is the
  broker, re-exported so the loop imports policy from one place.

The stop flag, the sleep helper, the signal handler, and the marker type are
implementation-private; only the four functions, the markers, and the five
constants are public.

#### `should_ping(target, now)`

Pure function of one watched-agent scan row (`agent_id`, `name`, `is_director`,
`pane_id` optional, `interval_seconds`, `last_ping_at` optional ISO string,
`enabled`, `pending_count`, `pane_alive`) and a tz-aware UTC `now`. Branch
conditions, in short-circuit order:

1. `enabled` false → false.
2. `pane_id` absent **or** `pane_alive` false → false (unplaced or dead/missing
   pane is always skipped).
3. `last_ping_at` set: `elapsed = (now − parse(last_ping_at))` in float seconds;
   if `elapsed < interval_seconds` → false (not yet due).
4. Otherwise → true. A never-pinged (`last_ping_at` absent) live, enabled agent
   is **immediately due** — the elapsed check is skipped entirely.

`is_director` is **not** consulted (retained only for status labeling);
`pending_count` is **not** consulted (due-ness is interval-driven). The
monitoring member never appears as a `target` — it is the unenrolled watcher.

#### `monitor_tick(fleet_id, now)`

One scan pass, steps in order:

1. **Ownership-checked heartbeat.** Call the broker's heartbeat with `(fleet_id,
   this-pid, now-as-ISO)`. Returns false (zero-row update — this process was
   displaced and another reclaimed the slot) → return `Stop`. This is the
   split-brain loser's exit.
2. **Fleet liveness.** Fetch the fleet; absent **or** `deleted_at` set → return
   `Stop`.
3. **Locate the watcher.** Ask the broker for the fleet's monitoring member (may
   be absent); shape `{agent_id, name, pane_id}`.
4. **Fetch pane liveness once.** A **single** `list_pane_ids` call resolves
   liveness for every agent this tick.
5. **Compute the due set.** For each watched `target` (root Director + ordinary
   members; never the monitoring member): set `target.pane_alive = (target.pane_id
   ∈ live_panes)`, then if `should_ping(target, now)` add it to the due list.
6. **Wake the watcher iff due and watcher live.** If the due list is non-empty
   **and** the watcher is present **and** its `pane_id` is in the live set: call
   the multiplexer's wake trigger against the watcher's own pane (the loop's
   **only** keystroke), passing the due agents and the fleet's
   `director_agent_id`; it returns a boolean `woke`.
   - If `woke` is true: call the broker's `record_pings` with the due ids and
     `now-as-ISO` (advancing each due agent's `last_ping_at` **only** on a
     successful wake, so a just-flagged agent is not due again next tick), and
     emit one stdout heartbeat line per due agent with this **exact** format:
     ```
     {now.isoformat()} due agent {agent_id} ({name}) -> wake monitor
     ```
     `name` is emitted **raw** (sanitization applies only to the keystroke
     payload).
   - If `woke` is false: do **not** record pings and do **not** echo — the due
     agents stay flagged, so the next tick retries (no wake-storm, no silent
     skip).
   - No live watcher to wake: nothing is recorded.
7. Return `Continue`.

**Critical ordering invariant:** `record_pings` and the heartbeat echo are gated
behind `woke == true`. Preserve this gating exactly.

#### `run_monitor_loop(fleet_id, tick_seconds)`

Foreground driver. The fleet's monitor-runtime row is the **only** coordination
artifact (no PID file); identity throughout is the OS process id.

1. Reset the shared stop flag to false; capture `pid = this-pid`.
2. **Claim the slot** via the broker's atomic claim `(fleet_id, pid,
   tick_seconds, now-as-ISO)`. On refusal (returns false) → application error
   (exit 1) `monitor already running for fleet {fleet_id}`. There is no silent
   fallback.
3. **Install signal handlers** for SIGTERM and SIGINT; each flips the shared stop
   flag to true (the handler is minimal — just a flag flip).
4. **Loop** while the stop flag is false: if `monitor_tick(fleet_id, now)` (each
   pass stamps `now` fresh as tz-aware UTC) returns `Stop` → break; else call
   `interruptible_sleep(tick_seconds)`.
5. **Cleanup (always, in a finally block):** the broker's ownership-checked clear
   `(fleet_id, pid)` — nulls the slot's process fields only if this pid still
   owns the slot, so a displaced loser's clear is a no-op.

**Stop paths:** (a) a signal sets the stop flag → loop exits → finally clears;
(b) `monitor_tick` returns `Stop` → break → finally clears; (c) a hard kill runs
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
the opencode bundled-agent preset materialization and the `CODING_AGENTS`
registry.

#### Interface

A `CodingAgent` is a stateless backend object selected per member at spawn time.
Each exposes two read-only properties and three methods:

- **`name`** — the backend's registry key, a stable lowercase string (`"claude"`
  / `"codex"` / `"opencode"`). This MUST equal both the persisted
  `placement.coding_agent` value used to look the backend up and the key under
  which it is registered.
- **`binary_name`** — the executable resolved against `PATH` (`"claude"` /
  `"codex"` / `"opencode"`).
- **`ensure_available()`** — raises if any spawn precondition is unmet; MAY
  materialize on-disk config as a side effect (opencode does). A shared helper
  resolves `binary_name` against `PATH` and, on a miss, raises `binary
  {binary_name} not found on PATH`.
- **`validate_model(model)`** — `model` is optional; raises a value-error if
  malformed for this backend; a `None` model is always valid. **Exit-code note:**
  `agent spawn` translates this value-error to a **usage error (exit 2)** with
  the backend's message (§6.3). This is distinct from the broker/messaging
  value-errors of §7.2, which the CLI wraps to **exit 1** — do **not** route a
  `validate_model` failure through the generic value-error→exit-1 path.
- **`build_spawn_argv(prompt, display_name, model)`** — returns the full argv
  vector (binary + flags + prompt) for the multiplexer's window-split.

**Ordering invariant:** the consumer (`agent spawn`) MUST call them in the
order **`validate_model` → `ensure_available` → `build_spawn_argv`**, so a
malformed model fails before any disk write (opencode preset) or registration
side effect.

**No-model byte-identity:** when `model` is `None`, `build_spawn_argv` emits
**no** `--model` tokens at all — the argv is identical to the no-model form.
Never emit an empty `--model ""`.

#### Registry resolution

A single module-level registry maps backend name → backend singleton, eagerly
constructed, with exactly three entries: `"claude"`, `"codex"`, `"opencode"`.
Resolution is a direct lookup keyed by the persisted `placement.coding_agent`
value — no fuzzy matching, no default fallback; an unknown name has no entry.
Each entry's key equals that backend's own `name` property.

#### Per-backend `build_spawn_argv` (exact, token-by-token)

**claude** — `validate_model` pass-through (accepts any string; the binary
validates). `ensure_available` PATH check on `claude` only. claude is the
**only** backend that honors `display_name` (via `--name`).

```
["claude", "--permission-mode", "dontAsk", "--name", <display_name>]
  (+ ["--model", <model>]  if model is not None)
  (+ <prompt>)                                       # bare trailing positional
```

**codex** — `validate_model` pass-through. `ensure_available` PATH check on
`codex` only. `display_name` is silently ignored.

```
["codex", "--ask-for-approval", "never", "--sandbox", "workspace-write"]
  (+ ["--model", <model>]  if model is not None)
  (+ <prompt>)                                       # bare trailing positional
```

**opencode** — `validate_model`: `None` is valid; otherwise split `model` on the
**first** `/` into `<provider-id>` and `<model-id>`, both halves MUST be
non-empty, else value-error `--model for the opencode backend must be
'<provider-id>/<model-id>' (got '{model}').`. (`"openai/gpt-4"` accepted;
`"a/b/c"` accepted as provider `a` / model `b/c`; `"a/"`, `"/b"`, `"abc"`
rejected.) `ensure_available` PATH check on `opencode` **first**, then
materialize the preset. `display_name` is silently ignored; the prompt is passed
as a `--prompt <prompt>` flag pair (two tokens), unlike claude/codex's bare
positional.

```
["opencode", "--agent", "cafleet"]
  (+ ["--model", <model>]  if model is not None)
  (+ ["--prompt", <prompt>])                         # prompt via flag — TWO tokens
```

#### opencode preset materialization

`ensure_available` for opencode writes the bundled `cafleet` agent definition to
`~/.opencode/agents/cafleet.md` (expanding `~`). **Two opencode base directories
serve two distinct purposes** and are not interchangeable: the agent preset lives
under `~/.opencode/`, which is opencode's mandated `--agent cafleet` discovery
path; `setup`'s skills install + home auto-detection (§6.3) use
`~/.config/opencode/`, cafleet's own skills-install / home-detection target. Both
paths are correct for their purpose — keep each as written. The writer is an
idempotent skip-if-exists guard with a refuse-to-overwrite branch; resolve the
target, then branch in this exact order:

1. **Target is a regular file** (following symlinks — a symlink to a regular file
   counts here): **return, no-op** (skips dotfile-managed customizations).
2. **Otherwise the target exists** but is not a regular file / symlink-to-regular
   (a directory, a broken symlink, a symlink to a non-file): **raise**, refusing
   to overwrite, with `cannot materialize CAFleet opencode agent preset:
   {target} exists but is not a regular file or symlink to one (e.g. directory,
   broken symlink, symlink to a non-file); refusing to overwrite`.
3. **Otherwise** (does not exist): create the parent directory chain recursively
   (no error if present), then write the rendered markdown as UTF-8. Any
   directory-create or write error is wrapped with the prefix `cannot
   materialize CAFleet opencode agent preset at {target}: ` (the prefix up to and
   including `: ` is the contract; the trailing OS message is platform-
   dependent).

The branch order is load-bearing: the regular-file skip is checked before the
exists-but-not-regular refusal.

#### Preset file rendering rules

The definition has four fields — `description` (string), `mode` (`primary`),
`permission` (a ruleset), `body` (markdown) — rendered as a `---`-delimited
**JSON** (not YAML) frontmatter block, then a blank line, then the body, then a
trailing newline. JSON formatting rules (all load-bearing for byte-identical
output): JSON not YAML; **2-space indent**; **non-ASCII preserved** (never
`\uXXXX`); **insertion-ordered maps** (top-level key order `description`, `mode`,
`permission`; within `permission`, field order `bash`, `read`, `edit`,
`external_directory`, `webfetch`, `websearch`, `repo_clone`, `question`,
`plan_enter`, `plan_exit`; within each glob map, entry order exactly as below).
`bash`/`read`/`edit` are glob→decision maps (`"allow"`/`"deny"`); the other
seven `permission` fields are scalar `"deny"`.

#### Exact preset file contents (verbatim)

Reproduce this file faithfully (the body contains literal backticks around
`dontAsk` and `.env`):

````markdown
---
{
  "description": "CAFleet-spawned member with workspace-scoped permission floor; matches Claude Code dontAsk safety posture.",
  "mode": "primary",
  "permission": {
    "bash": {
      "*": "allow",
      "bash -c*": "deny",
      "sh -c*": "deny",
      "zsh -c*": "deny",
      "python -c*": "deny",
      "python3 -c*": "deny",
      "perl -e*": "deny",
      "node -e*": "deny",
      "node --eval*": "deny",
      "ruby -e*": "deny",
      "eval*": "deny",
      "exec*": "deny",
      "rm -rf*": "deny",
      "sudo*": "deny",
      "git push*": "deny",
      "git reset --hard*": "deny",
      "chmod*": "deny",
      "chown*": "deny",
      "curl*": "deny",
      "wget*": "deny",
      "nc*": "deny",
      "ssh*": "deny",
      "scp*": "deny",
      "rsync*": "deny",
      "osascript*": "deny"
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

You are a CAFleet member spawned by the Director. The bash, read, and edit permission rulesets in your frontmatter enforce a workspace-scoped safety floor that mirrors Claude Code's `dontAsk` posture: dangerous shell-indirection wrappers, destructive operations, network egress utilities, and `.env` files are denied; everything else is allowed without user prompts. Refer to your Director's spawn-prompt instructions for the task.
````

The body is a single physical paragraph (no internal hard line breaks after the
heading and its blank line); the file ends with exactly one trailing newline.

#### Contract error strings

- PATH miss: `binary {binary_name} not found on PATH`
- opencode model format: `--model for the opencode backend must be
  '<provider-id>/<model-id>' (got '{model}').`
- preset non-regular-file collision: `cannot materialize CAFleet opencode agent
  preset: {target} exists but is not a regular file or symlink to one (e.g.
  directory, broken symlink, symlink to a non-file); refusing to overwrite`
- preset write failure (prefix is the contract; OS-message suffix is
  platform-dependent): `cannot materialize CAFleet opencode agent preset at
  {target}: {error}`

### 6.8 WebUI + Config

**Scope (two concerns):** (a) the HTTP app factory `create_app`, the `/api/*`
router, the `X-Fleet-Id` header dependency, the SPA-fallback static server, and
the `cafleet server` launcher; (b) the global `Settings` singleton from the
`CAFLEET_*` env block — consumed CLI-wide, not webui-local. The reference builds
a specific HTTP framework's app; the contract below is stack-neutral. The config
env-var table is §7.1.

#### App factory (`create_app`)

Takes an optional explicit "WebUI dist directory" argument and returns the
configured HTTP application:

1. Constructs an app titled `CAFleet Admin`, version `0.1.0`. **This `0.1.0` is a
   hardcoded literal, independent of the CLI's package version.** The CLI
   `--version` output and `setup`'s skills-release tag (§6.3) read the **installed
   package version** dynamically; the WebUI app-version string does not track it.
   Keep them decoupled.
2. **Registers the `/api/*` router before mounting the static file server.**
   This ordering is load-bearing: unmatched `/api/*` paths must produce a JSON
   404 from the router, never be swallowed by the SPA fallback.
3. The "not built" warning is enabled only when the caller passed no explicit
   dist directory (the default-dir branch); an explicit directory **suppresses**
   it. With no directory given, it resolves the default dist dir (`<webui module
   dir>/dist`).
4. If the warning is enabled **and** the resolved dist path does not exist, print
   this exact one-line text to **stderr** (one time, at factory call):
   ```
   warning: admin WebUI is not built. / will return 404. Run 'mise //admin:build'.
   ```
5. If the dist path exists, mount the SPA static file server at `/`, named
   `webui`. If it does not exist, no mount is added: `/` and every non-API path
   404, while `/api/*` still works.

A module-level app singleton is created by calling `create_app()` with no
argument (the server target); because it uses the default dir, it emits the
stderr warning when the SPA is unbuilt.

#### SPA static file server

Wraps a directory and a reserved-prefix set `("ui", "api")`. Delegates to the
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
nothing but **drops** `agent_id`. `GET /api/fleets` returns a **bare JSON
array**; every other list endpoint wraps in an object (agent listings under
`agents`, message listings under `messages`). All HTTP errors serialize as
`{"detail": <string>}`; body-validation failures use the framework's default
`422` validation-error body instead.

#### The 9 routes

- **`GET /api/fleets`** — unscoped (no `X-Fleet-Id`). Returns the broker fleet
  list **directly as a bare array**.
- **`GET /api/agents`** — fleet-scoped. For each agent, sets a `monitor` field to
  the projected monitor config when an enrolled config exists, else `null`.
  Response `{"agents": [ <agent dict> + "monitor": <MonitorConfig>|null, … ]}`.
  Projected `MonitorConfig`: `{interval_seconds, last_ping_at, enabled}`
  (`agent_id` dropped).
- **`GET /api/monitor`** — fleet-scoped. Returns `{running, pid, tick_seconds,
  last_tick_at, last_tick_age_seconds, started_at}`. Read the runtime row and
  the live-check (current UTC). If absent **or** not live: `running=false`,
  `pid=null`, `tick_seconds` = the row's value when a row exists else `null`,
  `last_tick_at`/`last_tick_age_seconds`/`started_at` all `null` — **a stale row
  never leaks a lingering pid or start time**. When live: `running=true` with the
  live `pid`, `tick_seconds`, `last_tick_at`, `started_at`, and a computed
  `last_tick_age_seconds` (null when `last_tick_at` is null; else whole-seconds
  now − parsed `last_tick_at`, **integer-truncated**).
- **`GET /api/agents/{agent_id}/monitor`** — fleet-scoped. Absent config → `404`,
  detail `Agent not enrolled`; else the projected `MonitorConfig` (single
  object).
- **`PATCH /api/agents/{agent_id}/monitor`** — fleet-scoped. Body
  `{interval_seconds?: int, enabled?: bool}` (both optional). A present
  `interval_seconds` must be **≥ 1**; `< 1` → `422` (framework default). A
  `null`/`null` patch is a valid no-op. Pre-check the config; absent → `404`,
  detail `Agent not enrolled`. Then update; if the agent was deregistered between
  the pre-check and the update (TOCTOU), the raised error is caught and
  **collapsed to `404` detail `Agent not enrolled`** (not 500). Returns the
  projected updated config.
- **`GET /api/agents/{agent_id}/inbox`** — fleet-scoped. Agent not in fleet →
  `404`, detail `Agent not found`; else `{"messages": [ <FormattedMessage>, …
  ]}` over the agent's inbox.
- **`GET /api/agents/{agent_id}/sent`** — fleet-scoped. Same as inbox over sent
  messages; same `404` detail `Agent not found`.
- **`GET /api/timeline`** — fleet-scoped, no per-agent check. `{"messages": […]}`
  over all of the fleet's messages.
- **`POST /api/messages/send`** — fleet-scoped. Body `{from_agent_id: int,
  to_agent_id: int | "*", text: string}`. `to_agent_id` deserializes as **either
  a JSON integer or the exact JSON string `"*"`** (broadcast); anything else
  (e.g. a stringified integer `"5"`) is rejected, not coerced. If `from_agent_id`
  is not in the fleet → `400`, detail `from_agent not in fleet`. If `"*"`:
  broadcast, return `{task_id: <summary task_id>, status: <summary
  status_state>}`. Else: recipient not in fleet → `404`, detail `Agent not
  found`; otherwise send and return `{task_id, status}`. Both branches:
  `{task_id: int, status: string}` (`status` = the broker task's `status_state`).

**`FormattedMessage`** (one element of any `messages` array): `{task_id,
from_agent_id, from_agent_name, to_agent_id, to_agent_name, type, status,
created_at, status_timestamp, origin_task_id, body}`. Names are resolved by a
single bulk lookup over the union of all `from_agent_id`/`to_agent_id` values,
using **direct keyed access** — a missing id is a hard failure (→ 500), never a
silent fallback. `status` is the renamed `status_state`; `body` the renamed
`text`; `type` the raw row type. Empty input → empty array.

#### `cafleet server` launcher

Runs the WebUI app under an HTTP server. `--host` (default `settings.broker_host`)
and `--port` (default `settings.broker_port`, integer) both read their defaults
from settings at command-definition time and are shown in `--help`. Serves the
app singleton in a single process **with no auto-reload**. Because the defaults
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
   `agent_id` dropped from the monitor projection.
5. **`list_fleets` returns a bare array**; every other list wraps in
   `{"agents"|"messages": [...]}`.
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
| `database_url` | `CAFLEET_DATABASE_URL` | string | `sqlite:///` + `~/.local/share/cafleet/cafleet.db` (home expanded **at startup**) |
| `broker_host` | `CAFLEET_BROKER_HOST` | string | `"127.0.0.1"` |
| `broker_port` | `CAFLEET_BROKER_PORT` | integer (16-bit port) | `8000` |
| `max_text_len` | `CAFLEET_MAX_TEXT_LEN` | non-negative integer | `200` |

- **Default DB URL** expands `~` to `$HOME` **only for the factory default**; a
  user-supplied `CAFLEET_DATABASE_URL` is passed through verbatim (no `~`
  expansion, so a user value must already be absolute). Net default on home
  `/home/u`: `sqlite:////home/u/.local/share/cafleet/cafleet.db` (four slashes).
- A non-integer `broker_port`/`max_text_len` must **fail loudly at startup** (a
  hard validation error, not a silent default).
- `max_text_len` truncates only CLI echo + the broker inline-preview keystroke.
  It is **never** applied by the WebUI API (raw broker results) and never
  truncates the persisted `Task.text` column.

**Command/identity env vars (not Settings-singleton fields).** These do not bind
to the `Settings` singleton; they are read at parse time or by the spawned agent:

| Env var | Type | Role |
|---|---|---|
| `CAFLEET_FLEET_ID` | integer | Default for the `--fleet-id` option on every fleet-scoped command (§3). An explicit `--fleet-id` overrides it; a non-integer value fails at parse time (exit 2). |
| `CAFLEET_AGENT_ID` | integer | The spawned agent's own id. **Not** bound to any flag default (the `--agent-id` polarity is overloaded, §1); the agent reads it and passes it explicitly. |
| `CAFLEET_DIRECTOR_AGENT_ID` | integer | The spawned agent's Director id. Same read-then-pass convention. |

- All four of `CAFLEET_DATABASE_URL`, `CAFLEET_FLEET_ID`, `CAFLEET_AGENT_ID`, and
  `CAFLEET_DIRECTOR_AGENT_ID` are **forwarded into spawned panes** by `agent
  spawn` (`split_window env={…}`; `CAFLEET_DATABASE_URL` only when set). These
  env vars are the identity-delivery mechanism for spawned agents (§6.3),
  complementing `agent spawn`'s `{fleet_id}` / `{agent_id}` /
  `{director_agent_id}` / `{coding_agent}` placeholder substitution (§6.3).

### 7.2 Error handling & exit-code policy

A unified, two-tier error model maps the exception taxonomy to exit codes. Use
per-module error types that carry an **exit-code class**, and a single top-level
printer that writes `Error: <message>` to stderr.

| Error class | Exit | Meaning | Mapping |
|---|---|---|---|
| usage error | **2** | argument/parse/usage mistakes: missing required option (now **including `--fleet-id`**); unknown option; invalid integer; integer-range violations; mutually-exclusive-option violations; explicit usage errors | a usage-class error; prints `Error: <msg>` (+ usage line). Parser-native parse errors already exit 2. |
| application error | **1** | application/runtime errors: runtime conflicts (one-monitor rule, Administrator immutability, not-enrolled, not-found-on-delete), the root-Director-deregistration guard, the spawn rollback ladder, and the pane-teardown timeout | an app-class error; prints `Error: <msg>`. |
| value-error / permission-error (broker/messaging/queries) | translated by caller | callable from CLI **and** WebUI; CLI wraps to exit 1, WebUI maps to HTTP status | distinct error variants; permission-error gates authorization (recipient-acks / sender-cancels). |
| HTTP error | — | serialized `{"detail": <string>}` | HTTP error responses with the same status + body. |

There is no per-path exit-code split: the root-Director-deregistration guard
raises a single **application error (exit 1)** on both the broker side and the
`agent deregister` CLI side, and the pane-teardown timeout is an **application
error (exit 1)** (runtime → exit 1), not an exit-2 path.

**Fail-fast points (never silently fall back):**

- `--fleet-id` is a parser-native required option (missing → exit 2); it
  **defaults from `CAFLEET_FLEET_ID` when set** but **must not** default to an
  arbitrary fleet otherwise.
- `client_command` fleet-gate runs **before** the handler body.
- `doctor` reads the `TMUX_PANE` environment variable by direct access — an
  error if unset is intentional.
- `is_administrator`/`is_monitoring_member` return false on malformed JSON (a
  deliberate non-match).
- `register_agent` monitoring-member-without-placement raises.
- The opencode preset refuses to overwrite a non-regular-file target.
- Broker "exactly one row" invariants raise if the assumption breaks — keep
  fail-loud.

Exact error strings are catalogued per module in §6; the **strings** are part of
the contract and should be reproduced (the relaxation in §1 concerns incidental
formatting artifacts, not these deliberate user-facing messages). Notable
cross-module string — the "must be run inside a tmux session" text exists in two
distinct forms with **two different provenances**:

- **Tmux-pane-command path** — the multiplexer's `ensure_available` raises
  `cafleet tmux-pane commands must be run inside a tmux session` (§6.5); the CLI
  surfaces that text as-is (it does not hardcode it).
- **`fleet create` path** — the CLI **catches** the multiplexer's `TmuxError`,
  **discards** its message, and raises its own hardcoded command-specific string
  `cafleet fleet create must be run inside a tmux session` (§6.3, exit 1). This
  one is genuinely CLI-hardcoded; do not expect it to echo the multiplexer's
  `tmux-pane commands` wording.

### 7.3 Output / JSON / truncation

Output formatting is specified in §6.4. The cross-cutting choices: the CLI selects
text-vs-JSON (global `--json`, or a local `--json` OR-ed with the global one in
`fleet *`) and full-vs-compact (documented `--full`); the WebUI bypasses `truncate_*`
(raw broker results) but its JSON serialization still preserves key order and raw
UTF-8 (no ASCII escaping).

### 7.4 Logging & stdout discipline

- The monitor loop emits per-due-agent heartbeat lines to **stdout**
  (`{iso} due agent {id} ({name}) -> wake monitor`), `name` raw (unsanitized).
- The "WebUI not built" warning, `agent spawn`/`agent deregister` rollback and
  timeout diagnostics, and `monitor start`'s "no monitoring member" warning all
  go to **stderr**. Preserve the stream choice (stdout vs. stderr) — it is part
  of the observable contract.

### 7.5 Time discipline

Every "now" is timezone-aware UTC; every DB-boundary write serializes to the
canonical ISO-8601 string. See §5.1 — string comparison for ordering, parse only
for age math.

---

## 8. Database schema

**Schema** = the six tables of §5.2, created by **one baseline `CREATE`** that
yields the final schema directly. There is no migration chain, no version table,
and no in-place upgrade path; existing databases are not migrated (§11). Column
types, defaults, FK rules, AUTOINCREMENT, and the create-order quirk are in §6.1.

**Indexes (non-unique):**

- `idx_agents_fleet_status` on `agents(fleet_id, status)`
- `idx_placements_director` on `agent_placements(director_agent_id)`
- `idx_tasks_context_status_ts` on `tasks(context_id, status_timestamp)`
- `idx_tasks_from_agent_status_ts` on `tasks(from_agent_id, status_timestamp)`

**The single baseline `CREATE`.** A fresh database is created empty, in one
ordered sequence (using `CREATE TABLE IF NOT EXISTS` / `CREATE INDEX IF NOT
EXISTS`, so re-running `setup` against an already-initialized DB creates nothing
new). The order honors the forward-reference quirk (§6.1):

1. `agents` (+ `idx_agents_fleet_status`) — **created first** because the others
   FK into it; `agents.fleet_id` forward-references the still-uncreated `fleets`.
   AUTOINCREMENT.
2. `fleets` (AUTOINCREMENT).
3. `tasks` (+ `idx_tasks_context_status_ts`, `idx_tasks_from_agent_status_ts`) —
   AUTOINCREMENT; `to_agent_id` nullable, no FK (§5.5).
4. `agent_placements` (+ `idx_placements_director`) — PK=FK `agent_id`, not
   AUTOINCREMENT; `coding_agent` DDL default `"claude"`.
5. `monitor_config` — PK=FK `agent_id` ON DELETE CASCADE, `interval_seconds`
   default 60, `enabled` default 1; not AUTOINCREMENT.
6. `monitor_runtime` — PK=FK `fleet_id` ON DELETE RESTRICT, `tick_seconds`
   default 5; not AUTOINCREMENT.

A fresh DB starts with **no rows in any table**; monitor enrollment is written at
runtime (the Director at 180s by `create_fleet`, pane-bound members at 720s by
`register_agent`, §6.2), never seeded by the schema.

**`setup` schema-create driver** (the DB half of `setup`, §6.3). Procedure: (1)
derive a sync SQLite URL by forcing the drivername to `sqlite`; (2) extract the
DB file path — if empty → application error `database URL has no file path`; (3)
create the file's parent directory; (4) create the baseline schema (the `IF NOT
EXISTS` sequence above). The driver's engine is disposed when the command
finishes (success or failure). On success it prints `Created {db_file} and
initialized the cafleet schema.`.

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
    validation, and the preset markdown structure; the materializer's
    skip/refuse/write branches against a temp HOME.
  - *Monitor* — `should_ping` is pure (table-test interval/enabled/pane states);
    `monitor_tick` against a fake broker+multiplexer asserting the `woke`-gated
    `record_pings` and the `STOP` paths.
  - *Config* — env-var parsing, the default-URL home expansion, and loud failure
    on non-integer port/len.
- **Integration:**
  - End-to-end DB lifecycle: `create_fleet → register_agent → send_message →
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

The full command surface — **24 commands across 5 groups + 3 top-level commands**.
Each must be reproduced with identical option names, types, defaults,
required-ness, documented-vs-hidden status, output shapes, and exit codes. Every
interaction flag is now **documented** (there are no hidden flags). Per-command
option semantics are in §6.3.

**Global:** `--json` (before subcommand), `--version` (`cafleet <version>`,
exit 0, bypasses `--fleet-id`).

**Top-level:**

- [ ] `cafleet setup` (`--agent` multiple: claude/codex/opencode)
- [ ] `cafleet doctor` (global `--json` only)
- [ ] `cafleet server` (`--host`=settings.broker_host, `--port`=settings.broker_port)

**`fleet`:**

- [ ] `cafleet fleet create` (`--label`, `--coding-agent`=claude, `--json`, `--full`)
- [ ] `cafleet fleet list` (`--json`)
- [ ] `cafleet fleet show` (`--fleet-id`, `--json`)
- [ ] `cafleet fleet delete` (`--fleet-id`)

**`agent`:**

- [ ] `cafleet agent register` (`--name`, `--description`, `--skills`)
- [ ] `cafleet agent list` (`--full`, `--activity`)
- [ ] `cafleet agent show` (`--agent-id` requester, `--id` target, `--full`)
- [ ] `cafleet agent deregister` (`--agent-id` target, `--force`/`-f`)
- [ ] `cafleet agent spawn` (`--agent-id` Director, `--name`, `--description`, `--coding-agent`, `--model`, `--role`=member, `--text` / `--text-file` xor-required, `--full`)

**`pane`:**

- [ ] `cafleet pane capture` (`--agent-id`, `--lines`=**20**, `--ansi`/`--no-ansi`)
- [ ] `cafleet pane exec` (`--agent-id`, positional `command`)
- [ ] `cafleet pane wake` (`--agent-id` target, exactly one of `--poll-only` / `--message`; `--message` requires `--from` sender + `--text` / `--text-file` xor-required)

**`message`:**

- [ ] `cafleet message send` (`--agent-id` requester, `--to`, `--text` / `--text-file` xor-required, `--full`)
- [ ] `cafleet message broadcast` (`--agent-id`, `--text` / `--text-file` xor-required, `--full`)
- [ ] `cafleet message poll` (`--agent-id`, `--full`)
- [ ] `cafleet message ack` (`--agent-id`, `--task-id`, `--full`)
- [ ] `cafleet message cancel` (`--agent-id`, `--task-id`, `--full`)
- [ ] `cafleet message show` (`--agent-id`, `--task-id`, `--full`)

**`monitor`:**

- [ ] `cafleet monitor start` (`--tick`≥1=5)
- [ ] `cafleet monitor status`
- [ ] `cafleet monitor config` (`--agent-id`, `--interval`≥1, `--enable`/`--disable`)

Every `agent *`, `pane *`, `message *`, and `monitor *` command, plus `fleet
show` and `fleet delete`, takes the **required `--fleet-id` option** (integer),
defaulting from `CAFLEET_FLEET_ID` when set; a missing `--fleet-id` is a
parser-native missing-required-option error (exit 2, §6.3). It is omitted from the
per-command rows above to avoid repetition. `setup`, `doctor`, `server`, `fleet
create`, and `fleet list` do **not** take `--fleet-id`.

---

## 11. Decisions & clarifications

### Architecture

The concurrency model is an implementation choice (§2). The only requirement is
that the monitor's "SQLite write lock serializes claims" assumption (§6.2) is
preserved.

### Output fidelity

Fidelity is structural and semantic, not byte-for-byte (§1). The host-language
artifacts that need only preserve *intent* (not exact bytes): the `repr()`-style
quoting in `pane exec` echo, the OS-error message suffix in a preset-write
failure, and an exception's exact internal-repr fragment.

### Surface-redesign decisions

The decisions that shape this surface (full rationale in the design doc):

- **`member` splits into `agent` + `pane`.** Registry + lifecycle live under
  `agent` (`register`/`list`/`show`/`deregister`/`spawn`); keystroke interaction
  lives under `pane` (`capture`/`exec`/`wake`). `pane wake` carries the
  `--poll-only` / `--message` modes.
- **`--fleet-id` is a plain required option**, defaulting from `CAFLEET_FLEET_ID`
  (§6.3/§7.1); a missing value is a parser-native exit-2 error.
- **One error/exit model** (§7.2): usage → exit 2, application/runtime → exit
  1; no per-path exit-code split.
- **Single-baseline schema** (§8): one `CREATE`, no migration chain, no `db init`,
  no DB interoperability.
- **Nullable `to_agent_id`** (§5.5): `NULL` on `broadcast_summary` rows; no `0`
  sentinel.
- **Env-var context injection** (§6.3/§7.1): identity reaches a spawned pane via
  `CAFLEET_FLEET_ID` / `CAFLEET_AGENT_ID` / `CAFLEET_DIRECTOR_AGENT_ID`; the spawn
  prompt additionally supports `{fleet_id}` / `{agent_id}` /
  `{director_agent_id}` / `{coding_agent}` placeholder substitution (§6.3).
- **Single absent glyph** (§6.4): ASCII `-` everywhere.
- **`--json` dual path retained** (§7.3): the `agent`/`message` global path and
  the `fleet` local-flag path are not unified.

### Per-module clarifications

Choices left unconstrained by the contract (each underlying behavior is fully
specified in the cited section):

- **CLI (§6.3):** the `--coding-agent`/`--role`/`--agent` choice sets may be
  hardcoded to `claude`/`codex`/`opencode` or data-driven off the registry — an
  implementation choice.
- **Multiplexer (§6.5):** `env` argument ordering in `split_window` is not
  behaviorally significant (tmux treats `-e` flags as a set).
- **Coding agents (§6.7):** the backend registry may be a name→backend map or a
  backend enum — an implementation choice.

### Cross-module consistency notes

- **Timestamps** unified in §5.1 (string storage + comparison; parse for math).
- **Agent kind** unified in §5.4 (three distinct representations, not one enum).
- **`enabled`** stored INTEGER 0/1, exposed as boolean at the broker boundary
  (§6.1/§6.2/§6.6/§6.8).
- **Policy tunables** (180/720/3/15) have a single home in the broker module,
  re-exported by the monitor module.
- **`settings` singleton** is config-module-owned and reachable from every
  module, not webui-local.
