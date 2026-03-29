# Redis Data Model Specification

All core data structures — `Task`, `Message`, `Part`, `Artifact`, `AgentCard`, `TaskStatus`, `TaskState` — use types defined by the A2A specification via `a2a-sdk` Pydantic models. No Broker-specific data models are created. Redis stores `a2a-sdk` Task objects as JSON, deserialized back to Pydantic models on read. Broker-specific information (routing metadata etc.) is stored in the Task's `metadata` field.

## Redis Key Schema

| Key Pattern | Type | Description | TTL |
|---|---|---|---|
| `agent:{agent_id}` | Hash | Agent metadata + serialized Agent Card. The `api_key_hash` field serves as `tenant_id`. | None |
| `apikey:{api_key_hash}` | Hash | API key metadata: `owner_sub`, `created_at`, `status`, `key_prefix`. Source of truth for key existence and validity. | None |
| `account:{auth0_sub}:keys` | Set | Set of `api_key_hash` values owned by this Auth0 account. Used for key listing and ownership validation. | None |
| `agents:active` | Set | Set of active agent_id UUIDs. **Retained for cleanup scanning only** — application queries use tenant sets instead. | None |
| `tenant:{api_key_hash}:agents` | Set | Set of active agent_ids belonging to this tenant. Updated on registration (SADD) and deregistration (SREM). This is the canonical source for tenant membership. | None |
| `task:{task_id}` | Hash | Full A2A Task JSON + routing metadata | None |
| `tasks:ctx:{context_id}` | Sorted Set | task_ids scored by status timestamp (updated on state change) | None |
| `tasks:sender:{agent_id}` | Set | task_ids created by this sender | None |

## Agent Record (`agent:{agent_id}`)

```json
{
  "agent_id": "uuid-v4",
  "api_key_hash": "sha256-hex",
  "name": "string",
  "description": "string",
  "agent_card_json": "serialized A2A AgentCard JSON",
  "status": "active | deregistered",
  "registered_at": "ISO 8601",
  "deregistered_at": "ISO 8601 | null"
}
```

## API Key Record (`apikey:{api_key_hash}`)

```json
{
  "owner_sub": "auth0|abc123",
  "created_at": "ISO 8601",
  "status": "active | revoked",
  "key_prefix": "hky_a1b2"
}
```

- `owner_sub`: Auth0 `sub` claim of the user who created the key
- `created_at`: Timestamp when the key was created via the WebUI
- `status`: `"active"` (valid for use) or `"revoked"` (key is disabled; all agent requests using this key are rejected with 401)
- `key_prefix`: First 8 characters of the raw API key, for display in the WebUI key list

The `apikey:{hash}` record is the source of truth for key existence and validity. Every authenticated request (both agent-to-broker and WebUI tenant-scoped) checks this record. Revoking a key sets `status` to `"revoked"` and deregisters all agents in `tenant:{hash}:agents`.

## Task Record (`task:{task_id}`)

```json
{
  "task_json": "serialized A2A Task object (id, contextId, status, artifacts, history, metadata)",
  "from_agent_id": "uuid-v4",
  "to_agent_id": "uuid-v4",
  "type": "unicast | broadcast | broadcast_summary",
  "created_at": "ISO 8601"
}
```

The `task_json` field contains the canonical A2A Task object. The additional fields (`from_agent_id`, `to_agent_id`, `type`) are Broker-internal routing metadata used for authorization checks.

For broadcast, a separate Task is created per recipient to ensure independent ACK and per-pair FIFO ordering.

## Sorted Set Indexing

The `tasks:ctx:{context_id}` sorted set indexes tasks by status timestamp (not creation time). The score is updated on every state change, ensuring `ListTasks` returns results in the A2A-required descending status timestamp order.

This enables efficient inbox queries:
- `ListTasks(contextId=agentB-uuid)` → `ZREVRANGEBYSCORE tasks:ctx:agentB-uuid`
- Status filtering is applied after retrieval from the sorted set
- Pagination uses `pageSize` with cursor-based offset

## Tenant Lifecycle

Tenants are created when a user creates an API key via the WebUI. The `apikey:{hash}` record is the source of truth for tenant existence. Agents join the tenant by registering with the API key.

When all agents in a tenant deregister, the `tenant:{api_key_hash}:agents` set becomes empty, but the tenant remains valid — new agents can still register using the API key as long as its `apikey:{hash}` status is `"active"`.

Revoking a key (via `DELETE /ui/api/keys/{tenant_id}`) sets the status to `"revoked"` and deregisters all agents. A revoked key cannot be used for agent registration or authentication.

## Task Visibility Rules

| Caller | Can Access |
|---|---|
| Recipient (same-tenant agent matching `to_agent_id`) | `ListTasks` by contextId (= their agent_id), `GetTask`, `SendMessage` (ACK) |
| Sender (same-tenant agent matching `from_agent_id`) | `GetTask` by known taskId, `CancelTask` |

`ListTasks` enforces that `contextId` must equal the caller's `agent_id`. If a different `contextId` is provided, the Broker returns an error. This prevents inbox snooping — even within the same tenant. `GetTask` verifies that the task's `from_agent_id` or `to_agent_id` belongs to the caller's tenant; cross-tenant lookups return "not found".

## Deregistered Agent Cleanup

A periodic background task cleans up Task data for agents that have been deregistered longer than `DEREGISTERED_TASK_TTL_DAYS`.

**Trigger**: `asyncio` periodic task launched via FastAPI's `lifespan` event. Runs every hour.

**Behavior**:

1. Scan `agent:*` hashes for records where `status == "deregistered"` and `deregistered_at` is older than the retention period
2. For each expired agent:
   - Delete all `task:{task_id}` hashes referenced in `tasks:ctx:{agent_id}`
   - Delete `tasks:ctx:{agent_id}` sorted set
   - Remove task_ids from relevant `tasks:sender:*` sets
   - Delete `agent:{agent_id}` hash
3. Log the number of cleaned-up agents per run

The scan uses Redis `SCAN` (not `KEYS`) to avoid blocking.

### Configuration

| Configuration | Default | Description |
|---|---|---|
| `DEREGISTERED_TASK_TTL_DAYS` | `7` | Retention period before cleanup |
| `CLEANUP_INTERVAL_SECONDS` | `3600` | How often the cleanup task runs |
