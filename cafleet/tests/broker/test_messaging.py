"""Tests for ``broker`` messaging operations."""

import pytest

from cafleet import broker
from tests.broker._helpers import (
    _create_fleet,
    _register_member,
    _setup_three_members,
    _setup_two_members,
)

# --- send_message --------------------------------------------------------


def test_send_message__returns_typed_envelope_with_message_and_notification():
    sid, sender, recipient = _setup_two_members()
    result = broker.send_message(sid, sender, recipient, "Did the API change?")
    message = result["message"]
    assert set(message.keys()) == {
        "message_id",
        "owner_member_id",
        "from_member_id",
        "to_member_id",
        "type",
        "created_at",
        "status_state",
        "status_timestamp",
        "origin_message_id",
        "text",
    }
    assert message["type"] == "unicast"
    assert message["from_member_id"] == sender
    assert message["to_member_id"] == recipient
    assert message["owner_member_id"] == recipient
    assert message["text"] == "Did the API change?"
    assert message["status_state"] == "input_required"
    assert isinstance(message["message_id"], int)
    assert "T" in message["status_timestamp"]
    assert "notification_sent" in result


@pytest.mark.parametrize(
    ("scenario", "build_args", "expected_match"),
    [
        ("non_integer", "invalid_dest", "Invalid destination format"),
        ("missing_member", "missing_dest", "Destination member not found"),
        ("deregistered_member", "deregistered_dest", "Destination member not found"),
        ("cross_fleet", "cross_fleet", "Destination member not in fleet"),
    ],
)
def test_send_message__validation_failures(scenario, build_args, expected_match):
    sid, sender, recipient = _setup_two_members()
    if build_args == "invalid_dest":
        dest_sid, dest_sender, dest = sid, sender, "not-a-number"
    elif build_args == "missing_dest":
        dest_sid, dest_sender, dest = sid, sender, 999999
    elif build_args == "deregistered_dest":
        broker.deregister_member(recipient)
        dest_sid, dest_sender, dest = sid, sender, recipient
    else:  # cross_fleet
        other = _create_fleet()
        other_recipient = _register_member(other["fleet_id"], name="outsider")
        dest_sid, dest_sender, dest = sid, sender, other_recipient["member_id"]
    with pytest.raises(ValueError, match=expected_match):
        broker.send_message(dest_sid, dest_sender, dest, "Hello")


def test_send_message__message_persisted_to_db():
    sid, sender, recipient = _setup_two_members()
    broker.send_message(sid, sender, recipient, "persisted?")
    messages = broker.poll_messages(recipient)
    assert {m["text"] for m in messages} == {"persisted?"}


# --- broadcast_message ---------------------------------------------------


def test_broadcast_message__summary_envelope_shape():
    sid, sender, _b, _c = _setup_three_members()
    [result] = broker.broadcast_message(sid, sender, "Attention all")
    summary = result["message"]
    assert summary["type"] == "broadcast_summary"
    assert summary["owner_member_id"] == sender
    assert summary["from_member_id"] == sender


def test_broadcast_message__delivery_shape_and_origin_link():
    sid, sender, b_id, _c = _setup_three_members()
    [result] = broker.broadcast_message(sid, sender, "Important update")
    summary_id = result["message"]["message_id"]
    [delivered] = broker.poll_messages(b_id)
    assert delivered["type"] == "unicast"
    assert delivered["origin_message_id"] == summary_id
    assert delivered["text"] == "Important update"
    # Sender excluded from own delivery list.
    sender_unicasts = [
        m for m in broker.poll_messages(sender) if m["type"] == "unicast"
    ]
    assert sender_unicasts == []


@pytest.mark.parametrize(
    ("scenario", "expected_text"),
    [
        ("bootstrap_only_director_reaches_none", "Broadcast sent to 0 recipients"),
        ("member_broadcast_reaches_all_but_sender", "Broadcast sent to 3 recipients"),
        ("director_broadcast_reaches_members", "Broadcast sent to 2 recipients"),
    ],
)
def test_broadcast_message__recipient_selection_matrix(scenario, expected_text):
    # A broadcast reaches every active member of the fleet except the sender.
    fleet = _create_fleet()
    sid = fleet["fleet_id"]
    director_id = fleet["director"]["member_id"]

    if scenario == "bootstrap_only_director_reaches_none":
        result = broker.broadcast_message(sid, director_id, "anybody?")
        assert result[0]["message"]["type"] == "broadcast_summary"
        assert result[0]["message"]["text"] == expected_text
    elif scenario == "member_broadcast_reaches_all_but_sender":
        sender = _register_member(sid, name="sender")
        user_a = _register_member(sid, name="user-a")
        user_b = _register_member(sid, name="user-b")
        result = broker.broadcast_message(sid, sender["member_id"], "hey")
        assert result[0]["message"]["text"] == expected_text
        sender_unicasts = [
            m
            for m in broker.poll_messages(sender["member_id"])
            if m["type"] == "unicast"
        ]
        assert sender_unicasts == []
        assert len(broker.poll_messages(user_a["member_id"])) == 1
        assert len(broker.poll_messages(user_b["member_id"])) == 1
        assert len(broker.poll_messages(director_id)) == 1
    else:  # director_broadcast_reaches_members
        user_a = _register_member(sid, name="user-a")
        user_b = _register_member(sid, name="user-b")
        result = broker.broadcast_message(sid, director_id, "hello team")
        assert result[0]["message"]["text"] == expected_text
        assert len(broker.poll_messages(user_a["member_id"])) == 1
        assert len(broker.poll_messages(user_b["member_id"])) == 1


# --- poll_messages -------------------------------------------------------


def test_poll_messages__empty_and_non_empty_shape():
    fleet = _create_fleet()
    idle = _register_member(fleet["fleet_id"], name="idle")
    assert broker.poll_messages(idle["member_id"]) == []

    sid, sender, recipient = _setup_two_members()
    broker.send_message(sid, sender, recipient, "Hello")
    [message] = broker.poll_messages(recipient)
    for key in (
        "message_id",
        "owner_member_id",
        "status_state",
        "from_member_id",
        "type",
        "text",
    ):
        assert key in message


def test_poll_messages__returns_only_unacked_deliveries():
    """``poll_messages`` returns only un-acked (``input_required``) deliveries;
    acked / completed messages are excluded."""
    sid, sender, recipient = _setup_two_members()
    sent_first = broker.send_message(sid, sender, recipient, "first")
    broker.send_message(sid, sender, recipient, "second")
    broker.send_message(sid, sender, recipient, "third")
    broker.ack_message(recipient, sent_first["message"]["message_id"])

    rows = broker.poll_messages(recipient)
    assert {m["text"] for m in rows} == {"second", "third"}
    assert all(m["status_state"] == "input_required" for m in rows)


def test_poll_messages__broadcast_summary_excluded():
    sid2, sender2, _b, _c = _setup_three_members()
    broker.broadcast_message(sid2, sender2, "broadcast")
    sender_messages = broker.poll_messages(sender2)
    assert "broadcast_summary" not in [m["type"] for m in sender_messages]


def test_poll_messages__ordering_recent_first():
    sid, sender, recipient = _setup_two_members()
    broker.send_message(sid, sender, recipient, "first")
    broker.send_message(sid, sender, recipient, "second")
    rows = broker.poll_messages(recipient)
    assert len(rows) == 2
    assert rows[0]["status_timestamp"] >= rows[1]["status_timestamp"]


def test_poll_messages__only_returns_messages_for_specified_member():
    sid, sender, recipient = _setup_two_members()
    broker.send_message(sid, sender, recipient, "for-recipient")
    assert broker.poll_messages(sender) == []


# --- ack_message ----------------------------------------------------------


def test_ack__state_transition_and_round_trip():
    sid, sender, recipient = _setup_two_members()
    sent = broker.send_message(sid, sender, recipient, "round trip ack")
    mid = sent["message"]["message_id"]
    result = broker.ack_message(recipient, mid)
    assert result["message"]["status_state"] == "completed"
    # Persist + round-trip via get_message (poll now returns only un-acked).
    persisted = broker.get_message(sid, mid)["message"]
    assert persisted["status_state"] == "completed"
    assert persisted["text"] == "round trip ack"


def test_ack__updates_timestamp():
    sid, sender, recipient = _setup_two_members()
    sent = broker.send_message(sid, sender, recipient, "body")
    original_ts = sent["message"]["status_timestamp"]
    result = broker.ack_message(recipient, sent["message"]["message_id"])
    assert result["message"]["status_timestamp"] >= original_ts


def test_ack__recipient_only_authorization_boundary():
    sid, sender, recipient = _setup_two_members()
    sent = broker.send_message(sid, sender, recipient, "body")
    mid = sent["message"]["message_id"]
    with pytest.raises(PermissionError, match="Only the recipient can ACK a message"):
        broker.ack_message(sender, mid)


def test_ack__missing_message_raises_not_found():
    _sid, _sender, recipient = _setup_two_members()
    with pytest.raises(ValueError, match="Message 999999 not found"):
        broker.ack_message(recipient, 999999)


def test_ack__double_ack_rejected():
    sid, sender, recipient = _setup_two_members()
    sent = broker.send_message(sid, sender, recipient, "body")
    mid = sent["message"]["message_id"]
    broker.ack_message(recipient, mid)
    with pytest.raises(ValueError, match="Cannot ACK message in state completed"):
        broker.ack_message(recipient, mid)


# --- get_message ----------------------------------------------------------


def test_get_message__returns_full_typed_envelope():
    sid, sender, recipient = _setup_two_members()
    sent = broker.send_message(sid, sender, recipient, "Full structure")
    mid = sent["message"]["message_id"]
    result = broker.get_message(sid, mid)
    message = result["message"]
    assert message["message_id"] == mid
    for key in (
        "message_id",
        "owner_member_id",
        "status_state",
        "from_member_id",
        "type",
        "text",
    ):
        assert key in message


def test_get_message__nonexistent_raises():
    fleet = _create_fleet()
    with pytest.raises(ValueError, match="not found"):
        broker.get_message(fleet["fleet_id"], 999999)


def test_get_message__fleet_boundary_rejects_foreign_fleet():
    fleet_a = _create_fleet()
    fleet_b = _create_fleet()
    sid_a = fleet_a["fleet_id"]
    sid_b = fleet_b["fleet_id"]
    sender = _register_member(sid_a, name="sender")
    recipient = _register_member(sid_a, name="recipient")
    sent = broker.send_message(sid_a, sender["member_id"], recipient["member_id"], "hi")
    mid = sent["message"]["message_id"]
    assert broker.get_message(sid_a, mid)["message"]["message_id"] == mid
    with pytest.raises(ValueError, match="not found"):
        broker.get_message(sid_b, mid)


@pytest.mark.parametrize("actor_role", ["sender", "recipient"])
def test_get_message__sender_or_recipient_can_read(actor_role):
    sid, sender, recipient = _setup_two_members()
    sent = broker.send_message(sid, sender, recipient, f"read by {actor_role}")
    mid = sent["message"]["message_id"]
    # get_message is fleet-scoped (doesn't require the actor member_id);
    # parametrize on the actor role exercises the symmetry of read access.
    assert broker.get_message(sid, mid)["message"]["message_id"] == mid
