# WebUI API Specification

Base path: `/ui/api`

## Request Headers

The WebUI does not require authentication. Session-scoped endpoints require an `X-Session-Id` header:

| Header | Purpose |
|---|---|
| `X-Session-Id: <session_id>` | Required on session-scoped endpoints (agents, inbox, sent, timeline, send). The backend verifies the session exists in the `sessions` table. |

No server-side session. No Auth0. The SPA manages the active session_id client-side via hash-based routing.

## Endpoints

### GET /ui/api/sessions — List Sessions

Returns all sessions with agent counts. No headers required.

**Response** (200 OK):

```json
[
  {
    "session_id": "550e8400-e29b-41d4-a716-446655440000",
    "label": "PR-42 review",
    "created_at": "2026-04-12T10:00:00+00:00",
    "agent_count": 3
  }
]
```

### GET /ui/api/agents — List Agents

Returns agents belonging to the selected session. Every agent carries a `kind` discriminator so the frontend can locate the built-in Administrator without matching on its name.

**Request**: `X-Session-Id: <session_id>` header.

**Response** (200 OK):

```json
{
  "agents": [
    {
      "agent_id": "uuid",
      "name": "Administrator",
      "description": "Built-in administrator agent for session 3f9a1b2c",
      "status": "active",
      "registered_at": "2026-04-15T10:00:00+00:00",
      "kind": "builtin-administrator"
    },
    {
      "agent_id": "uuid",
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
| `"builtin-administrator"` | The session's built-in Administrator agent. Exactly one per session. Derived from `agent_card_json.cafleet.kind == "builtin-administrator"`. |
| `"user"` | Any other agent (human-registered, spawned member, etc.). |

The discriminator is derived at read time from the stored `agent_card_json` blob — there is no dedicated column. `broker.list_session_agents` and `broker.get_agent` compute it via the `_is_administrator_card` helper.

### GET /ui/api/agents/{agent_id}/inbox — Inbox Messages

Returns messages received by the agent (`context_id = agent_id`), excluding `broadcast_summary` type tasks. Ordered newest first.

**Request**: `X-Session-Id: <session_id>` header.

**Response** (200 OK):

```json
{
  "messages": [
    {
      "task_id": "uuid",
      "from_agent_id": "uuid",
      "from_agent_name": "Agent A",
      "to_agent_id": "uuid",
      "to_agent_name": "Agent B",
      "type": "unicast",
      "status": "input_required",
      "created_at": "2026-03-29T10:00:00+00:00",
      "body": "Hello, Agent B!"
    }
  ]
}
```

The `body` field is extracted from the task's first artifact's first text part. If no text part exists, `body` is `""`.

**Status values**: `input_required` (Pending), `completed` (Acknowledged), `canceled` (Canceled).

### GET /ui/api/agents/{agent_id}/sent — Sent Messages

Returns messages sent by the agent (single SQL query against `tasks` filtered by `from_agent_id` and ordered by `status_timestamp DESC`, served by `idx_tasks_from_agent_status_ts`), excluding `broadcast_summary` type tasks. Ordered newest first.

**Request**: `X-Session-Id: <session_id>` header.

Same response format as inbox.

### GET /ui/api/timeline — Unified Session Timeline

Returns up to 200 most-recent non-`broadcast_summary` tasks for the selected session, newest first. Consumed by the Discord-style admin dashboard, which groups delivery rows sharing an `origin_task_id` into a single broadcast entry client-side.

**Request**: `X-Session-Id: <session_id>` header.

Session scoping is reached through the `tasks.context_id → agents.agent_id → agents.session_id` join. Only tasks whose recipient belongs to the header session are returned; cross-session tasks are invisible.

**Response** (200 OK):

```json
{
  "messages": [
    {
      "task_id": "uuid",
      "from_agent_id": "uuid",
      "from_agent_name": "Claude-A",
      "to_agent_id": "uuid",
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

**Row cap**: Hard-capped at 200 rows. No pagination in the first cut.

**Exclusions**: Rows with `type == "broadcast_summary"` are filtered out of the response. The summary row's metadata (`recipientIds`) is not needed for the UI; the grouping convention below lets the frontend reconstruct broadcasts from their delivery rows alone.

**Broadcast grouping**: Every row carries an `origin_task_id` field:

| Case | `origin_task_id` |
|---|---|
| Unicast delivery | `null` |
| Broadcast delivery | The broadcast's summary task id (shared across all N delivery rows in the same broadcast) |
| Historical row from before the `origin_task_id` migration | `null` |

The client groups rows by `origin_task_id` (non-null rows sharing a value form one broadcast entry; null rows are standalone unicast entries). Each broadcast entry's sort key is the `MIN(created_at)` of its rows — stable, so a broadcast never drifts when a lagging recipient ACKs.

**ACK timestamps**: Per-recipient ACK time is read from the `status_timestamp` of a `completed` delivery row. Delivery tasks make exactly one state transition over their lifetime (`input_required → completed` on ACK), so for `status == "completed"` rows `status_timestamp` IS the ACK moment. If this invariant is ever broken by a future change, the timeline will silently show wrong ACK times until a dedicated `acknowledged_at` column is added. See `docs/spec/data-model.md` for the accompanying design-debt note.

### POST /ui/api/messages/send — Send Message

Sends a message from a same-session active agent. Supports both unicast (`to_agent_id=<uuid>`) and broadcast (`to_agent_id="*"`).

**Request**:

```
X-Session-Id: <session_id>
```

```json
{
  "from_agent_id": "uuid",
  "to_agent_id": "uuid | *",
  "text": "Hello!"
}
```

**Unicast** (`to_agent_id` is a UUID): the server verifies both the sender and the destination belong to the caller's session and that the destination is active.

**Broadcast** (`to_agent_id == "*"`): the server skips destination validation (no specific recipient to verify) and the WebUI route calls `broker.broadcast_message(...)`, which fans out to every active agent in the session (except the built-in Administrator, which is filtered out of the recipient set at the broker layer) plus a summary task. The sender is still required to be active and in the caller's session; the sender MAY be the Administrator. The response's `task_id` is the summary task's id.

**Sender identity**: The Admin WebUI always submits `from_agent_id = administrator.agent_id` (the session's built-in Administrator). The endpoint itself is sender-agnostic — it accepts any active agent in the session — but no UI path lets the operator pick a different sender.

**Response** (200 OK):

```json
{
  "task_id": "uuid",
  "status": "input_required"
}
```

**Errors**:
- 400: Missing fields, `from_agent` not in session, destination is deregistered
- 404: Agent not found or cross-session
- 409 (reserved for future deregister endpoint): for any future endpoint that attempts to deregister or otherwise modify the built-in Administrator, the broker's `AdministratorProtectedError` must be translated to `raise HTTPException(status_code=409, detail=...)`. This 409 is not currently reachable through `POST /ui/api/messages/send` and the WebUI router does not yet register an exception handler for `AdministratorProtectedError`; this entry documents the required mapping for the future endpoint.

## Error Format

WebUI API errors use a simple JSON format:

```json
{"error": "Error message"}
```

Or FastAPI's default validation error format for 422 responses.
