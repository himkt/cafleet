"""Session-based request dependencies.

Two FastAPI dependencies resolve callers to their session:

- ``get_session_from_agent_id`` — reads ``X-Agent-Id``, looks up
  ``agents.session_id``, returns ``(agent_id, session_id)``.
- ``get_session_from_header`` — reads ``X-Session-Id``, verifies
  existence in the ``sessions`` table, returns ``session_id``.
"""

from fastapi import HTTPException, Request


async def get_session_from_agent_id(request: Request, store) -> tuple[str, str]:
    """Resolve ``X-Agent-Id`` header to ``(agent_id, session_id)``.

    Raises:
        HTTPException(400): header missing.
        HTTPException(404): agent not found.
    """
    agent_id = request.headers.get("x-agent-id")
    if not agent_id:
        raise HTTPException(status_code=400)

    session_id = await store.get_agent_session_id(agent_id)
    if session_id is None:
        raise HTTPException(status_code=404)

    return (agent_id, session_id)


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
