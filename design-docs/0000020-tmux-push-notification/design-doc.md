# tmux Push Notification

**Status**: Complete
**Progress**: 15/15 tasks complete
**Last Updated**: 2026-04-12

## Overview

Add server-side tmux push notifications to complement CAFleet's pull-based message delivery. After persisting a message, the broker injects a `cafleet poll` command into each recipient's tmux pane via `tmux send-keys`, enabling near-instant delivery when agents are idle while preserving the queue as the sole source of truth.

## Success Criteria

- [x] Unicast `cafleet send` triggers tmux poll notification to recipient pane
- [x] Broadcast `cafleet broadcast` triggers tmux poll notifications to all recipient panes
- [x] Self-send (sender == recipient) skips notification
- [x] Missing or dead pane fails silently; message remains in queue for normal polling
- [x] Response includes `notification_sent` (unicast) / `notifications_sent_count` (broadcast)
- [x] Existing polling behavior is completely unchanged

---

## Background

CAFleet uses a pull-based delivery model: senders call `cafleet send`, messages are persisted to the task store, and recipients discover them via `cafleet poll`. This works but introduces latency proportional to the poll interval. The `agent_placements` table already stores each member's `tmux_session`, `tmux_window_id`, and `tmux_pane_id`, giving the server enough information to push a poll trigger directly into a recipient's pane.

---

## Specification

### Mechanism

After persisting a message to the task store, `BrokerExecutor` looks up the recipient's `agent_placements` row via `RegistryStore.get_placement()`. If the recipient has a non-null `tmux_pane_id` and is not the sender, the server runs:

```
tmux send-keys -t <tmux_pane_id> "cafleet poll --agent-id <recipient_agent_id>" Enter
```

The recipient's agent_id is hardcoded in the command (not `$CAFLEET_AGENT_ID`) because the server already knows the concrete UUID. The `--url` and `--session-id` flags are unnecessary because the pane's environment already has `CAFLEET_URL` and `CAFLEET_SESSION_ID` set at spawn time.

The injected text lands in the coding agent's input prompt. If the agent is idle, it interprets the text as an instruction to poll. If the agent is busy, tmux buffers the keystrokes until the agent returns to its input prompt. Since `cafleet poll` is idempotent, duplicate or late-arriving triggers are harmless.

### Behavioral Rules

| Scenario | Behavior | `notification_sent` |
|---|---|---|
| Unicast to agent with placement + pane_id | Send poll trigger | `true` |
| Unicast to agent without placement row | Skip trigger | `false` |
| Unicast to agent with null pane_id (pending) | Skip trigger | `false` |
| Unicast to self (sender == recipient) | Skip trigger | `false` |
| Pane no longer exists (tmux error) | Catch error silently | `false` |
| tmux binary not on PATH | Skip trigger | `false` |
| Broadcast to N recipients | Trigger each (skip self, skip missing) | count in `notifications_sent_count` |

### Response Format Changes

**Unicast** (`SendMessage` response) -- add top-level `notification_sent`:

```json
{
  "task": { "id": "...", "status": {...}, "metadata": {...}, ... },
  "notification_sent": true
}
```

**Broadcast** (`SendMessage` with `destination: "*"`) -- add `notifications_sent_count` to summary task metadata:

```json
{
  "task": {
    "id": "...",
    "metadata": {
      "type": "broadcast_summary",
      "recipientCount": 3,
      "notifications_sent_count": 2,
      ...
    }
  }
}
```

### New Function: `tmux.send_poll_trigger()`

```python
def send_poll_trigger(*, target_pane_id: str, agent_id: str) -> bool:
    """Send a cafleet poll trigger to the given tmux pane.

    Returns True on success, False if tmux is unavailable or the pane
    no longer exists. Never raises — internally calls _run() and catches
    TmuxError, returning False on any failure.
    """
```

Key differences from `send_exit()`:
- Returns `bool` instead of raising on failure (best-effort optimization, not critical path)
- Does NOT require `TMUX` env var -- `tmux send-keys -t <pane>` works from any process on the same host as long as the tmux server socket is accessible
- Checks `shutil.which("tmux")` only (no `ensure_tmux_available()`)

### New Method: `BrokerExecutor._try_notify_agent()`

```python
async def _try_notify_agent(self, agent_id: str, from_agent_id: str) -> bool:
    """Attempt tmux push notification. Returns True on success."""
```

Logic:
1. If `agent_id == from_agent_id`, return `False` (self-send guard)
2. Look up placement via `self._registry_store.get_placement(agent_id)`
3. If no placement or `tmux_pane_id is None`, return `False`
4. Call `await asyncio.to_thread(tmux.send_poll_trigger, ...)` to avoid blocking the event loop
5. Return the result

### Integration Points

**`_handle_unicast()`** in `executor.py`:

```python
# After task_store.save(delivery_task):
notification_sent = await self._try_notify_agent(destination, from_agent_id)
delivery_task.metadata["notification_sent"] = notification_sent
# Then enqueue (in-memory metadata, not re-persisted)
await event_queue.enqueue_event(delivery_task)
```

**`_handle_broadcast()`** in `executor.py`:

```python
notifications_sent_count = 0

for agent in recipients:
    # ... construct, save, and enqueue delivery_task (unchanged) ...
    sent = await self._try_notify_agent(agent["agent_id"], from_agent_id)
    if sent:
        notifications_sent_count += 1

# In summary task metadata (individual delivery tasks are NOT annotated):
"notifications_sent_count": notifications_sent_count
```

Individual broadcast delivery tasks do not carry `notification_sent` in their metadata — only the summary task reports the aggregate count to the sender.

**`_handle_send_message()`** in `server.py`:

```python
result = {"task": _task_to_dict(last_task)}
if last_task.metadata:
    if "notification_sent" in last_task.metadata:
        result["notification_sent"] = last_task.metadata["notification_sent"]
    if "notifications_sent_count" in last_task.metadata:
        result["notifications_sent_count"] = last_task.metadata["notifications_sent_count"]
return result
```

### Out of Scope (v1)

- Per-agent notification preferences / opt-out
- Rate limiting or throttling of notifications
- Non-tmux notification channels (e.g., webhooks, SSE)

---

## Implementation

> Task format: `- [x] Done task <!-- completed: 2026-02-13T14:30 -->`
> When completing a task, check the box and record the timestamp in the same edit.

### Step 1: Documentation

- [x] Update `ARCHITECTURE.md` with push notification section <!-- completed: 2026-04-13T12:10 -->
- [x] Update `README.md` to mention tmux push notification <!-- completed: 2026-04-13T12:10 -->
- [x] Update `.claude/skills/cafleet/SKILL.md` to document notification behavior <!-- completed: 2026-04-13T12:10 -->

### Step 2: `tmux.py` -- Add `send_poll_trigger()`

- [x] Add `send_poll_trigger(*, target_pane_id: str, agent_id: str) -> bool` function that runs `tmux send-keys` and returns success/failure without raising <!-- completed: 2026-04-13T12:18 -->

### Step 3: `executor.py` -- Notification Logic

- [x] Add `import asyncio` and `from cafleet import tmux` <!-- completed: 2026-04-13T12:25 -->
- [x] Add `async _try_notify_agent(self, agent_id: str, from_agent_id: str) -> bool` helper method <!-- completed: 2026-04-13T12:25 -->
- [x] Modify `_handle_unicast()`: call `_try_notify_agent()` after `task_store.save()`, set `notification_sent` in task metadata before enqueue <!-- completed: 2026-04-13T12:25 -->
- [x] Modify `_handle_broadcast()`: call `_try_notify_agent()` per recipient after save, track count, add `notifications_sent_count` to summary metadata <!-- completed: 2026-04-13T12:25 -->

### Step 4: `server.py` -- Response Format

- [x] Update `_handle_send_message()` to extract `notification_sent` / `notifications_sent_count` from task metadata and include at response top level <!-- completed: 2026-04-13T12:30 -->

### Step 5: `cli.py` + `output.py` -- CLI Output

- [x] Update `send` command to show `(push notification sent)` or nothing based on `notification_sent` <!-- completed: 2026-04-13T12:35 -->
- [x] Update `broadcast` command to show notification count from response <!-- completed: 2026-04-13T12:35 -->

### Step 6: Tests

- [x] Unit test `send_poll_trigger()`: success case, pane-not-found, tmux binary missing <!-- completed: 2026-04-13T12:12 -->
- [x] Unit test executor notification paths: unicast with/without placement, self-send skip, broadcast with mixed placements <!-- completed: 2026-04-13T12:16 -->
- [x] Unit test `_handle_send_message()` response includes `notification_sent` (unicast) and `notifications_sent_count` (broadcast) at top level <!-- completed: 2026-04-13T12:22 -->
- [x] Unit test CLI output: `send` shows `(push notification sent)` when true, `broadcast` shows notification count <!-- completed: 2026-04-13T12:26 -->
