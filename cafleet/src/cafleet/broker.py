"""Sync SQLAlchemy data-access layer shared by the CLI and WebUI."""

import json
import uuid
from datetime import UTC, datetime

import click
from sqlalchemy import and_, delete, exists, func, or_, select, update
from sqlalchemy.dialects.sqlite import insert as sqlite_insert

from cafleet.config import settings
from cafleet.db.engine import get_sync_sessionmaker
from cafleet.db.models import Agent, AgentPlacement, Session, Task
from cafleet.multiplexer import MultiplexerContext

_DIRECTOR_NAME = "Director"
_DIRECTOR_DESCRIPTION = "Root Director for this session"

ADMINISTRATOR_KIND = "builtin-administrator"

_TASK_COLUMNS = tuple(Task.__table__.columns.keys())

_NOT_BROADCAST_SUMMARY = Task.type != "broadcast_summary"


def _is_administrator(agent_card_json: str | None) -> bool:
    if not agent_card_json:
        return False
    try:
        kind = json.loads(agent_card_json).get("cafleet", {}).get("kind")
    except ValueError:
        return False
    return kind == ADMINISTRATOR_KIND


def _now_iso() -> str:
    return datetime.now(UTC).isoformat()


def _placement_dict(row) -> dict:
    return {
        "director_agent_id": row.director_agent_id,
        "tmux_session": row.tmux_session,
        "tmux_window_id": row.tmux_window_id,
        "tmux_pane_id": row.tmux_pane_id,
        "coding_agent": row.coding_agent,
        "created_at": row.created_at,
    }


def _agent_is_active_in_session(session, agent_id: str, session_id: str) -> bool:
    return session.execute(
        select(
            exists().where(
                Agent.agent_id == agent_id,
                Agent.session_id == session_id,
                Agent.status == "active",
            )
        )
    ).scalar_one()


def _try_notify_recipient(
    session, *, recipient_id: str, sender_id: str, task_dict: dict
) -> bool:
    """Best-effort inline-preview keystroke for the recipient's pane.

    Keystrokes a 2-line preview of the message itself into the recipient's
    pane — the recipient's TUI processes the keystrokes as a fresh user-turn
    input and the recipient acks via
    ``cafleet message ack --task-id <id>``. The queue remains the source
    of truth; failures are swallowed.
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
    # Local import so tests that monkeypatch
    # ``cafleet.multiplexer.tmux.TmuxMultiplexer.send_inline_preview``
    # get picked up on every call rather than bound once at broker import.
    from cafleet.multiplexer.tmux import TmuxMultiplexer

    # Truncate before keystroking so a multi-KB body cannot dump itself into
    # the recipient's pane. Mirrors output.truncate_text's contract: same
    # limit (``settings.max_text_len`` / ``CAFLEET_MAX_TEXT_LEN``, default
    # 200) and same single-codepoint U+2026 suffix on overflow.
    preview_text = task_dict["text"]
    if len(preview_text) > settings.max_text_len:
        preview_text = preview_text[: settings.max_text_len] + "…"

    return TmuxMultiplexer().send_inline_preview(
        target_pane_id=pane_id,
        task_id_8=task_dict["task_id"][:8],
        sender_8=sender_id[:8],
        ts=task_dict["status_timestamp"],
        text=preview_text,
    )


def create_session(
    label: str | None = None,
    *,
    director_context: MultiplexerContext,
    coding_agent: str,
) -> dict:
    """Atomically bootstrap a session with its root Director and Administrator.

    The session row is written first with ``director_agent_id=NULL`` and
    back-filled once the Director's agent row exists, so the column is
    DB-nullable even though the post-bootstrap invariant is NOT NULL.

    Args:
        label: Optional human-readable label for the session.
        director_context: Resolved tmux pane identity for the root Director,
            obtained via ``Multiplexer.context_discovery``.
        coding_agent: Operator-declared metadata that lands in the root
            Director's ``placement.coding_agent`` column. The CLI is the only
            caller and always supplies it (default ``'claude'`` lives at the
            Click layer).

    Returns:
        A dict carrying ``session_id``, ``label``, ``created_at``,
        ``administrator_agent_id``, and a ``director`` sub-dict with the
        Director's identity and placement metadata.
    """
    session_id = str(uuid.uuid4())
    created_at = _now_iso()
    director_agent_id = str(uuid.uuid4())
    administrator_agent_id = str(uuid.uuid4())
    administrator_card = {
        "name": "Administrator",
        "description": f"Built-in administrator agent for session {session_id[:8]}",
        "skills": [],
        "cafleet": {"kind": ADMINISTRATOR_KIND},
    }
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
        "coding_agent": coding_agent,
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
            Session.director_agent_id,
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
        .group_by(
            Session.session_id,
            Session.director_agent_id,
            Session.label,
            Session.created_at,
        )
        .order_by(Session.created_at.desc(), Session.session_id.asc())
    )
    sm = get_sync_sessionmaker()
    with sm() as session:
        rows = session.execute(stmt).all()
    return [
        {
            "session_id": row.session_id,
            "director_agent_id": row.director_agent_id,
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

    Args:
        session_id: Session UUID to look up.

    Returns:
        Dict with ``session_id``, ``label``, ``created_at``, ``deleted_at``,
        and ``director_agent_id``, or ``None`` if no row exists.
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

    Args:
        session_id: Session UUID to soft-delete.

    Returns:
        Dict with ``deregistered_count`` — the number of agents flipped from
        ``active`` to ``deregistered`` by this call.

    Raises:
        click.ClickException: If the session does not exist.
    """
    now = _now_iso()
    sm = get_sync_sessionmaker()
    with sm() as session, session.begin():
        session_exists = session.execute(
            select(exists().where(Session.session_id == session_id))
        ).scalar_one()
        if not session_exists:
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
    "not found" case so callers can surface the right recovery hint. When
    ``placement`` is supplied, the named Director must be active in the
    same session and must not be the Administrator.

    Args:
        session_id: Session UUID the new agent belongs to.
        name: Short human-identifiable label.
        description: One-sentence purpose statement.
        skills: Optional list of skill dicts persisted into the agent's
            ``agent_card_json`` blob.
        placement: Optional dict carrying ``director_agent_id``,
            ``tmux_session``, ``tmux_window_id``, ``tmux_pane_id``, and
            ``coding_agent``. When present, an ``AgentPlacement`` row is
            created alongside the agent.

    Returns:
        Dict with ``agent_id``, ``name``, and ``registered_at``.

    Raises:
        click.UsageError: If the session does not exist, is soft-deleted, or
            the named Director is not active in the same session.
        click.ClickException: If the named Director is the built-in
            Administrator.
    """
    sess = get_session(session_id)
    if sess is None:
        raise click.UsageError(f"Session '{session_id}' not found.")
    if sess["deleted_at"] is not None:
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
            if _is_administrator(director_card):
                raise click.ClickException("Administrator cannot be a director")

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
                    tmux_pane_id=placement["tmux_pane_id"],
                    coding_agent=placement["coding_agent"],
                    created_at=registered_at,
                )
            )

    return {
        "agent_id": agent_id,
        "name": name,
        "registered_at": registered_at,
    }


def get_agent(agent_id: str, session_id: str) -> dict | None:
    """Return the active agent's detail (with placement) or None.

    Args:
        agent_id: Agent UUID to look up.
        session_id: Session UUID the agent must belong to.

    Returns:
        Dict with ``agent_id``, ``name``, ``description``, ``status``,
        ``registered_at``, ``kind`` (``"user"`` or the Administrator kind),
        and ``placement`` (the placement sub-dict or ``None``). Returns
        ``None`` if no active agent matches.
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

    result: dict = {
        "agent_id": agent.agent_id,
        "name": agent.name,
        "description": agent.description,
        "status": agent.status,
        "registered_at": agent.registered_at,
        "kind": (
            ADMINISTRATOR_KIND if _is_administrator(agent.agent_card_json) else "user"
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

    Args:
        agent_id: Agent UUID to deregister.

    Returns:
        ``True`` if a row was flipped from ``active`` to ``deregistered``;
        ``False`` if no matching active agent existed.

    Raises:
        click.UsageError: If ``agent_id`` is the root Director of any
            session — torn down via ``cafleet session delete`` instead.
        click.ClickException: If ``agent_id`` is the built-in Administrator.
    """
    sm = get_sync_sessionmaker()
    with sm() as session, session.begin():
        is_root_director = session.execute(
            select(exists().where(Session.director_agent_id == agent_id))
        ).scalar_one()
        if is_root_director:
            raise click.UsageError(
                "cannot deregister the root Director; "
                "use 'cafleet session delete' instead"
            )

        card_json = session.execute(
            select(Agent.agent_card_json).where(Agent.agent_id == agent_id)
        ).scalar_one_or_none()
        if card_json is not None and _is_administrator(card_json):
            raise click.ClickException("Administrator cannot be deregistered")
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
    """Patch the agent's placement with a freshly resolved tmux pane id.

    Called after ``split_window`` returns the spawned pane's id so the
    placement row reflects the live pane rather than the placeholder used
    during the initial INSERT.

    Args:
        agent_id: Agent UUID whose placement should be updated.
        pane_id: New ``tmux_pane_id`` value.

    Returns:
        The refreshed placement dict, or ``None`` if no placement row was
        affected.
    """
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
        ).scalar_one()
    return _placement_dict(row)


def _base_members_select(session_id: str, director_agent_id: str):
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
            Agent.session_id == session_id,
            Agent.status == "active",
            AgentPlacement.director_agent_id == director_agent_id,
        )
    )


def list_members(session_id: str, director_agent_id: str) -> list[dict]:
    """Return active members belonging to the given director, with placements.

    Args:
        session_id: Session UUID to scope the query to.
        director_agent_id: Director UUID; only members whose
            ``placement.director_agent_id`` matches are returned.

    Returns:
        List of dicts each carrying ``agent_id``, ``name``, ``description``,
        ``status``, ``registered_at``, and ``placement``.
    """
    stmt = _base_members_select(session_id, director_agent_id)
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
            "placement": _placement_dict(row),
        }
        for row in rows
    ]


def list_members_with_activity(session_id: str, director_agent_id: str) -> list[dict]:
    """``list_members`` plus per-member activity proxies sourced from ``tasks``.

    ``last_sent`` / ``last_recv`` / ``last_ack`` aggregate ``status_timestamp``
    over the ``tasks`` table per agent. All three filter ``Task.type !=
    'broadcast_summary'`` (mirrors ``poll_tasks``); broadcast_summary rows
    land in the broadcaster's own context with ``status_state='completed'``
    and would otherwise pollute every proxy for the broadcaster.

    Args:
        session_id: Session UUID to scope the query to.
        director_agent_id: Director UUID whose members will be enumerated.

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
            _NOT_BROADCAST_SUMMARY,
        )
        .correlate(Agent)
        .scalar_subquery()
    )
    last_recv_sq = (
        select(func.max(Task.status_timestamp))
        .where(
            Task.context_id == Agent.agent_id,
            _NOT_BROADCAST_SUMMARY,
        )
        .correlate(Agent)
        .scalar_subquery()
    )
    last_ack_sq = (
        select(func.max(Task.status_timestamp))
        .where(
            Task.context_id == Agent.agent_id,
            Task.status_state == "completed",
            _NOT_BROADCAST_SUMMARY,
        )
        .correlate(Agent)
        .scalar_subquery()
    )
    stmt = _base_members_select(session_id, director_agent_id).add_columns(
        last_sent_sq.label("last_sent"),
        last_recv_sq.label("last_recv"),
        last_ack_sq.label("last_ack"),
    )
    sm = get_sync_sessionmaker()
    with sm() as session:
        rows = session.execute(stmt).all()

    now = datetime.now(UTC)
    return [
        {
            "agent_id": row.agent_id,
            "name": row.name,
            "description": row.description,
            "status": row.status,
            "registered_at": row.registered_at,
            "placement": _placement_dict(row),
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


def verify_agent_session(agent_id: str, session_id: str) -> bool:
    """Return True iff the agent belongs to the session (any status).

    Args:
        agent_id: Agent UUID to verify.
        session_id: Session UUID to check membership against.

    Returns:
        ``True`` if a matching row exists; ``False`` otherwise. Status is
        ignored — deregistered agents still pass.
    """
    sm = get_sync_sessionmaker()
    with sm() as session:
        return session.execute(
            select(
                exists().where(
                    Agent.agent_id == agent_id,
                    Agent.session_id == session_id,
                )
            )
        ).scalar_one()


def _row_to_task_dict(row) -> dict:
    return {col: getattr(row, col) for col in _TASK_COLUMNS}


def _save_task(session, task_dict: dict) -> None:
    """UPSERT the task; ``created_at`` is preserved across conflicts."""
    stmt = sqlite_insert(Task).values(
        task_id=task_dict["task_id"],
        context_id=task_dict["context_id"],
        from_agent_id=task_dict["from_agent_id"],
        to_agent_id=task_dict["to_agent_id"],
        type=task_dict["type"],
        created_at=task_dict["created_at"],
        status_state=task_dict["status_state"],
        status_timestamp=task_dict["status_timestamp"],
        origin_task_id=task_dict["origin_task_id"],
        text=task_dict["text"],
    )
    stmt = stmt.on_conflict_do_update(
        index_elements=["task_id"],
        set_={
            "status_state": stmt.excluded.status_state,
            "status_timestamp": stmt.excluded.status_timestamp,
            "origin_task_id": stmt.excluded.origin_task_id,
            "text": stmt.excluded.text,
        },
    )
    session.execute(stmt)


def _read_task(session, task_id: str) -> dict | None:
    row = session.execute(
        select(*(getattr(Task, col) for col in _TASK_COLUMNS)).where(
            Task.task_id == task_id
        )
    ).first()
    if row is None:
        return None
    return _row_to_task_dict(row)


def _unicast_task_dict(
    *,
    recipient_id: str,
    sender_id: str,
    text: str,
    now: str,
    origin_task_id: str | None = None,
) -> dict:
    return {
        "task_id": str(uuid.uuid4()),
        "context_id": recipient_id,
        "from_agent_id": sender_id,
        "to_agent_id": recipient_id,
        "type": "unicast",
        "created_at": now,
        "status_state": "input_required",
        "status_timestamp": now,
        "origin_task_id": origin_task_id,
        "text": text,
    }


def send_message(session_id: str, agent_id: str, to: str, text: str) -> dict:
    """Create a unicast task addressed to ``to`` and best-effort notify it.

    Persists a new ``Task`` row with ``type='unicast'`` and
    ``status_state='input_required'``, then calls
    ``_try_notify_recipient`` to keystroke an inline preview into the
    recipient's tmux pane. Notification failure does not roll back the
    insert — the message remains available via :func:`poll_tasks`.

    Args:
        session_id: Session UUID; sender and recipient must both belong to it.
        agent_id: Sender's agent UUID.
        to: Recipient's agent UUID.
        text: Message body. Truncation is render-side; the persisted row
            holds the full string.

    Returns:
        Dict with ``task`` (the persisted task dict) and ``notification_sent``
        (boolean indicating whether the inline-preview keystroke landed).

    Raises:
        ValueError: If ``to`` is not a valid UUID, the sender is not active
            in ``session_id``, or the recipient is missing or lives in a
            different session.
    """
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
            recipient_id=to,
            sender_id=agent_id,
            task_dict=task_dict,
        )

    return {"task": task_dict, "notification_sent": notification_sent}


def broadcast_message(session_id: str, agent_id: str, text: str) -> list[dict]:
    """Fan out one delivery task per active non-admin peer plus a sender summary.

    Administrators are excluded at the SQL layer via ``json_extract`` so the
    card blob stays in the database; they are write-only identities. Every
    delivery row shares the same ``origin_task_id`` (the summary's task id)
    so receivers can thread back to the original broadcast.

    Args:
        session_id: Session UUID to scope the broadcast to.
        agent_id: Broadcaster's agent UUID.
        text: Message body delivered to every recipient.

    Returns:
        Single-element list containing a dict with ``task`` (the summary row
        owned by the broadcaster) and ``notifications_sent_count`` — the
        number of inline-preview keystrokes that landed successfully.

    Raises:
        ValueError: If the sender is not active in ``session_id``.
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

        deliveries: list[tuple[str, dict]] = []
        for recipient_id in recipient_ids:
            delivery_dict = _unicast_task_dict(
                recipient_id=recipient_id,
                sender_id=agent_id,
                text=text,
                now=_now_iso(),
                origin_task_id=summary_task_id,
            )
            _save_task(session, delivery_dict)
            deliveries.append((recipient_id, delivery_dict))

        now = _now_iso()
        summary_dict = {
            "task_id": summary_task_id,
            "context_id": agent_id,
            "from_agent_id": agent_id,
            "to_agent_id": "",
            "type": "broadcast_summary",
            "created_at": now,
            "status_state": "completed",
            "status_timestamp": now,
            "origin_task_id": summary_task_id,
            "text": f"Broadcast sent to {len(recipient_ids)} recipients",
        }
        _save_task(session, summary_dict)

        notifications_sent_count = sum(
            _try_notify_recipient(
                session,
                recipient_id=recipient_id,
                sender_id=agent_id,
                task_dict=delivery_dict,
            )
            for recipient_id, delivery_dict in deliveries
        )

    return [
        {"task": summary_dict, "notifications_sent_count": notifications_sent_count}
    ]


def poll_tasks(
    agent_id: str,
    since: str | None = None,
    page_size: int | None = None,
    status: str | None = None,
) -> list[dict]:
    """Return tasks addressed to ``agent_id`` in DESC timestamp order.

    ``broadcast_summary`` rows are filtered out — those belong to the
    broadcaster's own context and are not deliveries.

    Args:
        agent_id: Recipient agent UUID; matches ``Task.context_id``.
        since: Optional ISO-8601 timestamp; only tasks strictly newer than
            this value are returned (lexicographic comparison on the
            microsecond-precision ``+00:00`` form).
        page_size: Optional row cap applied after ordering.
        status: Optional ``status_state`` filter (e.g. ``"input_required"``).

    Returns:
        List of flat task dicts (one per row) carrying every column from the
        ``tasks`` table.
    """
    return _list_tasks_where(
        Task.context_id == agent_id,
        since=since,
        page_size=page_size,
        status=status,
    )


def _transition_task_state(
    agent_id: str,
    task_id: str,
    *,
    expected_agent_field: str,
    new_state: str,
    action_verb: str,
    permission_error_msg: str,
) -> dict:
    sm = get_sync_sessionmaker()
    with sm() as session, session.begin():
        task_dict = _read_task(session, task_id)
        if task_dict is None:
            raise ValueError(f"Task {task_id} not found")

        if task_dict[expected_agent_field] != agent_id:
            raise PermissionError(permission_error_msg)

        if task_dict["status_state"] != "input_required":
            raise ValueError(
                f"Cannot {action_verb} task in state {task_dict['status_state']}"
            )

        task_dict["status_state"] = new_state
        task_dict["status_timestamp"] = _now_iso()

        _save_task(session, task_dict)

    return {"task": task_dict}


def ack_task(agent_id: str, task_id: str) -> dict:
    """Transition a task from ``input_required`` to ``completed`` for the recipient.

    Args:
        agent_id: Recipient agent UUID; must match ``Task.context_id``.
        task_id: Task UUID to ack.

    Returns:
        Dict with ``task`` — the updated task dict.

    Raises:
        ValueError: If the task does not exist or is not in
            ``input_required`` state.
        PermissionError: If ``agent_id`` is not the recipient.
    """
    return _transition_task_state(
        agent_id,
        task_id,
        expected_agent_field="context_id",
        new_state="completed",
        action_verb="ACK",
        permission_error_msg="Only the recipient can ACK a task",
    )


def cancel_task(agent_id: str, task_id: str) -> dict:
    """Transition a task from ``input_required`` to ``canceled`` for the sender.

    Args:
        agent_id: Sender agent UUID; must match ``Task.from_agent_id``.
        task_id: Task UUID to cancel.

    Returns:
        Dict with ``task`` — the updated task dict.

    Raises:
        ValueError: If the task does not exist or is not in
            ``input_required`` state.
        PermissionError: If ``agent_id`` is not the sender.
    """
    return _transition_task_state(
        agent_id,
        task_id,
        expected_agent_field="from_agent_id",
        new_state="canceled",
        action_verb="cancel",
        permission_error_msg="Only the sender can cancel a task",
    )


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


def _list_tasks_where(
    *filters,
    since: str | None = None,
    page_size: int | None = None,
    status: str | None = None,
) -> list[dict]:
    stmt = (
        select(*(getattr(Task, col) for col in _TASK_COLUMNS))
        .where(*filters, _NOT_BROADCAST_SUMMARY)
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
    return [_row_to_task_dict(row) for row in rows]


def list_inbox(agent_id: str) -> list[dict]:
    """Return raw task rows addressed to ``agent_id`` (no broadcast_summary)."""
    return _list_tasks_where(Task.context_id == agent_id)


def list_sent(agent_id: str) -> list[dict]:
    """Return raw task rows sent by ``agent_id`` (no broadcast_summary)."""
    return _list_tasks_where(Task.from_agent_id == agent_id)


def list_timeline(session_id: str, limit: int = 200) -> list[dict]:
    """Return the session's recent tasks in DESC ``status_timestamp`` order.

    ``broadcast_summary`` rows are filtered out so the timeline shows only
    delivery rows. Membership is tested via ``from_agent_id`` joined to
    ``agents.session_id``.

    Args:
        session_id: Session UUID to scope the query to.
        limit: Maximum number of rows to return (default 200).

    Returns:
        List of flat task dicts in DESC ``status_timestamp`` order.
    """
    stmt = (
        select(*(getattr(Task, col) for col in _TASK_COLUMNS))
        .join(Agent, Task.from_agent_id == Agent.agent_id)
        .where(
            Agent.session_id == session_id,
            _NOT_BROADCAST_SUMMARY,
        )
        .order_by(Task.status_timestamp.desc())
        .limit(limit)
    )
    sm = get_sync_sessionmaker()
    with sm() as session:
        rows = session.execute(stmt).all()
    return [_row_to_task_dict(row) for row in rows]


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


def get_task(session_id: str, task_id: str) -> dict:
    """Return the task iff at least one of its endpoints lives in the session.

    Args:
        session_id: Session UUID used to gate visibility.
        task_id: Task UUID to fetch.

    Returns:
        Dict with ``task`` — the flat typed-column task dict.

    Raises:
        ValueError: If the task does not exist or neither endpoint belongs
            to ``session_id``.
    """
    sm = get_sync_sessionmaker()
    with sm() as session:
        task_dict = _read_task(session, task_id)
        if task_dict is None:
            raise ValueError(f"Task {task_id} not found")

        endpoint_ids = [task_dict["from_agent_id"]]
        to_id = task_dict["to_agent_id"]
        if to_id:
            endpoint_ids.append(to_id)
        in_session = session.execute(
            select(
                exists().where(
                    Agent.agent_id.in_(endpoint_ids),
                    Agent.session_id == session_id,
                )
            )
        ).scalar_one()
        if not in_session:
            raise ValueError(f"Task {task_id} not found")

    return {"task": task_dict}


def _resolve_id_prefix(session, *, id_column, base_where, ref: str, entity: str) -> str:
    """Resolve ``ref`` (a full UUID or unique prefix) to a full id.

    Exact-match short-circuits before any prefix scan, so a full UUID returns
    immediately and is never reported ambiguous (an 8-char prefix cannot equal
    a 36-char id, so it falls through to the scan). The prefix scan uses
    ``startswith(..., autoescape=True)`` so ``%`` / ``_`` in ``ref`` match
    literally rather than as LIKE wildcards, and is bounded to two rows.

    Raises:
        ValueError: zero matches (no-match) or more than one match (ambiguous),
            each with a distinct message.
    """
    exact = session.execute(
        select(id_column).where(id_column == ref, *base_where)
    ).first()
    if exact is not None:
        return ref

    matches = session.execute(
        select(id_column)
        .where(id_column.startswith(ref, autoescape=True), *base_where)
        .limit(2)
    ).all()
    if not matches:
        raise ValueError(f"no {entity} matches id '{ref}' in this session.")
    if len(matches) > 1:
        raise ValueError(
            f"id prefix '{ref}' is ambiguous; supply more characters or the full UUID."
        )
    return matches[0][0]


def resolve_agent_ref(session_id: str, ref: str) -> str:
    """Full agent UUID or unique prefix -> full agent_id.

    Scoped to ACTIVE agents in ``session_id`` (mirrors ``get_agent``).

    Raises:
        ValueError: ambiguous prefix, or no active agent in the session
            matches ``ref``.
    """
    sm = get_sync_sessionmaker()
    with sm() as session:
        return _resolve_id_prefix(
            session,
            id_column=Agent.agent_id,
            base_where=(Agent.session_id == session_id, Agent.status == "active"),
            ref=ref,
            entity="agent",
        )


def resolve_task_ref(session_id: str, ref: str) -> str:
    """Full task UUID or unique prefix -> full task_id.

    Scoped to tasks with at least one endpoint agent in ``session_id``
    (mirrors ``get_task`` visibility).

    Raises:
        ValueError: ambiguous prefix, or no session-visible task matches
            ``ref``.
    """
    sm = get_sync_sessionmaker()
    with sm() as session:
        return _resolve_id_prefix(
            session,
            id_column=Task.task_id,
            base_where=(
                exists().where(
                    Agent.session_id == session_id,
                    or_(
                        Agent.agent_id == Task.from_agent_id,
                        Agent.agent_id == Task.to_agent_id,
                    ),
                ),
            ),
            ref=ref,
            entity="task",
        )
