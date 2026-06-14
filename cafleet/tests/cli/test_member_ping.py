"""CLI tests for ``cafleet member ping``."""

import json

import pytest
from click.testing import CliRunner

from cafleet import broker
from cafleet.cli import cli
from cafleet.multiplexer import tmux as multiplexer_tmux
from cafleet.multiplexer.tmux import TmuxError, TmuxMultiplexer
from tests.cli._member_helpers import (
    MEMBER_ID,
    MEMBER_NAME,
    PANE_ID,
    _agent,
    _placement,
)


@pytest.fixture
def fleet_id():
    return 100


@pytest.fixture(autouse=True)
def _stub_tmux_available(monkeypatch):
    """``ensure_available`` is a no-op for every test in this module."""
    monkeypatch.setattr(TmuxMultiplexer, "ensure_available", lambda self: None)


@pytest.fixture
def runner():
    return CliRunner()


@pytest.fixture
def happy_path_agent(monkeypatch):
    """``broker.get_agent`` returns a well-formed target for the Director."""
    monkeypatch.setattr(broker, "get_agent", lambda *_args, **_kw: _agent())


@pytest.fixture
def poll_recorder(monkeypatch):
    """Record every call into ``TmuxMultiplexer.send_poll_trigger``.

    Uses ``raising=False`` so the fixture works before the Programmer adds
    the ``member ping`` subcommand to the CLI — clean FAIL beats setup ERROR.
    The fake returns ``True`` so happy-path dispatch tests succeed without
    additional setup.
    """
    calls: list[dict] = []

    def fake(self, **kwargs):
        calls.append(kwargs)
        return True

    monkeypatch.setattr(TmuxMultiplexer, "send_poll_trigger", fake, raising=False)
    return calls


def _invoke(runner, fleet_id, **invoke_kwargs):
    """Helper: call ``cafleet member ping --fleet-id <sid> ...`` (no positional)."""
    return runner.invoke(
        cli,
        [
            "member",
            "ping",
            "--fleet-id",
            str(fleet_id),
            "--member-id",
            str(MEMBER_ID),
        ],
        **invoke_kwargs,
    )


def test_ping_dispatch__poll_trigger_called_with_correct_kwargs(
    runner, fleet_id, happy_path_agent, poll_recorder
):
    result = _invoke(runner, fleet_id)
    assert result.exit_code == 0, result.output
    assert len(poll_recorder) == 1
    call = poll_recorder[0]
    assert call["target_pane_id"] == PANE_ID
    assert call["fleet_id"] == fleet_id
    assert call["agent_id"] == MEMBER_ID


def test_ping__keystrokes_escape_first(runner, fleet_id, happy_path_agent, monkeypatch):
    """`member ping` inherits the `Esc` safeguard from `send_poll_trigger`: with
    the real keystroke helper in play, the sequence leads with `Escape` before
    the literal poll command + Enter (design 0000090 §2, C7).

    Unlike the other tests in this module, this one does NOT stub
    `send_poll_trigger` — it lets the real helper run with `_run`/`shutil.which`
    mocked so the inherited `esc_first=True` behavior is exercised end-to-end.
    """
    monkeypatch.setattr("shutil.which", lambda _name: "/usr/bin/tmux")
    monkeypatch.setattr("time.sleep", lambda _secs: None)
    captured: list[list[str]] = []
    monkeypatch.setattr(
        multiplexer_tmux,
        "_run",
        lambda args, **_kw: captured.append(list(args)) or "",
    )

    result = _invoke(runner, fleet_id)
    assert result.exit_code == 0, result.output

    assert captured[0] == ["tmux", "send-keys", "-t", PANE_ID, "Escape"]
    assert captured[1] == [
        "tmux",
        "send-keys",
        "-t",
        PANE_ID,
        "-l",
        f"cafleet message poll --fleet-id {fleet_id} --agent-id {MEMBER_ID}",
    ]
    assert captured[-1] == ["tmux", "send-keys", "-t", PANE_ID, "Enter"]


def test_ping_dispatch__text_output(runner, fleet_id, happy_path_agent, poll_recorder):
    result = _invoke(runner, fleet_id)
    assert result.exit_code == 0, result.output
    out = result.output or ""
    assert "Pinged member" in out
    assert MEMBER_NAME in out
    assert PANE_ID in out
    assert "poll keystroke dispatched" in out


def test_ping_dispatch__json_output_two_keys(
    runner, fleet_id, happy_path_agent, poll_recorder
):
    result = runner.invoke(
        cli,
        [
            "--json",
            "member",
            "ping",
            "--fleet-id",
            str(fleet_id),
            "--member-id",
            str(MEMBER_ID),
        ],
    )
    assert result.exit_code == 0, result.output
    data = json.loads(result.output)
    assert set(data.keys()) == {"member_agent_id", "pane_id"}
    assert data["member_agent_id"] == MEMBER_ID
    assert data["pane_id"] == PANE_ID


def test_send_failure__send_poll_trigger_returns_false_exits_one(
    runner, fleet_id, happy_path_agent, monkeypatch
):
    monkeypatch.setattr(
        TmuxMultiplexer, "send_poll_trigger", lambda self, **_kw: False, raising=False
    )
    result = _invoke(runner, fleet_id)
    assert result.exit_code == 1, result.output
    out = result.output or ""
    assert "send failed" in out
    assert "tmux send-keys did not deliver the poll-trigger keystroke" in out
    assert PANE_ID in out


def test_send_failure__send_poll_trigger_raises_tmux_error_exits_one(
    runner, fleet_id, happy_path_agent, monkeypatch
):
    def raise_err(self, **_kw):
        raise TmuxError("simulated")

    monkeypatch.setattr(TmuxMultiplexer, "send_poll_trigger", raise_err, raising=False)
    result = _invoke(runner, fleet_id)
    assert result.exit_code == 1, result.output
    assert "send failed: simulated" in (result.output or "")


def test_authorization_boundary__missing_agent_exits_one(runner, fleet_id, monkeypatch):
    monkeypatch.setattr(broker, "get_agent", lambda *_a, **_kw: None)
    result = _invoke(runner, fleet_id)
    assert result.exit_code == 1, result.output
    assert str(MEMBER_ID) in (result.output or "")
    assert "not found" in (result.output or "").lower()


def test_authorization_boundary__placement_none_exits_one_with_exact_message(
    runner, fleet_id, monkeypatch
):
    monkeypatch.setattr(
        broker,
        "get_agent",
        lambda *_a, **_kw: _agent(placement=None),
    )
    result = _invoke(runner, fleet_id)
    assert result.exit_code == 1, result.output
    out = result.output or ""
    assert f"agent {MEMBER_ID}" in out
    assert "has no placement row" in out
    assert "cafleet member create" in out


def test_authorization_boundary__pending_pane_exits_one_with_exact_message(
    runner, fleet_id, monkeypatch
):
    monkeypatch.setattr(
        broker,
        "get_agent",
        lambda *_a, **_kw: _agent(placement=_placement(tmux_pane_id=None)),
    )
    result = _invoke(runner, fleet_id)
    assert result.exit_code == 1, result.output
    out = result.output or ""
    assert f"member {MEMBER_ID}" in out
    assert "has no pane yet" in out
    assert "pending placement" in out


def test_tmux_unavailable__tmux_not_available_exits_one(
    runner, fleet_id, happy_path_agent, monkeypatch
):
    def raise_unavailable(self):
        raise TmuxError("cafleet member commands must be run inside a tmux session")

    monkeypatch.setattr(TmuxMultiplexer, "ensure_available", raise_unavailable)
    result = _invoke(runner, fleet_id)
    assert result.exit_code == 1, result.output
    assert "cafleet member commands must be run inside a tmux session" in (
        result.output or ""
    )


def test_input_validation__agent_id_flag_removed(runner, fleet_id):
    """``member ping`` no longer accepts ``--agent-id`` — Click rejects it with
    its standard 'no such option' error (exit 2)."""
    result = runner.invoke(
        cli,
        [
            "member",
            "ping",
            "--fleet-id",
            str(fleet_id),
            "--agent-id",
            "999",
            "--member-id",
            str(MEMBER_ID),
        ],
    )
    assert result.exit_code == 2, result.output
    assert "no such option" in (result.output or "").lower()


def test_input_validation__missing_member_id_exits_two(runner, fleet_id):
    result = runner.invoke(
        cli,
        [
            "member",
            "ping",
            "--fleet-id",
            str(fleet_id),
        ],
    )
    assert result.exit_code == 2, result.output
    out = result.output or ""
    assert "Missing option" in out
    assert "--member-id" in out


def test_input_validation__unexpected_positional_argument_exits_two(runner, fleet_id):
    result = runner.invoke(
        cli,
        [
            "member",
            "ping",
            "--fleet-id",
            str(fleet_id),
            "--member-id",
            str(MEMBER_ID),
            "extra",
        ],
    )
    assert result.exit_code == 2, result.output
    out = result.output or ""
    assert "unexpected extra argument" in out.lower() or "got unexpected" in out.lower()
