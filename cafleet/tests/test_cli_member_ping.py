"""CLI tests for ``cafleet member ping`` (design doc 0000039)."""

import json
import uuid

import pytest
from click.testing import CliRunner

from cafleet import broker, tmux
from cafleet.cli import cli

DIRECTOR_ID = "11111111-1111-1111-1111-111111111111"
MEMBER_ID = "22222222-2222-2222-2222-222222222222"
OTHER_DIRECTOR_ID = "33333333-3333-3333-3333-333333333333"
PANE_ID = "%7"
MEMBER_NAME = "Claude-B"


# Sentinel for ``_agent(placement=...)`` so callers can pass explicit None
# (meaning "no placement row, exercise the missing-placement branch") without
# being silently coerced back to a default valid placement.
_UNSET: object = object()


def _placement(
    *,
    director_agent_id: str = DIRECTOR_ID,
    tmux_pane_id: str | None = PANE_ID,
    coding_agent: str = "claude",
) -> dict:
    return {
        "director_agent_id": director_agent_id,
        "tmux_session": "main",
        "tmux_window_id": "@3",
        "tmux_pane_id": tmux_pane_id,
        "coding_agent": coding_agent,
        "created_at": "2026-04-16T08:00:00+00:00",
    }


def _agent(
    *,
    agent_id: str = MEMBER_ID,
    name: str = MEMBER_NAME,
    placement: dict | None | object = _UNSET,
) -> dict:
    resolved_placement = _placement() if placement is _UNSET else placement
    return {
        "agent_id": agent_id,
        "name": name,
        "description": "Test member",
        "status": "active",
        "registered_at": "2026-04-16T08:00:00+00:00",
        "kind": "user",
        "placement": resolved_placement,
    }


@pytest.fixture
def session_id():
    return str(uuid.uuid4())


@pytest.fixture(autouse=True)
def _stub_tmux_available(monkeypatch):
    """``ensure_tmux_available`` is a no-op for every test in this module."""
    monkeypatch.setattr(tmux, "ensure_tmux_available", lambda: None)


@pytest.fixture
def runner():
    return CliRunner()


@pytest.fixture
def happy_path_agent(monkeypatch):
    """``broker.get_agent`` returns a well-formed target for the Director."""
    monkeypatch.setattr(broker, "get_agent", lambda *_args, **_kw: _agent())


@pytest.fixture
def poll_recorder(monkeypatch):
    """Record every call into ``tmux.send_poll_trigger``.

    Uses ``raising=False`` so the fixture works before the Programmer adds
    the ``member ping`` subcommand to the CLI — clean FAIL beats setup ERROR.
    The fake returns ``True`` so happy-path dispatch tests succeed without
    additional setup.
    """
    calls: list[dict] = []

    def fake(**kwargs):
        calls.append(kwargs)
        return True

    monkeypatch.setattr(tmux, "send_poll_trigger", fake, raising=False)
    return calls


def _invoke(runner, session_id, **invoke_kwargs):
    """Helper: call ``cafleet --session-id <sid> member ping ...`` (no positional)."""
    return runner.invoke(
        cli,
        [
            "--session-id",
            session_id,
            "member",
            "ping",
            "--agent-id",
            DIRECTOR_ID,
            "--member-id",
            MEMBER_ID,
        ],
        **invoke_kwargs,
    )


def test_ping_dispatch__poll_trigger_called_with_correct_kwargs(
    runner, session_id, happy_path_agent, poll_recorder
):
    result = _invoke(runner, session_id)
    assert result.exit_code == 0, result.output
    assert len(poll_recorder) == 1
    call = poll_recorder[0]
    assert call["target_pane_id"] == PANE_ID
    assert call["session_id"] == session_id
    assert call["agent_id"] == MEMBER_ID


def test_ping_dispatch__text_output(
    runner, session_id, happy_path_agent, poll_recorder
):
    result = _invoke(runner, session_id)
    assert result.exit_code == 0, result.output
    out = result.output or ""
    assert "Pinged member" in out
    assert MEMBER_NAME in out
    assert PANE_ID in out
    assert "poll keystroke dispatched" in out


def test_ping_dispatch__json_output_two_keys(
    runner, session_id, happy_path_agent, poll_recorder
):
    result = runner.invoke(
        cli,
        [
            "--session-id",
            session_id,
            "--json",
            "member",
            "ping",
            "--agent-id",
            DIRECTOR_ID,
            "--member-id",
            MEMBER_ID,
        ],
    )
    assert result.exit_code == 0, result.output
    data = json.loads(result.output)
    assert set(data.keys()) == {"member_agent_id", "pane_id"}
    assert data["member_agent_id"] == MEMBER_ID
    assert data["pane_id"] == PANE_ID


def test_send_failure__send_poll_trigger_returns_false_exits_one(
    runner, session_id, happy_path_agent, monkeypatch
):
    monkeypatch.setattr(tmux, "send_poll_trigger", lambda **_kw: False, raising=False)
    result = _invoke(runner, session_id)
    assert result.exit_code == 1, result.output
    out = result.output or ""
    assert "send failed" in out
    assert "tmux send-keys did not deliver the poll-trigger keystroke" in out
    assert PANE_ID in out


def test_send_failure__send_poll_trigger_raises_tmux_error_exits_one(
    runner, session_id, happy_path_agent, monkeypatch
):
    def raise_err(**_kw):
        raise tmux.TmuxError("simulated")

    monkeypatch.setattr(tmux, "send_poll_trigger", raise_err, raising=False)
    result = _invoke(runner, session_id)
    assert result.exit_code == 1, result.output
    assert "send failed: simulated" in (result.output or "")


def test_authorization_boundary__missing_agent_exits_one(
    runner, session_id, monkeypatch
):
    monkeypatch.setattr(broker, "get_agent", lambda *_a, **_kw: None)
    result = _invoke(runner, session_id)
    assert result.exit_code == 1, result.output
    assert MEMBER_ID in (result.output or "")
    assert "not found" in (result.output or "").lower()


def test_authorization_boundary__placement_none_exits_one_with_exact_message(
    runner, session_id, monkeypatch
):
    monkeypatch.setattr(
        broker,
        "get_agent",
        lambda *_a, **_kw: _agent(placement=None),
    )
    result = _invoke(runner, session_id)
    assert result.exit_code == 1, result.output
    out = result.output or ""
    assert f"agent {MEMBER_ID}" in out
    assert "has no placement row" in out
    assert "cafleet member create" in out


def test_authorization_boundary__cross_director_exits_one_with_exact_message(
    runner, session_id, monkeypatch
):
    monkeypatch.setattr(
        broker,
        "get_agent",
        lambda *_a, **_kw: _agent(
            placement=_placement(director_agent_id=OTHER_DIRECTOR_ID)
        ),
    )
    result = _invoke(runner, session_id)
    assert result.exit_code == 1, result.output
    out = result.output or ""
    assert f"agent {MEMBER_ID}" in out
    assert "is not a member of your team" in out
    assert OTHER_DIRECTOR_ID in out


def test_authorization_boundary__pending_pane_exits_one_with_exact_message(
    runner, session_id, monkeypatch
):
    monkeypatch.setattr(
        broker,
        "get_agent",
        lambda *_a, **_kw: _agent(placement=_placement(tmux_pane_id=None)),
    )
    result = _invoke(runner, session_id)
    assert result.exit_code == 1, result.output
    out = result.output or ""
    assert f"member {MEMBER_ID}" in out
    assert "has no pane yet" in out
    assert "pending placement" in out


def test_tmux_unavailable__tmux_not_available_exits_one(
    runner, session_id, happy_path_agent, monkeypatch
):
    def raise_unavailable():
        raise tmux.TmuxError(
            "cafleet member commands must be run inside a tmux session"
        )

    monkeypatch.setattr(tmux, "ensure_tmux_available", raise_unavailable)
    result = _invoke(runner, session_id)
    assert result.exit_code == 1, result.output
    assert "cafleet member commands must be run inside a tmux session" in (
        result.output or ""
    )


def test_input_validation__missing_agent_id_exits_two(runner, session_id):
    result = runner.invoke(
        cli,
        [
            "--session-id",
            session_id,
            "member",
            "ping",
            "--member-id",
            MEMBER_ID,
        ],
    )
    assert result.exit_code == 2, result.output
    out = result.output or ""
    assert "Missing option" in out
    assert "--agent-id" in out


def test_input_validation__missing_member_id_exits_two(runner, session_id):
    result = runner.invoke(
        cli,
        [
            "--session-id",
            session_id,
            "member",
            "ping",
            "--agent-id",
            DIRECTOR_ID,
        ],
    )
    assert result.exit_code == 2, result.output
    out = result.output or ""
    assert "Missing option" in out
    assert "--member-id" in out


def test_input_validation__unexpected_positional_argument_exits_two(runner, session_id):
    result = runner.invoke(
        cli,
        [
            "--session-id",
            session_id,
            "member",
            "ping",
            "--agent-id",
            DIRECTOR_ID,
            "--member-id",
            MEMBER_ID,
            "extra",
        ],
    )
    assert result.exit_code == 2, result.output
    out = result.output or ""
    assert "unexpected extra argument" in out.lower() or "got unexpected" in out.lower()
