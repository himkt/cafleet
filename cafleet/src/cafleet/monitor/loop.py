"""The monitor loop: pure ``should_ping`` policy + ``monitor_tick`` + driver (§4–5).

``should_ping`` is a pure function of a scan-row dict and a tz-aware ``now``
(unit-testable without tmux or the DB). ``monitor_tick`` performs one full scan
pass and returns ``CONTINUE`` / ``STOP``. ``run_monitor_loop`` is the thin
foreground driver `cafleet monitor start` runs in-process — a coding agent
launches it as a background task and owns its lifetime.

Time is threaded as tz-aware ``datetime`` through the pure functions; every
DB-storage boundary serializes with ``.isoformat()`` (the columns are TEXT).
"""

import os
import signal
import time
from datetime import UTC, datetime

import click

from cafleet import broker
from cafleet.multiplexer.tmux import TmuxMultiplexer


class _Sentinel:
    """Identity-comparable tick-result marker with a readable repr."""

    def __init__(self, name: str) -> None:
        self._name = name

    def __repr__(self) -> str:
        return self._name


CONTINUE = _Sentinel("CONTINUE")
STOP = _Sentinel("STOP")


def should_ping(target: dict, now: datetime) -> bool:
    """Decide whether an enrolled agent is due for a ping this tick.

    Every enrolled agent — Director and member alike — is pinged once its
    interval has elapsed, regardless of ``pending_count`` (R2). Disabled agents
    and dead/missing panes are always skipped, and a not-yet-due agent waits.
    """
    if not target["enabled"]:
        return False
    if target["pane_id"] is None or not target["pane_alive"]:
        return False
    if target["last_ping_at"] is not None:
        elapsed = (now - datetime.fromisoformat(target["last_ping_at"])).total_seconds()
        if elapsed < target["interval_seconds"]:
            return False
    return True


def monitor_tick(fleet_id: int, now: datetime) -> _Sentinel:
    """Run one scan pass: heartbeat, then ping every due agent.

    Returns ``STOP`` (self-terminate) when this process no longer owns the slot
    (ownership-checked heartbeat matched zero rows) or the fleet vanished /
    soft-deleted; otherwise ``CONTINUE``. The pane-liveness set is fetched once
    per tick (one tmux call), and ``last_ping_at`` advances whenever a ping is
    attempted, regardless of the best-effort keystroke's success.
    """
    if not broker.heartbeat_monitor_runtime(fleet_id, os.getpid(), now.isoformat()):
        return STOP
    fleet = broker.get_fleet(fleet_id)
    if fleet is None or fleet["deleted_at"] is not None:
        return STOP

    mux = TmuxMultiplexer()
    live_panes = mux.list_pane_ids()
    for target in broker.list_monitor_targets(fleet_id):
        target["pane_alive"] = target["pane_id"] in live_panes
        if should_ping(target, now):
            mux.send_poll_trigger(
                target_pane_id=target["pane_id"],
                fleet_id=fleet_id,
                agent_id=target["agent_id"],
            )
            broker.record_ping(target["agent_id"], now.isoformat())
            # Visible heartbeat: the launching agent's background task shows a
            # line per dispatched ping on its stdout.
            click.echo(
                f"{now.isoformat()} ping agent {target['agent_id']} ({target['name']})"
            )
    return CONTINUE


_stop_requested = False


def _request_stop(signum, frame) -> None:  # noqa: ARG001 - signal handler signature
    global _stop_requested
    _stop_requested = True


def _interruptible_sleep(seconds: float) -> None:
    """Sleep up to ``seconds``, waking early once a stop signal has arrived."""
    deadline = time.monotonic() + seconds
    while not _stop_requested:
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            return
        time.sleep(min(0.2, remaining))


def run_monitor_loop(fleet_id: int, tick_seconds: int) -> None:
    """Foreground driver: claim the slot, then loop ``scan → sleep`` until signalled.

    Runs in-process — a coding agent launches it as a background task. The
    ``monitor_runtime`` row is the only coordination artifact (no PID file): a
    clean stop (SIGTERM/SIGINT) clears it; a hard kill lets the heartbeat go
    stale.

    Raises:
        click.ClickException: If a live monitor already holds the slot.
    """
    global _stop_requested
    _stop_requested = False
    pid = os.getpid()
    if not broker.claim_monitor_runtime(
        fleet_id, pid, tick_seconds, datetime.now(UTC).isoformat()
    ):
        raise click.ClickException(f"monitor already running for fleet {fleet_id}")
    signal.signal(signal.SIGTERM, _request_stop)
    signal.signal(signal.SIGINT, _request_stop)
    try:
        while not _stop_requested:
            if monitor_tick(fleet_id, datetime.now(UTC)) is STOP:
                break
            _interruptible_sleep(tick_seconds)
    finally:
        broker.clear_monitor_runtime(fleet_id, pid)
