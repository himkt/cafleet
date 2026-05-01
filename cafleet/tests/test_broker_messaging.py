"""Tests for ``broker`` messaging operations."""

import uuid

import pytest
from sqlalchemy import create_engine
from sqlalchemy.orm import sessionmaker

import cafleet.db.engine  # noqa: F401 — registers PRAGMA listener globally
from cafleet import broker
from cafleet.db.models import Base
from cafleet.tmux import DirectorContext


@pytest.fixture
def sync_sessionmaker():
    engine = create_engine("sqlite:///:memory:")
    Base.metadata.create_all(engine)
    return sessionmaker(engine, expire_on_commit=False)


@pytest.fixture
def _patch_broker(sync_sessionmaker, monkeypatch):
    monkeypatch.setattr(broker, "get_sync_sessionmaker", lambda: sync_sessionmaker)


@pytest.fixture(autouse=True)
def broker_session(sync_sessionmaker, _patch_broker):
    return sync_sessionmaker


def _create_session(label: str | None = None) -> dict:
    return broker.create_session(
        label=label,
        director_context=DirectorContext(session="main", window_id="@3", pane_id="%0"),
    )


def _register_agent(
    session_id: str,
    name: str = "test-agent",
    description: str = "A test agent",
) -> dict:
    return broker.register_agent(
        session_id=session_id,
        name=name,
        description=description,
    )


def _setup_two_agents() -> tuple[str, str, str]:
    session = _create_session()
    sid = session["session_id"]
    agent_a = _register_agent(sid, name="sender")
    agent_b = _register_agent(sid, name="recipient")
    return sid, agent_a["agent_id"], agent_b["agent_id"]


def _setup_three_agents() -> tuple[str, str, str, str]:
    session = _create_session()
    sid = session["session_id"]
    a = _register_agent(sid, name="agent-a")
    b = _register_agent(sid, name="agent-b")
    c = _register_agent(sid, name="agent-c")
    return sid, a["agent_id"], b["agent_id"], c["agent_id"]


# --- send_message: broker.send_message(session_id, agent_id, to, text) → {"task": <dict>} ---


def test_send_message__returns_dict_with_task_key():
    sid, sender, recipient = _setup_two_agents()
    result = broker.send_message(sid, sender, recipient, "Hello")
    assert isinstance(result, dict)
    assert "task" in result


def test_send_message__task_has_camel_case_keys():
    sid, sender, recipient = _setup_two_agents()
    result = broker.send_message(sid, sender, recipient, "Hello")
    task = result["task"]
    assert "id" in task
    assert "contextId" in task
    assert "status" in task
    assert "artifacts" in task
    assert "metadata" in task


def test_send_message__task_id_is_valid_uuid():
    sid, sender, recipient = _setup_two_agents()
    result = broker.send_message(sid, sender, recipient, "Hello")
    uuid.UUID(result["task"]["id"])


def test_send_message__context_id_is_recipient():
    sid, sender, recipient = _setup_two_agents()
    result = broker.send_message(sid, sender, recipient, "Hello")
    assert result["task"]["contextId"] == recipient


def test_send_message__status_state_is_input_required():
    sid, sender, recipient = _setup_two_agents()
    result = broker.send_message(sid, sender, recipient, "Hello")
    assert result["task"]["status"]["state"] == "input_required"


def test_send_message__status_has_timestamp():
    sid, sender, recipient = _setup_two_agents()
    result = broker.send_message(sid, sender, recipient, "Hello")
    ts = result["task"]["status"]["timestamp"]
    assert isinstance(ts, str)
    assert "T" in ts


def test_send_message__metadata_from_agent_id():
    sid, sender, recipient = _setup_two_agents()
    result = broker.send_message(sid, sender, recipient, "Hello")
    assert result["task"]["metadata"]["fromAgentId"] == sender


def test_send_message__metadata_to_agent_id():
    sid, sender, recipient = _setup_two_agents()
    result = broker.send_message(sid, sender, recipient, "Hello")
    assert result["task"]["metadata"]["toAgentId"] == recipient


def test_send_message__metadata_type_is_unicast():
    sid, sender, recipient = _setup_two_agents()
    result = broker.send_message(sid, sender, recipient, "Hello")
    assert result["task"]["metadata"]["type"] == "unicast"


def test_send_message__artifact_contains_message_text():
    sid, sender, recipient = _setup_two_agents()
    result = broker.send_message(sid, sender, recipient, "Did the API change?")
    texts = [
        part["text"]
        for artifact in result["task"]["artifacts"]
        for part in artifact["parts"]
        if "text" in part
    ]
    assert "Did the API change?" in texts


def test_send_message__validates_destination_is_valid_uuid():
    sid, sender, _ = _setup_two_agents()
    with pytest.raises(ValueError, match="Invalid destination format"):
        broker.send_message(sid, sender, "not-a-uuid", "Hello")


def test_send_message__validates_destination_agent_exists():
    sid, sender, _ = _setup_two_agents()
    fake_agent = str(uuid.uuid4())
    with pytest.raises(ValueError, match="Destination agent not found"):
        broker.send_message(sid, sender, fake_agent, "Hello")


def test_send_message__validates_destination_agent_is_active():
    sid, sender, recipient = _setup_two_agents()
    broker.deregister_agent(recipient)
    with pytest.raises(ValueError, match="Destination agent not found"):
        broker.send_message(sid, sender, recipient, "Hello")


def test_send_message__validates_destination_in_same_session():
    session_a = _create_session()
    session_b = _create_session()
    sender = _register_agent(session_a["session_id"], name="sender")
    recipient = _register_agent(session_b["session_id"], name="recipient")
    with pytest.raises(ValueError, match="Destination agent not in session"):
        broker.send_message(
            session_a["session_id"],
            sender["agent_id"],
            recipient["agent_id"],
            "cross-session",
        )


def test_send_message__task_persisted_to_db():
    sid, sender, recipient = _setup_two_agents()
    broker.send_message(sid, sender, recipient, "persisted?")

    tasks = broker.poll_tasks(recipient)
    assert len(tasks) == 1
    texts = [
        p["text"]
        for t in tasks
        for a in t["artifacts"]
        for p in a["parts"]
        if "text" in p
    ]
    assert "persisted?" in texts


# --- broadcast_message: broker.broadcast_message(session_id, agent_id, text) → [{"task": <summary>}] ---


def test_broadcast_message__returns_list_with_summary():
    sid, sender, _, _ = _setup_three_agents()
    result = broker.broadcast_message(sid, sender, "Attention all")
    assert isinstance(result, list)
    assert len(result) == 1
    assert "task" in result[0]


def test_broadcast_message__summary_type_is_broadcast_summary():
    sid, sender, _, _ = _setup_three_agents()
    result = broker.broadcast_message(sid, sender, "Attention all")
    summary = result[0]["task"]
    assert summary["metadata"]["type"] == "broadcast_summary"


def test_broadcast_message__summary_context_id_is_sender():
    sid, sender, _, _ = _setup_three_agents()
    result = broker.broadcast_message(sid, sender, "Attention all")
    summary = result[0]["task"]
    assert summary["contextId"] == sender


def test_broadcast_message__creates_delivery_tasks_for_each_recipient():
    sid, sender, b_id, c_id = _setup_three_agents()
    broker.broadcast_message(sid, sender, "Hello everyone")

    b_tasks = broker.poll_tasks(b_id)
    c_tasks = broker.poll_tasks(c_id)
    assert len(b_tasks) == 1
    assert len(c_tasks) == 1


def test_broadcast_message__delivery_tasks_have_origin_task_id():
    sid, sender, b_id, _ = _setup_three_agents()
    result = broker.broadcast_message(sid, sender, "Hello")
    summary_id = result[0]["task"]["id"]

    b_tasks = broker.poll_tasks(b_id)
    assert len(b_tasks) == 1
    assert b_tasks[0]["metadata"]["originTaskId"] == summary_id


def test_broadcast_message__delivery_tasks_type_is_unicast():
    sid, sender, b_id, _ = _setup_three_agents()
    broker.broadcast_message(sid, sender, "Hello")

    b_tasks = broker.poll_tasks(b_id)
    assert len(b_tasks) == 1
    assert b_tasks[0]["metadata"]["type"] == "unicast"


def test_broadcast_message__excludes_sender_from_recipients():
    sid, sender, _, _ = _setup_three_agents()
    broker.broadcast_message(sid, sender, "Hello")

    sender_tasks = broker.poll_tasks(sender)
    delivery_tasks = [t for t in sender_tasks if t["metadata"]["type"] == "unicast"]
    assert len(delivery_tasks) == 0


def test_broadcast_message__broadcast_with_no_other_agents():
    session = _create_session()
    sid = session["session_id"]
    lone_agent = _register_agent(sid, name="lonely")

    result = broker.broadcast_message(sid, lone_agent["agent_id"], "Anyone?")
    assert isinstance(result, list)
    assert len(result) == 1
    summary = result[0]["task"]
    assert summary["metadata"]["type"] == "broadcast_summary"


def test_broadcast_message__delivery_task_contains_message_text():
    sid, sender, b_id, _ = _setup_three_agents()
    broker.broadcast_message(sid, sender, "Important update")

    b_tasks = broker.poll_tasks(b_id)
    texts = [
        p["text"]
        for t in b_tasks
        for a in t["artifacts"]
        for p in a["parts"]
        if "text" in p
    ]
    assert "Important update" in texts


def _get_summary_artifact_text(result: list[dict]) -> str:
    summary = result[0]["task"]
    artifacts = summary["artifacts"]
    assert len(artifacts) == 1
    parts = artifacts[0]["parts"]
    text_parts = [p["text"] for p in parts if "text" in p]
    assert len(text_parts) == 1
    return text_parts[0]


def test_broadcast_administrator_exclusion__broadcast_from_user_excludes_administrator_from_recipients():
    session = _create_session()
    sid = session["session_id"]
    admin_id = session["administrator_agent_id"]

    sender = _register_agent(sid, name="sender")
    user_a = _register_agent(sid, name="user-a")
    user_b = _register_agent(sid, name="user-b")

    broker.broadcast_message(sid, sender["agent_id"], "Hi all")

    admin_tasks = broker.poll_tasks(admin_id)
    admin_unicasts = [t for t in admin_tasks if t["metadata"]["type"] == "unicast"]
    assert len(admin_unicasts) == 0

    a_tasks = broker.poll_tasks(user_a["agent_id"])
    b_tasks = broker.poll_tasks(user_b["agent_id"])
    assert len(a_tasks) == 1
    assert len(b_tasks) == 1


def test_broadcast_administrator_exclusion__summary_count_reflects_post_exclusion_recipients():
    session = _create_session()
    sid = session["session_id"]
    admin_id = session["administrator_agent_id"]
    director_id = session["director"]["agent_id"]

    sender = _register_agent(sid, name="sender")
    user_a = _register_agent(sid, name="user-a")
    user_b = _register_agent(sid, name="user-b")

    result = broker.broadcast_message(sid, sender["agent_id"], "hey")

    text = _get_summary_artifact_text(result)
    assert text == "Broadcast sent to 3 recipients"

    summary_metadata = result[0]["task"]["metadata"]
    assert summary_metadata["recipientCount"] == 3

    recipient_ids = summary_metadata["recipientIds"]
    assert admin_id not in recipient_ids
    assert set(recipient_ids) == {
        user_a["agent_id"],
        user_b["agent_id"],
        director_id,
    }


def test_broadcast_administrator_exclusion__broadcast_from_administrator_delivers_to_all_user_agents():
    session = _create_session()
    sid = session["session_id"]
    admin_id = session["administrator_agent_id"]
    director_id = session["director"]["agent_id"]

    user_a = _register_agent(sid, name="user-a")
    user_b = _register_agent(sid, name="user-b")

    result = broker.broadcast_message(sid, admin_id, "hello from admin")

    a_tasks = broker.poll_tasks(user_a["agent_id"])
    b_tasks = broker.poll_tasks(user_b["agent_id"])
    assert len(a_tasks) == 1
    assert len(b_tasks) == 1

    text = _get_summary_artifact_text(result)
    assert text == "Broadcast sent to 3 recipients"

    summary_metadata = result[0]["task"]["metadata"]
    recipient_ids = summary_metadata["recipientIds"]
    assert set(recipient_ids) == {
        user_a["agent_id"],
        user_b["agent_id"],
        director_id,
    }


def test_broadcast_administrator_exclusion__admin_broadcast_in_bootstrap_only_session_reaches_only_director():
    session = _create_session()
    sid = session["session_id"]
    admin_id = session["administrator_agent_id"]
    director_id = session["director"]["agent_id"]

    result = broker.broadcast_message(sid, admin_id, "anybody?")

    text = _get_summary_artifact_text(result)
    assert text == "Broadcast sent to 1 recipients"

    summary_metadata = result[0]["task"]["metadata"]
    assert summary_metadata["recipientCount"] == 1
    assert summary_metadata["recipientIds"] == [director_id]


# --- poll_tasks: broker.poll_tasks(agent_id, since, page_size, status) → list of task dicts ---


def test_poll_tasks__returns_empty_list_when_no_tasks():
    session = _create_session()
    agent = _register_agent(session["session_id"], name="idle")
    result = broker.poll_tasks(agent["agent_id"])
    assert result == []


def test_poll_tasks__returns_tasks_for_agent():
    sid, sender, recipient = _setup_two_agents()
    broker.send_message(sid, sender, recipient, "msg1")
    broker.send_message(sid, sender, recipient, "msg2")

    result = broker.poll_tasks(recipient)
    assert len(result) == 2


def test_poll_tasks__returns_camel_case_task_dicts():
    sid, sender, recipient = _setup_two_agents()
    broker.send_message(sid, sender, recipient, "Hello")

    result = broker.poll_tasks(recipient)
    assert len(result) == 1
    task = result[0]
    assert "id" in task
    assert "contextId" in task
    assert "status" in task
    assert "metadata" in task


def test_poll_tasks__ordered_by_status_timestamp_desc():
    """Most recent tasks first."""
    sid, sender, recipient = _setup_two_agents()
    broker.send_message(sid, sender, recipient, "first")
    broker.send_message(sid, sender, recipient, "second")

    result = broker.poll_tasks(recipient)
    assert len(result) == 2
    ts0 = result[0]["status"]["timestamp"]
    ts1 = result[1]["status"]["timestamp"]
    assert ts0 >= ts1  # DESC order


def test_poll_tasks__filters_out_broadcast_summary():
    sid, sender, _b_id, _ = _setup_three_agents()
    broker.broadcast_message(sid, sender, "broadcast")

    sender_tasks = broker.poll_tasks(sender)
    summary_tasks = [
        t for t in sender_tasks if t["metadata"]["type"] == "broadcast_summary"
    ]
    assert len(summary_tasks) == 0


def test_poll_tasks__only_returns_tasks_for_specified_agent():
    """Tasks for other agents are not returned."""
    sid, sender, recipient = _setup_two_agents()
    broker.send_message(sid, sender, recipient, "for-recipient")

    sender_tasks = broker.poll_tasks(sender)
    # Sender should have no inbox tasks (they sent, not received)
    assert len(sender_tasks) == 0


def test_poll_tasks__status_filter():
    """Filter tasks by status state."""
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


def test_poll_tasks__page_size_limits_results():
    """page_size limits the number of returned tasks."""
    sid, sender, recipient = _setup_two_agents()
    broker.send_message(sid, sender, recipient, "msg1")
    broker.send_message(sid, sender, recipient, "msg2")
    broker.send_message(sid, sender, recipient, "msg3")

    result = broker.poll_tasks(recipient, page_size=2)
    assert len(result) == 2


def test_poll_tasks__since_filter():
    sid, sender, recipient = _setup_two_agents()
    result1 = broker.send_message(sid, sender, recipient, "early")
    ts = result1["task"]["status"]["timestamp"]

    broker.send_message(sid, sender, recipient, "later")

    result = broker.poll_tasks(recipient, since=ts)
    assert len(result) >= 1


# --- ack_task: broker.ack_task(agent_id, task_id) → {"task": <updated dict>} ---


def test_ack_task__returns_dict_with_task_key():
    sid, sender, recipient = _setup_two_agents()
    sent = broker.send_message(sid, sender, recipient, "Please ack")
    task_id = sent["task"]["id"]

    result = broker.ack_task(recipient, task_id)
    assert isinstance(result, dict)
    assert "task" in result


def test_ack_task__transitions_to_completed():
    sid, sender, recipient = _setup_two_agents()
    sent = broker.send_message(sid, sender, recipient, "Ack me")
    task_id = sent["task"]["id"]

    result = broker.ack_task(recipient, task_id)
    assert result["task"]["status"]["state"] == "completed"


def test_ack_task__updates_timestamp():
    sid, sender, recipient = _setup_two_agents()
    sent = broker.send_message(sid, sender, recipient, "Ack me")
    task_id = sent["task"]["id"]
    original_ts = sent["task"]["status"]["timestamp"]

    result = broker.ack_task(recipient, task_id)
    assert result["task"]["status"]["timestamp"] >= original_ts


def test_ack_task__verifies_context_id_matches_agent():
    """Only the recipient (context_id) can ACK."""
    sid, sender, recipient = _setup_two_agents()
    sent = broker.send_message(sid, sender, recipient, "Ack me")
    task_id = sent["task"]["id"]

    # Sender should not be able to ACK
    with pytest.raises(PermissionError):
        broker.ack_task(sender, task_id)


def test_ack_task__verifies_state_is_input_required():
    """Cannot ACK a task that is already completed."""
    sid, sender, recipient = _setup_two_agents()
    sent = broker.send_message(sid, sender, recipient, "Ack me")
    task_id = sent["task"]["id"]

    broker.ack_task(recipient, task_id)
    # Second ACK should fail
    with pytest.raises(ValueError, match="Cannot ACK"):
        broker.ack_task(recipient, task_id)


def test_ack_task__cannot_ack_canceled_task():
    """Cannot ACK a task that has been canceled."""
    sid, sender, recipient = _setup_two_agents()
    sent = broker.send_message(sid, sender, recipient, "Cancel me")
    task_id = sent["task"]["id"]

    broker.cancel_task(sender, task_id)
    with pytest.raises(ValueError, match="Cannot ACK"):
        broker.ack_task(recipient, task_id)


def test_ack_task__ack_persists_state():
    """After ACK, polling shows the task as completed."""
    sid, sender, recipient = _setup_two_agents()
    sent = broker.send_message(sid, sender, recipient, "Ack me")
    task_id = sent["task"]["id"]

    broker.ack_task(recipient, task_id)

    tasks = broker.poll_tasks(recipient)
    acked = [t for t in tasks if t["id"] == task_id]
    assert len(acked) == 1
    assert acked[0]["status"]["state"] == "completed"


# --- cancel_task: broker.cancel_task(agent_id, task_id) → {"task": <updated dict>} ---


def test_cancel_task__returns_dict_with_task_key():
    sid, sender, recipient = _setup_two_agents()
    sent = broker.send_message(sid, sender, recipient, "Cancel me")
    task_id = sent["task"]["id"]

    result = broker.cancel_task(sender, task_id)
    assert isinstance(result, dict)
    assert "task" in result


def test_cancel_task__transitions_to_canceled():
    sid, sender, recipient = _setup_two_agents()
    sent = broker.send_message(sid, sender, recipient, "Cancel me")
    task_id = sent["task"]["id"]

    result = broker.cancel_task(sender, task_id)
    assert result["task"]["status"]["state"] == "canceled"


def test_cancel_task__updates_timestamp():
    sid, sender, recipient = _setup_two_agents()
    sent = broker.send_message(sid, sender, recipient, "Cancel me")
    task_id = sent["task"]["id"]
    original_ts = sent["task"]["status"]["timestamp"]

    result = broker.cancel_task(sender, task_id)
    assert result["task"]["status"]["timestamp"] >= original_ts


def test_cancel_task__verifies_from_agent_id_matches():
    """Only the sender (metadata.fromAgentId) can cancel."""
    sid, sender, recipient = _setup_two_agents()
    sent = broker.send_message(sid, sender, recipient, "Cancel me")
    task_id = sent["task"]["id"]

    # Recipient should not be able to cancel
    with pytest.raises(PermissionError):
        broker.cancel_task(recipient, task_id)


def test_cancel_task__verifies_state_is_input_required():
    """Cannot cancel a task that is already completed."""
    sid, sender, recipient = _setup_two_agents()
    sent = broker.send_message(sid, sender, recipient, "Ack then cancel")
    task_id = sent["task"]["id"]

    broker.ack_task(recipient, task_id)
    with pytest.raises(ValueError, match="Cannot cancel"):
        broker.cancel_task(sender, task_id)


def test_cancel_task__cannot_cancel_already_canceled():
    """Cannot cancel a task that is already canceled."""
    sid, sender, recipient = _setup_two_agents()
    sent = broker.send_message(sid, sender, recipient, "Cancel me twice")
    task_id = sent["task"]["id"]

    broker.cancel_task(sender, task_id)
    with pytest.raises(ValueError, match="Cannot cancel"):
        broker.cancel_task(sender, task_id)


def test_cancel_task__cancel_persists_state():
    """After cancel, polling shows the task as canceled."""
    sid, sender, recipient = _setup_two_agents()
    sent = broker.send_message(sid, sender, recipient, "Cancel me")
    task_id = sent["task"]["id"]

    broker.cancel_task(sender, task_id)

    tasks = broker.poll_tasks(recipient)
    canceled = [t for t in tasks if t["id"] == task_id]
    assert len(canceled) == 1
    assert canceled[0]["status"]["state"] == "canceled"


# --- get_task: broker.get_task(session_id, task_id) → {"task": <dict>} ---


def test_get_task__returns_dict_with_task_key():
    sid, sender, recipient = _setup_two_agents()
    sent = broker.send_message(sid, sender, recipient, "Get me")
    task_id = sent["task"]["id"]

    result = broker.get_task(sid, task_id)
    assert isinstance(result, dict)
    assert "task" in result


def test_get_task__returns_correct_task():
    sid, sender, recipient = _setup_two_agents()
    sent = broker.send_message(sid, sender, recipient, "Specific task")
    task_id = sent["task"]["id"]

    result = broker.get_task(sid, task_id)
    assert result["task"]["id"] == task_id


def test_get_task__task_has_full_structure():
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


def test_get_task__verifies_agent_belongs_to_session():
    """get_task verifies fromAgentId or toAgentId belongs to the given session."""
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
    with pytest.raises(ValueError, match="not found"):
        broker.get_task(sid_b, task_id)


def test_get_task__nonexistent_task_raises():
    session = _create_session()
    with pytest.raises(ValueError, match="not found"):
        broker.get_task(session["session_id"], str(uuid.uuid4()))


def test_get_task__sender_can_get_task():
    """Sender (fromAgentId) can retrieve the task."""
    sid, sender, recipient = _setup_two_agents()
    sent = broker.send_message(sid, sender, recipient, "sender gets")
    task_id = sent["task"]["id"]

    result = broker.get_task(sid, task_id)
    assert result["task"]["id"] == task_id


def test_get_task__recipient_can_get_task():
    """Recipient (toAgentId/contextId) can retrieve the task."""
    sid, sender, recipient = _setup_two_agents()
    sent = broker.send_message(sid, sender, recipient, "recipient gets")
    task_id = sent["task"]["id"]

    result = broker.get_task(sid, task_id)
    assert result["task"]["id"] == task_id
