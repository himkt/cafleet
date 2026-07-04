"""CLI tests for the ``cafleet doctor`` subcommand."""

import importlib.metadata
import json
import re
import sqlite3

import pytest
from click.testing import CliRunner

from cafleet import config
from cafleet.cli import cli
from cafleet.multiplexer.tmux import TmuxError, TmuxMultiplexer

_TMUX_PANE_VALUE = "%0"

RUNTIME_VERSION = importlib.metadata.version("cafleet")
TS_CURRENT = "2026-07-04T00:12:09.123456+00:00"
TS_STALE = "2026-06-20T10:00:00.987654+00:00"
NO_INSTALL_LINE = "(no skills install recorded; run 'cafleet setup')"


@pytest.fixture
def runner():
    return CliRunner()


@pytest.fixture(autouse=True)
def registry_db(tmp_path, monkeypatch):
    """Redirect the registry at a temp SQLite so no test touches the real DB."""
    db_path = tmp_path / "registry" / "cafleet.db"
    monkeypatch.setattr(
        config.settings, "database_url", f"sqlite+aiosqlite:///{db_path}"
    )
    return db_path


@pytest.fixture
def mock_tmux_ok(monkeypatch, _mock_tmux_for_fleet_create):
    monkeypatch.setenv("TMUX", "/tmp/tmux-1000/default,12345,0")
    monkeypatch.setenv("TMUX_PANE", _TMUX_PANE_VALUE)


def _init_schema():
    from cafleet.db.init import run_db_init

    run_db_init()


def _seed_install(db_path, coding_agent, cafleet_version, installed_at):
    conn = sqlite3.connect(str(db_path))
    try:
        conn.execute(
            "INSERT OR REPLACE INTO skill_installs"
            " (coding_agent, cafleet_version, installed_at) VALUES (?, ?, ?)",
            (coding_agent, cafleet_version, installed_at),
        )
        conn.commit()
    finally:
        conn.close()


# --------------------------------------------------------------------------- #
# tmux block (unchanged surface)                                               #
# --------------------------------------------------------------------------- #


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


def test_doctor_outside_tmux__outside_tmux_exits_one(runner, monkeypatch):
    monkeypatch.delenv("TMUX", raising=False)

    def _raise(self):
        raise TmuxError("cafleet member commands must be run inside a tmux session")

    monkeypatch.setattr(TmuxMultiplexer, "ensure_available", _raise)
    result = runner.invoke(cli, ["doctor"])
    assert result.exit_code == 1, result.output
    combined = (result.output or "") + (result.stderr or "")
    assert "cafleet member commands must be run inside a tmux session" in combined


def test_doctor_rejects_fleet_id__rejected_in_both_positions(runner):
    """``doctor`` does not accept ``--fleet-id`` in either position — Click
    rejects it with its standard 'no such option' error (exit 2)."""
    global_pos = runner.invoke(cli, ["--fleet-id", "100", "doctor"])
    assert global_pos.exit_code == 2, global_pos.output
    assert "no such option" in (global_pos.output or "").lower()

    per_subcommand = runner.invoke(cli, ["doctor", "--fleet-id", "100"])
    assert per_subcommand.exit_code == 2, per_subcommand.output
    assert "no such option" in (per_subcommand.output or "").lower()


# --------------------------------------------------------------------------- #
# skills block — text form                                                     #
# --------------------------------------------------------------------------- #


def test_doctor_text_skills_section_ok_and_stale(runner, mock_tmux_ok, registry_db):
    """Every row is reported with its verbatim timestamp and ok/STALE verdict;
    a stale row never blocks ``doctor``."""
    _init_schema()
    _seed_install(registry_db, "claude", RUNTIME_VERSION, TS_CURRENT)
    _seed_install(registry_db, "codex", "0.0.1", TS_STALE)

    result = runner.invoke(cli, ["doctor"])

    assert result.exit_code == 0, result.output
    out = result.output
    assert "skills:" in out
    assert f"cli_version: {RUNTIME_VERSION}" in out
    assert re.search(
        rf"claude:\s+{re.escape(RUNTIME_VERSION)} \({re.escape(TS_CURRENT)}\) ok",
        out,
    ), out
    assert re.search(
        rf"codex:\s+0\.0\.1 \({re.escape(TS_STALE)}\) STALE",
        out,
    ), out


def test_doctor_text_skills_after_tmux_block(runner, mock_tmux_ok, registry_db):
    """The skills block is printed after the existing tmux block."""
    _init_schema()
    _seed_install(registry_db, "claude", RUNTIME_VERSION, TS_CURRENT)

    result = runner.invoke(cli, ["doctor"])

    assert result.exit_code == 0, result.output
    assert result.output.index("tmux:") < result.output.index("skills:")


@pytest.mark.parametrize("state", ["no-db-file", "empty-table"])
def test_doctor_text_no_install_recorded(runner, mock_tmux_ok, registry_db, state):
    """No rows / table missing yields the two-line no-install skills block."""
    if state == "empty-table":
        _init_schema()

    result = runner.invoke(cli, ["doctor"])

    assert result.exit_code == 0, result.output
    assert f"skills:\n  {NO_INSTALL_LINE}" in result.output


# --------------------------------------------------------------------------- #
# skills block — JSON form                                                     #
# --------------------------------------------------------------------------- #


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
        },
        "skills": {
            "cli_version": RUNTIME_VERSION,
            "installs": [],
        },
    }


def test_doctor_json_skills_installs(runner, mock_tmux_ok, registry_db):
    """Each row appears with its ``current`` verdict, ordered by coding_agent."""
    _init_schema()
    _seed_install(registry_db, "codex", "0.0.1", TS_STALE)
    _seed_install(registry_db, "claude", RUNTIME_VERSION, TS_CURRENT)

    result = runner.invoke(cli, ["--json", "doctor"])

    assert result.exit_code == 0, result.output
    data = json.loads(result.output)
    assert data["skills"] == {
        "cli_version": RUNTIME_VERSION,
        "installs": [
            {
                "coding_agent": "claude",
                "cafleet_version": RUNTIME_VERSION,
                "installed_at": TS_CURRENT,
                "current": True,
            },
            {
                "coding_agent": "codex",
                "cafleet_version": "0.0.1",
                "installed_at": TS_STALE,
                "current": False,
            },
        ],
    }
