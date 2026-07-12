"""Regression tests for split broadcast counts.

The broadcast result reports two distinct counts: ``recipients`` — the real
number of active non-admin peers a delivery row was fanned out to (N) — and
``delivered`` — how many inline-preview keystrokes actually landed (k, a subset
of N).

``_create_fleet`` + sender + recipient-a + recipient-b gives three peers of the
sender (the pane-bound root Director and the two registered recipients), so
N=3. Only the root Director has a placement, so its preview is the sole one
that lands (k=1) while the pane-less recipients receive none — a scenario where
N and k diverge.
"""

from cafleet import broker
from tests.broker._helpers import _create_fleet, _register_member


def test_broadcast_result_carries_separate_recipients_and_delivered():
    sid = _create_fleet()["fleet_id"]
    sender = _register_member(sid, "sender")
    _register_member(sid, "recipient-a")
    _register_member(sid, "recipient-b")

    [result] = broker.broadcast_message(sid, sender["member_id"], "hi all")

    assert result["recipients"] == 3
    assert result["delivered"] == 1
    assert isinstance(result["recipients"], int)
    assert isinstance(result["delivered"], int)
