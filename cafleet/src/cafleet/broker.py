"""Single data access layer — sync SQLAlchemy operations for CLI + WebUI.

Module-level functions using ``get_sync_sessionmaker()`` from ``db/engine.py``.
Each function opens a fresh session, executes within a transaction, and returns
dicts. CLI functions return dicts matching ``output.py`` expectations. WebUI
query functions return dicts matching ``webui_api.py`` response shapes.

Import: ``from cafleet import broker``.
"""

import json
import uuid
from datetime import UTC, datetime
from typing import cast

import click
from sqlalchemy import and_, delete, func, select, update
from sqlalchemy.dialects.sqlite import insert as sqlite_insert
from sqlalchemy.engine import CursorResult
from sqlalchemy.exc import IntegrityError

from cafleet.db.engine import get_sync_sessionmaker
from cafleet.db.models import Agent, AgentPlacement, Session, Task


# ---------------------------------------------------------------------------
# Internal helpers
# ---------------------------------------------------------------------------


def _now_iso() -> str:
    return datetime.now(UTC).isoformat()


# ---------------------------------------------------------------------------
# Session operations
# ---------------------------------------------------------------------------


def create_session(label: str | None = None) -> dict:
    """INSERT into sessions. Returns {"session_id": ..., "label": ..., "created_at": ...}."""
    session_id = str(uuid.uuid4())
    created_at = _now_iso()
    sm = get_sync_sessionmaker()
    with sm() as session:
        with session.begin():
            session.add(
                Session(
                    session_id=session_id,
                    label=label,
                    created_at=created_at,
                )
            )
    return {"session_id": session_id, "label": label, "created_at": created_at}


def list_sessions() -> list[dict]:
    """SELECT sessions with active agent count."""
    stmt = (
        select(
            Session.session_id,
            Session.label,
            Session.created_at,
            func.count(Agent.agent_id).label("agent_count"),
        )
        .select_from(Session)
        .outerjoin(
            Agent,
            and_(
                Agent.session_id == Session.session_id,
                Agent.status == "active",
            ),
        )
        .group_by(Session.session_id)
        .order_by(Session.created_at)
    )
    sm = get_sync_sessionmaker()
    with sm() as session:
        rows = session.execute(stmt).all()
    return [
        {
            "session_id": row.session_id,
            "label": row.label,
            "created_at": row.created_at,
            "agent_count": row.agent_count,
        }
        for row in rows
    ]


def get_session(session_id: str) -> dict | None:
    """SELECT single session. Returns dict or None."""
    sm = get_sync_sessionmaker()
    with sm() as session:
        result = session.execute(
            select(Session).where(Session.session_id == session_id)
        )
        row = result.scalar_one_or_none()
    if row is None:
        return None
    return {
        "session_id": row.session_id,
        "label": row.label,
        "created_at": row.created_at,
    }


def delete_session(session_id: str) -> None:
    """DELETE session. Raises click.UsageError if FK constraint blocks deletion."""
    sm = get_sync_sessionmaker()
    with sm() as session:
        with session.begin():
            try:
                result = cast(
                    CursorResult,
                    session.execute(
                        delete(Session).where(Session.session_id == session_id)
                    ),
                )
            except IntegrityError:
                count = session.execute(
                    select(func.count()).select_from(Agent).where(
                        Agent.session_id == session_id
                    )
                ).scalar()
                raise click.UsageError(
                    f"Cannot delete session {session_id}: "
                    f"it still has {count} agent(s) referencing it."
                )
        if result.rowcount == 0:
            raise click.UsageError(f"session '{session_id}' not found.")


# ---------------------------------------------------------------------------
# Agent registry operations
# ---------------------------------------------------------------------------


def register_agent(
    session_id: str,
    name: str,
    description: str,
    skills: list[dict] | None = None,
    placement: dict | None = None,
) -> dict:
    """INSERT into agents [+ agent_placements].

    Returns {"agent_id": ..., "name": ..., "registered_at": ...}.
    Validates session_id exists. When placement is provided, validates director
    exists and is active in the same session.
    """
    # Validate session exists
    if get_session(session_id) is None:
        raise click.UsageError(f"Session '{session_id}' not found.")

    agent_id = str(uuid.uuid4())
    registered_at = _now_iso()
    agent_card = {
        "name": name,
        "description": description,
        "skills": skills or [],
    }

    sm = get_sync_sessionmaker()
    with sm() as session:
        with session.begin():
            if placement is not None:
                # Validate director exists and is active in the same session
                director_id = placement["director_agent_id"]
                director = session.execute(
                    select(Agent).where(
                        Agent.agent_id == director_id,
                        Agent.session_id == session_id,
                        Agent.status == "active",
                    )
                ).scalar_one_or_none()
                if director is None:
                    raise click.UsageError(
                        f"Director agent '{director_id}' not found or not active "
                        f"in session '{session_id}'."
                    )

            session.add(
                Agent(
                    agent_id=agent_id,
                    session_id=session_id,
                    name=name,
                    description=description,
                    status="active",
                    registered_at=registered_at,
                    agent_card_json=json.dumps(agent_card),
                )
            )
            if placement is not None:
                session.add(
                    AgentPlacement(
                        agent_id=agent_id,
                        director_agent_id=placement["director_agent_id"],
                        tmux_session=placement["tmux_session"],
                        tmux_window_id=placement["tmux_window_id"],
                        tmux_pane_id=placement.get("tmux_pane_id"),
                        coding_agent=placement.get("coding_agent", "claude"),
                        created_at=registered_at,
                    )
                )

    return {
        "agent_id": agent_id,
        "name": name,
        "registered_at": registered_at,
    }


def get_agent(agent_id: str, session_id: str) -> dict | None:
    """Single agent detail with optional placement.

    Returns dict with agent info + placement or None.
    Filters by session and excludes deregistered agents.
    """
    sm = get_sync_sessionmaker()
    with sm() as session:
        agent = session.execute(
            select(Agent).where(
                Agent.agent_id == agent_id,
                Agent.session_id == session_id,
                Agent.status == "active",
            )
        ).scalar_one_or_none()

        if agent is None:
            return None

        placement_row = session.execute(
            select(AgentPlacement).where(AgentPlacement.agent_id == agent_id)
        ).scalar_one_or_none()

    result = {
        "agent_id": agent.agent_id,
        "name": agent.name,
        "description": agent.description,
        "status": agent.status,
        "registered_at": agent.registered_at,
        "placement": None,
    }
    if placement_row is not None:
        result["placement"] = {
            "director_agent_id": placement_row.director_agent_id,
            "tmux_session": placement_row.tmux_session,
            "tmux_window_id": placement_row.tmux_window_id,
            "tmux_pane_id": placement_row.tmux_pane_id,
            "coding_agent": placement_row.coding_agent,
            "created_at": placement_row.created_at,
        }
    return result


def list_agents(session_id: str) -> list[dict]:
    """Active agents in session."""
    stmt = select(
        Agent.agent_id,
        Agent.name,
        Agent.description,
        Agent.registered_at,
    ).where(
        Agent.session_id == session_id,
        Agent.status == "active",
    )
    sm = get_sync_sessionmaker()
    with sm() as session:
        rows = session.execute(stmt).all()
    return [
        {
            "agent_id": row.agent_id,
            "name": row.name,
            "description": row.description,
            "status": "active",
            "registered_at": row.registered_at,
        }
        for row in rows
    ]


def deregister_agent(agent_id: str) -> bool:
    """UPDATE status='deregistered', deregistered_at=now, DELETE placement.

    Returns True if agent was active and got deregistered.
    """
    sm = get_sync_sessionmaker()
    with sm() as session:
        with session.begin():
            result = cast(
                CursorResult,
                session.execute(
                    update(Agent)
                    .where(
                        Agent.agent_id == agent_id,
                        Agent.status == "active",
                    )
                    .values(
                        status="deregistered",
                        deregistered_at=_now_iso(),
                    )
                ),
            )
            if result.rowcount > 0:
                session.execute(
                    delete(AgentPlacement).where(
                        AgentPlacement.agent_id == agent_id
                    )
                )
    return result.rowcount > 0


def update_placement_pane_id(agent_id: str, pane_id: str) -> dict | None:
    """UPDATE agent_placements SET tmux_pane_id.

    Returns placement dict or None if no placement exists.
    """
    sm = get_sync_sessionmaker()
    with sm() as session:
        with session.begin():
            result = cast(
                CursorResult,
                session.execute(
                    update(AgentPlacement)
                    .where(AgentPlacement.agent_id == agent_id)
                    .values(tmux_pane_id=pane_id)
                ),
            )
            if result.rowcount == 0:
                return None

        # Re-read the placement after update
        row = session.execute(
            select(AgentPlacement).where(AgentPlacement.agent_id == agent_id)
        ).scalar_one_or_none()

    if row is None:
        return None
    return {
        "director_agent_id": row.director_agent_id,
        "tmux_session": row.tmux_session,
        "tmux_window_id": row.tmux_window_id,
        "tmux_pane_id": row.tmux_pane_id,
        "coding_agent": row.coding_agent,
        "created_at": row.created_at,
    }


def list_members(session_id: str, director_agent_id: str) -> list[dict]:
    """Member agents with placement info for a director.

    SELECT agents JOIN agent_placements WHERE director_agent_id.
    """
    stmt = (
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
            AgentPlacement.created_at.label("placement_created_at"),
        )
        .join(AgentPlacement, Agent.agent_id == AgentPlacement.agent_id)
        .where(
            Agent.session_id == session_id,
            Agent.status == "active",
            AgentPlacement.director_agent_id == director_agent_id,
        )
    )
    sm = get_sync_sessionmaker()
    with sm() as session:
        rows = session.execute(stmt).all()
    return [
        {
            "agent_id": row.agent_id,
            "name": row.name,
            "description": row.description,
            "status": row.status,
            "registered_at": row.registered_at,
            "placement": {
                "director_agent_id": row.director_agent_id,
                "tmux_session": row.tmux_session,
                "tmux_window_id": row.tmux_window_id,
                "tmux_pane_id": row.tmux_pane_id,
                "coding_agent": row.coding_agent,
                "created_at": row.placement_created_at,
            },
        }
        for row in rows
    ]


def verify_agent_session(agent_id: str, session_id: str) -> bool:
    """Check if agent belongs to session. Used by WebUI session validation."""
    sm = get_sync_sessionmaker()
    with sm() as session:
        result = session.execute(
            select(Agent.agent_id).where(
                Agent.agent_id == agent_id,
                Agent.session_id == session_id,
            )
        )
        return result.first() is not None
