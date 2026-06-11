"""Member roster and activity proxies."""

from datetime import UTC, datetime

from sqlalchemy import func, select

from cafleet.broker import _shared
from cafleet.db.models import Agent, AgentPlacement, Task


def _base_members_select(fleet_id: int):
    return (
        select(
            Agent.agent_id,
            Agent.name,
            Agent.description,
            Agent.status,
            Agent.registered_at,
            AgentPlacement.director_agent_id,
            AgentPlacement.tmux_session,
            AgentPlacement.tmux_window_id,
            AgentPlacement.tmux_pane_id,
            AgentPlacement.coding_agent,
            AgentPlacement.created_at,
        )
        .join(AgentPlacement, Agent.agent_id == AgentPlacement.agent_id)
        .where(
            Agent.fleet_id == fleet_id,
            Agent.status == "active",
            AgentPlacement.director_agent_id.is_not(None),
        )
    )


def list_members(fleet_id: int) -> list[dict]:
    """Return the fleet's active members, with placements.

    A fleet has exactly one Director (the root), so every member placement's
    ``director_agent_id`` equals the fleet root. Members are selected by
    ``director_agent_id IS NOT NULL``, which lists all members and excludes
    the root Director's own placement (the only row with a ``NULL`` director).

    Args:
        fleet_id: Fleet id to scope the query to.

    Returns:
        List of dicts each carrying ``agent_id``, ``name``, ``description``,
        ``status``, ``registered_at``, and ``placement``.
    """
    stmt = _base_members_select(fleet_id)
    with _shared.read_session() as session:
        rows = session.execute(stmt).all()
    return [
        {
            "agent_id": row.agent_id,
            "name": row.name,
            "description": row.description,
            "status": row.status,
            "registered_at": row.registered_at,
            "placement": _shared.placement_dict(row),
        }
        for row in rows
    ]


def list_members_with_activity(fleet_id: int) -> list[dict]:
    """``list_members`` plus per-member activity proxies sourced from ``tasks``.

    Scoped by ``fleet_id`` only — the flat single-Director model means every
    member placement's ``director_agent_id`` equals the fleet root, so the
    ``director_agent_id IS NOT NULL`` member filter applies here too (and
    excludes the root Director's own placement).

    ``last_sent`` / ``last_recv`` / ``last_ack`` aggregate ``status_timestamp``
    over the ``tasks`` table per agent. All three filter ``Task.type !=
    'broadcast_summary'`` (mirrors ``poll_tasks``); broadcast_summary rows
    land in the broadcaster's own context with ``status_state='completed'``
    and would otherwise pollute every proxy for the broadcaster.

    Args:
        fleet_id: Fleet id to scope the query to.

    Returns:
        List of dicts as in :func:`list_members`, additionally carrying
        ``last_sent``, ``last_recv``, ``last_ack`` (ISO timestamps or
        ``None``), and ``idle`` — the integer-second delta between ``now``
        and the most recent of ``last_sent`` / ``last_recv``, or ``None``
        when both are ``None``.
    """
    last_sent_sq = (
        select(func.max(Task.status_timestamp))
        .where(
            Task.from_agent_id == Agent.agent_id,
            _shared.NOT_BROADCAST_SUMMARY,
        )
        .correlate(Agent)
        .scalar_subquery()
    )
    last_recv_sq = (
        select(func.max(Task.status_timestamp))
        .where(
            Task.context_id == Agent.agent_id,
            _shared.NOT_BROADCAST_SUMMARY,
        )
        .correlate(Agent)
        .scalar_subquery()
    )
    last_ack_sq = (
        select(func.max(Task.status_timestamp))
        .where(
            Task.context_id == Agent.agent_id,
            Task.status_state == "completed",
            _shared.NOT_BROADCAST_SUMMARY,
        )
        .correlate(Agent)
        .scalar_subquery()
    )
    stmt = _base_members_select(fleet_id).add_columns(
        last_sent_sq.label("last_sent"),
        last_recv_sq.label("last_recv"),
        last_ack_sq.label("last_ack"),
    )
    with _shared.read_session() as session:
        rows = session.execute(stmt).all()

    now = datetime.now(UTC)
    return [
        {
            "agent_id": row.agent_id,
            "name": row.name,
            "description": row.description,
            "status": row.status,
            "registered_at": row.registered_at,
            "placement": _shared.placement_dict(row),
            "last_sent": row.last_sent,
            "last_recv": row.last_recv,
            "last_ack": row.last_ack,
            "idle": _idle_seconds(now, row.last_sent, row.last_recv),
        }
        for row in rows
    ]


def _idle_seconds(
    now: datetime, last_sent: str | None, last_recv: str | None
) -> int | None:
    candidates = [t for t in (last_sent, last_recv) if t is not None]
    if not candidates:
        return None
    most_recent = datetime.fromisoformat(max(candidates))
    delta = (now - most_recent).total_seconds()
    return max(0, int(delta))
