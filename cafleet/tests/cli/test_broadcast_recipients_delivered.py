"""End-to-end: ``message broadcast`` prints ``recipients=N delivered=k``.

The compact one-line echo surfaces both counts — ``recipients`` (real peer
count N) and ``delivered`` (previews that landed, k).
"""

import pytest
from click.testing import CliRunner

from cafleet.cli import cli
from tests.broker._helpers import _create_fleet, _register_member


@pytest.fixture
def runner():
    return CliRunner()


def test_broadcast_cli_prints_recipients_and_delivered(runner):
    sid = _create_fleet()["fleet_id"]
    sender = _register_member(sid, "sender")
    _register_member(sid, "recipient-a")
    _register_member(sid, "recipient-b")

    result = runner.invoke(
        cli,
        [
            "message",
            "broadcast",
            "--fleet-id",
            str(sid),
            "--from-member-id",
            str(sender["member_id"]),
            "--text",
            "hi all",
        ],
    )
    assert result.exit_code == 0, result.output
    # Three peers of the sender: pane-bound root Director + recipient-a/-b.
    # Only the Director's pane receives a preview → N=3, k=1.
    assert "recipients=3" in result.output
    assert "delivered=1" in result.output
