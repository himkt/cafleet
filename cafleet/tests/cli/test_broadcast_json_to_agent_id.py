"""End-to-end: ``message broadcast --json --full`` emits ``to_agent_id: null`` (design 0000118, item 1.1).

The broadcast-summary row carries no single recipient, so the ``--json``
surface emits JSON ``null`` (not ``0``) for its ``to_agent_id``. ``--full`` is
required because the compact render projects ``to_agent_id`` out entirely; the
full render is where the null-vs-zero distinction is observable.
"""

import json

import pytest
from click.testing import CliRunner

from cafleet.cli import cli
from tests.broker._helpers import _create_fleet, _register_agent


@pytest.fixture
def runner():
    return CliRunner()


def test_broadcast_json_full_emits_null_to_agent_id(runner):
    sid = _create_fleet()["fleet_id"]
    sender = _register_agent(sid, "sender")
    _register_agent(sid, "recipient-a")
    _register_agent(sid, "recipient-b")

    result = runner.invoke(
        cli,
        [
            "--json",
            "message",
            "broadcast",
            "--full",
            "--fleet-id",
            str(sid),
            "--agent-id",
            str(sender["agent_id"]),
            "--text",
            "hi all",
        ],
    )
    assert result.exit_code == 0, result.output
    payload = json.loads(result.output)
    assert payload[0]["task"]["to_agent_id"] is None
