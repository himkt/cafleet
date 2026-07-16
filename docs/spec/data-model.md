---
icon: lucide/table
---

# Data model

The `Message` payload is fully relational: every routing field plus the message
body lives in its own typed column. The only JSON `TEXT` blob is
`members.member_card_json`. The runtime engine is SQLAlchemy 2.x with the
synchronous `pysqlite` driver; the schema is managed by a chain of Alembic
migrations bundled inside the wheel — run `cafleet setup` (or `cafleet setup
db`) to migrate to head (idempotent, data-preserving; see
[Storage](../concepts/storage.md)). The exact column-level DDL contract lives
in the repository's `SPEC.md`.

## Schema diagram

```mermaid
erDiagram
    fleets ||--o{ members : "fleet_id"
    fleets ||--o| monitor_runtime : "1:1 (reused PK)"
    members ||--o| member_placements : "1:1 (reused PK)"
    members ||--o| monitor_config : "1:1 (reused PK)"
    members ||--o{ messages : "owner_member_id"

    fleets {
        INTEGER fleet_id PK "AUTOINCREMENT"
        TEXT name
        TEXT created_at
        TEXT deleted_at "NULL = active"
        INTEGER director_member_id FK "root Director"
    }
    members {
        INTEGER member_id PK "AUTOINCREMENT"
        INTEGER fleet_id FK
        TEXT name
        TEXT description
        TEXT status "active | deregistered"
        TEXT registered_at
        TEXT deregistered_at "NULL = active"
        TEXT member_card_json "member card blob"
    }
    messages {
        INTEGER message_id PK "AUTOINCREMENT"
        INTEGER owner_member_id FK "recipient (or broadcaster for summary)"
        INTEGER from_member_id "sender; not an FK"
        INTEGER to_member_id "NULL for broadcast_summary"
        TEXT type "unicast | broadcast_summary"
        TEXT created_at
        TEXT status_state "input_required | completed | canceled"
        TEXT status_timestamp
        INTEGER origin_message_id "broadcast grouping self-link"
        TEXT text "message body"
    }
    member_placements {
        INTEGER member_id PK "reuses members.member_id"
        TEXT mux_session
        TEXT mux_window_id
        TEXT mux_pane_id "NULL = pending"
        TEXT backend "tmux | herdr"
        TEXT coding_agent "claude | codex | opencode"
        TEXT created_at
    }
    monitor_config {
        INTEGER member_id PK "reuses members.member_id"
        INTEGER interval_seconds
        TEXT last_ping_at "NULL = due immediately"
        INTEGER enabled "bool as 0/1"
    }
    monitor_runtime {
        INTEGER fleet_id PK "reuses fleets.fleet_id"
        INTEGER pid
        TEXT started_at
        TEXT last_tick_at "liveness heartbeat"
        INTEGER tick_seconds
    }
    skill_installs {
        TEXT coding_agent PK "claude | codex | opencode"
        TEXT cafleet_version
        TEXT installed_at
    }
```

The three minted-id tables (`fleets`, `members`, `messages`) use `INTEGER PRIMARY
KEY AUTOINCREMENT`, so **ids are never reused** and real ids are always `>= 1`.
The three 1:1 tables reuse a parent id as their PK; `skill_installs` keys on
the coding-agent name and is not FK-linked.

## Tables

### `fleets`

Fleet deletion is a **soft-delete** keyed on `deleted_at`. `cafleet fleet
create` writes the fleet row, the root Director (and its placement), and the
`director_member_id` back-reference in one
all-or-nothing transaction — which is why `director_member_id` is DB-nullable
despite the post-bootstrap NOT NULL invariant.

### `members`

Deregistration is a soft-delete (`status='deregistered'` + `deregistered_at`);
active query paths filter `status='active'`. Special members are marked by a
broker-owned `cafleet.kind` flag inside `member_card_json` rather than a
column: `"monitoring-member"` marks the fleet's single monitoring member
(which skips `monitor_config` enrollment and is located by this marker — see
[Monitoring](../concepts/monitoring.md)). Callers cannot set `cafleet.kind`
through any public path.

### `messages`

One row per delivery: a unicast row, a broadcast delivery row, or a broadcast
summary row (see [Broadcast grouping](#broadcast-grouping)). `from_member_id`
is deliberately not a foreign key — historical messages may outlive their sender.
`status_timestamp` is updated on every state change and drives `ORDER BY DESC`
listing. The rendered envelope is specified in
[Message envelope](message-envelope.md).

### `member_placements`

Links a member to its multiplexer pane; pane ids are stored verbatim as opaque
strings. The root Director keeps its own placement row (it is pane-bound); an
ordinary member is a placed row other than the fleet's root Director
(`member_id != fleets.director_member_id`). Placement rows are hard-deleted
when the member is deregistered — they have no historical value.

### `monitor_config` and `monitor_runtime`

The two monitor tables: `monitor_config` holds one row per **enrolled** member
(the root Director and every ordinary member — never the monitoring member
or placementless registry rows), hard-deleted alongside the
placement on deregistration; `monitor_runtime` holds one row per fleet with
the running loop's pid and `last_tick_at` heartbeat — "no monitor" is modeled
as "no row". Both are removed explicitly inside the `fleet delete`
transaction. Enrollment, cadence, and liveness semantics are on
[Monitoring](../concepts/monitoring.md).

### `skill_installs`

One upserted row per coding-agent home, recording the CLI version whose skills
and preset (where one exists) install last landed there — the row attests
both. Written by the assets half of `cafleet setup` (bare or per-agent);
feeds the stale-skills guard and the `cafleet doctor` report (see
[CLI options](cli-options.md#stale-skills-guard)).

## Foreign key enforcement

SQLite ignores FK declarations unless `PRAGMA foreign_keys=ON` is issued per
connection; a SQLAlchemy `connect` event listener does so. FKs use
`ON DELETE RESTRICT` except the `member_id` PK=FK of the two 1:1 child tables
(`member_placements`, `monitor_config`), which uses `CASCADE` so a hard-deleted
member cannot leave dangling rows. Normal delete paths are soft-deletes, so
neither fires in practice.

## Message Visibility Rules

Read access is **fleet-scoped**; per-member identity is enforced only on state
transitions:

| Operation | Enforcement |
|---|---|
| `message poll` | Returns the `input_required` deliveries whose `owner_member_id` equals `--member-id`; any in-fleet caller can poll any in-fleet inbox by id. |
| `message show` | Returns the message iff at least one endpoint belongs to `--fleet-id`; cross-fleet lookups return "not found". |
| `message ack` | Recipient-only — the caller must equal the message's `owner_member_id`. |
| `message cancel` | Sender-only — the caller must equal the message's `from_member_id`. |

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
