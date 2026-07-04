"""Regression tests for nullable ``tasks.to_agent_id`` (design 0000118, items 1.1/1.2).

A broadcast-summary row is owned by the broadcaster and has no single
recipient, so its ``to_agent_id`` is NULL rather than the legacy ``0``
sentinel. The column is nullable, the returned summary dict carries ``None``,
and the persisted row reads back as ``None``. A real unicast still retains its
recipient id (the "shows on a real id" half of the surfacing contract).
"""

import pytest

from cafleet import broker
from cafleet.db.models import Task
from tests.broker._helpers import _create_fleet, _register_agent, _setup_two_agents


@pytest.fixture(autouse=True)
def _autouse_broker(broker_session):
    return broker_session


def test_task_model_to_agent_id_column_is_nullable():
    assert Task.__table__.c.to_agent_id.nullable is True


def test_broadcast_summary_returned_dict_has_null_to_agent_id():
    sid = _create_fleet()["fleet_id"]
    sender = _register_agent(sid, "sender")
    _register_agent(sid, "recipient-a")
    _register_agent(sid, "recipient-b")

    [result] = broker.broadcast_message(sid, sender["agent_id"], "hi all")

    assert result["task"]["to_agent_id"] is None


def test_broadcast_summary_persists_null_to_agent_id():
    sid = _create_fleet()["fleet_id"]
    sender = _register_agent(sid, "sender")
    _register_agent(sid, "recipient-a")

    [result] = broker.broadcast_message(sid, sender["agent_id"], "hi all")
    summary_task_id = result["task"]["task_id"]

    persisted = broker.get_task(sid, summary_task_id)["task"]
    assert persisted["to_agent_id"] is None


def test_unicast_persists_real_to_agent_id():
    sid, sender, recipient = _setup_two_agents()

    result = broker.send_message(sid, sender, recipient, "hi")
    task_id = result["task"]["task_id"]

    persisted = broker.get_task(sid, task_id)["task"]
    assert persisted["to_agent_id"] == recipient
