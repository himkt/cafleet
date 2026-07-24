"""FastAPI endpoints backing the admin WebUI (``/api/*``)."""

from datetime import UTC, datetime
from typing import Any, Literal

import click
from fastapi import APIRouter, Depends, HTTPException, Request
from pydantic import BaseModel, Field

from cafleet import broker

webui_router = APIRouter(prefix="/api")


def _monitor_config_response(cfg: dict) -> dict:
    """Project a broker config dict to the WebUI's ``monitor`` shape (no ``member_id``)."""
    return {
        "interval_seconds": cfg["interval_seconds"],
        "last_ping_at": cfg["last_ping_at"],
        "enabled": cfg["enabled"],
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
    member_ids = {row["from_member_id"] for row in rows} | {
        row["to_member_id"] for row in rows
    }
    member_names = broker.get_member_names(list(member_ids))
    return [
        {
            "message_id": row["message_id"],
            "from_member_id": row["from_member_id"],
            "from_member_name": member_names[row["from_member_id"]],
            "to_member_id": row["to_member_id"],
            "to_member_name": member_names[row["to_member_id"]],
            "type": row["type"],
            "status": row["status_state"],
            "created_at": row["created_at"],
            "status_timestamp": row["status_timestamp"],
            "origin_message_id": row["origin_message_id"],
            "body": row["text"],
        }
        for row in rows
    ]


class SendMessageRequest(BaseModel):
    from_member_id: int
    to_member_id: int | Literal["*"]
    text: str


class MonitorPatch(BaseModel):
    """Body for ``PATCH /api/members/{id}/monitor`` — both fields optional."""

    interval_seconds: int | None = Field(default=None, ge=1)
    enabled: bool | None = None


@webui_router.get("/fleets")
def list_fleets():
    return broker.list_fleets()


@webui_router.get("/members")
def list_members(fleet_id: int = Depends(get_webui_fleet)):
    members = broker.list_roster(fleet_id, include_message_holders=True)
    configs = {c["member_id"]: c for c in broker.list_monitor_configs(fleet_id)}
    for member in members:
        cfg = configs.get(member["member_id"])
        member["monitor"] = _monitor_config_response(cfg) if cfg is not None else None
    return {"members": members}


@webui_router.get("/monitor")
def get_monitor(fleet_id: int = Depends(get_webui_fleet)):
    # One `now` for the runtime dict and the members rows (CLI/WebUI parity).
    now = datetime.now(UTC)
    payload = broker.monitor_runtime_payload(fleet_id, now)
    payload["members"] = broker.monitor_members_payload(fleet_id, now)
    return payload


@webui_router.get("/members/{member_id}/monitor")
def get_member_monitor(member_id: int, fleet_id: int = Depends(get_webui_fleet)):
    cfg = broker.get_monitor_config(fleet_id, member_id)
    if cfg is None:
        raise HTTPException(status_code=404, detail="Member not enrolled")
    return _monitor_config_response(cfg)


@webui_router.patch("/members/{member_id}/monitor")
def patch_member_monitor(
    member_id: int,
    body: MonitorPatch,
    fleet_id: int = Depends(get_webui_fleet),
):
    if broker.get_monitor_config(fleet_id, member_id) is None:
        raise HTTPException(status_code=404, detail="Member not enrolled")
    try:
        cfg = broker.update_monitor_config(
            fleet_id,
            member_id,
            interval_seconds=body.interval_seconds,
            enabled=body.enabled,
        )
    except click.ClickException as exc:
        # TOCTOU: the member may be deregistered (its config row deleted) between
        # the pre-check above and this update — surface as 404, not a 500.
        raise HTTPException(status_code=404, detail="Member not enrolled") from exc
    return _monitor_config_response(cfg)


@webui_router.get("/members/{member_id}/inbox")
def get_inbox(
    member_id: int,
    fleet_id: int = Depends(get_webui_fleet),
):
    if not broker.verify_member_fleet(member_id, fleet_id):
        raise HTTPException(status_code=404, detail="Member not found")

    rows = broker.list_inbox(member_id)
    return {"messages": _format_messages(rows)}


@webui_router.get("/members/{member_id}/sent")
def get_sent(
    member_id: int,
    fleet_id: int = Depends(get_webui_fleet),
):
    if not broker.verify_member_fleet(member_id, fleet_id):
        raise HTTPException(status_code=404, detail="Member not found")

    rows = broker.list_sent(member_id)
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
    if broker.get_member(body.from_member_id, fleet_id) is None:
        raise HTTPException(status_code=400, detail="from_member not in fleet")

    if body.to_member_id == "*":
        result = broker.broadcast_message(fleet_id, body.from_member_id, body.text)
        summary = result[0]["message"]
        return {"message_id": summary["message_id"], "status": summary["status_state"]}

    if broker.get_member(body.to_member_id, fleet_id) is None:
        raise HTTPException(status_code=404, detail="Member not found")

    result = broker.send_message(
        fleet_id, body.from_member_id, body.to_member_id, body.text
    )
    message = result["message"]
    return {"message_id": message["message_id"], "status": message["status_state"]}
