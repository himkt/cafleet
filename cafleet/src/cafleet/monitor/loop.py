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
from cafleet.config import settings
from cafleet.multiplexer import AgentStateAware, resolve_multiplexer

# The sole native agent status that flags a wake. ``blocked`` is deliberately
# excluded: it means the member awaits a user answer, and the
# monitoring member's only correct action there is inaction — waking about it is
# pure token cost plus a nonzero chance of a destructive nudge. ``blocked`` is
# still recorded in ``_last_member_status`` so the episode is tracked and a later
# recovery is detected; it simply never flags a wake.
_WAKE_ON_STATUS = ("done",)

# Per-process last-seen native agent status, keyed by member_id. Only a transition
# INTO ``done`` flags a member due, so one ``done`` episode wakes the watcher
# once. Reset per run in ``run_monitor_loop``.
_last_member_status: dict[int, str | None] = {}

# Per-process last stall-check time per member (tz-aware ``datetime``, not an ISO
# string — it is compared against ``now`` directly, never persisted). A member
# absent from this map is stall-check due on its first observation. Reset per run
# in ``run_monitor_loop``.
_last_stall_check_at: dict[int, datetime] = {}


class _Sentinel:
    """Identity-comparable tick-result marker with a readable repr."""

    def __init__(self, name: str) -> None:
        self._name = name

    def __repr__(self) -> str:
        return self._name


CONTINUE = _Sentinel("CONTINUE")
STOP = _Sentinel("STOP")


def should_ping(target: dict, now: datetime) -> bool:
    """Decide whether a watched member is due for a check this tick.

    ``target`` is a ``list_monitor_targets`` row — a watched member: the root
    Director (180 s) or an ordinary member (720 s). The dedicated monitoring
    member is the unenrolled watcher and never appears here. A watched member is
    due once its interval has elapsed, regardless of ``pending_count`` (R2). The
    policy is role-agnostic (``is_director`` is retained for ``monitor status``
    labeling, not consulted here). Disabled members and dead/missing panes are
    always skipped, and a not-yet-due member waits.
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
    """Run one scan pass: heartbeat, then wake the watcher if any member is due.

    Returns ``STOP`` (self-terminate) when this process no longer owns the slot
    (ownership-checked heartbeat matched zero rows) or the fleet vanished /
    soft-deleted; otherwise ``CONTINUE``. The pane-liveness set is fetched once
    per tick (one tmux call). The due set is computed over the WATCHED members
    (the root Director + ordinary members); when ≥ 1 is due and the monitoring
    member's pane is live, the loop keystrokes a single wake nudge — naming the
    freshly-due members and the Director id — into the watcher's own pane (never
    into a watched pane) and advances each due member's cadence in one write.
    With no live watcher to wake, nothing is recorded.
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
            target["wake_reasons"] = ["interval"]
            due.append(target)

    # Stall-check cadence runs on BOTH backends, independent of the interval
    # trigger; it unions ``stall-check`` onto each stall-check-due watched member.
    _flag_stall_check_due(targets, due, now)

    pending_status: dict[int, str | None] = {}
    if isinstance(mux, AgentStateAware):
        pending_status = _flag_native_status_due(mux, targets, due)

    if due and watcher is not None and watcher["pane_id"] in live_panes:
        # The loop's only keystroke: a single best-effort wake nudge into the
        # watcher's own pane. The nudge NAMES the due members (each with its
        # ``wake_reasons``) and the Director id, driving the watcher's
        # capture-classify-reengage routine over exactly those panes plus the
        # Director. A watched pane (Director / member) is never keystroked.
        woke = mux.send_wake_trigger(
            target_pane_id=watcher["pane_id"],
            due_members=due,
            director_member_id=fleet["director_member_id"],
        )
        if woke:
            # All ledger writes are gated on a successful wake, so a failed
            # best-effort keystroke leaves the due members flagged and the next
            # tick retries instead of silently skipping a check.
            #
            # ``record_pings`` receives only members whose reasons include
            # ``interval`` or ``status:done`` — a stall-check-only member is
            # EXCLUDED, keeping the ping cadence and the stall cadence
            # independent. ``_last_stall_check_at`` is committed for every member
            # whose reasons include ``stall-check``. ``pending_status`` holds the
            # natively-flagged members' reads, committed on this same gate (the
            # non-flagged reads were already committed in
            # ``_flag_native_status_due``).
            ping_ids = [
                t["member_id"]
                for t in due
                if "interval" in t["wake_reasons"] or "status:done" in t["wake_reasons"]
            ]
            if ping_ids:
                broker.record_pings(ping_ids, now.isoformat())
            _last_member_status.update(pending_status)
            for target in due:
                if "stall-check" in target["wake_reasons"]:
                    _last_stall_check_at[target["member_id"]] = now
            # Visible heartbeat: one line per due member on the launching task's
            # stdout, each carrying its joined ``[<reasons>]`` suffix.
            for target in due:
                suffix = f" [{','.join(target['wake_reasons'])}]"
                click.echo(
                    f"{now.isoformat()} due member {target['member_id']} "
                    f"({target['name']}){suffix} -> wake monitor"
                )
    return CONTINUE


def _flag_native_status_due(
    mux: AgentStateAware, targets: list[dict], due: list[dict]
) -> dict[int, str | None]:
    """Union native ``done`` transitions into the due set.

    Point-reads each **enabled** watched live member's native status and flags any
    whose status just transitioned into ``done`` (the sole ``_WAKE_ON_STATUS``
    state), tagging it with a ``status:done`` wake reason. A transition into
    ``blocked`` NEVER flags a wake — it is committed to ``_last_member_status`` via
    the non-flagged branch below, so the episode is tracked and a later
    ``blocked → working`` recovery is detected, but the member is never woken
    about. Herdr-only: the caller guards on ``isinstance(mux, AgentStateAware)``.

    Non-flagged reads (recovery / idle / blocked / steady-state) are committed to
    ``_last_member_status`` **immediately**, so a recovery (e.g. blocked→working)
    is recorded even on a no-wake tick and the next episode is detected. Only the
    natively-**flagged** ``done`` reads are returned as pending; the caller commits
    those **only after a successful wake**, so a wake failure re-flags that
    episode next tick (mirrors the interval branch's ``record_pings`` gating).
    """
    due_ids = {t["member_id"] for t in due}
    pending: dict[int, str | None] = {}
    for target in targets:
        if not target["enabled"]:
            continue
        if target["pane_id"] is None or not target["pane_alive"]:
            continue
        member_id = target["member_id"]
        status = mux.agent_status(target_pane_id=target["pane_id"])
        prev = _last_member_status.get(member_id)
        if status in _WAKE_ON_STATUS and status != prev and member_id not in due_ids:
            target["wake_reasons"] = [f"status:{status}"]
            due.append(target)
            due_ids.add(member_id)
            pending[member_id] = status
        else:
            _last_member_status[member_id] = status
    return pending


def _flag_stall_check_due(targets: list[dict], due: list[dict], now: datetime) -> None:
    """Union each stall-check-due watched member into the due set (both backends).

    Driven by ``settings.monitor_stall_interval`` (default 240; ``0`` disables the
    trigger entirely). An **enabled** watched live member is stall-check due when
    it is ABSENT from ``_last_stall_check_at`` (first-tick semantics, mirroring
    ``should_ping``'s ``last_ping_at is None → due``; the map is never pre-seeded)
    or when at least one full interval has elapsed since its last stall-check. A
    stall-check-due member already in the due set (e.g. interval-due) has
    ``stall-check`` UNIONED onto its existing ``wake_reasons``; a member not yet
    due is appended with ``["stall-check"]``.

    This never commits ``_last_stall_check_at`` — that commit is gated on a
    successful wake in ``monitor_tick``, so a failed keystroke re-flags the member
    next tick (mirrors the ``record_pings`` gating).
    """
    interval = settings.monitor_stall_interval
    if interval <= 0:
        return
    due_ids = {t["member_id"] for t in due}
    for target in targets:
        if not target["enabled"]:
            continue
        if target["pane_id"] is None or not target["pane_alive"]:
            continue
        member_id = target["member_id"]
        last = _last_stall_check_at.get(member_id)
        if last is not None and (now - last).total_seconds() < interval:
            continue
        if member_id in due_ids:
            target["wake_reasons"].append("stall-check")
        else:
            target["wake_reasons"] = ["stall-check"]
            due.append(target)
            due_ids.add(member_id)


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
    _last_member_status.clear()
    _last_stall_check_at.clear()
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
