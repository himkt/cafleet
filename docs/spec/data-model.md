---
icon: lucide/table
---

# Data model

The `Task` payload is fully relational: every routing field plus the message body lives in its own typed column. The only JSON `TEXT` blob is `agents.agent_card_json`, which stores an `AgentCard`-shaped document.

The model is **predominantly relational**: every queried field on `tasks` is a typed column, and the only opaque payload is the `agent_card_json` document on `agents`.

Schema management is handled by Alembic; the runtime engine is SQLAlchemy 2.x with the synchronous `pysqlite` driver. The schema is created by a single initial migration; operators apply it once via `cafleet db init` before starting the server.

## SQL Schema

All tables key on `INTEGER` columns. The three minted-id tables (`fleets`, `agents`, `tasks`) use `INTEGER PRIMARY KEY AUTOINCREMENT`. AUTOINCREMENT creates a per-table `sqlite_sequence` row that tracks the high-water mark, so **ids are never reused** — even after the highest row is deleted. The first AUTOINCREMENT value is `1`, so real ids are always `>= 1`; this leaves `0` free as a sentinel (see `tasks.to_agent_id`). The other tables — `agent_placements`, `monitor_config`, and `monitor_runtime` — are the integer PKs **without** AUTOINCREMENT: each reuses a parent id (`agents.agent_id` for the first two, `fleets.fleet_id` for `monitor_runtime`) as a 1:1 PK rather than minting a fresh sequence.

### `fleets`

| Column | Type | Constraints | Notes |
|---|---|---|---|
| `fleet_id` | `INTEGER` | `PRIMARY KEY AUTOINCREMENT` | DB-assigned integer (first value `1`, monotonically increasing). |
| `label` | `TEXT` | nullable | Optional free-form text for human bookkeeping (e.g. `"PR-42 review"`). |
| `created_at` | `TEXT` | `NOT NULL` | ISO-8601 timestamp. |
| `deleted_at` | `TEXT` | nullable | `NULL` = active; non-NULL ISO-8601 timestamp = soft-deleted. Written on fleet delete; never cleared. |
| `director_agent_id` | `INTEGER` | nullable (DB), app-enforced NOT NULL after bootstrap; `REFERENCES agents(agent_id) ON DELETE RESTRICT` | Points at the fleet's root Director (the agent auto-registered by `cafleet fleet create`). DB-nullable so the bootstrap transaction can insert the fleet row before the Director's `agents` row exists; post-bootstrap every non-deleted fleet has a non-NULL value. |

Fleet deletion is a **soft-delete** keyed on `deleted_at`. See [cli-options.md](./cli-options.md) `fleet delete` for the observable delete behavior.

#### Root Director bootstrap

`cafleet fleet create` writes the fleet row, the root Director (and its placement), the `director_agent_id` back-reference, and the built-in Administrator in one all-or-nothing transaction. See [cli-options.md](./cli-options.md) `fleet create` for the observable bootstrap behavior.

### `agents`

| Column | Type | Constraints | Notes |
|---|---|---|---|
| `agent_id` | `INTEGER` | `PRIMARY KEY AUTOINCREMENT` | DB-assigned integer. |
| `fleet_id` | `INTEGER` | `NOT NULL`, `REFERENCES fleets(fleet_id) ON DELETE RESTRICT` | The owning fleet. SQLite enforces the FK once `PRAGMA foreign_keys=ON` is set. |
| `name` | `TEXT` | `NOT NULL` | |
| `description` | `TEXT` | `NOT NULL` | |
| `status` | `TEXT` | `NOT NULL` | `'active'` or `'deregistered'`. |
| `registered_at` | `TEXT` | `NOT NULL` | ISO-8601 timestamp. |
| `deregistered_at` | `TEXT` | nullable | ISO-8601 timestamp; populated on soft-delete. |
| `agent_card_json` | `TEXT` | `NOT NULL` | AgentCard-shaped blob (CAFleet-defined internal schema). |

Indexes:

| Name | Columns | Purpose |
|---|---|---|
| `idx_agents_fleet_status` | `(fleet_id, status)` | List active agents in a fleet; covers the `WHERE fleet_id = ? AND status = 'active'` predicate. |

Deregistration is a soft-delete: `status='deregistered'` plus `deregistered_at` is set in a single statement. There is no row delete and no background cleanup loop. Active query paths filter `status='active'` so dead rows are invisible to normal traffic.

#### Built-in Administrator agent

Each fleet owns exactly one built-in `Administrator` agent, distinguished by a flag inside `agent_card_json` (`cafleet.kind == "builtin-administrator"`) rather than a separate table or column:

```json
{
  "name": "Administrator",
  "description": "Built-in administrator agent for fleet <fleet_id>",
  "skills": [],
  "cafleet": {
    "kind": "builtin-administrator"
  }
}
```

The `cafleet.*` namespace inside `agent_card_json` is reserved for broker-owned flags; callers cannot set `cafleet.kind` through any public path. The Administrator row is written as the final operation of the fleet-create bootstrap transaction, and its `registered_at` matches `fleets.created_at`. No migration seeds it — the single initial migration is schema-only.

**Invariant**: Every fleet has exactly one active `Administrator` agent. The WebUI surfaces a derived `kind` field (`"builtin-administrator"` | `"user"`) so it can locate the Administrator without matching on the name.

**Protection**: The Administrator is a write-only identity — it is used as the WebUI's implicit sender but never receives messages or a tmux pane. It cannot be deregistered (`Administrator cannot be deregistered`) and cannot be made a Director (`Administrator cannot be a director`). It is excluded from broadcast recipients, though it may itself be the broadcast sender.

### `tasks`

| Column | Type | Constraints | Notes |
|---|---|---|---|
| `task_id` | `INTEGER` | `PRIMARY KEY AUTOINCREMENT` | DB-assigned integer. Identifies a single delivery (unicast row, broadcast delivery row, or broadcast summary row). |
| `context_id` | `INTEGER` | `NOT NULL`, `REFERENCES agents(agent_id) ON DELETE RESTRICT` | The recipient agent for unicast/broadcast deliveries; the broadcaster for `broadcast_summary`; the preserved original `context_id` for ACK/cancel. Always a registered `agent_id`. |
| `from_agent_id` | `INTEGER` | `NOT NULL` | Sender agent. **Not** a foreign key — historical tasks may outlive their sender. |
| `to_agent_id` | `INTEGER` | `NOT NULL` | Recipient agent. **`0`** for `broadcast_summary` rows (the "no single recipient" sentinel; real ids are `>= 1` so `0` never collides). |
| `type` | `TEXT` | `NOT NULL` | `'unicast'` or `'broadcast_summary'`. |
| `created_at` | `TEXT` | `NOT NULL` | ISO-8601 timestamp; set at insert time, never updated. |
| `status_state` | `TEXT` | `NOT NULL` | `input_required`, `completed`, or `canceled`. |
| `status_timestamp` | `TEXT` | `NOT NULL` | ISO-8601 timestamp; updated on every state change. Used for `ORDER BY DESC`. |
| `origin_task_id` | `INTEGER` | nullable | Broadcast grouping link. `NULL` on unicast deliveries. On broadcast delivery rows, holds the summary task's `task_id`, shared across every delivery row in the same broadcast. On the broadcast summary row itself, holds its own `task_id` (self-reference) so the delivery rows and the summary row all share a single grouping value. **Not** a foreign key — it is a nullable self-link. |
| `text` | `TEXT` | `NOT NULL` | Message body. For `broadcast_summary` rows, the broker writes the human-readable summary `"Broadcast sent to N recipients"` at insert time. |

Indexes:

| Name | Columns | Purpose |
|---|---|---|
| `idx_tasks_context_status_ts` | `(context_id, status_timestamp DESC)` | Inbox listing: `WHERE context_id = ? ORDER BY status_timestamp DESC`. |
| `idx_tasks_from_agent_status_ts` | `(from_agent_id, status_timestamp DESC)` | WebUI sender outbox: `WHERE from_agent_id = ? ORDER BY status_timestamp DESC`. |

### `agent_placements`

| Column | Type | Constraints | Notes |
|---|---|---|---|
| `agent_id` | `INTEGER` | `PRIMARY KEY` (no AUTOINCREMENT), `REFERENCES agents(agent_id) ON DELETE CASCADE` | The member agent. This is the parent `agents.agent_id` value reused as a 1:1 PK — it is **not** a freshly minted sequence, so AUTOINCREMENT is deliberately excluded. CASCADE ensures hard-delete of an agent (if any future path adds one) also removes the placement. |
| `director_agent_id` | `INTEGER` | nullable, `REFERENCES agents(agent_id) ON DELETE RESTRICT` | The fleet's root Director — the single Director that owns every member. RESTRICT prevents hard-deleting a Director with live placements. For every member placement this always equals `fleets.director_agent_id` (the fleet root); it is **NULL** only for the root Director's own placement (it has no parent), set at bootstrap time. Nested teams are forbidden — member registration rejects any placement whose `director_agent_id` is not the fleet root. |
| `tmux_session` | `TEXT` | `NOT NULL` | e.g. `'main'`, from `tmux display-message '#{session_name}'`. |
| `tmux_window_id` | `TEXT` | `NOT NULL` | e.g. `'@3'`, from `#{window_id}`. |
| `tmux_pane_id` | `TEXT` | nullable | e.g. `'%7'`. `NULL` = pending (row inserted at register time, pane not yet spawned). Set after the pane is spawned. |
| `coding_agent` | `TEXT` | `NOT NULL`, `DEFAULT 'claude'` | Free-form coding-agent identifier. Current known values are `"claude"` (the default for normal registrations) and `"codex"` / `"opencode"` when chosen at create time. The default `'claude'` applies when a placement is inserted without an explicit value. |
| `created_at` | `TEXT` | `NOT NULL` | ISO-8601 timestamp, set server-side to match `agents.registered_at`. |

Indexes:

| Name | Columns | Purpose |
|---|---|---|
| `idx_placements_director` | `(director_agent_id)` | List the fleet's members; every member placement's `director_agent_id` is the fleet root. |

Placement rows are hard-deleted (not soft-deleted) when the agent is deregistered through any path. They have no historical value and must not outlive the agent they describe.

If a user kills a pane manually without going through `cafleet member delete`, the placement row stays until the next `member delete` resolves it; the "pane already gone" case is handled gracefully.

### `monitor_config`

Per-agent monitoring schedule, one row per **pane-bound** agent. The `cafleet monitor` process reads this table each tick to decide which agents are due (see [Monitoring](../concepts/monitoring.md)).

| Column | Type | Constraints | Notes |
|---|---|---|---|
| `agent_id` | `INTEGER` | `PRIMARY KEY` (no AUTOINCREMENT), `REFERENCES agents(agent_id) ON DELETE CASCADE` | The enrolled agent. This is the parent `agents.agent_id` value reused as a 1:1 PK (mirrors `agent_placements.agent_id`) — not a freshly minted sequence, so AUTOINCREMENT is deliberately excluded. |
| `interval_seconds` | `INTEGER` | `NOT NULL`, `DEFAULT 60` | Ping cadence for this agent. |
| `last_ping_at` | `TEXT` | nullable | ISO-8601 of the last ping the monitor dispatched to this agent. `NULL` = never pinged ⇒ due immediately. Persisted (not in-memory) so a monitor restart resumes cadence and `monitor status` can display it. |
| `enabled` | `INTEGER` | `NOT NULL`, `DEFAULT 1` | Boolean stored as SQLite `0`/`1`. `0` = the monitor skips this agent while preserving its interval for re-enable. Every broker read casts it to a Python `bool` at the read boundary, so the integer representation never leaks past the broker (the CLI and the WebUI/JSON contract both see `enabled: bool`). |

A row is inserted automatically at registration for every agent that has a tmux pane — the root Director and every member. The write-only Administrator and card-only `agent register` calls (no placement) are **not** enrolled: only an agent with a pane can be pinged. There is no `fleet_id` column — fleet scoping is reached through the `monitor_config.agent_id → agents.agent_id → agents.fleet_id` join (the same pattern `agent_placements` uses), and director-vs-member is **derived** at scan time (`agent_id == fleets.director_agent_id`), never denormalized.

The row is hard-deleted (not soft-deleted) when the agent is deregistered, alongside its `agent_placements` row — runtime config with no historical value, on the same lifecycle as the placement.

### `monitor_runtime`

Per-fleet process and heartbeat state, one row per fleet. A dedicated single-row-per-fleet table (rather than columns on `fleets`) keeps high-write heartbeat telemetry off the slow-changing fleet identity row, models "no monitor" cleanly as "no row", and lets single-instance claims and teardown operate independently of the `fleets` lifecycle.

| Column | Type | Constraints | Notes |
|---|---|---|---|
| `fleet_id` | `INTEGER` | `PRIMARY KEY` (no AUTOINCREMENT), `REFERENCES fleets(fleet_id) ON DELETE RESTRICT` | One row per fleet; reuses `fleets.fleet_id` as a 1:1 PK. |
| `pid` | `INTEGER` | nullable | OS PID of the running worker. `NULL` after a clean stop. The single-instance claim records it; the ownership-checked heartbeat/clear match on it; `stop` reads it to signal. |
| `started_at` | `TEXT` | nullable | ISO-8601 when the current worker claimed the runtime. |
| `last_tick_at` | `TEXT` | nullable | ISO-8601 heartbeat, rewritten every tick. The **authority** for `status` liveness — a process that died silently stops updating it, so it reads as stale. |
| `tick_seconds` | `INTEGER` | `NOT NULL`, `DEFAULT 5` | Scan-tick cadence the running monitor uses, so `status` can report it. |

The `monitor_runtime` row is deleted explicitly inside the `fleet delete` transaction (along with the fleet's `monitor_config` rows). Because agents are soft-deregistered and fleets soft-deleted, neither the CASCADE off `agents` nor the RESTRICT off `fleets` fires on the normal delete paths — the rows are cleaned explicitly, mirroring how `agent_placements` is explicitly deleted today.

### Foreign key enforcement

SQLite ignores foreign key declarations unless `PRAGMA foreign_keys=ON` is issued on every connection. A SQLAlchemy engine `connect` event listener runs the PRAGMA on every new connection.

The foreign keys on `agents.fleet_id`, `tasks.context_id`, `fleets.director_agent_id`, `agent_placements.director_agent_id`, and `monitor_runtime.fleet_id` all use `ON DELETE RESTRICT` (agents are only soft-deregistered today, so RESTRICT is the safest default). `agent_placements.agent_id → agents.agent_id` and `monitor_config.agent_id → agents.agent_id` both use `ON DELETE CASCADE` so a hard-deleted agent (if any future path adds one) cannot leave a dangling placement or config row. Fleet deletion uses a soft-delete (`deleted_at`) — it never physically removes rows, so the RESTRICTs are not triggered by the normal delete path; the `monitor_config` and `monitor_runtime` rows are instead removed explicitly inside the `fleet delete` transaction.

## Task Visibility Rules

Read access is **fleet-scoped**; per-agent identity is enforced only on state transitions:

| Operation | Enforcement |
|---|---|
| `message poll` | Returns the `input_required` deliveries whose `context_id` equals the passed `--agent-id`. The CLI gates only that `--agent-id` belongs to `--fleet-id`, so any in-fleet caller can poll any in-fleet agent's inbox by id — there is no per-caller snoop guard. |
| `message show` | Returns the task iff at least one of its endpoints (`from_agent_id` / `to_agent_id`) belongs to `--fleet-id`. No per-caller check; cross-fleet lookups return "not found". |
| `message ack` | Only the recipient may ACK — the caller's `agent_id` must equal the task's `context_id`, otherwise `Only the recipient can ACK a task`. |
| `message cancel` | Only the sender may cancel — the caller's `agent_id` must equal the task's `from_agent_id`, otherwise `Only the sender can cancel a task`. |

The only cross-fleet boundary is fleet membership; within a fleet, reads are not partitioned per agent. Recipient/sender identity is enforced only when transitioning a task's state (ack / cancel).

## Broadcast Grouping

A broadcast produces N+1 rows — one delivery task per active recipient plus one `broadcast_summary` task — and the admin WebUI timeline presents all of them as a single entry. The `tasks.origin_task_id` column is the grouping link:

| Row kind | `origin_task_id` value |
|---|---|
| Unicast delivery | `NULL` |
| Broadcast delivery row (one per recipient) | The summary task's `task_id` (shared across all N delivery rows in the same broadcast) |
| Broadcast summary row | Its own `task_id` (self-reference) |

Because ids are DB-assigned, the summary row is inserted **first** with `origin_task_id` temporarily `NULL`; the broker reads back its DB-assigned `task_id`, updates that row's `origin_task_id` to itself (self-reference), then inserts each delivery row with `origin_task_id = summary_task_id`. Unicast deliveries are inserted with `origin_task_id` left `NULL`.

The grouping predicate on the wire is `origin_task_id IS NOT NULL`, which cleanly partitions the timeline into "standalone unicast entry" vs "part of a broadcast group".

### ACK timestamp inference

The timeline reads each broadcast's per-recipient ACK time from the `status_timestamp` of the matching `completed` delivery row, which is valid because a delivery task makes exactly one state transition over its lifetime (`input_required → completed` on ACK). If a future change adds a second transition that rewrites `status_timestamp`, a dedicated `acknowledged_at` column must be added.

## Deregistered Agents

Deregistration is a soft-delete only. There is no background cleanup loop and no physical removal of agent or task rows. Deregistered agents continue to exist as rows with `status='deregistered'` indefinitely; their inbox tasks remain readable by the WebUI (the only consumer that surfaces deregistered agents). Active query paths filter `status='active'` so dead rows are invisible to normal traffic.
