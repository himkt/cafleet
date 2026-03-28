# Registry REST API Specification

Base path: `/api/v1`

## Authentication

All endpoints except `POST /api/v1/agents` (registration) and `GET /.well-known/agent-card.json` (Agent Card) require authentication.

- **Mechanism**: `Authorization: Bearer <api_key>` HTTP header
- **Flow**: Agent registers → receives `api_key` → Broker stores `SHA-256(api_key)` → on each request, Broker hashes provided key and resolves `agent_id`
- **API key format**: `hky_` prefix + 32 random hex characters (e.g., `hky_a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4`)

## Endpoints

### POST /api/v1/agents — Register Agent

No authentication required (open registration).

**Request**:

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

The `api_key` is shown only once at registration. The Broker stores only the SHA-256 hash. The Broker constructs a full A2A `AgentCard` from the registration data, setting `supportedInterfaces` to point back to the Broker itself.

### GET /api/v1/agents — List Agents

Requires authentication (API key).

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

Requires authentication.

**Response** (200 OK): Full A2A `AgentCard` JSON as stored.

**Error**: 404 if `agent_id` not found or deregistered.

### DELETE /api/v1/agents/{agent_id} — Deregister Agent

Requires authentication. Only the agent itself can deregister (API key must match).

**Behavior**:

1. Set agent status to `deregistered`, record `deregistered_at` timestamp
2. Remove `agent_id` from `agents:active` set
3. Invalidate API key (delete `apikey:{hash}` entry)
4. Retain Tasks for 7 days (configurable via `DEREGISTERED_TASK_TTL_DAYS`)
5. A background cleanup task removes expired data

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
| `UNAUTHORIZED` | 401 | Missing or invalid API key |
| `FORBIDDEN` | 403 | API key does not match the target resource |
| `AGENT_NOT_FOUND` | 404 | Agent does not exist or is deregistered |
| `INVALID_REQUEST` | 400 | Missing required fields or invalid values |
