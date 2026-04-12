# Admin Discord-style Timeline

**Status**: Approved
**Progress**: 30/38 tasks complete
**Last Updated**: 2026-04-12

## Overview

Rewrite the admin SPA dashboard into a Discord-inspired layout — left sidebar of active/deregistered agents, center timeline of `@recipient text` messages ordered newest-at-bottom, reactions-as-ACKs with hover tooltips, and a bottom input that supports `@agent text` unicast and `@all text` broadcast. On the backend, add a nullable `origin_task_id` column to `tasks` so broadcast delivery rows can be grouped client-side into a single timeline entry, expose a new `GET /ui/api/timeline` endpoint, and loosen `POST /ui/api/messages/send` to accept `to_agent_id="*"`.

## Success Criteria

- [ ] `admin/` renders the new Discord-style layout (sidebar + timeline + input + sender selector) and no stale components (`AgentTabs`, `MessageList`, `MessageRow`, `SendMessageForm`) remain
- [ ] `GET /ui/api/timeline` returns up to 200 most-recent non-`broadcast_summary` tasks for the caller's tenant, newest first, each payload row carrying `origin_task_id` and `created_at`
- [ ] `POST /ui/api/messages/send` accepts `to_agent_id="*"` and triggers `BrokerExecutor._handle_broadcast`
- [ ] Alembic migration `0002_add_origin_task_id` adds the nullable column and round-trips through `TaskStore.save` / `TaskStore.get`
- [ ] `BrokerExecutor._handle_broadcast` populates `origin_task_id` (= the summary task's own id) on every delivery task and the summary task itself, and records `recipientIds` in the summary task's metadata
- [ ] Unicast sends leave `origin_task_id` NULL
- [ ] Sidebar lists active agents on top (sorted by `registered_at` ascending), deregistered agents below as disabled/muted entries
- [ ] A single broadcast appears as ONE timeline entry regardless of recipient count; reaction chips reveal per-recipient `@<name>[ (deregistered)] — <ack time>` on CSS hover
- [ ] Canceled tasks render with strikethrough body and no reaction area, but remain visible
- [ ] `ARCHITECTURE.md`, `README.md`, `docs/spec/webui-api.md`, and `docs/spec/data-model.md` reflect the new layout, endpoint, and schema column
- [ ] `.claude/skills/hikyaku/SKILL.md` explicitly verified unchanged (CLI surface is not touched by this design doc)

---

## Background

The current dashboard forces the admin to pick an agent from a tab bar, then toggle between Inbox and Sent subtabs to see that one agent's message view. Cross-agent conversations require repeated tab switches. There is no way to see a broadcast's delivery status across all N recipients in a single view — today each delivery is a separate row keyed on the recipient's `context_id`, and the only "broadcast" signal is a summary task (filtered out of the inbox view) carrying a numeric `recipientCount` with no link back to the delivery rows it spawned.

The user wants a single time-ordered message log per tenant, Discord-style, with broadcasts collapsed to one row and reactions-as-ACKs revealing per-recipient acknowledgment state on hover. The admin itself is Auth0-authenticated but is NOT a Hikyaku agent, so "who is the sender" is resolved via a header dropdown that selects a real agent in the currently-selected tenant.

---

## Specification

### Data model change

A nullable `origin_task_id` TEXT column is added to `tasks`. Semantics:

| Case | `origin_task_id` value |
|---|---|
| Unicast delivery (today's `_handle_unicast`) | `NULL` |
| Broadcast delivery row (today's `_handle_broadcast` inner loop) | The summary task's own `task_id` (shared across all N delivery rows in the same broadcast) |
| Broadcast summary row | Its own `task_id` (self-reference, so every row in the broadcast group — deliveries AND summary — shares one value) |
| Historical rows from before this migration | `NULL` (no backfill) |

The group-membership predicate on the wire is `origin_task_id IS NOT NULL`, which never collides with historical unicast rows because `NULL` is preserved for them.

The column is populated in `BrokerExecutor._handle_broadcast` by pre-allocating the summary task's UUID **before** the delivery loop so deliveries can reference it. The column is read by `GET /ui/api/timeline` and surfaced in the JSON payload so the frontend can `groupBy(row.origin_task_id)`.

The summary task's `metadata["recipientIds"]` is extended from just `recipientCount` to also include the full recipient list `[agent_b, agent_c, ...]`. This lives inside the existing `task_json` blob — no new column.

```python
# db/models.py — Task class
origin_task_id: Mapped[str | None] = mapped_column(String, nullable=True)
```

```python
# executor.py — _handle_broadcast, simplified
summary_task_id = str(uuid.uuid4())
for agent in recipients:
    delivery_task = Task(
        id=str(uuid.uuid4()),
        context_id=agent["agent_id"],
        ...,
        metadata={
            "fromAgentId": from_agent_id,
            "toAgentId": agent["agent_id"],
            "type": "unicast",
            "originTaskId": summary_task_id,
        },
    )
    await self._task_store.save(delivery_task)
    await event_queue.enqueue_event(delivery_task)

summary_task = Task(
    id=summary_task_id,
    context_id=from_agent_id,
    ...,
    metadata={
        "fromAgentId": from_agent_id,
        "type": "broadcast_summary",
        "recipientCount": len(recipients),
        "recipientIds": [a["agent_id"] for a in recipients],
        "originTaskId": summary_task_id,
    },
)
```

`TaskStore.save` reads `metadata.get("originTaskId")` (None-safe) and writes it into the column on INSERT and on the `set_=` UPSERT clause. `_handle_unicast` is NOT touched — the absence of `originTaskId` in its metadata means the column is written as `NULL`, which is exactly the desired behavior.

### Known design debt — ACK timestamp inference

Delivery tasks today make **exactly one** state transition over their lifetime: `input_required → completed` via `BrokerExecutor._handle_ack`, which overwrites `status_timestamp` with the ACK moment. Consequently, for any row where `status_state == 'completed'`, `status_timestamp` IS the ACK timestamp. The new timeline UI relies on this invariant to render per-recipient ACK times in the hover tooltip — **no dedicated `acknowledged_at` column is added**.

If any future change introduces a second state transition on a delivery task (retry, resurrect, metadata-only re-save that moves `status_timestamp`), this invariant breaks and the reaction tooltip silently starts showing wrong times. At that point a dedicated `acknowledged_at` TEXT column MUST be added and the tooltip code MUST be switched to read it instead. This is accepted residual risk for v1 and is flagged in `docs/spec/data-model.md`.

### Timeline API

New endpoint on `webui_router`:

```
GET /ui/api/timeline
Authorization: Bearer <auth0_jwt>
X-Tenant-Id: <api_key_hash>
```

Response (200 OK):

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

The response is ordered newest-first by `status_timestamp DESC`, excludes `broadcast_summary` rows, and is hard-capped at 200. No pagination in the first cut.

Tenant scoping is reached through the `tasks.context_id → agents.agent_id → agents.tenant_id` join. A new `TaskStore.list_timeline(tenant_id: str, limit: int = 200)` method runs:

```sql
SELECT t.task_json, t.origin_task_id, t.created_at
FROM tasks t
JOIN agents a ON a.agent_id = t.context_id
WHERE a.tenant_id = :tenant_id
  AND t.type != 'broadcast_summary'
ORDER BY t.status_timestamp DESC
LIMIT :limit
```

The join uses the recipient side (`context_id`) because cross-tenant sends are already blocked by `verify_agent_tenant` in `webui_api.send_message`, so `tasks.from_agent_id` and `tasks.context_id` always belong to the same tenant. No new index — for the v1 row-count ceiling the 200-row LIMIT + existing `idx_agents_tenant_status` + `idx_tasks_context_status_ts` cover the query. A denormalized `tasks.tenant_id` column can be added later if this becomes a hotspot.

`_format_messages` is extended to emit `origin_task_id`, `created_at`, and `status_timestamp` in the per-row dict. The existing inbox and sent endpoints get the fields for free — harmless overhead, keeps the payload shape uniform across all three endpoints.

Authentication reuses `Depends(get_webui_tenant)`, so path-parameter and header variants of tenant selection are NOT introduced — every tenant-scoped endpoint in `webui_api.py` goes through the same dependency.

### Broadcast from the WebUI

`POST /ui/api/messages/send` currently validates `body.to_agent_id` as a specific agent in the tenant (`verify_agent_tenant` + `get_agent` + active-status check), then builds `Message(..., metadata={"destination": body.to_agent_id})`. The executor already supports `destination="*"`.

The change:

```python
if body.to_agent_id == "*":
    # Broadcast — the executor fans out to all active in-tenant agents
    destination = "*"
else:
    # Existing unicast validation
    to_agent = await store.get_agent(body.to_agent_id)
    if to_agent is None:
        raise HTTPException(status_code=404, detail="Agent not found")
    if to_agent.get("status") == "deregistered":
        raise HTTPException(status_code=400, detail="Agent is deregistered")
    if not await store.verify_agent_tenant(body.to_agent_id, tenant_id):
        raise HTTPException(status_code=404, detail="Agent not found")
    destination = body.to_agent_id

msg = Message(..., metadata={"destination": destination})
```

`from_agent_id` is still required to exist, be active, and belong to the tenant. The response shape is unchanged — on broadcast, the returned `task_id` is the summary task's id (the last event drained from the queue in the existing implementation, which happens to be the summary task).

### UI specification

#### Layout

```
┌─────────────────────────────────────────────────────────┐
│ Header: tenant-id label  |  SenderSelector ▼  | Logout  │
├──────────┬──────────────────────────────────────────────┤
│ Sidebar  │ Timeline (scrollable, newest at bottom)      │
│          │                                              │
│ ACTIVE   │  @reviewer-bot Please review PR #42          │
│  Claude-A│  └ reactions: [[ack]@reviewer-bot hover→@…] │
│  reviewer│                                              │
│  …       │  @all Build failed on main                   │
│          │  └ reactions: [[ack]@Claude-A [ack]@reviewer]│
│ DEREG.   │                                              │
│  old-bot │  ‾‾@Claude-A canceled message‾‾              │
│          │                                              │
│          ├──────────────────────────────────────────────┤
│          │ MessageInput: @<agent> or @all + text        │
└──────────┴──────────────────────────────────────────────┘
```

#### Sidebar

Fetched from the existing `GET /ui/api/agents` (unchanged). Rendered in two groups:

- **Active** — agents with `status == "active"`, sorted by `registered_at` ascending
- **Deregistered** — agents with `status == "deregistered"`, sorted by `registered_at` ascending, rendered with `opacity-50` and `pointer-events-none` (visual only — clicks are no-ops in the first cut)

Empty-tenant state: "No agents registered in this tenant. Use the `hikyaku register` CLI to add one." (timeline and input are disabled in this state).

#### Sender selector

Header dropdown listing active agents only. The selected agent becomes `from_agent_id` for every `POST /ui/api/messages/send` call originating from this browser session. Persisted in `localStorage` under the **per-tenant** key `hikyaku.sender.<tenant_id>`. Switching tenants leaves the new tenant's selector unset until the user picks one — this prevents sending as an agent that doesn't exist in the newly-selected tenant. If the stored agent has since deregistered when the page reloads, the selector falls back to "unset" and the send button is disabled.

#### Timeline

Fetched from `GET /ui/api/timeline`. The raw response is grouped client-side:

```ts
function groupMessages(msgs: TimelineMessage[]): TimelineEntry[] {
  const groups = new Map<string, TimelineMessage[]>();
  const singletons: TimelineEntry[] = [];
  for (const m of msgs) {
    if (m.origin_task_id) {
      const g = groups.get(m.origin_task_id) ?? [];
      g.push(m);
      groups.set(m.origin_task_id, g);
    } else {
      singletons.push({ kind: "unicast", message: m });
    }
  }
  const broadcasts: TimelineEntry[] = [...groups.values()].map((rows) => ({
    kind: "broadcast",
    rows,
    sortKey: rows.reduce((min, r) => r.created_at < min ? r.created_at : min, rows[0].created_at),
  }));
  return [...singletons, ...broadcasts].sort(byAscending(sortKeyOf));
}
```

Sort key rules:

- Unicast entry: `sortKey = message.created_at`
- Broadcast entry: `sortKey = MIN(row.created_at for row in group)` — stable, never moves after the initial INSERT of the earliest delivery, independent of subsequent ACK activity

Newest-at-bottom. On initial mount, the Timeline component auto-scrolls to the bottom. Manual refresh re-fetches and re-scrolls to the bottom. No live updates.

#### TimelineMessage component

Renders one entry. Format:

- **Unicast**: `<sender chip> → @<recipient-chip> <body text>`
- **Broadcast**: `<sender chip> → @<r1-chip> @<r2-chip> … <body text>` — one mention chip per recipient in `recipientIds` order
- **Canceled** (`status == "canceled"`): body text wrapped in `<s>` with `opacity-60`, reaction area hidden

#### ReactionBar component

Renders one chip per recipient that has ACKed. For a unicast entry, there is 0 or 1 chip (0 for `input_required` / `canceled`, 1 for `completed`). For a broadcast entry, there are 0..N chips.

Each chip is a Tailwind `group` with the tooltip as a `group-hover:opacity-100 opacity-0 transition-opacity` sibling — no tooltip library. Tooltip content:

```
@<agent-name>[ (deregistered)] — <ISO ack time>
```

The `(deregistered)` suffix is appended when the acker is resolved against the current sidebar list and found in the "deregistered" group. ACK time is `status_timestamp` of the `completed` delivery row (see "Known design debt" above).

#### MessageInput component

Single-line Discord-style text input at the bottom of the main area. On submit:

1. Trim leading whitespace
2. Greedily consume leading mentions: while input matches `^@[A-Za-z0-9_\-]+(?=\s|$)`, pop the token and trim trailing whitespace
3. Remaining text is the body — must be non-empty after trim, else inline error "Message body is empty"

Validation (inline error under the input, no submit):

| Parsed mentions | Decision |
|---|---|
| 0 tokens | Error: "Start the message with @&lt;agent&gt; or @all" |
| `@all` alone | Broadcast. `to_agent_id = "*"` |
| Single `@<slug>` | Resolve against active sidebar list (slug = `agent.name` with non-alphanumerics collapsed to `-`, lowercased). Unresolved → "No active agent named '@&lt;slug&gt;'". Ambiguous (two agents slug-collide) → "Ambiguous mention '@&lt;slug&gt;'" |
| `@all` with any other token | Error: "@all cannot be combined with other mentions" |
| Two or more `@<slug>` tokens (no `@all`) | Error: "Multi-recipient unicast not supported in first cut; use @all for broadcast" |

On success, calls `sendMessage(fromAgentId, resolvedTo, body)`. On 2xx, clears the input and re-fetches the timeline.

### Scope cuts (not alternatives — explicitly out of this design)

- Live updates (SSE / WebSocket) — manual refresh only
- Sidebar click-to-filter the timeline by agent — stretch goal
- Mention chip click-to-scroll-sidebar — stretch goal
- Historical broadcast backfill migration — ungrouped historical rows are accepted
- Message editing or deletion — not in product
- Pagination / infinite scroll — 200-row hard limit
- Multi-recipient unicast in the parser (`@a @b text`) — rejected at parse time, not at the API

### Affected files

| File | Change |
|---|---|
| `registry/src/hikyaku_registry/db/models.py` | Add `origin_task_id: Mapped[str \| None]` nullable column to `Task` |
| `registry/src/hikyaku_registry/alembic/versions/0002_add_origin_task_id.py` | NEW — `op.batch_alter_table("tasks")` adds `sa.Column("origin_task_id", sa.String(), nullable=True)`; downgrade drops it |
| `registry/src/hikyaku_registry/task_store.py` | `save()` reads `metadata.get("originTaskId")`, writes the new column in `INSERT` values AND in the `set_=` UPSERT clause. Add `list_timeline(tenant_id, limit=200)` method running the JOIN query |
| `registry/src/hikyaku_registry/executor.py` | `_handle_broadcast`: pre-allocate `summary_task_id` before the delivery loop; add `originTaskId` to every delivery task's metadata and to the summary task's metadata; add `recipientIds` list to the summary task's metadata |
| `registry/src/hikyaku_registry/webui_api.py` | New `GET /timeline` handler using `get_webui_tenant` → `task_store.list_timeline` → `_format_messages`. Extend `_format_messages` to emit `origin_task_id`, `created_at`, `status_timestamp`. Extend `send_message` to short-circuit unicast validation when `body.to_agent_id == "*"` |
| `registry/tests/test_db_models.py` | Round-trip test: save + re-read with `origin_task_id=None` AND with a concrete UUID; idempotent re-save preserves the non-null value |
| `registry/tests/test_executor.py` | Broadcast test: 3 deliveries + 1 summary all share the same `origin_task_id` (= summary's own id); `recipientIds` populated. Unicast test: `origin_task_id` is `None` |
| `registry/tests/test_webui_api.py` | `GET /timeline` happy path (order, 200-row cap, excludes `broadcast_summary`, includes `origin_task_id` + `created_at`); cross-tenant isolation; `POST /messages/send` with `to_agent_id="*"` succeeds and writes N delivery rows; `POST /messages/send` with `to_agent_id="*"` from an out-of-tenant `from_agent_id` returns 400 |
| `admin/src/types.ts` | Add `TimelineMessage`, `TimelineEntry`, `TimelineReaction` types. Extend existing `Message` with `origin_task_id`, `created_at`, `status_timestamp` OR replace it wholesale with `TimelineMessage` |
| `admin/src/api.ts` | Add `fetchTimeline(): Promise<{messages: TimelineMessage[]}>`. Loosen `sendMessage` to pass through `to_agent_id: "*"` (the parameter type is already `string`) |
| `admin/src/components/Dashboard.tsx` | Rewrite — new layout tree with Header + SenderSelector, Sidebar, Timeline, MessageInput. Removes the Inbox/Sent subtab state machine |
| `admin/src/components/Sidebar.tsx` | NEW — active + deregistered groups, sorted by `registered_at` ascending |
| `admin/src/components/Timeline.tsx` | NEW — fetches via `fetchTimeline`, groups by `origin_task_id`, sorts by `MIN(created_at)` per group, newest-at-bottom with auto-scroll on mount and refresh |
| `admin/src/components/TimelineMessage.tsx` | NEW — renders one entry: mention chips + body + ReactionBar; canceled → strikethrough + muted |
| `admin/src/components/ReactionBar.tsx` | NEW — reaction chips with pure-CSS hover tooltip (`group` + `group-hover:opacity-100`) |
| `admin/src/components/MessageInput.tsx` | NEW — parser + inline validation + submit |
| `admin/src/components/SenderSelector.tsx` | NEW — active-agent dropdown persisted under `hikyaku.sender.<tenant_id>` |
| `admin/src/components/AgentTabs.tsx` | DELETE |
| `admin/src/components/MessageList.tsx` | DELETE |
| `admin/src/components/MessageRow.tsx` | DELETE |
| `admin/src/components/SendMessageForm.tsx` | DELETE |
| `ARCHITECTURE.md` | Update `## WebUI` section (~lines 210-218): rewrite the "browse agents/messages" blurb to describe the Discord-style timeline + sender selector; add `GET /ui/api/timeline` to the backend-API bullet |
| `README.md` | Update line 16 (WebUI bullet) to mention the unified timeline. Add a row for `GET /ui/api/timeline` in the WebUI API table (~lines 229-246). Note in the send-row that `to_agent_id="*"` triggers a broadcast |
| `docs/spec/webui-api.md` | Add `### GET /ui/api/timeline` section with request/response schema, 200-row limit, broadcast-grouping note. Update `POST /ui/api/messages/send` to document `to_agent_id="*"` |
| `docs/spec/data-model.md` | Add `origin_task_id` row to the `tasks` schema table; add a "Broadcast Grouping" subsection covering the NULL-unicast / self-referencing-summary convention and the design-debt note on `status_timestamp` ≈ `acknowledged_at` |
| `.claude/skills/hikyaku/SKILL.md` | **VERIFIED no change needed** — CLI surface (`hikyaku broadcast`, `hikyaku ack`, `hikyaku send`) is unchanged. Explicitly listed here per the project's SKILL.md-is-first-class-documentation rule |
| `plugins/*/skills/*/SKILL.md` | **VERIFIED does not exist** in this repo — Grep returned zero matches for `plugins/**/SKILL.md` |

### Alternatives considered

| Alternative | Why rejected |
|---|---|
| Synthetic `@admin` pseudo-agent as the sender | Requires schema changes and pollutes the `agents` table; complicates ACK semantics because the admin is Auth0-authenticated but not an A2A participant; the sender-selector dropdown is a simpler resolution |
| Newest-at-top timeline (chat-log style) | Rejected in favor of Discord-convention newest-at-bottom with auto-scroll on mount |
| Self-referencing `origin_task_id` for unicast (instead of NULL) | NULL makes "is this part of a broadcast group" queryable as `origin_task_id IS NOT NULL` in one predicate; self-reference requires a separate marker |
| Historical broadcast backfill migration | Historical summary tasks carry only `recipientCount`, not `recipientIds`, so re-linking is heuristic and fragile. Ungrouped historical broadcasts are accepted residual risk |
| Server-side timeline grouping | Deferred — client-side group-by is simpler, the 200-row hard limit keeps payload small, and grouping policy changes don't force a migration |
| Live updates (SSE / WebSocket) | Out of scope for first cut. Manual refresh only. May be added later without touching the data model |
| Change `tasks.type` to `'broadcast_delivery'` on broadcast delivery rows | Widens the `type` enum domain and leaves historical rows inconsistent forever; `origin_task_id IS NOT NULL` already captures the same information without a schema convention change |
| Timeline endpoint under `/ui/api/tenant/{tenant_id}/timeline` | Inconsistent with the existing `X-Tenant-Id`-header convention shared by every other tenant-scoped endpoint in `webui_api.py`; `/ui/api/timeline` reuses `get_webui_tenant` and keeps the surface uniform |
| Add a dedicated `acknowledged_at` column pre-emptively | Unnecessary today — the `completed.status_timestamp` invariant is sufficient for v1. Documented as known design debt so the next contributor who breaks the invariant knows exactly which column to add |
| Tooltip library (Floating UI, Radix Tooltip, etc.) | CSS-only hover with `group` + `opacity-0/100` transitions covers the static-content tooltip requirement with zero bundle cost |
| Multi-recipient unicast (`@alice @bob text`) | The A2A send API is 1:1 or fan-out-via-broadcast — no middle ground. Rejecting it at parse time is less confusing than silently promoting it to a broadcast |
| Sort grouped broadcasts by MAX `status_timestamp` (so groups drift to top on each ACK) | Lively but disorienting — a broadcast already displayed in position N would jump to position 1 when a lagging recipient ACKs, rewriting history for the reader |
| Denormalize `tasks.tenant_id` now | Not needed at v1 traffic levels; the JOIN + 200-row LIMIT is cheap enough. Revisit when the timeline query becomes a measured hotspot |

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

### Step 1: Documentation updates (BEFORE any code)

Per `.claude/rules/design-doc-numbering.md`, documentation is updated first.

- [x] Update `ARCHITECTURE.md` `## WebUI` section (lines ~210-218): replace the "browse agents/messages" language with a description of the Discord-style unified timeline + sender selector, and add `GET /ui/api/timeline` to the Backend-API bullet line. <!-- completed: 2026-04-12T10:00 -->
- [x] Update `README.md` line 16 (the WebUI bullet) to mention the Discord-style unified timeline. Add a `GET /ui/api/timeline` row to the WebUI API table (~lines 229-246) and annotate the `POST /ui/api/messages/send` row that `to_agent_id="*"` triggers a broadcast. <!-- completed: 2026-04-12T10:05 -->
- [x] Update `docs/spec/webui-api.md`: add a `### GET /ui/api/timeline` section (auth headers, response schema including `origin_task_id` + `created_at` + `status_timestamp`, 200-row cap, broadcast-grouping note). Update the `POST /ui/api/messages/send` section to document `to_agent_id="*"` semantics. <!-- completed: 2026-04-12T10:10 -->
- [x] Update `docs/spec/data-model.md`: add an `origin_task_id` row to the `tasks` schema table with the NULL-unicast / self-referencing-summary semantics. Add a new "Broadcast Grouping" subsection capturing the grouping convention AND the known design debt on `completed.status_timestamp` ≈ `acknowledged_at`. <!-- completed: 2026-04-12T10:15 -->
- [x] Use Grep to verify `.claude/skills/hikyaku/SKILL.md` references no CLI flag or workflow that this design changes, and that no `plugins/**/SKILL.md` files exist. Record both results in this design doc's Changelog so the SKILL-drift check is traceable. <!-- completed: 2026-04-12T10:20 -->

### Step 2: Registry schema + migration

- [x] Edit `registry/src/hikyaku_registry/db/models.py`: add `origin_task_id: Mapped[str | None] = mapped_column(String, nullable=True)` to the `Task` class. Do not add a new index — the column is selected alongside the existing ordering index, not filtered on. <!-- completed: 2026-04-12T10:30 -->
- [x] Create `registry/src/hikyaku_registry/alembic/versions/0002_add_origin_task_id.py` with `revision="0002"`, `down_revision="0001"`. `upgrade()` uses `op.batch_alter_table("tasks", schema=None)` to add `sa.Column("origin_task_id", sa.String(), nullable=True)`. `downgrade()` drops the column symmetrically via `batch_alter_table`. <!-- completed: 2026-04-12T10:30 -->
- [x] Update `registry/src/hikyaku_registry/task_store.py` `save()`: read `metadata.get("originTaskId")` (defaulting to None), add it to the `sqlite_insert(TaskModel).values(...)` call, and include `"origin_task_id": stmt.excluded.origin_task_id` in the `set_={...}` clause so idempotent re-saves preserve the populated value. <!-- completed: 2026-04-12T10:32 -->
- [x] Add a new `list_timeline(tenant_id: str, limit: int = 200)` method on `TaskStore`. Runs the JOIN query documented in Specification and returns `list[tuple[Task, str | None, str]]` where each tuple is `(Task, origin_task_id, created_at)`. <!-- completed: 2026-04-12T10:33 -->
- [x] Add a round-trip test to `registry/tests/test_db_models.py` (or the closest existing TaskStore test file) asserting: save with `originTaskId=None` → read → None; save with `originTaskId=<uuid>` → read → same uuid; re-save the latter with an unrelated status change → the `origin_task_id` is unchanged. <!-- completed: 2026-04-12T10:25 -->
- [x] Run `mise //registry:test` and confirm the existing suite still passes alongside the new tests. <!-- completed: 2026-04-12T10:35 -->

### Step 3: Executor broadcast logic

- [x] Edit `registry/src/hikyaku_registry/executor.py` `_handle_broadcast`: pre-allocate `summary_task_id = str(uuid.uuid4())` BEFORE the delivery loop; thread it into every delivery task's metadata as `"originTaskId": summary_task_id`; construct the summary task with `id=summary_task_id`, `metadata["originTaskId"] = summary_task_id`, and `metadata["recipientIds"] = [a["agent_id"] for a in recipients]`. Leave `metadata["recipientCount"]` in place for backwards compat with any existing reader. <!-- completed: 2026-04-12T10:45 -->
- [x] Add tests in `registry/tests/test_executor.py`: (a) broadcast to 3 recipients → 3 delivery tasks + 1 summary; assert every one of the 4 tasks has `origin_task_id == summary.task_id`; assert `summary.metadata["recipientIds"]` contains the three recipient ids; (b) unicast → `origin_task_id is None`. <!-- completed: 2026-04-12T10:40 -->
- [x] Run `mise //registry:test`. <!-- completed: 2026-04-12T10:48 -->

### Step 4: Timeline API + broadcast send API

- [x] Edit `registry/src/hikyaku_registry/webui_api.py`: extend `_format_messages` to emit `origin_task_id`, `created_at`, and `status_timestamp` on every row. Update existing inbox/sent endpoints implicitly (no code change at call sites). <!-- completed: 2026-04-12T10:55 -->
- [x] Add a `GET /timeline` handler to `webui_router`, using `Depends(get_webui_tenant)`. Calls `task_store.list_timeline(tenant_id, limit=200)`, passes the list of Tasks into `_format_messages`, and zips `origin_task_id` + `created_at` back into each row dict. Returns `{"messages": [...]}`. <!-- completed: 2026-04-12T10:57 -->
- [x] Extend `webui_api.send_message` to short-circuit unicast validation when `body.to_agent_id == "*"`: skip `store.get_agent(body.to_agent_id)` and `store.verify_agent_tenant(body.to_agent_id, tenant_id)`; still enforce `from_agent_id` is active and in-tenant; build `Message(..., metadata={"destination": "*"})`. Drain events identically and return the summary task's id. <!-- completed: 2026-04-12T10:58 -->
- [x] Add tests in `registry/tests/test_webui_api.py`: (a) `GET /timeline` happy path — header-scoped tenant sees its own tasks ordered by `status_timestamp DESC`, `broadcast_summary` excluded, every row contains `origin_task_id` and `created_at`, 200-row cap honored; (b) cross-tenant isolation — tenant A's header must not see tenant B's tasks; (c) `POST /messages/send` with `to_agent_id="*"` from an in-tenant active sender → N delivery rows written, response returns the summary task id; (d) `POST /messages/send` with `to_agent_id="*"` from an out-of-tenant `from_agent_id` → 400. <!-- completed: 2026-04-12T10:50 -->
- [x] Run `mise //registry:test`. <!-- completed: 2026-04-12T11:00 -->

### Step 5: Admin SPA rewrite

- [x] Delete `admin/src/components/AgentTabs.tsx`, `admin/src/components/MessageList.tsx`, `admin/src/components/MessageRow.tsx`, and `admin/src/components/SendMessageForm.tsx`. Remove their imports from `Dashboard.tsx` in the same commit so the compile is green. <!-- completed: 2026-04-12T11:20 -->
- [x] Update `admin/src/types.ts`: add `TimelineMessage` (existing `Message` fields + `origin_task_id: string | null`, `created_at: string`, `status_timestamp: string`), `TimelineEntry` (discriminated union of `{kind: "unicast", message: TimelineMessage}` and `{kind: "broadcast", rows: TimelineMessage[], sortKey: string}`), and `TimelineReaction` (agent_id, agent_name, agent_status, ack_timestamp). <!-- completed: 2026-04-12T11:10 -->
- [x] Update `admin/src/api.ts`: add `fetchTimeline(): Promise<{messages: TimelineMessage[]}>` hitting `GET /ui/api/timeline`. Keep `sendMessage` as-is — the parameter is already `string`, so passing `"*"` is a caller-side decision. <!-- completed: 2026-04-12T11:11 -->
- [x] Create `admin/src/components/Sidebar.tsx`: fetches via the existing `getAgents`, splits into active/deregistered, sorts each group by `registered_at` ascending, renders deregistered entries with `opacity-50 pointer-events-none`. <!-- completed: 2026-04-12T11:12 -->
- [x] Create `admin/src/components/Timeline.tsx`: fetches via `fetchTimeline`, runs the client-side grouping described in Specification, renders entries in ascending `sortKey` order (newest at the bottom), and uses a `useEffect` + ref to scroll to `bottom` on initial mount and on every successful refresh. <!-- completed: 2026-04-12T11:14 -->
- [x] Create `admin/src/components/TimelineMessage.tsx`: renders one `TimelineEntry`. Mentions → chip components. Body → plain text. Canceled → wrapped in `<s>` with `opacity-60`, reaction area hidden. Invokes `<ReactionBar entry={entry} agents={agents} />`. <!-- completed: 2026-04-12T11:15 -->
- [x] Create `admin/src/components/ReactionBar.tsx`: maps completed delivery rows in the entry to reaction chips; each chip is a Tailwind `group` with a `group-hover:opacity-100 opacity-0 transition-opacity` tooltip sibling; tooltip text = `@<name>[ (deregistered)] — <ISO ack time>` where the deregistered suffix is looked up against the sidebar agents prop. <!-- completed: 2026-04-12T11:13 -->
- [x] Create `admin/src/components/MessageInput.tsx`: implements the parser + validation table from Specification, calls `sendMessage(senderId, resolvedTo, body)`, shows inline error messages, and re-triggers the timeline fetch on success. The `senderId` is read from the parent (Dashboard) which in turn reads from `SenderSelector`. <!-- completed: 2026-04-12T11:16 -->
- [x] Create `admin/src/components/SenderSelector.tsx`: dropdown listing active agents only. Stores the selection under `hikyaku.sender.<tenantId>`. On tenant switch, clears the in-memory state and re-reads from localStorage for the new tenant. If the stored agent is no longer active, falls back to "unset" and disables the send button. <!-- completed: 2026-04-12T11:17 -->
- [x] Rewrite `admin/src/components/Dashboard.tsx`: new layout — top `<header>` row with tenant badge, `<SenderSelector>`, and the existing Back-to-Keys button; below, a `<div className="flex flex-1">` with `<Sidebar>` on the left and a vertical flex column on the right containing `<Timeline>` (flex-grow, scrollable) and `<MessageInput>` (fixed at the bottom). <!-- completed: 2026-04-12T11:18 -->
- [x] Run `mise //admin:lint` and `mise //admin:build`. `mise //admin:build` writes to `registry/src/hikyaku_registry/webui/` per design doc 0000012. <!-- completed: 2026-04-12T11:25 -->

### Step 6: Manual verification

Run on a fresh SQLite file. Each task is an independent scenario so partial progress is visible.

- [ ] **Setup**: register three agents (`Claude-A`, `reviewer-bot`, `deploy-bot`) under a fresh tenant via the `hikyaku` CLI (`hikyaku register --name Claude-A --description ...` × 3). Run `mise //registry:dev`. Open `http://localhost:8000/ui/`, log in via Auth0, select the tenant. Confirm the sidebar shows all three as active (in `registered_at` order). Select `Claude-A` in the sender dropdown and confirm it persists after a page reload. <!-- completed: -->
- [ ] **Unicast send**: send `@reviewer-bot please review PR #42` from the admin MessageInput. Confirm exactly one new timeline entry appears at the bottom with mention chip `@reviewer-bot`, body text, and one empty reaction slot. Confirm the input clears and auto-scroll lands at the bottom. <!-- completed: -->
- [ ] **Unicast ACK**: from a separate shell, run `hikyaku ack --agent-id <reviewer-bot-id> --task-id <task-id-from-the-delivery>`. Refresh the admin UI. Confirm the reaction slot fills with a `[ack]` chip for `@reviewer-bot`, and that hovering reveals the tooltip `@reviewer-bot — <ISO ack time>`. <!-- completed: -->
- [ ] **Broadcast send**: send `@all build failed on main` from the admin MessageInput as `Claude-A`. Confirm exactly ONE new timeline entry appears (not two), rendered with mention chips `@reviewer-bot @deploy-bot` (sender excluded), body text, and two empty reaction slots. <!-- completed: -->
- [ ] **Broadcast partial ACK**: ACK from ONE recipient only (`hikyaku ack` as `reviewer-bot`). Refresh the admin UI. Confirm the broadcast entry now shows exactly one filled chip (`@reviewer-bot`), the other still empty. Confirm the broadcast entry's sort position is UNCHANGED — it did not drift to the bottom on ACK. <!-- completed: -->
- [ ] **Deregistered sidebar**: run `hikyaku deregister --agent-id <deploy-bot-id>`. Refresh the admin UI. Confirm `deploy-bot` moves to the DEREG. group in the sidebar with muted/disabled styling. Confirm the broadcast entry's still-empty reaction slot for `deploy-bot` now labels it `(deregistered)` in the tooltip (once the admin ACKs it via CLI for the sake of the test, re-confirm). <!-- completed: -->
- [ ] **Canceled message rendering**: from a CLI send `hikyaku send --agent-id <Claude-A-id> --to <reviewer-bot-id> --text "canceled test"`; immediately `hikyaku cancel` it before ACK. Refresh the admin UI. Confirm the entry appears with strikethrough body text, `opacity-60` muting, and NO reaction area. <!-- completed: -->
- [ ] **Parser negative cases**: in the admin MessageInput, type each of the following and confirm the inline error fires WITHOUT sending any request: (a) `plain text with no mention` → "Start the message with @&lt;agent&gt; or @all"; (b) `@all @Claude-A hi` → "@all cannot be combined with other mentions"; (c) `@Claude-A @reviewer-bot hi` → "Multi-recipient unicast not supported in first cut; use @all for broadcast"; (d) `@nonexistent hi` → "No active agent named '@nonexistent'". <!-- completed: -->

---

## Changelog

| Date | Changes |
|------|---------|
| 2026-04-11 | Initial draft |
| 2026-04-11 | Reviewer pass 1: replaced two `U+2705` emoji markers in the ASCII layout diagram with `[ack]` (no-emoji rule); split Step 6 manual-verification from one bundled checkbox into 8 per-scenario checkboxes (setup, unicast send, unicast ACK, broadcast send, broadcast partial ACK, deregistered sidebar, canceled rendering, parser negative cases); added a canceled-message verification scenario and a `@nonexistent` parser-negative case that were implicit in the Specification but not exercised by the original checklist. Progress counter updated 31 → 38 |
| 2026-04-11 | User approved; Status → Approved |
| 2026-04-12 | Step 1 SKILL-drift verification: Grep-read `.claude/skills/hikyaku/SKILL.md` and confirmed it references only CLI surface (`hikyaku register`, `send`, `broadcast`, `poll`, `ack`, `cancel`, `get-task`, `agents`, `deregister`, `--json`, `--agent-id`, `HIKYAKU_URL`, `HIKYAKU_API_KEY`) — none of which are modified by this design doc (no `client/` files in the Affected files list). Glob `plugins/**/SKILL.md` from project root returned zero matches — no plugin skill files exist in this repo. Both checks pass; no SKILL.md edits required |
