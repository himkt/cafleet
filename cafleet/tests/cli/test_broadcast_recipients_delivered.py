"""End-to-end: ``message broadcast`` prints ``recipients=N delivered=k`` (design 0000118, item 2.1).

The compact one-line echo surfaces both counts — ``recipients`` (real peer
count N) and ``delivered`` (previews that landed, k) — rather than the single
legacy count mislabeled as ``recipients``.
"""

import pytest
from click.testing import CliRunner

from cafleet.cli import cli
from tests.broker._helpers import _create_fleet, _register_agent


@pytest.fixture
def runner():
    return CliRunner()


def test_broadcast_cli_prints_recipients_and_delivered(runner):
    sid = _create_fleet()["fleet_id"]
    sender = _register_agent(sid, "sender")
    _register_agent(sid, "recipient-a")
    _register_agent(sid, "recipient-b")

    result = runner.invoke(
        cli,
        [
            "message",
            "broadcast",
            "--fleet-id",
            str(sid),
            "--agent-id",
            str(sender["agent_id"]),
            "--text",
            "hi all",
        ],
    )
    assert result.exit_code == 0, result.output
    # Two recipients registered without panes → N=2, k=0.
    assert "recipients=2" in result.output
    assert "delivered=0" in result.output
