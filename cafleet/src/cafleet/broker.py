"""Sync SQLAlchemy data-access layer shared by the CLI and WebUI."""

import json
import uuid
from datetime import UTC, datetime

import click
from sqlalchemy import and_, delete, exists, func, or_, select, update
from sqlalchemy.dialects.sqlite import insert as sqlite_insert

from cafleet.db.engine import get_sync_sessionmaker
from cafleet.db.models import Agent, AgentPlacement, Session, Task
from cafleet.tmux import DirectorContext

_DIRECTOR_NAME = "director"
_DIRECTOR_DESCRIPTION = "Root Director for this session"
# FIXME(claude): auto-detect from $CLAUDECODE / $CLAUDE_CODE_ENTRYPOINT / codex env vars.
_ROOT_DIRECTOR_CODING_AGENT = "unknown"

ADMINISTRATOR_KIND = "builtin-administrator"


class AdministratorProtectedError(Exception):
    """Raised when an operation targets a built-in Administrator agent."""


def _administrator_agent_card(session_id: str) -> dict:
    short_id = session_id[:8]
    return {
        "name": "Administrator",
        "description": f"Built-in administrator agent for session {short_id}",
        "skills": [],
        "cafleet": {"kind": ADMINISTRATOR_KIND},
    }


def _is_administrator_card(agent_card_json: str | None) -> bool:
    if not agent_card_json:
        return False
    try:
        card = json.loads(agent_card_json)
    except (ValueError, TypeError):
        return False
    if not isinstance(card, dict):
        return False
    cafleet_ns = card.get("cafleet")
    if not isinstance(cafleet_ns, dict):
        return False
    return cafleet_ns.get("kind") == ADMINISTRATOR_KIND


def _now_iso() -> str:
    return datetime.now(UTC).isoformat()


def _placement_dict(row, *, created_at_attr: str = "created_at") -> dict:
    return {
        "director_agent_id": row.director_agent_id,
        "tmux_session": row.tmux_session,
        "tmux_window_id": row.tmux_window_id,
        "tmux_pane_id": row.tmux_pane_id,
        "coding_agent": row.coding_agent,
        "created_at": getattr(row, created_at_attr),
    }


def _agent_is_active_in_session(session, agent_id: str, session_id: str) -> bool:
    return (
        session.execute(
            select(Agent.agent_id).where(
                Agent.agent_id == agent_id,
                Agent.session_id == session_id,
                Agent.status == "active",
            )
        ).first()
        is not None
    )


def _try_notify_recipient(
    session, *, session_id: str, recipient_id: str, sender_id: str
) -> bool:
    """Best-effort ``tmux send-keys`` poll trigger for the recipient's pane.

    The literal ``cafleet --session-id <sid> poll --agent-id <aid>`` string
    is injected so the recipient's ``permissions.allow`` matches it exactly;
    failures are swallowed because the queue remains the source of truth.
    """
    if recipient_id == sender_id:
        return False
    pane_id = session.execute(
        select(AgentPlacement.tmux_pane_id).where(
            AgentPlacement.agent_id == recipient_id
        )
    ).scalar_one_or_none()
    if pane_id is None:
        return False
    # Local import so tests that monkeypatch ``cafleet.tmux.send_poll_trigger``
    # get picked up on every call rather than bound once at broker import.
    from cafleet.tmux import send_poll_trigger

    return send_poll_trigger(
        target_pane_id=pane_id,
        session_id=session_id,
        agent_id=recipient_id,
    )


def create_session(
    label: str | None = None,
    *,
    director_context: DirectorContext,
) -> dict:
    """Atomically bootstrap a session with its root Director and Administrator.

    The session row is written first with ``director_agent_id=NULL`` and
    back-filled once the Director's agent row exists, so the column is
    DB-nullable even though the post-bootstrap invariant is NOT NULL.
    """
    session_id = str(uuid.uuid4())
    created_at = _now_iso()
    director_agent_id = str(uuid.uuid4())
    administrator_agent_id = str(uuid.uuid4())
    administrator_card = _administrator_agent_card(session_id)
    director_card = {
        "name": _DIRECTOR_NAME,
        "description": _DIRECTOR_DESCRIPTION,
        "skills": [],
    }
    director_placement = {
        "director_agent_id": None,
        "tmux_session": director_context.session,
        "tmux_window_id": director_context.window_id,
        "tmux_pane_id": director_context.pane_id,
        "coding_agent": _ROOT_DIRECTOR_CODING_AGENT,
        "created_at": created_at,
    }

    sm = get_sync_sessionmaker()
    with sm() as session, session.begin():
        session.add(
            Session(
                session_id=session_id,
                label=label,
                created_at=created_at,
                deleted_at=None,
                director_agent_id=None,
            )
        )
        session.flush()
        session.add(
            Agent(
                agent_id=director_agent_id,
                session_id=session_id,
                name=_DIRECTOR_NAME,
                description=_DIRECTOR_DESCRIPTION,
                status="active",
                registered_at=created_at,
                deregistered_at=None,
                agent_card_json=json.dumps(director_card),
            )
        )
        session.flush()
        session.add(AgentPlacement(agent_id=director_agent_id, **director_placement))
        session.flush()
        session.execute(
            update(Session)
            .where(Session.session_id == session_id)
            .values(director_agent_id=director_agent_id)
        )
        session.add(
            Agent(
                agent_id=administrator_agent_id,
                session_id=session_id,
                name=administrator_card["name"],
                description=administrator_card["description"],
                status="active",
                registered_at=created_at,
                deregistered_at=None,
                agent_card_json=json.dumps(administrator_card),
            )
        )

    return {
        "session_id": session_id,
        "label": label,
        "created_at": created_at,
        "administrator_agent_id": administrator_agent_id,
        "director": {
            "agent_id": director_agent_id,
            "name": _DIRECTOR_NAME,
            "description": _DIRECTOR_DESCRIPTION,
            "registered_at": created_at,
            "placement": director_placement,
        },
    }


def list_sessions() -> list[dict]:
    """Return non-soft-deleted sessions with their active agent counts."""
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
        .where(Session.deleted_at.is_(None))
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
    """Return the session row (including soft-deleted) or None.

    The returned dict exposes ``deleted_at`` so callers can distinguish a
    missing session from a soft-deleted one — ``register_agent`` relies on
    this to reject soft-deleted sessions with a different error message.
    """
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
        "deleted_at": row.deleted_at,
        "director_agent_id": row.director_agent_id,
    }


def delete_session(session_id: str) -> dict:
    """Soft-delete a session and deregister its agents, in one transaction.

    Tasks are left untouched so audit history survives. Idempotent: re-running
    against an already-deleted row short-circuits on the ``deleted_at IS NULL``
    guard and returns ``deregistered_count=0``.
    """
    now = _now_iso()
    sm = get_sync_sessionmaker()
    with sm() as session, session.begin():
        exists_row = session.execute(
            select(Session.session_id).where(Session.session_id == session_id)
        ).first()
        if exists_row is None:
            # ClickException exits 1 (matching ``session show``); UsageError
            # would print a Usage: banner + exit 2, wrong for a runtime miss.
            raise click.ClickException(f"session '{session_id}' not found.")

        soft_deleted = session.execute(
            update(Session)
            .where(
                Session.session_id == session_id,
                Session.deleted_at.is_(None),
            )
            .values(deleted_at=now)
            .returning(Session.session_id)
        ).all()
        if not soft_deleted:
            return {"deregistered_count": 0}

        deregistered = session.execute(
            update(Agent)
            .where(
                Agent.session_id == session_id,
                Agent.status == "active",
            )
            .values(status="deregistered", deregistered_at=now)
            .returning(Agent.agent_id)
        ).all()
        deregistered_count = len(deregistered)
        agents_in_session = select(Agent.agent_id).where(Agent.session_id == session_id)
        session.execute(
            delete(AgentPlacement).where(AgentPlacement.agent_id.in_(agents_in_session))
        )

    return {"deregistered_count": deregistered_count}


def register_agent(
    session_id: str,
    name: str,
    description: str,
    skills: list[dict] | None = None,
    placement: dict | None = None,
) -> dict:
    """Register a new agent in the session and optionally create its placement.

    Rejects soft-deleted sessions with a message that differs from the
    "not found" case so callers can surface the right recovery hint
    (design 0000026). When ``placement`` is supplied, the named Director
    must be active in the same session and must not be the Administrator.
    """
    sess = get_session(session_id)
    if sess is None:
        raise click.UsageError(f"Session '{session_id}' not found.")
    if sess.get("deleted_at") is not None:
        raise click.UsageError(f"session {session_id} is deleted")

    agent_id = str(uuid.uuid4())
    registered_at = _now_iso()
    agent_card = {
        "name": name,
        "description": description,
        "skills": skills or [],
    }

    sm = get_sync_sessionmaker()
    with sm() as session, session.begin():
        if placement is not None:
            director_id = placement["director_agent_id"]
            director_card = session.execute(
                select(Agent.agent_card_json).where(
                    Agent.agent_id == director_id,
                    Agent.session_id == session_id,
                    Agent.status == "active",
                )
            ).scalar_one_or_none()
            if director_card is None:
                raise click.UsageError(
                    f"Director agent '{director_id}' not found or not active "
                    f"in session '{session_id}'."
                )
            if _is_administrator_card(director_card):
                raise AdministratorProtectedError("Administrator cannot be a director")

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
    """Return the active agent's detail (with placement) or None."""
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

    result: dict = {
        "agent_id": agent.agent_id,
        "name": agent.name,
        "description": agent.description,
        "status": agent.status,
        "registered_at": agent.registered_at,
        "kind": (
            ADMINISTRATOR_KIND
            if _is_administrator_card(agent.agent_card_json)
            else "user"
        ),
        "placement": None,
    }
    if placement_row is not None:
        result["placement"] = _placement_dict(placement_row)
    return result


def list_agents(session_id: str) -> list[dict]:
    """Return all active agents in the session."""
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
    """Soft-delete the agent and drop its placement.

    Returns True if a row was flipped from ``active`` to ``deregistered``.
    Raises ``AdministratorProtectedError`` for the built-in Administrator
    (design 0000025 §D) and ``click.UsageError`` for the root Director of
    any session (design 0000026), which must be torn down via
    ``cafleet session delete``.
    """
    sm = get_sync_sessionmaker()
    with sm() as session, session.begin():
        is_root_director = (
            session.execute(
                select(Session.session_id).where(Session.director_agent_id == agent_id)
            ).first()
            is not None
        )
        if is_root_director:
            raise click.UsageError(
                "cannot deregister the root Director; "
                "use 'cafleet session delete' instead"
            )

        card_json = session.execute(
            select(Agent.agent_card_json).where(Agent.agent_id == agent_id)
        ).scalar_one_or_none()
        if card_json is not None and _is_administrator_card(card_json):
            raise AdministratorProtectedError("Administrator cannot be deregistered")
        deregistered = session.execute(
            update(Agent)
            .where(
                Agent.agent_id == agent_id,
                Agent.status == "active",
            )
            .values(
                status="deregistered",
                deregistered_at=_now_iso(),
            )
            .returning(Agent.agent_id)
        ).all()
        if deregistered:
            session.execute(
                delete(AgentPlacement).where(AgentPlacement.agent_id == agent_id)
            )
    return bool(deregistered)


def update_placement_pane_id(agent_id: str, pane_id: str) -> dict | None:
    """Patch the pane id of an existing placement and return the new row."""
    sm = get_sync_sessionmaker()
    with sm() as session, session.begin():
        updated = session.execute(
            update(AgentPlacement)
            .where(AgentPlacement.agent_id == agent_id)
            .values(tmux_pane_id=pane_id)
            .returning(AgentPlacement.agent_id)
        ).first()
        if updated is None:
            return None
        row = session.execute(
            select(AgentPlacement).where(AgentPlacement.agent_id == agent_id)
        ).scalar_one_or_none()

    if row is None:
        return None
    return _placement_dict(row)


def list_members(session_id: str, director_agent_id: str) -> list[dict]:
    """Return active members belonging to the given director, with placements."""
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
            "placement": _placement_dict(row, created_at_attr="placement_created_at"),
        }
        for row in rows
    ]


def verify_agent_session(agent_id: str, session_id: str) -> bool:
    """Return True iff the agent belongs to the session (any status)."""
    sm = get_sync_sessionmaker()
    with sm() as session:
        return (
            session.execute(
                select(Agent.agent_id).where(
                    Agent.agent_id == agent_id,
                    Agent.session_id == session_id,
                )
            ).first()
            is not None
        )


def _save_task(session, task_dict: dict) -> None:
    """UPSERT the task, promoting indexed fields from the metadata blob."""
    metadata = task_dict["metadata"]
    stmt = sqlite_insert(Task).values(
        task_id=task_dict["id"],
        context_id=task_dict["contextId"],
        from_agent_id=metadata["fromAgentId"],
        # ``broadcast_summary`` has no recipient; store empty so the NOT NULL
        # column stays satisfied and the row still shows up in sender queries.
        to_agent_id=metadata.get("toAgentId", ""),
        type=metadata["type"],
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
    task_json = session.execute(
        select(Task.task_json).where(Task.task_id == task_id)
    ).scalar_one_or_none()
    if task_json is None:
        return None
    return json.loads(task_json)


def _unicast_task_dict(
    *,
    recipient_id: str,
    sender_id: str,
    text: str,
    now: str,
    origin_task_id: str | None = None,
) -> dict:
    metadata: dict = {
        "fromAgentId": sender_id,
        "toAgentId": recipient_id,
        "type": "unicast",
    }
    if origin_task_id is not None:
        metadata["originTaskId"] = origin_task_id
    return {
        "id": str(uuid.uuid4()),
        "contextId": recipient_id,
        "status": {"state": "input_required", "timestamp": now},
        "artifacts": [
            {
                "artifactId": str(uuid.uuid4()),
                "parts": [{"kind": "text", "text": text}],
            }
        ],
        "metadata": metadata,
        "history": [],
    }


def send_message(session_id: str, agent_id: str, to: str, text: str) -> dict:
    """Create a unicast task addressed to ``to`` and best-effort notify it."""
    try:
        uuid.UUID(to)
    except ValueError as exc:
        raise ValueError(f"Invalid destination format: {to}") from exc

    sm = get_sync_sessionmaker()
    with sm() as session, session.begin():
        if not _agent_is_active_in_session(session, agent_id, session_id):
            raise ValueError(
                f"Sender agent not found or not active in session: {agent_id}"
            )

        dest_session = session.execute(
            select(Agent.session_id).where(
                Agent.agent_id == to,
                Agent.status == "active",
            )
        ).scalar_one_or_none()
        if dest_session is None:
            raise ValueError(f"Destination agent not found: {to}")
        if dest_session != session_id:
            raise ValueError(f"Destination agent not in session: {to}")

        task_dict = _unicast_task_dict(
            recipient_id=to,
            sender_id=agent_id,
            text=text,
            now=_now_iso(),
        )
        _save_task(session, task_dict)
        notification_sent = _try_notify_recipient(
            session,
            session_id=session_id,
            recipient_id=to,
            sender_id=agent_id,
        )

    return {"task": task_dict, "notification_sent": notification_sent}


def broadcast_message(session_id: str, agent_id: str, text: str) -> list[dict]:
    """Fan out one delivery task per active non-admin peer plus a sender summary.

    Administrators are excluded at the SQL layer via ``json_extract`` so the
    card blob stays in the database; they are write-only identities per
    design 0000025 §E.
    """
    summary_task_id = str(uuid.uuid4())

    sm = get_sync_sessionmaker()
    with sm() as session, session.begin():
        if not _agent_is_active_in_session(session, agent_id, session_id):
            raise ValueError(
                f"Sender agent not found or not active in session: {agent_id}"
            )

        recipient_ids = list(
            session.execute(
                select(Agent.agent_id).where(
                    Agent.session_id == session_id,
                    Agent.status == "active",
                    Agent.agent_id != agent_id,
                    func.coalesce(
                        func.json_extract(Agent.agent_card_json, "$.cafleet.kind"),
                        "",
                    )
                    != ADMINISTRATOR_KIND,
                )
            ).scalars()
        )

        for recipient_id in recipient_ids:
            delivery_dict = _unicast_task_dict(
                recipient_id=recipient_id,
                sender_id=agent_id,
                text=text,
                now=_now_iso(),
                origin_task_id=summary_task_id,
            )
            _save_task(session, delivery_dict)

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

        notifications_sent_count = sum(
            _try_notify_recipient(
                session,
                session_id=session_id,
                recipient_id=recipient_id,
                sender_id=agent_id,
            )
            for recipient_id in recipient_ids
        )

    summary_dict["metadata"]["notificationsSentCount"] = notifications_sent_count
    return [
        {"task": summary_dict, "notifications_sent_count": notifications_sent_count}
    ]


def poll_tasks(
    agent_id: str,
    since: str | None = None,
    page_size: int | None = None,
    status: str | None = None,
) -> list[dict]:
    """Return tasks addressed to ``agent_id`` in DESC timestamp order."""
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
    """Transition a task from ``input_required`` to ``completed`` for the recipient."""
    sm = get_sync_sessionmaker()
    with sm() as session, session.begin():
        task_dict = _read_task(session, task_id)
        if task_dict is None:
            raise ValueError(f"Task {task_id} not found")

        if task_dict["contextId"] != agent_id:
            raise PermissionError("Only the recipient can ACK a task")

        if task_dict["status"]["state"] != "input_required":
            raise ValueError(f"Cannot ACK task in state {task_dict['status']['state']}")

        now = _now_iso()
        task_dict["status"] = {
            "state": "completed",
            "timestamp": now,
        }

        _save_task(session, task_dict)

    return {"task": task_dict}


def cancel_task(agent_id: str, task_id: str) -> dict:
    """Transition a task from ``input_required`` to ``canceled`` for the sender."""
    sm = get_sync_sessionmaker()
    with sm() as session, session.begin():
        task_dict = _read_task(session, task_id)
        if task_dict is None:
            raise ValueError(f"Task {task_id} not found")

        if task_dict["metadata"]["fromAgentId"] != agent_id:
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


def list_session_agents(session_id: str) -> list[dict]:
    """Return active agents plus deregistered agents that still own tasks.

    ``kind`` is derived in SQL via ``json_extract`` so the card blob never
    leaves SQLite — otherwise we would materialize every row's JSON just to
    compute a one-token discriminator. ``coalesce`` handles cards without a
    ``cafleet.kind`` path by substituting an empty string.
    """
    has_tasks = exists().where(
        or_(
            Task.context_id == Agent.agent_id,
            Task.from_agent_id == Agent.agent_id,
        )
    )
    kind_expr = func.coalesce(
        func.json_extract(Agent.agent_card_json, "$.cafleet.kind"), ""
    )
    stmt = select(
        Agent.agent_id,
        Agent.name,
        Agent.description,
        Agent.status,
        Agent.registered_at,
        kind_expr.label("kind_raw"),
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
            "kind": (
                ADMINISTRATOR_KIND if row.kind_raw == ADMINISTRATOR_KIND else "user"
            ),
        }
        for row in rows
    ]


def _list_tasks_where(*filters) -> list[dict]:
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
        .where(*filters, Task.type != "broadcast_summary")
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


def list_inbox(agent_id: str) -> list[dict]:
    """Return raw task rows addressed to ``agent_id`` (no broadcast_summary)."""
    return _list_tasks_where(Task.context_id == agent_id)


def list_sent(agent_id: str) -> list[dict]:
    """Return raw task rows sent by ``agent_id`` (no broadcast_summary)."""
    return _list_tasks_where(Task.from_agent_id == agent_id)


def list_timeline(session_id: str, limit: int = 200) -> list[dict]:
    """Return the session's recent tasks in DESC timestamp order."""
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
    """Batch ``agent_id → name`` lookup including deregistered agents."""
    if not agent_ids:
        return {}
    sm = get_sync_sessionmaker()
    with sm() as session:
        rows = session.execute(
            select(Agent.agent_id, Agent.name).where(Agent.agent_id.in_(agent_ids))
        ).all()
    return {row.agent_id: row.name for row in rows}


def get_task_created_ats(task_ids: list[str]) -> dict[str, str]:
    """Batch ``task_id → created_at`` lookup."""
    if not task_ids:
        return {}
    sm = get_sync_sessionmaker()
    with sm() as session:
        rows = session.execute(
            select(Task.task_id, Task.created_at).where(Task.task_id.in_(task_ids))
        ).all()
    return {row.task_id: row.created_at for row in rows}


def get_task(session_id: str, task_id: str) -> dict:
    """Return the task iff at least one of its endpoints lives in the session."""
    sm = get_sync_sessionmaker()
    with sm() as session:
        task_dict = _read_task(session, task_id)
        if task_dict is None:
            raise ValueError(f"Task {task_id} not found")

        metadata = task_dict["metadata"]
        endpoint_ids = [
            aid
            for aid in (metadata["fromAgentId"], metadata.get("toAgentId", ""))
            if aid
        ]
        in_session = session.execute(
            select(Agent.agent_id).where(
                Agent.agent_id.in_(endpoint_ids),
                Agent.session_id == session_id,
            )
        ).first()
        if in_session is None:
            raise ValueError(f"Task {task_id} not found")

    return {"task": task_dict}
