"""Integration tests for the full broker: Registry + A2A operations.

Tests end-to-end flows through the ASGI app (FastAPI + JSON-RPC):
- Registry flow: register → list → get → deregister
- Unicast flow: send → list → get → ACK → verify COMPLETED
- Broadcast flow: send → each recipient lists → ACK → verify all COMPLETED
- CancelTask retraction

Design doc 0000015 Step 10: replace ``store.create_api_key(owner_sub)``
fixture with direct session creation; replace ``Authorization: Bearer``
with ``X-Session-Id``; replace ``api_key`` variables with ``session_id``.
"""

import uuid
from datetime import UTC, datetime

import pytest
from httpx import ASGITransport, AsyncClient
from sqlalchemy.ext.asyncio import AsyncSession, async_sessionmaker

from hikyaku_registry.db.models import Session as SessionModel
from hikyaku_registry.main import create_app
from hikyaku_registry.registry_store import RegistryStore


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------


async def _create_test_session(
    db_sessionmaker: async_sessionmaker[AsyncSession],
    *,
    session_id: str | None = None,
    label: str | None = None,
) -> str:
    """Seed a session row directly via the DB sessionmaker."""
    if session_id is None:
        session_id = str(uuid.uuid4())
    created_at = datetime.now(UTC).isoformat()

    async with db_sessionmaker() as session:
        async with session.begin():
            session.add(
                SessionModel(
                    session_id=session_id,
                    label=label,
                    created_at=created_at,
                )
            )
    return session_id


# ---------------------------------------------------------------------------
# Fixtures
# ---------------------------------------------------------------------------


@pytest.fixture
async def broker_env(db_sessionmaker):
    """Full broker ASGI app backed by in-memory aiosqlite.

    Yields a dict with ``client`` (httpx.AsyncClient) and ``session_id``
    (a freshly created session). Tests register agents through
    the HTTP API using this session.
    """
    session_id = await _create_test_session(db_sessionmaker)

    app = create_app(sessionmaker=db_sessionmaker)
    transport = ASGITransport(app=app)

    async with AsyncClient(transport=transport, base_url="http://test") as client:
        yield {"client": client, "session_id": session_id}


async def _register_agent(
    client,
    session_id: str,
    name: str = "Test Agent",
    description: str = "A test agent",
    skills=None,
):
    """Register an agent via POST and return the response data."""
    body = {"name": name, "description": description, "session_id": session_id}
    if skills is not None:
        body["skills"] = skills
    resp = await client.post("/api/v1/agents", json=body)
    assert resp.status_code == 201, f"Registration failed: {resp.text}"
    return resp.json()


# ---------------------------------------------------------------------------
# Auth headers
# ---------------------------------------------------------------------------


def _auth(session_id: str, agent_id: str = "") -> dict:
    """Build X-Session-Id + X-Agent-Id headers."""
    headers = {"X-Session-Id": session_id}
    if agent_id:
        headers["X-Agent-Id"] = agent_id
    return headers


def _jsonrpc(method: str, params: dict, req_id: str | None = None) -> dict:
    """Build a JSON-RPC 2.0 request."""
    return {
        "jsonrpc": "2.0",
        "method": method,
        "params": params,
        "id": req_id or str(uuid.uuid4()),
    }


async def _send_message(client, session_id, agent_id, destination, text="Hello"):
    """Send a unicast or broadcast message via A2A SendMessage."""
    payload = _jsonrpc(
        "SendMessage",
        {
            "message": {
                "messageId": str(uuid.uuid4()),
                "role": "user",
                "parts": [{"kind": "text", "text": text}],
                "metadata": {"destination": destination},
            },
        },
    )
    resp = await client.post("/", json=payload, headers=_auth(session_id, agent_id))
    assert resp.status_code == 200
    data = resp.json()
    assert "result" in data, f"Expected result, got: {data}"
    return data["result"]


async def _list_tasks(client, session_id, agent_id, context_id, status=None):
    """Poll inbox via A2A ListTasks."""
    params = {"contextId": context_id}
    if status:
        params["status"] = status
    payload = _jsonrpc("ListTasks", params)
    resp = await client.post("/", json=payload, headers=_auth(session_id, agent_id))
    assert resp.status_code == 200
    data = resp.json()
    assert "result" in data, f"Expected result, got: {data}"
    return data["result"]


async def _get_task(client, session_id, agent_id, task_id):
    """Get a specific task via A2A GetTask."""
    payload = _jsonrpc("GetTask", {"id": task_id})
    resp = await client.post("/", json=payload, headers=_auth(session_id, agent_id))
    assert resp.status_code == 200
    return resp.json()


async def _ack_task(client, session_id, agent_id, task_id, text="ack"):
    """Acknowledge a task via A2A SendMessage (multi-turn)."""
    payload = _jsonrpc(
        "SendMessage",
        {
            "message": {
                "messageId": str(uuid.uuid4()),
                "role": "user",
                "taskId": task_id,
                "parts": [{"kind": "text", "text": text}],
            },
        },
    )
    resp = await client.post("/", json=payload, headers=_auth(session_id, agent_id))
    assert resp.status_code == 200
    data = resp.json()
    assert "result" in data, f"Expected result, got: {data}"
    return data["result"]


async def _cancel_task(client, session_id, agent_id, task_id):
    """Cancel a task via A2A CancelTask."""
    payload = _jsonrpc("CancelTask", {"id": task_id})
    resp = await client.post("/", json=payload, headers=_auth(session_id, agent_id))
    assert resp.status_code == 200
    return resp.json()


# ---------------------------------------------------------------------------
# Registry Flow: register → list → get → deregister
# ---------------------------------------------------------------------------


class TestRegistryFlow:
    """End-to-end test of Registry REST API operations."""

    async def test_full_lifecycle(self, broker_env):
        """Register → list → get detail → deregister → verify removed."""
        client, session_id = broker_env["client"], broker_env["session_id"]

        # 1. Register
        agent = await _register_agent(
            client, session_id, name="Flow Agent", description="E2E test"
        )
        agent_id = agent["agent_id"]

        assert agent["name"] == "Flow Agent"

        # Register a second agent in same session for later verification
        checker = await _register_agent(client, session_id, name="Checker")

        # 2. List — should include this agent
        resp = await client.get("/api/v1/agents", headers=_auth(session_id, agent_id))
        assert resp.status_code == 200
        agents = resp.json()["agents"]
        agent_ids = {a["agent_id"] for a in agents}
        assert agent_id in agent_ids

        # 3. Get detail
        resp = await client.get(
            f"/api/v1/agents/{agent_id}", headers=_auth(session_id, agent_id)
        )
        assert resp.status_code == 200
        detail = resp.json()
        assert detail["name"] == "Flow Agent"

        # 4. Deregister
        resp = await client.delete(
            f"/api/v1/agents/{agent_id}", headers=_auth(session_id, agent_id)
        )
        assert resp.status_code == 204

        # 5. Verify removed from list
        resp = await client.get(
            "/api/v1/agents", headers=_auth(session_id, checker["agent_id"])
        )
        agents = resp.json()["agents"]
        agent_ids = {a["agent_id"] for a in agents}
        assert agent_id not in agent_ids

    async def test_register_multiple_and_list(self, broker_env):
        """Register multiple agents and verify all appear in list."""
        client, session_id = broker_env["client"], broker_env["session_id"]

        agents = []
        for i in range(3):
            a = await _register_agent(
                client, session_id, name=f"Agent {i}", description=f"Agent {i}"
            )
            agents.append(a)

        resp = await client.get(
            "/api/v1/agents", headers=_auth(session_id, agents[0]["agent_id"])
        )
        assert resp.status_code == 200
        listed = resp.json()["agents"]
        listed_ids = {a["agent_id"] for a in listed}

        for a in agents:
            assert a["agent_id"] in listed_ids


# ---------------------------------------------------------------------------
# Unicast Flow: send → list → get → ACK → verify COMPLETED
# ---------------------------------------------------------------------------


class TestUnicastFlow:
    """End-to-end test of unicast message delivery and acknowledgment."""

    async def test_send_list_get_ack(self, broker_env):
        """Full unicast: A sends to B → B lists → B gets → B ACKs → COMPLETED."""
        client, session_id = broker_env["client"], broker_env["session_id"]

        # Register 2 agents in the same session
        agent_a = await _register_agent(client, session_id, name="Sender A")
        agent_b = await _register_agent(client, session_id, name="Recipient B")

        # 1. Agent A sends unicast to Agent B
        result = await _send_message(
            client,
            session_id,
            agent_a["agent_id"],
            destination=agent_b["agent_id"],
            text="Did the API schema change?",
        )
        task = result["task"]
        task_id = task["id"]

        assert task["status"]["state"] == "input-required"
        assert task["contextId"] == agent_b["agent_id"]

        # 2. Agent B polls inbox via ListTasks
        list_result = await _list_tasks(
            client,
            session_id,
            agent_b["agent_id"],
            context_id=agent_b["agent_id"],
        )
        tasks = list_result.get("tasks", list_result)
        task_ids = [
            t["id"] if isinstance(t, dict) else t
            for t in (tasks if isinstance(tasks, list) else [tasks])
        ]
        assert task_id in task_ids

        # 3. Agent B gets the specific task
        get_result = await _get_task(client, session_id, agent_b["agent_id"], task_id)
        assert "result" in get_result

        # 4. Agent B ACKs the message
        ack_result = await _ack_task(client, session_id, agent_b["agent_id"], task_id)
        ack_task = ack_result["task"]
        assert ack_task["status"]["state"] == "completed"

    async def test_sender_can_get_task_after_send(self, broker_env):
        """Sender can retrieve the task by its ID after sending."""
        client, session_id = broker_env["client"], broker_env["session_id"]

        agent_a = await _register_agent(client, session_id, name="Sender")
        agent_b = await _register_agent(client, session_id, name="Recipient")

        result = await _send_message(
            client, session_id, agent_a["agent_id"], destination=agent_b["agent_id"]
        )
        task_id = result["task"]["id"]

        get_result = await _get_task(client, session_id, agent_a["agent_id"], task_id)
        assert "result" in get_result

    async def test_send_to_nonexistent_agent_returns_error(self, broker_env):
        """Sending to a non-existent agent returns a JSON-RPC error."""
        client, session_id = broker_env["client"], broker_env["session_id"]

        agent_a = await _register_agent(client, session_id, name="Sender")

        payload = _jsonrpc(
            "SendMessage",
            {
                "message": {
                    "messageId": str(uuid.uuid4()),
                    "role": "user",
                    "parts": [{"kind": "text", "text": "Hello?"}],
                    "metadata": {"destination": "00000000-0000-4000-8000-000000000000"},
                },
            },
        )
        resp = await client.post(
            "/", json=payload, headers=_auth(session_id, agent_a["agent_id"])
        )
        data = resp.json()

        assert "error" in data

    async def test_missing_agent_id_returns_400(self, broker_env):
        """A2A SendMessage without X-Agent-Id header returns HTTP 400."""
        client = broker_env["client"]

        payload = _jsonrpc(
            "SendMessage",
            {
                "message": {
                    "messageId": str(uuid.uuid4()),
                    "role": "user",
                    "parts": [{"kind": "text", "text": "No agent id"}],
                    "metadata": {"destination": "some-agent"},
                },
            },
        )
        resp = await client.post("/", json=payload)
        assert resp.status_code == 400


# ---------------------------------------------------------------------------
# Broadcast Flow: send → each lists → ACK → verify all COMPLETED
# ---------------------------------------------------------------------------


class TestBroadcastFlow:
    """End-to-end test of broadcast message delivery."""

    async def test_broadcast_and_ack_all(self, broker_env):
        """Broadcast: A sends to * → B and C each list and ACK → all COMPLETED."""
        client, session_id = broker_env["client"], broker_env["session_id"]

        agent_a = await _register_agent(client, session_id, name="Broadcaster A")
        agent_b = await _register_agent(client, session_id, name="Recipient B")
        agent_c = await _register_agent(client, session_id, name="Recipient C")

        # 1. Agent A broadcasts
        result = await _send_message(
            client,
            session_id,
            agent_a["agent_id"],
            destination="*",
            text="Build failed on main branch",
        )
        summary_task = result["task"]
        assert summary_task["status"]["state"] == "completed"

        # 2. Agent B polls inbox and ACKs
        b_list = await _list_tasks(
            client, session_id, agent_b["agent_id"], context_id=agent_b["agent_id"]
        )
        b_tasks = b_list.get("tasks", b_list)
        b_tasks = b_tasks if isinstance(b_tasks, list) else [b_tasks]
        assert len(b_tasks) >= 1
        b_task_id = b_tasks[0]["id"] if isinstance(b_tasks[0], dict) else b_tasks[0]

        ack_b = await _ack_task(client, session_id, agent_b["agent_id"], b_task_id)
        assert ack_b["task"]["status"]["state"] == "completed"

        # 3. Agent C polls inbox and ACKs
        c_list = await _list_tasks(
            client, session_id, agent_c["agent_id"], context_id=agent_c["agent_id"]
        )
        c_tasks = c_list.get("tasks", c_list)
        c_tasks = c_tasks if isinstance(c_tasks, list) else [c_tasks]
        assert len(c_tasks) >= 1
        c_task_id = c_tasks[0]["id"] if isinstance(c_tasks[0], dict) else c_tasks[0]

        ack_c = await _ack_task(client, session_id, agent_c["agent_id"], c_task_id)
        assert ack_c["task"]["status"]["state"] == "completed"

    async def test_broadcast_excludes_sender(self, broker_env):
        """Sender does not receive their own broadcast."""
        client, session_id = broker_env["client"], broker_env["session_id"]

        agent_a = await _register_agent(client, session_id, name="Broadcaster")
        _agent_b = await _register_agent(client, session_id, name="Listener")

        await _send_message(client, session_id, agent_a["agent_id"], destination="*")

        # Sender's inbox should be empty (no self-delivery)
        a_list = await _list_tasks(
            client, session_id, agent_a["agent_id"], context_id=agent_a["agent_id"]
        )
        a_tasks = a_list.get("tasks", a_list)
        if isinstance(a_tasks, list):
            assert len(a_tasks) == 0
        else:
            assert a_tasks is None or a_tasks == []


# ---------------------------------------------------------------------------
# CancelTask — Message Retraction
# ---------------------------------------------------------------------------


class TestCancelTaskFlow:
    """End-to-end test of message retraction via CancelTask."""

    async def test_sender_cancels_unread_message(self, broker_env):
        """Sender cancels an unread (INPUT_REQUIRED) message → CANCELED."""
        client, session_id = broker_env["client"], broker_env["session_id"]

        agent_a = await _register_agent(client, session_id, name="Sender")
        agent_b = await _register_agent(client, session_id, name="Recipient")

        result = await _send_message(
            client, session_id, agent_a["agent_id"], destination=agent_b["agent_id"]
        )
        task_id = result["task"]["id"]

        cancel_result = await _cancel_task(
            client, session_id, agent_a["agent_id"], task_id
        )
        assert "result" in cancel_result
        assert cancel_result["result"]["task"]["status"]["state"] == "canceled"

    async def test_cancel_already_acked_returns_error(self, broker_env):
        """Cannot cancel a message that has already been ACKed."""
        client, session_id = broker_env["client"], broker_env["session_id"]

        agent_a = await _register_agent(client, session_id, name="Sender")
        agent_b = await _register_agent(client, session_id, name="Recipient")

        result = await _send_message(
            client, session_id, agent_a["agent_id"], destination=agent_b["agent_id"]
        )
        task_id = result["task"]["id"]
        await _ack_task(client, session_id, agent_b["agent_id"], task_id)

        cancel_result = await _cancel_task(
            client, session_id, agent_a["agent_id"], task_id
        )
        assert "error" in cancel_result

    async def test_non_sender_cannot_cancel(self, broker_env):
        """Recipient cannot cancel a task — only the sender can."""
        client, session_id = broker_env["client"], broker_env["session_id"]

        agent_a = await _register_agent(client, session_id, name="Sender")
        agent_b = await _register_agent(client, session_id, name="Recipient")

        result = await _send_message(
            client, session_id, agent_a["agent_id"], destination=agent_b["agent_id"]
        )
        task_id = result["task"]["id"]

        cancel_result = await _cancel_task(
            client, session_id, agent_b["agent_id"], task_id
        )
        assert "error" in cancel_result

    async def test_canceled_message_not_in_inbox(self, broker_env):
        """After cancellation, the message should not appear as unread in recipient's inbox."""
        client, session_id = broker_env["client"], broker_env["session_id"]

        agent_a = await _register_agent(client, session_id, name="Sender")
        agent_b = await _register_agent(client, session_id, name="Recipient")

        result = await _send_message(
            client, session_id, agent_a["agent_id"], destination=agent_b["agent_id"]
        )
        task_id = result["task"]["id"]
        await _cancel_task(client, session_id, agent_a["agent_id"], task_id)

        list_result = await _list_tasks(
            client,
            session_id,
            agent_b["agent_id"],
            context_id=agent_b["agent_id"],
            status="input-required",
        )
        tasks = list_result.get("tasks", list_result)
        if isinstance(tasks, list):
            task_ids = [t["id"] if isinstance(t, dict) else t for t in tasks]
            assert task_id not in task_ids

    async def test_broadcast_cancel_one_delivery(self, broker_env):
        """Sender can cancel one delivery task from a broadcast."""
        client, session_id = broker_env["client"], broker_env["session_id"]

        agent_a = await _register_agent(client, session_id, name="Broadcaster")
        _agent_b = await _register_agent(client, session_id, name="Recipient B")
        _agent_c = await _register_agent(client, session_id, name="Recipient C")

        result = await _send_message(
            client, session_id, agent_a["agent_id"], destination="*"
        )
        summary = result["task"]

        delivery_ids = []
        if summary.get("artifacts"):
            for artifact in summary["artifacts"]:
                for part in artifact.get("parts", []):
                    if isinstance(part, dict) and "data" in part:
                        data = part["data"]
                        if isinstance(data, dict) and "deliveryTaskIds" in data:
                            delivery_ids = data["deliveryTaskIds"]

        if delivery_ids:
            cancel_result = await _cancel_task(
                client, session_id, agent_a["agent_id"], delivery_ids[0]
            )
            assert "result" in cancel_result

            if len(delivery_ids) > 1:
                get_result = await _get_task(
                    client, session_id, agent_a["agent_id"], delivery_ids[1]
                )
                assert "result" in get_result
