"""Tests for subscribe.py — SSE endpoint for real-time inbox notifications.

Covers: authentication errors (401 via HTTP), and unit tests for the SSE
event generator logic (message streaming, keepalive, disconnect cleanup).

Endpoint: GET /api/v1/subscribe
Auth: Authorization: Bearer <api_key> + X-Agent-Id: <agent_id>
Response: text/event-stream

The Redis-backed predecessor used a fakeredis-based PubSub and task store,
and asserted post-cleanup state via ``redis.pubsub_numsub``. The SQL
rewrite uses the in-process ``PubSubManager`` (no Redis dependency) with
the conftest SQL fixtures and inspects ``pubsub._subscribers`` directly.
"""

import asyncio
import json
import uuid
from datetime import UTC, datetime
from unittest.mock import AsyncMock

import pytest
from a2a.types import Artifact, Part, Task, TaskState, TaskStatus, TextPart
from httpx import ASGITransport, AsyncClient

from hikyaku_registry.api.subscribe import event_generator
from hikyaku_registry.main import create_app
from hikyaku_registry.pubsub import PubSubManager
from hikyaku_registry.registry_store import RegistryStore
from hikyaku_registry.task_store import TaskStore


_SSE_OWNER_SUB = "auth0|sse-test"
_SSE_OTHER_OWNER_SUB = "auth0|sse-test-other"


# ---------------------------------------------------------------------------
# Fixtures — HTTP (for auth tests)
# ---------------------------------------------------------------------------


@pytest.fixture
async def sse_env(db_sessionmaker):
    """Provide a full ASGI app with the SQL store for SSE endpoint testing.

    Yields a dict with client, store, pre-registered agents, and the active
    api_key (used for Authorization headers). A second api_key is created
    up-front so tenant-mismatch tests can register an agent under a
    different tenant.
    """
    store = RegistryStore(db_sessionmaker)
    api_key, _, _ = await store.create_api_key(_SSE_OWNER_SUB)
    other_api_key, _, _ = await store.create_api_key(_SSE_OTHER_OWNER_SUB)

    app = create_app(sessionmaker=db_sessionmaker)
    transport = ASGITransport(app=app)

    async with AsyncClient(transport=transport, base_url="http://test") as client:
        agent_a = await _register_agent(client, "Agent A", "Sender", api_key=api_key)
        agent_b = await _register_agent(client, "Agent B", "Receiver", api_key=api_key)

        yield {
            "client": client,
            "store": store,
            "app": app,
            "agent_a": agent_a,
            "agent_b": agent_b,
            "api_key": api_key,
            "other_api_key": other_api_key,
        }


# ---------------------------------------------------------------------------
# Fixtures — Unit (for generator tests)
# ---------------------------------------------------------------------------


@pytest.fixture
async def gen_env(db_sessionmaker):
    """Provide PubSubManager, TaskStore, and a test agent_id for generator tests."""
    pubsub = PubSubManager()
    task_store = TaskStore(db_sessionmaker)
    store = RegistryStore(db_sessionmaker)

    # Create an api_key + agent so task_store.save's FK constraint succeeds
    api_key, _, _ = await store.create_api_key(_SSE_OWNER_SUB)
    created = await store.create_agent(
        name="Gen Test Agent",
        description="Receiver",
        api_key=api_key,
    )

    yield {
        "pubsub": pubsub,
        "task_store": task_store,
        "agent_id": created["agent_id"],
    }


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------


async def _register_agent(client, name, description, api_key):
    """Register an agent via POST and return the response data."""
    body = {"name": name, "description": description}
    headers = {"Authorization": f"Bearer {api_key}"}
    resp = await client.post("/api/v1/agents", json=body, headers=headers)
    assert resp.status_code == 201, f"Registration failed: {resp.text}"
    return resp.json()


def _auth(api_key: str, agent_id: str) -> dict:
    """Build Authorization + X-Agent-Id headers for SSE endpoint."""
    return {
        "Authorization": f"Bearer {api_key}",
        "X-Agent-Id": agent_id,
    }


def _make_task(
    task_id: str,
    context_id: str,
    sender_id: str,
    text: str = "Hello",
) -> Task:
    """Create an A2A Task for testing."""
    now = datetime.now(UTC).isoformat()
    return Task(
        id=task_id,
        context_id=context_id,
        status=TaskStatus(state=TaskState.input_required, timestamp=now),
        artifacts=[
            Artifact(
                artifact_id=str(uuid.uuid4()),
                parts=[Part(root=TextPart(text=text))],
            )
        ],
        metadata={
            "fromAgentId": sender_id,
            "toAgentId": context_id,
            "type": "unicast",
        },
    )


def _parse_sse_events(raw_chunks: list[str]) -> list[dict]:
    """Parse raw SSE output chunks into a list of event dicts.

    Each event dict may contain keys: 'event', 'id', 'data', 'comment'.
    """
    events = []
    current = {}

    lines = []
    for chunk in raw_chunks:
        lines.extend(chunk.split("\n"))

    for line in lines:
        line = line.rstrip("\r")

        if line == "":
            if current:
                events.append(current)
                current = {}
            continue

        if line.startswith(":"):
            current["comment"] = line[1:].strip()
            events.append(current)
            current = {}
            continue

        if ":" in line:
            field, _, value = line.partition(":")
            value = value.lstrip(" ")
            current[field] = value

    if current:
        events.append(current)

    return events


# ---------------------------------------------------------------------------
# Authentication Errors
# ---------------------------------------------------------------------------


class TestSSEAuth:
    """Tests for SSE endpoint authentication — all error cases return 401."""

    async def test_missing_authorization_header_returns_401(self, sse_env):
        """GET /api/v1/subscribe without Authorization header returns 401."""
        client = sse_env["client"]
        agent_b = sse_env["agent_b"]

        resp = await client.get(
            "/api/v1/subscribe",
            headers={"X-Agent-Id": agent_b["agent_id"]},
        )
        assert resp.status_code == 401

    async def test_missing_x_agent_id_header_returns_401(self, sse_env):
        """GET /api/v1/subscribe without X-Agent-Id header returns 401."""
        client = sse_env["client"]
        api_key = sse_env["api_key"]

        resp = await client.get(
            "/api/v1/subscribe",
            headers={"Authorization": f"Bearer {api_key}"},
        )
        assert resp.status_code == 401

    async def test_invalid_api_key_returns_401(self, sse_env):
        """GET /api/v1/subscribe with unknown API key returns 401."""
        client = sse_env["client"]
        agent_b = sse_env["agent_b"]

        resp = await client.get(
            "/api/v1/subscribe",
            headers=_auth("hky_invalid_key_000000000000000000", agent_b["agent_id"]),
        )
        assert resp.status_code == 401

    async def test_nonexistent_agent_id_returns_401(self, sse_env):
        """GET /api/v1/subscribe with nonexistent agent_id returns 401."""
        client = sse_env["client"]
        api_key = sse_env["api_key"]

        resp = await client.get(
            "/api/v1/subscribe",
            headers=_auth(api_key, "nonexistent-agent-id"),
        )
        assert resp.status_code == 401

    async def test_tenant_mismatch_returns_401(self, sse_env):
        """GET /api/v1/subscribe with agent from different tenant returns 401."""
        client = sse_env["client"]
        api_key = sse_env["api_key"]
        other_api_key = sse_env["other_api_key"]

        other_agent = await _register_agent(
            client, "Other Agent", "Different tenant", api_key=other_api_key
        )

        resp = await client.get(
            "/api/v1/subscribe",
            headers=_auth(api_key, other_agent["agent_id"]),
        )
        assert resp.status_code == 401

    async def test_malformed_bearer_token_returns_401(self, sse_env):
        """GET /api/v1/subscribe with 'Basic' scheme instead of 'Bearer' returns 401."""
        client = sse_env["client"]
        agent_b = sse_env["agent_b"]

        resp = await client.get(
            "/api/v1/subscribe",
            headers={
                "Authorization": "Basic dXNlcjpwYXNz",
                "X-Agent-Id": agent_b["agent_id"],
            },
        )
        assert resp.status_code == 401

    async def test_empty_bearer_token_returns_401(self, sse_env):
        """GET /api/v1/subscribe with empty Bearer token returns 401."""
        client = sse_env["client"]
        agent_b = sse_env["agent_b"]

        resp = await client.get(
            "/api/v1/subscribe",
            headers={
                "Authorization": "Bearer ",
                "X-Agent-Id": agent_b["agent_id"],
            },
        )
        assert resp.status_code == 401


# ---------------------------------------------------------------------------
# SSE Event Generator — Unit Tests
# ---------------------------------------------------------------------------


class TestSSEGeneratorMessages:
    """Unit tests for event_generator() — message event streaming.

    Tests the generator directly with the in-process PubSubManager and
    SQL TaskStore, bypassing HTTP transport. A mock request with
    is_disconnected() → False simulates an active client.
    """

    async def test_yields_message_event_on_publish(self, gen_env):
        """Generator yields an SSE 'message' event when task_id is published."""
        pubsub = gen_env["pubsub"]
        task_store = gen_env["task_store"]
        agent_id = gen_env["agent_id"]

        task_id = str(uuid.uuid4())
        task = _make_task(task_id, agent_id, sender_id=agent_id, text="Hello via SSE")
        await task_store.save(task)

        request = AsyncMock()
        request.is_disconnected = AsyncMock(return_value=False)

        collected = []

        async def consume():
            async for chunk in event_generator(
                agent_id=agent_id,
                pubsub=pubsub,
                task_store=task_store,
                request=request,
            ):
                collected.append(chunk)
                if "data:" in chunk:
                    break

        consumer_task = asyncio.create_task(consume())

        await asyncio.sleep(0.05)

        await pubsub.publish(f"inbox:{agent_id}", task_id)

        await asyncio.wait_for(consumer_task, timeout=3.0)

        events = _parse_sse_events(collected)
        message_events = [e for e in events if e.get("event") == "message"]
        assert len(message_events) >= 1, f"No message events. Chunks: {collected}"

    async def test_event_id_matches_task_id(self, gen_env):
        """SSE event 'id' field equals the task_id."""
        pubsub = gen_env["pubsub"]
        task_store = gen_env["task_store"]
        agent_id = gen_env["agent_id"]

        task_id = str(uuid.uuid4())
        task = _make_task(task_id, agent_id, sender_id=agent_id)
        await task_store.save(task)

        request = AsyncMock()
        request.is_disconnected = AsyncMock(return_value=False)

        collected = []

        async def consume():
            async for chunk in event_generator(
                agent_id=agent_id,
                pubsub=pubsub,
                task_store=task_store,
                request=request,
            ):
                collected.append(chunk)
                if "data:" in chunk:
                    break

        consumer_task = asyncio.create_task(consume())
        await asyncio.sleep(0.05)
        await pubsub.publish(f"inbox:{agent_id}", task_id)
        await asyncio.wait_for(consumer_task, timeout=3.0)

        events = _parse_sse_events(collected)
        message_events = [e for e in events if e.get("event") == "message"]
        assert len(message_events) >= 1

        event = message_events[0]
        assert event["id"] == task_id

    async def test_event_data_is_full_task_json(self, gen_env):
        """SSE event 'data' field is full A2A Task JSON with camelCase keys."""
        pubsub = gen_env["pubsub"]
        task_store = gen_env["task_store"]
        agent_id = gen_env["agent_id"]

        task_id = str(uuid.uuid4())
        task = _make_task(task_id, agent_id, sender_id=agent_id, text="Full JSON check")
        await task_store.save(task)

        request = AsyncMock()
        request.is_disconnected = AsyncMock(return_value=False)

        collected = []

        async def consume():
            async for chunk in event_generator(
                agent_id=agent_id,
                pubsub=pubsub,
                task_store=task_store,
                request=request,
            ):
                collected.append(chunk)
                if "data:" in chunk:
                    break

        consumer_task = asyncio.create_task(consume())
        await asyncio.sleep(0.05)
        await pubsub.publish(f"inbox:{agent_id}", task_id)
        await asyncio.wait_for(consumer_task, timeout=3.0)

        events = _parse_sse_events(collected)
        message_events = [e for e in events if e.get("event") == "message"]
        assert len(message_events) >= 1

        task_data = json.loads(message_events[0]["data"])

        assert task_data["id"] == task_id
        assert "status" in task_data
        context_id_key = "contextId" if "contextId" in task_data else "context_id"
        assert task_data.get(context_id_key) == agent_id

    async def test_multiple_messages_arrive_as_separate_events(self, gen_env):
        """Multiple published task_ids produce multiple SSE message events."""
        pubsub = gen_env["pubsub"]
        task_store = gen_env["task_store"]
        agent_id = gen_env["agent_id"]

        task_ids = [str(uuid.uuid4()) for _ in range(3)]
        for tid in task_ids:
            task = _make_task(tid, agent_id, sender_id=agent_id, text=f"Msg {tid[:8]}")
            await task_store.save(task)

        request = AsyncMock()
        request.is_disconnected = AsyncMock(return_value=False)

        collected = []
        data_count = 0

        async def consume():
            nonlocal data_count
            async for chunk in event_generator(
                agent_id=agent_id,
                pubsub=pubsub,
                task_store=task_store,
                request=request,
            ):
                collected.append(chunk)
                if "data:" in chunk:
                    data_count += 1
                    if data_count >= 3:
                        break

        consumer_task = asyncio.create_task(consume())
        await asyncio.sleep(0.05)

        for tid in task_ids:
            await pubsub.publish(f"inbox:{agent_id}", tid)

        await asyncio.wait_for(consumer_task, timeout=3.0)

        events = _parse_sse_events(collected)
        message_events = [e for e in events if e.get("event") == "message"]
        assert len(message_events) >= 3

        event_ids = {e["id"] for e in message_events}
        assert event_ids == set(task_ids)

    async def test_skips_missing_task(self, gen_env):
        """If task_store.get returns None for a task_id, the event is skipped."""
        pubsub = gen_env["pubsub"]
        task_store = gen_env["task_store"]
        agent_id = gen_env["agent_id"]

        missing_task_id = str(uuid.uuid4())
        real_task_id = str(uuid.uuid4())
        real_task = _make_task(
            real_task_id, agent_id, sender_id=agent_id, text="I exist"
        )
        await task_store.save(real_task)

        request = AsyncMock()
        request.is_disconnected = AsyncMock(return_value=False)

        collected = []

        async def consume():
            async for chunk in event_generator(
                agent_id=agent_id,
                pubsub=pubsub,
                task_store=task_store,
                request=request,
            ):
                collected.append(chunk)
                if "data:" in chunk:
                    break

        consumer_task = asyncio.create_task(consume())
        await asyncio.sleep(0.05)

        await pubsub.publish(f"inbox:{agent_id}", missing_task_id)
        await pubsub.publish(f"inbox:{agent_id}", real_task_id)

        await asyncio.wait_for(consumer_task, timeout=3.0)

        events = _parse_sse_events(collected)
        message_events = [e for e in events if e.get("event") == "message"]

        assert len(message_events) >= 1
        assert message_events[0]["id"] == real_task_id


# ---------------------------------------------------------------------------
# SSE Event Generator — Keepalive
# ---------------------------------------------------------------------------


class TestSSEGeneratorKeepalive:
    """Unit tests for event_generator() keepalive behavior."""

    async def test_keepalive_comment_sent_after_interval(self, gen_env, monkeypatch):
        """Generator yields ': keepalive' comment after the keepalive interval.

        Monkeypatches _keepalive_interval to a short value for fast testing.
        """
        import hikyaku_registry.api.subscribe as subscribe_mod

        monkeypatch.setattr(subscribe_mod, "_keepalive_interval", 0.1)

        pubsub = gen_env["pubsub"]
        task_store = gen_env["task_store"]
        agent_id = gen_env["agent_id"]

        request = AsyncMock()
        request.is_disconnected = AsyncMock(return_value=False)

        collected = []

        async def consume():
            async for chunk in event_generator(
                agent_id=agent_id,
                pubsub=pubsub,
                task_store=task_store,
                request=request,
            ):
                collected.append(chunk)
                if "keepalive" in chunk:
                    break

        await asyncio.wait_for(consume(), timeout=3.0)

        keepalive_chunks = [c for c in collected if "keepalive" in c]
        assert len(keepalive_chunks) >= 1

    async def test_keepalive_does_not_interfere_with_messages(
        self, gen_env, monkeypatch
    ):
        """Messages still arrive correctly even when keepalives are firing."""
        import hikyaku_registry.api.subscribe as subscribe_mod

        monkeypatch.setattr(subscribe_mod, "_keepalive_interval", 0.1)

        pubsub = gen_env["pubsub"]
        task_store = gen_env["task_store"]
        agent_id = gen_env["agent_id"]

        task_id = str(uuid.uuid4())
        task = _make_task(task_id, agent_id, sender_id=agent_id, text="With keepalive")
        await task_store.save(task)

        request = AsyncMock()
        request.is_disconnected = AsyncMock(return_value=False)

        collected = []

        async def consume():
            async for chunk in event_generator(
                agent_id=agent_id,
                pubsub=pubsub,
                task_store=task_store,
                request=request,
            ):
                collected.append(chunk)
                if "data:" in chunk:
                    break

        consumer_task = asyncio.create_task(consume())
        await asyncio.sleep(0.15)
        await pubsub.publish(f"inbox:{agent_id}", task_id)

        await asyncio.wait_for(consumer_task, timeout=3.0)

        events = _parse_sse_events(collected)
        message_events = [e for e in events if e.get("event") == "message"]
        keepalive_events = [e for e in events if e.get("comment") == "keepalive"]

        assert len(message_events) >= 1
        assert message_events[0]["id"] == task_id
        assert len(keepalive_events) >= 1


# ---------------------------------------------------------------------------
# SSE Event Generator — Disconnect Cleanup
# ---------------------------------------------------------------------------


class TestSSEGeneratorCleanup:
    """Unit tests for event_generator() cleanup on disconnect/exit."""

    async def test_cleanup_unsubscribes_on_generator_exit(self, gen_env):
        """When the generator is exited, the PubSub channel is unsubscribed."""
        pubsub = gen_env["pubsub"]
        task_store = gen_env["task_store"]
        agent_id = gen_env["agent_id"]
        channel = f"inbox:{agent_id}"

        request = AsyncMock()
        request.is_disconnected = AsyncMock(return_value=False)

        gen = event_generator(
            agent_id=agent_id,
            pubsub=pubsub,
            task_store=task_store,
            request=request,
        )

        async def consume_briefly():
            async for _chunk in gen:
                break

        try:
            await asyncio.wait_for(consume_briefly(), timeout=1.0)
        except asyncio.TimeoutError:
            pass

        await gen.aclose()

        await asyncio.sleep(0.05)

        # In-process PubSubManager: the channel entry is deleted when the
        # last subscriber unsubscribes, so the channel should be gone.
        subscribers = pubsub._subscribers.get(channel)
        assert not subscribers, (
            f"Expected no subscribers after cleanup, got {subscribers!r}"
        )

    async def test_cleanup_on_client_disconnect(self, gen_env, monkeypatch):
        """Generator exits cleanly when request.is_disconnected() returns True."""
        import hikyaku_registry.api.subscribe as subscribe_mod

        monkeypatch.setattr(subscribe_mod, "_keepalive_interval", 0.1)

        pubsub = gen_env["pubsub"]
        task_store = gen_env["task_store"]
        agent_id = gen_env["agent_id"]

        disconnect_after = 2
        call_count = 0

        async def mock_is_disconnected():
            nonlocal call_count
            call_count += 1
            return call_count > disconnect_after

        request = AsyncMock()
        request.is_disconnected = mock_is_disconnected

        collected = []

        async def consume():
            async for chunk in event_generator(
                agent_id=agent_id,
                pubsub=pubsub,
                task_store=task_store,
                request=request,
            ):
                collected.append(chunk)

        await asyncio.wait_for(consume(), timeout=3.0)

        assert call_count > disconnect_after
