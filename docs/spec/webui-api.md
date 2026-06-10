# WebUI API Specification

Base path: `/api`

## Request Headers

The WebUI does not require authentication. Fleet-scoped endpoints require an `X-Fleet-Id` header:

| Header | Purpose |
|---|---|
| `X-Fleet-Id: <fleet_id>` | Required on fleet-scoped endpoints (agents, inbox, sent, timeline, send). The header value is the integer fleet id (sent as a string over HTTP; the backend coerces it with `int(...)` and returns 400 if it is not an integer). The backend verifies the fleet exists in the `fleets` table. |

No server-side session cookies. The SPA stores the active fleet_id client-side via hash-based routing and sends it in the X-Fleet-Id header on each request.

## Endpoints

### GET /api/fleets — List Fleets

Returns all fleets with agent counts, ordered newest-first by `created_at DESC, fleet_id ASC`. No headers required.

**Response** (200 OK):

```json
[
  {
    "fleet_id": 1,
    "label": "PR-42 review",
    "created_at": "2026-04-12T10:00:00+00:00",
    "agent_count": 3
  }
]
```

### GET /api/agents — List Agents

Returns agents belonging to the selected fleet. Every agent carries a `kind` discriminator so the frontend can locate the built-in Administrator without matching on its name.

**Request**: `X-Fleet-Id: <fleet_id>` header.

**Response** (200 OK):

```json
{
  "agents": [
    {
      "agent_id": 3,
      "name": "Administrator",
      "description": "Built-in administrator agent for fleet 1",
      "status": "active",
      "registered_at": "2026-04-15T10:00:00+00:00",
      "kind": "builtin-administrator"
    },
    {
      "agent_id": 4,
      "name": "Claude-B",
      "description": "Reviewer",
      "status": "active",
      "registered_at": "2026-04-15T10:05:00+00:00",
      "kind": "user"
    }
  ]
}
```

**`kind` values**:

| Value | Meaning |
|---|---|
| `"builtin-administrator"` | The fleet's built-in Administrator agent. Exactly one per fleet. Derived from `agent_card_json.cafleet.kind == "builtin-administrator"`. |
| `"user"` | Any other agent (human-registered, spawned member, etc.). |

The discriminator is derived at read time from the stored agent card; there is no dedicated column. See [data-model.md](./data-model.md) for the Administrator's full definition.

### GET /api/agents/{agent_id}/inbox — Inbox Messages

Returns messages received by the agent (`context_id = agent_id`), excluding `broadcast_summary` type tasks. Ordered newest first. Consumed by the agent detail view's **Inbox** tab in the admin WebUI.

**Request**: `X-Fleet-Id: <fleet_id>` header.

**Response** (200 OK):

```json
{
  "messages": [
    {
      "task_id": 42,
      "from_agent_id": 4,
      "from_agent_name": "Agent A",
      "to_agent_id": 5,
      "to_agent_name": "Agent B",
      "type": "unicast",
      "status": "input_required",
      "created_at": "2026-03-29T10:00:00+00:00",
      "status_timestamp": "2026-03-29T10:00:00+00:00",
      "origin_task_id": null,
      "body": "Hello, Agent B!"
    }
  ]
}
```

All message endpoints (inbox, sent, timeline) share the same row formatter, so the field set is identical to `GET /api/timeline` — including `status_timestamp` and `origin_task_id` (see the timeline section below for their semantics).

The `body` field is extracted from the task's first artifact's first text part. If no text part exists, `body` is `""`.

**Row cap**: none — the endpoint returns every matching row. The agent detail view truncates client-side to the 200 most recent rows per tab.

**Status values**: `input_required` (Pending), `completed` (Acknowledged), `canceled` (Canceled).

### GET /api/agents/{agent_id}/sent — Sent Messages

Returns messages sent by the agent (single SQL query against `tasks` filtered by `from_agent_id` and ordered by `status_timestamp DESC`, served by `idx_tasks_from_agent_status_ts`), excluding `broadcast_summary` type tasks. Ordered newest first. Consumed by the agent detail view's **Sent** tab in the admin WebUI.

**Request**: `X-Fleet-Id: <fleet_id>` header.

Same response format (and row-cap behavior) as inbox.

### GET /api/timeline — Unified Fleet Timeline

Returns up to 200 most-recent non-`broadcast_summary` tasks for the selected fleet, newest first. Consumed by the Discord-style admin dashboard, which groups delivery rows sharing an `origin_task_id` into a single broadcast entry client-side.

**Request**: `X-Fleet-Id: <fleet_id>` header.

Fleet scoping is reached through the `tasks.context_id → agents.agent_id → agents.fleet_id` join. Only tasks whose recipient belongs to the header fleet are returned; cross-fleet tasks are invisible.

**Response** (200 OK):

```json
{
  "messages": [
    {
      "task_id": 50,
      "from_agent_id": 4,
      "from_agent_name": "Claude-A",
      "to_agent_id": 5,
      "to_agent_name": "reviewer-bot",
      "type": "unicast",
      "status": "input_required",
      "created_at": "2026-04-11T10:00:00+00:00",
      "status_timestamp": "2026-04-11T10:00:00+00:00",
      "origin_task_id": null,
      "body": "Please review PR #42"
    }
  ]
}
```

**Ordering**: `status_timestamp DESC` (newest first). The frontend re-orders ascending for newest-at-bottom chat rendering.

**Row cap**: Hard-capped at 200 rows; no pagination.

**Exclusions**: Rows with `type == "broadcast_summary"` are filtered out of the response. The summary row is not needed for the UI; the grouping convention below lets the frontend reconstruct broadcasts from their delivery rows alone.

**Broadcast grouping**: Every row carries an `origin_task_id` field:

| Case | `origin_task_id` |
|---|---|
| Unicast delivery | `null` |
| Broadcast delivery | The broadcast's summary task id (shared across all N delivery rows in the same broadcast) |

The client groups rows by `origin_task_id` (non-null rows sharing a value form one broadcast entry; null rows are standalone unicast entries). Each broadcast entry's sort key is the `MIN(created_at)` of its rows — stable, so a broadcast never drifts when a lagging recipient ACKs.

**ACK timestamps**: Per-recipient ACK time is read from the `status_timestamp` of a `completed` delivery row. Delivery tasks make exactly one state transition over their lifetime (`input_required → completed` on ACK), so for `status == "completed"` rows `status_timestamp` IS the ACK moment. If this invariant is ever broken by a future change, the timeline will silently show wrong ACK times until a dedicated `acknowledged_at` column is added. See `docs/spec/data-model.md` for the accompanying design-debt note.

### POST /api/messages/send — Send Message

Sends a message from a same-fleet active agent. Supports both unicast (`to_agent_id=<int>`) and broadcast (`to_agent_id="*"`).

**Request**:

```
X-Fleet-Id: <fleet_id>
```

```json
{
  "from_agent_id": 3,
  "to_agent_id": 4,
  "text": "Hello!"
}
```

`to_agent_id` accepts an integer (unicast) or the string `"*"` (broadcast). `from_agent_id` is always an integer.

**Unicast** (`to_agent_id` is an integer): the server verifies both the sender and the destination belong to the caller's fleet and that the destination is active.

**Broadcast** (`to_agent_id == "*"`): the server skips destination validation (no specific recipient to verify) and fans out to every active agent in the fleet (except the built-in Administrator, which is filtered out of the recipient set at the broker layer) plus a summary task. The sender is still required to be active and in the caller's fleet; the sender MAY be the Administrator. The response's `task_id` is the summary task's id.

**Sender identity**: The Admin WebUI always submits `from_agent_id = administrator.agent_id` (the fleet's built-in Administrator). The endpoint itself is sender-agnostic — it accepts any active agent in the fleet — but no UI path lets the operator pick a different sender.

**Response** (200 OK):

```json
{
  "task_id": 42,
  "status": "input_required"
}
```

**Errors**:
- 422: Missing or invalid request-body fields (`from_agent_id`, `to_agent_id`, or `text`) — FastAPI/Pydantic validation on the request model.
- 400: `from_agent_id` is not an active agent in the caller's fleet (`from_agent not in fleet`). A missing or non-integer `X-Fleet-Id` header is also 400 — see [Request Headers](#request-headers).
- 404 — two cases, each with its own `detail` string: the destination `to_agent_id` does not resolve to an active agent in the fleet (unknown, cross-fleet, or deregistered) → `Agent not found`; an unknown `X-Fleet-Id` fleet → `Fleet not found`.
- 409 (reserved for future deregister endpoint): for any future endpoint that attempts to deregister or otherwise modify the built-in Administrator, the broker's protection error (`Administrator cannot be deregistered`) must be translated to a 409 response. This 409 is not currently reachable through `POST /api/messages/send`; this entry documents the required mapping for the future endpoint.

## Error Format

WebUI API errors use FastAPI's default error shape. `HTTPException` responses (400 / 404) carry a `detail` string:

```json
{"detail": "Error message"}
```

Request-body validation failures (422) use FastAPI's default validation error format — a `detail` array of per-field error objects.
