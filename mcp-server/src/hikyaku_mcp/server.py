"""MCP server tool handlers — transparent proxy to broker.

poll reads from the local SSEClient buffer; all other tools forward
to the broker via RegistryForwarder.
"""

from __future__ import annotations

from datetime import UTC, datetime

from hikyaku_mcp.registry import RegistryForwarder
from hikyaku_mcp.sse_client import SSEClient


async def handle_poll(
    *,
    sse_client: SSEClient,
    page_size: int | None = None,
    since: str | None = None,
) -> list[dict]:
    """Drain buffered messages from SSEClient, with optional filtering."""
    if page_size is not None:
        items = sse_client.drain(max_items=page_size)
    else:
        items = sse_client.drain()

    if since is not None:
        since_dt = datetime.fromisoformat(since.replace("Z", "+00:00"))
        filtered = []
        for task in items:
            ts_str = task.get("status", {}).get("timestamp", "")
            if ts_str:
                task_dt = datetime.fromisoformat(ts_str.replace("Z", "+00:00"))
                if task_dt > since_dt:
                    filtered.append(task)
        items = filtered

    if page_size is not None:
        items = items[:page_size]

    return items


async def handle_send(
    *, forwarder: RegistryForwarder, to: str, text: str
) -> dict:
    """Forward send to RegistryForwarder."""
    return await forwarder.send(to=to, text=text)


async def handle_broadcast(*, forwarder: RegistryForwarder, text: str) -> dict:
    """Forward broadcast to RegistryForwarder."""
    return await forwarder.broadcast(text=text)


async def handle_ack(*, forwarder: RegistryForwarder, task_id: str) -> dict:
    """Forward ack to RegistryForwarder."""
    return await forwarder.ack(task_id=task_id)


async def handle_cancel(*, forwarder: RegistryForwarder, task_id: str) -> dict:
    """Forward cancel to RegistryForwarder."""
    return await forwarder.cancel(task_id=task_id)


async def handle_get_task(
    *, forwarder: RegistryForwarder, task_id: str
) -> dict:
    """Forward get_task to RegistryForwarder."""
    return await forwarder.get_task(task_id=task_id)


async def handle_agents(
    *, forwarder: RegistryForwarder, id: str | None = None
) -> dict:
    """Forward agents to RegistryForwarder."""
    return await forwarder.agents(id=id)


async def handle_register(
    *,
    forwarder: RegistryForwarder,
    name: str,
    description: str,
    skills: str | None = None,
    api_key: str | None = None,
) -> dict:
    """Forward register to RegistryForwarder."""
    return await forwarder.register(
        name=name, description=description, skills=skills, api_key=api_key
    )


async def handle_deregister(*, forwarder: RegistryForwarder) -> dict:
    """Forward deregister to RegistryForwarder."""
    return await forwarder.deregister()
