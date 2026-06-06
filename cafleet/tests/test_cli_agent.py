"""Tests for ``cafleet agent ...`` CLI commands.

``agent deregister`` is gated by ``broker.verify_agent_fleet`` (the same
membership check ``agent list`` / ``agent show`` / ``message show`` use): a
caller must prove its ``--agent-id`` belongs to the supplied ``--fleet-id``,
so it cannot deregister an agent outside its fleet.
"""

import uuid

import pytest
from click.testing import CliRunner

from cafleet import broker
from cafleet.cli import cli


@pytest.fixture
def fleet_id():
    return str(uuid.uuid4())


@pytest.fixture
def agent_id():
    return str(uuid.uuid4())


@pytest.fixture
def runner():
    return CliRunner()


# --- agent_deregister_auth_check: ``agent deregister`` must call
# ``broker.verify_agent_fleet`` BEFORE ``broker.deregister_agent``. Without
# the gate, a caller can deregister any agent in the database by supplying an
# unrelated ``--fleet-id``. ---


def test_agent_deregister_auth_check__rejects_unknown_agent(
    runner, fleet_id, agent_id, monkeypatch
):
    deregister_calls: list[tuple] = []

    def fake_verify(aid, sid):
        assert aid == agent_id
        assert sid == fleet_id
        return False

    def fake_deregister(*args, **kwargs):
        deregister_calls.append((args, kwargs))
        return True

    monkeypatch.setattr(broker, "verify_agent_fleet", fake_verify)
    monkeypatch.setattr(broker, "deregister_agent", fake_deregister)

    result = runner.invoke(
        cli,
        [
            "--fleet-id",
            fleet_id,
            "agent",
            "deregister",
            "--agent-id",
            agent_id,
        ],
    )
    assert result.exit_code == 1, result.output
    out = result.output or ""
    assert agent_id in out
    assert "not a member of fleet" in out
    assert fleet_id in out
    assert deregister_calls == [], (
        "broker.deregister_agent must not be invoked when verify_agent_fleet fails"
    )


def test_agent_deregister_auth_check__accepts_valid_agent(
    runner, fleet_id, agent_id, monkeypatch
):
    verify_calls: list[tuple] = []
    deregister_calls: list[tuple] = []

    def fake_verify(aid, sid):
        verify_calls.append((aid, sid))
        return True

    def fake_deregister(aid):
        deregister_calls.append(aid)
        return True

    monkeypatch.setattr(broker, "verify_agent_fleet", fake_verify)
    monkeypatch.setattr(broker, "deregister_agent", fake_deregister)

    result = runner.invoke(
        cli,
        [
            "--fleet-id",
            fleet_id,
            "agent",
            "deregister",
            "--agent-id",
            agent_id,
        ],
    )
    assert result.exit_code == 0, result.output
    assert verify_calls == [(agent_id, fleet_id)]
    assert deregister_calls == [agent_id]
