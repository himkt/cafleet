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
from sqlalchemy import and_, delete, exists, func, or_, select, update
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


# ---------------------------------------------------------------------------
# Internal helpers (messaging)
# ---------------------------------------------------------------------------


def _save_task(session, task_dict: dict) -> None:
    """INSERT with UPSERT. Promotes indexed fields to columns.

    Preserves created_at on re-save.
    """
    metadata = task_dict.get("metadata", {})
    stmt = sqlite_insert(Task).values(
        task_id=task_dict["id"],
        context_id=task_dict["contextId"],
        from_agent_id=metadata.get("fromAgentId", ""),
        to_agent_id=metadata.get("toAgentId", ""),
        type=metadata.get("type", ""),
        created_at=_now_iso(),
        status_state=task_dict["status"]["state"],
        status_timestamp=task_dict["status"]["timestamp"],
        origin_task_id=metadata.get("originTaskId"),
        task_json=json.dumps(task_dict),
    )
    stmt = stmt.on_conflict_do_update(
        index_elements=["task_id"],
        set_={
            "status_state": stmt.excluded.status_state,
            "status_timestamp": stmt.excluded.status_timestamp,
            "origin_task_id": stmt.excluded.origin_task_id,
            "task_json": stmt.excluded.task_json,
        },
    )
    session.execute(stmt)


def _read_task(session, task_id: str) -> dict | None:
    """SELECT task_json by task_id. Returns parsed dict or None."""
    row = session.execute(
        select(Task.task_json).where(Task.task_id == task_id)
    ).first()
    if row is None:
        return None
    return json.loads(row[0])


# ---------------------------------------------------------------------------
# Messaging operations
# ---------------------------------------------------------------------------


def send_message(session_id: str, agent_id: str, to: str, text: str) -> dict:
    """Unicast. Returns {"task": <camelCase task dict>}.

    Validation:
      1. Destination is valid UUID
      2. Destination agent exists and status='active'
      3. Destination agent is in the same session
    Creates task: status.state='input_required', context_id=destination.
    """
    # 1. Validate destination UUID
    try:
        uuid.UUID(to)
    except ValueError:
        raise ValueError(f"Invalid destination format: {to}")

    sm = get_sync_sessionmaker()
    with sm() as session:
        with session.begin():
            # 2. Destination agent exists and is active
            dest_agent = session.execute(
                select(Agent).where(
                    Agent.agent_id == to,
                    Agent.status == "active",
                )
            ).scalar_one_or_none()
            if dest_agent is None:
                raise ValueError(f"Destination agent not found: {to}")

            # 3. Destination agent is in the same session
            if dest_agent.session_id != session_id:
                raise ValueError(
                    f"Destination agent not in session: {to}"
                )

            now = _now_iso()
            task_dict = {
                "id": str(uuid.uuid4()),
                "contextId": to,
                "status": {
                    "state": "input_required",
                    "timestamp": now,
                },
                "artifacts": [
                    {
                        "artifactId": str(uuid.uuid4()),
                        "parts": [{"kind": "text", "text": text}],
                    }
                ],
                "metadata": {
                    "fromAgentId": agent_id,
                    "toAgentId": to,
                    "type": "unicast",
                },
                "history": [],
            }

            _save_task(session, task_dict)

    return {"task": task_dict}


def broadcast_message(session_id: str, agent_id: str, text: str) -> list[dict]:
    """Broadcast. Returns [{"task": <summary task dict>}].

    Lists active agents in session (excluding sender). Creates one delivery task
    per recipient (type='unicast', originTaskId=summary_id) plus one summary
    (type='broadcast_summary', context_id=sender).
    """
    summary_task_id = str(uuid.uuid4())

    sm = get_sync_sessionmaker()
    with sm() as session:
        with session.begin():
            # List active agents in session, excluding sender
            rows = session.execute(
                select(Agent.agent_id).where(
                    Agent.session_id == session_id,
                    Agent.status == "active",
                    Agent.agent_id != agent_id,
                )
            ).all()
            recipient_ids = [row[0] for row in rows]

            # Create delivery tasks for each recipient
            for recipient_id in recipient_ids:
                now = _now_iso()
                delivery_dict = {
                    "id": str(uuid.uuid4()),
                    "contextId": recipient_id,
                    "status": {
                        "state": "input_required",
                        "timestamp": now,
                    },
                    "artifacts": [
                        {
                            "artifactId": str(uuid.uuid4()),
                            "parts": [{"kind": "text", "text": text}],
                        }
                    ],
                    "metadata": {
                        "fromAgentId": agent_id,
                        "toAgentId": recipient_id,
                        "type": "unicast",
                        "originTaskId": summary_task_id,
                    },
                    "history": [],
                }
                _save_task(session, delivery_dict)

            # Create summary task
            now = _now_iso()
            summary_dict = {
                "id": summary_task_id,
                "contextId": agent_id,
                "status": {
                    "state": "completed",
                    "timestamp": now,
                },
                "artifacts": [
                    {
                        "artifactId": str(uuid.uuid4()),
                        "parts": [
                            {
                                "kind": "text",
                                "text": f"Broadcast sent to {len(recipient_ids)} recipients",
                            }
                        ],
                    }
                ],
                "metadata": {
                    "fromAgentId": agent_id,
                    "type": "broadcast_summary",
                    "recipientCount": len(recipient_ids),
                    "recipientIds": recipient_ids,
                    "originTaskId": summary_task_id,
                },
                "history": [],
            }
            _save_task(session, summary_dict)

    return [{"task": summary_dict}]


def poll_tasks(
    agent_id: str,
    since: str | None = None,
    page_size: int | None = None,
    status: str | None = None,
) -> list[dict]:
    """Inbox query. Returns [<camelCase task dict>, ...].

    SELECT WHERE context_id=agent_id, ORDER BY status_timestamp DESC.
    Filters out type='broadcast_summary'.
    """
    stmt = (
        select(Task.task_json)
        .where(
            Task.context_id == agent_id,
            Task.type != "broadcast_summary",
        )
        .order_by(Task.status_timestamp.desc())
    )

    if since is not None:
        stmt = stmt.where(Task.status_timestamp > since)
    if status is not None:
        stmt = stmt.where(Task.status_state == status)
    if page_size is not None:
        stmt = stmt.limit(page_size)

    sm = get_sync_sessionmaker()
    with sm() as session:
        rows = session.execute(stmt).all()

    return [json.loads(row[0]) for row in rows]


def ack_task(agent_id: str, task_id: str) -> dict:
    """ACK. Returns {"task": <updated task dict>}.

    Verifies context_id == agent_id. Verifies state == 'input_required'.
    Transitions to 'completed'.
    """
    sm = get_sync_sessionmaker()
    with sm() as session:
        with session.begin():
            task_dict = _read_task(session, task_id)
            if task_dict is None:
                raise ValueError(f"Task {task_id} not found")

            if task_dict["contextId"] != agent_id:
                raise PermissionError("Only the recipient can ACK a task")

            if task_dict["status"]["state"] != "input_required":
                raise ValueError(
                    f"Cannot ACK task in state {task_dict['status']['state']}"
                )

            now = _now_iso()
            task_dict["status"] = {
                "state": "completed",
                "timestamp": now,
            }

            _save_task(session, task_dict)

    return {"task": task_dict}


def cancel_task(agent_id: str, task_id: str) -> dict:
    """Cancel. Returns {"task": <updated task dict>}.

    Verifies metadata.fromAgentId == agent_id. Verifies state == 'input_required'.
    Transitions to 'canceled'.
    """
    sm = get_sync_sessionmaker()
    with sm() as session:
        with session.begin():
            task_dict = _read_task(session, task_id)
            if task_dict is None:
                raise ValueError(f"Task {task_id} not found")

            metadata = task_dict.get("metadata", {})
            if metadata.get("fromAgentId") != agent_id:
                raise PermissionError("Only the sender can cancel a task")

            if task_dict["status"]["state"] != "input_required":
                raise ValueError(
                    f"Cannot cancel task in state {task_dict['status']['state']}"
                )

            now = _now_iso()
            task_dict["status"] = {
                "state": "canceled",
                "timestamp": now,
            }

            _save_task(session, task_dict)

    return {"task": task_dict}


# ---------------------------------------------------------------------------
# WebUI query operations
# ---------------------------------------------------------------------------


def list_session_agents(session_id: str) -> list[dict]:
    """Active agents + deregistered agents that have tasks.

    Returns list of dicts with keys: agent_id, name, description, status,
    registered_at.  Deregistered agents appear only when they are referenced
    by at least one task (as sender or recipient).
    """
    has_tasks = exists().where(
        or_(
            Task.context_id == Agent.agent_id,
            Task.from_agent_id == Agent.agent_id,
        )
    )
    stmt = select(
        Agent.agent_id,
        Agent.name,
        Agent.description,
        Agent.status,
        Agent.registered_at,
    ).where(
        Agent.session_id == session_id,
        or_(
            Agent.status == "active",
            and_(Agent.status == "deregistered", has_tasks),
        ),
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
        }
        for row in rows
    ]


def list_inbox(agent_id: str) -> list[dict]:
    """Inbox tasks as raw row dicts.

    SELECT WHERE context_id=agent_id, filters broadcast_summary,
    ordered by status_timestamp DESC.
    """
    stmt = (
        select(
            Task.task_id,
            Task.context_id,
            Task.from_agent_id,
            Task.to_agent_id,
            Task.type,
            Task.created_at,
            Task.status_state,
            Task.status_timestamp,
            Task.origin_task_id,
            Task.task_json,
        )
        .where(
            Task.context_id == agent_id,
            Task.type != "broadcast_summary",
        )
        .order_by(Task.status_timestamp.desc())
    )
    sm = get_sync_sessionmaker()
    with sm() as session:
        rows = session.execute(stmt).all()
    return [
        {
            "task_id": row.task_id,
            "context_id": row.context_id,
            "from_agent_id": row.from_agent_id,
            "to_agent_id": row.to_agent_id,
            "type": row.type,
            "created_at": row.created_at,
            "status_state": row.status_state,
            "status_timestamp": row.status_timestamp,
            "origin_task_id": row.origin_task_id,
            "task_json": row.task_json,
        }
        for row in rows
    ]


def list_sent(agent_id: str) -> list[dict]:
    """Sent tasks as raw row dicts.

    SELECT WHERE from_agent_id=agent_id, filters broadcast_summary,
    ordered by status_timestamp DESC.
    """
    stmt = (
        select(
            Task.task_id,
            Task.context_id,
            Task.from_agent_id,
            Task.to_agent_id,
            Task.type,
            Task.created_at,
            Task.status_state,
            Task.status_timestamp,
            Task.origin_task_id,
            Task.task_json,
        )
        .where(
            Task.from_agent_id == agent_id,
            Task.type != "broadcast_summary",
        )
        .order_by(Task.status_timestamp.desc())
    )
    sm = get_sync_sessionmaker()
    with sm() as session:
        rows = session.execute(stmt).all()
    return [
        {
            "task_id": row.task_id,
            "context_id": row.context_id,
            "from_agent_id": row.from_agent_id,
            "to_agent_id": row.to_agent_id,
            "type": row.type,
            "created_at": row.created_at,
            "status_state": row.status_state,
            "status_timestamp": row.status_timestamp,
            "origin_task_id": row.origin_task_id,
            "task_json": row.task_json,
        }
        for row in rows
    ]


def list_timeline(session_id: str, limit: int = 200) -> list[dict]:
    """Session-wide timeline. Returns structured entries.

    Each entry: {"task": <parsed dict>, "origin_task_id": ..., "created_at": ...}.
    Filters broadcast_summary. Scoped to session via JOIN on from_agent_id.
    Ordered by status_timestamp DESC.
    """
    stmt = (
        select(
            Task.task_id,
            Task.origin_task_id,
            Task.created_at,
            Task.status_timestamp,
            Task.task_json,
        )
        .join(Agent, Task.from_agent_id == Agent.agent_id)
        .where(
            Agent.session_id == session_id,
            Task.type != "broadcast_summary",
        )
        .order_by(Task.status_timestamp.desc())
        .limit(limit)
    )
    sm = get_sync_sessionmaker()
    with sm() as session:
        rows = session.execute(stmt).all()
    return [
        {
            "task": json.loads(row.task_json),
            "origin_task_id": row.origin_task_id,
            "created_at": row.created_at,
        }
        for row in rows
    ]


def get_agent_names(agent_ids: list[str]) -> dict[str, str]:
    """Batch lookup: {agent_id: name}. Includes deregistered agents."""
    if not agent_ids:
        return {}
    sm = get_sync_sessionmaker()
    with sm() as session:
        rows = session.execute(
            select(Agent.agent_id, Agent.name).where(
                Agent.agent_id.in_(agent_ids)
            )
        ).all()
    return {row.agent_id: row.name for row in rows}


def get_task_created_ats(task_ids: list[str]) -> dict[str, str]:
    """Batch lookup: {task_id: created_at}."""
    if not task_ids:
        return {}
    sm = get_sync_sessionmaker()
    with sm() as session:
        rows = session.execute(
            select(Task.task_id, Task.created_at).where(
                Task.task_id.in_(task_ids)
            )
        ).all()
    return {row.task_id: row.created_at for row in rows}


def get_task(session_id: str, task_id: str) -> dict:
    """Get task. Returns {"task": <task dict>}.

    Verifies fromAgentId or toAgentId belongs to session.
    """
    sm = get_sync_sessionmaker()
    with sm() as session:
        task_dict = _read_task(session, task_id)
        if task_dict is None:
            raise ValueError(f"Task {task_id} not found")

        metadata = task_dict.get("metadata", {})
        from_id = metadata.get("fromAgentId", "")
        to_id = metadata.get("toAgentId", "")

        # Verify at least one of the agents belongs to the given session
        from_ok = (
            session.execute(
                select(Agent.agent_id).where(
                    Agent.agent_id == from_id,
                    Agent.session_id == session_id,
                )
            ).first()
            is not None
        )
        to_ok = (
            session.execute(
                select(Agent.agent_id).where(
                    Agent.agent_id == to_id,
                    Agent.session_id == session_id,
                )
            ).first()
            is not None
        )

        if not from_ok and not to_ok:
            raise ValueError(f"Task {task_id} not found")

    return {"task": task_dict}
