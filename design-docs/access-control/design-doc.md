# Access Control: Tenant Isolation via Shared API Key

**Status**: Complete
**Progress**: 33/33 tasks complete
**Last Updated**: 2026-03-28

## Overview

Add tenant isolation to the Hikyaku message broker by making the API key a shared tenant credential. Agents that register with the same API key belong to the same tenant and can communicate with each other; agents in different tenants are invisible to one another.

## Success Criteria

- [ ] Multiple agents can register under the same API key (shared tenant key)
- [ ] First registration (no auth) creates a new tenant and returns a fresh API key
- [ ] Subsequent registrations (with auth) join the existing tenant and receive the same API key
- [ ] `GET /api/v1/agents` returns only agents in the caller's tenant
- [ ] `GET /api/v1/agents/{id}` returns 404 for agents in a different tenant
- [ ] `SendMessage` (unicast) fails with "agent not found" for cross-tenant destinations
- [ ] `SendMessage` (broadcast `*`) delivers only to same-tenant agents
- [ ] `GetTask` returns "not found" for tasks involving agents in a different tenant
- [ ] Cross-tenant agents cannot infer each other's existence from any API response

---

## Background

The current Hikyaku broker assigns each agent a unique API key at registration. The API key serves dual duty: authentication (prove you are a valid agent) and identity resolution (the key maps 1:1 to an agent_id). All agents share a single global namespace — any agent can discover, message, or broadcast to any other agent.

This is fine for trusted single-team scenarios but insufficient when multiple independent groups of agents share the same broker instance. Without isolation, Agent A from Team X can see and message Agent B from Team Y, which is undesirable.

**Design choice**: Rather than introducing a separate "tenant" entity with its own CRUD lifecycle, the API key itself becomes the tenant boundary. All agents sharing the same API key form a tenant. This keeps the conceptual model simple — the API key is the only credential agents need — and avoids adding tenant management endpoints.

**Key consequence**: Since multiple agents share one API key, the API key alone no longer identifies a specific agent. Authenticated requests must now provide both the API key (in the `Authorization` header) and the agent_id (in a new `X-Agent-Id` header). The server verifies that the agent_id belongs to the tenant identified by the API key hash.

**Breaking change**: This change removes the `apikey:{hash}` → agent_id mapping from Redis, changes the auth flow to require `X-Agent-Id` header, and restructures tenant membership into a new key. Existing agent registrations will be invalidated. All agents must re-register after this change is deployed.

---

## Specification

### Authentication Model Change

**Current**: `Authorization: Bearer <api_key>` → server resolves to a single `agent_id`.

**New**: Two pieces of identity are required for authenticated requests:

| Header | Purpose |
|---|---|
| `Authorization: Bearer <api_key>` | Authenticates the tenant (API key hash = tenant_id) |
| `X-Agent-Id: <agent_id>` | Identifies the specific agent within the tenant |

The server verifies that the agent record for `<agent_id>` has an `api_key_hash` matching `SHA256(<api_key>)`. If not, the request is rejected with 401.

**Registration is the exception**: `POST /api/v1/agents` does not require `X-Agent-Id` (the agent doesn't exist yet). It optionally accepts `Authorization: Bearer <api_key>` to join an existing tenant.

### Registration Flow

Two modes, determined by presence of `Authorization` header:

**Create new tenant** (no `Authorization` header):

1. Client: `POST /api/v1/agents` with body `{name, description, skills?}`
2. Server generates: `agent_id` (UUID v4), `api_key` (`hky_<random>`), `api_key_hash` = SHA256(api_key)
3. Server stores agent record and adds to tenant set
4. Response: `{agent_id, api_key, name, registered_at}`

**Join existing tenant** (`Authorization: Bearer <api_key>` provided):

1. Client: `POST /api/v1/agents` with body `{name, description, skills?}` and `Authorization: Bearer <api_key>`
2. Server computes `api_key_hash` = SHA256(api_key)
3. Server verifies tenant exists: `tenant:{api_key_hash}:agents` is non-empty. If empty/missing → 401 "Invalid API key"
4. Server generates: `agent_id` (UUID v4). Reuses the provided `api_key`.
5. Server stores agent record and adds to tenant set
6. Response: `{agent_id, api_key, name, registered_at}` (api_key echoed back)

### Redis Key Schema Changes

New keys added, one key repurposed:

| Key Pattern | Type | Change | Description |
|---|---|---|---|
| `agent:{agent_id}` | Hash | Unchanged | Agent metadata; `api_key_hash` field already present, now serves as tenant_id |
| `apikey:{sha256(key)}` | String | **Removed** | No longer maps to single agent_id. Replaced by tenant set membership check. |
| `agents:active` | Set | **Retained (write-only for reads)** | Global set of all active agent_ids. No longer read by application queries (replaced by tenant sets), but retained for cleanup scanning which needs to iterate all agents regardless of tenant. |
| `tenant:{api_key_hash}:agents` | Set | **New** | Set of active agent_ids belonging to this tenant |
| `task:{task_id}` | Hash | Unchanged | Full A2A Task JSON + routing metadata |
| `tasks:ctx:{context_id}` | Sorted Set | Unchanged | task_ids scored by timestamp |
| `tasks:sender:{agent_id}` | Set | Unchanged | task_ids created by this sender |

The `tenant:{api_key_hash}:agents` set is the canonical source for tenant membership. It is updated on registration (SADD) and deregistration (SREM).

**Tenant lifecycle**: Tenants are ephemeral. When the last agent in a tenant deregisters, the tenant set becomes empty. At that point, no new agent can join using that API key (the join flow rejects empty/missing tenant sets). The API key effectively dies with the last agent. To re-create the tenant, an agent must register without auth to get a fresh API key. This is intentional — it prevents stale API keys from being reused indefinitely after all agents have left.

### Tenant Isolation Rules

Every operation that reads or writes agent/task data enforces tenant isolation:

| Operation | Isolation Enforcement |
|---|---|
| `GET /api/v1/agents` | Query `tenant:{api_key_hash}:agents` instead of `agents:active` |
| `GET /api/v1/agents/{id}` | Verify target agent's `api_key_hash` matches caller's. Return 404 if mismatch. |
| `DELETE /api/v1/agents/{id}` | Existing ownership check (caller = target) already implies same tenant |
| `SendMessage` (unicast) | Verify destination agent's `api_key_hash` matches sender's. Return "agent not found" JSON-RPC error if mismatch. |
| `SendMessage` (broadcast `*`) | Query `tenant:{api_key_hash}:agents` for recipient list instead of all active agents |
| `SendMessage` (ACK) | Existing check (contextId = agent_id) already implies same tenant |
| `ListTasks` | **New check needed**: enforce `contextId == caller's agent_id` in `_handle_list_tasks`. The current code does NOT verify this — it accepts any contextId, allowing inbox snooping. This fix closes a pre-existing authorization gap and ensures tenant isolation. |
| `GetTask` | Verify task's `from_agent_id` or `to_agent_id` belongs to caller's tenant. Return "not found" if neither matches. |
| `CancelTask` | Existing sender ownership check already implies same tenant |

**Information leakage rule**: Cross-tenant violations always produce the same error as "resource does not exist" (404 for REST, "not found" JSON-RPC error). The caller cannot distinguish between "agent/task exists in another tenant" and "agent/task does not exist at all."

### Affected Files

| File | Changes |
|---|---|
| `registry/src/hikyaku_registry/auth.py` | Return `(agent_id, tenant_id)` tuple; verify agent-tenant membership; handle registration (no `X-Agent-Id` required) |
| `registry/src/hikyaku_registry/registry_store.py` | `create_agent` accepts optional `api_key`; `list_active_agents` takes `tenant_id` param and queries tenant set; `deregister_agent` removes from tenant set; remove `lookup_by_api_key` |
| `registry/src/hikyaku_registry/executor.py` | Accept `tenant_id` in context state; verify destination agent tenant in unicast; scope broadcast to tenant |
| `registry/src/hikyaku_registry/main.py` | Extract `X-Agent-Id` header; pass `tenant_id` to executor via `ServerCallContext`; update auth dependency |
| `registry/src/hikyaku_registry/api/registry.py` | Registration handles both flows; list/get agents use tenant-scoped queries; auth dependency returns tuple |
| `registry/src/hikyaku_registry/cleanup.py` | Also remove agent from `tenant:{hash}:agents` on cleanup |
| `registry/src/hikyaku_registry/models.py` | No changes needed (response models unchanged) |
| `client/src/hikyaku_client/api.py` | Add `X-Agent-Id` header to all authenticated requests; registration optionally sends API key |
| `client/src/hikyaku_client/cli.py` | `register` command: add `--api-key` option for joining existing tenant |

### Client-Side Changes

The CLI and API client must send `X-Agent-Id` header on all authenticated requests. Example:

```python
headers = {
    "Authorization": f"Bearer {api_key}",
    "X-Agent-Id": agent_id,
}
```

The `register` command gains an optional `--api-key` flag:

```bash
# Create new tenant (new API key generated)
hikyaku register --name "Agent A" --description "My agent"

# Join existing tenant (use shared API key)
hikyaku register --name "Agent B" --description "My agent" --api-key "hky_abc123..."
```

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

### Step 0: Documentation Updates

- [x] Update `docs/spec/registry-api.md`: document join-tenant registration flow (`Authorization` header on `POST /api/v1/agents`), add `X-Agent-Id` header requirement to authentication section, update error cases for cross-tenant 404s <!-- completed: 2026-03-28T12:00 -->
- [x] Update `docs/spec/a2a-operations.md`: document tenant-scoped SendMessage/broadcast behavior, add `X-Agent-Id` header requirement, document ListTasks `contextId` enforcement (must equal caller's agent_id) <!-- completed: 2026-03-28T12:00 -->
- [x] Update `docs/spec/data-model.md`: add `tenant:{api_key_hash}:agents` key, mark `apikey:{hash}` as removed, note `agents:active` is retained for cleanup only <!-- completed: 2026-03-28T12:00 -->
- [x] Update `ARCHITECTURE.md`: add tenant isolation to architecture description, update diagram to show tenant boundary, note `X-Agent-Id` header in request flow <!-- completed: 2026-03-28T12:00 -->

### Step 1: Auth Module Changes

- [x] Update `get_authenticated_agent` in `auth.py` to extract both `Authorization` and `X-Agent-Id` headers <!-- completed: 2026-03-28T12:10 -->
- [x] Return `(agent_id, tenant_id)` tuple where `tenant_id = api_key_hash` <!-- completed: 2026-03-28T12:10 -->
- [x] Verify agent record's `api_key_hash` matches the provided API key's hash <!-- completed: 2026-03-28T12:10 -->
- [x] Add `get_registration_tenant` helper that extracts optional `Authorization` header for registration flow (returns `api_key` and `api_key_hash` or `None`) <!-- completed: 2026-03-28T12:10 -->
- [x] Update `test_auth.py` with tests for new auth flow (shared key, missing X-Agent-Id, mismatched tenant) <!-- completed: 2026-03-28T12:10 -->

### Step 2: Registry Store Changes

- [x] Add `tenant_id` (api_key_hash) parameter to `create_agent`; accept optional `api_key` for join-tenant flow <!-- completed: 2026-03-28T12:20 -->
- [x] In `create_agent`, add agent to `tenant:{api_key_hash}:agents` set <!-- completed: 2026-03-28T12:20 -->
- [x] Update `list_active_agents` to accept `tenant_id` param and query `tenant:{tenant_id}:agents` set <!-- completed: 2026-03-28T12:20 -->
- [x] Update `deregister_agent` to also SREM from `tenant:{api_key_hash}:agents` <!-- completed: 2026-03-28T12:20 -->
- [x] Remove `lookup_by_api_key` method (no longer needed) <!-- completed: 2026-03-28T12:20 -->
- [x] Add `verify_agent_tenant(agent_id, tenant_id)` method to check membership <!-- completed: 2026-03-28T12:20 -->
- [x] Update `test_registry_store.py` with multi-tenant tests <!-- completed: 2026-03-28T12:20 -->

### Step 3: Executor Changes

- [x] Add `tenant_id` to `ServerCallContext.state` alongside `agent_id` <!-- completed: 2026-03-28T12:30 -->
- [x] In `_handle_unicast`, verify destination agent's `api_key_hash` matches sender's tenant. Return "agent not found" error on mismatch. <!-- completed: 2026-03-28T12:30 -->
- [x] In `_handle_broadcast`, query `tenant:{tenant_id}:agents` for recipient list instead of all active agents <!-- completed: 2026-03-28T12:30 -->
- [x] In `_handle_ack` and `cancel`, no changes needed (existing ownership checks suffice) <!-- completed: 2026-03-28T12:30 -->
- [x] Update `test_executor.py` with cross-tenant rejection tests <!-- completed: 2026-03-28T12:30 -->

### Step 4: API Route & Main App Changes

- [x] Update `register_agent` endpoint to handle both create-tenant and join-tenant flows based on `Authorization` header presence <!-- completed: 2026-03-28T13:00 -->
- [x] Update `list_agents` and `get_agent_detail` to use tenant-scoped queries (reject cross-tenant lookups with 404) <!-- completed: 2026-03-28T13:00 -->
- [x] Update `jsonrpc_endpoint` in `main.py` to extract `X-Agent-Id` header, verify tenant membership, pass `tenant_id` to executor <!-- completed: 2026-03-28T13:00 -->
- [x] Update `_handle_list_tasks` in `main.py` to enforce `contextId == caller's agent_id` (fixes pre-existing authorization gap where any agent could query another's inbox) <!-- completed: 2026-03-28T13:00 -->
- [x] Update `_handle_get_task` to verify task belongs to caller's tenant <!-- completed: 2026-03-28T13:00 -->
- [x] Update `test_registry_api.py` with tenant isolation tests <!-- completed: 2026-03-28T13:00 -->

### Step 5: Cleanup Changes

- [x] Update `cleanup_expired_agents` to also remove agent from `tenant:{api_key_hash}:agents` set <!-- completed: 2026-03-28T13:00 -->
- [x] Update `test_cleanup.py` to verify tenant set cleanup <!-- completed: 2026-03-28T13:00 -->

### Step 6: Client Changes

- [x] Update all authenticated requests in `api.py` to include `X-Agent-Id` header <!-- completed: 2026-03-28T13:00 -->
- [x] Update `register_agent` in `api.py` to optionally send `Authorization: Bearer <api_key>` header <!-- completed: 2026-03-28T13:00 -->
- [x] Update `register` command in `cli.py` to accept optional `--api-key` flag for join-tenant flow <!-- completed: 2026-03-28T13:00 -->
- [x] Update `test_cli.py` with join-tenant registration test <!-- completed: 2026-03-28T13:00 -->

---

## Changelog (optional -- spec revisions only, not implementation progress)

| Date | Changes |
|------|---------|
| 2026-03-28 | Initial draft |
| 2026-03-28 | Approved after review. |
| 2026-03-28 | Implementation complete. 33/33 tasks done. 277 tests passing. Status → Complete. |
