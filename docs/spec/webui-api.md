# WebUI API Specification

Base path: `/ui/api`

## Authentication

The WebUI uses two authentication mechanisms:

- **Auth0 JWT**: `Authorization: Bearer <auth0_jwt>` — required on all endpoints except `GET /ui/api/auth/config`. Validated server-side via Auth0 JWKS.
- **Tenant selection**: `X-Tenant-Id: <api_key_hash>` — required on tenant-scoped endpoints (agents, inbox, sent, send). The backend verifies that the `X-Tenant-Id` belongs to the authenticated Auth0 user by checking `account:{sub}:keys` set membership.

No server-side session. The Auth0 SPA SDK manages tokens in the browser.

## Endpoints

### GET /ui/api/auth/config — Auth0 Client Config

Returns Auth0 domain and client ID for SPA initialization. No authentication required.

**Response** (200 OK):

```json
{
  "domain": "myapp.auth0.com",
  "client_id": "abc123..."
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
{
  "keys": [
    {
      "tenant_id": "sha256hex...",
      "key_prefix": "hky_a1b2",
      "created_at": "2026-03-29T10:00:00+00:00",
      "status": "active",
      "agent_count": 3
    }
  ]
}
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

Returns messages sent by the agent (task IDs from `tasks:sender:{agent_id}`), excluding `broadcast_summary` type tasks. Ordered newest first.

**Request**: `Authorization: Bearer <auth0_jwt>` + `X-Tenant-Id: <api_key_hash>` headers.

Same response format as inbox.

### POST /ui/api/messages/send — Send Message

Sends a unicast message to a destination agent within the same tenant.

**Request**:

```
Authorization: Bearer <auth0_jwt>
X-Tenant-Id: <api_key_hash>
```

```json
{
  "from_agent_id": "uuid",
  "to_agent_id": "uuid",
  "text": "Hello!"
}
```

The server verifies both agents belong to the caller's tenant and that the destination is active.

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
