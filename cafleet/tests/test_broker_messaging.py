"""Tests for broker.py — messaging operations.

Design doc 0000021 Step 4: messaging operations in the broker module.
All functions are sync, using ``get_sync_sessionmaker()`` from ``db/engine.py``.

Test isolation strategy:
  Same as test_broker_registry.py — each test gets a fresh in-memory SQLite
  database via the ``broker_session`` fixture with ``broker.get_sync_sessionmaker``
  monkeypatched.

Coverage map:
  | Function           | Test class                |
  |--------------------|---------------------------|
  | send_message       | TestSendMessage           |
  | broadcast_message  | TestBroadcastMessage      |
  | poll_tasks         | TestPollTasks             |
  | ack_task           | TestAckTask               |
  | cancel_task        | TestCancelTask            |
  | get_task           | TestGetTask               |
"""

import uuid

import pytest
from sqlalchemy import create_engine, event
from sqlalchemy.engine import Engine
from sqlalchemy.orm import sessionmaker

from cafleet.db.models import Base


# ---------------------------------------------------------------------------
# Fixtures
# ---------------------------------------------------------------------------


@event.listens_for(Engine, "connect")
def _enable_fk_pragma(dbapi_conn, _record):
    """Mirror the production pragma listener for test engines."""
    import sqlite3

    if isinstance(dbapi_conn, sqlite3.Connection):
        cursor = dbapi_conn.cursor()
        cursor.execute("PRAGMA foreign_keys=ON")
        cursor.close()


@pytest.fixture
def sync_sessionmaker():
    """Create a sync in-memory SQLite engine with all tables."""
    engine = create_engine("sqlite:///:memory:")
    Base.metadata.create_all(engine)
    return sessionmaker(engine, expire_on_commit=False)


@pytest.fixture
def _patch_broker(sync_sessionmaker, monkeypatch):
    """Monkeypatch broker.get_sync_sessionmaker to use the test engine."""
    from cafleet import broker

    monkeypatch.setattr(broker, "get_sync_sessionmaker", lambda: sync_sessionmaker)


@pytest.fixture(autouse=True)
def broker_session(sync_sessionmaker, _patch_broker):
    """Autouse fixture that sets up broker with a test DB for every test."""
    return sync_sessionmaker


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------


def _create_session(label: str | None = None) -> dict:
    from cafleet import broker

    return broker.create_session(label=label)


def _register_agent(
    session_id: str,
    name: str = "test-agent",
    description: str = "A test agent",
) -> dict:
    from cafleet import broker

    return broker.register_agent(
        session_id=session_id,
        name=name,
        description=description,
    )


def _setup_two_agents() -> tuple[str, str, str]:
    """Create a session with two agents. Returns (session_id, agent_a_id, agent_b_id)."""
    session = _create_session()
    sid = session["session_id"]
    agent_a = _register_agent(sid, name="sender")
    agent_b = _register_agent(sid, name="recipient")
    return sid, agent_a["agent_id"], agent_b["agent_id"]


def _setup_three_agents() -> tuple[str, str, str, str]:
    """Create a session with three agents. Returns (session_id, a_id, b_id, c_id)."""
    session = _create_session()
    sid = session["session_id"]
    a = _register_agent(sid, name="agent-a")
    b = _register_agent(sid, name="agent-b")
    c = _register_agent(sid, name="agent-c")
    return sid, a["agent_id"], b["agent_id"], c["agent_id"]


# ===========================================================================
# send_message
# ===========================================================================


class TestSendMessage:
    """broker.send_message(session_id, agent_id, to, text) → {"task": <dict>}."""

    def test_returns_dict_with_task_key(self):
        from cafleet import broker

        sid, sender, recipient = _setup_two_agents()
        result = broker.send_message(sid, sender, recipient, "Hello")
        assert isinstance(result, dict)
        assert "task" in result

    def test_task_has_camel_case_keys(self):
        from cafleet import broker

        sid, sender, recipient = _setup_two_agents()
        result = broker.send_message(sid, sender, recipient, "Hello")
        task = result["task"]
        assert "id" in task
        assert "contextId" in task
        assert "status" in task
        assert "artifacts" in task
        assert "metadata" in task

    def test_task_id_is_valid_uuid(self):
        from cafleet import broker

        sid, sender, recipient = _setup_two_agents()
        result = broker.send_message(sid, sender, recipient, "Hello")
        uuid.UUID(result["task"]["id"])

    def test_context_id_is_recipient(self):
        from cafleet import broker

        sid, sender, recipient = _setup_two_agents()
        result = broker.send_message(sid, sender, recipient, "Hello")
        assert result["task"]["contextId"] == recipient

    def test_status_state_is_input_required(self):
        from cafleet import broker

        sid, sender, recipient = _setup_two_agents()
        result = broker.send_message(sid, sender, recipient, "Hello")
        assert result["task"]["status"]["state"] == "input_required"

    def test_status_has_timestamp(self):
        from cafleet import broker

        sid, sender, recipient = _setup_two_agents()
        result = broker.send_message(sid, sender, recipient, "Hello")
        ts = result["task"]["status"]["timestamp"]
        assert isinstance(ts, str)
        assert "T" in ts

    def test_metadata_from_agent_id(self):
        from cafleet import broker

        sid, sender, recipient = _setup_two_agents()
        result = broker.send_message(sid, sender, recipient, "Hello")
        assert result["task"]["metadata"]["fromAgentId"] == sender

    def test_metadata_to_agent_id(self):
        from cafleet import broker

        sid, sender, recipient = _setup_two_agents()
        result = broker.send_message(sid, sender, recipient, "Hello")
        assert result["task"]["metadata"]["toAgentId"] == recipient

    def test_metadata_type_is_unicast(self):
        from cafleet import broker

        sid, sender, recipient = _setup_two_agents()
        result = broker.send_message(sid, sender, recipient, "Hello")
        assert result["task"]["metadata"]["type"] == "unicast"

    def test_artifact_contains_message_text(self):
        from cafleet import broker

        sid, sender, recipient = _setup_two_agents()
        result = broker.send_message(sid, sender, recipient, "Did the API change?")
        task = result["task"]
        # Extract text from artifacts
        texts = []
        for artifact in task.get("artifacts", []):
            for part in artifact.get("parts", []):
                if isinstance(part, dict) and "text" in part:
                    texts.append(part["text"])
        assert "Did the API change?" in texts

    def test_validates_destination_is_valid_uuid(self):
        from cafleet import broker

        sid, sender, _ = _setup_two_agents()
        with pytest.raises(Exception):
            broker.send_message(sid, sender, "not-a-uuid", "Hello")

    def test_validates_destination_agent_exists(self):
        from cafleet import broker

        sid, sender, _ = _setup_two_agents()
        fake_agent = str(uuid.uuid4())
        with pytest.raises(Exception):
            broker.send_message(sid, sender, fake_agent, "Hello")

    def test_validates_destination_agent_is_active(self):
        from cafleet import broker

        sid, sender, recipient = _setup_two_agents()
        broker.deregister_agent(recipient)
        with pytest.raises(Exception):
            broker.send_message(sid, sender, recipient, "Hello")

    def test_validates_destination_in_same_session(self):
        from cafleet import broker

        session_a = _create_session()
        session_b = _create_session()
        sender = _register_agent(session_a["session_id"], name="sender")
        recipient = _register_agent(session_b["session_id"], name="recipient")
        with pytest.raises(Exception):
            broker.send_message(
                session_a["session_id"],
                sender["agent_id"],
                recipient["agent_id"],
                "cross-session",
            )

    def test_task_persisted_to_db(self):
        """Sent task can be retrieved via poll_tasks."""
        from cafleet import broker

        sid, sender, recipient = _setup_two_agents()
        broker.send_message(sid, sender, recipient, "persisted?")

        tasks = broker.poll_tasks(recipient)
        assert len(tasks) >= 1
        texts = []
        for t in tasks:
            for a in t.get("artifacts", []):
                for p in a.get("parts", []):
                    if isinstance(p, dict) and "text" in p:
                        texts.append(p["text"])
        assert "persisted?" in texts


# ===========================================================================
# broadcast_message
# ===========================================================================


class TestBroadcastMessage:
    """broker.broadcast_message(session_id, agent_id, text) → [{"task": <summary>}]."""

    def test_returns_list_with_summary(self):
        from cafleet import broker

        sid, sender, _, _ = _setup_three_agents()
        result = broker.broadcast_message(sid, sender, "Attention all")
        assert isinstance(result, list)
        assert len(result) >= 1
        assert "task" in result[0]

    def test_summary_type_is_broadcast_summary(self):
        from cafleet import broker

        sid, sender, _, _ = _setup_three_agents()
        result = broker.broadcast_message(sid, sender, "Attention all")
        summary = result[0]["task"]
        assert summary["metadata"]["type"] == "broadcast_summary"

    def test_summary_context_id_is_sender(self):
        from cafleet import broker

        sid, sender, _, _ = _setup_three_agents()
        result = broker.broadcast_message(sid, sender, "Attention all")
        summary = result[0]["task"]
        assert summary["contextId"] == sender

    def test_creates_delivery_tasks_for_each_recipient(self):
        """Each non-sender agent gets a delivery task."""
        from cafleet import broker

        sid, sender, b_id, c_id = _setup_three_agents()
        broker.broadcast_message(sid, sender, "Hello everyone")

        # Both b and c should have inbox tasks
        b_tasks = broker.poll_tasks(b_id)
        c_tasks = broker.poll_tasks(c_id)
        assert len(b_tasks) >= 1
        assert len(c_tasks) >= 1

    def test_delivery_tasks_have_origin_task_id(self):
        """Delivery tasks reference the summary via originTaskId."""
        from cafleet import broker

        sid, sender, b_id, _ = _setup_three_agents()
        result = broker.broadcast_message(sid, sender, "Hello")
        summary_id = result[0]["task"]["id"]

        b_tasks = broker.poll_tasks(b_id)
        assert len(b_tasks) >= 1
        origin_ids = [
            t.get("metadata", {}).get("originTaskId") for t in b_tasks
        ]
        assert summary_id in origin_ids

    def test_delivery_tasks_type_is_unicast(self):
        """Individual delivery tasks have type='unicast'."""
        from cafleet import broker

        sid, sender, b_id, _ = _setup_three_agents()
        broker.broadcast_message(sid, sender, "Hello")

        b_tasks = broker.poll_tasks(b_id)
        assert len(b_tasks) >= 1
        assert b_tasks[0]["metadata"]["type"] == "unicast"

    def test_excludes_sender_from_recipients(self):
        """Sender does not receive a delivery task in their inbox."""
        from cafleet import broker

        sid, sender, _, _ = _setup_three_agents()
        broker.broadcast_message(sid, sender, "Hello")

        # Sender's poll should not return delivery tasks (broadcast_summary
        # is filtered out by poll_tasks)
        sender_tasks = broker.poll_tasks(sender)
        delivery_tasks = [
            t for t in sender_tasks
            if t.get("metadata", {}).get("type") == "unicast"
        ]
        assert len(delivery_tasks) == 0

    def test_broadcast_with_no_other_agents(self):
        """Broadcast when sender is the only agent: no delivery tasks, still returns summary."""
        from cafleet import broker

        session = _create_session()
        sid = session["session_id"]
        lone_agent = _register_agent(sid, name="lonely")

        result = broker.broadcast_message(sid, lone_agent["agent_id"], "Anyone?")
        assert isinstance(result, list)
        assert len(result) >= 1
        # Summary should exist
        summary = result[0]["task"]
        assert summary["metadata"]["type"] == "broadcast_summary"

    def test_delivery_task_contains_message_text(self):
        from cafleet import broker

        sid, sender, b_id, _ = _setup_three_agents()
        broker.broadcast_message(sid, sender, "Important update")

        b_tasks = broker.poll_tasks(b_id)
        texts = []
        for t in b_tasks:
            for a in t.get("artifacts", []):
                for p in a.get("parts", []):
                    if isinstance(p, dict) and "text" in p:
                        texts.append(p["text"])
        assert "Important update" in texts


# ===========================================================================
# poll_tasks
# ===========================================================================


class TestPollTasks:
    """broker.poll_tasks(agent_id, since, page_size, status) → list of task dicts."""

    def test_returns_empty_list_when_no_tasks(self):
        from cafleet import broker

        session = _create_session()
        agent = _register_agent(session["session_id"], name="idle")
        result = broker.poll_tasks(agent["agent_id"])
        assert result == []

    def test_returns_tasks_for_agent(self):
        from cafleet import broker

        sid, sender, recipient = _setup_two_agents()
        broker.send_message(sid, sender, recipient, "msg1")
        broker.send_message(sid, sender, recipient, "msg2")

        result = broker.poll_tasks(recipient)
        assert len(result) == 2

    def test_returns_camel_case_task_dicts(self):
        from cafleet import broker

        sid, sender, recipient = _setup_two_agents()
        broker.send_message(sid, sender, recipient, "Hello")

        result = broker.poll_tasks(recipient)
        assert len(result) == 1
        task = result[0]
        assert "id" in task
        assert "contextId" in task
        assert "status" in task
        assert "metadata" in task

    def test_ordered_by_status_timestamp_desc(self):
        """Most recent tasks first."""
        from cafleet import broker

        sid, sender, recipient = _setup_two_agents()
        broker.send_message(sid, sender, recipient, "first")
        broker.send_message(sid, sender, recipient, "second")

        result = broker.poll_tasks(recipient)
        assert len(result) == 2
        ts0 = result[0]["status"]["timestamp"]
        ts1 = result[1]["status"]["timestamp"]
        assert ts0 >= ts1  # DESC order

    def test_filters_out_broadcast_summary(self):
        """broadcast_summary tasks do not appear in poll results."""
        from cafleet import broker

        sid, sender, b_id, _ = _setup_three_agents()
        broker.broadcast_message(sid, sender, "broadcast")

        # Sender's poll should not show the broadcast_summary
        sender_tasks = broker.poll_tasks(sender)
        summary_tasks = [
            t for t in sender_tasks
            if t.get("metadata", {}).get("type") == "broadcast_summary"
        ]
        assert len(summary_tasks) == 0

    def test_only_returns_tasks_for_specified_agent(self):
        """Tasks for other agents are not returned."""
        from cafleet import broker

        sid, sender, recipient = _setup_two_agents()
        broker.send_message(sid, sender, recipient, "for-recipient")

        sender_tasks = broker.poll_tasks(sender)
        # Sender should have no inbox tasks (they sent, not received)
        assert len(sender_tasks) == 0

    def test_status_filter(self):
        """Filter tasks by status state."""
        from cafleet import broker

        sid, sender, recipient = _setup_two_agents()
        result1 = broker.send_message(sid, sender, recipient, "msg1")
        broker.send_message(sid, sender, recipient, "msg2")
        # ACK the first task
        broker.ack_task(recipient, result1["task"]["id"])

        # Filter by input_required
        pending = broker.poll_tasks(recipient, status="input_required")
        assert len(pending) == 1

        # Filter by completed
        completed = broker.poll_tasks(recipient, status="completed")
        assert len(completed) == 1

    def test_page_size_limits_results(self):
        """page_size limits the number of returned tasks."""
        from cafleet import broker

        sid, sender, recipient = _setup_two_agents()
        broker.send_message(sid, sender, recipient, "msg1")
        broker.send_message(sid, sender, recipient, "msg2")
        broker.send_message(sid, sender, recipient, "msg3")

        result = broker.poll_tasks(recipient, page_size=2)
        assert len(result) == 2

    def test_since_filter(self):
        """since filters tasks by timestamp."""
        from cafleet import broker

        sid, sender, recipient = _setup_two_agents()
        result1 = broker.send_message(sid, sender, recipient, "early")
        ts = result1["task"]["status"]["timestamp"]

        broker.send_message(sid, sender, recipient, "later")

        # Filtering since the first message's timestamp should return
        # at least the later message
        result = broker.poll_tasks(recipient, since=ts)
        assert len(result) >= 1


# ===========================================================================
# ack_task
# ===========================================================================


class TestAckTask:
    """broker.ack_task(agent_id, task_id) → {"task": <updated dict>}."""

    def test_returns_dict_with_task_key(self):
        from cafleet import broker

        sid, sender, recipient = _setup_two_agents()
        sent = broker.send_message(sid, sender, recipient, "Please ack")
        task_id = sent["task"]["id"]

        result = broker.ack_task(recipient, task_id)
        assert isinstance(result, dict)
        assert "task" in result

    def test_transitions_to_completed(self):
        from cafleet import broker

        sid, sender, recipient = _setup_two_agents()
        sent = broker.send_message(sid, sender, recipient, "Ack me")
        task_id = sent["task"]["id"]

        result = broker.ack_task(recipient, task_id)
        assert result["task"]["status"]["state"] == "completed"

    def test_updates_timestamp(self):
        from cafleet import broker

        sid, sender, recipient = _setup_two_agents()
        sent = broker.send_message(sid, sender, recipient, "Ack me")
        task_id = sent["task"]["id"]
        original_ts = sent["task"]["status"]["timestamp"]

        result = broker.ack_task(recipient, task_id)
        assert result["task"]["status"]["timestamp"] >= original_ts

    def test_verifies_context_id_matches_agent(self):
        """Only the recipient (context_id) can ACK."""
        from cafleet import broker

        sid, sender, recipient = _setup_two_agents()
        sent = broker.send_message(sid, sender, recipient, "Ack me")
        task_id = sent["task"]["id"]

        # Sender should not be able to ACK
        with pytest.raises(Exception):
            broker.ack_task(sender, task_id)

    def test_verifies_state_is_input_required(self):
        """Cannot ACK a task that is already completed."""
        from cafleet import broker

        sid, sender, recipient = _setup_two_agents()
        sent = broker.send_message(sid, sender, recipient, "Ack me")
        task_id = sent["task"]["id"]

        broker.ack_task(recipient, task_id)
        # Second ACK should fail
        with pytest.raises(Exception):
            broker.ack_task(recipient, task_id)

    def test_cannot_ack_canceled_task(self):
        """Cannot ACK a task that has been canceled."""
        from cafleet import broker

        sid, sender, recipient = _setup_two_agents()
        sent = broker.send_message(sid, sender, recipient, "Cancel me")
        task_id = sent["task"]["id"]

        broker.cancel_task(sender, task_id)
        with pytest.raises(Exception):
            broker.ack_task(recipient, task_id)

    def test_ack_persists_state(self):
        """After ACK, polling shows the task as completed."""
        from cafleet import broker

        sid, sender, recipient = _setup_two_agents()
        sent = broker.send_message(sid, sender, recipient, "Ack me")
        task_id = sent["task"]["id"]

        broker.ack_task(recipient, task_id)

        tasks = broker.poll_tasks(recipient)
        acked = [t for t in tasks if t["id"] == task_id]
        assert len(acked) == 1
        assert acked[0]["status"]["state"] == "completed"


# ===========================================================================
# cancel_task
# ===========================================================================


class TestCancelTask:
    """broker.cancel_task(agent_id, task_id) → {"task": <updated dict>}."""

    def test_returns_dict_with_task_key(self):
        from cafleet import broker

        sid, sender, recipient = _setup_two_agents()
        sent = broker.send_message(sid, sender, recipient, "Cancel me")
        task_id = sent["task"]["id"]

        result = broker.cancel_task(sender, task_id)
        assert isinstance(result, dict)
        assert "task" in result

    def test_transitions_to_canceled(self):
        from cafleet import broker

        sid, sender, recipient = _setup_two_agents()
        sent = broker.send_message(sid, sender, recipient, "Cancel me")
        task_id = sent["task"]["id"]

        result = broker.cancel_task(sender, task_id)
        assert result["task"]["status"]["state"] == "canceled"

    def test_updates_timestamp(self):
        from cafleet import broker

        sid, sender, recipient = _setup_two_agents()
        sent = broker.send_message(sid, sender, recipient, "Cancel me")
        task_id = sent["task"]["id"]
        original_ts = sent["task"]["status"]["timestamp"]

        result = broker.cancel_task(sender, task_id)
        assert result["task"]["status"]["timestamp"] >= original_ts

    def test_verifies_from_agent_id_matches(self):
        """Only the sender (metadata.fromAgentId) can cancel."""
        from cafleet import broker

        sid, sender, recipient = _setup_two_agents()
        sent = broker.send_message(sid, sender, recipient, "Cancel me")
        task_id = sent["task"]["id"]

        # Recipient should not be able to cancel
        with pytest.raises(Exception):
            broker.cancel_task(recipient, task_id)

    def test_verifies_state_is_input_required(self):
        """Cannot cancel a task that is already completed."""
        from cafleet import broker

        sid, sender, recipient = _setup_two_agents()
        sent = broker.send_message(sid, sender, recipient, "Ack then cancel")
        task_id = sent["task"]["id"]

        broker.ack_task(recipient, task_id)
        with pytest.raises(Exception):
            broker.cancel_task(sender, task_id)

    def test_cannot_cancel_already_canceled(self):
        """Cannot cancel a task that is already canceled."""
        from cafleet import broker

        sid, sender, recipient = _setup_two_agents()
        sent = broker.send_message(sid, sender, recipient, "Cancel me twice")
        task_id = sent["task"]["id"]

        broker.cancel_task(sender, task_id)
        with pytest.raises(Exception):
            broker.cancel_task(sender, task_id)

    def test_cancel_persists_state(self):
        """After cancel, polling shows the task as canceled."""
        from cafleet import broker

        sid, sender, recipient = _setup_two_agents()
        sent = broker.send_message(sid, sender, recipient, "Cancel me")
        task_id = sent["task"]["id"]

        broker.cancel_task(sender, task_id)

        tasks = broker.poll_tasks(recipient)
        canceled = [t for t in tasks if t["id"] == task_id]
        assert len(canceled) == 1
        assert canceled[0]["status"]["state"] == "canceled"


# ===========================================================================
# get_task
# ===========================================================================


class TestGetTask:
    """broker.get_task(session_id, task_id) → {"task": <dict>}."""

    def test_returns_dict_with_task_key(self):
        from cafleet import broker

        sid, sender, recipient = _setup_two_agents()
        sent = broker.send_message(sid, sender, recipient, "Get me")
        task_id = sent["task"]["id"]

        result = broker.get_task(sid, task_id)
        assert isinstance(result, dict)
        assert "task" in result

    def test_returns_correct_task(self):
        from cafleet import broker

        sid, sender, recipient = _setup_two_agents()
        sent = broker.send_message(sid, sender, recipient, "Specific task")
        task_id = sent["task"]["id"]

        result = broker.get_task(sid, task_id)
        assert result["task"]["id"] == task_id

    def test_task_has_full_structure(self):
        from cafleet import broker

        sid, sender, recipient = _setup_two_agents()
        sent = broker.send_message(sid, sender, recipient, "Full structure")
        task_id = sent["task"]["id"]

        result = broker.get_task(sid, task_id)
        task = result["task"]
        assert "id" in task
        assert "contextId" in task
        assert "status" in task
        assert "artifacts" in task
        assert "metadata" in task

    def test_verifies_agent_belongs_to_session(self):
        """get_task verifies fromAgentId or toAgentId belongs to the given session."""
        from cafleet import broker

        session_a = _create_session()
        session_b = _create_session()
        sid_a = session_a["session_id"]
        sid_b = session_b["session_id"]

        sender = _register_agent(sid_a, name="sender")
        recipient = _register_agent(sid_a, name="recipient")

        sent = broker.send_message(sid_a, sender["agent_id"], recipient["agent_id"], "Hi")
        task_id = sent["task"]["id"]

        # Should succeed with the correct session
        result = broker.get_task(sid_a, task_id)
        assert result["task"]["id"] == task_id

        # Should fail with a different session
        with pytest.raises(Exception):
            broker.get_task(sid_b, task_id)

    def test_nonexistent_task_raises(self):
        from cafleet import broker

        session = _create_session()
        with pytest.raises(Exception):
            broker.get_task(session["session_id"], str(uuid.uuid4()))

    def test_sender_can_get_task(self):
        """Sender (fromAgentId) can retrieve the task."""
        from cafleet import broker

        sid, sender, recipient = _setup_two_agents()
        sent = broker.send_message(sid, sender, recipient, "sender gets")
        task_id = sent["task"]["id"]

        result = broker.get_task(sid, task_id)
        assert result["task"]["id"] == task_id

    def test_recipient_can_get_task(self):
        """Recipient (toAgentId/contextId) can retrieve the task."""
        from cafleet import broker

        sid, sender, recipient = _setup_two_agents()
        sent = broker.send_message(sid, sender, recipient, "recipient gets")
        task_id = sent["task"]["id"]

        result = broker.get_task(sid, task_id)
        assert result["task"]["id"] == task_id
