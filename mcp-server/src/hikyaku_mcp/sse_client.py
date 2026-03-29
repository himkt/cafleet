"""SSEClient — background SSE connection with buffered message queue.

Connects to the broker's /api/v1/subscribe endpoint, reads SSE events
in a background asyncio task, and buffers them in an asyncio.Queue.
The drain() method retrieves buffered messages synchronously.
"""

from __future__ import annotations

import asyncio
import json

import httpx

MAX_BUFFER_SIZE: int = 1000


class SSEClient:
    """Background SSE reader with in-memory message buffer."""

    def __init__(self, broker_url: str, api_key: str, agent_id: str) -> None:
        self.broker_url = broker_url
        self.api_key = api_key
        self.agent_id = agent_id
        self.queue: asyncio.Queue = asyncio.Queue(maxsize=MAX_BUFFER_SIZE)
        self._task: asyncio.Task | None = None
        self._client: httpx.AsyncClient | None = None

    async def connect(self) -> None:
        """Start background SSE connection to the broker."""
        self._client = httpx.AsyncClient()
        self._task = asyncio.create_task(self._read_loop())

    async def _read_loop(self) -> None:
        """Read SSE events from broker and buffer them in the queue."""
        url = f"{self.broker_url}/api/v1/subscribe"
        headers = {
            "Authorization": f"Bearer {self.api_key}",
            "X-Agent-Id": self.agent_id,
        }
        try:
            async with self._client.stream(
                "GET", url, headers=headers, timeout=None
            ) as response:
                buffer = ""
                async for chunk in response.aiter_text():
                    buffer += chunk
                    while "\n\n" in buffer:
                        event_str, buffer = buffer.split("\n\n", 1)
                        task = self._parse_sse_event(event_str)
                        if task is not None:
                            if self.queue.full():
                                self.queue.get_nowait()  # Drop oldest
                            await self.queue.put(task)
        except (asyncio.CancelledError, httpx.ReadError):
            pass

    def _parse_sse_event(self, event_str: str) -> dict | None:
        """Parse an SSE event string into a task dict, or None for comments/keepalives."""
        data_lines = []
        for line in event_str.strip().split("\n"):
            if line.startswith("data: "):
                data_lines.append(line[6:])
            elif line.startswith("data:"):
                data_lines.append(line[5:])
        if not data_lines:
            return None
        try:
            return json.loads("".join(data_lines))
        except (json.JSONDecodeError, ValueError):
            return None

    def drain(self, max_items: int | None = None) -> list[dict]:
        """Retrieve buffered messages, optionally limited to max_items."""
        result = []
        count = 0
        while not self.queue.empty():
            if max_items is not None and count >= max_items:
                break
            try:
                result.append(self.queue.get_nowait())
                count += 1
            except asyncio.QueueEmpty:
                break
        return result

    async def disconnect(self) -> None:
        """Stop SSE connection and clean up resources."""
        if self._task is not None:
            self._task.cancel()
            if isinstance(self._task, asyncio.Task):
                try:
                    await self._task
                except asyncio.CancelledError:
                    pass
            self._task = None

        if self._client is not None:
            await self._client.aclose()
            self._client = None

        # Drain remaining buffer
        while not self.queue.empty():
            try:
                self.queue.get_nowait()
            except asyncio.QueueEmpty:
                break
