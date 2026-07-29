"""CLI tests for the ``cafleet monitor`` group.

``monitor start`` runs the foreground loop in-process, so the tests mock
``loop.run_monitor_loop`` at the module boundary the CLI calls through
(module-attribute access, matching the established ``broker.get_member``
convention) — the loop never actually runs.

The monitor group is exactly the monitoring toolkit: the loop (``start``) and
its read primitive (``capture``). ``monitor start`` warns when no monitoring
member is present but still runs the loop.
"""

import json
import sqlite3

import click
import pytest
from click.testing import CliRunner

from cafleet import broker
from cafleet.cli import cli
from cafleet.monitor import DEFAULT_TICK_SECONDS, loop


@pytest.fixture
def fresh_db(_cli_registry):
    """The autouse ``_cli_registry`` seeds a fresh temp DB; expose its path + a runner."""
    return _cli_registry, CliRunner()


@pytest.fixture
def fleet(fresh_db, _mock_tmux_for_fleet_create):
    db_file, runner = fresh_db
    result = runner.invoke(
        cli,
        [
            "fleet",
            "create",
            "--name",
            "test-fleet",
            "--coding-agent",
            "claude",
            "--json",
        ],
    )
    assert result.exit_code == 0, result.output
    return db_file, runner, json.loads(result.output)


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
    sid: int, *, name: str = "watcher", pane_id: str = "%7"
) -> dict:
    """Register the dedicated monitoring member — the unenrolled watcher located
    by kind (the ``member create --role monitor`` CLI path is exercised in
    test_member.py)."""
    return broker.register_member(
        fleet_id=sid,
        name=name,
        description="monitoring member",
        placement={
            "backend": "tmux",
            "mux_session": "main",
            "mux_window_id": "@3",
            "mux_pane_id": pane_id,
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
    # warn-but-run: a fleet with no monitoring member (find_monitoring_member is
    # None) prints a startup warning to stderr, then runs the loop unchanged.
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
    # combine stdout+stderr so the assertion is robust to Click's stderr
    # capture mode (the established tests/cli pattern)
    combined = (result.output or "") + (getattr(result, "stderr", "") or "")
    assert "no monitoring member" in combined
    assert calls == [(sid, DEFAULT_TICK_SECONDS)]  # the loop still runs


def test_monitor_start__no_warning_when_monitoring_member_present(fleet, monkeypatch):
    db_file, runner, data = fleet
    sid = data["fleet_id"]
    _register_monitoring_member(sid)
    calls = []
    monkeypatch.setattr(
        loop,
        "run_monitor_loop",
        lambda fleet_id, tick_seconds: calls.append((fleet_id, tick_seconds)),
    )
    result = runner.invoke(cli, ["monitor", "start", "--fleet-id", str(sid)])

    assert result.exit_code == 0, result.output
    combined = (result.output or "") + (getattr(result, "stderr", "") or "")
    assert "no monitoring member" not in combined
    assert calls == [(sid, DEFAULT_TICK_SECONDS)]


# --- group shape -------------------------------------------------------------


def test_monitor_group_has_exactly_start_and_capture():
    """The monitor group is the loop and its read primitive — nothing else."""
    assert set(cli.commands["monitor"].commands) == {"start", "capture"}


def test_member_group_no_longer_has_capture():
    assert "capture" not in cli.commands["member"].commands
