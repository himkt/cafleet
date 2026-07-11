"""Regression tests for nullable ``tasks.to_member_id`` (design 0000118, items 1.1/1.2).

A broadcast-summary row is owned by the broadcaster and has no single
recipient, so its ``to_member_id`` is NULL rather than the legacy ``0``
sentinel. The column is nullable, the returned summary dict carries ``None``,
and the persisted row reads back as ``None``. A real unicast still retains its
recipient id (the "shows on a real id" half of the surfacing contract).
"""

from cafleet import broker
from cafleet.db.models import Task
from tests.broker._helpers import _create_fleet, _register_member, _setup_two_members


def test_task_model_to_member_id_column_is_nullable():
    assert Task.__table__.c.to_member_id.nullable is True


def test_broadcast_summary_returned_dict_has_null_to_member_id():
    sid = _create_fleet()["fleet_id"]
    sender = _register_member(sid, "sender")
    _register_member(sid, "recipient-a")
    _register_member(sid, "recipient-b")

    [result] = broker.broadcast_message(sid, sender["member_id"], "hi all")

    assert result["task"]["to_member_id"] is None


def test_broadcast_summary_persists_null_to_member_id():
    sid = _create_fleet()["fleet_id"]
    sender = _register_member(sid, "sender")
    _register_member(sid, "recipient-a")

    [result] = broker.broadcast_message(sid, sender["member_id"], "hi all")
    summary_task_id = result["task"]["task_id"]

    persisted = broker.get_task(sid, summary_task_id)["task"]
    assert persisted["to_member_id"] is None


def test_unicast_persists_real_to_member_id():
    sid, sender, recipient = _setup_two_members()

    result = broker.send_message(sid, sender, recipient, "hi")
    task_id = result["task"]["task_id"]

    persisted = broker.get_task(sid, task_id)["task"]
    assert persisted["to_member_id"] == recipient
