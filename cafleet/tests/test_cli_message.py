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


def test_message_show_auth_check__rejects_unknown_agent(
    runner, session_id, agent_id, task_id, monkeypatch
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


def test_message_show_auth_check__accepts_valid_agent(
    runner, session_id, agent_id, task_id, monkeypatch
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


# --- message_poll_auth_check: round-9 consistency follow-up. ``message poll``
# must gate its ``broker.poll_tasks`` call on ``broker.verify_agent_session`` so
# a caller cannot drain another session's inbox by passing any ``--session-id``
# they like. ---


def test_message_poll_auth_check__rejects_unknown_agent(runner, session_id, agent_id, monkeypatch):
    poll_calls: list[tuple] = []

    def fake_verify(aid, sid):
        assert aid == agent_id
        assert sid == session_id
        return False

    def fake_poll_tasks(*args, **kwargs):
        poll_calls.append((args, kwargs))
        return []

    monkeypatch.setattr(broker, "verify_agent_session", fake_verify)
    monkeypatch.setattr(broker, "poll_tasks", fake_poll_tasks)

    result = runner.invoke(
        cli,
        [
            "--session-id",
            session_id,
            "message",
            "poll",
            "--agent-id",
            agent_id,
        ],
    )
    assert result.exit_code == 1, result.output
    out = result.output or ""
    assert agent_id in out
    assert "not a member of session" in out
    assert session_id in out
    assert poll_calls == [], (
        "broker.poll_tasks must not be invoked when verify_agent_session fails"
    )


def test_message_poll_auth_check__accepts_valid_agent(runner, session_id, agent_id, monkeypatch):
    verify_calls: list[tuple] = []
    poll_calls: list[tuple] = []

    def fake_verify(aid, sid):
        verify_calls.append((aid, sid))
        return True

    def fake_poll_tasks(aid, **kwargs):
        poll_calls.append((aid, kwargs))
        return []

    monkeypatch.setattr(broker, "verify_agent_session", fake_verify)
    monkeypatch.setattr(broker, "poll_tasks", fake_poll_tasks)

    result = runner.invoke(
        cli,
        [
            "--session-id",
            session_id,
            "--json",
            "message",
            "poll",
            "--agent-id",
            agent_id,
        ],
    )
    assert result.exit_code == 0, result.output
    assert verify_calls == [(agent_id, session_id)]
    assert len(poll_calls) == 1
    assert poll_calls[0][0] == agent_id


# --- message_ack_auth_check: round-9 consistency follow-up. ``message ack``
# must gate its ``broker.ack_task`` call on ``broker.verify_agent_session``. ---


def test_message_ack_auth_check__rejects_unknown_agent(
    runner, session_id, agent_id, task_id, monkeypatch
):
    ack_calls: list[tuple] = []

    def fake_verify(aid, sid):
        assert aid == agent_id
        assert sid == session_id
        return False

    def fake_ack_task(*args, **kwargs):
        ack_calls.append((args, kwargs))
        return {"task": {}}

    monkeypatch.setattr(broker, "verify_agent_session", fake_verify)
    monkeypatch.setattr(broker, "ack_task", fake_ack_task)

    result = runner.invoke(
        cli,
        [
            "--session-id",
            session_id,
            "message",
            "ack",
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
    assert ack_calls == [], (
        "broker.ack_task must not be invoked when verify_agent_session fails"
    )


def test_message_ack_auth_check__accepts_valid_agent(
    runner, session_id, agent_id, task_id, monkeypatch
):
    verify_calls: list[tuple] = []
    ack_calls: list[tuple] = []

    def fake_verify(aid, sid):
        verify_calls.append((aid, sid))
        return True

    fake_task = {
        "task": {
            "id": task_id,
            "kind": "user",
            "status": "acknowledged",
            "history": [],
            "metadata": {
                "fromAgentId": str(uuid.uuid4()),
                "toAgentId": agent_id,
            },
        }
    }

    def fake_ack_task(aid, tid):
        ack_calls.append((aid, tid))
        return fake_task

    monkeypatch.setattr(broker, "verify_agent_session", fake_verify)
    monkeypatch.setattr(broker, "ack_task", fake_ack_task)

    result = runner.invoke(
        cli,
        [
            "--session-id",
            session_id,
            "--json",
            "message",
            "ack",
            "--agent-id",
            agent_id,
            "--task-id",
            task_id,
        ],
    )
    assert result.exit_code == 0, result.output
    assert verify_calls == [(agent_id, session_id)]
    assert ack_calls == [(agent_id, task_id)]


# --- message_cancel_auth_check: round-9 consistency follow-up. ``message
# cancel`` must gate its ``broker.cancel_task`` call on
# ``broker.verify_agent_session``. ---


def test_message_cancel_auth_check__rejects_unknown_agent(
    runner, session_id, agent_id, task_id, monkeypatch
):
    cancel_calls: list[tuple] = []

    def fake_verify(aid, sid):
        assert aid == agent_id
        assert sid == session_id
        return False

    def fake_cancel_task(*args, **kwargs):
        cancel_calls.append((args, kwargs))
        return {"task": {}}

    monkeypatch.setattr(broker, "verify_agent_session", fake_verify)
    monkeypatch.setattr(broker, "cancel_task", fake_cancel_task)

    result = runner.invoke(
        cli,
        [
            "--session-id",
            session_id,
            "message",
            "cancel",
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
    assert cancel_calls == [], (
        "broker.cancel_task must not be invoked when verify_agent_session fails"
    )


def test_message_cancel_auth_check__accepts_valid_agent(
    runner, session_id, agent_id, task_id, monkeypatch
):
    verify_calls: list[tuple] = []
    cancel_calls: list[tuple] = []

    def fake_verify(aid, sid):
        verify_calls.append((aid, sid))
        return True

    fake_task = {
        "task": {
            "id": task_id,
            "kind": "user",
            "status": "canceled",
            "history": [],
            "metadata": {
                "fromAgentId": agent_id,
                "toAgentId": str(uuid.uuid4()),
            },
        }
    }

    def fake_cancel_task(aid, tid):
        cancel_calls.append((aid, tid))
        return fake_task

    monkeypatch.setattr(broker, "verify_agent_session", fake_verify)
    monkeypatch.setattr(broker, "cancel_task", fake_cancel_task)

    result = runner.invoke(
        cli,
        [
            "--session-id",
            session_id,
            "--json",
            "message",
            "cancel",
            "--agent-id",
            agent_id,
            "--task-id",
            task_id,
        ],
    )
    assert result.exit_code == 0, result.output
    assert verify_calls == [(agent_id, session_id)]
    assert cancel_calls == [(agent_id, task_id)]
