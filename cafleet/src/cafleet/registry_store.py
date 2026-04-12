"""SQL-backed RegistryStore.

Owns an ``async_sessionmaker[AsyncSession]`` and opens a fresh session per
call. Multi-statement operations wrap their bodies in
``async with session.begin():`` so they commit (or roll back) as a single
transaction.
"""

import json
import uuid
from datetime import UTC, datetime
from typing import TypedDict, cast

from sqlalchemy import and_, delete, func, select, update
from sqlalchemy.engine import CursorResult
from sqlalchemy.ext.asyncio import AsyncSession, async_sessionmaker

from cafleet.db.models import Agent, AgentPlacement, Session, Task
from cafleet.models import PlacementCreate


class CreateAgentResult(TypedDict):
    agent_id: str
    name: str
    registered_at: str


class AgentRecord(TypedDict, total=False):
    agent_id: str
    name: str
    description: str
    agent_card_json: str
    status: str
    registered_at: str
    deregistered_at: str


class AgentListItem(TypedDict):
    agent_id: str
    name: str
    description: str
    registered_at: str
    agent_card_json: str


def _now_iso() -> str:
    return datetime.now(UTC).isoformat()


class RegistryStore:
    def __init__(self, sessionmaker: async_sessionmaker[AsyncSession]) -> None:
        self._sessionmaker = sessionmaker

    async def create_agent(
        self,
        name: str,
        description: str,
        skills: list[dict] | None = None,
        *,
        session_id: str,
    ) -> CreateAgentResult:
        return await self.create_agent_with_placement(
            name=name,
            description=description,
            skills=skills,
            session_id=session_id,
            placement=None,
        )

    async def create_agent_with_placement(
        self,
        name: str,
        description: str,
        skills: list[dict] | None = None,
        *,
        session_id: str,
        placement: PlacementCreate | None = None,
    ) -> CreateAgentResult:
        agent_id = str(uuid.uuid4())
        registered_at = _now_iso()
        agent_card = {
            "name": name,
            "description": description,
            "skills": skills or [],
        }

        async with self._sessionmaker() as session:
            async with session.begin():
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
                            director_agent_id=placement.director_agent_id,
                            tmux_session=placement.tmux_session,
                            tmux_window_id=placement.tmux_window_id,
                            tmux_pane_id=placement.tmux_pane_id,
                            coding_agent=placement.coding_agent,
                            created_at=registered_at,
                        )
                    )

        return {
            "agent_id": agent_id,
            "name": name,
            "registered_at": registered_at,
        }

    async def get_agent(self, agent_id: str) -> AgentRecord | None:
        async with self._sessionmaker() as session:
            result = await session.execute(
                select(Agent).where(Agent.agent_id == agent_id)
            )
            agent = result.scalar_one_or_none()

        if agent is None:
            return None

        record: AgentRecord = {
            "agent_id": agent.agent_id,
            "name": agent.name,
            "description": agent.description,
            "agent_card_json": agent.agent_card_json,
            "status": agent.status,
            "registered_at": agent.registered_at,
        }
        if agent.deregistered_at is not None:
            record["deregistered_at"] = agent.deregistered_at
        return record

    async def get_agent_session_id(self, agent_id: str) -> str | None:
        """Return the session_id for an agent, or None if not found."""
        async with self._sessionmaker() as session:
            result = await session.execute(
                select(Agent.session_id).where(Agent.agent_id == agent_id)
            )
            row = result.first()
        return row[0] if row else None

    async def list_active_agents(
        self, session_id: str | None = None
    ) -> list[AgentListItem]:
        stmt = select(
            Agent.agent_id,
            Agent.name,
            Agent.description,
            Agent.registered_at,
            Agent.agent_card_json,
        ).where(Agent.status == "active")
        if session_id is not None:
            stmt = stmt.where(Agent.session_id == session_id)

        async with self._sessionmaker() as session:
            result = await session.execute(stmt)
            rows = result.all()

        return [
            {
                "agent_id": row.agent_id,
                "name": row.name,
                "description": row.description,
                "registered_at": row.registered_at,
                "agent_card_json": row.agent_card_json,
            }
            for row in rows
        ]

    async def deregister_agent(self, agent_id: str) -> bool:
        async with self._sessionmaker() as session:
            async with session.begin():
                # ``session.execute`` on a Core DML statement returns a
                # ``CursorResult`` at runtime but is typed as the more
                # general ``Result[Any]`` in the SQLAlchemy stubs, which
                # doesn't expose ``rowcount``. Cast to narrow.
                result = cast(
                    CursorResult,
                    await session.execute(
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
                    await session.execute(
                        delete(AgentPlacement).where(
                            AgentPlacement.agent_id == agent_id
                        )
                    )
            return result.rowcount > 0

    async def get_placement(self, agent_id: str) -> dict | None:
        async with self._sessionmaker() as session:
            result = await session.execute(
                select(AgentPlacement).where(AgentPlacement.agent_id == agent_id)
            )
            row = result.scalar_one_or_none()
        if row is None:
            return None
        return {
            "agent_id": row.agent_id,
            "director_agent_id": row.director_agent_id,
            "tmux_session": row.tmux_session,
            "tmux_window_id": row.tmux_window_id,
            "tmux_pane_id": row.tmux_pane_id,
            "coding_agent": row.coding_agent,
            "created_at": row.created_at,
        }

    async def update_placement_pane_id(
        self, agent_id: str, pane_id: str
    ) -> dict | None:
        async with self._sessionmaker() as session:
            async with session.begin():
                result = cast(
                    CursorResult,
                    await session.execute(
                        update(AgentPlacement)
                        .where(AgentPlacement.agent_id == agent_id)
                        .values(tmux_pane_id=pane_id)
                    ),
                )
                if result.rowcount == 0:
                    return None
        return await self.get_placement(agent_id)

    async def list_placements_for_director(
        self, *, session_id: str, director_agent_id: str
    ) -> list[dict]:
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
        async with self._sessionmaker() as session:
            result = await session.execute(stmt)
            rows = result.all()
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

    async def verify_agent_session(self, agent_id: str, session_id: str) -> bool:
        async with self._sessionmaker() as session:
            result = await session.execute(
                select(Agent.agent_id).where(
                    Agent.agent_id == agent_id,
                    Agent.session_id == session_id,
                )
            )
            return result.first() is not None

    async def list_sessions(self) -> list[dict]:
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

        async with self._sessionmaker() as session:
            result = await session.execute(stmt)
            rows = result.all()

        return [
            {
                "session_id": row.session_id,
                "label": row.label,
                "created_at": row.created_at,
                "agent_count": row.agent_count,
            }
            for row in rows
        ]

    async def get_session(self, session_id: str) -> dict | None:
        async with self._sessionmaker() as session:
            result = await session.execute(
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

    async def get_agent_name(self, agent_id: str) -> str:
        async with self._sessionmaker() as session:
            result = await session.execute(
                select(Agent.name).where(Agent.agent_id == agent_id)
            )
            row = result.first()
        return row[0] if row else ""

    async def get_agent_names(self, agent_ids: list[str]) -> dict[str, str]:
        """Batch lookup for agent names.

        Returns a dict mapping ``agent_id`` -> ``name`` for every id that
        exists in the ``agents`` table. Missing ids are simply absent from
        the returned dict (callers should use ``.get(id, "")`` if they
        want the same empty-string fallback as ``get_agent_name``).

        Used by ``webui_api._format_messages`` to avoid N+1 lookups when
        rendering a batch of inbox/sent messages.
        """
        if not agent_ids:
            return {}
        async with self._sessionmaker() as session:
            result = await session.execute(
                select(Agent.agent_id, Agent.name).where(Agent.agent_id.in_(agent_ids))
            )
            rows = result.all()
        return {row.agent_id: row.name for row in rows}

    async def list_deregistered_agents_with_tasks(self, session_id: str) -> list[dict]:
        has_task = (
            select(Task.task_id).where(Task.context_id == Agent.agent_id).exists()
        )
        stmt = select(
            Agent.agent_id,
            Agent.name,
            Agent.description,
            Agent.registered_at,
        ).where(
            Agent.session_id == session_id,
            Agent.status == "deregistered",
            has_task,
        )

        async with self._sessionmaker() as session:
            result = await session.execute(stmt)
            rows = result.all()

        return [
            {
                "agent_id": row.agent_id,
                "name": row.name,
                "description": row.description,
                "registered_at": row.registered_at,
            }
            for row in rows
        ]
