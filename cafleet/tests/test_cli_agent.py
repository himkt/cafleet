"""Tests for ``cafleet agent ...`` CLI commands (round-9 auth-check follow-up).

``agent deregister`` previously relied on ``broker.deregister_agent(agent_id)``
which is ``session_id``-blind: any caller passing any ``--session-id`` could
deregister any agent in the database. Round-9 adds the same
``broker.verify_agent_session`` gate that ``agent list`` / ``agent show`` /
``message show`` already use.
"""

import uuid

import pytest
from click.testing import CliRunner

from cafleet import broker
from cafleet.cli import cli


@pytest.fixture
def session_id():
    return str(uuid.uuid4())


@pytest.fixture
def agent_id():
    return str(uuid.uuid4())


@pytest.fixture
def runner():
    return CliRunner()


class TestAgentDeregisterAuthCheck:
    """``agent deregister`` must call ``broker.verify_agent_session`` BEFORE
    ``broker.deregister_agent``. Without the gate, a caller can deregister
    any agent in the database by supplying an unrelated ``--session-id``.
    """

    def test_rejects_unknown_agent(
        self, runner, session_id, agent_id, monkeypatch
    ):
        deregister_calls: list[tuple] = []

        def fake_verify(aid, sid):
            assert aid == agent_id
            assert sid == session_id
            return False

        def fake_deregister(*args, **kwargs):
            deregister_calls.append((args, kwargs))
            return True

        monkeypatch.setattr(broker, "verify_agent_session", fake_verify)
        monkeypatch.setattr(broker, "deregister_agent", fake_deregister)

        result = runner.invoke(
            cli,
            [
                "--session-id",
                session_id,
                "agent",
                "deregister",
                "--agent-id",
                agent_id,
            ],
        )
        assert result.exit_code == 1, result.output
        out = result.output or ""
        assert agent_id in out
        assert "not a member of session" in out
        assert session_id in out
        assert deregister_calls == [], (
            "broker.deregister_agent must not be invoked when "
            "verify_agent_session fails"
        )

    def test_accepts_valid_agent(
        self, runner, session_id, agent_id, monkeypatch
    ):
        verify_calls: list[tuple] = []
        deregister_calls: list[tuple] = []

        def fake_verify(aid, sid):
            verify_calls.append((aid, sid))
            return True

        def fake_deregister(aid):
            deregister_calls.append(aid)
            return True

        monkeypatch.setattr(broker, "verify_agent_session", fake_verify)
        monkeypatch.setattr(broker, "deregister_agent", fake_deregister)

        result = runner.invoke(
            cli,
            [
                "--session-id",
                session_id,
                "agent",
                "deregister",
                "--agent-id",
                agent_id,
            ],
        )
        assert result.exit_code == 0, result.output
        assert verify_calls == [(agent_id, session_id)]
        assert deregister_calls == [agent_id]
