---
icon: lucide/globe
---

# WebUI API

Base path: `/api`

## Request Headers

The WebUI does not require authentication. Fleet-scoped endpoints require an `X-Fleet-Id` header:

| Header | Purpose |
|---|---|
| `X-Fleet-Id: <fleet_id>` | Required on fleet-scoped endpoints (members, inbox, sent, timeline, send). The header value is the integer fleet id (sent as a string over HTTP; the backend coerces it with `int(...)` and returns 400 if it is not an integer). The backend verifies the fleet exists in the `fleets` table. |

No server-side session cookies. The SPA stores the active fleet_id client-side via hash-based routing and sends it in the X-Fleet-Id header on each request.

## Endpoints

### GET /api/fleets — List Fleets

Returns non-soft-deleted fleets (`deleted_at IS NULL`) with member counts, ordered newest-first by `created_at DESC, fleet_id ASC`. No headers required.

**Response** (200 OK):

```json
[
  {
    "fleet_id": 1,
    "director_member_id": 2,
    "name": "PR-42 review",
    "created_at": "2026-04-12T10:00:00+00:00",
    "member_count": 3
  }
]
```

### GET /api/members — List Members

Returns the selected fleet's roster via `list_roster(include_message_holders=True)`: every active registry entry plus deregistered members that still own messages (so their message history stays inspectable). Every row carries a `kind` discriminator so the frontend can locate the root Director without matching on its name.

**Request**: `X-Fleet-Id: <fleet_id>` header.

**Response** (200 OK):

```json
{
  "members": [
    {
      "member_id": 2,
      "name": "Director",
      "description": "Root Director for this fleet",
      "status": "active",
      "registered_at": "2026-04-15T09:59:00+00:00",
      "kind": "director",
      "monitor": {"interval_seconds": 180, "last_ping_at": null, "enabled": true}
    },
    {
      "member_id": 3,
      "name": "monitor",
      "description": "Monitoring member: owns the heartbeat",
      "status": "active",
      "registered_at": "2026-04-15T10:05:00+00:00",
      "kind": "monitor",
      "monitor": null
    },
    {
      "member_id": 4,
      "name": "alice",
      "description": "Ordinary member",
      "status": "active",
      "registered_at": "2026-04-15T10:06:00+00:00",
      "kind": "member",
      "monitor": {"interval_seconds": 720, "last_ping_at": null, "enabled": true}
    }
  ]
}
```

**`monitor` field**: each member carries its folded monitoring schedule —
`{"interval_seconds": int, "last_ping_at": str|null, "enabled": bool}` — or
`null` when the member is not enrolled (the unenrolled watcher, deregistered
members, and members without a placement all carry
`monitor: null`). Folding the schedule into the list lets the SPA render every
member's schedule without an extra request per member. Which members are
enrolled — the watched set — is defined in
[Monitoring](../concepts/monitoring.md).

**`kind` values** — the unified 3-value vocabulary:

| Value | Meaning |
|---|---|
| `"director"` | The fleet's root Director (`member_id == fleets.director_member_id`). Exactly one per fleet. |
| `"monitor"` | The fleet's dedicated monitoring member. Derived from `member_card_json.cafleet.kind == "monitoring-member"`. |
| `"member"` | Any other (ordinary) member. |

The discriminator is derived at read time — the fleets join supplies "is this the root Director" and the stored member card supplies the special-kind marker; there is no dedicated column.

### GET /api/monitor — Fleet Monitor Runtime

Returns the liveness of the fleet's `cafleet monitor` process, derived from the
`monitor_runtime` heartbeat (true even when the process died silently). Lets the
members page show a "monitor running / stopped" indicator so an inert schedule
does not mislead. See [Monitoring](../concepts/monitoring.md).

**Request**: `X-Fleet-Id: <fleet_id>` header.

**Response** (200 OK):

```json
{
  "running": true,
  "pid": 4821,
  "tick_seconds": 5,
  "last_tick_at": "2026-06-13T04:51:02+00:00",
  "last_tick_age_seconds": 2,
  "started_at": "2026-06-13T04:50:00+00:00"
}
```

When no monitor is running (no row, or a stale/cleared heartbeat) `running` is
`false` and `pid` / `last_tick_at` / `started_at` / `last_tick_age_seconds` are
`null`. `tick_seconds` is `null` only when **no runtime row has ever existed**;
for a stale or cleared row it is **preserved** (the cadence the monitor last ran
at). Launching the loop is CLI-only (`cafleet monitor start`, run as a
background task); there is no `POST`/`DELETE` counterpart here and no
`monitor stop` command — the loop terminates with the monitoring member's
pane (`member delete`), or self-terminates after `fleet delete`.

### GET /api/members/{member_id}/monitor — Member Monitor Config

Returns one member's monitoring schedule.

**Request**: `X-Fleet-Id: <fleet_id>` header.

**Response** (200 OK):

```json
{
  "interval_seconds": 180,
  "last_ping_at": null,
  "enabled": true
}
```

**Errors**: 404 (`detail: "Member not enrolled"`) when the member is not in the
fleet or not enrolled (the monitoring member, deregistered,
placementless). 400 for a missing or non-integer `X-Fleet-Id`; 404
(`detail: "Fleet not found"`) for an unknown fleet. The SPA reads the folded
`monitor` field on `GET /api/members` instead of calling this endpoint per
member — it exists for CLI/API parity.

### PATCH /api/members/{member_id}/monitor — Edit Member Monitor Config

Updates a member's interval and/or enabled flag and returns the new config.

**Request**: `X-Fleet-Id: <fleet_id>` header.

```json
{
  "interval_seconds": 30,
  "enabled": false
}
```

Both fields are optional (Pydantic `MonitorPatch`); `interval_seconds >= 1` —
the same lower bound the CLI `--interval` (`click.IntRange(min=1)`) enforces.

**Response** (200 OK): the updated config, same shape as the `GET` above.

**Errors**: 422 on an invalid body (e.g. `interval_seconds < 1`, wrong type) —
FastAPI/Pydantic validation. 404 (`detail: "Member not enrolled"`) when the
member is not in the fleet or not enrolled. 400 for a missing or non-integer
`X-Fleet-Id`; 404 (`detail: "Fleet not found"`) for an unknown fleet.

### GET /api/members/{member_id}/inbox — Inbox Messages

Returns messages received by the member (`owner_member_id = member_id`), excluding `broadcast_summary` type messages. Ordered newest first. Consumed by the member detail view's **Inbox** tab in the admin WebUI.

**Request**: `X-Fleet-Id: <fleet_id>` header.

**Response** (200 OK):

```json
{
  "messages": [
    {
      "message_id": 42,
      "from_member_id": 4,
      "from_member_name": "Member A",
      "to_member_id": 5,
      "to_member_name": "Member B",
      "type": "unicast",
      "status": "input_required",
      "created_at": "2026-03-29T10:00:00+00:00",
      "status_timestamp": "2026-03-29T10:00:00+00:00",
      "origin_message_id": null,
      "body": "Hello, Member B!"
    }
  ]
}
```

All message endpoints (inbox, sent, timeline) share the same row formatter, so the field set is identical to `GET /api/timeline` — including `status_timestamp` and `origin_message_id` (see the timeline section below for their semantics).

The `body` field is the message's `text` column.

**Row cap**: none — the endpoint returns every matching row. The member detail view truncates client-side to the 200 most recent rows per tab.

**Status values**: `input_required` (Pending), `completed` (Acknowledged), `canceled` (Canceled).

### GET /api/members/{member_id}/sent — Sent Messages

Returns messages sent by the member (single SQL query against `messages` filtered by `from_member_id` and ordered by `status_timestamp DESC`, served by `idx_messages_from_member_status_ts`), excluding `broadcast_summary` type messages. Ordered newest first. Consumed by the member detail view's **Sent** tab in the admin WebUI.

**Request**: `X-Fleet-Id: <fleet_id>` header.

Same response format (and row-cap behavior) as inbox.

### GET /api/timeline — Unified Fleet Timeline

Returns up to 200 most-recent non-`broadcast_summary` messages for the selected fleet, newest first. Consumed by the Discord-style admin dashboard, which groups delivery rows sharing an `origin_message_id` into a single broadcast entry client-side.

**Request**: `X-Fleet-Id: <fleet_id>` header.

Fleet scoping is reached through the `messages.from_member_id → members.member_id → members.fleet_id` join. Only messages whose **sender** belongs to the header fleet are returned; cross-fleet messages are invisible.

**Response** (200 OK):

```json
{
  "messages": [
    {
      "message_id": 50,
      "from_member_id": 4,
      "from_member_name": "Claude-A",
      "to_member_id": 5,
      "to_member_name": "reviewer-bot",
      "type": "unicast",
      "status": "input_required",
      "created_at": "2026-04-11T10:00:00+00:00",
      "status_timestamp": "2026-04-11T10:00:00+00:00",
      "origin_message_id": null,
      "body": "Please review PR #42"
    }
  ]
}
```

**Ordering**: `status_timestamp DESC` (newest first). The frontend re-orders ascending for newest-at-bottom chat rendering.

**Row cap**: Hard-capped at 200 rows; no pagination.

**Exclusions**: Rows with `type == "broadcast_summary"` are filtered out of the response. The summary row is not needed for the UI; the grouping convention below lets the frontend reconstruct broadcasts from their delivery rows alone.

**Broadcast grouping**: Every row carries an `origin_message_id` field:

| Case | `origin_message_id` |
|---|---|
| Unicast delivery | `null` |
| Broadcast delivery | The broadcast's summary message id (shared across all N delivery rows in the same broadcast) |

The client groups rows by `origin_message_id` (non-null rows sharing a value form one broadcast entry; null rows are standalone unicast entries). Each broadcast entry's sort key is the `MIN(created_at)` of its rows — stable, so a broadcast never drifts when a lagging recipient ACKs.

**ACK timestamps**: Per-recipient ACK time is read from the `status_timestamp` of a `completed` delivery row. Delivery messages make exactly one state transition over their lifetime (`input_required → completed` on ACK), so for `status == "completed"` rows `status_timestamp` IS the ACK moment. If this invariant is ever broken by a future change, the timeline will silently show wrong ACK times until a dedicated `acknowledged_at` column is added. See [Data model](data-model.md) § ACK timestamp inference.

### POST /api/messages/send — Send Message

Sends a message from a same-fleet active member. Supports both unicast (`to_member_id=<int>`) and broadcast (`to_member_id="*"`).

**Request**:

```
X-Fleet-Id: <fleet_id>
```

```json
{
  "from_member_id": 2,
  "to_member_id": 4,
  "text": "Hello!"
}
```

`to_member_id` accepts an integer (unicast) or the string `"*"` (broadcast). `from_member_id` is always an integer.

**Unicast** (`to_member_id` is an integer): the server verifies both the sender and the destination belong to the caller's fleet and that the destination is active.

**Broadcast** (`to_member_id == "*"`): the server skips destination validation (no specific recipient to verify) and fans out to every active member in the fleet except the sender, plus a summary message. The sender is still required to be active and in the caller's fleet. The response's `message_id` is the summary message's id.

**Sender identity**: The Admin WebUI always submits `from_member_id = director.member_id` (the fleet's root Director). The endpoint itself is sender-agnostic — it accepts any active member in the fleet — but no UI path lets the operator pick a different sender.

**Response** (200 OK):

```json
{
  "message_id": 42,
  "status": "input_required"
}
```

**Errors**:
- 422: Missing or invalid request-body fields (`from_member_id`, `to_member_id`, or `text`) — FastAPI/Pydantic validation on the request model.
- 400: `from_member_id` is not an active member in the caller's fleet (`from_member not in fleet`). A missing or non-integer `X-Fleet-Id` header is also 400 — see [Request Headers](#request-headers).
- 404 — two cases, each with its own `detail` string: the destination `to_member_id` does not resolve to an active member in the fleet (unknown, cross-fleet, or deregistered) → `Member not found`; an unknown `X-Fleet-Id` fleet → `Fleet not found`.

## Error Format

WebUI API errors use FastAPI's default error shape. `HTTPException` responses (400 / 404) carry a `detail` string:

```json
{"detail": "Error message"}
```

Request-body validation failures (422) use FastAPI's default validation error format — a `detail` array of per-field error objects.
