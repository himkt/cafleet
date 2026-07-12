"""Tests for ``cafleet message show``.

``message show`` is gated by ``broker.verify_member_fleet``: a caller must
prove it belongs to the fleet before it can fetch a message by ID.
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
def message_id():
    return 300


@pytest.fixture
def runner():
    return CliRunner()


def test_message_show_auth_check__rejects_unknown_member(
    runner, fleet_id, member_id, message_id, monkeypatch
):
    """Caller's ``member_id`` is not in ``fleet_id`` → exit 1.

    ``broker.get_message`` MUST NOT be called when verification fails: the
    membership check is the gate.
    """
    get_message_calls: list[tuple] = []

    def fake_verify(mid, sid):
        assert mid == member_id
        assert sid == fleet_id
        return False

    def fake_get_message(*args, **kwargs):
        get_message_calls.append((args, kwargs))
        return {"message": {}}

    monkeypatch.setattr(broker, "verify_member_fleet", fake_verify)
    monkeypatch.setattr(broker, "get_message", fake_get_message)

    result = runner.invoke(
        cli,
        [
            "message",
            "show",
            "--fleet-id",
            str(fleet_id),
            "--member-id",
            str(member_id),
            "--message-id",
            str(message_id),
        ],
    )
    assert result.exit_code == 1, result.output
    out = result.output or ""
    assert str(member_id) in out
    assert "is not in fleet" in out
    assert str(fleet_id) in out
    assert get_message_calls == [], (
        "broker.get_message must not be invoked when verify_member_fleet fails"
    )


def test_message_show_auth_check__accepts_valid_member(
    runner, fleet_id, member_id, message_id, monkeypatch
):
    """Registered member in fleet → broker.get_message is called and the
    message JSON reaches the user."""
    verify_calls: list[tuple] = []

    def fake_verify(mid, sid):
        verify_calls.append((mid, sid))
        return True

    fake_message = {
        "message": {
            "message_id": message_id,
            "owner_member_id": member_id,
            "from_member_id": member_id,
            "to_member_id": 999,
            "type": "unicast",
            "created_at": "2026-05-01T00:00:00+00:00",
            "status_state": "input_required",
            "status_timestamp": "2026-05-01T00:00:00+00:00",
            "origin_message_id": None,
            "text": "hello",
        }
    }

    def fake_get_message(sid, mid):
        assert sid == fleet_id
        assert mid == message_id
        return fake_message

    monkeypatch.setattr(broker, "verify_member_fleet", fake_verify)
    monkeypatch.setattr(broker, "get_message", fake_get_message)

    result = runner.invoke(
        cli,
        [
            "message",
            "show",
            "--fleet-id",
            str(fleet_id),
            "--member-id",
            str(member_id),
            "--message-id",
            str(message_id),
            "--json",
        ],
    )
    assert result.exit_code == 0, result.output
    assert verify_calls == [(member_id, fleet_id)]
    # Compact rendered envelope carries the full integer message_id as ``id``.
    assert str(message_id) in (result.output or "")


# --- message_poll_auth_check: ``message poll`` must gate its
# ``broker.poll_messages`` call on ``broker.verify_member_fleet`` so a caller
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

    def fake_poll_messages(*args, **kwargs):
        poll_calls.append((args, kwargs))
        return []

    monkeypatch.setattr(broker, "verify_member_fleet", fake_verify)
    monkeypatch.setattr(broker, "poll_messages", fake_poll_messages)

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
        "broker.poll_messages must not be invoked when verify_member_fleet fails"
    )


def test_message_poll_auth_check__accepts_valid_member(
    runner, fleet_id, member_id, monkeypatch
):
    verify_calls: list[tuple] = []
    poll_calls: list[int] = []

    def fake_verify(mid, sid):
        verify_calls.append((mid, sid))
        return True

    # No-kwargs signature: the reshaped CLI calls ``broker.poll_messages(member_id)``
    # with no ``--since`` / ``--page-size`` to forward.
    def fake_poll_messages(mid):
        poll_calls.append(mid)
        return []

    monkeypatch.setattr(broker, "verify_member_fleet", fake_verify)
    monkeypatch.setattr(broker, "poll_messages", fake_poll_messages)

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


@pytest.mark.parametrize("subcommand", ["ack", "cancel", "show"])
def test_message_task_id_flag_rejected(runner, fleet_id, member_id, subcommand):
    """The id flag is ``--message-id``; ``--task-id`` no longer parses —
    Click rejects it with its standard 'no such option' error (exit 2)."""
    result = runner.invoke(
        cli,
        [
            "message",
            subcommand,
            "--fleet-id",
            str(fleet_id),
            "--member-id",
            str(member_id),
            "--task-id",
            "300",
        ],
    )
    assert result.exit_code == 2, result.output
    assert "no such option" in (result.output or "").lower()


# --- message_ack_auth_check: ``message ack`` must gate its
# ``broker.ack_message`` call on ``broker.verify_member_fleet``. ---


def test_message_ack_auth_check__rejects_unknown_member(
    runner, fleet_id, member_id, message_id, monkeypatch
):
    ack_calls: list[tuple] = []

    def fake_verify(mid, sid):
        assert mid == member_id
        assert sid == fleet_id
        return False

    def fake_ack_message(*args, **kwargs):
        ack_calls.append((args, kwargs))
        return {"message": {}}

    monkeypatch.setattr(broker, "verify_member_fleet", fake_verify)
    monkeypatch.setattr(broker, "ack_message", fake_ack_message)

    result = runner.invoke(
        cli,
        [
            "message",
            "ack",
            "--fleet-id",
            str(fleet_id),
            "--member-id",
            str(member_id),
            "--message-id",
            str(message_id),
        ],
    )
    assert result.exit_code == 1, result.output
    out = result.output or ""
    assert str(member_id) in out
    assert "is not in fleet" in out
    assert str(fleet_id) in out
    assert ack_calls == [], (
        "broker.ack_message must not be invoked when verify_member_fleet fails"
    )


def test_message_ack_auth_check__accepts_valid_member(
    runner, fleet_id, member_id, message_id, monkeypatch
):
    verify_calls: list[tuple] = []
    ack_calls: list[tuple] = []

    def fake_verify(mid, sid):
        verify_calls.append((mid, sid))
        return True

    fake_message = {
        "message": {
            "message_id": message_id,
            "owner_member_id": member_id,
            "from_member_id": 999,
            "to_member_id": member_id,
            "type": "unicast",
            "created_at": "2026-05-01T00:00:00+00:00",
            "status_state": "completed",
            "status_timestamp": "2026-05-01T00:00:00+00:00",
            "origin_message_id": None,
            "text": "ack-me",
        }
    }

    def fake_ack_message(mid, message_id_arg):
        ack_calls.append((mid, message_id_arg))
        return fake_message

    monkeypatch.setattr(broker, "verify_member_fleet", fake_verify)
    monkeypatch.setattr(broker, "ack_message", fake_ack_message)

    result = runner.invoke(
        cli,
        [
            "message",
            "ack",
            "--fleet-id",
            str(fleet_id),
            "--member-id",
            str(member_id),
            "--message-id",
            str(message_id),
            "--json",
        ],
    )
    assert result.exit_code == 0, result.output
    assert verify_calls == [(member_id, fleet_id)]
    assert ack_calls == [(member_id, message_id)]


# --- message_cancel_auth_check: ``message cancel`` must gate its
# ``broker.cancel_message`` call on ``broker.verify_member_fleet``. ---


def test_message_cancel_auth_check__rejects_unknown_member(
    runner, fleet_id, member_id, message_id, monkeypatch
):
    cancel_calls: list[tuple] = []

    def fake_verify(mid, sid):
        assert mid == member_id
        assert sid == fleet_id
        return False

    def fake_cancel_message(*args, **kwargs):
        cancel_calls.append((args, kwargs))
        return {"message": {}}

    monkeypatch.setattr(broker, "verify_member_fleet", fake_verify)
    monkeypatch.setattr(broker, "cancel_message", fake_cancel_message)

    result = runner.invoke(
        cli,
        [
            "message",
            "cancel",
            "--fleet-id",
            str(fleet_id),
            "--member-id",
            str(member_id),
            "--message-id",
            str(message_id),
        ],
    )
    assert result.exit_code == 1, result.output
    out = result.output or ""
    assert str(member_id) in out
    assert "is not in fleet" in out
    assert str(fleet_id) in out
    assert cancel_calls == [], (
        "broker.cancel_message must not be invoked when verify_member_fleet fails"
    )


def test_message_cancel_auth_check__accepts_valid_member(
    runner, fleet_id, member_id, message_id, monkeypatch
):
    verify_calls: list[tuple] = []
    cancel_calls: list[tuple] = []

    def fake_verify(mid, sid):
        verify_calls.append((mid, sid))
        return True

    fake_message = {
        "message": {
            "message_id": message_id,
            "owner_member_id": 999,
            "from_member_id": member_id,
            "to_member_id": 999,
            "type": "unicast",
            "created_at": "2026-05-01T00:00:00+00:00",
            "status_state": "canceled",
            "status_timestamp": "2026-05-01T00:00:00+00:00",
            "origin_message_id": None,
            "text": "cancel-me",
        }
    }

    def fake_cancel_message(mid, message_id_arg):
        cancel_calls.append((mid, message_id_arg))
        return fake_message

    monkeypatch.setattr(broker, "verify_member_fleet", fake_verify)
    monkeypatch.setattr(broker, "cancel_message", fake_cancel_message)

    result = runner.invoke(
        cli,
        [
            "message",
            "cancel",
            "--fleet-id",
            str(fleet_id),
            "--member-id",
            str(member_id),
            "--message-id",
            str(message_id),
            "--json",
        ],
    )
    assert result.exit_code == 0, result.output
    assert verify_calls == [(member_id, fleet_id)]
    assert cancel_calls == [(member_id, message_id)]


# --- broker-error wrapping: an unexpected broker exception surfaces as a
# clean exit-1 error message (its text verbatim, no traceback). ---


def test_message_show_broker_error__wrapped_as_exit_one_verbatim(
    runner, fleet_id, member_id, message_id, monkeypatch
):
    monkeypatch.setattr(broker, "verify_member_fleet", lambda _m, _s: True)

    def boom(*_args, **_kwargs):
        raise RuntimeError("boom!")

    monkeypatch.setattr(broker, "get_message", boom)

    result = runner.invoke(
        cli,
        [
            "message",
            "show",
            "--fleet-id",
            str(fleet_id),
            "--member-id",
            str(member_id),
            "--message-id",
            str(message_id),
        ],
    )
    assert result.exit_code == 1, result.output
    out = result.output or ""
    assert "boom!" in out
    assert "Traceback" not in out
