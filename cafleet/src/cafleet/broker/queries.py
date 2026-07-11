"""Read-only task queries (inbox/sent/timeline/get_task)."""

from sqlalchemy import exists, select

from cafleet.broker import _shared
from cafleet.db.models import Member, Task


def list_inbox(member_id: int) -> list[dict]:
    """Return raw task rows addressed to ``member_id`` (no broadcast_summary)."""
    return _shared.list_tasks_where(Task.context_id == member_id)


def list_sent(member_id: int) -> list[dict]:
    """Return raw task rows sent by ``member_id`` (no broadcast_summary)."""
    return _shared.list_tasks_where(Task.from_member_id == member_id)


def list_timeline(fleet_id: int, limit: int = 200) -> list[dict]:
    """Return the fleet's recent tasks in DESC ``status_timestamp`` order.

    ``broadcast_summary`` rows are filtered out so the timeline shows only
    delivery rows. Membership is tested via ``from_member_id`` joined to
    ``members.fleet_id``.

    Args:
        fleet_id: Fleet id to scope the query to.
        limit: Maximum number of rows to return (default 200).

    Returns:
        List of flat task dicts in DESC ``status_timestamp`` order.
    """
    stmt = (
        select(*(getattr(Task, col) for col in _shared.TASK_COLUMNS))
        .join(Member, Task.from_member_id == Member.member_id)
        .where(
            Member.fleet_id == fleet_id,
            _shared.NOT_BROADCAST_SUMMARY,
        )
        .order_by(Task.status_timestamp.desc())
        .limit(limit)
    )
    with _shared.read_session() as session:
        rows = session.execute(stmt).all()
    return [_shared.row_to_task_dict(row) for row in rows]


def get_task(fleet_id: int, task_id: int) -> dict:
    """Return the task iff at least one of its endpoints lives in the fleet.

    Args:
        fleet_id: Fleet id used to gate visibility.
        task_id: Task id to fetch.

    Returns:
        Dict with ``task`` — the flat typed-column task dict.

    Raises:
        ValueError: If the task does not exist or neither endpoint belongs
            to ``fleet_id``.
    """
    with _shared.read_session() as session:
        task_dict = _shared.read_task(session, task_id)
        if task_dict is None:
            raise ValueError(f"Task {task_id} not found")

        endpoint_ids = [task_dict["from_member_id"]]
        to_id = task_dict["to_member_id"]
        if to_id is not None:
            endpoint_ids.append(to_id)
        in_fleet = session.execute(
            select(
                exists().where(
                    Member.member_id.in_(endpoint_ids),
                    Member.fleet_id == fleet_id,
                )
            )
        ).scalar_one()
        if not in_fleet:
            raise ValueError(f"Task {task_id} not found")

    return {"task": task_dict}
