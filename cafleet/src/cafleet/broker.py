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

from cafleet.db.engine import get_sync_sessionmaker
from cafleet.db.models import Agent, AgentPlacement, Session, Task
from cafleet.tmux import DirectorContext


# ---------------------------------------------------------------------------
# Root Director bootstrap constants (design 0000026)
# ---------------------------------------------------------------------------


_DIRECTOR_NAME = "director"
_DIRECTOR_DESCRIPTION = "Root Director for this session"
# FIXME(claude): auto-detect from $CLAUDECODE / $CLAUDE_CODE_ENTRYPOINT / codex env vars.
_ROOT_DIRECTOR_CODING_AGENT = "unknown"


# ---------------------------------------------------------------------------
# Built-in Administrator agent — constants, helpers, and exception
# ---------------------------------------------------------------------------


ADMINISTRATOR_KIND = "builtin-administrator"


class AdministratorProtectedError(Exception):
    """Raised when an operation targets a built-in Administrator agent."""


def _administrator_agent_card(session_id: str) -> dict:
    """Canonical AgentCard dict for the built-in Administrator of a session."""
    short_id = session_id[:8]
    return {
        "name": "Administrator",
        "description": f"Built-in administrator agent for session {short_id}",
        "skills": [],
        "cafleet": {"kind": ADMINISTRATOR_KIND},
    }


def _is_administrator_card(agent_card_json: str | None) -> bool:
    """Return True iff the stored card JSON marks an Administrator agent."""
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


# ---------------------------------------------------------------------------
# Internal helpers
# ---------------------------------------------------------------------------


def _now_iso() -> str:
    return datetime.now(UTC).isoformat()


def _try_notify_recipient(
    session, *, session_id: str, recipient_id: str, sender_id: str
) -> bool:
    """Best-effort tmux push notification. Returns True on success.

    Looks up the recipient's placement. If a tmux_pane_id exists and the
    recipient is not the sender, sends a ``cafleet --session-id <sid> poll
    --agent-id <aid>`` trigger via ``tmux send-keys`` so the literal command
    text can be matched by the recipient's ``permissions.allow``.
    Failures are silent — the message queue is the source of truth.
    """
    if recipient_id == sender_id:
        return False
    row = session.execute(
        select(AgentPlacement.tmux_pane_id).where(
            AgentPlacement.agent_id == recipient_id
        )
    ).first()
    if row is None or row[0] is None:
        return False
    from cafleet.tmux import send_poll_trigger

    return send_poll_trigger(
        target_pane_id=row[0],
        session_id=session_id,
        agent_id=recipient_id,
    )


# ---------------------------------------------------------------------------
# Session operations
# ---------------------------------------------------------------------------


def create_session(
    label: str | None = None,
    director_context: DirectorContext | None = None,
) -> dict:
    """Atomically bootstrap a session + root Director + Administrator.

    Design doc 0000026. Runs 5 ordered operations inside a single
    ``with session.begin():`` block:

      1. INSERT sessions (deleted_at=NULL, director_agent_id=NULL).
      2. INSERT agents (the hardcoded root Director).
      3. INSERT agent_placements (director_agent_id=NULL, coding_agent="unknown").
      4. UPDATE sessions SET director_agent_id=<director agent_id>.
      5. INSERT agents (the built-in Administrator, per design 0000025).

    Any exception inside the block triggers SQLAlchemy rollback — no partial
    rows persist. ``director_context`` MUST be read from tmux BEFORE calling
    this function (the CLI layer owns that resolution); a missing context is
    a programmer error.

    Returns the nested shape::

        {
          "session_id", "label", "created_at", "administrator_agent_id",
          "director": {
            "agent_id", "name", "description", "registered_at",
            "placement": {
              "director_agent_id", "tmux_session", "tmux_window_id",
              "tmux_pane_id", "coding_agent", "created_at",
            },
          },
        }
    """
    if director_context is None:
        raise TypeError(
            "broker.create_session requires a director_context "
            "(resolve it via tmux.director_context() before calling)"
        )

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

    sm = get_sync_sessionmaker()
    with sm() as session:
        with session.begin():
            # 1. INSERT sessions
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

            # 2. INSERT root Director agent
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

            # 3. INSERT Director placement (director_agent_id=NULL, no parent)
            session.add(
                AgentPlacement(
                    agent_id=director_agent_id,
                    director_agent_id=None,
                    tmux_session=director_context.session,
                    tmux_window_id=director_context.window_id,
                    tmux_pane_id=director_context.pane_id,
                    coding_agent=_ROOT_DIRECTOR_CODING_AGENT,
                    created_at=created_at,
                )
            )
            session.flush()

            # 4. UPDATE sessions.director_agent_id
            session.execute(
                update(Session)
                .where(Session.session_id == session_id)
                .values(director_agent_id=director_agent_id)
            )

            # 5. INSERT built-in Administrator (per design 0000025)
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
            "placement": {
                "director_agent_id": None,
                "tmux_session": director_context.session,
                "tmux_window_id": director_context.window_id,
                "tmux_pane_id": director_context.pane_id,
                "coding_agent": _ROOT_DIRECTOR_CODING_AGENT,
                "created_at": created_at,
            },
        },
    }


def list_sessions() -> list[dict]:
    """SELECT non-soft-deleted sessions with active agent count."""
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
    """SELECT single session. Returns dict or None (no ``deleted_at`` filter).

    The returned dict exposes ``deleted_at`` so callers can distinguish between
    a missing session and a soft-deleted one (``register_agent`` relies on this).
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
    """Soft-delete cascade (design 0000026). Returns ``{"deregistered_count": N}``.

    All three operations run in a single transaction:

      1. UPDATE sessions SET deleted_at = now WHERE session_id=X AND deleted_at IS NULL.
      2. UPDATE agents SET status='deregistered', deregistered_at=now
         WHERE session_id=X AND status='active' — this is the N that we return.
      3. DELETE FROM agent_placements WHERE agent_id IN (<agents in session>).

    Tasks are untouched (audit history preserved). Idempotent: the initial
    ``WHERE deleted_at IS NULL`` guard on step 1 short-circuits the cascade on
    re-run, so step 2 reports 0 rows and the call is a no-op.
    """
    now = _now_iso()
    sm = get_sync_sessionmaker()
    with sm() as session:
        with session.begin():
            # Check the session exists at all first so we can distinguish
            # "not found" from "already soft-deleted".
            exists_row = session.execute(
                select(Session.session_id).where(Session.session_id == session_id)
            ).first()
            if exists_row is None:
                # ClickException exits 1 (matches ``session show`` wording +
                # exit code per design 0000026); UsageError would exit 2 with
                # a Usage: banner which is wrong for a runtime "not found".
                raise click.ClickException(f"session '{session_id}' not found.")

            # 1. Soft-delete the session row (idempotent via deleted_at IS NULL).
            step1 = cast(
                CursorResult,
                session.execute(
                    update(Session)
                    .where(
                        Session.session_id == session_id,
                        Session.deleted_at.is_(None),
                    )
                    .values(deleted_at=now)
                ),
            )

            if step1.rowcount == 0:
                # Session exists but was already soft-deleted. Short-circuit the cascade.
                return {"deregistered_count": 0}

            # 2. Deregister every active agent in the session and count them.
            step2 = cast(
                CursorResult,
                session.execute(
                    update(Agent)
                    .where(
                        Agent.session_id == session_id,
                        Agent.status == "active",
                    )
                    .values(status="deregistered", deregistered_at=now)
                ),
            )
            deregistered_count = step2.rowcount

            # 3. Drop every placement whose agent belongs to this session.
            agents_in_session = select(Agent.agent_id).where(
                Agent.session_id == session_id
            )
            session.execute(
                delete(AgentPlacement).where(
                    AgentPlacement.agent_id.in_(agents_in_session)
                )
            )

    return {"deregistered_count": deregistered_count}


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
    Validates session_id exists and is not soft-deleted. When placement is
    provided, validates director exists and is active in the same session.
    """
    # Validate session exists and is not soft-deleted (design 0000026).
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
                if _is_administrator_card(director.agent_card_json):
                    raise AdministratorProtectedError(
                        "Administrator cannot be a director"
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
    Raises ``AdministratorProtectedError`` when the target is the built-in
    Administrator agent (design 0000025 §D).
    Raises ``click.UsageError`` when the target is the root Director of any
    session (design 0000026): use ``cafleet session delete`` instead.
    """
    sm = get_sync_sessionmaker()
    with sm() as session:
        with session.begin():
            # Root-Director guard (design 0000026). A session's root Director
            # is the agent referenced by ``sessions.director_agent_id`` — if
            # any session points at this agent_id, refuse.
            root_director_hit = session.execute(
                select(Session.session_id).where(Session.director_agent_id == agent_id)
            ).first()
            if root_director_hit is not None:
                raise click.UsageError(
                    "cannot deregister the root Director; "
                    "use 'cafleet session delete' instead"
                )

            card_json = session.execute(
                select(Agent.agent_card_json).where(Agent.agent_id == agent_id)
            ).scalar_one_or_none()
            if card_json is not None and _is_administrator_card(card_json):
                raise AdministratorProtectedError(
                    "Administrator cannot be deregistered"
                )
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
                    delete(AgentPlacement).where(AgentPlacement.agent_id == agent_id)
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
    row = session.execute(select(Task.task_json).where(Task.task_id == task_id)).first()
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
            # Validate sender exists and is active in session
            sender_agent = session.execute(
                select(Agent).where(
                    Agent.agent_id == agent_id,
                    Agent.session_id == session_id,
                    Agent.status == "active",
                )
            ).scalar_one_or_none()
            if sender_agent is None:
                raise ValueError(
                    f"Sender agent not found or not active in session: {agent_id}"
                )

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
                raise ValueError(f"Destination agent not in session: {to}")

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

            # tmux push notification (best-effort)
            notification_sent = _try_notify_recipient(
                session,
                session_id=session_id,
                recipient_id=to,
                sender_id=agent_id,
            )

    return {"task": task_dict, "notification_sent": notification_sent}


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
            # Validate sender exists and is active in session
            sender_agent = session.execute(
                select(Agent).where(
                    Agent.agent_id == agent_id,
                    Agent.session_id == session_id,
                    Agent.status == "active",
                )
            ).scalar_one_or_none()
            if sender_agent is None:
                raise ValueError(
                    f"Sender agent not found or not active in session: {agent_id}"
                )

            # List active agents in session, excluding sender and any
            # built-in Administrator agents (they are write-only identities,
            # per design 0000025 §E). Filter the Administrator out at the
            # SQL layer via ``json_extract`` so we don't have to ship every
            # ``agent_card_json`` blob into Python just to discard it. The
            # sender may itself be an Administrator — that case is handled
            # by the sender exclusion above, not by this filter.
            rows = session.execute(
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
            ).all()
            recipient_ids = [aid for (aid,) in rows]

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
            notifications_sent_count = 0
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

            # tmux push notifications (best-effort, still inside the session
            # transaction; the queue remains the source of truth if a notify
            # races the commit).
            for recipient_id in recipient_ids:
                if _try_notify_recipient(
                    session,
                    session_id=session_id,
                    recipient_id=recipient_id,
                    sender_id=agent_id,
                ):
                    notifications_sent_count += 1

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
    registered_at, kind.  ``kind`` is ``"builtin-administrator"`` when
    ``agent_card_json`` contains ``$.cafleet.kind`` with that value
    (extracted in SQL via ``json_extract``/``coalesce`` so the full blob
    never leaves the database) and ``"user"`` for every other agent.
    Deregistered agents appear only when they are referenced by at least
    one task (as sender or recipient).
    """
    has_tasks = exists().where(
        or_(
            Task.context_id == Agent.agent_id,
            Task.from_agent_id == Agent.agent_id,
        )
    )
    # Derive ``kind`` directly in SQL via ``json_extract`` so we do not
    # have to ship the entire ``agent_card_json`` blob into Python just to
    # compute a one-token discriminator. ``coalesce`` substitutes an empty
    # string when the row has no ``cafleet.kind`` path so the comparison
    # always evaluates cleanly.
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
            select(Agent.agent_id, Agent.name).where(Agent.agent_id.in_(agent_ids))
        ).all()
    return {row.agent_id: row.name for row in rows}


def get_task_created_ats(task_ids: list[str]) -> dict[str, str]:
    """Batch lookup: {task_id: created_at}."""
    if not task_ids:
        return {}
    sm = get_sync_sessionmaker()
    with sm() as session:
        rows = session.execute(
            select(Task.task_id, Task.created_at).where(Task.task_id.in_(task_ids))
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
