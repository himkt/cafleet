"""In-process Pub/Sub fan-out.

Design: per-channel subscriber sets, each subscriber is an
``asyncio.Queue``. ``publish()`` iterates the subscriber set and calls
``put_nowait`` on every queue. ``subscribe()`` constructs a fresh queue,
registers it, and returns a ``_Subscription`` wrapping the queue.

Single-process only: the subscriber registry lives in this process's
memory, so a publish in uvicorn worker A cannot reach a subscriber in
worker B. The registry server MUST run with a single worker.
"""

from __future__ import annotations

import asyncio
from collections.abc import AsyncIterator


class _Subscription:
    """Async iterator wrapping a single subscriber's queue."""

    def __init__(self, queue: asyncio.Queue[str]) -> None:
        self._queue = queue

    def __aiter__(self) -> AsyncIterator[str]:
        return self

    async def __anext__(self) -> str:
        return await self._queue.get()


class PubSubManager:
    """In-process Pub/Sub fan-out for inbox notification channels."""

    def __init__(self) -> None:
        self._subscribers: dict[str, set[asyncio.Queue[str]]] = {}

    async def publish(self, channel: str, message: str) -> None:
        subscribers = self._subscribers.get(channel)
        if not subscribers:
            return
        for queue in subscribers:
            queue.put_nowait(message)

    async def subscribe(self, channel: str) -> _Subscription:
        queue: asyncio.Queue[str] = asyncio.Queue()
        self._subscribers.setdefault(channel, set()).add(queue)
        return _Subscription(queue)

    async def unsubscribe(self, channel: str, subscription: _Subscription) -> None:
        subscribers = self._subscribers.get(channel)
        if subscribers is None:
            return
        subscribers.discard(subscription._queue)
        if not subscribers:
            del self._subscribers[channel]
