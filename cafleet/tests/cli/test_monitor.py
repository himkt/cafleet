"""CLI tests for the ``cafleet monitor`` group (§7).

``monitor start`` runs the foreground loop in-process, so the tests mock
``loop.run_monitor_loop`` at the module boundary the CLI calls through
(module-attribute access, matching the established ``broker.get_member``
convention) — the loop never actually runs.

The watched set is the root Director (180 s) + every ordinary member (720 s);
the monitoring member is the unenrolled watcher, located by kind. ``monitor
status`` lists the watched set with role ``director`` / ``member`` (never a
``monitor`` row) and renders ``last_ping`` as a human age; ``monitor start``
warns when no monitoring member is present; ``monitor config`` edits any
enrolled member and reports not-enrolled for the watcher.
"""

import json
import os
import sqlite3
from datetime import UTC, datetime, timedelta

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
    result = runner.invoke(cli, ["fleet", "create", "--name", "test-fleet", "--json"])
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


def _monitor_config_row(db_file, member_id: int):
    conn = sqlite3.connect(str(db_file))
    try:
        return conn.execute(
            "SELECT interval_seconds, enabled FROM monitor_config WHERE member_id = ?",
            (member_id,),
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


def _register_ordinary_member(
    sid: int, *, name: str = "alice", pane_id: str = "%9"
) -> dict:
    """Register a pane-bound ordinary member — a watched member enrolled @720."""
    return broker.register_member(
        fleet_id=sid,
        name=name,
        description="ordinary member",
        placement={
            "backend": "tmux",
            "mux_session": "main",
            "mux_window_id": "@3",
            "mux_pane_id": pane_id,
            "coding_agent": "claude",
        },
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


# --- monitor status --------------------------------------------------------


def test_monitor_status__running_text_shows_runtime_and_member_table(fleet):
    db_file, runner, data = fleet
    sid = data["fleet_id"]
    director_id = data["director"]["member_id"]
    _register_monitoring_member(sid)  # the watcher — NOT a table row
    member_id = _register_ordinary_member(sid)["member_id"]
    _seed_runtime(db_file, sid, os.getpid())

    result = runner.invoke(cli, ["monitor", "status", "--fleet-id", str(sid)])

    assert result.exit_code == 0, result.output
    out = result.output
    assert "running" in out.lower()
    assert str(os.getpid()) in out
    # the watched set (Director + member) appears with director / member roles
    assert str(director_id) in out
    assert str(member_id) in out
    assert "alice" in out
    assert "director" in out
    assert "member" in out
    # the monitoring member is the unenrolled watcher and is not a watched row
    assert "watcher" not in out


def test_monitor_status__json_shape(fleet):
    db_file, runner, data = fleet
    sid = data["fleet_id"]
    director_id = data["director"]["member_id"]
    watcher_id = _register_monitoring_member(sid)["member_id"]
    member_id = _register_ordinary_member(sid)["member_id"]
    _seed_runtime(db_file, sid, os.getpid())

    result = runner.invoke(cli, ["monitor", "status", "--fleet-id", str(sid), "--json"])

    assert result.exit_code == 0, result.output
    payload = json.loads(result.output)
    runtime = payload["runtime"]
    assert runtime["running"] is True
    assert runtime["pid"] == os.getpid()
    for key in ("tick_seconds", "last_tick_at", "last_tick_age_seconds", "started_at"):
        assert key in runtime

    # the watched set is the Director + member; the watcher is not enrolled
    members = {m["member_id"]: m for m in payload["members"]}
    assert set(members) == {director_id, member_id}
    assert watcher_id not in members

    assert members[director_id]["role"] == "director"
    assert members[director_id]["interval_seconds"] == 180
    assert members[member_id]["role"] == "member"
    assert members[member_id]["interval_seconds"] == 720
    for m in members.values():
        assert m["enabled"] is True
        assert m["pending_count"] == 0
        # never pinged → ISO is null and the derived age is null (parity field)
        assert m["last_ping_at"] is None
        assert m["last_ping_age_seconds"] is None
        for key in ("name", "interval_seconds", "last_ping_at"):
            assert key in m


def test_monitor_status__not_running_when_no_runtime(fleet):
    db_file, runner, data = fleet
    sid = data["fleet_id"]
    result = runner.invoke(cli, ["monitor", "status", "--fleet-id", str(sid), "--json"])

    assert result.exit_code == 0, result.output
    assert json.loads(result.output)["runtime"]["running"] is False


def test_monitor_status__labels_director_and_member_roles(fleet):
    # the table labels the watched Director as ``director`` and ordinary members
    # as ``member``; the monitoring member is the unenrolled watcher and never
    # appears as a row (no ``monitor`` role).
    db_file, runner, data = fleet
    sid = data["fleet_id"]
    director_id = data["director"]["member_id"]
    watcher = _register_monitoring_member(sid)
    member_id = _register_ordinary_member(sid)["member_id"]
    _seed_runtime(db_file, sid, os.getpid())

    result = runner.invoke(cli, ["monitor", "status", "--fleet-id", str(sid), "--json"])
    assert result.exit_code == 0, result.output
    roles = {m["member_id"]: m["role"] for m in json.loads(result.output)["members"]}
    assert roles[director_id] == "director"
    assert roles[member_id] == "member"
    assert watcher["member_id"] not in roles
    assert "monitor" not in roles.values()


def test_monitor_status__last_ping_age_rendered(fleet):
    db_file, runner, data = fleet
    sid = data["fleet_id"]
    member_id = _register_ordinary_member(sid)["member_id"]
    _seed_runtime(db_file, sid, os.getpid())
    # stamp the member's last_ping_at ~120 s in the past
    pinged = (datetime.now(UTC) - timedelta(seconds=120)).isoformat()
    broker.record_pings([member_id], pinged)

    result = runner.invoke(cli, ["monitor", "status", "--fleet-id", str(sid), "--json"])
    assert result.exit_code == 0, result.output
    members = {m["member_id"]: m for m in json.loads(result.output)["members"]}
    assert members[member_id]["last_ping_at"] == pinged
    age = members[member_id]["last_ping_age_seconds"]
    assert isinstance(age, int)
    assert 115 <= age <= 200  # ~120 s, generous bounds for test timing


def test_monitor_status__text_renders_last_ping_as_age_not_iso(fleet):
    db_file, runner, data = fleet
    sid = data["fleet_id"]
    member_id = _register_ordinary_member(sid)["member_id"]
    _seed_runtime(db_file, sid, os.getpid())
    pinged = (datetime.now(UTC) - timedelta(seconds=120)).isoformat()
    broker.record_pings([member_id], pinged)

    result = runner.invoke(cli, ["monitor", "status", "--fleet-id", str(sid)])
    assert result.exit_code == 0, result.output
    # the last-ping column shows a human age, not the raw ISO timestamp
    assert pinged not in result.output


def test_monitor_status__stale_row_reports_not_running_with_nulls(fleet):
    # a stale heartbeat (beyond STALE_AFTER) reads as not-live: the process
    # fields null out even though the row (with a real pid/started_at) exists
    db_file, runner, data = fleet
    sid = data["fleet_id"]
    stale = (datetime.now(UTC) - timedelta(hours=1)).isoformat()
    _seed_runtime(db_file, sid, os.getpid(), last_tick_at=stale)

    result = runner.invoke(cli, ["monitor", "status", "--fleet-id", str(sid), "--json"])

    assert result.exit_code == 0, result.output
    runtime = json.loads(result.output)["runtime"]
    assert runtime["running"] is False
    assert runtime["pid"] is None
    assert runtime["started_at"] is None
    assert runtime["last_tick_at"] is None
    assert runtime["tick_seconds"] == 5  # the row exists (stale, not absent)


# --- monitor config --------------------------------------------------------


def test_monitor_config__show(fleet):
    # the watched Director is enrolled @180
    db_file, runner, data = fleet
    sid = data["fleet_id"]
    director_id = data["director"]["member_id"]
    result = runner.invoke(
        cli,
        ["monitor", "config", "--fleet-id", str(sid), "--member-id", str(director_id)],
    )

    assert result.exit_code == 0, result.output
    assert "180" in result.output  # the Director's default interval


def test_monitor_config__edits_director_interval(fleet):
    db_file, runner, data = fleet
    sid = data["fleet_id"]
    director_id = data["director"]["member_id"]
    result = runner.invoke(
        cli,
        [
            "monitor",
            "config",
            "--fleet-id",
            str(sid),
            "--member-id",
            str(director_id),
            "--interval",
            "90",
        ],
    )

    assert result.exit_code == 0, result.output
    assert _monitor_config_row(db_file, director_id)[0] == 90


def test_monitor_config__edits_member_interval(fleet):
    db_file, runner, data = fleet
    sid = data["fleet_id"]
    member_id = _register_ordinary_member(sid)["member_id"]
    result = runner.invoke(
        cli,
        [
            "monitor",
            "config",
            "--fleet-id",
            str(sid),
            "--member-id",
            str(member_id),
            "--interval",
            "300",
        ],
    )

    assert result.exit_code == 0, result.output
    assert _monitor_config_row(db_file, member_id)[0] == 300


def test_monitor_config__disable_then_enable(fleet):
    db_file, runner, data = fleet
    sid = data["fleet_id"]
    member_id = _register_ordinary_member(sid)["member_id"]

    disabled = runner.invoke(
        cli,
        [
            "monitor",
            "config",
            "--fleet-id",
            str(sid),
            "--member-id",
            str(member_id),
            "--disable",
        ],
    )
    assert disabled.exit_code == 0, disabled.output
    assert _monitor_config_row(db_file, member_id)[1] == 0

    enabled = runner.invoke(
        cli,
        [
            "monitor",
            "config",
            "--fleet-id",
            str(sid),
            "--member-id",
            str(member_id),
            "--enable",
        ],
    )
    assert enabled.exit_code == 0, enabled.output
    assert _monitor_config_row(db_file, member_id)[1] == 1


def test_monitor_config__mutual_exclusion_exits_two(fleet):
    # the mutual-exclusion guard fires before any enrollment lookup, so the
    # target member need not be enrolled
    db_file, runner, data = fleet
    sid = data["fleet_id"]
    director_id = data["director"]["member_id"]
    result = runner.invoke(
        cli,
        [
            "monitor",
            "config",
            "--fleet-id",
            str(sid),
            "--member-id",
            str(director_id),
            "--enable",
            "--disable",
        ],
    )

    assert result.exit_code == 2, result.output
    # a real usage error, not the group's "no such command" during the red phase
    assert "no such command" not in result.output.lower()


def test_monitor_config__administrator_not_enrolled_exits_one(fleet):
    db_file, runner, data = fleet
    sid = data["fleet_id"]
    admin_id = data["administrator_member_id"]
    result = runner.invoke(
        cli, ["monitor", "config", "--fleet-id", str(sid), "--member-id", str(admin_id)]
    )

    assert result.exit_code == 1, result.output


def test_monitor_config__monitoring_member_reports_not_enrolled(fleet):
    # the monitoring member is the unenrolled watcher → config reports not-enrolled
    db_file, runner, data = fleet
    sid = data["fleet_id"]
    watcher_id = _register_monitoring_member(sid)["member_id"]
    result = runner.invoke(
        cli,
        ["monitor", "config", "--fleet-id", str(sid), "--member-id", str(watcher_id)],
    )

    assert result.exit_code == 1, result.output
    assert "not enrolled" in result.output.lower()
