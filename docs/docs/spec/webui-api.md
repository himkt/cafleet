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

## Response compatibility

Typed broker records are internal. HTTP presenters retain each endpoint's
existing key names and order, nulls versus omitted fields, scalar types, list
order, and response envelope. Rust enum names or struct layouts do not become
wire fields. Timestamps retain their existing formatting and parsing rules.
The CLI keeps its separate text/JSON formatting, exit codes, diagnostics, and
validation order.

Message presenters still rename `status_state` to `status` and `text` to `body`,
and resolve names for non-null member ids. They emit keys in this order:
`message_id`, `from_member_id`, `from_member_name`, `to_member_id`,
`to_member_name`, `type`, `status`, `created_at`, `status_timestamp`,
`origin_message_id`, `body`. A summary's recipient id and name remain null.

| Condition | Boundary behavior |
|---|---|
| Missing fleet/member or active-monitor conflict | Map the domain error using the operation's existing status/detail or CLI error; retain its validation order. |
| Invalid stored enum or missing required sender/recipient name | Return an integrity failure as HTTP 500 with `{"detail": <string>}`; do not panic, substitute a fabricated name, or return a successful partial row. |
| Persisted message whose pane notification fails | Preserve the successful HTTP send response; CLI unicast retains its exit-1 partial-failure result and recovery instruction, while broadcast retains its delivered count. |

Concrete process/probe/notifier adapters belong to `runtime/`; HTTP handlers
use them without importing CLI helpers. This move preserves the notification
attempt, durable message id, and raw transport diagnostic. A failed preview
never rolls back or resends the message. See
[CLI partial-failure recovery](cli-options.md#message-send-partial-failure) and
[typed records](data-model.md#typed-broker-records).

## Endpoints

| Method | Path | Returns | X-Fleet-Id required |
|---|---|---|---|
| `GET` | `/api/fleets` | Non-soft-deleted fleets with member counts | no |
| `GET` | `/api/members` | The fleet's roster | yes |
| `GET` | `/api/monitor` | Liveness of the fleet's `cafleet monitor` process plus per-member pending-delivery counts | yes |
| `PATCH` | `/api/monitor` | The updated Director wake interval | yes |
| `POST` | `/api/monitor/wake` | The timestamp of the recorded immediate-wake request | yes |
| `GET` | `/api/members/{member_id}/inbox` | Messages received by the member | yes |
| `GET` | `/api/members/{member_id}/sent` | Messages sent by the member | yes |
| `GET` | `/api/timeline` | The fleet's unified message timeline | yes |
| `POST` | `/api/messages/send` | The new message's id and status | yes |

### GET /api/fleets — List Fleets

Returns non-soft-deleted fleets (`deleted_at IS NULL`) with member counts, ordered newest-first by `created_at DESC, fleet_id DESC` (higher id first when timestamps tie). No headers required.

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

Returns the selected fleet's roster: every active registry entry plus deregistered members that still own messages (so their message history stays inspectable). Every row carries a `kind` discriminator so the frontend can locate the root Director without matching on its name.

Rows are ordered by `member_id ASC`. Holder inclusion checks
`messages.owner_member_id = members.member_id`; a sender-only reference does
not include a deregistered member. The lean query preserves
this condition and the existing response while omitting unused message
activity aggregates. CLI member listing retains its separate activity query;
see [query and activity contracts](data-model.md#query-and-activity-contracts).

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

**`kind` values** — the unified 3-value vocabulary:

| Value | Meaning |
|---|---|
| `"director"` | The fleet's root Director (`member_id == fleets.director_member_id`). Exactly one per fleet. |
| `"monitor"` | The fleet's monitor member — its `member_card_json` carries the `$.cafleet.kind = 'monitor'` marker. At most one active per fleet. |
| `"member"` | Any other member. |

The discriminator is derived at read time — the fleets join supplies "is this the root Director" and the member-card marker supplies "is this the monitor member"; there is no dedicated column (see [Data model](data-model.md)).

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
  "wake_interval_seconds": 600,
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

The `members` array is the wake roster: every active placed member excluding
the Director and the monitor member, ordered by `member_id` ascending. Each
element carries the member's count of `input_required` unicast deliveries
(`pending_count`) and the timestamp and age of the oldest one (`null` when
there is none).

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
| `wake_interval_seconds` | `null` | **preserved** — the wake interval the monitor last ran at; `null` when the row predates the column and was never re-stamped |

This is a presenter projection, not a serialization of the stored
`MonitorRuntime` record. Stored null, zero, and a positive wake interval remain
distinct: zero disables scheduled wakes but still allows forced wake, while a
legacy null interval remains null until claimed. A normal clear retains the
stored `last_wake_at` and pending `wake_requested_at` even though stopped-process
timestamps are hidden here; the request field is not added to this response.
The complete field lifecycle is in the
[data model](data-model.md#monitor_runtime).


Launching the loop is CLI-only (`cafleet monitor`), and the monitor member owns
it as a long-lived execution resolved by its backend. It has no `POST`/`DELETE` counterpart
and no CLI stop command — deleting the monitor member kills the pane hosting
the loop, and a still-running loop self-terminates after `fleet delete`.

### PATCH /api/monitor — Update the Wake Interval {#patch-api-monitor}

Updates the fleet's wake interval. The running loop re-reads the
stored value on every tick, so the edit changes the cadence within one scan
tick; the next `cafleet monitor` start re-stamps the interval from the CLI/env
resolution. See
[Monitoring](../concepts/monitoring.md#cadence-and-tick-precision).

**Request**: `X-Fleet-Id: <fleet_id>` header.

```json
{"wake_interval_seconds": 300}
```

`wake_interval_seconds` must be a JSON integer in `0..=i64::MAX` — floats,
stringified integers, negatives, and numbers above `i64::MAX` are rejected,
not coerced, mirroring the send endpoint's strictness. `0` disables the wake;
there is no application-level cap below `i64::MAX`.

**Response** (200 OK):

```json
{"wake_interval_seconds": 300}
```

**Errors** (all `{"detail": <string>}`-shaped):

| Status | `detail` | Trigger |
|---|---|---|
| 400 | `X-Fleet-Id header required` | The `X-Fleet-Id` header is missing or empty |
| 400 | `X-Fleet-Id must be an integer` | The header value is not an integer |
| 422 | `invalid JSON body: <parse error>` | The request body is not parsable JSON |
| 422 | `wake_interval_seconds must be a non-negative integer` | `wake_interval_seconds` is missing, or not an integer in `0..=i64::MAX` |
| 404 | `Fleet not found` | The header names a fleet id that does not exist |
| 404 | `monitor has never run for this fleet` | The fleet has no `monitor_runtime` row |

Resolution order (the table's row order): header errors, then body
validation, then the fleet check, then the row update — matching
`POST /api/messages/send`, whose body parse likewise precedes the fleet
check, so an unknown fleet plus an invalid body yields 422 on both
endpoints. A no-row 404 means the fleet's monitor has never run —
`monitor_runtime` rows are removed only by `fleet delete`. The two monitor
write endpoints gate their 404s differently: this endpoint's gate is row
**existence** (the interval is a durable setting), while
`POST /api/monitor/wake` below gates on **liveness** — a wake request needs
a live consumer; against a dead loop it would silently never fire.

### POST /api/monitor/wake — Request an Immediate Wake {#post-api-monitor-wake}

Records a durable request for an immediate monitor wake on the fleet's
runtime row. The running loop honors the request on its next tick, so the
wake lands within one scan tick (default 5 s) — bypassing a disabled
schedule (`wake_interval_seconds = 0`) and a not-yet-due one alike. Repeat
requests overwrite the stored timestamp, coalescing into a single wake. A
delivered wake — scheduled or forced — stamps the last-wake timestamp and
clears the request in one write, so a forced wake resets the schedule
baseline. See
[Monitoring](../concepts/monitoring.md#cadence-and-tick-precision).

**Request**: `X-Fleet-Id: <fleet_id>` header. No request body; any body is
ignored.

**Response** (200 OK):

```json
{"wake_requested_at": "2026-06-13T04:52:00+00:00"}
```

**Errors** (all `{"detail": <string>}`-shaped):

| Status | `detail` | Trigger |
|---|---|---|
| 400 | `X-Fleet-Id header required` | The `X-Fleet-Id` header is missing or empty |
| 400 | `X-Fleet-Id must be an integer` | The header value is not an integer |
| 404 | `Fleet not found` | The header names a fleet id that does not exist |
| 404 | `monitor is not running for this fleet` | The fleet's monitor loop is not live — no runtime row, a cleared slot, or a stale heartbeat — or the row vanished between the liveness check and the write (e.g. a concurrent fleet delete) |

### GET /api/members/{member_id}/inbox — Inbox Messages

Returns messages received by the member. Consumed by the member detail view's **Inbox** tab in the admin WebUI.

The three message endpoints compare as follows (this table owns their row-selection, exclusion, ordering, and cap attributes):

| Endpoint | Rows returned | Excluded | Ordering | Row cap |
|---|---|---|---|---|
| `GET /api/members/{member_id}/inbox` | Messages where `owner_member_id = member_id` | `type == "broadcast_summary"` | `status_timestamp DESC, message_id DESC` (newest status update first; id breaks ties) | planned optional SQL `limit` of 1–1000; omitted means unbounded |
| `GET /api/members/{member_id}/sent` | Messages where `from_member_id = member_id` | `type == "broadcast_summary"` | `status_timestamp DESC, message_id DESC` (newest status update first; id breaks ties) | planned optional SQL `limit` of 1–1000; omitted means unbounded |
| `GET /api/timeline` | `type == "unicast"` deliveries, scoped through the owning member join | All non-delivery rows, including `broadcast_summary` | `status_timestamp DESC, message_id DESC` (newest status update first; id breaks ties) | SQL limit of 200 delivery rows, applied after filtering; may split a broadcast group; no pagination |

#### Member history limits

The following optional-limit contract is planned; the current implementation
still returns unbounded history. It applies to both inbox and sent.

**Request**: `X-Fleet-Id: <fleet_id>` header and optional `?limit=201`.

| Query input | Result |
|---|---|
| `limit` omitted | All matching deliveries, including more than 201; existing callers keep their behavior. |
| One decimal integer from 1 through 1000 | At most that many deliveries, selected by a bound SQL `LIMIT`. |
| Empty, 0, negative, fractional, nonnumeric, overflowing, or repeated `limit` | `422`, exactly `{"detail":"limit must be an integer between 1 and 1000"}`. |
| Unknown query parameter | Ignored, as before. |

After URL form decoding, the value must contain only ASCII digits. Leading
zeros are accepted (`001` means 1); signs, whitespace, Unicode digits, and
exponent notation are rejected. Duplicate decoded `limit` keys are rejected
even when a key was percent-encoded.

Validation preserves the existing Path extraction first, then fleet header
(`400`), fleet existence (`404`), and member membership (`404`), before checking
`limit`. An invalid limit must not hide an earlier header or membership error.
Deregistered members remain readable within their fleet. SQL errors keep the
existing `500` response with a string `detail`.

Selection and ordering happen before the limit: delivery rows only, newest
`status_timestamp` first, then largest `message_id` for ties. Limits count rows
and may split a broadcast. The response remains `{"messages":[...]}` with the
same row keys, order, and nulls; there is no cursor, `has_more`, or total field.
CLI retrieval remains unchanged.

The planned WebUI change requests `?limit=201` for each tab and displays the
first 200 deliveries. It shows `Showing the 200 most recent messages` only when
there is a 201st delivery; empty results and results of exactly 200 have no
omission footer. “Most recent” follows status updates, even though each row
also displays its creation timestamp. This limits WebUI reads and explicit
HTTP limits, not unbounded HTTP callers or database storage.

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

The wire `type` distinguishes `unicast` deliveries, with non-null `to_member_id` and `to_member_name`, from `broadcast_summary` rows, whose recipient id and name are null. The frontend models this distinction and narrows inbox, sent, and timeline data to delivery rows. Existing response keys and envelopes are preserved.

The `body` field is the message's `text` column.

**Status values**: `input_required` (Pending), `completed` (Acknowledged).

### GET /api/members/{member_id}/sent — Sent Messages

Returns messages sent by the member (see the comparison table above). Consumed by the member detail view's **Sent** tab in the admin WebUI.

**Request**: `X-Fleet-Id: <fleet_id>` header.

Same response format and [planned limit contract](#member-history-limits) as inbox.

### GET /api/timeline — Unified Fleet Timeline

Returns the fleet's unified message timeline (see the comparison table above). Consumed by the Discord-style admin dashboard, which groups delivery rows sharing an `origin_message_id` into a single broadcast entry client-side.

**Request**: `X-Fleet-Id: <fleet_id>` header.

Fleet scoping follows `messages.owner_member_id → members.member_id → members.fleet_id`. Only delivery rows whose **owning member** belongs to the header fleet are returned. SQL selects `type = 'unicast'` before ordering and applying the 200-row cap.

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

The frontend orders the returned entries by creation time, ascending for newest-at-bottom chat rendering. This is distinct from the API's selection by most recent status update: an ACK updates `status_timestamp` but leaves `created_at` unchanged.

**Exclusions**: `broadcast_summary` rows never enter the timeline response or consume its row cap. They remain stored and accessible through `message show` and the broadcast command's result. A summary is created in the `completed` state; that state is not a recipient ACK. The frontend also ignores summary rows defensively before grouping if they appear in its input.

**Broadcast grouping**: Every row carries an `origin_message_id` field:

| Case | `origin_message_id` |
|---|---|
| Unicast delivery | `null` |
| Broadcast delivery | The broadcast's summary message id (shared across all N delivery rows in the same broadcast) |

The client groups delivery rows by `origin_message_id` using an explicit null check: non-null rows sharing a value form one broadcast entry; null rows are standalone unicast entries. Each broadcast entry's sort key is the minimum `created_at` among its returned delivery rows. A standalone unicast uses its own `created_at`.

**Partial groups and counts**: the cap applies to delivery rows, not whole broadcasts. It can omit some recipients of a group. Recipient counts and the ReactionBar's ACK indicators describe only the returned deliveries; the UI explains this limit and does not present them as a whole-broadcast completion rate. Omitted recipients are not fetched to complete a group.

For two pending broadcast deliveries and their stored summary, the timeline shows two recipients and zero ACKs. Acknowledging one delivery produces one ACK; acknowledging both produces two. The summary contributes neither a recipient nor an ACK. Empty or summary-only input produces no timeline entries.

**ACK timestamps**: Per-recipient ACK time is read from the `status_timestamp` of a `completed` delivery row. Delivery messages make exactly one state transition over their lifetime (`input_required → completed` on ACK), so for `status == "completed"` rows `status_timestamp` IS the ACK moment. See [Data model § Broadcast Grouping](data-model.md#broadcast-grouping).

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
