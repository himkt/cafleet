"""Workstream B: ``fleet list`` exposes ``director_agent_id``.

``broker.list_fleets`` returns the director's full integer id in each row
dict; the CLI ``fleet list`` surfaces it in JSON output and as a
``DIRECTOR`` text column placed immediately after ``FLEET_ID``.
"""

import json

import pytest
from click.testing import CliRunner

from cafleet import broker
from cafleet.cli import cli
from tests._broker_helpers import _create_fleet


@pytest.fixture(autouse=True)
def _autouse_broker(broker_session):
    return broker_session


@pytest.fixture
def runner():
    return CliRunner()


def test_list_fleets__row_includes_director_agent_id():
    fleet = _create_fleet(label="b-test")
    rows = broker.list_fleets()
    row = next(r for r in rows if r["fleet_id"] == fleet["fleet_id"])
    assert row["director_agent_id"] == fleet["director"]["agent_id"]


def test_fleet_list_json__exposes_director_agent_id(runner):
    fleet = _create_fleet(label="b-test")
    result = runner.invoke(cli, ["fleet", "list", "--json"])
    assert result.exit_code == 0, result.output
    data = json.loads(result.output)
    row = next(r for r in data if r["fleet_id"] == fleet["fleet_id"])
    assert row["director_agent_id"] == fleet["director"]["agent_id"]


def test_fleet_list_text__shows_full_director_id_after_fleet_id(runner):
    fleet = _create_fleet(label="b-test")
    director_id = fleet["director"]["agent_id"]
    result = runner.invoke(cli, ["fleet", "list"])
    assert result.exit_code == 0, result.output

    header = next(line for line in result.output.splitlines() if "FLEET_ID" in line)
    assert "DIRECTOR" in header
    assert header.index("FLEET_ID") < header.index("DIRECTOR") < header.index("LABEL")
    # Full integer id is rendered (no prefix slicing).
    assert str(director_id) in result.output
