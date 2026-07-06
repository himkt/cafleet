"""Cross-submodule helpers and session context managers for the broker package."""

import json
from contextlib import contextmanager
from datetime import UTC, datetime

from sqlalchemy import exists, func, select

from cafleet.db.engine import get_sync_sessionmaker
from cafleet.db.models import Agent, Task

ADMINISTRATOR_KIND = "builtin-administrator"
MONITORING_MEMBER_KIND = "monitoring-member"

TASK_COLUMNS = tuple(Task.__table__.columns.keys())

NOT_BROADCAST_SUMMARY = Task.type != "broadcast_summary"

# Shared SQL expression extracting an agent card's ``$.cafleet.kind`` (NULL → "")
# so a comparison against a non-empty kind constant selects identical rows.
CARD_KIND_SQL = func.coalesce(
    func.json_extract(Agent.agent_card_json, "$.cafleet.kind"), ""
)


@contextmanager
def read_session():
    sm = get_sync_sessionmaker()
    with sm() as session:
        yield session


@contextmanager
def write_session():
    sm = get_sync_sessionmaker()
    with sm() as session, session.begin():
        yield session


def _card_kind(agent_card_json: str | None) -> str | None:
    """Extract ``$.cafleet.kind`` from a card, or None on any malformation.

    Guards against invalid JSON, a non-object top level, and a null / non-object
    ``cafleet`` value — every malformed shape resolves to a non-match (None)
    rather than raising ``AttributeError``.
    """
    if not agent_card_json:
        return None
    try:
        card = json.loads(agent_card_json)
    except ValueError:
        return None
    if not isinstance(card, dict):
        return None
    cafleet = card.get("cafleet")
    if not isinstance(cafleet, dict):
        return None
    return cafleet.get("kind")


def is_administrator(agent_card_json: str | None) -> bool:
    return _card_kind(agent_card_json) == ADMINISTRATOR_KIND


def derive_agent_kind(is_root_director: bool, card_kind: str | None) -> str:
    """Collapse an agent to its 4-value ``kind``.

    ``card_kind`` values that match neither builtin constant (including the
    SQL-coalesced ``""``) fall through to ``member``.
    """
    if is_root_director:
        return "director"
    if card_kind == ADMINISTRATOR_KIND:
        return "administrator"
    if card_kind == MONITORING_MEMBER_KIND:
        return "monitor"
    return "member"


def now_iso() -> str:
    return datetime.now(UTC).isoformat()


def placement_dict(row) -> dict:
    return {
        "director_agent_id": row.director_agent_id,
        "backend": row.backend,
        "mux_session": row.mux_session,
        "mux_window_id": row.mux_window_id,
        "mux_pane_id": row.mux_pane_id,
        "coding_agent": row.coding_agent,
        "created_at": row.created_at,
    }


def agent_is_active_in_fleet(session, agent_id: int, fleet_id: int) -> bool:
    return session.execute(
        select(
            exists().where(
                Agent.agent_id == agent_id,
                Agent.fleet_id == fleet_id,
                Agent.status == "active",
            )
        )
    ).scalar_one()


def row_to_task_dict(row) -> dict:
    return {col: getattr(row, col) for col in TASK_COLUMNS}


def read_task(session, task_id: int) -> dict | None:
    row = session.execute(
        select(*(getattr(Task, col) for col in TASK_COLUMNS)).where(
            Task.task_id == task_id
        )
    ).first()
    if row is None:
        return None
    return row_to_task_dict(row)


def list_tasks_where(
    *filters,
    status: str | None = None,
) -> list[dict]:
    stmt = (
        select(*(getattr(Task, col) for col in TASK_COLUMNS))
        .where(*filters, NOT_BROADCAST_SUMMARY)
        .order_by(Task.status_timestamp.desc())
    )
    if status is not None:
        stmt = stmt.where(Task.status_state == status)
    with read_session() as session:
        rows = session.execute(stmt).all()
    return [row_to_task_dict(row) for row in rows]
