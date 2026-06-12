"""FastAPI endpoints backing the admin WebUI (``/api/*``)."""

from typing import Any, Literal

from fastapi import APIRouter, Depends, HTTPException, Request
from pydantic import BaseModel

from cafleet import broker

webui_router = APIRouter(prefix="/api")


def get_webui_fleet(request: Request) -> int:
    """Return the integer ``X-Fleet-Id``; 400 if missing/non-integer, 404 if gone."""
    raw = request.headers.get("x-fleet-id")
    if not raw:
        raise HTTPException(status_code=400, detail="X-Fleet-Id header required")

    try:
        fleet_id = int(raw)
    except ValueError:
        raise HTTPException(
            status_code=400, detail="X-Fleet-Id must be an integer"
        ) from None

    result = broker.get_fleet(fleet_id)
    if result is None:
        raise HTTPException(status_code=404, detail="Fleet not found")

    return fleet_id


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
    from_agent_id: int
    to_agent_id: int | Literal["*"]
    text: str


@webui_router.get("/fleets")
def list_fleets():
    return broker.list_fleets()


@webui_router.get("/agents")
def list_agents(fleet_id: int = Depends(get_webui_fleet)):
    agents = broker.list_fleet_agents(fleet_id)
    return {"agents": agents}


@webui_router.get("/agents/{agent_id}/inbox")
def get_inbox(
    agent_id: int,
    fleet_id: int = Depends(get_webui_fleet),
):
    if not broker.verify_agent_fleet(agent_id, fleet_id):
        raise HTTPException(status_code=404, detail="Agent not found")

    rows = broker.list_inbox(agent_id)
    return {"messages": _format_messages(rows)}


@webui_router.get("/agents/{agent_id}/sent")
def get_sent(
    agent_id: int,
    fleet_id: int = Depends(get_webui_fleet),
):
    if not broker.verify_agent_fleet(agent_id, fleet_id):
        raise HTTPException(status_code=404, detail="Agent not found")

    rows = broker.list_sent(agent_id)
    return {"messages": _format_messages(rows)}


@webui_router.get("/timeline")
def get_timeline(
    fleet_id: int = Depends(get_webui_fleet),
):
    rows = broker.list_timeline(fleet_id)
    return {"messages": _format_messages(rows)}


@webui_router.post("/messages/send")
def send_message(
    body: SendMessageRequest,
    fleet_id: int = Depends(get_webui_fleet),
):
    if broker.get_agent(body.from_agent_id, fleet_id) is None:
        raise HTTPException(status_code=400, detail="from_agent not in fleet")

    if body.to_agent_id == "*":
        result = broker.broadcast_message(fleet_id, body.from_agent_id, body.text)
        summary = result[0]["task"]
        return {"task_id": summary["task_id"], "status": summary["status_state"]}

    if broker.get_agent(body.to_agent_id, fleet_id) is None:
        raise HTTPException(status_code=404, detail="Agent not found")

    result = broker.send_message(
        fleet_id, body.from_agent_id, body.to_agent_id, body.text
    )
    task = result["task"]
    return {"task_id": task["task_id"], "status": task["status_state"]}
