"""Tests for ``cafleet message show``.

``message show`` is gated by ``broker.verify_agent_fleet`` (the same
membership check ``agent list`` / ``agent show`` use): a caller must prove it
belongs to the fleet before it can fetch a task by ID.
"""

import pytest
from click.testing import CliRunner

from cafleet import broker
from cafleet.cli import cli


@pytest.fixture
def fleet_id():
    return 100


@pytest.fixture
def agent_id():
    return 200


@pytest.fixture
def task_id():
    return 300


@pytest.fixture
def runner():
    return CliRunner()


def test_message_show_auth_check__rejects_unknown_agent(
    runner, fleet_id, agent_id, task_id, monkeypatch
):
    """Caller's ``agent_id`` is not a member of ``fleet_id`` → exit 1.

    ``broker.get_task`` MUST NOT be called when verification fails: the
    membership check is the gate.
    """
    get_task_calls: list[tuple] = []

    def fake_verify(aid, sid):
        assert aid == agent_id
        assert sid == fleet_id
        return False

    def fake_get_task(*args, **kwargs):
        get_task_calls.append((args, kwargs))
        return {"task": {}}

    monkeypatch.setattr(broker, "verify_agent_fleet", fake_verify)
    monkeypatch.setattr(broker, "get_task", fake_get_task)

    result = runner.invoke(
        cli,
        [
            "message",
            "show",
            "--fleet-id",
            str(fleet_id),
            "--agent-id",
            str(agent_id),
            "--task-id",
            str(task_id),
        ],
    )
    assert result.exit_code == 1, result.output
    out = result.output or ""
    assert str(agent_id) in out
    assert "not a member of fleet" in out
    assert str(fleet_id) in out
    assert get_task_calls == [], (
        "broker.get_task must not be invoked when verify_agent_fleet fails"
    )


def test_message_show_auth_check__accepts_valid_agent(
    runner, fleet_id, agent_id, task_id, monkeypatch
):
    """Registered agent in fleet → broker.get_task is called and the
    task JSON reaches the user."""
    verify_calls: list[tuple] = []

    def fake_verify(aid, sid):
        verify_calls.append((aid, sid))
        return True

    fake_task = {
        "task": {
            "task_id": task_id,
            "context_id": agent_id,
            "from_agent_id": agent_id,
            "to_agent_id": 999,
            "type": "unicast",
            "created_at": "2026-05-01T00:00:00+00:00",
            "status_state": "input_required",
            "status_timestamp": "2026-05-01T00:00:00+00:00",
            "origin_task_id": None,
            "text": "hello",
        }
    }

    def fake_get_task(sid, tid):
        assert sid == fleet_id
        assert tid == task_id
        return fake_task

    monkeypatch.setattr(broker, "verify_agent_fleet", fake_verify)
    monkeypatch.setattr(broker, "get_task", fake_get_task)

    result = runner.invoke(
        cli,
        [
            "--json",
            "message",
            "show",
            "--fleet-id",
            str(fleet_id),
            "--agent-id",
            str(agent_id),
            "--task-id",
            str(task_id),
        ],
    )
    assert result.exit_code == 0, result.output
    assert verify_calls == [(agent_id, fleet_id)]
    # Compact rendered envelope carries the full integer task_id as ``id``.
    assert str(task_id) in (result.output or "")


# --- message_poll_auth_check: ``message poll`` must gate its
# ``broker.poll_tasks`` call on ``broker.verify_agent_fleet`` so a caller
# cannot drain another fleet's inbox by passing any ``--fleet-id`` they
# like. ---


def test_message_poll_auth_check__rejects_unknown_agent(
    runner, fleet_id, agent_id, monkeypatch
):
    poll_calls: list[tuple] = []

    def fake_verify(aid, sid):
        assert aid == agent_id
        assert sid == fleet_id
        return False

    def fake_poll_tasks(*args, **kwargs):
        poll_calls.append((args, kwargs))
        return []

    monkeypatch.setattr(broker, "verify_agent_fleet", fake_verify)
    monkeypatch.setattr(broker, "poll_tasks", fake_poll_tasks)

    result = runner.invoke(
        cli,
        [
            "message",
            "poll",
            "--fleet-id",
            str(fleet_id),
            "--agent-id",
            str(agent_id),
        ],
    )
    assert result.exit_code == 1, result.output
    out = result.output or ""
    assert str(agent_id) in out
    assert "not a member of fleet" in out
    assert str(fleet_id) in out
    assert poll_calls == [], (
        "broker.poll_tasks must not be invoked when verify_agent_fleet fails"
    )


def test_message_poll_auth_check__accepts_valid_agent(
    runner, fleet_id, agent_id, monkeypatch
):
    verify_calls: list[tuple] = []
    poll_calls: list[int] = []

    def fake_verify(aid, sid):
        verify_calls.append((aid, sid))
        return True

    # No-kwargs signature: the reshaped CLI calls ``broker.poll_tasks(agent_id)``
    # with no ``--since`` / ``--page-size`` to forward.
    def fake_poll_tasks(aid):
        poll_calls.append(aid)
        return []

    monkeypatch.setattr(broker, "verify_agent_fleet", fake_verify)
    monkeypatch.setattr(broker, "poll_tasks", fake_poll_tasks)

    result = runner.invoke(
        cli,
        [
            "--json",
            "message",
            "poll",
            "--fleet-id",
            str(fleet_id),
            "--agent-id",
            str(agent_id),
        ],
    )
    assert result.exit_code == 0, result.output
    assert verify_calls == [(agent_id, fleet_id)]
    assert poll_calls == [agent_id]


@pytest.mark.parametrize("removed_flag", ["--since", "--page-size"])
def test_message_poll__removed_flags_rejected(runner, fleet_id, agent_id, removed_flag):
    """``message poll`` no longer accepts ``--since`` / ``--page-size`` —
    Click rejects them with its standard 'no such option' error (exit 2)."""
    result = runner.invoke(
        cli,
        [
            "message",
            "poll",
            "--fleet-id",
            str(fleet_id),
            "--agent-id",
            str(agent_id),
            removed_flag,
            "2026-01-01T00:00:00+00:00",
        ],
    )
    assert result.exit_code == 2, result.output
    assert "no such option" in (result.output or "").lower()


# --- message_ack_auth_check: ``message ack`` must gate its ``broker.ack_task``
# call on ``broker.verify_agent_fleet``. ---


def test_message_ack_auth_check__rejects_unknown_agent(
    runner, fleet_id, agent_id, task_id, monkeypatch
):
    ack_calls: list[tuple] = []

    def fake_verify(aid, sid):
        assert aid == agent_id
        assert sid == fleet_id
        return False

    def fake_ack_task(*args, **kwargs):
        ack_calls.append((args, kwargs))
        return {"task": {}}

    monkeypatch.setattr(broker, "verify_agent_fleet", fake_verify)
    monkeypatch.setattr(broker, "ack_task", fake_ack_task)

    result = runner.invoke(
        cli,
        [
            "message",
            "ack",
            "--fleet-id",
            str(fleet_id),
            "--agent-id",
            str(agent_id),
            "--task-id",
            str(task_id),
        ],
    )
    assert result.exit_code == 1, result.output
    out = result.output or ""
    assert str(agent_id) in out
    assert "not a member of fleet" in out
    assert str(fleet_id) in out
    assert ack_calls == [], (
        "broker.ack_task must not be invoked when verify_agent_fleet fails"
    )


def test_message_ack_auth_check__accepts_valid_agent(
    runner, fleet_id, agent_id, task_id, monkeypatch
):
    verify_calls: list[tuple] = []
    ack_calls: list[tuple] = []

    def fake_verify(aid, sid):
        verify_calls.append((aid, sid))
        return True

    fake_task = {
        "task": {
            "task_id": task_id,
            "context_id": agent_id,
            "from_agent_id": 999,
            "to_agent_id": agent_id,
            "type": "unicast",
            "created_at": "2026-05-01T00:00:00+00:00",
            "status_state": "completed",
            "status_timestamp": "2026-05-01T00:00:00+00:00",
            "origin_task_id": None,
            "text": "ack-me",
        }
    }

    def fake_ack_task(aid, tid):
        ack_calls.append((aid, tid))
        return fake_task

    monkeypatch.setattr(broker, "verify_agent_fleet", fake_verify)
    monkeypatch.setattr(broker, "ack_task", fake_ack_task)

    result = runner.invoke(
        cli,
        [
            "--json",
            "message",
            "ack",
            "--fleet-id",
            str(fleet_id),
            "--agent-id",
            str(agent_id),
            "--task-id",
            str(task_id),
        ],
    )
    assert result.exit_code == 0, result.output
    assert verify_calls == [(agent_id, fleet_id)]
    assert ack_calls == [(agent_id, task_id)]


# --- message_cancel_auth_check: ``message cancel`` must gate its
# ``broker.cancel_task`` call on ``broker.verify_agent_fleet``. ---


def test_message_cancel_auth_check__rejects_unknown_agent(
    runner, fleet_id, agent_id, task_id, monkeypatch
):
    cancel_calls: list[tuple] = []

    def fake_verify(aid, sid):
        assert aid == agent_id
        assert sid == fleet_id
        return False

    def fake_cancel_task(*args, **kwargs):
        cancel_calls.append((args, kwargs))
        return {"task": {}}

    monkeypatch.setattr(broker, "verify_agent_fleet", fake_verify)
    monkeypatch.setattr(broker, "cancel_task", fake_cancel_task)

    result = runner.invoke(
        cli,
        [
            "message",
            "cancel",
            "--fleet-id",
            str(fleet_id),
            "--agent-id",
            str(agent_id),
            "--task-id",
            str(task_id),
        ],
    )
    assert result.exit_code == 1, result.output
    out = result.output or ""
    assert str(agent_id) in out
    assert "not a member of fleet" in out
    assert str(fleet_id) in out
    assert cancel_calls == [], (
        "broker.cancel_task must not be invoked when verify_agent_fleet fails"
    )


def test_message_cancel_auth_check__accepts_valid_agent(
    runner, fleet_id, agent_id, task_id, monkeypatch
):
    verify_calls: list[tuple] = []
    cancel_calls: list[tuple] = []

    def fake_verify(aid, sid):
        verify_calls.append((aid, sid))
        return True

    fake_task = {
        "task": {
            "task_id": task_id,
            "context_id": 999,
            "from_agent_id": agent_id,
            "to_agent_id": 999,
            "type": "unicast",
            "created_at": "2026-05-01T00:00:00+00:00",
            "status_state": "canceled",
            "status_timestamp": "2026-05-01T00:00:00+00:00",
            "origin_task_id": None,
            "text": "cancel-me",
        }
    }

    def fake_cancel_task(aid, tid):
        cancel_calls.append((aid, tid))
        return fake_task

    monkeypatch.setattr(broker, "verify_agent_fleet", fake_verify)
    monkeypatch.setattr(broker, "cancel_task", fake_cancel_task)

    result = runner.invoke(
        cli,
        [
            "--json",
            "message",
            "cancel",
            "--fleet-id",
            str(fleet_id),
            "--agent-id",
            str(agent_id),
            "--task-id",
            str(task_id),
        ],
    )
    assert result.exit_code == 0, result.output
    assert verify_calls == [(agent_id, fleet_id)]
    assert cancel_calls == [(agent_id, task_id)]
