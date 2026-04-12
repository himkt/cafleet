"""Session-based request dependencies.

Two FastAPI dependencies resolve callers to their session:

- ``get_session_from_agent_id`` — reads ``X-Agent-Id``, looks up
  ``agents.session_id``, returns ``(agent_id, session_id)``.
- ``get_session_from_header`` — reads ``X-Session-Id``, verifies
  existence in the ``sessions`` table, returns ``session_id``.
"""

from fastapi import HTTPException, Request
from sqlalchemy import select

from hikyaku_registry.db.models import Agent


async def get_session_from_agent_id(request: Request, store) -> tuple[str, str]:
    """Resolve ``X-Agent-Id`` header to ``(agent_id, session_id)``.

    Raises:
        HTTPException(400): header missing.
        HTTPException(404): agent not found.
    """
    agent_id = request.headers.get("x-agent-id")
    if not agent_id:
        raise HTTPException(status_code=400)

    async with store._sessionmaker() as session:
        result = await session.execute(
            select(Agent.session_id).where(Agent.agent_id == agent_id)
        )
        row = result.first()

    if row is None:
        raise HTTPException(status_code=404)

    return (agent_id, row[0])


async def get_session_from_header(request: Request, store) -> str:
    """Resolve ``X-Session-Id`` header to ``session_id``.

    Raises:
        HTTPException(400): header missing or empty.
        HTTPException(404): session not found.
    """
    session_id = request.headers.get("x-session-id")
    if not session_id:
        raise HTTPException(status_code=400)

    result = await store.get_session(session_id)
    if result is None:
        raise HTTPException(status_code=404)

    return session_id
