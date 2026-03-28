# Redis Data Model Specification

All core data structures — `Task`, `Message`, `Part`, `Artifact`, `AgentCard`, `TaskStatus`, `TaskState` — use types defined by the A2A specification via `a2a-sdk` Pydantic models. No Broker-specific data models are created. Redis stores `a2a-sdk` Task objects as JSON, deserialized back to Pydantic models on read. Broker-specific information (routing metadata etc.) is stored in the Task's `metadata` field.

## Redis Key Schema

| Key Pattern | Type | Description | TTL |
|---|---|---|---|
| `agent:{agent_id}` | Hash | Agent metadata + serialized Agent Card | None |
| `apikey:{sha256(key)}` | String | Maps hashed API key → agent_id | None |
| `agents:active` | Set | Set of active agent_id UUIDs | None |
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

## Task Visibility Rules

| Caller | Can Access |
|---|---|
| Recipient (API key maps to `to_agent_id`) | `ListTasks` by contextId (= their agent_id), `GetTask`, `SendMessage` (ACK) |
| Sender (API key maps to `from_agent_id`) | `GetTask` by known taskId, `CancelTask` |

`ListTasks` returns only tasks where the authenticated agent is the recipient (contextId match). If the provided `contextId` does not match the authenticated agent's ID, the Broker returns an empty task list (no error — consistent with A2A's rule that servers MUST NOT reveal existence of unauthorized resources). Senders track their tasks by the taskId returned from SendMessage.

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
