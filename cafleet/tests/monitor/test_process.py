"""Tests for the detached-process launcher + stopper (§6–8).

These mock the OS boundary (``subprocess.Popen``, ``os.kill``) on the
``cafleet.monitor.process`` module namespace, so the module is expected to
``import subprocess`` / ``import os`` (module-level) and to read
``settings.monitor_state_dir`` at call time.
"""

import os
import signal
import sys
from datetime import UTC, datetime
from types import SimpleNamespace

import click
import pytest

from cafleet import broker, config
from cafleet.monitor import process
from tests.broker._helpers import _create_fleet


@pytest.fixture(autouse=True)
def _autouse_broker(broker_session):
    return broker_session


@pytest.fixture
def state_dir(tmp_path, monkeypatch):
    monkeypatch.setattr(config.settings, "monitor_state_dir", tmp_path)
    monkeypatch.setenv("TMUX", "fake")
    return tmp_path


def _iso_now() -> str:
    return datetime.now(UTC).isoformat()


def test_start_detached__spawns_detached_worker_with_expected_argv(
    state_dir, monkeypatch
):
    sid = _create_fleet()["fleet_id"]
    child_pid = 654321
    captured = {}

    def fake_popen(argv, **kwargs):
        captured["argv"] = list(argv)
        captured["kwargs"] = kwargs
        # simulate the detached worker claiming the runtime + heartbeating
        broker.claim_monitor_runtime(sid, child_pid, 7, _iso_now())
        return SimpleNamespace(pid=child_pid, poll=lambda: None)

    monkeypatch.setattr(process.subprocess, "Popen", fake_popen)
    monkeypatch.setattr(process.os, "kill", lambda pid, sig: None)

    result = process.start_detached(sid, 7)

    argv = captured["argv"]
    assert argv[0] == sys.executable
    assert argv[1:3] == ["-m", "cafleet"]
    assert "--foreground" in argv
    assert "monitor" in argv
    assert "start" in argv
    assert "--fleet-id" in argv
    assert str(sid) in argv
    assert "--tick" in argv
    assert "7" in argv
    assert captured["kwargs"].get("start_new_session") is True
    # reports the spawned child pid on success
    assert str(child_pid) in str(result)


def test_start_detached__refuses_when_runtime_live(state_dir, monkeypatch):
    sid = _create_fleet()["fleet_id"]
    broker.claim_monitor_runtime(sid, os.getpid(), 5, _iso_now())  # this live process

    popen_calls = []
    monkeypatch.setattr(
        process.subprocess, "Popen", lambda *a, **k: popen_calls.append((a, k))
    )

    with pytest.raises(click.ClickException):
        process.start_detached(sid, 5)
    assert popen_calls == []  # the single-instance pre-check blocks the spawn


def test_start_detached__failure_result_names_log_path(state_dir, monkeypatch):
    sid = _create_fleet()["fleet_id"]

    def fake_popen(argv, **kwargs):
        # the worker never claims the runtime → no matching-pid heartbeat appears
        return SimpleNamespace(pid=777001, poll=lambda: 1)

    monkeypatch.setattr(process.subprocess, "Popen", fake_popen)
    monkeypatch.setattr(process.os, "kill", lambda pid, sig: None)

    result = process.start_detached(sid, 5)

    # never falsely reports "started" — the failure names the worker log file
    assert f"{sid}.log" in str(result)


def test_stop_monitor__signals_clears_and_removes_pidfile(state_dir, monkeypatch):
    sid = _create_fleet()["fleet_id"]
    pid = os.getpid()
    broker.claim_monitor_runtime(sid, pid, 5, _iso_now())
    pid_file = state_dir / f"{sid}.pid"
    pid_file.write_text(str(pid))

    kills = []

    def fake_kill(target_pid, sig):
        kills.append((target_pid, sig))
        if sig == signal.SIGTERM:
            broker.clear_monitor_runtime(sid, pid)  # simulate clean worker shutdown

    monkeypatch.setattr(process.os, "kill", fake_kill)

    process.stop_monitor(sid)

    assert (pid, signal.SIGTERM) in kills
    row = broker.read_monitor_runtime(sid)
    assert row is None or row["pid"] is None
    assert not pid_file.exists()


def test_stop_monitor__no_op_when_nothing_running(state_dir, monkeypatch):
    sid = _create_fleet()["fleet_id"]  # no runtime claimed, no pid file

    kills = []
    monkeypatch.setattr(process.os, "kill", lambda p, s: kills.append((p, s)))

    process.stop_monitor(sid)  # idempotent: must not raise
    assert kills == []
