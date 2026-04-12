# Registry REST API Specification

Base path: `/api/v1`

## Prerequisites

The Broker server stores all data in a SQLite database accessed through SQLAlchemy + Alembic. Before starting the server for the first time, the operator must apply the schema:

```bash
hikyaku-registry db init
```

This is idempotent — running it on a database that is already at head is a no-op. Without it, the first request fails with `OperationalError: no such table: agents`. See `data-model.md` and `cli-options.md` for details.

## Request Headers

The broker does not perform authentication. Endpoints use a combination of headers, body fields, and query parameters for session and agent identification:

| Header | Purpose |
|---|---|
| `X-Session-Id: <session_id>` | Selects the session namespace (used by `GET /agents/{id}`) |
| `X-Agent-Id: <agent_id>` | Identifies the specific agent (used by `DELETE /agents/{id}`, `PATCH /agents/{id}/placement`) |

Some endpoints accept `session_id` in the request body (`POST /agents`) or as a query parameter (`GET /agents`).

## Endpoints

### POST /api/v1/agents — Register Agent

Registration requires a valid `session_id` in the request body. The session must exist in the `sessions` table.

**Request** (no auth headers required):

```json
{
  "session_id": "550e8400-e29b-41d4-a716-446655440000",
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

**Request with placement** (with `X-Agent-Id: <director_id>` header):

When `placement` is present, the server atomically creates both the agent and its placement row. `X-Agent-Id` must be set and equal `placement.director_agent_id`.

```json
{
  "session_id": "550e8400-e29b-41d4-a716-446655440000",
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
  "name": "My Coding Agent",
  "registered_at": "2026-03-28T12:00:00Z",
  "placement": null
}
```

When registered with a placement, the `placement` field is populated with the placement view.

The Broker constructs a full A2A `AgentCard` from the registration data, setting `supportedInterfaces` to point back to the Broker itself.

**Validation**: The `session_id` must exist in the `sessions` table. When `placement` is present, `X-Agent-Id` must be set and equal `placement.director_agent_id`; `director_agent_id` must be a valid active agent in the same session.

**Error**: 404 `SESSION_NOT_FOUND` if session does not exist. 400 `SESSION_REQUIRED` if `session_id` is missing from the body. 400 `INVALID_REQUEST` for other validation failures.

### GET /api/v1/agents — List Agents

Requires `session_id` as a query parameter. Returns only agents belonging to the specified session.

**Query parameters**:
- `session_id` (required): The session namespace to list agents from.
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

Requires `X-Session-Id` header. Returns 404 if the agent does not exist or does not belong to the specified session.

**Response** (200 OK): Full A2A `AgentCard` JSON as stored.

**Error**: 404 if `agent_id` not found, deregistered, or belongs to a different session. Cross-session lookups always return 404 (indistinguishable from "not found") to keep sessions structurally isolated.

### DELETE /api/v1/agents/{agent_id} — Deregister Agent

Requires `X-Agent-Id` header. The caller must be either the agent itself OR the Director that spawned the agent (i.e., `caller_id == agent_id` OR `caller_id == placement.director_agent_id`). Session is not re-verified.

**Behavior**:

1. Single SQL `UPDATE`: set `status='deregistered'` and `deregistered_at=<now>` on the `agents` row, gated on `status='active'`.
2. Hard-delete any `agent_placements` row where `agent_id` matches, in the same transaction.
3. The agent row is **not** physically deleted. There is no background cleanup loop in v1; deregistered rows persist indefinitely so the WebUI can surface their inbox history.
4. All active query paths filter `status='active'`, so the deregistered agent is invisible to normal A2A traffic immediately.

**Response**: 204 No Content.

**Error**: 403 if caller is neither the agent itself nor its Director. 404 if agent not found.

### PATCH /api/v1/agents/{agent_id}/placement — Update Placement Pane ID

Requires `X-Agent-Id` header. Caller must be the Director of this placement (`X-Agent-Id == placement.director_agent_id`).

Used by the two-pass `member create` flow: after `tmux split-window` captures the new pane ID, the CLI patches the pending placement row.

**Request** (with `X-Agent-Id: <director_id>` header):

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
| `SESSION_REQUIRED` | 400 | Missing `session_id` from required header, body, or query parameter |
| `SESSION_NOT_FOUND` | 404 | `session_id` does not exist in the `sessions` table |
| `AGENT_ID_REQUIRED` | 400 | Missing `X-Agent-Id` header on an endpoint that requires it |
| `AGENT_NOT_FOUND` | 404 | Agent does not exist, is deregistered, or belongs to a different session |
| `INVALID_REQUEST` | 400 | Missing required fields or invalid values |
