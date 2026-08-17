# Data model

The `Message` payload is fully relational: every routing field plus the message
body lives in its own typed column. The only JSON `TEXT` blob is
`members.member_card_json`. The database is SQLite, accessed synchronously
and bundled into the binary; the schema is managed by a chain of SQL
migrations embedded in the binary — run `cafleet setup` to migrate to head
(idempotent, data-preserving; see [Storage](../concepts/storage.md)). The exact column-level DDL contract lives
in the repository's `SPEC.md`.

Minted ids are **never reused** and real ids are always `>= 1`.

## Tables

| Table | Primary key | Parent | FK ON DELETE | Row removal |
|---|---|---|---|---|
| `fleets` | `INTEGER PRIMARY KEY AUTOINCREMENT` | `members.member_id`, via the nullable `director_member_id` back-reference | `RESTRICT` | Soft-delete keyed on `deleted_at` |
| `members` | `INTEGER PRIMARY KEY AUTOINCREMENT` | `fleets.fleet_id` | `RESTRICT` | Soft-delete (`status='deregistered'` + `deregistered_at`) |
| `messages` | `INTEGER PRIMARY KEY AUTOINCREMENT` | `members.member_id`, via `owner_member_id` | `RESTRICT` | Not deleted |
| `member_placements` | Reuses `members.member_id` | `members` | `CASCADE` | Hard-deleted on deregistration |
| `monitor_runtime` | Reuses `fleets.fleet_id` | `fleets` | `RESTRICT` | Removed inside the `fleet delete` transaction; "no monitor" is modeled as "no row" |
| `asset_installs` | `(coding_agent, path)` composite | — | — | Upserted, one row per coding agent and install path |

### `fleets`

`cafleet fleet create` writes the fleet row, the root Director (and its
placement), the `director_member_id` back-reference, and the monitor member
(its row, its monitor card marker, and — after the pane spawn — its
placement) in one all-or-nothing transaction — which is why
`director_member_id` is DB-nullable despite the post-bootstrap NOT NULL
invariant. The pane spawn happens **inside** the transaction, between the
monitor registration and its placement insert, so any failure (spawn
included) unwinds every row; a pane spawned before a placement-insert or
commit failure is killed by the CLI error path. The flip side is the
write-lock window: the connection holds SQLite's write lock across the
pane-spawn subprocess call, so a concurrent cafleet writer on the shared
database blocks for the duration of the multiplexer call, backstopped by
the connection's `busy_timeout=5000` PRAGMA — an accepted trade-off of the
literally-atomic bootstrap.

### `members`

Active query paths filter `status='active'`. A member's `kind` (`director` /
`monitor` / `member`) is derived at read time from the fleet's
`director_member_id` back-reference plus the member card: a monitor-member
registration writes the application-level marker
`"cafleet": {"kind": "monitor"}` into `member_card_json`, while the Director
and ordinary members write no `$.cafleet` object. The marker is plain JSON —
no schema migration backs it, and no dedicated column exists.

### `messages`

One row per delivery: a unicast row, a broadcast delivery row, or a broadcast
summary row (see [Broadcast grouping](#broadcast-grouping)). `from_member_id`,
`to_member_id`, and `origin_message_id` are deliberately not foreign keys —
historical messages may outlive their sender. `status_timestamp` is updated on
every state change and drives `ORDER BY DESC` listing. The rendered envelope is specified in
[Message envelope](message-envelope.md).

### `member_placements`

Links a member to its multiplexer pane; pane ids are stored verbatim as opaque
strings. The root Director keeps its own placement row (it is pane-bound); an
ordinary member is a placed row other than the fleet's root Director
(`member_id != fleets.director_member_id`). Placement rows have no historical
value.

### `monitor_runtime`

`monitor_runtime` is the one-row-per-fleet loop pid/heartbeat table: the
single-instance claim (`pid`, `started_at`), the liveness heartbeat
(`last_tick_at`, `tick_seconds`), `last_wake_at` — the nullable UTC ISO
timestamp of the last successfully delivered wake, kept durable
across loop restarts so an immediate restart honors the remaining wake
cadence — and `wake_interval_seconds`, the nullable live mirror of the
running loop's wake interval: stamped with the startup-resolved
value at every `cafleet monitor` start (claim and reclaim), re-read by the loop on
every tick, overwritten by `PATCH /api/monitor`, and preserved across a
loop stop like `tick_seconds`. It is `NULL` only in rows that predate the
column and have not been re-claimed since — a running loop's row is always
stamped. The row also carries `wake_requested_at`, the nullable UTC ISO
timestamp of the latest pending forced-wake request
(`POST /api/monitor/wake`): `NULL` when no request is pending, and repeat
requests overwrite the timestamp, coalescing into a single wake. Exactly two
writes clear it: a delivered wake — scheduled or forced — stamps
`last_wake_at` and clears the request in the same write, and a new loop
instance's reclaim resets it, so a pending request never survives into a
later loop instance. The cadence semantics are defined in
[Monitoring](../concepts/monitoring.md#cadence-and-tick-precision).

### `asset_installs`

One upserted row per `(coding_agent, path)`, recording the CLI version whose
skills and preset (where one exists) install last landed at that path — the
row attests both. `path` is the agent's resolved identity path (see
[CLI options](cli-options.md#config-dir-resolution)), stored absolute exactly
as resolved. Consumers partition an agent's rows by comparing `path` against
the currently-resolved identity path: the row at the resolved path (at most
one, by the primary key) is **current**; every other row of that agent is
**superseded**. Written by the assets half of `cafleet setup`; the current
row feeds the stale-assets guard and the `cafleet doctor` setup column, while
superseded rows surface only as informational doctor footnotes (see
[CLI options](cli-options.md#stale-assets-guard)).

## Foreign key enforcement

SQLite ignores FK declarations unless `PRAGMA foreign_keys=ON` is issued per
connection; the connection opener applies it on every connection. FKs use
`ON DELETE RESTRICT` except the `member_id` PK=FK of the 1:1 child table
(`member_placements`), which uses `CASCADE` so a hard-deleted member cannot
leave dangling rows. Normal delete paths are soft-deletes, so neither fires
in practice.

## Message Visibility Rules

Read access is **by id** — the subject row carries its own fleet and
recipient, so existence (plus, for ACK, message state) is the enforcement:

| Operation | Enforcement |
|---|---|
| `message poll` | Returns the `input_required` deliveries whose `owner_member_id` equals the positional `MEMBER_ID`; the member must exist, and any caller can poll any inbox by id. |
| `message show` | Returns the message iff the `MESSAGE_ID` exists; unknown ids return "not found". |
| `message ack` | Transitions the message iff it exists and is in the `input_required` state; the recipient is derived from the message row. |

## Broadcast Grouping

A broadcast produces N+1 rows — one delivery message per active recipient plus
one `broadcast_summary` message — grouped by `origin_message_id`:

| Row kind | `origin_message_id` value |
|---|---|
| Unicast delivery | `NULL` |
| Broadcast delivery row (one per recipient) | The summary message's `message_id` |
| Broadcast summary row | Its own `message_id` (self-reference) |

Because ids are DB-assigned, the summary row is inserted first with a
temporarily `NULL` `origin_message_id`, then self-linked before the delivery rows
are inserted. The grouping predicate `origin_message_id IS NOT NULL` cleanly
partitions the timeline into standalone unicasts vs broadcast groups. The
per-recipient ACK time is read from the `completed` delivery row's
`status_timestamp`, which is valid because a delivery message makes exactly one
state transition over its lifetime.
