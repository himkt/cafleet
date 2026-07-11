"""End-to-end: ``message broadcast --json --full`` emits ``to_member_id: null`` (design 0000118, item 1.1).

The broadcast-summary row carries no single recipient, so the ``--json``
surface emits JSON ``null`` (not ``0``) for its ``to_member_id``. ``--full`` is
required because the compact render projects ``to_member_id`` out entirely; the
full render is where the null-vs-zero distinction is observable.
"""

import json

import pytest
from click.testing import CliRunner

from cafleet.cli import cli
from tests.broker._helpers import _create_fleet, _register_member


@pytest.fixture
def runner():
    return CliRunner()


def test_broadcast_json_full_emits_null_to_member_id(runner):
    sid = _create_fleet()["fleet_id"]
    sender = _register_member(sid, "sender")
    _register_member(sid, "recipient-a")
    _register_member(sid, "recipient-b")

    result = runner.invoke(
        cli,
        [
            "message",
            "broadcast",
            "--full",
            "--fleet-id",
            str(sid),
            "--from-member-id",
            str(sender["member_id"]),
            "--text",
            "hi all",
            "--json",
        ],
    )
    assert result.exit_code == 0, result.output
    payload = json.loads(result.output)
    assert payload[0]["task"]["to_member_id"] is None
