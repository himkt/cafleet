"""FastAPI endpoints backing the admin WebUI (``/api/*``)."""

from datetime import UTC, datetime
from typing import Any, Literal

from fastapi import APIRouter, Depends, HTTPException, Request
from pydantic import BaseModel, Field

from cafleet import broker

webui_router = APIRouter(prefix="/api")


def _monitor_config_response(cfg: dict) -> dict:
    """Project a broker config dict to the WebUI's ``monitor`` shape (no ``agent_id``)."""
    return {
        "interval_seconds": cfg["interval_seconds"],
        "last_ping_at": cfg["last_ping_at"],
        "enabled": cfg["enabled"],
    }


def _monitor_runtime_payload(fleet_id: int) -> dict:
    """Build the ``GET /api/monitor`` liveness dict from the DB heartbeat.

    When the monitor is not live (no row, or a stale/cleared heartbeat), the
    process fields (``pid`` / ``started_at`` / ``last_tick_at`` /
    ``last_tick_age_seconds``) are ``null`` — a stale row never reports a
    lingering pid or start time.
    """
    now = datetime.now(UTC)
    row = broker.read_monitor_runtime(fleet_id)
    if row is None or not broker.monitor_is_live(fleet_id, now):
        return {
            "running": False,
            "pid": None,
            "tick_seconds": row["tick_seconds"] if row is not None else None,
            "last_tick_at": None,
            "last_tick_age_seconds": None,
            "started_at": None,
        }
    age = None
    if row["last_tick_at"] is not None:
        age = int((now - datetime.fromisoformat(row["last_tick_at"])).total_seconds())
    return {
        "running": True,
        "pid": row["pid"],
        "tick_seconds": row["tick_seconds"],
        "last_tick_at": row["last_tick_at"],
        "last_tick_age_seconds": age,
        "started_at": row["started_at"],
    }


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


class MonitorPatch(BaseModel):
    """Body for ``PATCH /api/agents/{id}/monitor`` — both fields optional."""

    interval_seconds: int | None = Field(default=None, ge=1)
    enabled: bool | None = None


@webui_router.get("/fleets")
def list_fleets():
    return broker.list_fleets()


@webui_router.get("/agents")
def list_agents(fleet_id: int = Depends(get_webui_fleet)):
    agents = broker.list_fleet_agents(fleet_id)
    configs = {c["agent_id"]: c for c in broker.list_monitor_configs(fleet_id)}
    for agent in agents:
        cfg = configs.get(agent["agent_id"])
        agent["monitor"] = _monitor_config_response(cfg) if cfg is not None else None
    return {"agents": agents}


@webui_router.get("/monitor")
def get_monitor(fleet_id: int = Depends(get_webui_fleet)):
    return _monitor_runtime_payload(fleet_id)


@webui_router.get("/agents/{agent_id}/monitor")
def get_agent_monitor(agent_id: int, fleet_id: int = Depends(get_webui_fleet)):
    cfg = broker.get_monitor_config(fleet_id, agent_id)
    if cfg is None:
        raise HTTPException(status_code=404, detail="Agent not enrolled")
    return _monitor_config_response(cfg)


@webui_router.patch("/agents/{agent_id}/monitor")
def patch_agent_monitor(
    agent_id: int,
    body: MonitorPatch,
    fleet_id: int = Depends(get_webui_fleet),
):
    if broker.get_monitor_config(fleet_id, agent_id) is None:
        raise HTTPException(status_code=404, detail="Agent not enrolled")
    cfg = broker.update_monitor_config(
        fleet_id,
        agent_id,
        interval_seconds=body.interval_seconds,
        enabled=body.enabled,
    )
    return _monitor_config_response(cfg)


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
