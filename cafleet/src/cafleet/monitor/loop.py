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
from cafleet.multiplexer import AgentStateAware, resolve_multiplexer

_ATTENTION_STATES = ("blocked", "done")

# Per-process last-seen native agent status, keyed by agent_id. Only transitions
# INTO an attention state flag an agent due, so one blocked/done episode wakes the
# watcher once. Reset per run in ``run_monitor_loop``.
_last_agent_status: dict[int, str | None] = {}


class _Sentinel:
    """Identity-comparable tick-result marker with a readable repr."""

    def __init__(self, name: str) -> None:
        self._name = name

    def __repr__(self) -> str:
        return self._name


CONTINUE = _Sentinel("CONTINUE")
STOP = _Sentinel("STOP")


def should_ping(target: dict, now: datetime) -> bool:
    """Decide whether a watched agent is due for a check this tick.

    ``target`` is a ``list_monitor_targets`` row — a watched agent: the root
    Director (180 s) or an ordinary member (720 s). The dedicated monitoring
    member is the unenrolled watcher and never appears here. A watched agent is
    due once its interval has elapsed, regardless of ``pending_count`` (R2). The
    policy is role-agnostic (``is_director`` is retained for ``monitor status``
    labeling, not consulted here). Disabled agents and dead/missing panes are
    always skipped, and a not-yet-due agent waits.
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
    """Run one scan pass: heartbeat, then wake the watcher if any agent is due.

    Returns ``STOP`` (self-terminate) when this process no longer owns the slot
    (ownership-checked heartbeat matched zero rows) or the fleet vanished /
    soft-deleted; otherwise ``CONTINUE``. The pane-liveness set is fetched once
    per tick (one tmux call). The due set is computed over the WATCHED agents
    (the root Director + ordinary members); when ≥ 1 is due and the monitoring
    member's pane is live, the loop keystrokes a single wake nudge — naming the
    freshly-due agents and the Director id — into the watcher's own pane (never
    into a watched pane) and advances each due agent's cadence in one write. With
    no live watcher to wake, nothing is recorded.
    """
    if not broker.heartbeat_monitor_runtime(fleet_id, os.getpid(), now.isoformat()):
        return STOP
    fleet = broker.get_fleet(fleet_id)
    if fleet is None or fleet["deleted_at"] is not None:
        return STOP

    watcher = broker.find_monitoring_member(fleet_id)
    mux = resolve_multiplexer()
    live_panes = mux.list_pane_ids()

    targets = broker.list_monitor_targets(fleet_id)
    due: list[dict] = []
    for target in targets:
        target["pane_alive"] = target["pane_id"] in live_panes
        if should_ping(target, now):
            due.append(target)

    pending_status: dict[int, str | None] = {}
    if isinstance(mux, AgentStateAware):
        pending_status = _flag_native_status_due(mux, targets, due)

    if due and watcher is not None and watcher["pane_id"] in live_panes:
        # The loop's only keystroke: a single best-effort wake nudge into the
        # watcher's own pane. The nudge NAMES the freshly-due agents and the
        # Director id, driving the watcher's capture-classify-reengage routine
        # over exactly those panes plus the Director. A watched pane (Director /
        # member) is never keystroked.
        woke = mux.send_wake_trigger(
            target_pane_id=watcher["pane_id"],
            due_agents=due,
            director_agent_id=fleet["director_agent_id"],
        )
        if woke:
            # Stamp each due agent's cadence ONLY on a successful wake, so a
            # just-flagged agent is not due again next tick (no wake-storm while
            # the watcher works). A failed best-effort keystroke leaves the due
            # agents flagged, so the next tick retries instead of silently
            # skipping a check for a full interval. pending_status holds only the
            # natively-flagged agents' reads, committed on this same wake gate so
            # a wake failure re-flags the episode (non-flagged reads were already
            # committed in _flag_native_status_due).
            broker.record_pings([t["agent_id"] for t in due], now.isoformat())
            _last_agent_status.update(pending_status)
            # Visible heartbeat: one line per due agent on the launching task's stdout.
            # Native-due agents carry a ``[status:<state>]`` suffix; interval-due
            # agents keep the bare line unchanged.
            for target in due:
                reason = target.get("wake_reason")
                suffix = f" [{reason}]" if reason else ""
                click.echo(
                    f"{now.isoformat()} due agent {target['agent_id']} "
                    f"({target['name']}){suffix} -> wake monitor"
                )
    return CONTINUE


def _flag_native_status_due(
    mux: AgentStateAware, targets: list[dict], due: list[dict]
) -> dict[int, str | None]:
    """Union native ``blocked``/``done`` transitions into the interval-due set.

    Point-reads each **enabled** watched live agent's native status and flags any
    whose status just transitioned into an attention state, tagging it with a
    ``status:<state>`` wake reason. Herdr-only: the caller guards on
    ``isinstance(mux, AgentStateAware)``.

    Non-flagged reads (recovery / idle / steady-state) are committed to
    ``_last_agent_status`` **immediately**, so a recovery (e.g. blocked→working)
    is recorded even on a no-wake tick and the next episode is detected. Only the
    natively-**flagged** agents' reads are returned as pending; the caller commits
    those **only after a successful wake**, so a wake failure re-flags that
    episode next tick (mirrors the interval branch's ``record_pings`` gating).
    """
    due_ids = {t["agent_id"] for t in due}
    pending: dict[int, str | None] = {}
    for target in targets:
        if not target["enabled"]:
            continue
        if target["pane_id"] is None or not target["pane_alive"]:
            continue
        agent_id = target["agent_id"]
        status = mux.agent_status(target_pane_id=target["pane_id"])
        prev = _last_agent_status.get(agent_id)
        if status in _ATTENTION_STATES and status != prev and agent_id not in due_ids:
            target["wake_reason"] = f"status:{status}"
            due.append(target)
            due_ids.add(agent_id)
            pending[agent_id] = status
        else:
            _last_agent_status[agent_id] = status
    return pending


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
    _last_agent_status.clear()
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
