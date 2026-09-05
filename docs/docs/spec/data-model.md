# Data model

The `Message` payload is fully relational: every routing field plus the message
body lives in its own typed column. The only JSON `TEXT` blob is
`members.member_card_json`. The database is SQLite, accessed synchronously
and bundled into the binary; the schema is managed by a chain of SQL
migrations embedded in the binary — run `cafleet setup` to migrate to head
(idempotent, data-preserving; see [Storage](../concepts/storage.md)). The exact column-level DDL contract lives
in the repository's `SPEC.md`.

Minted ids are **never reused** and real ids are always `>= 1`.

## Query and activity contracts

Fleet lists exclude soft-deleted fleets and order by
`created_at DESC, fleet_id DESC`; a timestamp tie puts the higher id first.
Member lists and rosters order by `member_id ASC` and preserve the root
Director → monitor → ordinary member kind precedence. Missing placement
remains null, while a placement whose pane is pending remains an object
with `mux_pane_id: null`.

`list_member_records` returns active members with activity. The WebUI uses
`list_roster_records` with message holders included: active members plus
deregistered members for whom an owned message exists
(`messages.owner_member_id = members.member_id`). A sender-only reference
does not include a deregistered member. The lean roster query keeps
this `EXISTS` condition but computes no send/receive/ACK aggregates.

| Activity field | Selection |
|---|---|
| `last_sent` | `MAX(created_at)` for all messages sent by this member, including broadcast summaries. |
| `last_recv` | `MAX(created_at)` for unicast deliveries owned by this member. |
| `last_ack` | `MAX(status_timestamp)` for completed unicast deliveries owned by this member. |

For `idle`, take the lexicographically greatest non-null string among all
three fields, then parse that one value with the existing lenient RFC3339
reader. All null or an unparseable selected value yields null, even if a
smaller string would parse. Use one `now` for the list, retain whole-second
truncation and existing timezone/fraction handling, and clamp only the final
result to zero: `max(0, (now - latest).num_seconds())`. Future stored values
stay unchanged. ACK can change `last_ack` and idle without changing the
creation timestamps used by `last_sent` and `last_recv`.

Name resolution returns `BTreeMap<i64, String>` with ascending keys. The
batched lookup deduplicates ids before issuing `IN` queries with at
most 500 bound ids each: empty input executes zero SQL, and other inputs
execute at most `ceil(unique_ids / 500)` queries. Unknown ids are omitted;
deregistered members are included. Only placeholders are assembled into SQL;
ids remain bound parameters.

Timeline scope follows the **owning member**, through
`messages.owner_member_id → members.member_id → members.fleet_id`, rather
than a sender join. This preserves the delivery selection and ordering in
the [WebUI API](webui-api.md).

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
monitor registration and its placement insert. A failure attempts to roll
back every added row; rollback failure is explicitly reported rather than
claimed as complete cancellation. A Herdr run failure is compensated by the
backend before the callback error causes DB rollback. After a successful
callback, placement-insert or commit failure closes the broker transaction
before the CLI kills its owned pane. A split failure with no confirmed id
leaves pane compensation unconfirmed. See the
[creation failure order](cli-options.md#creation-failure-compensation). The connection holds
SQLite's write lock across the pane-spawn subprocess call, so a concurrent
cafleet writer on the shared database blocks for the duration of the
multiplexer call, backstopped by the connection's `busy_timeout=5000`
PRAGMA.

### `members`

Active query paths filter `status='active'`. A member's `kind` (`director` /
`monitor` / `member`) is derived at read time from the fleet's
`director_member_id` back-reference plus the member card: a monitor-member
registration writes the application-level marker
`"cafleet": {"kind": "monitor"}` into `member_card_json`, while the Director
and ordinary members write no `$.cafleet` object. The marker remains plain JSON with no dedicated column. Schema V8 adds a
partial unique index enforcing at most one active monitor per fleet:

```sql
CREATE UNIQUE INDEX idx_members_one_active_monitor_per_fleet
ON members(fleet_id)
WHERE status = 'active'
  AND json_extract(member_card_json, '$.cafleet.kind') = 'monitor';
```

The predicate is the same one used by the active-monitor lookup. It applies
to inserts and updates of status, card, or fleet id. Ordinary members and
deregistered monitors are outside the constraint; different fleets are
independent. Root Director cards continue to omit the monitor marker, and
read-time kind resolution still gives the Director back-reference priority.
Registration takes an `IMMEDIATE` transaction and rechecks the monitor slot
inside it, before inserting either a member or placement. The CLI's early
check preserves validation order; the DB constraint also protects direct
broker callers and concurrent registrations. A conflict retains the existing
CLI error and exit 1, without creating a losing member, placement, or pane.
See [duplicate-monitor recovery](../concepts/storage.md#duplicate-monitor-recovery)
for migration of databases that already contain conflicting rows.

### `messages`

One row per unicast delivery, plus a separate summary row for each broadcast.
Broadcast deliveries also have type `unicast`; their summary has type
`broadcast_summary` (see [Broadcast grouping](#broadcast-grouping)). `from_member_id`,
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

`monitor_runtime` is the one-row-per-fleet loop pid/heartbeat table:

| Field | Rust type | Meaning and lifecycle |
|---|---|---|
| `fleet_id` | `i64` | Non-null fleet key; an absent row is `None`, not an id-zero row. |
| `pid` | `Option<i64>` | Null means no claim; claim stores the direct loop pid and normal clear nulls it. Preserve the existing process probe's handling of zero. |
| `started_at` | `Option<String>` | Null means no claim start time; claim/reclaim stamps it and normal clear nulls it. |
| `last_tick_at` | `Option<String>` | Null or an unparseable timestamp is not a fresh heartbeat. Claim/tick updates it and normal clear nulls it. |
| `last_wake_at` | `Option<String>` | Null means no successful wake is recorded; cadence then falls back to `started_at`. Successful scheduled/forced delivery updates it, and clear/reclaim preserve it. |
| `wake_requested_at` | `Option<String>` | Null means no forced-wake request; repeated requests overwrite/coalesce. Successful delivery or reclaim clears it; normal clear alone preserves it. |
| `tick_seconds` | `i64` | Non-null tick cadence, DB default 5; CLI rejects zero rather than treating it as disabled. Normal clear preserves it. |
| `wake_interval_seconds` | `Option<i64>` | Null is the legacy state before a row has been claimed since V5 added the column; zero disables scheduled wakes while permitting forced wake. Positive values are seconds; claim/reclaim/PATCH stamps the value and normal clear preserves it. |

A claimed loop always has a non-null wake interval. Do not normalize the legacy
null interval into zero or treat a stopped HTTP response as the stored row:
[the monitor response](webui-api.md#get-apimonitor--fleet-monitor-runtime) hides
process timestamps when stopped but preserves stored intervals. The cadence
rules are defined in
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

## Typed broker records

The broker decodes database columns into Rust records once. CLI and HTTP
presenters construct the existing output; these internal types do not change
the database schema or wire keys.

| Record | Fields |
|---|---|
| `MemberRecord` | `member_id`, `fleet_id`: `i64`; `name`, `description`, `registered_at`: `String`; `status: MemberStatus`, `kind: MemberKind`; `skills: Vec<Value>`; `placement: Option<Placement>` |
| `Placement` | `backend`, `mux_session`, `mux_window_id`, `coding_agent`, `created_at`: `String`; `mux_pane_id: Option<String>` |
| `MessageRecord` | `message_id`, `owner_member_id`, `from_member_id`: `i64`; `to_member_id`, `origin_message_id`: `Option<i64>`; `kind: MessageKind`, `status: MessageStatus`; `created_at`, `status_timestamp`, `text`: `String` |
| `MonitorRuntime` | The fields and nullable states in the `monitor_runtime` table above; row absence is `Option<MonitorRuntime>::None`. |

A missing placement differs from an existing placement whose pane id is still
null. A summary's null recipient differs from any actual member id. Preserve
those distinctions, timestamp strings, and the existing parse/format rules.
Only free-form skill elements remain generic `Value` inside these records;
JSON values at the output boundary remain appropriate.

When extracting skills, preserve the existing empty-array fallback for malformed
card JSON, missing skills, or a non-array skills value. Unknown stored enum
values instead return `InvalidStoredValue`; do not panic or invent a valid
status/kind. CLI/HTTP adapters map domain failures to their existing error
categories and response shapes. See
[response compatibility](webui-api.md#response-compatibility) and
[contributor boundaries](../contributing.md#rust-boundaries-and-compatibility).

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
are inserted. The timeline first selects only `type = 'unicast'` delivery rows,
then groups non-null `origin_message_id` values into broadcasts; a null origin
identifies a standalone unicast. Summary rows remain in the database and in
`message show` / broadcast results, but do not count as recipients or ACKs.

The per-recipient ACK time is read from the `completed` delivery row's
`status_timestamp`, which is valid because a delivery message makes exactly one
state transition over its lifetime. A summary is already `completed` when
created, without any recipient having acknowledged it.

The timeline's 200-row limit can return only part of a broadcast. Its recipient
and ACK counts describe the returned deliveries, not the entire broadcast.
See [timeline selection and grouping](webui-api.md#get-apitimeline--unified-fleet-timeline)
for SQL ordering and the separate UI creation-time ordering.
