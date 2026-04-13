"""WebUI API endpoints for the CAFleet message viewer.

All endpoints call ``broker`` directly (sync). FastAPI runs sync handlers
in a thread pool automatically.
"""

import json

from fastapi import APIRouter, Depends, HTTPException, Request
from pydantic import BaseModel

from cafleet import broker

webui_router = APIRouter(prefix="/ui/api")


# ---------------------------------------------------------------------------
# Shared helpers
# ---------------------------------------------------------------------------


def get_webui_session(request: Request) -> str:
    """Extract and validate X-Session-Id header.

    Returns session_id. Raises 400 if missing, 404 if session not found.
    """
    session_id = request.headers.get("x-session-id")
    if not session_id:
        raise HTTPException(status_code=400, detail="X-Session-Id header required")

    result = broker.get_session(session_id)
    if result is None:
        raise HTTPException(status_code=404, detail="Session not found")

    return session_id


def _extract_body(task_dict: dict) -> str:
    """Extract text body from a camelCase task dict."""
    for artifact in task_dict.get("artifacts", []):
        for part in artifact.get("parts", []):
            if part.get("text"):
                return part["text"]
    return ""


def _format_raw_tasks(rows: list[dict]) -> list[dict]:
    """Format raw task row dicts (from list_inbox/list_sent) into WebUI messages."""
    if not rows:
        return []

    agent_ids: set[str] = set()
    for row in rows:
        if row["from_agent_id"]:
            agent_ids.add(row["from_agent_id"])
        if row["to_agent_id"]:
            agent_ids.add(row["to_agent_id"])
    agent_names = broker.get_agent_names(list(agent_ids))

    messages: list[dict] = []
    for row in rows:
        task_dict = json.loads(row["task_json"])
        from_id = row["from_agent_id"]
        to_id = row["to_agent_id"]
        messages.append(
            {
                "task_id": row["task_id"],
                "from_agent_id": from_id,
                "from_agent_name": agent_names.get(from_id, "") if from_id else "",
                "to_agent_id": to_id,
                "to_agent_name": agent_names.get(to_id, "") if to_id else "",
                "type": row["type"],
                "status": row["status_state"],
                "created_at": row["created_at"],
                "status_timestamp": row["status_timestamp"],
                "origin_task_id": row["origin_task_id"],
                "body": _extract_body(task_dict),
            }
        )
    return messages


def _format_timeline_entries(entries: list[dict]) -> list[dict]:
    """Format timeline entries (from list_timeline) into WebUI messages."""
    if not entries:
        return []

    agent_ids: set[str] = set()
    for entry in entries:
        task = entry["task"]
        metadata = task.get("metadata", {})
        from_id = metadata.get("fromAgentId", "")
        to_id = metadata.get("toAgentId", "")
        if from_id:
            agent_ids.add(from_id)
        if to_id:
            agent_ids.add(to_id)
    agent_names = broker.get_agent_names(list(agent_ids))

    messages: list[dict] = []
    for entry in entries:
        task = entry["task"]
        metadata = task.get("metadata", {})
        from_id = metadata.get("fromAgentId", "")
        to_id = metadata.get("toAgentId", "")
        messages.append(
            {
                "task_id": task["id"],
                "from_agent_id": from_id,
                "from_agent_name": agent_names.get(from_id, "") if from_id else "",
                "to_agent_id": to_id,
                "to_agent_name": agent_names.get(to_id, "") if to_id else "",
                "type": metadata.get("type", ""),
                "status": task.get("status", {}).get("state", ""),
                "created_at": entry["created_at"],
                "status_timestamp": task.get("status", {}).get("timestamp", ""),
                "origin_task_id": entry["origin_task_id"],
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
    messages = _format_raw_tasks(rows)
    return {"messages": messages}


@webui_router.get("/agents/{agent_id}/sent")
def get_sent(
    agent_id: str,
    session_id: str = Depends(get_webui_session),
):
    if not broker.verify_agent_session(agent_id, session_id):
        raise HTTPException(status_code=404, detail="Agent not found")

    rows = broker.list_sent(agent_id)
    messages = _format_raw_tasks(rows)
    return {"messages": messages}


@webui_router.get("/timeline")
def get_timeline(
    session_id: str = Depends(get_webui_session),
):
    entries = broker.list_timeline(session_id)
    messages = _format_timeline_entries(entries)
    return {"messages": messages}


@webui_router.post("/messages/send")
def send_message(
    body: SendMessageRequest,
    session_id: str = Depends(get_webui_session),
):
    from_agent = broker.get_agent(body.from_agent_id, session_id)
    if from_agent is None:
        raise HTTPException(status_code=400, detail="from_agent not in session")

    if body.to_agent_id == "*":
        result = broker.broadcast_message(session_id, body.from_agent_id, body.text)
        summary = result[0]["task"]
        return {"task_id": summary["id"], "status": summary["status"]["state"]}

    to_agent = broker.get_agent(body.to_agent_id, session_id)
    if to_agent is None:
        raise HTTPException(status_code=404, detail="Agent not found")

    result = broker.send_message(
        session_id, body.from_agent_id, body.to_agent_id, body.text
    )
    task = result["task"]
    return {"task_id": task["id"], "status": task["status"]["state"]}
