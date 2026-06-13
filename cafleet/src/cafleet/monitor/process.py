"""Process lifecycle for ``cafleet monitor`` — detach launcher, PID file, signals.

The OS-side half of the monitor (the broker is the pure DB half): writes the
per-fleet PID file under ``settings.monitor_state_dir``, launches the detached
worker via ``subprocess.Popen`` (re-exec'd as ``python -m cafleet … --foreground``),
and stops a running monitor with ``SIGTERM`` → wait → ``SIGKILL`` escalation.
"""

import contextlib
import os
import signal
import subprocess
import sys
import time
from dataclasses import dataclass
from datetime import UTC, datetime
from pathlib import Path

import click

from cafleet import broker
from cafleet.config import settings
from cafleet.monitor import MONITOR_STOP_TIMEOUT


@dataclass
class StartResult:
    """Outcome of ``start_detached``, consumed by the CLI for output + exit code."""

    ok: bool
    pid: int | None
    tick_seconds: int | None
    log_path: Path | None
    message: str


@dataclass
class StopResult:
    """Outcome of ``stop_monitor`` (idempotent — ``ok`` is always True)."""

    ok: bool
    stopped: bool
    pid: int | None
    message: str


def _state_dir() -> Path:
    return Path(settings.monitor_state_dir)


def pid_file_path(fleet_id: int) -> Path:
    return _state_dir() / f"{fleet_id}.pid"


def log_file_path(fleet_id: int) -> Path:
    return _state_dir() / f"{fleet_id}.log"


def write_pid_file(fleet_id: int, pid: int) -> None:
    state_dir = _state_dir()
    state_dir.mkdir(parents=True, exist_ok=True)
    pid_file_path(fleet_id).write_text(str(pid))


def read_pid_file(fleet_id: int) -> int | None:
    path = pid_file_path(fleet_id)
    if not path.exists():
        return None
    try:
        return int(path.read_text().strip())
    except (ValueError, OSError):
        return None


def remove_pid_file(fleet_id: int) -> None:
    pid_file_path(fleet_id).unlink(missing_ok=True)


def start_detached(fleet_id: int, tick_seconds: int) -> StartResult:
    """Spawn the detached monitor worker and confirm it claimed the slot.

    Advisory single-instance pre-check (the worker's atomic claim is the real
    guard), then ``Popen`` of ``python -m cafleet … monitor start --foreground``
    in a new session with stdout/stderr → ``<state_dir>/<fleet_id>.log``. Polls
    up to ~2 s for a fresh heartbeat whose ``pid`` equals the spawned child's —
    never reports "started" against a different already-running monitor's pid.

    Raises:
        click.ClickException: If a live monitor already holds the slot.
    """
    if broker.monitor_is_live(fleet_id, datetime.now(UTC)):
        row = broker.read_monitor_runtime(fleet_id)
        pid = row["pid"] if row is not None else "?"
        raise click.ClickException(
            f"monitor already running for fleet {fleet_id} (pid {pid})"
        )

    state_dir = _state_dir()
    state_dir.mkdir(parents=True, exist_ok=True)
    log_path = log_file_path(fleet_id)
    argv = [
        sys.executable,
        "-m",
        "cafleet",
        "--fleet-id",
        str(fleet_id),
        "monitor",
        "start",
        "--tick",
        str(tick_seconds),
        "--foreground",
    ]
    with log_path.open("ab") as logf:
        child = subprocess.Popen(
            argv,
            start_new_session=True,
            stdin=subprocess.DEVNULL,
            stdout=logf,
            stderr=subprocess.STDOUT,
            close_fds=True,
        )

    deadline = time.monotonic() + 2.0
    while time.monotonic() < deadline:
        row = broker.read_monitor_runtime(fleet_id)
        if row is not None and row["pid"] == child.pid:
            return StartResult(
                ok=True,
                pid=child.pid,
                tick_seconds=tick_seconds,
                log_path=None,
                message=f"monitor started (pid {child.pid}, tick {tick_seconds}s)",
            )
        if child.poll() is not None:
            break  # the worker exited before claiming the slot
        time.sleep(0.05)

    return StartResult(
        ok=False,
        pid=None,
        tick_seconds=None,
        log_path=log_path,
        message=f"monitor failed to start; see {log_path}",
    )


def stop_monitor(fleet_id: int) -> StopResult:
    """Stop the fleet's monitor: ``SIGTERM`` → wait → ``SIGKILL`` escalation.

    Idempotent — a no-monitor fleet returns a "nothing running" result without
    signalling. Always clears the runtime row and removes the PID file at the
    end, so a half-dead monitor cannot leave stale artifacts behind.
    """
    pid = read_pid_file(fleet_id)
    if pid is None:
        row = broker.read_monitor_runtime(fleet_id)
        pid = row["pid"] if row is not None else None

    if pid is None:
        remove_pid_file(fleet_id)
        return StopResult(
            ok=True,
            stopped=False,
            pid=None,
            message=f"no monitor running for fleet {fleet_id}",
        )

    # already-dead pid → ProcessLookupError is benign; fall through to cleanup
    with contextlib.suppress(ProcessLookupError):
        os.kill(pid, signal.SIGTERM)

    deadline = time.monotonic() + MONITOR_STOP_TIMEOUT
    while time.monotonic() < deadline:
        row = broker.read_monitor_runtime(fleet_id)
        if row is None or row["pid"] is None:
            break
        time.sleep(0.05)
    else:
        with contextlib.suppress(ProcessLookupError):
            os.kill(pid, signal.SIGKILL)

    broker.clear_monitor_runtime(fleet_id, pid)
    remove_pid_file(fleet_id)
    return StopResult(
        ok=True, stopped=True, pid=pid, message=f"monitor stopped (pid {pid})"
    )
