"""Tests for executor.py — BrokerExecutor business logic.

Design doc 0000015 Step 6 changes:
  - ``tenant_id`` renamed to ``session_id`` throughout
  - ``SessionMismatchError(ValueError)`` defined at module level
  - Cross-session unicast raises ``SessionMismatchError`` (not plain ``ValueError``)
  - Broadcast scoped by ``session_id``

Covers: unicast send, broadcast send, ACK (multi-turn), GetTask visibility,
CancelTask. Tests the executor methods directly with SQL-backed stores.
"""

import uuid
from datetime import UTC, datetime

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
from sqlalchemy.ext.asyncio import AsyncSession, async_sessionmaker

from hikyaku.db.models import Session
from hikyaku.executor import BrokerExecutor, SessionMismatchError
from hikyaku.registry_store import RegistryStore
from hikyaku.task_store import TaskStore


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------


async def _create_test_session(
    db_sessionmaker: async_sessionmaker[AsyncSession],
    *,
    session_id: str | None = None,
) -> str:
    """Seed a session row directly via the DB sessionmaker."""
    if session_id is None:
        session_id = str(uuid.uuid4())
    created_at = datetime.now(UTC).isoformat()
    async with db_sessionmaker() as session:
        async with session.begin():
            session.add(
                Session(session_id=session_id, label=None, created_at=created_at)
            )
    return session_id


def _make_call_context(agent_id: str, session_id: str) -> ServerCallContext:
    """Create a ServerCallContext with agent_id and session_id."""
    return ServerCallContext(
        state={"agent_id": agent_id, "session_id": session_id},
    )


def _make_send_context(
    from_agent_id: str,
    session_id: str,
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
        call_context=_make_call_context(from_agent_id, session_id),
    )


def _make_ack_context(
    from_agent_id: str,
    session_id: str,
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
        call_context=_make_call_context(from_agent_id, session_id),
    )


def _make_cancel_context(
    from_agent_id: str,
    session_id: str,
    task_id: str,
    task: Task | None = None,
) -> RequestContext:
    """Create a RequestContext for canceling a task."""
    return RequestContext(
        task_id=task_id,
        task=task,
        call_context=_make_call_context(from_agent_id, session_id),
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
# Single-session fixture
# ---------------------------------------------------------------------------


@pytest.fixture
async def env(store: RegistryStore, task_store: TaskStore, db_sessionmaker):
    """Set up BrokerExecutor with SQL-backed stores and test agents.

    All agents share the same session for basic tests.
    """
    session_id = await _create_test_session(db_sessionmaker)
    executor = BrokerExecutor(registry_store=store, task_store=task_store)

    agent_a = await store.create_agent(
        name="Agent A", description="Sender", session_id=session_id
    )
    agent_b = await store.create_agent(
        name="Agent B", description="Recipient", session_id=session_id
    )
    agent_c = await store.create_agent(
        name="Agent C", description="Third agent", session_id=session_id
    )

    return {
        "executor": executor,
        "store": store,
        "task_store": task_store,
        "session_id": session_id,
        "agent_a": agent_a,
        "agent_b": agent_b,
        "agent_c": agent_c,
    }


# ---------------------------------------------------------------------------
# SessionMismatchError class
# ---------------------------------------------------------------------------


class TestSessionMismatchError:
    """Verify ``SessionMismatchError`` exists and is a ``ValueError`` subclass."""

    def test_is_value_error_subclass(self):
        assert issubclass(SessionMismatchError, ValueError)

    def test_can_be_instantiated(self):
        err = SessionMismatchError("Session mismatch")
        assert str(err) == "Session mismatch"

    def test_caught_by_value_error_handler(self):
        """SessionMismatchError is caught by ``except ValueError``."""
        with pytest.raises(ValueError):
            raise SessionMismatchError("test")


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
            session_id=env["session_id"],
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
            session_id=env["session_id"],
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
            session_id=env["session_id"],
            destination=agent_b["agent_id"],
        )
        await executor.execute(context, queue)

        events = await _collect_events(queue)
        task = next(e for e in events if isinstance(e, Task))
        assert task.context_id == agent_b["agent_id"]

    async def test_task_metadata_has_from_and_to(self, env):
        """Delivery Task metadata has fromAgentId and toAgentId."""
        executor, agent_a, agent_b = env["executor"], env["agent_a"], env["agent_b"]
        queue = EventQueue()

        context = _make_send_context(
            from_agent_id=agent_a["agent_id"],
            session_id=env["session_id"],
            destination=agent_b["agent_id"],
        )
        await executor.execute(context, queue)

        events = await _collect_events(queue)
        task = next(e for e in events if isinstance(e, Task))
        assert task.metadata["fromAgentId"] == agent_a["agent_id"]
        assert task.metadata["toAgentId"] == agent_b["agent_id"]

    async def test_invalid_destination_raises_error(self, env):
        """Non-UUID destination raises ValueError."""
        executor, agent_a = env["executor"], env["agent_a"]
        queue = EventQueue()

        context = _make_send_context(
            from_agent_id=agent_a["agent_id"],
            session_id=env["session_id"],
            destination="not-a-uuid",
        )

        with pytest.raises(ValueError):
            await executor.execute(context, queue)

    async def test_deregistered_destination_raises_error(self, env):
        """Sending to a deregistered agent raises ValueError."""
        executor, store, agent_a, agent_b = (
            env["executor"],
            env["store"],
            env["agent_a"],
            env["agent_b"],
        )
        await store.deregister_agent(agent_b["agent_id"])
        queue = EventQueue()

        context = _make_send_context(
            from_agent_id=agent_a["agent_id"],
            session_id=env["session_id"],
            destination=agent_b["agent_id"],
        )

        with pytest.raises(ValueError):
            await executor.execute(context, queue)


# ---------------------------------------------------------------------------
# Broadcast Send
# ---------------------------------------------------------------------------


class TestBroadcastSend:
    """Tests for BrokerExecutor.execute — broadcast message delivery."""

    async def test_sends_to_all_session_agents(self, env):
        """Broadcast sends delivery tasks to all agents in the session."""
        executor, agent_a = env["executor"], env["agent_a"]
        queue = EventQueue()

        context = _make_send_context(
            from_agent_id=agent_a["agent_id"],
            session_id=env["session_id"],
            destination="*",
        )
        await executor.execute(context, queue)

        events = await _collect_events(queue)
        tasks = [e for e in events if isinstance(e, Task)]
        assert len(tasks) >= 3  # 2 delivery + 1 summary

    async def test_excludes_sender_from_recipients(self, env):
        """Sender does not receive their own broadcast."""
        executor, agent_a = env["executor"], env["agent_a"]
        queue = EventQueue()

        context = _make_send_context(
            from_agent_id=agent_a["agent_id"],
            session_id=env["session_id"],
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
            session_id=env["session_id"],
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

    async def test_delivery_tasks_have_input_required_state(self, env):
        """Each delivery Task is in INPUT_REQUIRED state."""
        executor, agent_a = env["executor"], env["agent_a"]
        queue = EventQueue()

        context = _make_send_context(
            from_agent_id=agent_a["agent_id"],
            session_id=env["session_id"],
            destination="*",
        )
        await executor.execute(context, queue)

        events = await _collect_events(queue)
        delivery_tasks = [
            e
            for e in events
            if isinstance(e, Task) and e.status.state == TaskState.input_required
        ]
        assert len(delivery_tasks) == 2

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

        await store.deregister_agent(agent_b["agent_id"])
        await store.deregister_agent(agent_c["agent_id"])

        context = _make_send_context(
            from_agent_id=agent_a["agent_id"],
            session_id=env["session_id"],
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
            session_id=env["session_id"],
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
            session_id=env["session_id"],
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
            session_id=env["session_id"],
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
            session_id=env["session_id"],
            task_id=delivery_task.id,
        )
        await executor.execute(ctx1, queue1)

        queue2 = EventQueue()
        ctx2 = _make_ack_context(
            from_agent_id=agent_b["agent_id"],
            session_id=env["session_id"],
            task_id=delivery_task.id,
        )

        with pytest.raises(Exception):
            await executor.execute(ctx2, queue2)

    async def test_ack_on_unknown_task_raises_error(self, env):
        """ACK on a non-existent task raises an error."""
        executor, agent_b = env["executor"], env["agent_b"]

        queue = EventQueue()
        context = _make_ack_context(
            from_agent_id=agent_b["agent_id"],
            session_id=env["session_id"],
            task_id="nonexistent-task-id",
        )

        with pytest.raises(Exception):
            await executor.execute(context, queue)


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
            session_id=env["session_id"],
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
            session_id=env["session_id"],
            task_id=delivery_task.id,
            task=delivery_task,
        )
        await executor.cancel(context, queue)

        events = await _collect_events(queue)
        assert any(
            isinstance(e, Task) and e.status.state == TaskState.canceled for e in events
        )

    async def test_non_sender_cannot_cancel(self, env):
        """Non-sender cannot cancel a task — raises error."""
        executor, agent_b = env["executor"], env["agent_b"]
        delivery_task = await self._create_unicast_task(env)

        queue = EventQueue()
        context = _make_cancel_context(
            from_agent_id=agent_b["agent_id"],
            session_id=env["session_id"],
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
            session_id=env["session_id"],
            task_id=delivery_task.id,
        )
        await executor.execute(ack_ctx, ack_queue)

        cancel_queue = EventQueue()
        cancel_ctx = _make_cancel_context(
            from_agent_id=agent_a["agent_id"],
            session_id=env["session_id"],
            task_id=delivery_task.id,
            task=delivery_task,
        )

        with pytest.raises(Exception):
            await executor.cancel(cancel_ctx, cancel_queue)

    async def test_cancel_unknown_task_raises_error(self, env):
        """Cannot cancel a task that doesn't exist."""
        executor, agent_a = env["executor"], env["agent_a"]

        queue = EventQueue()
        context = _make_cancel_context(
            from_agent_id=agent_a["agent_id"],
            session_id=env["session_id"],
            task_id="nonexistent-task-id",
        )

        with pytest.raises(Exception):
            await executor.cancel(context, queue)


# ===========================================================================
# Cross-session executor tests (renamed from multi-tenant)
# ===========================================================================


@pytest.fixture
async def session_env(store: RegistryStore, task_store: TaskStore, db_sessionmaker):
    """Set up BrokerExecutor with agents in two separate sessions.

    Session A: agent_a1, agent_a2
    Session B: agent_b1
    """
    session_a = await _create_test_session(db_sessionmaker)
    session_b = await _create_test_session(db_sessionmaker)

    executor = BrokerExecutor(registry_store=store, task_store=task_store)

    agent_a1 = await store.create_agent(
        name="Agent A1", description="Session A first", session_id=session_a
    )
    agent_a2 = await store.create_agent(
        name="Agent A2", description="Session A second", session_id=session_a
    )
    agent_b1 = await store.create_agent(
        name="Agent B1", description="Session B only", session_id=session_b
    )

    return {
        "executor": executor,
        "store": store,
        "task_store": task_store,
        "session_a": session_a,
        "session_b": session_b,
        "agent_a1": agent_a1,
        "agent_a2": agent_a2,
        "agent_b1": agent_b1,
    }


class TestCrossSessionUnicast:
    """Tests for cross-session unicast rejection.

    Cross-session sends raise ``SessionMismatchError`` (not plain ValueError).
    """

    async def test_same_session_unicast_succeeds(self, session_env):
        """Agent A1 sends to Agent A2 (same session) → succeeds."""
        executor = session_env["executor"]
        agent_a1, agent_a2 = session_env["agent_a1"], session_env["agent_a2"]
        queue = EventQueue()

        context = _make_send_context(
            from_agent_id=agent_a1["agent_id"],
            session_id=session_env["session_a"],
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

    async def test_cross_session_unicast_raises_session_mismatch_error(
        self, session_env
    ):
        """Agent A1 sends to Agent B1 (different session) → SessionMismatchError."""
        executor = session_env["executor"]
        agent_a1, agent_b1 = session_env["agent_a1"], session_env["agent_b1"]
        queue = EventQueue()

        context = _make_send_context(
            from_agent_id=agent_a1["agent_id"],
            session_id=session_env["session_a"],
            destination=agent_b1["agent_id"],
        )

        with pytest.raises(SessionMismatchError):
            await executor.execute(context, queue)

    async def test_cross_session_no_task_created(self, session_env):
        """Cross-session send does not persist any delivery task."""
        executor, task_store = session_env["executor"], session_env["task_store"]
        agent_a1, agent_b1 = session_env["agent_a1"], session_env["agent_b1"]
        queue = EventQueue()

        context = _make_send_context(
            from_agent_id=agent_a1["agent_id"],
            session_id=session_env["session_a"],
            destination=agent_b1["agent_id"],
        )

        with pytest.raises(SessionMismatchError):
            await executor.execute(context, queue)

        tasks = await task_store.list(agent_b1["agent_id"])
        assert len(tasks) == 0

    async def test_reverse_cross_session_also_blocked(self, session_env):
        """Agent B1 sends to Agent A1 (reverse direction) → also blocked."""
        executor = session_env["executor"]
        agent_a1, agent_b1 = session_env["agent_a1"], session_env["agent_b1"]
        queue = EventQueue()

        context = _make_send_context(
            from_agent_id=agent_b1["agent_id"],
            session_id=session_env["session_b"],
            destination=agent_a1["agent_id"],
        )

        with pytest.raises(SessionMismatchError):
            await executor.execute(context, queue)


class TestSessionScopedBroadcast:
    """Tests for session-scoped broadcast.

    Broadcast from session A → only delivers to agents in session A.
    """

    async def test_broadcast_delivers_only_to_same_session(self, session_env):
        """Broadcast from A1 delivers to A2 only, not B1."""
        executor = session_env["executor"]
        agent_a1, agent_a2 = session_env["agent_a1"], session_env["agent_a2"]
        agent_b1 = session_env["agent_b1"]
        queue = EventQueue()

        context = _make_send_context(
            from_agent_id=agent_a1["agent_id"],
            session_id=session_env["session_a"],
            destination="*",
            text="Session A broadcast",
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

    async def test_broadcast_excludes_sender(self, session_env):
        """Broadcast excludes the sender even within the same session."""
        executor = session_env["executor"]
        agent_a1 = session_env["agent_a1"]
        queue = EventQueue()

        context = _make_send_context(
            from_agent_id=agent_a1["agent_id"],
            session_id=session_env["session_a"],
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

    async def test_broadcast_delivery_count_matches_session_size(self, session_env):
        """Number of delivery tasks equals (session agents - 1)."""
        executor = session_env["executor"]
        agent_a1 = session_env["agent_a1"]
        queue = EventQueue()

        context = _make_send_context(
            from_agent_id=agent_a1["agent_id"],
            session_id=session_env["session_a"],
            destination="*",
        )
        await executor.execute(context, queue)

        events = await _collect_events(queue)
        delivery_tasks = [
            e
            for e in events
            if isinstance(e, Task) and e.status.state == TaskState.input_required
        ]
        # Session A has agent_a1 + agent_a2, so 1 delivery
        assert len(delivery_tasks) == 1
