"""SQL-backed TaskStore.

Owns an ``async_sessionmaker[AsyncSession]`` and uses SQLite's
``INSERT ... ON CONFLICT DO UPDATE`` for the save-path UPSERT.

``created_at`` is preserved across re-saves: it is assigned only on
the initial INSERT and deliberately omitted from the ``set_=`` clause
so the original value survives subsequent updates. The A2A Task's
deeper fields (artifacts, metadata, history) live verbatim in the
``task_json`` TEXT blob.
"""

from __future__ import annotations

from datetime import UTC, datetime

from a2a.types import Task
from sqlalchemy import delete, select
from sqlalchemy.dialects.sqlite import insert as sqlite_insert
from sqlalchemy.ext.asyncio import AsyncSession, async_sessionmaker

from hikyaku_registry.db.models import Task as TaskModel

# Alias the ``list`` builtin because the ``list()`` method defined on
# ``TaskStore`` below shadows it inside the class body, breaking
# annotations like ``list[Task]``. ty (and other PEP 563 checkers) read
# these annotations before Python's name resolution kicks in, so the
# alias is required even with ``from __future__ import annotations``.
_TaskList = list


def _now_iso() -> str:
    return datetime.now(UTC).isoformat()


class TaskStore:
    def __init__(self, sessionmaker: async_sessionmaker[AsyncSession]) -> None:
        self._sessionmaker = sessionmaker

    async def save(self, task: Task) -> None:
        metadata = task.metadata or {}
        from_agent_id = metadata.get("fromAgentId", "")
        to_agent_id = metadata.get("toAgentId", "")
        msg_type = metadata.get("type", "")

        assert task.status.timestamp is not None
        status_timestamp = task.status.timestamp
        status_state = task.status.state.value
        task_json = task.model_dump_json()

        stmt = sqlite_insert(TaskModel).values(
            task_id=task.id,
            context_id=task.context_id,
            from_agent_id=from_agent_id,
            to_agent_id=to_agent_id,
            type=msg_type,
            created_at=_now_iso(),
            status_state=status_state,
            status_timestamp=status_timestamp,
            task_json=task_json,
        )
        stmt = stmt.on_conflict_do_update(
            index_elements=["task_id"],
            set_={
                "status_state": stmt.excluded.status_state,
                "status_timestamp": stmt.excluded.status_timestamp,
                "task_json": stmt.excluded.task_json,
                # created_at is deliberately omitted so the original
                # INSERT value survives subsequent saves.
            },
        )

        async with self._sessionmaker() as session:
            async with session.begin():
                await session.execute(stmt)

    async def get(self, task_id: str) -> Task | None:
        async with self._sessionmaker() as session:
            result = await session.execute(
                select(TaskModel.task_json).where(TaskModel.task_id == task_id)
            )
            row = result.first()
        if row is None:
            return None
        return Task.model_validate_json(row[0])

    async def delete(self, task_id: str) -> None:
        async with self._sessionmaker() as session:
            async with session.begin():
                await session.execute(
                    delete(TaskModel).where(TaskModel.task_id == task_id)
                )

    async def list(self, context_id: str) -> _TaskList[Task]:
        stmt = (
            select(TaskModel.task_json)
            .where(TaskModel.context_id == context_id)
            .order_by(TaskModel.status_timestamp.desc())
        )
        async with self._sessionmaker() as session:
            result = await session.execute(stmt)
            rows = result.all()
        return [Task.model_validate_json(row[0]) for row in rows]

    async def list_by_sender(self, agent_id: str) -> _TaskList[Task]:
        stmt = (
            select(TaskModel.task_json)
            .where(TaskModel.from_agent_id == agent_id)
            .order_by(TaskModel.status_timestamp.desc())
        )
        async with self._sessionmaker() as session:
            result = await session.execute(stmt)
            rows = result.all()
        return [Task.model_validate_json(row[0]) for row in rows]

    async def get_endpoints(self, task_id: str) -> tuple[str, str] | None:
        async with self._sessionmaker() as session:
            result = await session.execute(
                select(TaskModel.from_agent_id, TaskModel.to_agent_id).where(
                    TaskModel.task_id == task_id
                )
            )
            row = result.first()
        if row is None:
            return None
        return (row.from_agent_id, row.to_agent_id)

    async def get_created_at(self, task_id: str) -> str | None:
        async with self._sessionmaker() as session:
            result = await session.execute(
                select(TaskModel.created_at).where(TaskModel.task_id == task_id)
            )
            row = result.first()
        return row[0] if row else None
