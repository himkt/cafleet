"""FastAPI endpoints backing the admin WebUI."""

from typing import Any

from fastapi import APIRouter, Depends, HTTPException, Request
from pydantic import BaseModel

from cafleet import broker

webui_router = APIRouter(prefix="/ui/api")


def get_webui_session(request: Request) -> str:
    """Return ``X-Session-Id``; 400 if missing, 404 if the row is gone."""
    session_id = request.headers.get("x-session-id")
    if not session_id:
        raise HTTPException(status_code=400, detail="X-Session-Id header required")

    result = broker.get_session(session_id)
    if result is None:
        raise HTTPException(status_code=404, detail="Session not found")

    return session_id


def _format_messages(rows: list[dict[str, Any]]) -> list[dict[str, Any]]:
    if not rows:
        return []
    agent_ids = {row["from_agent_id"] for row in rows} | {
        row["to_agent_id"] for row in rows
    }
    agent_names = broker.get_agent_names(list(agent_ids))
    return [
        {
            "task_id": row["task_id"],
            "from_agent_id": row["from_agent_id"],
            "from_agent_name": agent_names[row["from_agent_id"]],
            "to_agent_id": row["to_agent_id"],
            "to_agent_name": agent_names[row["to_agent_id"]],
            "type": row["type"],
            "status": row["status_state"],
            "created_at": row["created_at"],
            "status_timestamp": row["status_timestamp"],
            "origin_task_id": row["origin_task_id"],
            "body": row["text"],
        }
        for row in rows
    ]


class SendMessageRequest(BaseModel):
    from_agent_id: str
    to_agent_id: str
    text: str


@webui_router.get("/sessions")
def list_sessions():
    return broker.list_sessions()


@webui_router.get("/agents")
def list_agents(session_id: str = Depends(get_webui_session)):
    agents = broker.list_session_agents(session_id)
    return {"agents": agents}


@webui_router.get("/agents/{agent_id}/inbox")
def get_inbox(
    agent_id: str,
    session_id: str = Depends(get_webui_session),
):
    if not broker.verify_agent_session(agent_id, session_id):
        raise HTTPException(status_code=404, detail="Agent not found")

    rows = broker.list_inbox(agent_id)
    return {"messages": _format_messages(rows)}


@webui_router.get("/agents/{agent_id}/sent")
def get_sent(
    agent_id: str,
    session_id: str = Depends(get_webui_session),
):
    if not broker.verify_agent_session(agent_id, session_id):
        raise HTTPException(status_code=404, detail="Agent not found")

    rows = broker.list_sent(agent_id)
    return {"messages": _format_messages(rows)}


@webui_router.get("/timeline")
def get_timeline(
    session_id: str = Depends(get_webui_session),
):
    rows = broker.list_timeline(session_id)
    return {"messages": _format_messages(rows)}


@webui_router.post("/messages/send")
def send_message(
    body: SendMessageRequest,
    session_id: str = Depends(get_webui_session),
):
    if broker.get_agent(body.from_agent_id, session_id) is None:
        raise HTTPException(status_code=400, detail="from_agent not in session")

    if body.to_agent_id == "*":
        result = broker.broadcast_message(session_id, body.from_agent_id, body.text)
        summary = result[0]["task"]
        return {"task_id": summary["task_id"], "status": summary["status_state"]}

    if broker.get_agent(body.to_agent_id, session_id) is None:
        raise HTTPException(status_code=404, detail="Agent not found")

    result = broker.send_message(
        session_id, body.from_agent_id, body.to_agent_id, body.text
    )
    task = result["task"]
    return {"task_id": task["task_id"], "status": task["status_state"]}
