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

**Request with placement** (with `Authorization: Bearer <api_key>` + `X-Agent-Id: <director_id>` headers):

When `placement` is present, the server atomically creates both the agent and its placement row. `X-Agent-Id` must be set and equal `placement.director_agent_id`.

```json
{
  "name": "Claude-B",
  "description": "Reviewer bot",
  "skills": [],
  "placement": {
    "director_agent_id": "<director-uuid>",
    "tmux_session": "main",
    "tmux_window_id": "@3",
    "tmux_pane_id": null
  }
}
```

**Response** (201 Created):

```json
{
  "agent_id": "550e8400-e29b-41d4-a716-446655440000",
  "api_key": "hky_a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4",
  "name": "My Coding Agent",
  "registered_at": "2026-03-28T12:00:00Z",
  "placement": null
}
```

When registered with a placement, the `placement` field is populated with the placement view.

The `api_key` in the response is the same key provided in the `Authorization` header (echoed back for convenience). The Broker stores only the SHA-256 hash. The Broker constructs a full A2A `AgentCard` from the registration data, setting `supportedInterfaces` to point back to the Broker itself.

**Validation**: The API key must have a row in `api_keys` with `status='active'` (created via WebUI). If no `Authorization` header is provided, or the key is revoked/unknown, the server returns 401 Unauthorized. When `placement` is present, `X-Agent-Id` must be set and equal `placement.director_agent_id`; `director_agent_id` must be a valid active agent in the caller's tenant.

**Error**: 401 if `Authorization` header is missing, the key has no row in `api_keys`, or its status is not `'active'`. 403 if placement has a cross-tenant `director_agent_id`.

### GET /api/v1/agents — List Agents

Requires authentication (`Authorization` + `X-Agent-Id` headers).

Returns only agents belonging to the caller's tenant. Agents in other tenants are not visible.

**Query parameters**:
- `director_agent_id` (optional): Filter to agents whose placement row has this Director. Used by `hikyaku member list`.

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

Requires authentication. The caller must be either the agent itself OR the Director that spawned the agent (i.e., `caller_id == agent_id` OR `caller_id == placement.director_agent_id`).

**Behavior**:

1. Single SQL `UPDATE`: set `status='deregistered'` and `deregistered_at=<now>` on the `agents` row, gated on `status='active'`.
2. Hard-delete any `agent_placements` row where `agent_id` matches, in the same transaction.
3. The agent row is **not** physically deleted. There is no background cleanup loop in v1; deregistered rows persist indefinitely so the WebUI can surface their inbox history.
4. All active query paths filter `status='active'`, so the deregistered agent is invisible to normal A2A traffic immediately.

**Response**: 204 No Content.

**Error**: 403 if caller is neither the agent itself nor its Director. 404 if agent not found.

### PATCH /api/v1/agents/{agent_id}/placement — Update Placement Pane ID

Requires authentication. Caller must be the Director of this placement (`X-Agent-Id == placement.director_agent_id`).

Used by the two-pass `member create` flow: after `tmux split-window` captures the new pane ID, the CLI patches the pending placement row.

**Request** (with `Authorization: Bearer <api_key>` + `X-Agent-Id: <director_id>` headers):

```json
{
  "tmux_pane_id": "%7"
}
```

**Response** (200 OK):

```json
{
  "director_agent_id": "<director-uuid>",
  "tmux_session": "main",
  "tmux_window_id": "@3",
  "tmux_pane_id": "%7",
  "created_at": "2026-04-12T10:15:00Z"
}
```

**Error**: 403 if caller is not the Director. 404 if no placement exists.

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
