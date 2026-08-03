# WebUI API

Base path: `/api`

## Request Headers

The WebUI does not require authentication. Fleet-scoped endpoints require an
`X-Fleet-Id` header. The header value is the integer fleet id, sent as a string
over HTTP and coerced by the backend with `int(...)`. A missing or non-integer
value returns 400. The backend verifies the fleet exists in the `fleets` table.

These fleet-scoping errors apply to **every** fleet-scoped endpoint:

| Status | `detail` | Trigger |
|---|---|---|
| 400 | `X-Fleet-Id header required` | The `X-Fleet-Id` header is missing or empty |
| 400 | `X-Fleet-Id must be an integer` | The header value is not an integer |
| 404 | `Fleet not found` | The header names a fleet id that does not exist |

No server-side session cookies. The SPA stores the active fleet_id client-side via hash-based routing and sends it in the X-Fleet-Id header on each request.

## Endpoints

| Method | Path | Returns | X-Fleet-Id required |
|---|---|---|---|
| `GET` | `/api/fleets` | Non-soft-deleted fleets with member counts | no |
| `GET` | `/api/members` | The fleet's roster | yes |
| `GET` | `/api/monitor` | Liveness of the fleet's `cafleet monitor` process plus per-member pending-delivery counts | yes |
| `GET` | `/api/members/{member_id}/inbox` | Messages received by the member | yes |
| `GET` | `/api/members/{member_id}/sent` | Messages sent by the member | yes |
| `GET` | `/api/timeline` | The fleet's unified message timeline | yes |
| `POST` | `/api/messages/send` | The new message's id and status | yes |

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
      "placement": null
    },
    {
      "member_id": 4,
      "name": "alice",
      "description": "Ordinary member",
      "status": "active",
      "registered_at": "2026-04-15T10:06:00+00:00",
      "kind": "member",
      "placement": {"backend": "tmux", "mux_session": "main", "mux_window_id": "@1", "mux_pane_id": "%13", "coding_agent": "claude", "created_at": "2026-04-15T10:06:00+00:00"}
    }
  ]
}
```

**`kind` values** — the unified 2-value vocabulary:

| Value | Meaning |
|---|---|
| `"director"` | The fleet's root Director (`member_id == fleets.director_member_id`). Exactly one per fleet. |
| `"member"` | Any other member. |

The discriminator is derived at read time — the fleets join supplies "is this the root Director"; there is no dedicated column.

### GET /api/monitor — Fleet Monitor Runtime

Returns the liveness of the fleet's `cafleet monitor` process, derived from the
`monitor_runtime` heartbeat (true even when the process died silently), plus a
`members` array with each member's pending-delivery counts. Lets the members
page show a "monitor running / stopped" indicator. See
[Monitoring](../concepts/monitoring.md).

**Request**: `X-Fleet-Id: <fleet_id>` header.

**Response** (200 OK):

```json
{
  "running": true,
  "pid": 4821,
  "tick_seconds": 5,
  "last_tick_at": "2026-06-13T04:51:02+00:00",
  "last_tick_age_seconds": 2,
  "started_at": "2026-06-13T04:50:00+00:00",
  "last_wake_at": "2026-06-13T04:50:30+00:00",
  "last_wake_age_seconds": 32,
  "members": [
    {
      "member_id": 4,
      "name": "drafter",
      "pending_count": 2,
      "oldest_pending_ts": "2026-08-03T09:00:00.000000+00:00",
      "oldest_pending_age_seconds": 120
    }
  ]
}
```

Each `members` element carries the member's count of `input_required` unicast
deliveries (`pending_count`) and the timestamp and age of the oldest one
(`null` when there is none), ordered by `member_id` ascending.

When no monitor is running — no runtime row, or a stale or cleared heartbeat —
the runtime fields take these values:

| Field | No runtime row has ever existed | Stale or cleared heartbeat row |
|---|---|---|
| `running` | `false` | `false` |
| `pid` | `null` | `null` |
| `started_at` | `null` | `null` |
| `last_tick_at` | `null` | `null` |
| `last_tick_age_seconds` | `null` | `null` |
| `last_wake_at` | `null` | `null` |
| `last_wake_age_seconds` | `null` | `null` |
| `tick_seconds` | `null` | **preserved** — the cadence the monitor last ran at |

Launching the loop is CLI-only (`cafleet monitor start`, run by the Director
as a background task in its own pane); there is no `POST`/`DELETE` counterpart
here and no `monitor stop` command — the Director stops the background task,
and a still-running loop self-terminates after `fleet delete`.

### GET /api/members/{member_id}/inbox — Inbox Messages

Returns messages received by the member (`owner_member_id = member_id`), excluding `broadcast_summary` type messages. Consumed by the member detail view's **Inbox** tab in the admin WebUI.

The three message endpoints compare as follows:

| Endpoint | Rows returned | Excluded | Ordering | Row cap |
|---|---|---|---|---|
| `GET /api/members/{member_id}/inbox` | Messages where `owner_member_id = member_id` | `type == "broadcast_summary"` | `status_timestamp DESC` (newest first) | none — the member detail view truncates client-side to the 200 most recent rows per tab |
| `GET /api/members/{member_id}/sent` | Messages where `from_member_id = member_id` | `type == "broadcast_summary"` | `status_timestamp DESC` (newest first) | none — the member detail view truncates client-side to the 200 most recent rows per tab |
| `GET /api/timeline` | All fleet messages, scoped through the sender join | `type == "broadcast_summary"` | `status_timestamp DESC` (newest first) | Hard-capped at 200 rows; no pagination |

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

**Status values**: `input_required` (Pending), `completed` (Acknowledged).

### GET /api/members/{member_id}/sent — Sent Messages

Returns messages sent by the member (single SQL query against `messages` filtered by `from_member_id` and ordered by `status_timestamp DESC`, served by `idx_messages_from_member_status_ts`), excluding `broadcast_summary` type messages. Consumed by the member detail view's **Sent** tab in the admin WebUI.

**Request**: `X-Fleet-Id: <fleet_id>` header.

Same response format as inbox.

### GET /api/timeline — Unified Fleet Timeline

Returns the selected fleet's non-`broadcast_summary` messages. Consumed by the Discord-style admin dashboard, which groups delivery rows sharing an `origin_message_id` into a single broadcast entry client-side.

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

The frontend re-orders ascending for newest-at-bottom chat rendering.

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

**Errors** — in addition to the shared fleet-scoping errors in
[Request Headers](#request-headers):

| Status | `detail` | Trigger |
|---|---|---|
| 422 | A `detail` string — see [Error Format](#error-format) | Missing or invalid `from_member_id`, `to_member_id`, or `text` |
| 400 | `from_member not in fleet` | `from_member_id` is not an active member in the caller's fleet |
| 404 | `Member not found` | `to_member_id` does not resolve to an active member in the fleet (unknown, cross-fleet, or deregistered) |

## Error Format

Every WebUI API error — the 400 / 404 responses and request-validation
failures (422) alike — carries a single `detail` string:

```json
{"detail": "Error message"}
```
