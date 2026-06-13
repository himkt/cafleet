"""CLI tests for the ``cafleet monitor`` group (§8).

The process layer is mocked at the module boundary the CLI is expected to call
through (``process.start_detached`` / ``process.stop_monitor`` /
``loop.run_monitor_loop`` — module-attribute access, matching the established
``broker.get_agent`` convention), so these tests never spawn a real worker.
"""

import json
import os
import sqlite3
from datetime import UTC, datetime

import click
import pytest
from click.testing import CliRunner

from cafleet import config
from cafleet.cli import cli
from cafleet.monitor import DEFAULT_TICK_SECONDS, loop, process
from cafleet.monitor.process import StartResult, StopResult
from cafleet.multiplexer import MultiplexerContext as DirectorContext


@pytest.fixture(autouse=True)
def _autouse_reset_engine(_reset_engine_singletons):
    pass


@pytest.fixture(autouse=True)
def _mock_tmux_for_fleet_create(monkeypatch):
    ctx = DirectorContext(session="main", window_id="@3", pane_id="%0")
    monkeypatch.setattr(
        "cafleet.multiplexer.tmux.TmuxMultiplexer.ensure_available", lambda self: None
    )
    monkeypatch.setattr(
        "cafleet.multiplexer.tmux.TmuxMultiplexer.context_discovery", lambda self: ctx
    )


@pytest.fixture
def fresh_db(tmp_path, monkeypatch):
    db_file = tmp_path / "cafleet.db"
    monkeypatch.setattr(
        config.settings, "database_url", f"sqlite+aiosqlite:///{db_file}"
    )
    monkeypatch.setattr(config.settings, "monitor_state_dir", tmp_path / "monitor")
    runner = CliRunner()
    result = runner.invoke(cli, ["db", "init"])
    assert result.exit_code == 0, result.output
    return db_file, runner


@pytest.fixture
def fleet(fresh_db):
    db_file, runner = fresh_db
    result = runner.invoke(cli, ["fleet", "create", "--json"])
    assert result.exit_code == 0, result.output
    return db_file, runner, json.loads(result.output)


def _seed_runtime(db_file, fleet_id: int, pid: int, *, tick: int = 5) -> None:
    now = datetime.now(UTC).isoformat()
    conn = sqlite3.connect(str(db_file))
    try:
        conn.execute(
            "INSERT INTO monitor_runtime "
            "(fleet_id, pid, started_at, last_tick_at, tick_seconds) "
            "VALUES (?, ?, ?, ?, ?)",
            (fleet_id, pid, now, now, tick),
        )
        conn.commit()
    finally:
        conn.close()


def _monitor_config_row(db_file, agent_id: int):
    conn = sqlite3.connect(str(db_file))
    try:
        return conn.execute(
            "SELECT interval_seconds, enabled FROM monitor_config WHERE agent_id = ?",
            (agent_id,),
        ).fetchone()
    finally:
        conn.close()


def _soft_delete_fleet(db_file, fleet_id: int) -> None:
    conn = sqlite3.connect(str(db_file))
    try:
        conn.execute(
            "UPDATE fleets SET deleted_at = ? WHERE fleet_id = ?",
            ("2026-01-01T00:00:00+00:00", fleet_id),
        )
        conn.commit()
    finally:
        conn.close()


# --- monitor start ---------------------------------------------------------


def test_monitor_start__default_spawns_detached_and_reports_started(fleet, monkeypatch):
    db_file, runner, data = fleet
    sid = data["fleet_id"]
    calls = []

    def fake_start(fleet_id, tick_seconds):
        calls.append((fleet_id, tick_seconds))
        return StartResult(
            ok=True,
            pid=4821,
            tick_seconds=tick_seconds,
            log_path=None,
            message=f"monitor started (pid 4821, tick {tick_seconds}s)",
        )

    monkeypatch.setattr(process, "start_detached", fake_start)
    result = runner.invoke(cli, ["--fleet-id", str(sid), "monitor", "start"])

    assert result.exit_code == 0, result.output
    assert calls == [(sid, DEFAULT_TICK_SECONDS)]
    assert "started" in result.output.lower()


def test_monitor_start__foreground_invokes_run_monitor_loop(fleet, monkeypatch):
    db_file, runner, data = fleet
    sid = data["fleet_id"]
    calls = []
    monkeypatch.setattr(
        loop,
        "run_monitor_loop",
        lambda fleet_id, tick_seconds: calls.append((fleet_id, tick_seconds)),
    )
    result = runner.invoke(
        cli, ["--fleet-id", str(sid), "monitor", "start", "--foreground", "--tick", "3"]
    )

    assert result.exit_code == 0, result.output
    assert calls == [(sid, 3)]


def test_monitor_start__already_running_exits_one(fleet, monkeypatch):
    db_file, runner, data = fleet
    sid = data["fleet_id"]

    def fake_start(fleet_id, tick_seconds):
        raise click.ClickException(
            f"monitor already running for fleet {fleet_id} (pid 4821)"
        )

    monkeypatch.setattr(process, "start_detached", fake_start)
    result = runner.invoke(cli, ["--fleet-id", str(sid), "monitor", "start"])

    assert result.exit_code == 1, result.output
    assert "already running" in result.output.lower()


def test_monitor_start__unknown_fleet_exits_one(fresh_db, monkeypatch):
    db_file, runner = fresh_db
    calls = []
    monkeypatch.setattr(process, "start_detached", lambda *a, **k: calls.append(a))
    result = runner.invoke(cli, ["--fleet-id", "999999", "monitor", "start"])

    assert result.exit_code == 1, result.output
    assert calls == []  # fleet validation fails before any spawn


def test_monitor_start__soft_deleted_fleet_exits_one(fleet, monkeypatch):
    db_file, runner, data = fleet
    sid = data["fleet_id"]
    _soft_delete_fleet(db_file, sid)
    calls = []
    monkeypatch.setattr(process, "start_detached", lambda *a, **k: calls.append(a))
    result = runner.invoke(cli, ["--fleet-id", str(sid), "monitor", "start"])

    assert result.exit_code == 1, result.output
    assert calls == []


# --- monitor stop ----------------------------------------------------------


def test_monitor_stop__reports_stopped(fleet, monkeypatch):
    db_file, runner, data = fleet
    sid = data["fleet_id"]
    monkeypatch.setattr(
        process,
        "stop_monitor",
        lambda fleet_id: StopResult(
            ok=True, stopped=True, pid=4821, message="monitor stopped (pid 4821)"
        ),
    )
    result = runner.invoke(cli, ["--fleet-id", str(sid), "monitor", "stop"])

    assert result.exit_code == 0, result.output
    assert "stopped" in result.output.lower()


def test_monitor_stop__reports_nothing_running(fleet, monkeypatch):
    db_file, runner, data = fleet
    sid = data["fleet_id"]
    monkeypatch.setattr(
        process,
        "stop_monitor",
        lambda fleet_id: StopResult(
            ok=True,
            stopped=False,
            pid=None,
            message=f"no monitor running for fleet {fleet_id}",
        ),
    )
    result = runner.invoke(cli, ["--fleet-id", str(sid), "monitor", "stop"])

    assert result.exit_code == 0, result.output
    assert "no monitor running" in result.output.lower()


# --- monitor status --------------------------------------------------------


def test_monitor_status__running_text_shows_runtime_and_agent_table(fleet):
    db_file, runner, data = fleet
    sid = data["fleet_id"]
    _seed_runtime(db_file, sid, os.getpid())

    result = runner.invoke(cli, ["--fleet-id", str(sid), "monitor", "status"])

    assert result.exit_code == 0, result.output
    out = result.output.lower()
    assert "running" in out
    assert str(os.getpid()) in result.output
    assert "Director" in result.output  # the enrolled root director row


def test_monitor_status__json_shape(fleet):
    db_file, runner, data = fleet
    sid = data["fleet_id"]
    director_id = data["director"]["agent_id"]
    _seed_runtime(db_file, sid, os.getpid())

    result = runner.invoke(cli, ["--fleet-id", str(sid), "--json", "monitor", "status"])

    assert result.exit_code == 0, result.output
    payload = json.loads(result.output)
    runtime = payload["runtime"]
    assert runtime["running"] is True
    assert runtime["pid"] == os.getpid()
    for key in ("tick_seconds", "last_tick_at", "last_tick_age_seconds", "started_at"):
        assert key in runtime

    director = next(a for a in payload["agents"] if a["agent_id"] == director_id)
    assert director["role"] == "director"
    assert director["enabled"] is True
    assert director["pending_count"] == 0
    for key in ("name", "interval_seconds", "last_ping_at"):
        assert key in director


def test_monitor_status__not_running_when_no_runtime(fleet):
    db_file, runner, data = fleet
    sid = data["fleet_id"]
    result = runner.invoke(cli, ["--fleet-id", str(sid), "--json", "monitor", "status"])

    assert result.exit_code == 0, result.output
    assert json.loads(result.output)["runtime"]["running"] is False


# --- monitor config --------------------------------------------------------


def test_monitor_config__show(fleet):
    db_file, runner, data = fleet
    sid = data["fleet_id"]
    director_id = data["director"]["agent_id"]
    result = runner.invoke(
        cli,
        ["--fleet-id", str(sid), "monitor", "config", "--agent-id", str(director_id)],
    )

    assert result.exit_code == 0, result.output
    assert "60" in result.output  # the default interval


def test_monitor_config__set_interval_persists(fleet):
    db_file, runner, data = fleet
    sid = data["fleet_id"]
    director_id = data["director"]["agent_id"]
    result = runner.invoke(
        cli,
        [
            "--fleet-id",
            str(sid),
            "monitor",
            "config",
            "--agent-id",
            str(director_id),
            "--interval",
            "30",
        ],
    )

    assert result.exit_code == 0, result.output
    assert _monitor_config_row(db_file, director_id)[0] == 30


def test_monitor_config__disable_then_enable(fleet):
    db_file, runner, data = fleet
    sid = data["fleet_id"]
    director_id = data["director"]["agent_id"]

    disabled = runner.invoke(
        cli,
        [
            "--fleet-id",
            str(sid),
            "monitor",
            "config",
            "--agent-id",
            str(director_id),
            "--disable",
        ],
    )
    assert disabled.exit_code == 0, disabled.output
    assert _monitor_config_row(db_file, director_id)[1] == 0

    enabled = runner.invoke(
        cli,
        [
            "--fleet-id",
            str(sid),
            "monitor",
            "config",
            "--agent-id",
            str(director_id),
            "--enable",
        ],
    )
    assert enabled.exit_code == 0, enabled.output
    assert _monitor_config_row(db_file, director_id)[1] == 1


def test_monitor_config__mutual_exclusion_exits_two(fleet):
    db_file, runner, data = fleet
    sid = data["fleet_id"]
    director_id = data["director"]["agent_id"]
    result = runner.invoke(
        cli,
        [
            "--fleet-id",
            str(sid),
            "monitor",
            "config",
            "--agent-id",
            str(director_id),
            "--enable",
            "--disable",
        ],
    )

    assert result.exit_code == 2, result.output
    # a real usage error, not the group's "no such command" during the red phase
    assert "no such command" not in result.output.lower()


def test_monitor_config__not_enrolled_exits_one(fleet):
    db_file, runner, data = fleet
    sid = data["fleet_id"]
    admin_id = data["administrator_agent_id"]
    result = runner.invoke(
        cli, ["--fleet-id", str(sid), "monitor", "config", "--agent-id", str(admin_id)]
    )

    assert result.exit_code == 1, result.output


# --- fleet delete teardown -------------------------------------------------


def test_fleet_delete__calls_stop_monitor(fleet, monkeypatch):
    db_file, runner, data = fleet
    sid = data["fleet_id"]
    calls = []
    monkeypatch.setattr(
        process,
        "stop_monitor",
        lambda fleet_id: (
            calls.append(fleet_id)
            or StopResult(ok=True, stopped=False, pid=None, message="x")
        ),
    )
    result = runner.invoke(cli, ["fleet", "delete", str(sid)])

    assert result.exit_code == 0, result.output
    assert calls == [sid]
