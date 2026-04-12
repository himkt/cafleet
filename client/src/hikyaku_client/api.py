import uuid
from typing import Any

import httpx


async def register_agent(
    broker_url: str,
    name: str,
    description: str,
    skills: list[dict] | None = None,
    *,
    api_key: str,
    placement: dict | None = None,
    director_agent_id: str | None = None,
) -> dict:
    body: dict[str, Any] = {"name": name, "description": description}
    if skills is not None:
        body["skills"] = skills
    if placement is not None:
        body["placement"] = placement
    headers = {"Authorization": f"Bearer {api_key}"}
    if director_agent_id is not None:
        headers["X-Agent-Id"] = director_agent_id
    async with httpx.AsyncClient() as client:
        resp = await client.post(
            f"{broker_url}/api/v1/agents", json=body, headers=headers
        )
        resp.raise_for_status()
        return resp.json()


async def send_message(
    broker_url: str,
    api_key: str,
    agent_id: str,
    to: str,
    text: str,
) -> dict:
    payload = {
        "jsonrpc": "2.0",
        "method": "SendMessage",
        "params": {
            "message": {
                "messageId": str(uuid.uuid4()),
                "role": "user",
                "parts": [{"kind": "text", "text": text}],
                "metadata": {"destination": to},
            },
        },
        "id": str(uuid.uuid4()),
    }
    async with httpx.AsyncClient() as client:
        resp = await client.post(
            f"{broker_url}/",
            json=payload,
            headers={
                "Authorization": f"Bearer {api_key}",
                "X-Agent-Id": agent_id,
            },
        )
        resp.raise_for_status()
        data = resp.json()
        if "error" in data:
            raise Exception(data["error"].get("message", str(data["error"])))
        return data["result"]


async def broadcast_message(
    broker_url: str,
    api_key: str,
    agent_id: str,
    text: str,
) -> list:
    result = await send_message(broker_url, api_key, agent_id, to="*", text=text)
    return [result]


async def poll_tasks(
    broker_url: str,
    api_key: str,
    agent_id: str,
    since: str | None = None,
    page_size: int | None = None,
    status: str | None = None,
) -> list:
    params: dict[str, Any] = {"contextId": agent_id}
    if since:
        params["since"] = since
    if page_size:
        params["pageSize"] = page_size
    if status:
        params["status"] = status
    payload = {
        "jsonrpc": "2.0",
        "method": "ListTasks",
        "params": params,
        "id": str(uuid.uuid4()),
    }
    async with httpx.AsyncClient() as client:
        resp = await client.post(
            f"{broker_url}/",
            json=payload,
            headers={
                "Authorization": f"Bearer {api_key}",
                "X-Agent-Id": agent_id,
            },
        )
        resp.raise_for_status()
        data = resp.json()
        if "error" in data:
            raise Exception(data["error"].get("message", str(data["error"])))
        result = data["result"]
        if isinstance(result, list):
            return result
        return result.get("tasks", [])


async def ack_task(
    broker_url: str,
    api_key: str,
    agent_id: str,
    task_id: str,
) -> dict:
    payload = {
        "jsonrpc": "2.0",
        "method": "SendMessage",
        "params": {
            "message": {
                "messageId": str(uuid.uuid4()),
                "role": "user",
                "taskId": task_id,
                "parts": [{"kind": "text", "text": "ack"}],
            },
        },
        "id": str(uuid.uuid4()),
    }
    async with httpx.AsyncClient() as client:
        resp = await client.post(
            f"{broker_url}/",
            json=payload,
            headers={
                "Authorization": f"Bearer {api_key}",
                "X-Agent-Id": agent_id,
            },
        )
        resp.raise_for_status()
        data = resp.json()
        if "error" in data:
            raise Exception(data["error"].get("message", str(data["error"])))
        return data["result"]


async def cancel_task(
    broker_url: str,
    api_key: str,
    agent_id: str,
    task_id: str,
) -> dict:
    payload = {
        "jsonrpc": "2.0",
        "method": "CancelTask",
        "params": {"id": task_id},
        "id": str(uuid.uuid4()),
    }
    async with httpx.AsyncClient() as client:
        resp = await client.post(
            f"{broker_url}/",
            json=payload,
            headers={
                "Authorization": f"Bearer {api_key}",
                "X-Agent-Id": agent_id,
            },
        )
        resp.raise_for_status()
        data = resp.json()
        if "error" in data:
            raise Exception(data["error"].get("message", str(data["error"])))
        return data["result"]


async def get_task(
    broker_url: str,
    api_key: str,
    agent_id: str,
    task_id: str,
) -> dict:
    payload = {
        "jsonrpc": "2.0",
        "method": "GetTask",
        "params": {"id": task_id},
        "id": str(uuid.uuid4()),
    }
    async with httpx.AsyncClient() as client:
        resp = await client.post(
            f"{broker_url}/",
            json=payload,
            headers={
                "Authorization": f"Bearer {api_key}",
                "X-Agent-Id": agent_id,
            },
        )
        resp.raise_for_status()
        data = resp.json()
        if "error" in data:
            raise Exception(data["error"].get("message", str(data["error"])))
        return data["result"]


async def list_agents(
    broker_url: str,
    api_key: str,
    caller_id: str | None = None,
    agent_id: str | None = None,
) -> list | dict:
    headers = {"Authorization": f"Bearer {api_key}"}
    if caller_id:
        headers["X-Agent-Id"] = caller_id
    async with httpx.AsyncClient() as client:
        if agent_id:
            resp = await client.get(
                f"{broker_url}/api/v1/agents/{agent_id}",
                headers=headers,
            )
            resp.raise_for_status()
            return resp.json()
        resp = await client.get(
            f"{broker_url}/api/v1/agents",
            headers=headers,
        )
        resp.raise_for_status()
        data = resp.json()
        return data.get("agents", data)


async def deregister_agent(
    broker_url: str,
    api_key: str,
    agent_id: str,
    *,
    caller_id: str | None = None,
) -> None:
    async with httpx.AsyncClient() as client:
        resp = await client.delete(
            f"{broker_url}/api/v1/agents/{agent_id}",
            headers={
                "Authorization": f"Bearer {api_key}",
                "X-Agent-Id": caller_id if caller_id is not None else agent_id,
            },
        )
        resp.raise_for_status()


async def patch_placement(
    broker_url: str,
    api_key: str,
    *,
    director_agent_id: str,
    member_agent_id: str,
    pane_id: str,
) -> dict:
    async with httpx.AsyncClient() as client:
        resp = await client.patch(
            f"{broker_url}/api/v1/agents/{member_agent_id}/placement",
            json={"tmux_pane_id": pane_id},
            headers={
                "Authorization": f"Bearer {api_key}",
                "X-Agent-Id": director_agent_id,
            },
        )
        resp.raise_for_status()
        return resp.json()


async def list_members(
    broker_url: str,
    api_key: str,
    director_agent_id: str,
) -> list[dict]:
    async with httpx.AsyncClient() as client:
        resp = await client.get(
            f"{broker_url}/api/v1/agents",
            params={"director_agent_id": director_agent_id},
            headers={
                "Authorization": f"Bearer {api_key}",
                "X-Agent-Id": director_agent_id,
            },
        )
        resp.raise_for_status()
        return resp.json().get("agents", [])
