"""WebUI API endpoints for the Hikyaku message viewer."""

import asyncio
import uuid

from fastapi import APIRouter, Depends, HTTPException, Request
from pydantic import BaseModel
from a2a.server.agent_execution import RequestContext
from a2a.server.context import ServerCallContext
from a2a.server.events import EventQueue
from a2a.types import (
    Message,
    MessageSendParams,
    Part,
    Role,
    Task,
    TextPart,
)

from hikyaku.db.engine import get_sessionmaker
from hikyaku.executor import BrokerExecutor
from hikyaku.registry_store import RegistryStore
from hikyaku.task_store import TaskStore


webui_router = APIRouter(prefix="/ui/api")


# ---------------------------------------------------------------------------
# Dependency injection
# ---------------------------------------------------------------------------


def get_webui_store() -> RegistryStore:
    return RegistryStore(get_sessionmaker())


def get_webui_task_store() -> TaskStore:
    return TaskStore(get_sessionmaker())


def get_webui_executor() -> BrokerExecutor:
    return BrokerExecutor(
        registry_store=get_webui_store(),
        task_store=get_webui_task_store(),
    )


# ---------------------------------------------------------------------------
# Shared helpers
# ---------------------------------------------------------------------------


async def get_webui_session(
    request: Request,
    store: RegistryStore = Depends(get_webui_store),
) -> str:
    """Extract and validate X-Session-Id header.

    Returns session_id. Raises 400 if missing, 404 if session not found.
    """
    session_id = request.headers.get("x-session-id")
    if not session_id:
        raise HTTPException(status_code=400, detail="X-Session-Id header required")

    result = await store.get_session(session_id)
    if result is None:
        raise HTTPException(status_code=404, detail="Session not found")

    return session_id


async def _get_session_agents(
    session_id: str,
    store: RegistryStore,
) -> list[dict]:
    agents: list[dict] = []

    active_agents = await store.list_active_agents(session_id=session_id)
    for a in active_agents:
        agents.append(
            {
                "agent_id": a["agent_id"],
                "name": a["name"],
                "description": a["description"],
                "status": "active",
                "registered_at": a["registered_at"],
            }
        )

    deregistered = await store.list_deregistered_agents_with_tasks(session_id)
    for d in deregistered:
        agents.append(
            {
                "agent_id": d["agent_id"],
                "name": d["name"],
                "description": d["description"],
                "status": "deregistered",
                "registered_at": d["registered_at"],
            }
        )

    return agents


def _extract_body(task: Task) -> str:
    if not task.artifacts:
        return ""
    for artifact in task.artifacts:
        if artifact.parts:
            for part in artifact.parts:
                if isinstance(part.root, TextPart):
                    return part.root.text
    return ""


async def _format_messages(
    tasks: list[Task],
    store: RegistryStore,
    task_store: TaskStore,
    created_ats_override: dict[str, str] | None = None,
) -> list[dict]:
    """Format a batch of Tasks into WebUI message dicts."""
    if not tasks:
        return []

    task_ids = [task.id for task in tasks]
    created_ats = created_ats_override or await task_store.get_created_ats(task_ids)

    agent_ids: set[str] = set()
    for task in tasks:
        metadata = task.metadata or {}
        from_id = metadata.get("fromAgentId", "")
        to_id = metadata.get("toAgentId", "")
        if from_id:
            agent_ids.add(from_id)
        if to_id:
            agent_ids.add(to_id)
    agent_names = await store.get_agent_names(list(agent_ids))

    messages: list[dict] = []
    for task in tasks:
        metadata = task.metadata or {}
        from_id = metadata.get("fromAgentId", "")
        to_id = metadata.get("toAgentId", "")
        messages.append(
            {
                "task_id": task.id,
                "from_agent_id": from_id,
                "from_agent_name": agent_names.get(from_id, "") if from_id else "",
                "to_agent_id": to_id,
                "to_agent_name": agent_names.get(to_id, "") if to_id else "",
                "type": metadata.get("type", ""),
                "status": task.status.state.name,
                "created_at": created_ats.get(task.id, ""),
                "status_timestamp": task.status.timestamp or "",
                "origin_task_id": metadata.get("originTaskId"),
                "body": _extract_body(task),
            }
        )
    return messages


# ---------------------------------------------------------------------------
# Request model
# ---------------------------------------------------------------------------


class SendMessageRequest(BaseModel):
    from_agent_id: str
    to_agent_id: str
    text: str


# ---------------------------------------------------------------------------
# Endpoints
# ---------------------------------------------------------------------------


@webui_router.get("/sessions")
async def list_sessions(
    store: RegistryStore = Depends(get_webui_store),
):
    return await store.list_sessions()


@webui_router.get("/agents")
async def list_agents(
    session_id: str = Depends(get_webui_session),
    store: RegistryStore = Depends(get_webui_store),
):
    agents = await _get_session_agents(session_id, store)
    return {"agents": agents}


@webui_router.get("/agents/{agent_id}/inbox")
async def get_inbox(
    agent_id: str,
    session_id: str = Depends(get_webui_session),
    store: RegistryStore = Depends(get_webui_store),
    task_store: TaskStore = Depends(get_webui_task_store),
):

    if not await store.verify_agent_session(agent_id, session_id):
        raise HTTPException(status_code=404, detail="Agent not found")

    tasks = await task_store.list(agent_id)
    tasks = [
        t
        for t in tasks
        if not (t.metadata and t.metadata.get("type") == "broadcast_summary")
    ]

    messages = await _format_messages(tasks, store, task_store)
    return {"messages": messages}


@webui_router.get("/agents/{agent_id}/sent")
async def get_sent(
    agent_id: str,
    session_id: str = Depends(get_webui_session),
    store: RegistryStore = Depends(get_webui_store),
    task_store: TaskStore = Depends(get_webui_task_store),
):

    if not await store.verify_agent_session(agent_id, session_id):
        raise HTTPException(status_code=404, detail="Agent not found")

    tasks = await task_store.list_by_sender(agent_id)
    filtered_tasks = [
        t
        for t in tasks
        if not (t.metadata and t.metadata.get("type") == "broadcast_summary")
    ]

    messages = await _format_messages(filtered_tasks, store, task_store)
    return {"messages": messages}


@webui_router.get("/timeline")
async def get_timeline(
    session_id: str = Depends(get_webui_session),
    store: RegistryStore = Depends(get_webui_store),
    task_store: TaskStore = Depends(get_webui_task_store),
):
    results = await task_store.list_timeline(session_id, limit=200)
    tasks = [task for task, _origin, _created in results]
    precomputed = {task.id: created for task, _origin, created in results}
    messages = await _format_messages(
        tasks, store, task_store, created_ats_override=precomputed
    )
    for msg, (_task, origin, _created) in zip(messages, results):
        msg["origin_task_id"] = origin
    return {"messages": messages}


@webui_router.post("/messages/send")
async def send_message(
    body: SendMessageRequest,
    session_id: str = Depends(get_webui_session),
    store: RegistryStore = Depends(get_webui_store),
    task_store: TaskStore = Depends(get_webui_task_store),
    executor: BrokerExecutor = Depends(get_webui_executor),
):

    if not await store.verify_agent_session(body.from_agent_id, session_id):
        raise HTTPException(status_code=400, detail="from_agent not in session")

    from_agent = await store.get_agent(body.from_agent_id)
    if from_agent is None or from_agent.get("status") == "deregistered":
        raise HTTPException(status_code=400, detail="from_agent is deregistered")

    if body.to_agent_id == "*":
        destination = "*"
    else:
        to_agent = await store.get_agent(body.to_agent_id)
        if to_agent is None:
            raise HTTPException(status_code=404, detail="Agent not found")

        if to_agent.get("status") == "deregistered":
            raise HTTPException(status_code=400, detail="Agent is deregistered")

        to_in_session = await store.verify_agent_session(body.to_agent_id, session_id)
        if not to_in_session:
            raise HTTPException(status_code=404, detail="Agent not found")

        destination = body.to_agent_id

    msg = Message(
        message_id=str(uuid.uuid4()),
        role=Role("user"),
        parts=[Part(root=TextPart(text=body.text))],
        metadata={"destination": destination},
    )

    call_context = ServerCallContext(
        state={"agent_id": body.from_agent_id, "session_id": session_id}
    )
    send_params = MessageSendParams(message=msg)
    context = RequestContext(
        request=send_params,
        call_context=call_context,
    )

    event_queue = EventQueue()
    await executor.execute(context, event_queue)

    # Drain events to find the produced task
    last_task = None
    try:
        while True:
            event = event_queue.queue.get_nowait()
            if isinstance(event, Task):
                last_task = event
    except asyncio.QueueEmpty:
        pass

    if last_task is None:
        raise HTTPException(status_code=500, detail="No task produced")

    return {
        "task_id": last_task.id,
        "status": last_task.status.state.name,
    }
