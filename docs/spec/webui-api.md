# WebUI API Specification

Base path: `/ui/api`

## Authentication

The WebUI uses two authentication mechanisms:

- **Auth0 JWT**: `Authorization: Bearer <auth0_jwt>` — required on all endpoints except `GET /ui/api/auth/config`. Validated server-side via Auth0 JWKS.
- **Tenant selection**: `X-Tenant-Id: <api_key_hash>` — required on tenant-scoped endpoints (agents, inbox, sent, send). The backend verifies that the `X-Tenant-Id` belongs to the authenticated Auth0 user by calling `RegistryStore.is_key_owner(api_key_hash, owner_sub)` (single `SELECT` against the `api_keys` table).

No server-side session. The Auth0 SPA SDK manages tokens in the browser.

## Endpoints

### GET /ui/api/auth/config — Auth0 Client Config

Returns Auth0 domain, client ID, and audience for SPA initialization. No authentication required.

**Response** (200 OK):

```json
{
  "domain": "myapp.auth0.com",
  "client_id": "abc123...",
  "audience": "hikyaku"
}
```

### POST /ui/api/keys — Create API Key

Creates a new API key owned by the authenticated Auth0 user. The raw key is shown only once.

**Request**: `Authorization: Bearer <auth0_jwt>` header only (no body).

**Response** (201 Created):

```json
{
  "api_key": "hky_a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4",
  "tenant_id": "sha256hex...",
  "created_at": "2026-03-29T10:00:00+00:00"
}
```

### GET /ui/api/keys — List API Keys

Lists all API keys owned by the authenticated Auth0 user. Does NOT return raw keys.

**Request**: `Authorization: Bearer <auth0_jwt>` header.

**Response** (200 OK):

```json
[
  {
    "tenant_id": "sha256hex...",
    "key_prefix": "hky_a1b2",
    "created_at": "2026-03-29T10:00:00+00:00",
    "status": "active",
    "agent_count": 3
  }
]
```

### DELETE /ui/api/keys/{tenant_id} — Revoke API Key

Revokes an API key and deregisters all agents under the tenant. The authenticated user must own the key.

**Request**: `Authorization: Bearer <auth0_jwt>` header.

**Response**: 204 No Content.

**Error**: 404 if `tenant_id` is not owned by the authenticated user.

### GET /ui/api/agents — List Agents

Returns agents belonging to the selected tenant.

**Request**: `Authorization: Bearer <auth0_jwt>` + `X-Tenant-Id: <api_key_hash>` headers.

**Response** (200 OK):

```json
{
  "agents": [...]
}
```

### GET /ui/api/agents/{agent_id}/inbox — Inbox Messages

Returns messages received by the agent (`context_id = agent_id`), excluding `broadcast_summary` type tasks. Ordered newest first.

**Request**: `Authorization: Bearer <auth0_jwt>` + `X-Tenant-Id: <api_key_hash>` headers.

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

**Request**: `Authorization: Bearer <auth0_jwt>` + `X-Tenant-Id: <api_key_hash>` headers.

Same response format as inbox.

### GET /ui/api/timeline — Unified Tenant Timeline

Returns up to 200 most-recent non-`broadcast_summary` tasks for the selected tenant, newest first. Consumed by the Discord-style admin dashboard, which groups delivery rows sharing an `origin_task_id` into a single broadcast entry client-side.

**Request**: `Authorization: Bearer <auth0_jwt>` + `X-Tenant-Id: <api_key_hash>` headers.

Tenant scoping is reached through the `tasks.context_id → agents.agent_id → agents.tenant_id` join. Only tasks whose recipient belongs to the header tenant are returned; cross-tenant tasks are invisible.

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

Sends a message from a same-tenant active agent. Supports both unicast (`to_agent_id=<uuid>`) and broadcast (`to_agent_id="*"`).

**Request**:

```
Authorization: Bearer <auth0_jwt>
X-Tenant-Id: <api_key_hash>
```

```json
{
  "from_agent_id": "uuid",
  "to_agent_id": "uuid | *",
  "text": "Hello!"
}
```

**Unicast** (`to_agent_id` is a UUID): the server verifies both the sender and the destination belong to the caller's tenant and that the destination is active.

**Broadcast** (`to_agent_id == "*"`): the server skips destination validation (no specific recipient to verify) and hands the message to `BrokerExecutor._handle_broadcast`, which fans out to every active agent in the tenant as individual delivery tasks plus a summary task. The sender is still required to be active and in the caller's tenant. The response's `task_id` is the summary task's id.

**Response** (200 OK):

```json
{
  "task_id": "uuid",
  "status": "input_required"
}
```

**Errors**:
- 401: Invalid API key
- 400: Missing fields, `from_agent` not in tenant, destination is deregistered
- 404: Agent not found or cross-tenant

## Error Format

WebUI API errors use a simple JSON format:

```json
{"error": "Error message"}
```

Or FastAPI's default validation error format for 422 responses.
