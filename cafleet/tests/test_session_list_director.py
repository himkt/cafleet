"""Workstream B: ``session list`` exposes ``director_agent_id``.

``broker.list_sessions`` returns the director's full UUID in each row dict;
the CLI ``session list`` surfaces it in JSON output and as a full-UUID
``DIRECTOR`` text column placed immediately after ``SESSION_ID``.
"""

import json

import pytest
from click.testing import CliRunner

from cafleet import broker
from cafleet.cli import cli
from tests._broker_helpers import _create_session


@pytest.fixture(autouse=True)
def _autouse_broker(broker_session):
    return broker_session


@pytest.fixture
def runner():
    return CliRunner()


def test_list_sessions__row_includes_director_agent_id():
    session = _create_session(label="b-test")
    rows = broker.list_sessions()
    row = next(r for r in rows if r["session_id"] == session["session_id"])
    assert row["director_agent_id"] == session["director"]["agent_id"]


def test_session_list_json__exposes_director_agent_id(runner):
    session = _create_session(label="b-test")
    result = runner.invoke(cli, ["session", "list", "--json"])
    assert result.exit_code == 0, result.output
    data = json.loads(result.output)
    row = next(r for r in data if r["session_id"] == session["session_id"])
    assert row["director_agent_id"] == session["director"]["agent_id"]


def test_session_list_text__shows_full_director_uuid_after_session_id(runner):
    session = _create_session(label="b-test")
    director_id = session["director"]["agent_id"]
    result = runner.invoke(cli, ["session", "list"])
    assert result.exit_code == 0, result.output

    header = next(line for line in result.output.splitlines() if "SESSION_ID" in line)
    assert "DIRECTOR" in header
    assert header.index("SESSION_ID") < header.index("DIRECTOR") < header.index("LABEL")
    # Full UUID (not an 8-char prefix), since it is pasted into the full-only
    # acting --agent-id.
    assert director_id in result.output
