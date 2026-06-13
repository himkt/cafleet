"""Monitor schedule + runtime DB layer.

The pure data-access half of ``cafleet monitor`` (see
``docs/concepts/monitoring.md``): per-agent schedule CRUD (``monitor_config``),
the per-tick scan, ping recording, and the single-instance runtime
claim/heartbeat/clear (``monitor_runtime``) with an ownership-checked
split-brain guard. No OS side effects live here — process lifecycle, signals,
and the PID file belong to the ``cafleet.monitor`` package.

The ``monitor_config.enabled`` column is an ``INTEGER`` 0/1, but every read
function casts it to a Python ``bool`` at the boundary, so the integer
representation never leaks past the broker.
"""

import os
from datetime import datetime

import click
from sqlalchemy import delete, func, select, update

from cafleet.broker import _shared
from cafleet.db.models import (
    Agent,
    AgentPlacement,
    Fleet,
    MonitorConfig,
    MonitorRuntime,
    Task,
)

# Enrollment default and liveness window. The ``cafleet.monitor`` package
# imports these so the value has a single home in the layer that computes
# liveness; ``STALE_AFTER = max(FACTOR * tick_seconds, FLOOR_SECONDS)``.
DEFAULT_PING_INTERVAL_SECONDS = 60
MONITOR_STALE_FACTOR = 3
MONITOR_STALE_FLOOR_SECONDS = 15


def _config_dict(row) -> dict:
    return {
        "agent_id": row.agent_id,
        "interval_seconds": row.interval_seconds,
        "last_ping_at": row.last_ping_at,
        "enabled": bool(row.enabled),
    }


def _enroll(
    session, agent_id: int, interval: int = DEFAULT_PING_INTERVAL_SECONDS
) -> None:
    """Insert a ``monitor_config`` row for a pane-bound agent.

    Called inside the same write transaction as the agent/placement insert, so
    enrollment is atomic with registration. Only agents with a tmux pane (the
    root Director and members) are enrolled.
    """
    session.add(MonitorConfig(agent_id=agent_id, interval_seconds=interval, enabled=1))


def get_monitor_config(fleet_id: int, agent_id: int) -> dict | None:
    """Return the agent's schedule, or ``None`` if not enrolled / not in fleet."""
    stmt = (
        select(
            MonitorConfig.agent_id,
            MonitorConfig.interval_seconds,
            MonitorConfig.last_ping_at,
            MonitorConfig.enabled,
        )
        .join(Agent, Agent.agent_id == MonitorConfig.agent_id)
        .where(MonitorConfig.agent_id == agent_id, Agent.fleet_id == fleet_id)
    )
    with _shared.read_session() as session:
        row = session.execute(stmt).first()
    return _config_dict(row) if row is not None else None


def list_monitor_configs(fleet_id: int) -> list[dict]:
    """Return every enrolled agent's schedule in the fleet (``enabled`` as bool)."""
    stmt = (
        select(
            MonitorConfig.agent_id,
            MonitorConfig.interval_seconds,
            MonitorConfig.last_ping_at,
            MonitorConfig.enabled,
        )
        .join(Agent, Agent.agent_id == MonitorConfig.agent_id)
        .where(Agent.fleet_id == fleet_id)
    )
    with _shared.read_session() as session:
        rows = session.execute(stmt).all()
    return [_config_dict(row) for row in rows]


def update_monitor_config(
    fleet_id: int,
    agent_id: int,
    *,
    interval_seconds: int | None = None,
    enabled: bool | None = None,
) -> dict:
    """Update an enrolled agent's interval and/or enabled flag; return the new config.

    A partial update leaves the unspecified field untouched. ``enabled`` is
    accepted as a bool and written as 0/1.

    Raises:
        click.ClickException: If the agent is not in the fleet or not enrolled.
    """
    with _shared.write_session() as session:
        enrolled = session.execute(
            select(MonitorConfig.agent_id)
            .join(Agent, Agent.agent_id == MonitorConfig.agent_id)
            .where(MonitorConfig.agent_id == agent_id, Agent.fleet_id == fleet_id)
        ).scalar_one_or_none()
        if enrolled is None:
            raise click.ClickException(
                f"agent {agent_id} is not enrolled in monitoring for fleet {fleet_id}."
            )

        values: dict = {}
        if interval_seconds is not None:
            values["interval_seconds"] = interval_seconds
        if enabled is not None:
            values["enabled"] = 1 if enabled else 0
        if values:
            session.execute(
                update(MonitorConfig)
                .where(MonitorConfig.agent_id == agent_id)
                .values(**values)
            )

        row = session.execute(
            select(
                MonitorConfig.agent_id,
                MonitorConfig.interval_seconds,
                MonitorConfig.last_ping_at,
                MonitorConfig.enabled,
            ).where(MonitorConfig.agent_id == agent_id)
        ).first()
        return _config_dict(row)


def record_ping(agent_id: int, when: str) -> None:
    """Stamp ``last_ping_at`` for the agent after a ping is dispatched.

    ``when`` is an ISO-8601 string stored verbatim in the TEXT column.
    """
    with _shared.write_session() as session:
        session.execute(
            update(MonitorConfig)
            .where(MonitorConfig.agent_id == agent_id)
            .values(last_ping_at=when)
        )


def list_monitor_targets(fleet_id: int) -> list[dict]:
    """Per-tick scan: one row per active, enrolled agent in the fleet.

    Each dict carries ``agent_id``, ``name``, ``is_director`` (derived from
    ``fleets.director_agent_id``), ``pane_id``, ``interval_seconds``,
    ``last_ping_at``, ``enabled`` (bool), and ``pending_count`` — the count of
    the agent's ``input_required`` deliveries excluding ``broadcast_summary``
    rows, a correlated subquery mirroring ``members.py``.
    """
    pending_sq = (
        select(func.count(Task.task_id))
        .where(
            Task.context_id == Agent.agent_id,
            Task.status_state == "input_required",
            _shared.NOT_BROADCAST_SUMMARY,
        )
        .correlate(Agent)
        .scalar_subquery()
    )
    stmt = (
        select(
            Agent.agent_id,
            Agent.name,
            AgentPlacement.tmux_pane_id,
            Fleet.director_agent_id,
            MonitorConfig.interval_seconds,
            MonitorConfig.last_ping_at,
            MonitorConfig.enabled,
            pending_sq.label("pending_count"),
        )
        .join(MonitorConfig, MonitorConfig.agent_id == Agent.agent_id)
        .join(AgentPlacement, AgentPlacement.agent_id == Agent.agent_id)
        .join(Fleet, Fleet.fleet_id == Agent.fleet_id)
        .where(Agent.fleet_id == fleet_id, Agent.status == "active")
    )
    with _shared.read_session() as session:
        rows = session.execute(stmt).all()
    return [
        {
            "agent_id": row.agent_id,
            "name": row.name,
            "is_director": row.agent_id == row.director_agent_id,
            "pane_id": row.tmux_pane_id,
            "interval_seconds": row.interval_seconds,
            "last_ping_at": row.last_ping_at,
            "enabled": bool(row.enabled),
            "pending_count": row.pending_count,
        }
        for row in rows
    ]


def _is_live(row, now: datetime) -> bool:
    """True iff the runtime row's owner is still alive.

    Heartbeat freshness is the authority — a process that died silently stops
    rewriting ``last_tick_at``, so a stale heartbeat reads as dead even though
    the PID may still resolve. ``os.kill(pid, 0)`` is a corroborating signal.
    """
    if row.pid is None or row.last_tick_at is None:
        return False
    stale_after = max(
        MONITOR_STALE_FACTOR * row.tick_seconds, MONITOR_STALE_FLOOR_SECONDS
    )
    elapsed = (now - datetime.fromisoformat(row.last_tick_at)).total_seconds()
    if elapsed > stale_after:
        return False
    try:
        os.kill(row.pid, 0)
    except ProcessLookupError:
        return False
    except PermissionError:
        # The PID resolves but is owned by another user — it exists, so alive.
        return True
    return True


def claim_monitor_runtime(
    fleet_id: int, pid: int, tick_seconds: int, when: str
) -> bool:
    """Atomically claim the fleet's single monitor slot.

    Inserts a fresh row, reclaims a stale one, or refuses a live one — all in
    one write transaction (SQLite's write lock serializes concurrent claims).

    Returns:
        ``True`` if this ``pid`` now owns the slot; ``False`` if a live monitor
        already holds it.
    """
    now = datetime.fromisoformat(when)
    with _shared.write_session() as session:
        row = session.get(MonitorRuntime, fleet_id)
        if row is None:
            session.add(
                MonitorRuntime(
                    fleet_id=fleet_id,
                    pid=pid,
                    started_at=when,
                    last_tick_at=when,
                    tick_seconds=tick_seconds,
                )
            )
            return True
        if _is_live(row, now):
            return False
        row.pid = pid
        row.started_at = when
        row.last_tick_at = when
        row.tick_seconds = tick_seconds
        return True


def heartbeat_monitor_runtime(fleet_id: int, pid: int, when: str) -> bool:
    """Ownership-checked heartbeat — rewrites ``last_tick_at`` iff ``pid`` owns the slot.

    Returns ``False`` when the slot was reclaimed by another instance (the
    ``WHERE pid=?`` matches zero rows), the signal the displaced monitor uses
    to self-terminate without pinging.
    """
    with _shared.write_session() as session:
        result = session.execute(
            update(MonitorRuntime)
            .where(MonitorRuntime.fleet_id == fleet_id, MonitorRuntime.pid == pid)
            .values(pid=pid, last_tick_at=when)
        )
        return result.rowcount == 1


def clear_monitor_runtime(fleet_id: int, pid: int) -> None:
    """Ownership-checked clear — nulls ``pid``/``last_tick_at`` iff ``pid`` owns the slot.

    A non-owner clear matches zero rows and is a no-op, so a self-terminating
    loser never wipes the winner's row on exit.
    """
    with _shared.write_session() as session:
        session.execute(
            update(MonitorRuntime)
            .where(MonitorRuntime.fleet_id == fleet_id, MonitorRuntime.pid == pid)
            .values(pid=None, last_tick_at=None)
        )


def read_monitor_runtime(fleet_id: int) -> dict | None:
    """Return the fleet's runtime row, or ``None`` when no monitor ever claimed it."""
    with _shared.read_session() as session:
        row = session.get(MonitorRuntime, fleet_id)
        if row is None:
            return None
        return {
            "fleet_id": row.fleet_id,
            "pid": row.pid,
            "started_at": row.started_at,
            "last_tick_at": row.last_tick_at,
            "tick_seconds": row.tick_seconds,
        }


def monitor_is_live(fleet_id: int, now: datetime) -> bool:
    """Return True iff the fleet currently has a live monitor holding the slot.

    The advisory single-instance pre-check for ``monitor start`` (the atomic
    ``claim_monitor_runtime`` is the authoritative guard). Reuses ``_is_live``:
    heartbeat freshness is authoritative, ``os.kill(pid, 0)`` corroborates.
    """
    with _shared.read_session() as session:
        row = session.get(MonitorRuntime, fleet_id)
        if row is None:
            return False
        return _is_live(row, now)


def delete_fleet_monitor_rows(session, fleet_id: int) -> None:
    """Delete the fleet's ``monitor_config`` rows and its ``monitor_runtime`` row.

    Called inside ``delete_fleet``'s transaction, mirroring the explicit
    ``agent_placements`` cleanup.
    """
    agents_in_fleet = select(Agent.agent_id).where(Agent.fleet_id == fleet_id)
    session.execute(
        delete(MonitorConfig).where(MonitorConfig.agent_id.in_(agents_in_fleet))
    )
    session.execute(delete(MonitorRuntime).where(MonitorRuntime.fleet_id == fleet_id))


def delete_agent_monitor_row(session, agent_id: int) -> None:
    """Delete one agent's ``monitor_config`` row, alongside its placement cleanup."""
    session.execute(delete(MonitorConfig).where(MonitorConfig.agent_id == agent_id))
