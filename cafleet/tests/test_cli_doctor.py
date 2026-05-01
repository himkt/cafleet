"""CLI tests for the ``cafleet doctor`` subcommand (design 0000032 §2)."""

import json

import pytest
from click.testing import CliRunner

from cafleet import tmux
from cafleet.cli import cli
from cafleet.tmux import DirectorContext, TmuxError

_FAKE_DIRECTOR_CTX = DirectorContext(session="main", window_id="@3", pane_id="%0")
_TMUX_PANE_VALUE = "%0"


@pytest.fixture
def runner():
    return CliRunner()


@pytest.fixture
def mock_tmux_ok(monkeypatch):
    monkeypatch.setenv("TMUX", "/tmp/tmux-1000/default,12345,0")
    monkeypatch.setenv("TMUX_PANE", _TMUX_PANE_VALUE)
    monkeypatch.setattr(tmux, "ensure_tmux_available", lambda: None)
    monkeypatch.setattr(tmux, "director_context", lambda: _FAKE_DIRECTOR_CTX)


def test_doctor_text_output__text_output_has_all_four_fields(runner, mock_tmux_ok):
    result = runner.invoke(cli, ["doctor"])
    assert result.exit_code == 0, result.output
    out = result.output
    assert "session_name:" in out
    assert "main" in out
    assert "window_id:" in out
    assert "@3" in out
    assert "pane_id:" in out
    assert "%0" in out
    assert "TMUX_PANE:" in out
    assert _TMUX_PANE_VALUE in out


def test_doctor_json_output__json_output_shape(runner, mock_tmux_ok):
    result = runner.invoke(cli, ["--json", "doctor"])
    assert result.exit_code == 0, result.output
    data = json.loads(result.output)
    assert data == {
        "tmux": {
            "session_name": "main",
            "window_id": "@3",
            "pane_id": "%0",
            "tmux_pane_env": _TMUX_PANE_VALUE,
        }
    }


def test_doctor_outside_tmux__outside_tmux_exits_one(runner, monkeypatch):
    monkeypatch.delenv("TMUX", raising=False)

    def _raise():
        raise TmuxError("cafleet member commands must be run inside a tmux session")

    monkeypatch.setattr(tmux, "ensure_tmux_available", _raise)
    result = runner.invoke(cli, ["doctor"])
    assert result.exit_code == 1, result.output
    combined = (result.output or "") + (result.stderr or "")
    assert "cafleet member commands must be run inside a tmux session" in combined


def test_doctor_session_id_silently_ignored__session_id_flag_silently_ignored(
    runner, mock_tmux_ok
):
    result = runner.invoke(
        cli,
        [
            "--session-id",
            "00000000-0000-0000-0000-000000000000",
            "doctor",
        ],
    )
    assert result.exit_code == 0, result.output
    out = result.output
    assert "session_name:" in out
    assert "main" in out
    assert "window_id:" in out
    assert "@3" in out
    assert "pane_id:" in out
    assert "%0" in out
    assert "TMUX_PANE:" in out
    assert _TMUX_PANE_VALUE in out
    assert "--session-id" not in out
    assert "is required for this subcommand" not in out
