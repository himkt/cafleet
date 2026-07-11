"""Tests for the ``webui.api`` message formatter."""

import pytest

from cafleet import broker
from cafleet.webui import api
from cafleet.webui.api import _format_messages
from tests.broker._helpers import _create_fleet


@pytest.fixture(autouse=True)
def _autouse_broker(broker_session):
    return broker_session


_EXPECTED_KEYS = {
    "task_id",
    "from_member_id",
    "from_member_name",
    "to_member_id",
    "to_member_name",
    "type",
    "status",
    "created_at",
    "status_timestamp",
    "origin_task_id",
    "body",
}


def _two_members():
    fleet = _create_fleet()
    sid = fleet["fleet_id"]
    a = broker.register_member(fleet_id=sid, name="alpha", description="A")
    b = broker.register_member(fleet_id=sid, name="beta", description="B")
    return sid, a["member_id"], b["member_id"]


def _typed_column_row(**overrides) -> dict:
    base = {
        "task_id": 1,
        "context_id": 22,
        "from_member_id": 11,
        "to_member_id": 22,
        "type": "unicast",
        "status_state": "input_required",
        "created_at": "2026-04-30T01:00:00+00:00",
        "status_timestamp": "2026-04-30T02:00:00+00:00",
        "origin_task_id": None,
        "text": "hello world",
    }
    base.update(overrides)
    return base


def test_format_messages__empty_rows_skips_lookup(monkeypatch):
    calls = []

    def fake_get_member_names(ids):
        calls.append(list(ids))
        return {}

    monkeypatch.setattr(api.broker, "get_member_names", fake_get_member_names)
    assert _format_messages([]) == []
    assert calls == []


def test_format_messages__shape_field_mapping_and_batched_lookup(monkeypatch):
    rows = [
        _typed_column_row(
            task_id=2,
            from_member_id=101,
            to_member_id=102,
            status_state="completed",
            origin_task_id=999,
            text="timeline body",
        ),
        _typed_column_row(task_id=3, from_member_id=101, to_member_id=103),
    ]
    lookup_calls = []

    def fake_get_member_names(ids):
        lookup_calls.append(set(ids))
        return {101: "alpha", 102: "beta", 103: "gamma"}

    monkeypatch.setattr(api.broker, "get_member_names", fake_get_member_names)

    result = _format_messages(rows)
    # Batched: one lookup with the full id set.
    assert len(lookup_calls) == 1
    assert lookup_calls[0] == {101, 102, 103}
    # Shape contract.
    assert len(result) == 2
    for msg in result:
        assert set(msg.keys()) == _EXPECTED_KEYS
    # Field mapping.
    first = result[0]
    assert first["task_id"] == 2
    assert first["from_member_id"] == 101
    assert first["from_member_name"] == "alpha"
    assert first["to_member_id"] == 102
    assert first["to_member_name"] == "beta"
    assert first["type"] == "unicast"
    assert first["status"] == "completed"
    assert first["origin_task_id"] == 999
    assert first["body"] == "timeline body"
    assert result[1]["to_member_name"] == "gamma"


@pytest.mark.parametrize(
    ("source", "expected_body"),
    [("inbox", "snapshot body"), ("timeline", "timeline snapshot")],
)
def test_format_messages__end_to_end_against_real_broker(source, expected_body):
    sid, sender, recipient = _two_members()
    broker.send_message(sid, sender, recipient, expected_body)
    if source == "inbox":
        rows = broker.list_inbox(recipient)
    else:
        rows = broker.list_timeline(sid)
    result = _format_messages(rows)
    assert len(result) == 1
    msg = result[0]
    assert set(msg.keys()) == _EXPECTED_KEYS
    assert msg["from_member_id"] == sender
    assert msg["from_member_name"] == "alpha"
    assert msg["to_member_id"] == recipient
    assert msg["to_member_name"] == "beta"
    assert msg["type"] == "unicast"
    assert msg["status"] == "input_required"
    assert msg["body"] == expected_body
    assert isinstance(msg["task_id"], int)
    assert msg["task_id"]
    assert isinstance(msg["created_at"], str)
    assert msg["created_at"]
    assert isinstance(msg["status_timestamp"], str)
    assert msg["status_timestamp"]
