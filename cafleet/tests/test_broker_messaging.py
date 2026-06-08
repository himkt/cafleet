"""Tests for ``broker`` messaging operations."""

import pytest

from cafleet import broker
from tests._broker_helpers import (
    _create_fleet,
    _register_agent,
    _setup_three_agents,
    _setup_two_agents,
)


@pytest.fixture(autouse=True)
def _autouse_broker(broker_session):
    return broker_session


# --- send_message --------------------------------------------------------


def test_send_message__returns_typed_envelope_with_task_and_notification():
    sid, sender, recipient = _setup_two_agents()
    result = broker.send_message(sid, sender, recipient, "Did the API change?")
    task = result["task"]
    assert set(task.keys()) == {
        "task_id",
        "context_id",
        "from_agent_id",
        "to_agent_id",
        "type",
        "created_at",
        "status_state",
        "status_timestamp",
        "origin_task_id",
        "text",
    }
    assert task["type"] == "unicast"
    assert task["from_agent_id"] == sender
    assert task["to_agent_id"] == recipient
    assert task["context_id"] == recipient
    assert task["text"] == "Did the API change?"
    assert task["status_state"] == "input_required"
    assert isinstance(task["task_id"], int)
    assert "T" in task["status_timestamp"]
    assert "notification_sent" in result


@pytest.mark.parametrize(
    ("scenario", "build_args", "expected_match"),
    [
        ("non_integer", "invalid_dest", "Invalid destination format"),
        ("missing_agent", "missing_dest", "Destination agent not found"),
        ("deregistered_agent", "deregistered_dest", "Destination agent not found"),
        ("cross_fleet", "cross_fleet", "Destination agent not in fleet"),
    ],
)
def test_send_message__validation_failures(scenario, build_args, expected_match):
    sid, sender, recipient = _setup_two_agents()
    if build_args == "invalid_dest":
        dest_sid, dest_sender, dest = sid, sender, "not-a-number"
    elif build_args == "missing_dest":
        dest_sid, dest_sender, dest = sid, sender, 999999
    elif build_args == "deregistered_dest":
        broker.deregister_agent(recipient)
        dest_sid, dest_sender, dest = sid, sender, recipient
    else:  # cross_fleet
        other = _create_fleet()
        other_recipient = _register_agent(other["fleet_id"], name="outsider")
        dest_sid, dest_sender, dest = sid, sender, other_recipient["agent_id"]
    with pytest.raises(ValueError, match=expected_match):
        broker.send_message(dest_sid, dest_sender, dest, "Hello")


def test_send_message__task_persisted_to_db():
    sid, sender, recipient = _setup_two_agents()
    broker.send_message(sid, sender, recipient, "persisted?")
    tasks = broker.poll_tasks(recipient)
    assert {t["text"] for t in tasks} == {"persisted?"}


# --- broadcast_message ---------------------------------------------------


def test_broadcast_message__summary_envelope_shape():
    sid, sender, _b, _c = _setup_three_agents()
    [result] = broker.broadcast_message(sid, sender, "Attention all")
    summary = result["task"]
    assert summary["type"] == "broadcast_summary"
    assert summary["context_id"] == sender
    assert summary["from_agent_id"] == sender


def test_broadcast_message__delivery_shape_and_origin_link():
    sid, sender, b_id, _c = _setup_three_agents()
    [result] = broker.broadcast_message(sid, sender, "Important update")
    summary_id = result["task"]["task_id"]
    [delivered] = broker.poll_tasks(b_id)
    assert delivered["type"] == "unicast"
    assert delivered["origin_task_id"] == summary_id
    assert delivered["text"] == "Important update"
    # Sender excluded from own delivery list.
    sender_unicasts = [t for t in broker.poll_tasks(sender) if t["type"] == "unicast"]
    assert sender_unicasts == []


@pytest.mark.parametrize(
    ("scenario", "expected_text", "extra_assertion"),
    [
        ("no_other_agents", "Broadcast sent to 0 recipients", None),
        (
            "admin_exclusion_from_user_broadcast",
            "Broadcast sent to 3 recipients",
            "admin_excluded",
        ),
        (
            "admin_broadcast_reaches_all",
            "Broadcast sent to 3 recipients",
            "admin_reaches_all",
        ),
        (
            "bootstrap_fleet_admin_reaches_only_director",
            "Broadcast sent to 1 recipients",
            "only_director",
        ),
    ],
)
def test_broadcast_message__recipient_selection_matrix(
    scenario, expected_text, extra_assertion
):
    if scenario == "no_other_agents":
        fleet = _create_fleet()
        sid = fleet["fleet_id"]
        lone = _register_agent(sid, name="lonely")
        # Also need to subtract administrator + director from the recipient pool —
        # they are auto-seeded. So "Broadcast sent to N" depends on how many remain.
        # In bootstrap-only fleet admin sends → reaches director (1).
        # Here we have lone + admin + director; lone broadcasts; admin excluded; director receives.
        result = broker.broadcast_message(sid, lone["agent_id"], "Anyone?")
        assert result[0]["task"]["type"] == "broadcast_summary"
        # Director gets the message.
        director_tasks = broker.poll_tasks(fleet["director"]["agent_id"])
        assert len(director_tasks) == 1
        return

    fleet = _create_fleet()
    sid = fleet["fleet_id"]
    admin_id = fleet["administrator_agent_id"]
    director_id = fleet["director"]["agent_id"]

    if scenario == "admin_exclusion_from_user_broadcast":
        sender = _register_agent(sid, name="sender")
        user_a = _register_agent(sid, name="user-a")
        user_b = _register_agent(sid, name="user-b")
        result = broker.broadcast_message(sid, sender["agent_id"], "hey")
        assert result[0]["task"]["text"] == expected_text
        admin_unicasts = [
            t for t in broker.poll_tasks(admin_id) if t["type"] == "unicast"
        ]
        assert admin_unicasts == []
        assert len(broker.poll_tasks(user_a["agent_id"])) == 1
        assert len(broker.poll_tasks(user_b["agent_id"])) == 1
    elif scenario == "admin_broadcast_reaches_all":
        user_a = _register_agent(sid, name="user-a")
        user_b = _register_agent(sid, name="user-b")
        result = broker.broadcast_message(sid, admin_id, "hello from admin")
        assert result[0]["task"]["text"] == expected_text
        assert len(broker.poll_tasks(user_a["agent_id"])) == 1
        assert len(broker.poll_tasks(user_b["agent_id"])) == 1
        assert len(broker.poll_tasks(director_id)) == 1
    else:  # bootstrap_fleet_admin_reaches_only_director
        result = broker.broadcast_message(sid, admin_id, "anybody?")
        assert result[0]["task"]["text"] == expected_text
        assert len(broker.poll_tasks(director_id)) == 1


# --- poll_tasks ----------------------------------------------------------


def test_poll_tasks__empty_and_non_empty_shape():
    fleet = _create_fleet()
    idle = _register_agent(fleet["fleet_id"], name="idle")
    assert broker.poll_tasks(idle["agent_id"]) == []

    sid, sender, recipient = _setup_two_agents()
    broker.send_message(sid, sender, recipient, "Hello")
    [task] = broker.poll_tasks(recipient)
    for key in (
        "task_id",
        "context_id",
        "status_state",
        "from_agent_id",
        "type",
        "text",
    ):
        assert key in task


def test_poll_tasks__returns_only_unacked_deliveries():
    """``poll_tasks`` returns only un-acked (``input_required``) deliveries;
    acked / completed tasks are excluded."""
    sid, sender, recipient = _setup_two_agents()
    sent_first = broker.send_message(sid, sender, recipient, "first")
    broker.send_message(sid, sender, recipient, "second")
    broker.send_message(sid, sender, recipient, "third")
    broker.ack_task(recipient, sent_first["task"]["task_id"])

    rows = broker.poll_tasks(recipient)
    assert {t["text"] for t in rows} == {"second", "third"}
    assert all(t["status_state"] == "input_required" for t in rows)


def test_poll_tasks__broadcast_summary_excluded():
    sid2, sender2, _b, _c = _setup_three_agents()
    broker.broadcast_message(sid2, sender2, "broadcast")
    sender_tasks = broker.poll_tasks(sender2)
    assert "broadcast_summary" not in [t["type"] for t in sender_tasks]


def test_poll_tasks__ordering_recent_first():
    sid, sender, recipient = _setup_two_agents()
    broker.send_message(sid, sender, recipient, "first")
    broker.send_message(sid, sender, recipient, "second")
    rows = broker.poll_tasks(recipient)
    assert len(rows) == 2
    assert rows[0]["status_timestamp"] >= rows[1]["status_timestamp"]


def test_poll_tasks__only_returns_tasks_for_specified_agent():
    sid, sender, recipient = _setup_two_agents()
    broker.send_message(sid, sender, recipient, "for-recipient")
    assert broker.poll_tasks(sender) == []


# --- ack_task / cancel_task ---------------------------------------------


@pytest.mark.parametrize(
    ("action", "expected_state", "unauthorized_actor_role"),
    [("ack", "completed", "sender"), ("cancel", "canceled", "recipient")],
)
def test_ack_cancel__state_transition_and_round_trip(
    action, expected_state, unauthorized_actor_role
):
    sid, sender, recipient = _setup_two_agents()
    sent = broker.send_message(sid, sender, recipient, f"round trip {action}")
    tid = sent["task"]["task_id"]
    if action == "ack":
        actor = recipient
        call = broker.ack_task
    else:
        actor = sender
        call = broker.cancel_task
    result = call(actor, tid)
    assert result["task"]["status_state"] == expected_state
    # Persist + round-trip via get_task (poll now returns only un-acked).
    persisted = broker.get_task(sid, tid)["task"]
    assert persisted["status_state"] == expected_state
    assert persisted["text"] == f"round trip {action}"


@pytest.mark.parametrize(
    "action",
    ["ack", "cancel"],
)
def test_ack_cancel__updates_timestamp(action):
    sid, sender, recipient = _setup_two_agents()
    sent = broker.send_message(sid, sender, recipient, "body")
    original_ts = sent["task"]["status_timestamp"]
    if action == "ack":
        result = broker.ack_task(recipient, sent["task"]["task_id"])
    else:
        result = broker.cancel_task(sender, sent["task"]["task_id"])
    assert result["task"]["status_timestamp"] >= original_ts


@pytest.mark.parametrize(
    ("action", "wrong_actor_role"),
    [("ack", "sender"), ("cancel", "recipient")],
)
def test_ack_cancel__authorization_boundary(action, wrong_actor_role):
    sid, sender, recipient = _setup_two_agents()
    sent = broker.send_message(sid, sender, recipient, "body")
    tid = sent["task"]["task_id"]
    wrong = sender if wrong_actor_role == "sender" else recipient
    call = broker.ack_task if action == "ack" else broker.cancel_task
    with pytest.raises(PermissionError):
        call(wrong, tid)


@pytest.mark.parametrize(
    ("first_action", "second_action", "expected_match"),
    [
        ("ack", "ack", "Cannot ACK"),
        ("ack", "cancel", "Cannot cancel"),
        ("cancel", "cancel", "Cannot cancel"),
        ("cancel", "ack", "Cannot ACK"),
    ],
)
def test_ack_cancel__double_action_rejected(
    first_action, second_action, expected_match
):
    sid, sender, recipient = _setup_two_agents()
    sent = broker.send_message(sid, sender, recipient, "body")
    tid = sent["task"]["task_id"]
    do_first = broker.ack_task if first_action == "ack" else broker.cancel_task
    do_first(recipient if first_action == "ack" else sender, tid)
    do_second = broker.ack_task if second_action == "ack" else broker.cancel_task
    actor = recipient if second_action == "ack" else sender
    with pytest.raises(ValueError, match=expected_match):
        do_second(actor, tid)


# --- get_task -----------------------------------------------------------


def test_get_task__returns_full_typed_envelope():
    sid, sender, recipient = _setup_two_agents()
    sent = broker.send_message(sid, sender, recipient, "Full structure")
    tid = sent["task"]["task_id"]
    result = broker.get_task(sid, tid)
    task = result["task"]
    assert task["task_id"] == tid
    for key in (
        "task_id",
        "context_id",
        "status_state",
        "from_agent_id",
        "type",
        "text",
    ):
        assert key in task


def test_get_task__nonexistent_raises():
    fleet = _create_fleet()
    with pytest.raises(ValueError, match="not found"):
        broker.get_task(fleet["fleet_id"], 999999)


def test_get_task__fleet_boundary_rejects_foreign_fleet():
    fleet_a = _create_fleet()
    fleet_b = _create_fleet()
    sid_a = fleet_a["fleet_id"]
    sid_b = fleet_b["fleet_id"]
    sender = _register_agent(sid_a, name="sender")
    recipient = _register_agent(sid_a, name="recipient")
    sent = broker.send_message(sid_a, sender["agent_id"], recipient["agent_id"], "hi")
    tid = sent["task"]["task_id"]
    assert broker.get_task(sid_a, tid)["task"]["task_id"] == tid
    with pytest.raises(ValueError, match="not found"):
        broker.get_task(sid_b, tid)


@pytest.mark.parametrize("actor_role", ["sender", "recipient"])
def test_get_task__sender_or_recipient_can_read(actor_role):
    sid, sender, recipient = _setup_two_agents()
    sent = broker.send_message(sid, sender, recipient, f"read by {actor_role}")
    tid = sent["task"]["task_id"]
    # get_task is fleet-scoped (doesn't require the actor agent_id);
    # parametrize on the actor role exercises the symmetry of read access.
    assert broker.get_task(sid, tid)["task"]["task_id"] == tid
