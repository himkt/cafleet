"""CLI tests for the ``cafleet monitor`` group (§8).

``monitor start`` runs the foreground loop in-process, so the tests mock
``loop.run_monitor_loop`` at the module boundary the CLI calls through
(module-attribute access, matching the established ``broker.get_agent``
convention) — the loop never actually runs.
"""

import json
import os
import sqlite3
from datetime import UTC, datetime, timedelta

import click
import pytest
from click.testing import CliRunner

from cafleet import broker, config
from cafleet.cli import cli
from cafleet.monitor import DEFAULT_TICK_SECONDS, loop
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


def _seed_runtime(
    db_file, fleet_id: int, pid: int, *, tick: int = 5, last_tick_at: str | None = None
) -> None:
    now = datetime.now(UTC).isoformat()
    last_tick = last_tick_at if last_tick_at is not None else now
    conn = sqlite3.connect(str(db_file))
    try:
        conn.execute(
            "INSERT INTO monitor_runtime "
            "(fleet_id, pid, started_at, last_tick_at, tick_seconds) "
            "VALUES (?, ?, ?, ?, ?)",
            (fleet_id, pid, now, last_tick, tick),
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


def _register_monitoring_member(
    sid: int, director_id: int, *, name: str = "watcher", pane_id: str = "%7"
) -> dict:
    """Register the dedicated monitoring member — the only enrolled role after
    design 0000091 (the ``member create --role monitor`` CLI path is exercised
    in test_member.py)."""
    return broker.register_agent(
        fleet_id=sid,
        name=name,
        description="monitoring member",
        placement={
            "director_agent_id": director_id,
            "tmux_session": "main",
            "tmux_window_id": "@3",
            "tmux_pane_id": pane_id,
            "coding_agent": "claude",
        },
        kind="monitoring-member",
    )


# --- monitor start ---------------------------------------------------------


def test_monitor_start__runs_loop_in_process_with_default_tick(fleet, monkeypatch):
    db_file, runner, data = fleet
    sid = data["fleet_id"]
    calls = []
    monkeypatch.setattr(
        loop,
        "run_monitor_loop",
        lambda fleet_id, tick_seconds: calls.append((fleet_id, tick_seconds)),
    )
    result = runner.invoke(cli, ["monitor", "start", "--fleet-id", str(sid)])

    assert result.exit_code == 0, result.output
    assert calls == [(sid, DEFAULT_TICK_SECONDS)]


def test_monitor_start__passes_tick_through(fleet, monkeypatch):
    db_file, runner, data = fleet
    sid = data["fleet_id"]
    calls = []
    monkeypatch.setattr(
        loop,
        "run_monitor_loop",
        lambda fleet_id, tick_seconds: calls.append((fleet_id, tick_seconds)),
    )
    result = runner.invoke(
        cli, ["monitor", "start", "--fleet-id", str(sid), "--tick", "3"]
    )

    assert result.exit_code == 0, result.output
    assert calls == [(sid, 3)]


def test_monitor_start__already_running_exits_one(fleet, monkeypatch):
    db_file, runner, data = fleet
    sid = data["fleet_id"]

    def boom(fleet_id, tick_seconds):
        raise click.ClickException(f"monitor already running for fleet {fleet_id}")

    monkeypatch.setattr(loop, "run_monitor_loop", boom)
    result = runner.invoke(cli, ["monitor", "start", "--fleet-id", str(sid)])

    assert result.exit_code == 1, result.output
    assert "already running" in result.output.lower()


def test_monitor_start__unknown_fleet_exits_one(fresh_db, monkeypatch):
    db_file, runner = fresh_db
    calls = []
    monkeypatch.setattr(loop, "run_monitor_loop", lambda *a, **k: calls.append(a))
    result = runner.invoke(cli, ["monitor", "start", "--fleet-id", "999999"])

    assert result.exit_code == 1, result.output
    assert calls == []  # fleet validation fails before the loop runs


def test_monitor_start__soft_deleted_fleet_exits_one(fleet, monkeypatch):
    db_file, runner, data = fleet
    sid = data["fleet_id"]
    _soft_delete_fleet(db_file, sid)
    calls = []
    monkeypatch.setattr(loop, "run_monitor_loop", lambda *a, **k: calls.append(a))
    result = runner.invoke(cli, ["monitor", "start", "--fleet-id", str(sid)])

    assert result.exit_code == 1, result.output
    assert calls == []


def test_monitor_start__warns_when_no_monitoring_member_but_still_runs(
    fleet, monkeypatch
):
    # §4 / Q3: warn-but-run — a fleet with no enrolled monitoring member prints a
    # startup warning to stderr, then runs the loop unchanged.
    db_file, runner, data = fleet
    sid = data["fleet_id"]
    calls = []
    monkeypatch.setattr(
        loop,
        "run_monitor_loop",
        lambda fleet_id, tick_seconds: calls.append((fleet_id, tick_seconds)),
    )
    result = runner.invoke(cli, ["monitor", "start", "--fleet-id", str(sid)])

    assert result.exit_code == 0, result.output
    assert "no enrolled monitoring member" in result.stderr
    assert calls == [(sid, DEFAULT_TICK_SECONDS)]  # the loop still runs


def test_monitor_start__no_warning_when_monitoring_member_enrolled(fleet, monkeypatch):
    db_file, runner, data = fleet
    sid = data["fleet_id"]
    director_id = data["director"]["agent_id"]
    _register_monitoring_member(sid, director_id)
    calls = []
    monkeypatch.setattr(
        loop,
        "run_monitor_loop",
        lambda fleet_id, tick_seconds: calls.append((fleet_id, tick_seconds)),
    )
    result = runner.invoke(cli, ["monitor", "start", "--fleet-id", str(sid)])

    assert result.exit_code == 0, result.output
    assert "no enrolled monitoring member" not in result.stderr
    assert calls == [(sid, DEFAULT_TICK_SECONDS)]


# --- monitor status --------------------------------------------------------


def test_monitor_status__running_text_shows_runtime_and_agent_table(fleet):
    db_file, runner, data = fleet
    sid = data["fleet_id"]
    director_id = data["director"]["agent_id"]
    _register_monitoring_member(sid, director_id)
    _seed_runtime(db_file, sid, os.getpid())

    result = runner.invoke(cli, ["monitor", "status", "--fleet-id", str(sid)])

    assert result.exit_code == 0, result.output
    out = result.output.lower()
    assert "running" in out
    assert str(os.getpid()) in result.output
    assert "watcher" in result.output  # the enrolled monitoring member row
    assert "Director" not in result.output  # the Director is no longer enrolled


def test_monitor_status__json_shape(fleet):
    db_file, runner, data = fleet
    sid = data["fleet_id"]
    director_id = data["director"]["agent_id"]
    watcher_id = _register_monitoring_member(sid, director_id)["agent_id"]
    _seed_runtime(db_file, sid, os.getpid())

    result = runner.invoke(cli, ["--json", "monitor", "status", "--fleet-id", str(sid)])

    assert result.exit_code == 0, result.output
    payload = json.loads(result.output)
    runtime = payload["runtime"]
    assert runtime["running"] is True
    assert runtime["pid"] == os.getpid()
    for key in ("tick_seconds", "last_tick_at", "last_tick_age_seconds", "started_at"):
        assert key in runtime

    # only the monitoring member appears; the Director is no longer enrolled
    agent_ids = {a["agent_id"] for a in payload["agents"]}
    assert agent_ids == {watcher_id}
    watcher = next(a for a in payload["agents"] if a["agent_id"] == watcher_id)
    assert watcher["role"] == "monitor"
    assert watcher["enabled"] is True
    assert watcher["pending_count"] == 0
    for key in ("name", "interval_seconds", "last_ping_at"):
        assert key in watcher


def test_monitor_status__not_running_when_no_runtime(fleet):
    db_file, runner, data = fleet
    sid = data["fleet_id"]
    result = runner.invoke(cli, ["--json", "monitor", "status", "--fleet-id", str(sid)])

    assert result.exit_code == 0, result.output
    assert json.loads(result.output)["runtime"]["running"] is False


def test_monitor_status__labels_monitoring_member_role(fleet):
    # the agents table labels the dedicated monitoring member's role as
    # ``monitor`` (derived from is_monitoring_member). After design 0000091 the
    # Director is no longer enrolled, so it does not appear in the table at all.
    db_file, runner, data = fleet
    sid = data["fleet_id"]
    director_id = data["director"]["agent_id"]
    watcher = _register_monitoring_member(sid, director_id)
    _seed_runtime(db_file, sid, os.getpid())

    result = runner.invoke(cli, ["--json", "monitor", "status", "--fleet-id", str(sid)])
    assert result.exit_code == 0, result.output
    roles = {a["agent_id"]: a["role"] for a in json.loads(result.output)["agents"]}
    assert director_id not in roles
    assert roles[watcher["agent_id"]] == "monitor"


def test_monitor_status__stale_row_reports_not_running_with_nulls(fleet):
    # a stale heartbeat (beyond STALE_AFTER) reads as not-live: the process
    # fields null out even though the row (with a real pid/started_at) exists
    db_file, runner, data = fleet
    sid = data["fleet_id"]
    stale = (datetime.now(UTC) - timedelta(hours=1)).isoformat()
    _seed_runtime(db_file, sid, os.getpid(), last_tick_at=stale)

    result = runner.invoke(cli, ["--json", "monitor", "status", "--fleet-id", str(sid)])

    assert result.exit_code == 0, result.output
    runtime = json.loads(result.output)["runtime"]
    assert runtime["running"] is False
    assert runtime["pid"] is None
    assert runtime["started_at"] is None
    assert runtime["last_tick_at"] is None
    assert runtime["tick_seconds"] == 5  # the row exists (stale, not absent)


# --- monitor config --------------------------------------------------------


def test_monitor_config__show(fleet):
    # the monitoring member is the only enrolled role (design 0000091)
    db_file, runner, data = fleet
    sid = data["fleet_id"]
    director_id = data["director"]["agent_id"]
    watcher_id = _register_monitoring_member(sid, director_id)["agent_id"]
    result = runner.invoke(
        cli,
        ["monitor", "config", "--fleet-id", str(sid), "--agent-id", str(watcher_id)],
    )

    assert result.exit_code == 0, result.output
    assert "60" in result.output  # the default interval


def test_monitor_config__set_interval_persists(fleet):
    db_file, runner, data = fleet
    sid = data["fleet_id"]
    director_id = data["director"]["agent_id"]
    watcher_id = _register_monitoring_member(sid, director_id)["agent_id"]
    result = runner.invoke(
        cli,
        [
            "monitor",
            "config",
            "--fleet-id",
            str(sid),
            "--agent-id",
            str(watcher_id),
            "--interval",
            "30",
        ],
    )

    assert result.exit_code == 0, result.output
    assert _monitor_config_row(db_file, watcher_id)[0] == 30


def test_monitor_config__disable_then_enable(fleet):
    db_file, runner, data = fleet
    sid = data["fleet_id"]
    director_id = data["director"]["agent_id"]
    watcher_id = _register_monitoring_member(sid, director_id)["agent_id"]

    disabled = runner.invoke(
        cli,
        [
            "monitor",
            "config",
            "--fleet-id",
            str(sid),
            "--agent-id",
            str(watcher_id),
            "--disable",
        ],
    )
    assert disabled.exit_code == 0, disabled.output
    assert _monitor_config_row(db_file, watcher_id)[1] == 0

    enabled = runner.invoke(
        cli,
        [
            "monitor",
            "config",
            "--fleet-id",
            str(sid),
            "--agent-id",
            str(watcher_id),
            "--enable",
        ],
    )
    assert enabled.exit_code == 0, enabled.output
    assert _monitor_config_row(db_file, watcher_id)[1] == 1


def test_monitor_config__mutual_exclusion_exits_two(fleet):
    # the mutual-exclusion guard fires before any enrollment lookup, so the
    # target agent need not be enrolled
    db_file, runner, data = fleet
    sid = data["fleet_id"]
    director_id = data["director"]["agent_id"]
    result = runner.invoke(
        cli,
        [
            "monitor",
            "config",
            "--fleet-id",
            str(sid),
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
        cli, ["monitor", "config", "--fleet-id", str(sid), "--agent-id", str(admin_id)]
    )

    assert result.exit_code == 1, result.output
