"""FastAPI endpoints backing the admin WebUI."""

import json

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


def _extract_body(task_dict: dict) -> str:
    for artifact in task_dict["artifacts"]:
        for part in artifact["parts"]:
            if part.get("text"):
                return part["text"]
    return ""


def _build_message(
    *,
    task_id: str,
    from_id: str,
    to_id: str,
    type_: str,
    status: str,
    created_at: str,
    status_timestamp: str,
    origin_task_id: str | None,
    body: str,
    agent_names: dict[str, str],
) -> dict:
    return {
        "task_id": task_id,
        "from_agent_id": from_id,
        "from_agent_name": agent_names[from_id],
        "to_agent_id": to_id,
        "to_agent_name": agent_names[to_id],
        "type": type_,
        "status": status,
        "created_at": created_at,
        "status_timestamp": status_timestamp,
        "origin_task_id": origin_task_id,
        "body": body,
    }


def _raw_task_accessor(row: dict) -> dict:
    return {
        "task_id": row["task_id"],
        "from_id": row["from_agent_id"],
        "to_id": row["to_agent_id"],
        "type_": row["type"],
        "status": row["status_state"],
        "created_at": row["created_at"],
        "status_timestamp": row["status_timestamp"],
        "origin_task_id": row["origin_task_id"],
        "body": _extract_body(json.loads(row["task_json"])),
    }


def _timeline_entry_accessor(entry: dict) -> dict:
    task = entry["task"]
    meta = task["metadata"]
    return {
        "task_id": task["id"],
        "from_id": meta["fromAgentId"],
        "to_id": meta["toAgentId"],
        "type_": meta["type"],
        "status": task["status"]["state"],
        "created_at": entry["created_at"],
        "status_timestamp": task["status"]["timestamp"],
        "origin_task_id": entry["origin_task_id"],
        "body": _extract_body(task),
    }


def _format_messages(rows, accessor) -> list[dict]:
    if not rows:
        return []
    extracted = [accessor(row) for row in rows]
    agent_ids = {x["from_id"] for x in extracted} | {x["to_id"] for x in extracted}
    agent_names = broker.get_agent_names(list(agent_ids))
    return [_build_message(**x, agent_names=agent_names) for x in extracted]


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
    return {"messages": _format_messages(rows, _raw_task_accessor)}


@webui_router.get("/agents/{agent_id}/sent")
def get_sent(
    agent_id: str,
    session_id: str = Depends(get_webui_session),
):
    if not broker.verify_agent_session(agent_id, session_id):
        raise HTTPException(status_code=404, detail="Agent not found")

    rows = broker.list_sent(agent_id)
    return {"messages": _format_messages(rows, _raw_task_accessor)}


@webui_router.get("/timeline")
def get_timeline(
    session_id: str = Depends(get_webui_session),
):
    entries = broker.list_timeline(session_id)
    return {"messages": _format_messages(entries, _timeline_entry_accessor)}


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
        return {"task_id": summary["id"], "status": summary["status"]["state"]}

    if broker.get_agent(body.to_agent_id, session_id) is None:
        raise HTTPException(status_code=404, detail="Agent not found")

    result = broker.send_message(
        session_id, body.from_agent_id, body.to_agent_id, body.text
    )
    task = result["task"]
    return {"task_id": task["id"], "status": task["status"]["state"]}
