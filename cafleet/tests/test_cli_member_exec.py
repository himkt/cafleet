"""CLI tests for ``cafleet member exec`` (design doc 0000038)."""

import json
import uuid

import pytest
from click.testing import CliRunner

from cafleet import broker, tmux
from cafleet.cli import cli
from tests._member_cli_helpers import (
    DIRECTOR_ID,
    MEMBER_ID,
    MEMBER_NAME,
    OTHER_DIRECTOR_ID,
    PANE_ID,
    _agent,
    _placement,
)


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
def bash_recorder(monkeypatch):
    """Record every call into ``tmux.send_bash_command``.

    Uses ``raising=False`` so the fixture works before the Programmer adds
    the ``member exec`` subcommand to the CLI — clean FAIL beats setup ERROR.
    """
    calls: list[dict] = []

    def fake(**kwargs):
        calls.append(kwargs)

    monkeypatch.setattr(tmux, "send_bash_command", fake, raising=False)
    return calls


def _invoke(runner, session_id, *extra_args, **invoke_kwargs):
    """Helper: call ``cafleet --session-id <sid> member exec ...``."""
    return runner.invoke(
        cli,
        [
            "--session-id",
            session_id,
            "member",
            "exec",
            "--agent-id",
            DIRECTOR_ID,
            "--member-id",
            MEMBER_ID,
            *extra_args,
        ],
        **invoke_kwargs,
    )


def test_exec_dispatch__positional_cmd_dispatched_with_pane_and_command(
    runner, session_id, happy_path_agent, bash_recorder
):
    result = _invoke(runner, session_id, "git log -1 --oneline")
    assert result.exit_code == 0, result.output
    assert len(bash_recorder) == 1
    call = bash_recorder[0]
    assert call["target_pane_id"] == PANE_ID
    assert call["command"] == "git log -1 --oneline"


def test_exec_dispatch__text_output(
    runner, session_id, happy_path_agent, bash_recorder
):
    result = _invoke(runner, session_id, "git log -1 --oneline")
    assert result.exit_code == 0, result.output
    out = result.output or ""
    assert "Sent bash command" in out
    assert "git log -1 --oneline" in out
    assert MEMBER_NAME in out
    assert PANE_ID in out


def test_exec_dispatch__json_output_three_keys(
    runner, session_id, happy_path_agent, bash_recorder
):
    payload = "git log -1 --oneline"
    result = runner.invoke(
        cli,
        [
            "--session-id",
            session_id,
            "--json",
            "member",
            "exec",
            "--agent-id",
            DIRECTOR_ID,
            "--member-id",
            MEMBER_ID,
            payload,
        ],
    )
    assert result.exit_code == 0, result.output
    data = json.loads(result.output)
    assert set(data.keys()) == {"member_agent_id", "pane_id", "command"}
    assert data["member_agent_id"] == MEMBER_ID
    assert data["pane_id"] == PANE_ID
    assert data["command"] == payload


def test_input_validation__missing_positional_exits_two(
    runner, session_id, happy_path_agent
):
    result = _invoke(runner, session_id)
    assert result.exit_code == 2, result.output
    assert "Missing argument" in (result.output or "")


def test_input_validation__empty_command_exits_two(
    runner, session_id, happy_path_agent
):
    result = _invoke(runner, session_id, "")
    assert result.exit_code == 2, result.output
    assert "command may not be empty." in (result.output or "")


def test_input_validation__whitespace_only_command_exits_two(
    runner, session_id, happy_path_agent
):
    result = _invoke(runner, session_id, "   ")
    assert result.exit_code == 2, result.output
    assert "command may not be empty." in (result.output or "")


@pytest.mark.parametrize(
    "bad_command",
    [
        "\n",
        "\r",
        "\r\n",
        "\nls",
        "ls\n",
    ],
)
def test_input_validation__command_with_newline_exits_two(
    runner, session_id, happy_path_agent, bad_command
):
    result = _invoke(runner, session_id, bad_command)
    assert result.exit_code == 2, result.output
    assert "command may not contain newlines." in (result.output or "")


def test_authorization_boundary__missing_agent_exits_one(
    runner, session_id, monkeypatch
):
    monkeypatch.setattr(broker, "get_agent", lambda *_a, **_kw: None)
    result = _invoke(runner, session_id, "git log -1")
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
    result = _invoke(runner, session_id, "git log -1")
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
    result = _invoke(runner, session_id, "git log -1")
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
    result = _invoke(runner, session_id, "git log -1")
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
    result = _invoke(runner, session_id, "git log -1")
    assert result.exit_code == 1, result.output
    assert "cafleet member commands must be run inside a tmux session" in (
        result.output or ""
    )
