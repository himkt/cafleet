"""Typed-column broker shape.

Every public broker caller returns a flat dict whose keys map 1:1 onto the
typed ``Message`` columns plus the ``text`` body column — no ``metadata``
wrapping, no ``artifacts``/``parts``/``kind``/``history``, no leftover
``task_json`` field on result rows.
"""

import pytest
from sqlalchemy import create_engine, text

from cafleet import broker
from cafleet.broker import _shared, messaging
from cafleet.db.models import Base, Message
from tests.broker._helpers import (
    _create_fleet,
    _register_member,
    _setup_three_members,
    _setup_two_members,
)

REQUIRED_MESSAGE_KEYS = {
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


def _assert_flat_typed_shape(d: dict, *, expect_type: str | None = None) -> None:
    assert set(d.keys()) == REQUIRED_MESSAGE_KEYS, (
        f"unexpected typed-column key set: {sorted(d.keys())}"
    )
    if expect_type is not None:
        assert d["type"] == expect_type


def test_send_message__unicast_returns_flat_typed_envelope():
    sid, sender, recipient = _setup_two_members()
    result = broker.send_message(sid, sender, recipient, "Did the API change?")
    message = result["message"]

    assert set(message.keys()) == REQUIRED_MESSAGE_KEYS
    assert message["type"] == "unicast"
    assert message["from_member_id"] == sender
    assert message["to_member_id"] == recipient
    assert message["owner_member_id"] == recipient
    assert message["text"] == "Did the API change?"
    assert message["status_state"] == "input_required"
    assert message["origin_message_id"] is None
    assert isinstance(message["message_id"], int)
    assert "notification_sent" in result


@pytest.mark.parametrize(
    ("origin_in", "origin_out"),
    [(None, None), (777, 777)],
)
def test_send_message__origin_message_id_default_and_propagate(origin_in, origin_out):
    now = "2026-05-05T12:00:00.000000+00:00"
    kwargs = {"origin_message_id": origin_in} if origin_in is not None else {}
    result = messaging._unicast_message_dict(
        recipient_id=2,
        sender_id=1,
        text="Hello",
        now=now,
        **kwargs,
    )
    assert result["origin_message_id"] == origin_out


def test_broadcast_message__summary_envelope_shape():
    sid, sender, _b, _c = _setup_three_members()
    [result] = broker.broadcast_message(sid, sender, "Attention all")
    summary = result["message"]

    assert set(summary.keys()) == REQUIRED_MESSAGE_KEYS
    assert summary["type"] == "broadcast_summary"
    assert summary["from_member_id"] == sender
    assert summary["owner_member_id"] == sender
    assert summary["to_member_id"] is None
    assert summary["status_state"] == "completed"
    assert "Broadcast sent" in summary["text"]
    assert isinstance(result["recipients"], int)
    assert isinstance(result["delivered"], int)


def test_broadcast_message__delivery_message_shape_and_origin_link():
    sid, sender, b_id, _c = _setup_three_members()
    [result] = broker.broadcast_message(sid, sender, "delivery body")
    summary_id = result["message"]["message_id"]

    [delivered] = broker.poll_messages(b_id)
    _assert_flat_typed_shape(delivered, expect_type="unicast")
    assert delivered["text"] == "delivery body"
    assert delivered["origin_message_id"] == summary_id


def test_poll_messages__returns_flat_typed_message_dicts():
    sid, sender, recipient = _setup_two_members()
    broker.send_message(sid, sender, recipient, "first")
    broker.send_message(sid, sender, recipient, "second")

    rows = broker.poll_messages(recipient)
    assert len(rows) == 2
    for row in rows:
        _assert_flat_typed_shape(row, expect_type="unicast")
    assert {r["text"] for r in rows} == {"first", "second"}

    sid2, sender2, _b, _c = _setup_three_members()
    broker.broadcast_message(sid2, sender2, "broadcast body")
    sender_messages = broker.poll_messages(sender2)
    assert "broadcast_summary" not in [m["type"] for m in sender_messages]


@pytest.mark.parametrize(
    ("action", "expected_state"),
    [("ack", "completed"), ("cancel", "canceled")],
)
def test_ack_and_cancel__transition_and_round_trip(action, expected_state):
    sid, sender, recipient = _setup_two_members()
    sent = broker.send_message(sid, sender, recipient, f"round trip {action}")
    mid = sent["message"]["message_id"]

    if action == "ack":
        result = broker.ack_message(recipient, mid)
        unauthorized_actor = sender
        unauthorized_call = broker.ack_message
    else:
        result = broker.cancel_message(sender, mid)
        unauthorized_actor = recipient
        unauthorized_call = broker.cancel_message

    _assert_flat_typed_shape(result["message"], expect_type="unicast")
    assert result["message"]["status_state"] == expected_state

    # poll now returns only un-acked deliveries; verify persistence via get_message.
    persisted = broker.get_message(sid, mid)["message"]
    assert persisted["text"] == f"round trip {action}"
    assert persisted["status_state"] == expected_state

    sent2 = broker.send_message(sid, sender, recipient, "unauthorized check")
    with pytest.raises(PermissionError):
        unauthorized_call(unauthorized_actor, sent2["message"]["message_id"])


@pytest.mark.parametrize("api", ["list_inbox", "list_sent", "list_timeline"])
def test_list_apis__no_metadata_wrapping_and_filter_broadcast_summary(api):
    sid, sender, _b, _c = _setup_three_members()
    broker.send_message(sid, sender, _b, "direct body")
    broker.broadcast_message(sid, sender, "broadcast body")

    if api == "list_inbox":
        rows = broker.list_inbox(_b)
        messages = rows
    elif api == "list_sent":
        rows = broker.list_sent(sender)
        messages = rows
    else:
        rows = broker.list_timeline(sid)
        messages = [
            r.get("message") if isinstance(r.get("message"), dict) else r for r in rows
        ]

    assert len(messages) >= 1
    for message in messages:
        for forbidden in ("metadata", "artifacts", "history", "task_json"):
            assert forbidden not in message

    if api == "list_sent":
        assert "broadcast_summary" not in [m["type"] for m in messages]
    if api == "list_timeline":
        assert "broadcast_summary" not in [
            m.get("type") for m in messages if "type" in m
        ]


def test_message_table_and_model__text_column_present_task_json_absent():
    engine = create_engine("sqlite:///:memory:")
    Base.metadata.create_all(engine)
    try:
        with engine.connect() as conn:
            rows = conn.execute(text("PRAGMA table_info(messages)")).fetchall()
        columns = {r[1] for r in rows}
    finally:
        engine.dispose()
    assert "text" in columns
    assert "task_json" not in columns

    assert hasattr(Message, "text")
    assert not hasattr(Message, "task_json")

    sid, sender, recipient = _setup_two_members()
    sent = broker.send_message(sid, sender, recipient, "via send_message body")
    sm = _shared.get_sync_sessionmaker()
    with sm() as s:
        row = s.execute(
            text("SELECT text FROM messages WHERE message_id = :mid"),
            {"mid": sent["message"]["message_id"]},
        ).fetchone()
    assert row is not None
    assert row[0] == "via send_message body"

    # Cross-fleet boundary check (subsumes the fleet-scope and missing-row cases
    # for get_message / read_message)
    other = _create_fleet()
    other_sender = _register_member(other["fleet_id"], name="outsider-sender")
    other_recipient = _register_member(other["fleet_id"], name="outsider-recipient")
    foreign_sent = broker.send_message(
        other["fleet_id"],
        other_sender["member_id"],
        other_recipient["member_id"],
        "elsewhere",
    )
    with pytest.raises(ValueError, match="not found"):
        broker.get_message(sid, foreign_sent["message"]["message_id"])
