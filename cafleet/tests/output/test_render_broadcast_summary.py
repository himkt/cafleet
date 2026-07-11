"""Broadcast summary omits ``recipient_ids``."""

import pytest

from cafleet import broker
from tests.broker._helpers import _create_fleet, _register_member


@pytest.fixture(autouse=True)
def _autouse_broker(broker_session):
    return broker_session


def test_broadcast_summary__exactly_typed_column_keys_no_recipient_ids_no_metadata():
    s = _create_fleet()
    sid = s["fleet_id"]
    sender = _register_member(sid, "sender")
    _register_member(sid, "recipient-a")
    _register_member(sid, "recipient-b")

    [result] = broker.broadcast_message(sid, sender["member_id"], "hi all")
    summary = result["message"]

    expected_keys = {
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
    extra = set(summary.keys()) - expected_keys
    assert not extra, f"unexpected extras: {sorted(extra)}"


def test_broadcast_summary__wrapper_count_and_text_describes_recipient_count():
    s = _create_fleet()
    sid = s["fleet_id"]
    sender = _register_member(sid, "sender")
    _register_member(sid, "recipient-a")
    _register_member(sid, "recipient-b")

    [result] = broker.broadcast_message(sid, sender["member_id"], "hi all")
    assert isinstance(result["recipients"], int)
    assert isinstance(result["delivered"], int)
    summary_text = result["message"]["text"]
    assert summary_text.startswith("Broadcast sent to ")
    assert "recipients" in summary_text
