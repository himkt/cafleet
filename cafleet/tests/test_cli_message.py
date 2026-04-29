"""Tests for ``cafleet message show`` (round-8 auth-check follow-up).

``message show`` previously skipped the ``broker.verify_agent_session``
membership check — any process holding the database file could fetch any
task by ID without proving its caller belonged to the session. Round-8
adds the same check that ``agent list`` / ``agent show`` already do.
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
def task_id():
    return str(uuid.uuid4())


@pytest.fixture
def runner():
    return CliRunner()


class TestMessageShowAuthCheck:
    def test_rejects_unknown_agent(
        self, runner, session_id, agent_id, task_id, monkeypatch
    ):
        """Caller's ``agent_id`` is not a member of ``session_id`` → exit 1.

        ``broker.get_task`` MUST NOT be called when verification fails: the
        membership check is the gate.
        """
        get_task_calls: list[tuple] = []

        def fake_verify(aid, sid):
            assert aid == agent_id
            assert sid == session_id
            return False

        def fake_get_task(*args, **kwargs):
            get_task_calls.append((args, kwargs))
            return {"task": {}}

        monkeypatch.setattr(broker, "verify_agent_session", fake_verify)
        monkeypatch.setattr(broker, "get_task", fake_get_task)

        result = runner.invoke(
            cli,
            [
                "--session-id",
                session_id,
                "message",
                "show",
                "--agent-id",
                agent_id,
                "--task-id",
                task_id,
            ],
        )
        assert result.exit_code == 1, result.output
        out = result.output or ""
        assert agent_id in out
        assert "not a member of session" in out
        assert session_id in out
        assert get_task_calls == [], (
            "broker.get_task must not be invoked when verify_agent_session fails"
        )

    def test_accepts_valid_agent(
        self, runner, session_id, agent_id, task_id, monkeypatch
    ):
        """Registered agent in session → broker.get_task is called and the
        task JSON reaches the user."""
        verify_calls: list[tuple] = []

        def fake_verify(aid, sid):
            verify_calls.append((aid, sid))
            return True

        fake_task = {
            "task": {
                "id": task_id,
                "kind": "user",
                "status": "submitted",
                "history": [],
                "metadata": {
                    "fromAgentId": agent_id,
                    "toAgentId": str(uuid.uuid4()),
                },
            }
        }

        def fake_get_task(sid, tid):
            assert sid == session_id
            assert tid == task_id
            return fake_task

        monkeypatch.setattr(broker, "verify_agent_session", fake_verify)
        monkeypatch.setattr(broker, "get_task", fake_get_task)

        result = runner.invoke(
            cli,
            [
                "--session-id",
                session_id,
                "--json",
                "message",
                "show",
                "--agent-id",
                agent_id,
                "--task-id",
                task_id,
            ],
        )
        assert result.exit_code == 0, result.output
        assert verify_calls == [(agent_id, session_id)]
        assert task_id in (result.output or "")
