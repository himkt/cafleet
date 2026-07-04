"""Regression tests for split broadcast counts (design 0000118, item 2.1).

The broadcast result reports two distinct counts: ``recipients`` — the real
number of active non-admin peers a delivery row was fanned out to (N) — and
``delivered`` — how many inline-preview keystrokes actually landed (k, a subset
of N). The legacy single ``notifications_sent_count`` key is gone.

Recipients registered without a placement have no pane, so ``delivered`` is 0
while ``recipients`` counts every peer — a scenario where N and k diverge.
"""

import pytest

from cafleet import broker
from tests.broker._helpers import _create_fleet, _register_agent


@pytest.fixture(autouse=True)
def _autouse_broker(broker_session):
    return broker_session


def test_broadcast_result_carries_separate_recipients_and_delivered():
    sid = _create_fleet()["fleet_id"]
    sender = _register_agent(sid, "sender")
    _register_agent(sid, "recipient-a")
    _register_agent(sid, "recipient-b")

    [result] = broker.broadcast_message(sid, sender["agent_id"], "hi all")

    assert result["recipients"] == 2
    assert result["delivered"] == 0
    assert isinstance(result["recipients"], int)
    assert isinstance(result["delivered"], int)


def test_broadcast_result_drops_legacy_notifications_sent_count_key():
    sid = _create_fleet()["fleet_id"]
    sender = _register_agent(sid, "sender")
    _register_agent(sid, "recipient-a")

    [result] = broker.broadcast_message(sid, sender["agent_id"], "hi all")

    assert "notifications_sent_count" not in result
