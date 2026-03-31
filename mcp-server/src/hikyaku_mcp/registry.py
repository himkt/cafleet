"""RegistryForwarder — proxy requests to the hikyaku-registry broker.

All methods forward HTTP requests to the broker's JSON-RPC or REST
endpoints using httpx.AsyncClient.
"""

from __future__ import annotations

import json
import uuid

import httpx


class RegistryForwarder:
    """Forwards MCP tool calls to the hikyaku-registry broker."""

    def __init__(self, broker_url: str, api_key: str, agent_id: str) -> None:
        self._broker_url = broker_url
        self._api_key = api_key
        self._agent_id = agent_id
        self._client = httpx.AsyncClient()

    def _auth_headers(self) -> dict[str, str]:
        return {
            "Authorization": f"Bearer {self._api_key}",
            "X-Agent-Id": self._agent_id,
        }

    def _jsonrpc_envelope(self, method: str, params: dict) -> dict:
        return {
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
            "id": str(uuid.uuid4()),
        }

    async def send(self, *, to: str, text: str) -> dict:
        """Send a unicast message to a specific agent."""
        message = {
            "parts": [{"type": "text", "text": text}],
            "metadata": {"destination": to},
        }
        body = self._jsonrpc_envelope("SendMessage", {"message": message})
        response = await self._client.post(
            f"{self._broker_url}/",
            json=body,
            headers=self._auth_headers(),
        )
        response.raise_for_status()
        return response.json()

    async def broadcast(self, *, text: str) -> dict:
        """Broadcast a message to all agents."""
        message = {
            "parts": [{"type": "text", "text": text}],
            "metadata": {"destination": "*"},
        }
        body = self._jsonrpc_envelope("SendMessage", {"message": message})
        response = await self._client.post(
            f"{self._broker_url}/",
            json=body,
            headers=self._auth_headers(),
        )
        response.raise_for_status()
        return response.json()

    async def ack(self, *, task_id: str) -> dict:
        """Acknowledge a task (multi-turn continuation)."""
        message = {
            "taskId": task_id,
            "parts": [{"type": "text", "text": "ack"}],
        }
        body = self._jsonrpc_envelope("SendMessage", {"message": message})
        response = await self._client.post(
            f"{self._broker_url}/",
            json=body,
            headers=self._auth_headers(),
        )
        response.raise_for_status()
        return response.json()

    async def cancel(self, *, task_id: str) -> dict:
        """Cancel a task."""
        body = self._jsonrpc_envelope("CancelTask", {"id": task_id})
        response = await self._client.post(
            f"{self._broker_url}/",
            json=body,
            headers=self._auth_headers(),
        )
        response.raise_for_status()
        return response.json()

    async def get_task(self, *, task_id: str) -> dict:
        """Get a task by ID."""
        body = self._jsonrpc_envelope("GetTask", {"id": task_id})
        response = await self._client.post(
            f"{self._broker_url}/",
            json=body,
            headers=self._auth_headers(),
        )
        response.raise_for_status()
        return response.json()

    async def register(
        self,
        *,
        name: str,
        description: str,
        skills: str | None = None,
    ) -> dict:
        """Register a new agent."""
        body: dict = {"name": name, "description": description}
        if skills is not None:
            body["skills"] = json.loads(skills)

        headers = {"Authorization": f"Bearer {self._api_key}"}

        response = await self._client.post(
            f"{self._broker_url}/api/v1/agents",
            json=body,
            headers=headers,
        )
        response.raise_for_status()
        return response.json()

    async def agents(self, *, id: str | None = None) -> dict:
        """List agents or get a specific agent."""
        if id is not None:
            url = f"{self._broker_url}/api/v1/agents/{id}"
        else:
            url = f"{self._broker_url}/api/v1/agents"

        response = await self._client.get(
            url,
            headers=self._auth_headers(),
        )
        response.raise_for_status()
        return response.json()

    async def deregister(self) -> dict:
        """Deregister this agent."""
        response = await self._client.delete(
            f"{self._broker_url}/api/v1/agents/{self._agent_id}",
            headers=self._auth_headers(),
        )
        response.raise_for_status()
        return response.json()
