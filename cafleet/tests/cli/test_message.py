"""Tests for ``cafleet message show``.

``message show`` is gated by ``broker.verify_member_fleet``: a caller must
prove it belongs to the fleet before it can fetch a task by ID.
"""

import pytest
from click.testing import CliRunner

from cafleet import broker
from cafleet.cli import cli


@pytest.fixture
def fleet_id():
    return 100


@pytest.fixture
def member_id():
    return 200


@pytest.fixture
def task_id():
    return 300


@pytest.fixture
def runner():
    return CliRunner()


def test_message_show_auth_check__rejects_unknown_member(
    runner, fleet_id, member_id, task_id, monkeypatch
):
    """Caller's ``member_id`` is not in ``fleet_id`` → exit 1.

    ``broker.get_task`` MUST NOT be called when verification fails: the
    membership check is the gate.
    """
    get_task_calls: list[tuple] = []

    def fake_verify(mid, sid):
        assert mid == member_id
        assert sid == fleet_id
        return False

    def fake_get_task(*args, **kwargs):
        get_task_calls.append((args, kwargs))
        return {"task": {}}

    monkeypatch.setattr(broker, "verify_member_fleet", fake_verify)
    monkeypatch.setattr(broker, "get_task", fake_get_task)

    result = runner.invoke(
        cli,
        [
            "message",
            "show",
            "--fleet-id",
            str(fleet_id),
            "--member-id",
            str(member_id),
            "--task-id",
            str(task_id),
        ],
    )
    assert result.exit_code == 1, result.output
    out = result.output or ""
    assert str(member_id) in out
    assert "is not in fleet" in out
    assert str(fleet_id) in out
    assert get_task_calls == [], (
        "broker.get_task must not be invoked when verify_member_fleet fails"
    )


def test_message_show_auth_check__accepts_valid_member(
    runner, fleet_id, member_id, task_id, monkeypatch
):
    """Registered member in fleet → broker.get_task is called and the
    task JSON reaches the user."""
    verify_calls: list[tuple] = []

    def fake_verify(mid, sid):
        verify_calls.append((mid, sid))
        return True

    fake_task = {
        "task": {
            "task_id": task_id,
            "context_id": member_id,
            "from_member_id": member_id,
            "to_member_id": 999,
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

    monkeypatch.setattr(broker, "verify_member_fleet", fake_verify)
    monkeypatch.setattr(broker, "get_task", fake_get_task)

    result = runner.invoke(
        cli,
        [
            "message",
            "show",
            "--fleet-id",
            str(fleet_id),
            "--member-id",
            str(member_id),
            "--task-id",
            str(task_id),
            "--json",
        ],
    )
    assert result.exit_code == 0, result.output
    assert verify_calls == [(member_id, fleet_id)]
    # Compact rendered envelope carries the full integer task_id as ``id``.
    assert str(task_id) in (result.output or "")


# --- message_poll_auth_check: ``message poll`` must gate its
# ``broker.poll_tasks`` call on ``broker.verify_member_fleet`` so a caller
# cannot drain another fleet's inbox by passing any ``--fleet-id`` they
# like. ---


def test_message_poll_auth_check__rejects_unknown_member(
    runner, fleet_id, member_id, monkeypatch
):
    poll_calls: list[tuple] = []

    def fake_verify(mid, sid):
        assert mid == member_id
        assert sid == fleet_id
        return False

    def fake_poll_tasks(*args, **kwargs):
        poll_calls.append((args, kwargs))
        return []

    monkeypatch.setattr(broker, "verify_member_fleet", fake_verify)
    monkeypatch.setattr(broker, "poll_tasks", fake_poll_tasks)

    result = runner.invoke(
        cli,
        [
            "message",
            "poll",
            "--fleet-id",
            str(fleet_id),
            "--member-id",
            str(member_id),
        ],
    )
    assert result.exit_code == 1, result.output
    out = result.output or ""
    assert str(member_id) in out
    assert "is not in fleet" in out
    assert str(fleet_id) in out
    assert poll_calls == [], (
        "broker.poll_tasks must not be invoked when verify_member_fleet fails"
    )


def test_message_poll_auth_check__accepts_valid_member(
    runner, fleet_id, member_id, monkeypatch
):
    verify_calls: list[tuple] = []
    poll_calls: list[int] = []

    def fake_verify(mid, sid):
        verify_calls.append((mid, sid))
        return True

    # No-kwargs signature: the reshaped CLI calls ``broker.poll_tasks(member_id)``
    # with no ``--since`` / ``--page-size`` to forward.
    def fake_poll_tasks(mid):
        poll_calls.append(mid)
        return []

    monkeypatch.setattr(broker, "verify_member_fleet", fake_verify)
    monkeypatch.setattr(broker, "poll_tasks", fake_poll_tasks)

    result = runner.invoke(
        cli,
        [
            "message",
            "poll",
            "--fleet-id",
            str(fleet_id),
            "--member-id",
            str(member_id),
            "--json",
        ],
    )
    assert result.exit_code == 0, result.output
    assert verify_calls == [(member_id, fleet_id)]
    assert poll_calls == [member_id]


@pytest.mark.parametrize("removed_flag", ["--since", "--page-size"])
def test_message_poll__removed_flags_rejected(
    runner, fleet_id, member_id, removed_flag
):
    """``message poll`` takes no ``--since`` / ``--page-size`` options —
    Click rejects them with its standard 'no such option' error (exit 2)."""
    result = runner.invoke(
        cli,
        [
            "message",
            "poll",
            "--fleet-id",
            str(fleet_id),
            "--member-id",
            str(member_id),
            removed_flag,
            "2026-01-01T00:00:00+00:00",
        ],
    )
    assert result.exit_code == 2, result.output
    assert "no such option" in (result.output or "").lower()


# --- message_ack_auth_check: ``message ack`` must gate its ``broker.ack_task``
# call on ``broker.verify_member_fleet``. ---


def test_message_ack_auth_check__rejects_unknown_member(
    runner, fleet_id, member_id, task_id, monkeypatch
):
    ack_calls: list[tuple] = []

    def fake_verify(mid, sid):
        assert mid == member_id
        assert sid == fleet_id
        return False

    def fake_ack_task(*args, **kwargs):
        ack_calls.append((args, kwargs))
        return {"task": {}}

    monkeypatch.setattr(broker, "verify_member_fleet", fake_verify)
    monkeypatch.setattr(broker, "ack_task", fake_ack_task)

    result = runner.invoke(
        cli,
        [
            "message",
            "ack",
            "--fleet-id",
            str(fleet_id),
            "--member-id",
            str(member_id),
            "--task-id",
            str(task_id),
        ],
    )
    assert result.exit_code == 1, result.output
    out = result.output or ""
    assert str(member_id) in out
    assert "is not in fleet" in out
    assert str(fleet_id) in out
    assert ack_calls == [], (
        "broker.ack_task must not be invoked when verify_member_fleet fails"
    )


def test_message_ack_auth_check__accepts_valid_member(
    runner, fleet_id, member_id, task_id, monkeypatch
):
    verify_calls: list[tuple] = []
    ack_calls: list[tuple] = []

    def fake_verify(mid, sid):
        verify_calls.append((mid, sid))
        return True

    fake_task = {
        "task": {
            "task_id": task_id,
            "context_id": member_id,
            "from_member_id": 999,
            "to_member_id": member_id,
            "type": "unicast",
            "created_at": "2026-05-01T00:00:00+00:00",
            "status_state": "completed",
            "status_timestamp": "2026-05-01T00:00:00+00:00",
            "origin_task_id": None,
            "text": "ack-me",
        }
    }

    def fake_ack_task(mid, tid):
        ack_calls.append((mid, tid))
        return fake_task

    monkeypatch.setattr(broker, "verify_member_fleet", fake_verify)
    monkeypatch.setattr(broker, "ack_task", fake_ack_task)

    result = runner.invoke(
        cli,
        [
            "message",
            "ack",
            "--fleet-id",
            str(fleet_id),
            "--member-id",
            str(member_id),
            "--task-id",
            str(task_id),
            "--json",
        ],
    )
    assert result.exit_code == 0, result.output
    assert verify_calls == [(member_id, fleet_id)]
    assert ack_calls == [(member_id, task_id)]


# --- message_cancel_auth_check: ``message cancel`` must gate its
# ``broker.cancel_task`` call on ``broker.verify_member_fleet``. ---


def test_message_cancel_auth_check__rejects_unknown_member(
    runner, fleet_id, member_id, task_id, monkeypatch
):
    cancel_calls: list[tuple] = []

    def fake_verify(mid, sid):
        assert mid == member_id
        assert sid == fleet_id
        return False

    def fake_cancel_task(*args, **kwargs):
        cancel_calls.append((args, kwargs))
        return {"task": {}}

    monkeypatch.setattr(broker, "verify_member_fleet", fake_verify)
    monkeypatch.setattr(broker, "cancel_task", fake_cancel_task)

    result = runner.invoke(
        cli,
        [
            "message",
            "cancel",
            "--fleet-id",
            str(fleet_id),
            "--member-id",
            str(member_id),
            "--task-id",
            str(task_id),
        ],
    )
    assert result.exit_code == 1, result.output
    out = result.output or ""
    assert str(member_id) in out
    assert "is not in fleet" in out
    assert str(fleet_id) in out
    assert cancel_calls == [], (
        "broker.cancel_task must not be invoked when verify_member_fleet fails"
    )


def test_message_cancel_auth_check__accepts_valid_member(
    runner, fleet_id, member_id, task_id, monkeypatch
):
    verify_calls: list[tuple] = []
    cancel_calls: list[tuple] = []

    def fake_verify(mid, sid):
        verify_calls.append((mid, sid))
        return True

    fake_task = {
        "task": {
            "task_id": task_id,
            "context_id": 999,
            "from_member_id": member_id,
            "to_member_id": 999,
            "type": "unicast",
            "created_at": "2026-05-01T00:00:00+00:00",
            "status_state": "canceled",
            "status_timestamp": "2026-05-01T00:00:00+00:00",
            "origin_task_id": None,
            "text": "cancel-me",
        }
    }

    def fake_cancel_task(mid, tid):
        cancel_calls.append((mid, tid))
        return fake_task

    monkeypatch.setattr(broker, "verify_member_fleet", fake_verify)
    monkeypatch.setattr(broker, "cancel_task", fake_cancel_task)

    result = runner.invoke(
        cli,
        [
            "message",
            "cancel",
            "--fleet-id",
            str(fleet_id),
            "--member-id",
            str(member_id),
            "--task-id",
            str(task_id),
            "--json",
        ],
    )
    assert result.exit_code == 0, result.output
    assert verify_calls == [(member_id, fleet_id)]
    assert cancel_calls == [(member_id, task_id)]
