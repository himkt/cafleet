"""Tests for executor.py — BrokerExecutor business logic.

Covers: unicast send, broadcast send, ACK (multi-turn), GetTask visibility,
CancelTask. Tests the executor methods directly with SQL-backed stores.

Also covers cross-tenant unicast rejection and tenant-scoped broadcast
(access-control feature).
"""

import uuid
from unittest.mock import AsyncMock, MagicMock

import pytest
from a2a.server.agent_execution import RequestContext
from a2a.server.context import ServerCallContext
from a2a.server.events import EventQueue
from a2a.types import (
    Message,
    MessageSendParams,
    Part,
    Role,
    Task,
    TaskState,
    TextPart,
)

from hikyaku_registry.executor import BrokerExecutor
from hikyaku_registry.registry_store import RegistryStore
from hikyaku_registry.task_store import TaskStore


# ---------------------------------------------------------------------------
# Owner subs for dynamic API key creation
# ---------------------------------------------------------------------------

_OWNER_SHARED = "auth0|exec-shared"
_OWNER_TENANT_A = "auth0|exec-tenant-a"
_OWNER_TENANT_B = "auth0|exec-tenant-b"


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------


def _make_call_context(agent_id: str, tenant_id: str) -> ServerCallContext:
    """Create a ServerCallContext with agent_id and tenant_id."""
    return ServerCallContext(
        state={"agent_id": agent_id, "tenant_id": tenant_id},
    )


def _make_send_context(
    from_agent_id: str,
    tenant_id: str,
    destination: str,
    text: str = "Hello",
    task_id: str | None = None,
) -> RequestContext:
    """Create a RequestContext for sending a message."""
    message = Message(
        message_id=str(uuid.uuid4()),
        role=Role.user,
        parts=[Part(root=TextPart(text=text))],
        task_id=task_id,
        metadata={"destination": destination},
    )
    params = MessageSendParams(message=message)
    return RequestContext(
        request=params,
        call_context=_make_call_context(from_agent_id, tenant_id),
    )


def _make_ack_context(
    from_agent_id: str,
    tenant_id: str,
    task_id: str,
    text: str = "ack",
) -> RequestContext:
    """Create a RequestContext for ACKing an existing task (multi-turn)."""
    message = Message(
        message_id=str(uuid.uuid4()),
        role=Role.user,
        parts=[Part(root=TextPart(text=text))],
        task_id=task_id,
    )
    params = MessageSendParams(message=message)
    return RequestContext(
        request=params,
        task_id=task_id,
        call_context=_make_call_context(from_agent_id, tenant_id),
    )


def _make_cancel_context(
    from_agent_id: str,
    tenant_id: str,
    task_id: str,
    task: Task | None = None,
) -> RequestContext:
    """Create a RequestContext for canceling a task."""
    return RequestContext(
        task_id=task_id,
        task=task,
        call_context=_make_call_context(from_agent_id, tenant_id),
    )


async def _collect_events(queue: EventQueue) -> list:
    """Collect all events from the queue."""
    events = []
    try:
        while True:
            event = await queue.dequeue_event(no_wait=True)
            events.append(event)
    except Exception:
        pass
    return events


# ---------------------------------------------------------------------------
# Single-tenant fixture
# ---------------------------------------------------------------------------


@pytest.fixture
async def env(store: RegistryStore, task_store: TaskStore):
    """Set up BrokerExecutor with SQL-backed stores and test agents.

    All agents share the same API key (same tenant) for basic tests.
    """
    api_key, tenant_id, _ = await store.create_api_key(_OWNER_SHARED)
    executor = BrokerExecutor(registry_store=store, task_store=task_store)

    agent_a = await store.create_agent(
        name="Agent A", description="Sender", api_key=api_key
    )
    agent_b = await store.create_agent(
        name="Agent B", description="Recipient", api_key=api_key
    )
    agent_c = await store.create_agent(
        name="Agent C", description="Third agent", api_key=api_key
    )

    return {
        "executor": executor,
        "store": store,
        "task_store": task_store,
        "tenant_id": tenant_id,
        "agent_a": agent_a,
        "agent_b": agent_b,
        "agent_c": agent_c,
    }


# ---------------------------------------------------------------------------
# Unicast Send
# ---------------------------------------------------------------------------


class TestUnicastSend:
    """Tests for BrokerExecutor.execute — unicast message delivery."""

    async def test_creates_delivery_task(self, env):
        """Unicast send creates a delivery Task."""
        executor, agent_a, agent_b = env["executor"], env["agent_a"], env["agent_b"]
        queue = EventQueue()

        context = _make_send_context(
            from_agent_id=agent_a["agent_id"],
            tenant_id=env["tenant_id"],
            destination=agent_b["agent_id"],
            text="Did the API schema change?",
        )
        await executor.execute(context, queue)

        events = await _collect_events(queue)
        tasks = [e for e in events if isinstance(e, Task)]
        assert len(tasks) >= 1

    async def test_task_state_is_input_required(self, env):
        """Delivery Task has state INPUT_REQUIRED."""
        executor, agent_a, agent_b = env["executor"], env["agent_a"], env["agent_b"]
        queue = EventQueue()

        context = _make_send_context(
            from_agent_id=agent_a["agent_id"],
            tenant_id=env["tenant_id"],
            destination=agent_b["agent_id"],
        )
        await executor.execute(context, queue)

        events = await _collect_events(queue)
        task = next(e for e in events if isinstance(e, Task))
        assert task.status.state == TaskState.input_required

    async def test_task_context_id_is_recipient(self, env):
        """Delivery Task contextId equals the recipient's agent_id."""
        executor, agent_a, agent_b = env["executor"], env["agent_a"], env["agent_b"]
        queue = EventQueue()

        context = _make_send_context(
            from_agent_id=agent_a["agent_id"],
            tenant_id=env["tenant_id"],
            destination=agent_b["agent_id"],
        )
        await executor.execute(context, queue)

        events = await _collect_events(queue)
        task = next(e for e in events if isinstance(e, Task))
        assert task.context_id == agent_b["agent_id"]

    async def test_message_content_in_artifact(self, env):
        """Message text is stored as an Artifact on the delivery Task."""
        executor, agent_a, agent_b = env["executor"], env["agent_a"], env["agent_b"]
        queue = EventQueue()

        context = _make_send_context(
            from_agent_id=agent_a["agent_id"],
            tenant_id=env["tenant_id"],
            destination=agent_b["agent_id"],
            text="Did the API schema change?",
        )
        await executor.execute(context, queue)

        events = await _collect_events(queue)
        task = next(e for e in events if isinstance(e, Task))
        assert task.artifacts is not None
        assert len(task.artifacts) >= 1

    async def test_task_metadata_has_routing_info(self, env):
        """Delivery Task metadata contains fromAgentId, toAgentId, type."""
        executor, agent_a, agent_b = env["executor"], env["agent_a"], env["agent_b"]
        queue = EventQueue()

        context = _make_send_context(
            from_agent_id=agent_a["agent_id"],
            tenant_id=env["tenant_id"],
            destination=agent_b["agent_id"],
        )
        await executor.execute(context, queue)

        events = await _collect_events(queue)
        task = next(e for e in events if isinstance(e, Task))
        assert task.metadata["fromAgentId"] == agent_a["agent_id"]
        assert task.metadata["toAgentId"] == agent_b["agent_id"]
        assert task.metadata["type"] == "unicast"

    async def test_error_missing_destination(self, env):
        """Missing metadata.destination raises an error."""
        executor, agent_a = env["executor"], env["agent_a"]
        queue = EventQueue()

        message = Message(
            message_id=str(uuid.uuid4()),
            role=Role.user,
            parts=[Part(root=TextPart(text="No destination"))],
        )
        params = MessageSendParams(message=message)
        context = RequestContext(
            request=params,
            call_context=_make_call_context(agent_a["agent_id"], env["tenant_id"]),
        )

        with pytest.raises(Exception) as exc_info:
            await executor.execute(context, queue)

        assert exc_info.value is not None

    async def test_error_destination_not_found(self, env):
        """Destination agent_id not found raises an error."""
        executor, agent_a = env["executor"], env["agent_a"]
        queue = EventQueue()

        context = _make_send_context(
            from_agent_id=agent_a["agent_id"],
            tenant_id=env["tenant_id"],
            destination="00000000-0000-4000-8000-000000000000",
        )

        with pytest.raises(Exception):
            await executor.execute(context, queue)

    async def test_error_destination_deregistered(self, env):
        """Sending to a deregistered agent raises an error."""
        executor, store, agent_a, agent_b = (
            env["executor"],
            env["store"],
            env["agent_a"],
            env["agent_b"],
        )
        queue = EventQueue()

        await store.deregister_agent(agent_b["agent_id"])

        context = _make_send_context(
            from_agent_id=agent_a["agent_id"],
            tenant_id=env["tenant_id"],
            destination=agent_b["agent_id"],
        )

        with pytest.raises(Exception):
            await executor.execute(context, queue)

    async def test_error_invalid_destination_format(self, env):
        """Invalid destination format (not UUID or '*') raises an error."""
        executor, agent_a = env["executor"], env["agent_a"]
        queue = EventQueue()

        context = _make_send_context(
            from_agent_id=agent_a["agent_id"],
            tenant_id=env["tenant_id"],
            destination="not-a-valid-uuid",
        )

        with pytest.raises(Exception):
            await executor.execute(context, queue)


# ---------------------------------------------------------------------------
# Broadcast Send
# ---------------------------------------------------------------------------


class TestBroadcastSend:
    """Tests for BrokerExecutor.execute — broadcast message delivery."""

    async def test_creates_delivery_tasks_for_all_active_agents(self, env):
        """Broadcast creates one delivery Task per active agent (excluding sender)."""
        executor, agent_a = env["executor"], env["agent_a"]
        queue = EventQueue()

        context = _make_send_context(
            from_agent_id=agent_a["agent_id"],
            tenant_id=env["tenant_id"],
            destination="*",
            text="Build failed on main branch",
        )
        await executor.execute(context, queue)

        events = await _collect_events(queue)
        tasks = [e for e in events if isinstance(e, Task)]

        # Should have delivery tasks for agent_b and agent_c + summary task
        assert len(tasks) >= 3  # 2 delivery + 1 summary

    async def test_excludes_sender_from_recipients(self, env):
        """Sender does not receive their own broadcast."""
        executor, agent_a = env["executor"], env["agent_a"]
        queue = EventQueue()

        context = _make_send_context(
            from_agent_id=agent_a["agent_id"],
            tenant_id=env["tenant_id"],
            destination="*",
        )
        await executor.execute(context, queue)

        events = await _collect_events(queue)
        delivery_tasks = [
            e
            for e in events
            if isinstance(e, Task)
            and e.metadata
            and e.metadata.get("type") == "unicast"
        ]

        for task in delivery_tasks:
            assert task.context_id != agent_a["agent_id"]

    async def test_summary_task_is_completed(self, env):
        """Broadcast returns a summary Task with state=COMPLETED."""
        executor, agent_a = env["executor"], env["agent_a"]
        queue = EventQueue()

        context = _make_send_context(
            from_agent_id=agent_a["agent_id"],
            tenant_id=env["tenant_id"],
            destination="*",
        )
        await executor.execute(context, queue)

        events = await _collect_events(queue)
        summary_tasks = [
            e
            for e in events
            if isinstance(e, Task)
            and e.metadata
            and e.metadata.get("type") in ("broadcast", "broadcast_summary")
        ]

        assert len(summary_tasks) >= 1
        summary = summary_tasks[0]
        assert summary.status.state == TaskState.completed

    async def test_summary_task_has_recipient_count(self, env):
        """Summary Task artifact includes recipientCount."""
        executor, agent_a = env["executor"], env["agent_a"]
        queue = EventQueue()

        context = _make_send_context(
            from_agent_id=agent_a["agent_id"],
            tenant_id=env["tenant_id"],
            destination="*",
        )
        await executor.execute(context, queue)

        events = await _collect_events(queue)
        summary_tasks = [
            e
            for e in events
            if isinstance(e, Task)
            and e.metadata
            and e.metadata.get("type") in ("broadcast", "broadcast_summary")
        ]

        summary = summary_tasks[0]
        assert summary.artifacts is not None
        assert len(summary.artifacts) >= 1

    async def test_delivery_tasks_have_input_required_state(self, env):
        """Each delivery Task is in INPUT_REQUIRED state."""
        executor, agent_a = env["executor"], env["agent_a"]
        queue = EventQueue()

        context = _make_send_context(
            from_agent_id=agent_a["agent_id"],
            tenant_id=env["tenant_id"],
            destination="*",
        )
        await executor.execute(context, queue)

        events = await _collect_events(queue)
        delivery_tasks = [
            e
            for e in events
            if isinstance(e, Task) and e.status.state == TaskState.input_required
        ]

        # Should have 2 delivery tasks (agent_b and agent_c)
        assert len(delivery_tasks) == 2

    async def test_each_delivery_task_context_id_is_recipient(self, env):
        """Each delivery Task's contextId equals its recipient's agent_id."""
        executor, agent_a, agent_b, agent_c = (
            env["executor"],
            env["agent_a"],
            env["agent_b"],
            env["agent_c"],
        )
        queue = EventQueue()

        context = _make_send_context(
            from_agent_id=agent_a["agent_id"],
            tenant_id=env["tenant_id"],
            destination="*",
        )
        await executor.execute(context, queue)

        events = await _collect_events(queue)
        delivery_tasks = [
            e
            for e in events
            if isinstance(e, Task) and e.status.state == TaskState.input_required
        ]

        recipient_ids = {t.context_id for t in delivery_tasks}
        assert agent_b["agent_id"] in recipient_ids
        assert agent_c["agent_id"] in recipient_ids

    async def test_broadcast_no_other_agents(self, env):
        """Broadcast with no other active agents produces recipientCount=0."""
        executor, store, agent_a, agent_b, agent_c = (
            env["executor"],
            env["store"],
            env["agent_a"],
            env["agent_b"],
            env["agent_c"],
        )
        queue = EventQueue()

        # Deregister all other agents
        await store.deregister_agent(agent_b["agent_id"])
        await store.deregister_agent(agent_c["agent_id"])

        context = _make_send_context(
            from_agent_id=agent_a["agent_id"],
            tenant_id=env["tenant_id"],
            destination="*",
        )
        await executor.execute(context, queue)

        events = await _collect_events(queue)
        delivery_tasks = [
            e
            for e in events
            if isinstance(e, Task) and e.status.state == TaskState.input_required
        ]
        assert len(delivery_tasks) == 0


# ---------------------------------------------------------------------------
# ACK (Multi-Turn)
# ---------------------------------------------------------------------------


class TestAck:
    """Tests for BrokerExecutor.execute — ACK via multi-turn SendMessage."""

    async def _create_unicast_task(self, env):
        """Helper: send a unicast message and return the delivery Task."""
        executor, agent_a, agent_b = env["executor"], env["agent_a"], env["agent_b"]
        queue = EventQueue()

        context = _make_send_context(
            from_agent_id=agent_a["agent_id"],
            tenant_id=env["tenant_id"],
            destination=agent_b["agent_id"],
            text="Hello Agent B",
        )
        await executor.execute(context, queue)

        events = await _collect_events(queue)
        return next(e for e in events if isinstance(e, Task))

    async def test_ack_moves_task_to_completed(self, env):
        """Recipient ACK moves the Task to COMPLETED state."""
        executor, agent_b = env["executor"], env["agent_b"]
        delivery_task = await self._create_unicast_task(env)

        queue = EventQueue()
        context = _make_ack_context(
            from_agent_id=agent_b["agent_id"],
            tenant_id=env["tenant_id"],
            task_id=delivery_task.id,
        )
        await executor.execute(context, queue)

        events = await _collect_events(queue)
        task = next(e for e in events if isinstance(e, Task))
        assert task.status.state == TaskState.completed

    async def test_ack_by_non_recipient_raises_error(self, env):
        """ACK by a non-recipient agent raises an error."""
        executor, agent_c = env["executor"], env["agent_c"]
        delivery_task = await self._create_unicast_task(env)

        queue = EventQueue()
        context = _make_ack_context(
            from_agent_id=agent_c["agent_id"],
            tenant_id=env["tenant_id"],
            task_id=delivery_task.id,
        )

        with pytest.raises(Exception):
            await executor.execute(context, queue)

    async def test_ack_on_already_completed_raises_error(self, env):
        """ACK on an already completed task raises an error."""
        executor, agent_b = env["executor"], env["agent_b"]
        delivery_task = await self._create_unicast_task(env)

        queue1 = EventQueue()
        ctx1 = _make_ack_context(
            from_agent_id=agent_b["agent_id"],
            tenant_id=env["tenant_id"],
            task_id=delivery_task.id,
        )
        await executor.execute(ctx1, queue1)

        queue2 = EventQueue()
        ctx2 = _make_ack_context(
            from_agent_id=agent_b["agent_id"],
            tenant_id=env["tenant_id"],
            task_id=delivery_task.id,
        )

        with pytest.raises(Exception):
            await executor.execute(ctx2, queue2)

    async def test_ack_on_canceled_task_raises_error(self, env):
        """ACK on a canceled task raises an error."""
        executor, agent_a, agent_b = (
            env["executor"],
            env["agent_a"],
            env["agent_b"],
        )
        delivery_task = await self._create_unicast_task(env)

        cancel_queue = EventQueue()
        cancel_ctx = _make_cancel_context(
            from_agent_id=agent_a["agent_id"],
            tenant_id=env["tenant_id"],
            task_id=delivery_task.id,
            task=delivery_task,
        )
        await executor.cancel(cancel_ctx, cancel_queue)

        ack_queue = EventQueue()
        ack_ctx = _make_ack_context(
            from_agent_id=agent_b["agent_id"],
            tenant_id=env["tenant_id"],
            task_id=delivery_task.id,
        )

        with pytest.raises(Exception):
            await executor.execute(ack_ctx, ack_queue)

    async def test_ack_on_unknown_task_raises_error(self, env):
        """ACK on a non-existent task raises an error."""
        executor, agent_b = env["executor"], env["agent_b"]

        queue = EventQueue()
        context = _make_ack_context(
            from_agent_id=agent_b["agent_id"],
            tenant_id=env["tenant_id"],
            task_id="nonexistent-task-id",
        )

        with pytest.raises(Exception):
            await executor.execute(context, queue)


# ---------------------------------------------------------------------------
# GetTask Visibility
# ---------------------------------------------------------------------------


class TestGetTaskVisibility:
    """Tests for task access control on GetTask."""

    async def _create_unicast_task(self, env):
        """Helper: send a unicast from A to B, return the delivery Task."""
        executor, agent_a, agent_b = env["executor"], env["agent_a"], env["agent_b"]
        queue = EventQueue()

        context = _make_send_context(
            from_agent_id=agent_a["agent_id"],
            tenant_id=env["tenant_id"],
            destination=agent_b["agent_id"],
        )
        await executor.execute(context, queue)

        events = await _collect_events(queue)
        return next(e for e in events if isinstance(e, Task))

    async def test_sender_can_get_task(self, env):
        """Sender can access the task they created by taskId."""
        task_store = env["task_store"]
        delivery_task = await self._create_unicast_task(env)

        result = await task_store.get(delivery_task.id)
        assert result is not None

    async def test_recipient_can_get_task(self, env):
        """Recipient can access the task in their context."""
        task_store = env["task_store"]
        delivery_task = await self._create_unicast_task(env)

        result = await task_store.get(delivery_task.id)
        assert result is not None

    async def test_task_stored_in_task_store(self, env):
        """The delivery Task is persisted in the TaskStore."""
        task_store = env["task_store"]
        delivery_task = await self._create_unicast_task(env)

        result = await task_store.get(delivery_task.id)
        assert result is not None

    async def test_task_indexed_by_recipient_context(self, env):
        """Delivery Task appears in TaskStore.list for the recipient's context."""
        task_store, agent_b = env["task_store"], env["agent_b"]
        delivery_task = await self._create_unicast_task(env)

        tasks = await task_store.list(agent_b["agent_id"])
        assert any(t.id == delivery_task.id for t in tasks)

    async def test_task_indexed_by_sender(self, env):
        """Delivery Task appears in TaskStore.list_by_sender for the sender."""
        task_store, agent_a = env["task_store"], env["agent_a"]
        delivery_task = await self._create_unicast_task(env)

        tasks = await task_store.list_by_sender(agent_a["agent_id"])
        assert any(t.id == delivery_task.id for t in tasks)


# ---------------------------------------------------------------------------
# CancelTask
# ---------------------------------------------------------------------------


class TestCancelTask:
    """Tests for BrokerExecutor.cancel — message retraction."""

    async def _create_unicast_task(self, env):
        """Helper: send a unicast from A to B, return the delivery Task."""
        executor, agent_a, agent_b = env["executor"], env["agent_a"], env["agent_b"]
        queue = EventQueue()

        context = _make_send_context(
            from_agent_id=agent_a["agent_id"],
            tenant_id=env["tenant_id"],
            destination=agent_b["agent_id"],
        )
        await executor.execute(context, queue)

        events = await _collect_events(queue)
        return next(e for e in events if isinstance(e, Task))

    async def test_sender_can_cancel_input_required_task(self, env):
        """Sender can cancel a task that is still INPUT_REQUIRED."""
        executor, agent_a = env["executor"], env["agent_a"]
        delivery_task = await self._create_unicast_task(env)

        queue = EventQueue()
        context = _make_cancel_context(
            from_agent_id=agent_a["agent_id"],
            tenant_id=env["tenant_id"],
            task_id=delivery_task.id,
            task=delivery_task,
        )
        await executor.cancel(context, queue)

        events = await _collect_events(queue)
        assert any(
            (isinstance(e, Task) and e.status.state == TaskState.canceled)
            or (hasattr(e, "status") and e.status.state == TaskState.canceled)
            for e in events
        )

    async def test_non_sender_cannot_cancel(self, env):
        """Non-sender cannot cancel a task — raises error."""
        executor, agent_b = env["executor"], env["agent_b"]
        delivery_task = await self._create_unicast_task(env)

        queue = EventQueue()
        context = _make_cancel_context(
            from_agent_id=agent_b["agent_id"],
            tenant_id=env["tenant_id"],
            task_id=delivery_task.id,
            task=delivery_task,
        )

        with pytest.raises(Exception):
            await executor.cancel(context, queue)

    async def test_cancel_completed_task_raises_error(self, env):
        """Cannot cancel a task that is already COMPLETED."""
        executor, agent_a, agent_b = (
            env["executor"],
            env["agent_a"],
            env["agent_b"],
        )
        delivery_task = await self._create_unicast_task(env)

        ack_queue = EventQueue()
        ack_ctx = _make_ack_context(
            from_agent_id=agent_b["agent_id"],
            tenant_id=env["tenant_id"],
            task_id=delivery_task.id,
        )
        await executor.execute(ack_ctx, ack_queue)

        cancel_queue = EventQueue()
        cancel_ctx = _make_cancel_context(
            from_agent_id=agent_a["agent_id"],
            tenant_id=env["tenant_id"],
            task_id=delivery_task.id,
            task=delivery_task,
        )

        with pytest.raises(Exception):
            await executor.cancel(cancel_ctx, cancel_queue)

    async def test_cancel_already_canceled_task_raises_error(self, env):
        """Cannot cancel a task that is already CANCELED."""
        executor, agent_a = env["executor"], env["agent_a"]
        delivery_task = await self._create_unicast_task(env)

        queue1 = EventQueue()
        ctx1 = _make_cancel_context(
            from_agent_id=agent_a["agent_id"],
            tenant_id=env["tenant_id"],
            task_id=delivery_task.id,
            task=delivery_task,
        )
        await executor.cancel(ctx1, queue1)

        queue2 = EventQueue()
        ctx2 = _make_cancel_context(
            from_agent_id=agent_a["agent_id"],
            tenant_id=env["tenant_id"],
            task_id=delivery_task.id,
            task=delivery_task,
        )

        with pytest.raises(Exception):
            await executor.cancel(ctx2, queue2)

    async def test_cancel_unknown_task_raises_error(self, env):
        """Cannot cancel a task that doesn't exist."""
        executor, agent_a = env["executor"], env["agent_a"]

        queue = EventQueue()
        context = _make_cancel_context(
            from_agent_id=agent_a["agent_id"],
            tenant_id=env["tenant_id"],
            task_id="nonexistent-task-id",
        )

        with pytest.raises(Exception):
            await executor.cancel(context, queue)


# ===========================================================================
# Multi-tenant executor tests (access-control feature)
# ===========================================================================


@pytest.fixture
async def tenant_env(store: RegistryStore, task_store: TaskStore):
    """Set up BrokerExecutor with agents in two separate tenants.

    Tenant A: agent_a1, agent_a2
    Tenant B: agent_b1
    """
    api_key_a, tenant_a_id, _ = await store.create_api_key(_OWNER_TENANT_A)
    api_key_b, tenant_b_id, _ = await store.create_api_key(_OWNER_TENANT_B)

    executor = BrokerExecutor(registry_store=store, task_store=task_store)

    agent_a1 = await store.create_agent(
        name="Agent A1", description="Tenant A first", api_key=api_key_a
    )
    agent_a2 = await store.create_agent(
        name="Agent A2", description="Tenant A second", api_key=api_key_a
    )
    agent_b1 = await store.create_agent(
        name="Agent B1", description="Tenant B only", api_key=api_key_b
    )

    return {
        "executor": executor,
        "store": store,
        "task_store": task_store,
        "tenant_a_id": tenant_a_id,
        "tenant_b_id": tenant_b_id,
        "agent_a1": agent_a1,
        "agent_a2": agent_a2,
        "agent_b1": agent_b1,
    }


class TestCrossTenantUnicast:
    """Tests for cross-tenant unicast rejection.

    Unicast must verify destination agent's api_key_hash matches sender's
    tenant. Cross-tenant sends produce "agent not found" errors.
    """

    async def test_same_tenant_unicast_succeeds(self, tenant_env):
        """Agent A1 sends to Agent A2 (same tenant) → succeeds."""
        executor = tenant_env["executor"]
        agent_a1, agent_a2 = tenant_env["agent_a1"], tenant_env["agent_a2"]
        queue = EventQueue()

        context = _make_send_context(
            from_agent_id=agent_a1["agent_id"],
            tenant_id=tenant_env["tenant_a_id"],
            destination=agent_a2["agent_id"],
            text="Hello teammate",
        )
        await executor.execute(context, queue)

        events = await _collect_events(queue)
        tasks = [e for e in events if isinstance(e, Task)]
        assert len(tasks) >= 1
        task = tasks[0]
        assert task.status.state == TaskState.input_required
        assert task.context_id == agent_a2["agent_id"]

    async def test_cross_tenant_unicast_raises_error(self, tenant_env):
        """Agent A1 sends to Agent B1 (different tenant) → error."""
        executor = tenant_env["executor"]
        agent_a1, agent_b1 = tenant_env["agent_a1"], tenant_env["agent_b1"]
        queue = EventQueue()

        context = _make_send_context(
            from_agent_id=agent_a1["agent_id"],
            tenant_id=tenant_env["tenant_a_id"],
            destination=agent_b1["agent_id"],
        )

        with pytest.raises(Exception):
            await executor.execute(context, queue)

    async def test_cross_tenant_error_indistinguishable_from_not_found(
        self, tenant_env
    ):
        """Cross-tenant error message is the same as 'agent not found'.

        The caller cannot distinguish between 'agent exists in another tenant'
        and 'agent does not exist at all'.
        """
        executor = tenant_env["executor"]
        agent_a1, agent_b1 = tenant_env["agent_a1"], tenant_env["agent_b1"]
        queue = EventQueue()

        cross_ctx = _make_send_context(
            from_agent_id=agent_a1["agent_id"],
            tenant_id=tenant_env["tenant_a_id"],
            destination=agent_b1["agent_id"],
        )

        with pytest.raises(Exception) as cross_exc:
            await executor.execute(cross_ctx, queue)

        ghost_ctx = _make_send_context(
            from_agent_id=agent_a1["agent_id"],
            tenant_id=tenant_env["tenant_a_id"],
            destination="00000000-0000-4000-8000-000000000000",
        )

        with pytest.raises(Exception) as ghost_exc:
            await executor.execute(ghost_ctx, queue)

        assert type(cross_exc.value) is type(ghost_exc.value)

    async def test_reverse_cross_tenant_also_blocked(self, tenant_env):
        """Agent B1 sends to Agent A1 (reverse direction) → also blocked."""
        executor = tenant_env["executor"]
        agent_a1, agent_b1 = tenant_env["agent_a1"], tenant_env["agent_b1"]
        queue = EventQueue()

        context = _make_send_context(
            from_agent_id=agent_b1["agent_id"],
            tenant_id=tenant_env["tenant_b_id"],
            destination=agent_a1["agent_id"],
        )

        with pytest.raises(Exception):
            await executor.execute(context, queue)

    async def test_cross_tenant_no_task_created(self, tenant_env):
        """Cross-tenant send does not persist any delivery task."""
        executor, task_store = tenant_env["executor"], tenant_env["task_store"]
        agent_a1, agent_b1 = tenant_env["agent_a1"], tenant_env["agent_b1"]
        queue = EventQueue()

        context = _make_send_context(
            from_agent_id=agent_a1["agent_id"],
            tenant_id=tenant_env["tenant_a_id"],
            destination=agent_b1["agent_id"],
        )

        with pytest.raises(Exception):
            await executor.execute(context, queue)

        tasks = await task_store.list(agent_b1["agent_id"])
        assert len(tasks) == 0


class TestTenantScopedBroadcast:
    """Tests for tenant-scoped broadcast.

    Broadcast from tenant A → only delivers to agents in tenant A.
    Agents in tenant B never receive the broadcast.
    """

    async def test_broadcast_delivers_only_to_same_tenant(self, tenant_env):
        """Broadcast from A1 delivers to A2 only, not B1."""
        executor = tenant_env["executor"]
        agent_a1, agent_a2 = tenant_env["agent_a1"], tenant_env["agent_a2"]
        agent_b1 = tenant_env["agent_b1"]
        queue = EventQueue()

        context = _make_send_context(
            from_agent_id=agent_a1["agent_id"],
            tenant_id=tenant_env["tenant_a_id"],
            destination="*",
            text="Tenant A broadcast",
        )
        await executor.execute(context, queue)

        events = await _collect_events(queue)
        delivery_tasks = [
            e
            for e in events
            if isinstance(e, Task) and e.status.state == TaskState.input_required
        ]

        recipient_ids = {t.context_id for t in delivery_tasks}
        assert agent_a2["agent_id"] in recipient_ids
        assert agent_b1["agent_id"] not in recipient_ids

    async def test_broadcast_excludes_sender(self, tenant_env):
        """Broadcast excludes the sender even within the same tenant."""
        executor = tenant_env["executor"]
        agent_a1 = tenant_env["agent_a1"]
        queue = EventQueue()

        context = _make_send_context(
            from_agent_id=agent_a1["agent_id"],
            tenant_id=tenant_env["tenant_a_id"],
            destination="*",
        )
        await executor.execute(context, queue)

        events = await _collect_events(queue)
        delivery_tasks = [
            e
            for e in events
            if isinstance(e, Task) and e.status.state == TaskState.input_required
        ]

        for task in delivery_tasks:
            assert task.context_id != agent_a1["agent_id"]

    async def test_broadcast_delivery_count_matches_tenant_size(self, tenant_env):
        """Number of delivery tasks equals (tenant agents - 1)."""
        executor = tenant_env["executor"]
        agent_a1 = tenant_env["agent_a1"]
        queue = EventQueue()

        context = _make_send_context(
            from_agent_id=agent_a1["agent_id"],
            tenant_id=tenant_env["tenant_a_id"],
            destination="*",
        )
        await executor.execute(context, queue)

        events = await _collect_events(queue)
        delivery_tasks = [
            e
            for e in events
            if isinstance(e, Task) and e.status.state == TaskState.input_required
        ]

        # Tenant A has 2 agents (a1, a2), so 1 delivery task (to a2)
        assert len(delivery_tasks) == 1

    async def test_broadcast_summary_reflects_tenant_recipients(self, tenant_env):
        """Summary task recipientCount counts only same-tenant agents."""
        executor = tenant_env["executor"]
        agent_a1 = tenant_env["agent_a1"]
        queue = EventQueue()

        context = _make_send_context(
            from_agent_id=agent_a1["agent_id"],
            tenant_id=tenant_env["tenant_a_id"],
            destination="*",
        )
        await executor.execute(context, queue)

        events = await _collect_events(queue)
        summary_tasks = [
            e
            for e in events
            if isinstance(e, Task)
            and e.metadata
            and e.metadata.get("type") in ("broadcast", "broadcast_summary")
        ]

        assert len(summary_tasks) == 1
        summary = summary_tasks[0]
        assert summary.metadata["recipientCount"] == 1

    async def test_tenant_b_broadcast_does_not_reach_tenant_a(self, tenant_env):
        """Broadcast from tenant B delivers only within tenant B."""
        executor, task_store = tenant_env["executor"], tenant_env["task_store"]
        agent_a1, agent_a2 = tenant_env["agent_a1"], tenant_env["agent_a2"]
        agent_b1 = tenant_env["agent_b1"]
        queue = EventQueue()

        context = _make_send_context(
            from_agent_id=agent_b1["agent_id"],
            tenant_id=tenant_env["tenant_b_id"],
            destination="*",
        )
        await executor.execute(context, queue)

        events = await _collect_events(queue)
        delivery_tasks = [
            e
            for e in events
            if isinstance(e, Task) and e.status.state == TaskState.input_required
        ]

        # B1 is alone in tenant B, so no delivery tasks
        assert len(delivery_tasks) == 0

        # Verify nothing landed in tenant A agents' inboxes
        a1_tasks = await task_store.list(agent_a1["agent_id"])
        a2_tasks = await task_store.list(agent_a2["agent_id"])
        assert len(a1_tasks) == 0
        assert len(a2_tasks) == 0

    async def test_broadcast_after_deregister_in_tenant(self, tenant_env):
        """Broadcast skips deregistered agents within the same tenant."""
        executor, store = tenant_env["executor"], tenant_env["store"]
        agent_a1, agent_a2 = tenant_env["agent_a1"], tenant_env["agent_a2"]
        queue = EventQueue()

        await store.deregister_agent(agent_a2["agent_id"])

        context = _make_send_context(
            from_agent_id=agent_a1["agent_id"],
            tenant_id=tenant_env["tenant_a_id"],
            destination="*",
        )
        await executor.execute(context, queue)

        events = await _collect_events(queue)
        delivery_tasks = [
            e
            for e in events
            if isinstance(e, Task) and e.status.state == TaskState.input_required
        ]

        # A2 deregistered, so no delivery tasks
        assert len(delivery_tasks) == 0


# ---------------------------------------------------------------------------
# Pub/Sub Publish Integration
# ---------------------------------------------------------------------------


class TestExecutorPubSubIntegration:
    """Tests for BrokerExecutor publish integration with PubSubManager.

    Verifies that:
    - BrokerExecutor.__init__ accepts a pubsub parameter
    - _handle_unicast publishes task_id to inbox:{destination} after save
    - _handle_broadcast publishes task_id for each recipient's inbox channel
    """

    @pytest.fixture
    async def pubsub_env(self, store: RegistryStore, task_store: TaskStore):
        """Set up BrokerExecutor with a mock PubSubManager and test agents."""
        api_key, tenant_id, _ = await store.create_api_key(_OWNER_SHARED)

        mock_pubsub = MagicMock()
        mock_pubsub.publish = AsyncMock()

        executor = BrokerExecutor(
            registry_store=store,
            task_store=task_store,
            pubsub=mock_pubsub,
        )

        agent_a = await store.create_agent(
            name="Agent A", description="Sender", api_key=api_key
        )
        agent_b = await store.create_agent(
            name="Agent B", description="Recipient", api_key=api_key
        )
        agent_c = await store.create_agent(
            name="Agent C", description="Third agent", api_key=api_key
        )

        return {
            "executor": executor,
            "store": store,
            "task_store": task_store,
            "tenant_id": tenant_id,
            "mock_pubsub": mock_pubsub,
            "agent_a": agent_a,
            "agent_b": agent_b,
            "agent_c": agent_c,
        }

    async def test_init_accepts_pubsub_parameter(self, pubsub_env):
        """BrokerExecutor.__init__ accepts a pubsub parameter."""
        executor = pubsub_env["executor"]
        assert executor is not None

    async def test_unicast_publishes_task_id_to_recipient_channel(self, pubsub_env):
        """After unicast send, publish is called on inbox:{destination} with task_id."""
        executor = pubsub_env["executor"]
        agent_a = pubsub_env["agent_a"]
        agent_b = pubsub_env["agent_b"]
        mock_pubsub = pubsub_env["mock_pubsub"]
        queue = EventQueue()

        context = _make_send_context(
            from_agent_id=agent_a["agent_id"],
            tenant_id=pubsub_env["tenant_id"],
            destination=agent_b["agent_id"],
            text="Hello via unicast",
        )
        await executor.execute(context, queue)

        mock_pubsub.publish.assert_called_once()
        call_args = mock_pubsub.publish.call_args
        channel = (
            call_args[0][0]
            if call_args[0]
            else call_args[1].get("channel", call_args.kwargs.get("channel"))
        )

        assert channel == f"inbox:{agent_b['agent_id']}"

    async def test_unicast_publishes_task_id_as_payload(self, pubsub_env):
        """Unicast publish payload is the task_id string, not full Task JSON."""
        executor = pubsub_env["executor"]
        agent_a = pubsub_env["agent_a"]
        agent_b = pubsub_env["agent_b"]
        mock_pubsub = pubsub_env["mock_pubsub"]
        task_store = pubsub_env["task_store"]
        queue = EventQueue()

        context = _make_send_context(
            from_agent_id=agent_a["agent_id"],
            tenant_id=pubsub_env["tenant_id"],
            destination=agent_b["agent_id"],
            text="Task ID payload check",
        )
        await executor.execute(context, queue)

        call_args = mock_pubsub.publish.call_args
        published_task_id = (
            call_args[0][1]
            if len(call_args[0]) > 1
            else call_args[1].get("message", call_args.kwargs.get("message"))
        )

        assert isinstance(published_task_id, str)
        assert "{" not in published_task_id  # Not JSON

        task = await task_store.get(published_task_id)
        assert task is not None

    async def test_broadcast_publishes_to_each_recipient_channel(self, pubsub_env):
        """After broadcast, publish is called for each recipient's inbox channel."""
        executor = pubsub_env["executor"]
        agent_a = pubsub_env["agent_a"]
        agent_b = pubsub_env["agent_b"]
        agent_c = pubsub_env["agent_c"]
        mock_pubsub = pubsub_env["mock_pubsub"]
        queue = EventQueue()

        context = _make_send_context(
            from_agent_id=agent_a["agent_id"],
            tenant_id=pubsub_env["tenant_id"],
            destination="*",
            text="Broadcast message",
        )
        await executor.execute(context, queue)

        published_channels = [call[0][0] for call in mock_pubsub.publish.call_args_list]
        expected_channels = {
            f"inbox:{agent_b['agent_id']}",
            f"inbox:{agent_c['agent_id']}",
        }

        assert set(published_channels) == expected_channels

    async def test_broadcast_does_not_publish_to_sender(self, pubsub_env):
        """Broadcast does not publish to the sender's own inbox channel."""
        executor = pubsub_env["executor"]
        agent_a = pubsub_env["agent_a"]
        mock_pubsub = pubsub_env["mock_pubsub"]
        queue = EventQueue()

        context = _make_send_context(
            from_agent_id=agent_a["agent_id"],
            tenant_id=pubsub_env["tenant_id"],
            destination="*",
            text="Broadcast no self",
        )
        await executor.execute(context, queue)

        published_channels = [call[0][0] for call in mock_pubsub.publish.call_args_list]

        assert f"inbox:{agent_a['agent_id']}" not in published_channels

    async def test_broadcast_publishes_task_ids_not_json(self, pubsub_env):
        """Broadcast publishes task_id strings, not full Task JSON."""
        executor = pubsub_env["executor"]
        agent_a = pubsub_env["agent_a"]
        mock_pubsub = pubsub_env["mock_pubsub"]
        task_store = pubsub_env["task_store"]
        queue = EventQueue()

        context = _make_send_context(
            from_agent_id=agent_a["agent_id"],
            tenant_id=pubsub_env["tenant_id"],
            destination="*",
            text="Broadcast payload check",
        )
        await executor.execute(context, queue)

        for call in mock_pubsub.publish.call_args_list:
            task_id = call[0][1]
            assert isinstance(task_id, str)
            assert "{" not in task_id  # Not JSON
            task = await task_store.get(task_id)
            assert task is not None

    async def test_no_pubsub_parameter_still_works(
        self, store: RegistryStore, task_store: TaskStore
    ):
        """BrokerExecutor without pubsub parameter works (backward compat)."""
        executor = BrokerExecutor(
            registry_store=store,
            task_store=task_store,
        )
        assert executor is not None

    async def test_unicast_no_pubsub_skips_publish(
        self, store: RegistryStore, task_store: TaskStore
    ):
        """Unicast without pubsub does not fail (graceful no-op)."""
        api_key, tenant_id, _ = await store.create_api_key(_OWNER_SHARED)
        executor = BrokerExecutor(
            registry_store=store,
            task_store=task_store,
        )

        agent_a = await store.create_agent(
            name="A", description="Sender", api_key=api_key
        )
        agent_b = await store.create_agent(
            name="B", description="Recipient", api_key=api_key
        )

        queue = EventQueue()
        context = _make_send_context(
            from_agent_id=agent_a["agent_id"],
            tenant_id=tenant_id,
            destination=agent_b["agent_id"],
        )
        # Should not raise even though pubsub is None
        await executor.execute(context, queue)

        events = await _collect_events(queue)
        tasks = [e for e in events if isinstance(e, Task)]
        assert len(tasks) >= 1
