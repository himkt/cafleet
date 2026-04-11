# Registry REST API Specification

Base path: `/api/v1`

## Prerequisites

The Broker server stores all data in a SQLite database accessed through SQLAlchemy + Alembic. Before starting the server for the first time, the operator must apply the schema:

```bash
hikyaku-registry db init
```

This is idempotent — running it on a database that is already at head is a no-op. Without it, the first request fails with `OperationalError: no such table: agents`. See `data-model.md` and `cli-options.md` for details.

## Authentication

All endpoints except `POST /api/v1/agents` (registration) and `GET /.well-known/agent-card.json` (Agent Card) require authentication.

- **Mechanism**: Two headers are required on all authenticated requests:

| Header | Purpose |
|---|---|
| `Authorization: Bearer <api_key>` | Authenticates the tenant (`SHA-256(api_key)` = `tenant_id`) |
| `X-Agent-Id: <agent_id>` | Identifies the specific agent within the tenant |

- **Flow**: Agent registers with a pre-existing API key → receives `agent_id` → Broker stores `SHA-256(api_key)` as `api_key_hash` (= `tenant_id`) on the `agents` row → on each request, Broker hashes the provided key, checks the `api_keys` row has `status='active'`, verifies `agents.tenant_id` matches, and confirms agent-tenant membership.
- **API key format**: `hky_` prefix + 32 random hex characters (e.g., `hky_a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4`)
- **API key issuance**: Keys are created through the WebUI key management interface (requires Auth0 login). Keys are not generated during agent registration.
- **API key status check**: Every authenticated request verifies that the API key row has `status='active'`. Revoked keys immediately fail with 401.
- **Tenant model**: The API key is a shared tenant credential. All agents registered with the same API key belong to the same tenant and can discover and communicate with each other. Agents in different tenants are invisible to one another.
- **Registration**: `POST /api/v1/agents` requires `Authorization: Bearer <api_key>` (the key must exist and be active). The `X-Agent-Id` header is not required for registration (the agent doesn't exist yet).

## Endpoints

### POST /api/v1/agents — Register Agent

Registration always requires a valid API key. API keys are created through the WebUI key management interface (requires Auth0 login).

**Request** (with `Authorization: Bearer <api_key>` header):

```json
{
  "name": "My Coding Agent",
  "description": "A Claude Code agent specializing in Python",
  "skills": [
    {
      "id": "python-dev",
      "name": "Python Development",
      "description": "Writes and reviews Python code",
      "tags": ["python", "backend"]
    }
  ]
}
```

**Response** (201 Created):

```json
{
  "agent_id": "550e8400-e29b-41d4-a716-446655440000",
  "api_key": "hky_a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4",
  "name": "My Coding Agent",
  "registered_at": "2026-03-28T12:00:00Z"
}
```

The `api_key` in the response is the same key provided in the `Authorization` header (echoed back for convenience). The Broker stores only the SHA-256 hash. The Broker constructs a full A2A `AgentCard` from the registration data, setting `supportedInterfaces` to point back to the Broker itself.

**Validation**: The API key must have a row in `api_keys` with `status='active'` (created via WebUI). If no `Authorization` header is provided, or the key is revoked/unknown, the server returns 401 Unauthorized.

**Error**: 401 if `Authorization` header is missing, the key has no row in `api_keys`, or its status is not `'active'`.

### GET /api/v1/agents — List Agents

Requires authentication (`Authorization` + `X-Agent-Id` headers).

Returns only agents belonging to the caller's tenant. Agents in other tenants are not visible.

**Response** (200 OK):

```json
{
  "agents": [
    {
      "agent_id": "550e8400-...",
      "name": "My Coding Agent",
      "description": "A Claude Code agent specializing in Python",
      "skills": [
        {"id": "python-dev", "name": "Python Development", "tags": ["python", "backend"]}
      ],
      "registered_at": "2026-03-28T12:00:00Z"
    }
  ]
}
```

### GET /api/v1/agents/{agent_id} — Get Agent Detail

Requires authentication (`Authorization` + `X-Agent-Id` headers).

**Response** (200 OK): Full A2A `AgentCard` JSON as stored.

**Error**: 404 if `agent_id` not found, deregistered, or belongs to a different tenant. Cross-tenant lookups always return 404 (indistinguishable from "not found") to prevent information leakage.

### DELETE /api/v1/agents/{agent_id} — Deregister Agent

Requires authentication. Only the agent itself can deregister (API key must match).

**Behavior**:

1. Single SQL `UPDATE`: set `status='deregistered'` and `deregistered_at=<now>` on the `agents` row, gated on `status='active'`.
2. The row is **not** physically deleted. There is no background cleanup loop in v1; deregistered rows persist indefinitely so the WebUI can surface their inbox history.
3. All active query paths filter `status='active'`, so the deregistered agent is invisible to normal A2A traffic immediately.

**Response**: 204 No Content.

**Error**: 403 if API key does not match `agent_id`. 404 if agent not found.

## Error Format

REST API errors use a consistent JSON format:

```json
{
  "error": {
    "code": "AGENT_NOT_FOUND",
    "message": "Agent with id '...' not found"
  }
}
```

## Error Codes

| Error Code | HTTP Status | Description |
|---|---|---|
| `UNAUTHORIZED` | 401 | Missing or invalid API key, missing `X-Agent-Id` header, or agent-tenant membership mismatch |
| `FORBIDDEN` | 403 | API key does not match the target resource |
| `AGENT_NOT_FOUND` | 404 | Agent does not exist, is deregistered, or belongs to a different tenant |
| `INVALID_REQUEST` | 400 | Missing required fields or invalid values |
