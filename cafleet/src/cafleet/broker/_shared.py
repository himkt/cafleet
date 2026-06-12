"""Cross-submodule helpers and session context managers for the broker package."""

import json
from contextlib import contextmanager
from datetime import UTC, datetime

from sqlalchemy import exists, select

from cafleet.db.engine import get_sync_sessionmaker
from cafleet.db.models import Agent, Task

ADMINISTRATOR_KIND = "builtin-administrator"

TASK_COLUMNS = tuple(Task.__table__.columns.keys())

NOT_BROADCAST_SUMMARY = Task.type != "broadcast_summary"


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


def is_administrator(agent_card_json: str | None) -> bool:
    if not agent_card_json:
        return False
    try:
        kind = json.loads(agent_card_json).get("cafleet", {}).get("kind")
    except ValueError:
        return False
    return kind == ADMINISTRATOR_KIND


def now_iso() -> str:
    return datetime.now(UTC).isoformat()


def placement_dict(row) -> dict:
    return {
        "director_agent_id": row.director_agent_id,
        "tmux_session": row.tmux_session,
        "tmux_window_id": row.tmux_window_id,
        "tmux_pane_id": row.tmux_pane_id,
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
