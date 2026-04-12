"""Tests for webui_api.py — WebUI API endpoint behavior.

Covers: GET /ui/api/agents, GET /ui/api/agents/{agent_id}/inbox,
GET /ui/api/agents/{agent_id}/sent, POST /ui/api/messages/send.
Tests message formatting, response structure, broadcast filtering,
ordering, cross-tenant isolation, and deregistered agent access.

Auth mechanism tests are in test_webui_auth_migration.py.
"""

import uuid
from datetime import UTC, datetime, timedelta

import pytest
from a2a.types import (
    Artifact,
    Part,
    Task,
    TaskState,
    TaskStatus,
    TextPart,
)
from fastapi import FastAPI
from httpx import ASGITransport, AsyncClient

from hikyaku_registry.auth import get_user_id, verify_auth0_user
from hikyaku_registry.executor import BrokerExecutor
from hikyaku_registry.registry_store import RegistryStore
from hikyaku_registry.task_store import TaskStore
from hikyaku_registry.webui_api import (
    get_webui_executor,
    get_webui_store,
    get_webui_task_store,
    webui_router,
)


# ---------------------------------------------------------------------------
# Test constants
# ---------------------------------------------------------------------------

_TEST_SUB = "auth0|webui-test-user"
_TEST_JWT_TOKEN = "eyJ0eXAiOiJKV1QiLCJhbGciOiJSUzI1NiJ9.test.sig"


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------


def _auth_header(tenant_id: str) -> dict:
    """Build Authorization + X-Tenant-Id headers for JWT auth."""
    return {
        "Authorization": f"Bearer {_TEST_JWT_TOKEN}",
        "X-Tenant-Id": tenant_id,
    }


async def _setup_agent(
    store: RegistryStore,
    name: str,
    api_key: str,
    deregister: bool = False,
) -> dict:
    """Create an agent, optionally deregister it. Returns create_agent result."""
    result = await store.create_agent(
        name=name, description=f"Test agent {name}", skills=None, api_key=api_key
    )
    if deregister:
        await store.deregister_agent(result["agent_id"])
    return result


async def _create_task(
    task_store: TaskStore,
    from_agent_id: str,
    to_agent_id: str,
    text: str = "Hello",
    msg_type: str = "unicast",
    state: TaskState = TaskState.input_required,
    created_at: str | None = None,
    origin_task_id: str | None = None,
) -> Task:
    """Create and save a task. Returns the saved Task.

    ``created_at`` is forwarded to ``TaskStatus.timestamp``, which TaskStore
    persists as ``status_timestamp`` — the ordering key used by ``list`` and
    ``list_by_sender``. TaskStore assigns its own creation wallclock to the
    ``created_at`` column, so ordering-sensitive tests must set this.
    """
    if created_at is None:
        created_at = datetime.now(UTC).isoformat()

    metadata = {
        "fromAgentId": from_agent_id,
        "toAgentId": to_agent_id,
        "type": msg_type,
    }
    if origin_task_id is not None:
        metadata["originTaskId"] = origin_task_id

    task = Task(
        id=str(uuid.uuid4()),
        context_id=to_agent_id,
        status=TaskStatus(state=state, timestamp=created_at),
        artifacts=[
            Artifact(
                artifact_id=str(uuid.uuid4()),
                parts=[Part(root=TextPart(text=text))],
            )
        ],
        metadata=metadata,
    )
    await task_store.save(task)
    return task


# ---------------------------------------------------------------------------
# Fixture
# ---------------------------------------------------------------------------


@pytest.fixture
async def webui_env(store: RegistryStore, task_store: TaskStore):
    """Set up test FastAPI app with JWT auth and two tenants.

    Yields a dict with:
      - client: httpx.AsyncClient for making requests
      - store: RegistryStore backed by in-memory aiosqlite
      - task_store: TaskStore backed by in-memory aiosqlite
      - executor: BrokerExecutor wired to the stores
      - app: the FastAPI app
      - api_key / tenant_id: primary tenant
      - other_api_key / other_tenant_id: secondary tenant (cross-tenant tests)
    """
    executor = BrokerExecutor(registry_store=store, task_store=task_store)

    app = FastAPI()
    app.include_router(webui_router)

    app.dependency_overrides[get_webui_store] = lambda: store
    app.dependency_overrides[get_webui_task_store] = lambda: task_store
    app.dependency_overrides[get_webui_executor] = lambda: executor

    async def _mock_verify(request=None, cred=None):
        if request is not None:
            request.scope["auth0"] = {"sub": _TEST_SUB}
            request.scope["token"] = _TEST_JWT_TOKEN
        return None

    app.dependency_overrides[verify_auth0_user] = _mock_verify
    app.dependency_overrides[get_user_id] = lambda: _TEST_SUB

    api_key, tenant_id, _ = await store.create_api_key(_TEST_SUB)
    other_api_key, other_tenant_id, _ = await store.create_api_key(_TEST_SUB)

    transport = ASGITransport(app=app)
    async with AsyncClient(transport=transport, base_url="http://test") as client:
        yield {
            "client": client,
            "store": store,
            "task_store": task_store,
            "executor": executor,
            "app": app,
            "api_key": api_key,
            "tenant_id": tenant_id,
            "other_api_key": other_api_key,
            "other_tenant_id": other_tenant_id,
        }


# ===========================================================================
# GET /ui/api/agents
# ===========================================================================


class TestAgentsList:
    """Tests for GET /ui/api/agents — agent list behavior."""

    async def test_returns_active_agents(self, webui_env):
        """GET /agents returns active agents in the tenant."""
        store, client = webui_env["store"], webui_env["client"]
        api_key, tenant_id = webui_env["api_key"], webui_env["tenant_id"]

        agent = await _setup_agent(store, "Active Agent", api_key=api_key)

        resp = await client.get("/ui/api/agents", headers=_auth_header(tenant_id))
        assert resp.status_code == 200

        ids = {a["agent_id"] for a in resp.json()["agents"]}
        assert agent["agent_id"] in ids

    async def test_includes_deregistered_with_messages(self, webui_env):
        """GET /agents includes deregistered agents that still have messages."""
        store, task_store, client = (
            webui_env["store"],
            webui_env["task_store"],
            webui_env["client"],
        )
        api_key, tenant_id = webui_env["api_key"], webui_env["tenant_id"]

        sender = await _setup_agent(store, "Sender", api_key=api_key)
        active = await _setup_agent(store, "Active", api_key=api_key)
        dereg = await _setup_agent(
            store, "Deregistered", api_key=api_key, deregister=False
        )
        await _create_task(
            task_store,
            from_agent_id=sender["agent_id"],
            to_agent_id=dereg["agent_id"],
        )
        await store.deregister_agent(dereg["agent_id"])

        resp = await client.get("/ui/api/agents", headers=_auth_header(tenant_id))
        assert resp.status_code == 200

        ids = {a["agent_id"] for a in resp.json()["agents"]}
        assert active["agent_id"] in ids
        assert dereg["agent_id"] in ids

    async def test_excludes_other_tenant_agents(self, webui_env):
        """GET /agents does not include agents from other tenants."""
        store, client = webui_env["store"], webui_env["client"]
        api_key, tenant_id = webui_env["api_key"], webui_env["tenant_id"]
        other_api_key = webui_env["other_api_key"]

        my_agent = await _setup_agent(store, "My Agent", api_key=api_key)
        other = await _setup_agent(store, "Other Agent", api_key=other_api_key)

        resp = await client.get("/ui/api/agents", headers=_auth_header(tenant_id))
        ids = {a["agent_id"] for a in resp.json()["agents"]}
        assert my_agent["agent_id"] in ids
        assert other["agent_id"] not in ids


# ===========================================================================
# GET /ui/api/agents/{agent_id}/inbox
# ===========================================================================


class TestInbox:
    """Tests for GET /ui/api/agents/{agent_id}/inbox — message formatting."""

    async def test_returns_received_messages(self, webui_env):
        """Inbox returns messages where the agent is the recipient."""
        store, task_store, client = (
            webui_env["store"],
            webui_env["task_store"],
            webui_env["client"],
        )
        api_key, tenant_id = webui_env["api_key"], webui_env["tenant_id"]

        sender = await _setup_agent(store, "Sender", api_key=api_key)
        recipient = await _setup_agent(store, "Recipient", api_key=api_key)

        task = await _create_task(
            task_store,
            from_agent_id=sender["agent_id"],
            to_agent_id=recipient["agent_id"],
            text="Hello, Recipient!",
        )

        resp = await client.get(
            f"/ui/api/agents/{recipient['agent_id']}/inbox",
            headers=_auth_header(tenant_id),
        )
        assert resp.status_code == 200

        data = resp.json()
        assert len(data["messages"]) == 1
        msg = data["messages"][0]
        assert msg["task_id"] == task.id
        assert msg["from_agent_id"] == sender["agent_id"]
        assert msg["to_agent_id"] == recipient["agent_id"]
        assert msg["body"] == "Hello, Recipient!"

    async def test_resolves_agent_names(self, webui_env):
        """Inbox messages include from_agent_name and to_agent_name."""
        store, task_store, client = (
            webui_env["store"],
            webui_env["task_store"],
            webui_env["client"],
        )
        api_key, tenant_id = webui_env["api_key"], webui_env["tenant_id"]

        sender = await _setup_agent(store, "Alice", api_key=api_key)
        recipient = await _setup_agent(store, "Bob", api_key=api_key)

        await _create_task(
            task_store,
            from_agent_id=sender["agent_id"],
            to_agent_id=recipient["agent_id"],
        )

        resp = await client.get(
            f"/ui/api/agents/{recipient['agent_id']}/inbox",
            headers=_auth_header(tenant_id),
        )
        msg = resp.json()["messages"][0]

        assert msg["from_agent_name"] == "Alice"
        assert msg["to_agent_name"] == "Bob"

    async def test_filters_broadcast_summary(self, webui_env):
        """Inbox excludes broadcast_summary type tasks."""
        store, task_store, client = (
            webui_env["store"],
            webui_env["task_store"],
            webui_env["client"],
        )
        api_key, tenant_id = webui_env["api_key"], webui_env["tenant_id"]

        sender = await _setup_agent(store, "Sender", api_key=api_key)
        recipient = await _setup_agent(store, "Recipient", api_key=api_key)

        await _create_task(
            task_store,
            from_agent_id=sender["agent_id"],
            to_agent_id=recipient["agent_id"],
            text="Regular message",
        )
        await _create_task(
            task_store,
            from_agent_id=sender["agent_id"],
            to_agent_id=recipient["agent_id"],
            text="Broadcast sent to 3 recipients",
            msg_type="broadcast_summary",
        )

        resp = await client.get(
            f"/ui/api/agents/{recipient['agent_id']}/inbox",
            headers=_auth_header(tenant_id),
        )
        data = resp.json()
        assert len(data["messages"]) == 1
        assert data["messages"][0]["body"] == "Regular message"

    async def test_newest_first_order(self, webui_env):
        """Inbox returns messages in newest-first (descending) order."""
        store, task_store, client = (
            webui_env["store"],
            webui_env["task_store"],
            webui_env["client"],
        )
        api_key, tenant_id = webui_env["api_key"], webui_env["tenant_id"]

        sender = await _setup_agent(store, "Sender", api_key=api_key)
        recipient = await _setup_agent(store, "Recipient", api_key=api_key)

        now = datetime.now(UTC)
        old_time = (now - timedelta(hours=2)).isoformat()
        new_time = (now - timedelta(hours=1)).isoformat()

        await _create_task(
            task_store,
            from_agent_id=sender["agent_id"],
            to_agent_id=recipient["agent_id"],
            text="Older message",
            created_at=old_time,
        )
        await _create_task(
            task_store,
            from_agent_id=sender["agent_id"],
            to_agent_id=recipient["agent_id"],
            text="Newer message",
            created_at=new_time,
        )

        resp = await client.get(
            f"/ui/api/agents/{recipient['agent_id']}/inbox",
            headers=_auth_header(tenant_id),
        )
        msgs = resp.json()["messages"]
        assert len(msgs) == 2
        assert msgs[0]["body"] == "Newer message"
        assert msgs[1]["body"] == "Older message"

    async def test_empty_inbox_returns_empty_array(self, webui_env):
        """Agent with no inbox messages returns 200 with empty messages array."""
        store, client = webui_env["store"], webui_env["client"]
        api_key, tenant_id = webui_env["api_key"], webui_env["tenant_id"]

        agent = await _setup_agent(store, "Lonely Agent", api_key=api_key)

        resp = await client.get(
            f"/ui/api/agents/{agent['agent_id']}/inbox",
            headers=_auth_header(tenant_id),
        )
        assert resp.status_code == 200
        assert resp.json()["messages"] == []

    async def test_body_empty_when_no_text_part(self, webui_env):
        """Body is empty string when task has no text part in artifacts."""
        store, task_store, client = (
            webui_env["store"],
            webui_env["task_store"],
            webui_env["client"],
        )
        api_key, tenant_id = webui_env["api_key"], webui_env["tenant_id"]

        sender = await _setup_agent(store, "Sender", api_key=api_key)
        recipient = await _setup_agent(store, "Recipient", api_key=api_key)

        now = datetime.now(UTC).isoformat()
        task = Task(
            id=str(uuid.uuid4()),
            context_id=recipient["agent_id"],
            status=TaskStatus(state=TaskState.input_required, timestamp=now),
            artifacts=[],
            metadata={
                "fromAgentId": sender["agent_id"],
                "toAgentId": recipient["agent_id"],
                "type": "unicast",
            },
        )
        await task_store.save(task)

        resp = await client.get(
            f"/ui/api/agents/{recipient['agent_id']}/inbox",
            headers=_auth_header(tenant_id),
        )
        assert resp.json()["messages"][0]["body"] == ""

    async def test_message_has_required_fields(self, webui_env):
        """Each message in inbox has all required fields per spec."""
        store, task_store, client = (
            webui_env["store"],
            webui_env["task_store"],
            webui_env["client"],
        )
        api_key, tenant_id = webui_env["api_key"], webui_env["tenant_id"]

        sender = await _setup_agent(store, "Sender", api_key=api_key)
        recipient = await _setup_agent(store, "Recipient", api_key=api_key)

        await _create_task(
            task_store,
            from_agent_id=sender["agent_id"],
            to_agent_id=recipient["agent_id"],
        )

        resp = await client.get(
            f"/ui/api/agents/{recipient['agent_id']}/inbox",
            headers=_auth_header(tenant_id),
        )
        msg = resp.json()["messages"][0]

        required_fields = [
            "task_id",
            "from_agent_id",
            "from_agent_name",
            "to_agent_id",
            "to_agent_name",
            "type",
            "status",
            "created_at",
            "body",
        ]
        for field in required_fields:
            assert field in msg, f"Missing field: {field}"

    async def test_status_input_required(self, webui_env):
        """Task with input_required state has status='input_required'."""
        store, task_store, client = (
            webui_env["store"],
            webui_env["task_store"],
            webui_env["client"],
        )
        api_key, tenant_id = webui_env["api_key"], webui_env["tenant_id"]

        sender = await _setup_agent(store, "Sender", api_key=api_key)
        recipient = await _setup_agent(store, "Recipient", api_key=api_key)

        await _create_task(
            task_store,
            from_agent_id=sender["agent_id"],
            to_agent_id=recipient["agent_id"],
            state=TaskState.input_required,
        )

        resp = await client.get(
            f"/ui/api/agents/{recipient['agent_id']}/inbox",
            headers=_auth_header(tenant_id),
        )
        assert resp.json()["messages"][0]["status"] == "input_required"

    async def test_status_completed(self, webui_env):
        """Task with completed state has status='completed'."""
        store, task_store, client = (
            webui_env["store"],
            webui_env["task_store"],
            webui_env["client"],
        )
        api_key, tenant_id = webui_env["api_key"], webui_env["tenant_id"]

        sender = await _setup_agent(store, "Sender", api_key=api_key)
        recipient = await _setup_agent(store, "Recipient", api_key=api_key)

        await _create_task(
            task_store,
            from_agent_id=sender["agent_id"],
            to_agent_id=recipient["agent_id"],
            state=TaskState.completed,
        )

        resp = await client.get(
            f"/ui/api/agents/{recipient['agent_id']}/inbox",
            headers=_auth_header(tenant_id),
        )
        assert resp.json()["messages"][0]["status"] == "completed"

    async def test_status_canceled(self, webui_env):
        """Task with canceled state has status='canceled'."""
        store, task_store, client = (
            webui_env["store"],
            webui_env["task_store"],
            webui_env["client"],
        )
        api_key, tenant_id = webui_env["api_key"], webui_env["tenant_id"]

        sender = await _setup_agent(store, "Sender", api_key=api_key)
        recipient = await _setup_agent(store, "Recipient", api_key=api_key)

        await _create_task(
            task_store,
            from_agent_id=sender["agent_id"],
            to_agent_id=recipient["agent_id"],
            state=TaskState.canceled,
        )

        resp = await client.get(
            f"/ui/api/agents/{recipient['agent_id']}/inbox",
            headers=_auth_header(tenant_id),
        )
        assert resp.json()["messages"][0]["status"] == "canceled"

    async def test_message_type_field(self, webui_env):
        """Message type field reflects the task metadata type."""
        store, task_store, client = (
            webui_env["store"],
            webui_env["task_store"],
            webui_env["client"],
        )
        api_key, tenant_id = webui_env["api_key"], webui_env["tenant_id"]

        sender = await _setup_agent(store, "Sender", api_key=api_key)
        recipient = await _setup_agent(store, "Recipient", api_key=api_key)

        await _create_task(
            task_store,
            from_agent_id=sender["agent_id"],
            to_agent_id=recipient["agent_id"],
            msg_type="unicast",
        )

        resp = await client.get(
            f"/ui/api/agents/{recipient['agent_id']}/inbox",
            headers=_auth_header(tenant_id),
        )
        assert resp.json()["messages"][0]["type"] == "unicast"


# ===========================================================================
# GET /ui/api/agents/{agent_id}/sent
# ===========================================================================


class TestSent:
    """Tests for GET /ui/api/agents/{agent_id}/sent — message formatting."""

    async def test_returns_sent_messages(self, webui_env):
        """Sent returns messages where the agent is the sender."""
        store, task_store, client = (
            webui_env["store"],
            webui_env["task_store"],
            webui_env["client"],
        )
        api_key, tenant_id = webui_env["api_key"], webui_env["tenant_id"]

        sender = await _setup_agent(store, "Sender", api_key=api_key)
        recipient = await _setup_agent(store, "Recipient", api_key=api_key)

        task = await _create_task(
            task_store,
            from_agent_id=sender["agent_id"],
            to_agent_id=recipient["agent_id"],
            text="Outgoing message",
        )

        resp = await client.get(
            f"/ui/api/agents/{sender['agent_id']}/sent",
            headers=_auth_header(tenant_id),
        )
        assert resp.status_code == 200

        data = resp.json()
        assert len(data["messages"]) == 1
        msg = data["messages"][0]
        assert msg["task_id"] == task.id
        assert msg["from_agent_id"] == sender["agent_id"]
        assert msg["to_agent_id"] == recipient["agent_id"]
        assert msg["body"] == "Outgoing message"

    async def test_resolves_agent_names(self, webui_env):
        """Sent messages include from_agent_name and to_agent_name."""
        store, task_store, client = (
            webui_env["store"],
            webui_env["task_store"],
            webui_env["client"],
        )
        api_key, tenant_id = webui_env["api_key"], webui_env["tenant_id"]

        sender = await _setup_agent(store, "Alice", api_key=api_key)
        recipient = await _setup_agent(store, "Bob", api_key=api_key)

        await _create_task(
            task_store,
            from_agent_id=sender["agent_id"],
            to_agent_id=recipient["agent_id"],
        )

        resp = await client.get(
            f"/ui/api/agents/{sender['agent_id']}/sent",
            headers=_auth_header(tenant_id),
        )
        msg = resp.json()["messages"][0]

        assert msg["from_agent_name"] == "Alice"
        assert msg["to_agent_name"] == "Bob"

    async def test_filters_broadcast_summary(self, webui_env):
        """Sent excludes broadcast_summary type tasks."""
        store, task_store, client = (
            webui_env["store"],
            webui_env["task_store"],
            webui_env["client"],
        )
        api_key, tenant_id = webui_env["api_key"], webui_env["tenant_id"]

        sender = await _setup_agent(store, "Sender", api_key=api_key)
        recipient = await _setup_agent(store, "Recipient", api_key=api_key)

        await _create_task(
            task_store,
            from_agent_id=sender["agent_id"],
            to_agent_id=recipient["agent_id"],
            text="Regular sent",
        )
        await _create_task(
            task_store,
            from_agent_id=sender["agent_id"],
            to_agent_id=sender["agent_id"],
            text="Broadcast sent to 3 recipients",
            msg_type="broadcast_summary",
        )

        resp = await client.get(
            f"/ui/api/agents/{sender['agent_id']}/sent",
            headers=_auth_header(tenant_id),
        )
        data = resp.json()
        assert len(data["messages"]) == 1
        assert data["messages"][0]["body"] == "Regular sent"

    async def test_newest_first_order(self, webui_env):
        """Sent returns messages sorted by date descending (newest first)."""
        store, task_store, client = (
            webui_env["store"],
            webui_env["task_store"],
            webui_env["client"],
        )
        api_key, tenant_id = webui_env["api_key"], webui_env["tenant_id"]

        sender = await _setup_agent(store, "Sender", api_key=api_key)
        recipient = await _setup_agent(store, "Recipient", api_key=api_key)

        now = datetime.now(UTC)
        old_time = (now - timedelta(hours=2)).isoformat()
        new_time = (now - timedelta(hours=1)).isoformat()

        await _create_task(
            task_store,
            from_agent_id=sender["agent_id"],
            to_agent_id=recipient["agent_id"],
            text="Older",
            created_at=old_time,
        )
        await _create_task(
            task_store,
            from_agent_id=sender["agent_id"],
            to_agent_id=recipient["agent_id"],
            text="Newer",
            created_at=new_time,
        )

        resp = await client.get(
            f"/ui/api/agents/{sender['agent_id']}/sent",
            headers=_auth_header(tenant_id),
        )
        msgs = resp.json()["messages"]
        assert len(msgs) == 2
        assert msgs[0]["body"] == "Newer"
        assert msgs[1]["body"] == "Older"

    async def test_empty_sent_returns_empty_array(self, webui_env):
        """Agent with no sent messages returns 200 with empty messages array."""
        store, client = webui_env["store"], webui_env["client"]
        api_key, tenant_id = webui_env["api_key"], webui_env["tenant_id"]

        agent = await _setup_agent(store, "Silent Agent", api_key=api_key)

        resp = await client.get(
            f"/ui/api/agents/{agent['agent_id']}/sent",
            headers=_auth_header(tenant_id),
        )
        assert resp.status_code == 200
        assert resp.json()["messages"] == []

    async def test_same_response_format_as_inbox(self, webui_env):
        """Sent messages have the same fields as inbox messages."""
        store, task_store, client = (
            webui_env["store"],
            webui_env["task_store"],
            webui_env["client"],
        )
        api_key, tenant_id = webui_env["api_key"], webui_env["tenant_id"]

        sender = await _setup_agent(store, "Sender", api_key=api_key)
        recipient = await _setup_agent(store, "Recipient", api_key=api_key)

        await _create_task(
            task_store,
            from_agent_id=sender["agent_id"],
            to_agent_id=recipient["agent_id"],
        )

        resp = await client.get(
            f"/ui/api/agents/{sender['agent_id']}/sent",
            headers=_auth_header(tenant_id),
        )
        msg = resp.json()["messages"][0]

        required_fields = [
            "task_id",
            "from_agent_id",
            "from_agent_name",
            "to_agent_id",
            "to_agent_name",
            "type",
            "status",
            "created_at",
            "body",
        ]
        for field in required_fields:
            assert field in msg, f"Missing field: {field}"


# ===========================================================================
# POST /ui/api/messages/send
# ===========================================================================


class TestSendMessage:
    """Tests for POST /ui/api/messages/send — send behavior and validation."""

    async def test_successful_unicast(self, webui_env):
        """Successful send returns 200 with task_id and status."""
        store, client = webui_env["store"], webui_env["client"]
        api_key, tenant_id = webui_env["api_key"], webui_env["tenant_id"]

        sender = await _setup_agent(store, "Sender", api_key=api_key)
        recipient = await _setup_agent(store, "Recipient", api_key=api_key)

        resp = await client.post(
            "/ui/api/messages/send",
            json={
                "from_agent_id": sender["agent_id"],
                "to_agent_id": recipient["agent_id"],
                "text": "Hello!",
            },
            headers=_auth_header(tenant_id),
        )
        assert resp.status_code == 200

        data = resp.json()
        assert "task_id" in data
        assert data["status"] == "input_required"

    async def test_cross_tenant_recipient_returns_404(self, webui_env):
        """Sending to an agent in a different tenant returns 404."""
        store, client = webui_env["store"], webui_env["client"]
        api_key, tenant_id = webui_env["api_key"], webui_env["tenant_id"]
        other_api_key = webui_env["other_api_key"]

        sender = await _setup_agent(store, "Sender", api_key=api_key)
        other = await _setup_agent(store, "Other Tenant", api_key=other_api_key)

        resp = await client.post(
            "/ui/api/messages/send",
            json={
                "from_agent_id": sender["agent_id"],
                "to_agent_id": other["agent_id"],
                "text": "Cross-tenant",
            },
            headers=_auth_header(tenant_id),
        )
        assert resp.status_code == 404

    async def test_send_to_deregistered_returns_400(self, webui_env):
        """Sending to a deregistered agent returns 400."""
        store, client = webui_env["store"], webui_env["client"]
        api_key, tenant_id = webui_env["api_key"], webui_env["tenant_id"]

        sender = await _setup_agent(store, "Sender", api_key=api_key)
        dereg = await _setup_agent(
            store, "Deregistered", api_key=api_key, deregister=True
        )

        resp = await client.post(
            "/ui/api/messages/send",
            json={
                "from_agent_id": sender["agent_id"],
                "to_agent_id": dereg["agent_id"],
                "text": "To deregistered",
            },
            headers=_auth_header(tenant_id),
        )
        assert resp.status_code == 400

    async def test_nonexistent_recipient_returns_404(self, webui_env):
        """Sending to a nonexistent agent returns 404."""
        store, client = webui_env["store"], webui_env["client"]
        api_key, tenant_id = webui_env["api_key"], webui_env["tenant_id"]

        sender = await _setup_agent(store, "Sender", api_key=api_key)

        resp = await client.post(
            "/ui/api/messages/send",
            json={
                "from_agent_id": sender["agent_id"],
                "to_agent_id": str(uuid.uuid4()),
                "text": "To nobody",
            },
            headers=_auth_header(tenant_id),
        )
        assert resp.status_code == 404

    async def test_missing_to_agent_id_returns_error(self, webui_env):
        """Missing to_agent_id in request body returns 400 or 422."""
        store, client = webui_env["store"], webui_env["client"]
        api_key, tenant_id = webui_env["api_key"], webui_env["tenant_id"]

        await _setup_agent(store, "Agent", api_key=api_key)

        resp = await client.post(
            "/ui/api/messages/send",
            json={"from_agent_id": "x", "text": "Hello"},
            headers=_auth_header(tenant_id),
        )
        assert resp.status_code in (400, 422)

    async def test_missing_text_returns_error(self, webui_env):
        """Missing text in request body returns 400 or 422."""
        store, client = webui_env["store"], webui_env["client"]
        api_key, tenant_id = webui_env["api_key"], webui_env["tenant_id"]

        await _setup_agent(store, "Agent", api_key=api_key)

        resp = await client.post(
            "/ui/api/messages/send",
            json={"from_agent_id": "x", "to_agent_id": "y"},
            headers=_auth_header(tenant_id),
        )
        assert resp.status_code in (400, 422)

    async def test_missing_from_agent_id_returns_error(self, webui_env):
        """Missing from_agent_id in request body returns 400 or 422."""
        store, client = webui_env["store"], webui_env["client"]
        api_key, tenant_id = webui_env["api_key"], webui_env["tenant_id"]

        await _setup_agent(store, "Agent", api_key=api_key)

        resp = await client.post(
            "/ui/api/messages/send",
            json={"to_agent_id": "x", "text": "Hello"},
            headers=_auth_header(tenant_id),
        )
        assert resp.status_code in (400, 422)

    async def test_from_agent_not_in_tenant_rejected(self, webui_env):
        """Sending from an agent not in the caller's tenant is rejected."""
        store, client = webui_env["store"], webui_env["client"]
        api_key, tenant_id = webui_env["api_key"], webui_env["tenant_id"]
        other_api_key = webui_env["other_api_key"]

        await _setup_agent(store, "My Agent", api_key=api_key)
        other = await _setup_agent(store, "Other Agent", api_key=other_api_key)
        recipient = await _setup_agent(store, "Recipient", api_key=api_key)

        resp = await client.post(
            "/ui/api/messages/send",
            json={
                "from_agent_id": other["agent_id"],
                "to_agent_id": recipient["agent_id"],
                "text": "Impersonation attempt",
            },
            headers=_auth_header(tenant_id),
        )
        assert resp.status_code in (400, 403, 404)

    async def test_message_appears_in_recipient_inbox(self, webui_env):
        """After sending, the message appears in the recipient's inbox."""
        store, client = webui_env["store"], webui_env["client"]
        api_key, tenant_id = webui_env["api_key"], webui_env["tenant_id"]

        sender = await _setup_agent(store, "Sender", api_key=api_key)
        recipient = await _setup_agent(store, "Recipient", api_key=api_key)

        send_resp = await client.post(
            "/ui/api/messages/send",
            json={
                "from_agent_id": sender["agent_id"],
                "to_agent_id": recipient["agent_id"],
                "text": "Test delivery",
            },
            headers=_auth_header(tenant_id),
        )
        assert send_resp.status_code == 200

        inbox_resp = await client.get(
            f"/ui/api/agents/{recipient['agent_id']}/inbox",
            headers=_auth_header(tenant_id),
        )
        msgs = inbox_resp.json()["messages"]
        assert len(msgs) == 1
        assert msgs[0]["body"] == "Test delivery"
        assert msgs[0]["from_agent_id"] == sender["agent_id"]

    async def test_message_appears_in_sender_sent(self, webui_env):
        """After sending, the message appears in the sender's sent list."""
        store, client = webui_env["store"], webui_env["client"]
        api_key, tenant_id = webui_env["api_key"], webui_env["tenant_id"]

        sender = await _setup_agent(store, "Sender", api_key=api_key)
        recipient = await _setup_agent(store, "Recipient", api_key=api_key)

        send_resp = await client.post(
            "/ui/api/messages/send",
            json={
                "from_agent_id": sender["agent_id"],
                "to_agent_id": recipient["agent_id"],
                "text": "Sent test",
            },
            headers=_auth_header(tenant_id),
        )
        assert send_resp.status_code == 200

        sent_resp = await client.get(
            f"/ui/api/agents/{sender['agent_id']}/sent",
            headers=_auth_header(tenant_id),
        )
        msgs = sent_resp.json()["messages"]
        assert len(msgs) == 1
        assert msgs[0]["body"] == "Sent test"
        assert msgs[0]["to_agent_id"] == recipient["agent_id"]


# ===========================================================================
# Cross-tenant isolation for inbox/sent
# ===========================================================================


class TestCrossTenantIsolation:
    """Tests for cross-tenant rejection on inbox and sent endpoints."""

    async def test_inbox_cross_tenant_agent_rejected(self, webui_env):
        """Accessing inbox of an agent in another tenant is rejected."""
        store, client = webui_env["store"], webui_env["client"]
        api_key, tenant_id = webui_env["api_key"], webui_env["tenant_id"]
        other_api_key = webui_env["other_api_key"]

        await _setup_agent(store, "My Agent", api_key=api_key)
        other = await _setup_agent(store, "Other Agent", api_key=other_api_key)

        resp = await client.get(
            f"/ui/api/agents/{other['agent_id']}/inbox",
            headers=_auth_header(tenant_id),
        )
        assert resp.status_code in (403, 404)

    async def test_sent_cross_tenant_agent_rejected(self, webui_env):
        """Accessing sent of an agent in another tenant is rejected."""
        store, client = webui_env["store"], webui_env["client"]
        api_key, tenant_id = webui_env["api_key"], webui_env["tenant_id"]
        other_api_key = webui_env["other_api_key"]

        await _setup_agent(store, "My Agent", api_key=api_key)
        other = await _setup_agent(store, "Other Agent", api_key=other_api_key)

        resp = await client.get(
            f"/ui/api/agents/{other['agent_id']}/sent",
            headers=_auth_header(tenant_id),
        )
        assert resp.status_code in (403, 404)


# ===========================================================================
# Deregistered agent inbox/sent access
# ===========================================================================


class TestDeregisteredAgentAccess:
    """Tests that deregistered agents with messages can still be viewed."""

    async def test_deregistered_agent_inbox_accessible(self, webui_env):
        """Inbox of a deregistered agent (with messages) is still accessible."""
        store, task_store, client = (
            webui_env["store"],
            webui_env["task_store"],
            webui_env["client"],
        )
        api_key, tenant_id = webui_env["api_key"], webui_env["tenant_id"]

        active = await _setup_agent(store, "Active Agent", api_key=api_key)
        dereg = await _setup_agent(
            store, "Deregistered Agent", api_key=api_key, deregister=False
        )

        await _create_task(
            task_store,
            from_agent_id=active["agent_id"],
            to_agent_id=dereg["agent_id"],
            text="Message to deregistered",
        )
        await store.deregister_agent(dereg["agent_id"])

        resp = await client.get(
            f"/ui/api/agents/{dereg['agent_id']}/inbox",
            headers=_auth_header(tenant_id),
        )
        assert resp.status_code == 200

        msgs = resp.json()["messages"]
        assert len(msgs) == 1
        assert msgs[0]["body"] == "Message to deregistered"

    async def test_deregistered_agent_sent_accessible(self, webui_env):
        """Sent messages of a deregistered agent are still accessible."""
        store, task_store, client = (
            webui_env["store"],
            webui_env["task_store"],
            webui_env["client"],
        )
        api_key, tenant_id = webui_env["api_key"], webui_env["tenant_id"]

        active = await _setup_agent(store, "Active Agent", api_key=api_key)
        dereg = await _setup_agent(
            store, "Deregistered Agent", api_key=api_key, deregister=False
        )

        await _create_task(
            task_store,
            from_agent_id=dereg["agent_id"],
            to_agent_id=active["agent_id"],
            text="Sent before deregistration",
        )
        await store.deregister_agent(dereg["agent_id"])

        resp = await client.get(
            f"/ui/api/agents/{dereg['agent_id']}/sent",
            headers=_auth_header(tenant_id),
        )
        assert resp.status_code == 200

        msgs = resp.json()["messages"]
        assert len(msgs) == 1
        assert msgs[0]["body"] == "Sent before deregistration"


# ===========================================================================
# Send from deregistered agent
# ===========================================================================


class TestSendFromDeregistered:
    """Tests that deregistered agents cannot send messages."""

    async def test_send_from_deregistered_agent_rejected(self, webui_env):
        """Sending from a deregistered agent is rejected."""
        store, client = webui_env["store"], webui_env["client"]
        api_key, tenant_id = webui_env["api_key"], webui_env["tenant_id"]

        active = await _setup_agent(store, "Active Agent", api_key=api_key)
        dereg = await _setup_agent(
            store, "Deregistered Sender", api_key=api_key, deregister=True
        )

        resp = await client.post(
            "/ui/api/messages/send",
            json={
                "from_agent_id": dereg["agent_id"],
                "to_agent_id": active["agent_id"],
                "text": "Ghost message",
            },
            headers=_auth_header(tenant_id),
        )
        assert resp.status_code in (400, 403, 404)


# ===========================================================================
# GET /ui/api/timeline
# ===========================================================================


class TestTimeline:
    """Tests for GET /ui/api/timeline — tenant-wide message timeline."""

    async def test_returns_200_with_messages_key(self, webui_env):
        """GET /timeline returns 200 with a ``messages`` array."""
        store, client = webui_env["store"], webui_env["client"]
        api_key, tenant_id = webui_env["api_key"], webui_env["tenant_id"]

        await _setup_agent(store, "Agent", api_key=api_key)

        resp = await client.get(
            "/ui/api/timeline", headers=_auth_header(tenant_id)
        )
        assert resp.status_code == 200
        assert "messages" in resp.json()

    async def test_returns_tenant_tasks_across_agents(self, webui_env):
        """Timeline includes tasks addressed to different agents in the tenant."""
        store, task_store, client = (
            webui_env["store"],
            webui_env["task_store"],
            webui_env["client"],
        )
        api_key, tenant_id = webui_env["api_key"], webui_env["tenant_id"]

        sender = await _setup_agent(store, "Sender", api_key=api_key)
        r1 = await _setup_agent(store, "Recipient 1", api_key=api_key)
        r2 = await _setup_agent(store, "Recipient 2", api_key=api_key)

        t1 = await _create_task(
            task_store,
            from_agent_id=sender["agent_id"],
            to_agent_id=r1["agent_id"],
            text="Message to R1",
        )
        t2 = await _create_task(
            task_store,
            from_agent_id=sender["agent_id"],
            to_agent_id=r2["agent_id"],
            text="Message to R2",
        )

        resp = await client.get(
            "/ui/api/timeline", headers=_auth_header(tenant_id)
        )
        task_ids = {m["task_id"] for m in resp.json()["messages"]}
        assert t1.id in task_ids
        assert t2.id in task_ids

    async def test_excludes_broadcast_summary(self, webui_env):
        """Timeline filters out ``broadcast_summary`` type tasks."""
        store, task_store, client = (
            webui_env["store"],
            webui_env["task_store"],
            webui_env["client"],
        )
        api_key, tenant_id = webui_env["api_key"], webui_env["tenant_id"]

        sender = await _setup_agent(store, "Sender", api_key=api_key)
        recipient = await _setup_agent(store, "Recipient", api_key=api_key)

        regular = await _create_task(
            task_store,
            from_agent_id=sender["agent_id"],
            to_agent_id=recipient["agent_id"],
            text="Regular message",
        )
        await _create_task(
            task_store,
            from_agent_id=sender["agent_id"],
            to_agent_id=sender["agent_id"],
            text="Broadcast sent to 2 recipients",
            msg_type="broadcast_summary",
        )

        resp = await client.get(
            "/ui/api/timeline", headers=_auth_header(tenant_id)
        )
        msgs = resp.json()["messages"]
        assert len(msgs) == 1
        assert msgs[0]["task_id"] == regular.id

    async def test_ordered_by_status_timestamp_desc(self, webui_env):
        """Timeline returns messages newest-first by status_timestamp."""
        store, task_store, client = (
            webui_env["store"],
            webui_env["task_store"],
            webui_env["client"],
        )
        api_key, tenant_id = webui_env["api_key"], webui_env["tenant_id"]

        sender = await _setup_agent(store, "Sender", api_key=api_key)
        recipient = await _setup_agent(store, "Recipient", api_key=api_key)

        now = datetime.now(UTC)
        old_time = (now - timedelta(hours=2)).isoformat()
        new_time = (now - timedelta(hours=1)).isoformat()

        await _create_task(
            task_store,
            from_agent_id=sender["agent_id"],
            to_agent_id=recipient["agent_id"],
            text="Older message",
            created_at=old_time,
        )
        await _create_task(
            task_store,
            from_agent_id=sender["agent_id"],
            to_agent_id=recipient["agent_id"],
            text="Newer message",
            created_at=new_time,
        )

        resp = await client.get(
            "/ui/api/timeline", headers=_auth_header(tenant_id)
        )
        msgs = resp.json()["messages"]
        assert len(msgs) == 2
        assert msgs[0]["body"] == "Newer message"
        assert msgs[1]["body"] == "Older message"

    async def test_cross_tenant_isolation(self, webui_env):
        """Timeline only shows tasks from the requested tenant."""
        store, task_store, client = (
            webui_env["store"],
            webui_env["task_store"],
            webui_env["client"],
        )
        api_key, tenant_id = webui_env["api_key"], webui_env["tenant_id"]
        other_api_key, _other_tenant_id = (
            webui_env["other_api_key"],
            webui_env["other_tenant_id"],
        )

        my_sender = await _setup_agent(store, "My Sender", api_key=api_key)
        my_recip = await _setup_agent(store, "My Recipient", api_key=api_key)
        other_sender = await _setup_agent(
            store, "Other Sender", api_key=other_api_key
        )
        other_recip = await _setup_agent(
            store, "Other Recipient", api_key=other_api_key
        )

        my_task = await _create_task(
            task_store,
            from_agent_id=my_sender["agent_id"],
            to_agent_id=my_recip["agent_id"],
            text="My tenant message",
        )
        other_task = await _create_task(
            task_store,
            from_agent_id=other_sender["agent_id"],
            to_agent_id=other_recip["agent_id"],
            text="Other tenant message",
        )

        resp = await client.get(
            "/ui/api/timeline", headers=_auth_header(tenant_id)
        )
        task_ids = {m["task_id"] for m in resp.json()["messages"]}
        assert my_task.id in task_ids
        assert other_task.id not in task_ids

    async def test_each_row_has_origin_task_id_field(self, webui_env):
        """Each timeline row includes ``origin_task_id`` (null for unicast)."""
        store, task_store, client = (
            webui_env["store"],
            webui_env["task_store"],
            webui_env["client"],
        )
        api_key, tenant_id = webui_env["api_key"], webui_env["tenant_id"]

        sender = await _setup_agent(store, "Sender", api_key=api_key)
        recipient = await _setup_agent(store, "Recipient", api_key=api_key)

        await _create_task(
            task_store,
            from_agent_id=sender["agent_id"],
            to_agent_id=recipient["agent_id"],
        )

        resp = await client.get(
            "/ui/api/timeline", headers=_auth_header(tenant_id)
        )
        msg = resp.json()["messages"][0]
        assert "origin_task_id" in msg
        assert msg["origin_task_id"] is None

    async def test_origin_task_id_populated_for_broadcast_row(self, webui_env):
        """A broadcast delivery row carries its ``origin_task_id`` value."""
        store, task_store, client = (
            webui_env["store"],
            webui_env["task_store"],
            webui_env["client"],
        )
        api_key, tenant_id = webui_env["api_key"], webui_env["tenant_id"]

        sender = await _setup_agent(store, "Sender", api_key=api_key)
        recipient = await _setup_agent(store, "Recipient", api_key=api_key)

        origin_id = str(uuid.uuid4())
        await _create_task(
            task_store,
            from_agent_id=sender["agent_id"],
            to_agent_id=recipient["agent_id"],
            text="Broadcast delivery",
            origin_task_id=origin_id,
        )

        resp = await client.get(
            "/ui/api/timeline", headers=_auth_header(tenant_id)
        )
        msg = resp.json()["messages"][0]
        assert msg["origin_task_id"] == origin_id

    async def test_each_row_has_status_timestamp_field(self, webui_env):
        """Each timeline row includes ``status_timestamp``."""
        store, task_store, client = (
            webui_env["store"],
            webui_env["task_store"],
            webui_env["client"],
        )
        api_key, tenant_id = webui_env["api_key"], webui_env["tenant_id"]

        sender = await _setup_agent(store, "Sender", api_key=api_key)
        recipient = await _setup_agent(store, "Recipient", api_key=api_key)

        await _create_task(
            task_store,
            from_agent_id=sender["agent_id"],
            to_agent_id=recipient["agent_id"],
        )

        resp = await client.get(
            "/ui/api/timeline", headers=_auth_header(tenant_id)
        )
        msg = resp.json()["messages"][0]
        assert "status_timestamp" in msg
        assert msg["status_timestamp"] != ""

    async def test_empty_tenant_returns_empty_list(self, webui_env):
        """Tenant with no tasks returns 200 with empty messages."""
        client = webui_env["client"]
        tenant_id = webui_env["tenant_id"]

        resp = await client.get(
            "/ui/api/timeline", headers=_auth_header(tenant_id)
        )
        assert resp.status_code == 200
        assert resp.json()["messages"] == []

    async def test_multi_agent_tasks_all_visible(self, webui_env):
        """Tasks addressed to 3 different agents all appear in timeline."""
        store, task_store, client = (
            webui_env["store"],
            webui_env["task_store"],
            webui_env["client"],
        )
        api_key, tenant_id = webui_env["api_key"], webui_env["tenant_id"]

        sender = await _setup_agent(store, "Sender", api_key=api_key)
        r1 = await _setup_agent(store, "R1", api_key=api_key)
        r2 = await _setup_agent(store, "R2", api_key=api_key)
        r3 = await _setup_agent(store, "R3", api_key=api_key)

        tasks = []
        for recipient in [r1, r2, r3]:
            t = await _create_task(
                task_store,
                from_agent_id=sender["agent_id"],
                to_agent_id=recipient["agent_id"],
                text=f"To {recipient['name']}",
            )
            tasks.append(t)

        resp = await client.get(
            "/ui/api/timeline", headers=_auth_header(tenant_id)
        )
        task_ids = {m["task_id"] for m in resp.json()["messages"]}
        for t in tasks:
            assert t.id in task_ids

    async def test_200_row_cap(self, webui_env):
        """Timeline returns at most 200 rows even when more tasks exist."""
        store, task_store, client = (
            webui_env["store"],
            webui_env["task_store"],
            webui_env["client"],
        )
        api_key, tenant_id = webui_env["api_key"], webui_env["tenant_id"]

        sender = await _setup_agent(store, "Sender", api_key=api_key)
        recipient = await _setup_agent(store, "Recipient", api_key=api_key)

        now = datetime.now(UTC)
        for i in range(205):
            ts = (now - timedelta(seconds=205 - i)).isoformat()
            await _create_task(
                task_store,
                from_agent_id=sender["agent_id"],
                to_agent_id=recipient["agent_id"],
                text=f"Message {i}",
                created_at=ts,
            )

        resp = await client.get(
            "/ui/api/timeline", headers=_auth_header(tenant_id)
        )
        assert resp.status_code == 200
        assert len(resp.json()["messages"]) == 200


# ===========================================================================
# Existing endpoints: new fields from _format_messages
# ===========================================================================


class TestFormatMessagesNewFields:
    """Verify inbox/sent endpoints now include origin_task_id and status_timestamp."""

    async def test_inbox_has_origin_task_id_field(self, webui_env):
        """Inbox messages include ``origin_task_id`` field."""
        store, task_store, client = (
            webui_env["store"],
            webui_env["task_store"],
            webui_env["client"],
        )
        api_key, tenant_id = webui_env["api_key"], webui_env["tenant_id"]

        sender = await _setup_agent(store, "Sender", api_key=api_key)
        recipient = await _setup_agent(store, "Recipient", api_key=api_key)

        await _create_task(
            task_store,
            from_agent_id=sender["agent_id"],
            to_agent_id=recipient["agent_id"],
        )

        resp = await client.get(
            f"/ui/api/agents/{recipient['agent_id']}/inbox",
            headers=_auth_header(tenant_id),
        )
        msg = resp.json()["messages"][0]
        assert "origin_task_id" in msg

    async def test_inbox_has_status_timestamp_field(self, webui_env):
        """Inbox messages include ``status_timestamp`` field."""
        store, task_store, client = (
            webui_env["store"],
            webui_env["task_store"],
            webui_env["client"],
        )
        api_key, tenant_id = webui_env["api_key"], webui_env["tenant_id"]

        sender = await _setup_agent(store, "Sender", api_key=api_key)
        recipient = await _setup_agent(store, "Recipient", api_key=api_key)

        await _create_task(
            task_store,
            from_agent_id=sender["agent_id"],
            to_agent_id=recipient["agent_id"],
        )

        resp = await client.get(
            f"/ui/api/agents/{recipient['agent_id']}/inbox",
            headers=_auth_header(tenant_id),
        )
        msg = resp.json()["messages"][0]
        assert "status_timestamp" in msg
        assert msg["status_timestamp"] != ""

    async def test_sent_has_origin_task_id_field(self, webui_env):
        """Sent messages include ``origin_task_id`` field."""
        store, task_store, client = (
            webui_env["store"],
            webui_env["task_store"],
            webui_env["client"],
        )
        api_key, tenant_id = webui_env["api_key"], webui_env["tenant_id"]

        sender = await _setup_agent(store, "Sender", api_key=api_key)
        recipient = await _setup_agent(store, "Recipient", api_key=api_key)

        await _create_task(
            task_store,
            from_agent_id=sender["agent_id"],
            to_agent_id=recipient["agent_id"],
        )

        resp = await client.get(
            f"/ui/api/agents/{sender['agent_id']}/sent",
            headers=_auth_header(tenant_id),
        )
        msg = resp.json()["messages"][0]
        assert "origin_task_id" in msg

    async def test_sent_has_status_timestamp_field(self, webui_env):
        """Sent messages include ``status_timestamp`` field."""
        store, task_store, client = (
            webui_env["store"],
            webui_env["task_store"],
            webui_env["client"],
        )
        api_key, tenant_id = webui_env["api_key"], webui_env["tenant_id"]

        sender = await _setup_agent(store, "Sender", api_key=api_key)
        recipient = await _setup_agent(store, "Recipient", api_key=api_key)

        await _create_task(
            task_store,
            from_agent_id=sender["agent_id"],
            to_agent_id=recipient["agent_id"],
        )

        resp = await client.get(
            f"/ui/api/agents/{sender['agent_id']}/sent",
            headers=_auth_header(tenant_id),
        )
        msg = resp.json()["messages"][0]
        assert "status_timestamp" in msg
        assert msg["status_timestamp"] != ""


# ===========================================================================
# POST /ui/api/messages/send — broadcast (to_agent_id="*")
# ===========================================================================


class TestSendMessageBroadcast:
    """Tests for POST /ui/api/messages/send with ``to_agent_id="*"``."""

    async def test_broadcast_returns_200_with_task_id(self, webui_env):
        """Broadcast send returns 200 with task_id and completed status."""
        store, client = webui_env["store"], webui_env["client"]
        api_key, tenant_id = webui_env["api_key"], webui_env["tenant_id"]

        sender = await _setup_agent(store, "Sender", api_key=api_key)
        await _setup_agent(store, "R1", api_key=api_key)
        await _setup_agent(store, "R2", api_key=api_key)

        resp = await client.post(
            "/ui/api/messages/send",
            json={
                "from_agent_id": sender["agent_id"],
                "to_agent_id": "*",
                "text": "Broadcast message",
            },
            headers=_auth_header(tenant_id),
        )
        assert resp.status_code == 200
        data = resp.json()
        assert "task_id" in data
        assert data["status"] == "completed"

    async def test_broadcast_creates_delivery_tasks_in_inboxes(self, webui_env):
        """Broadcast creates delivery tasks in each recipient's inbox."""
        store, client = webui_env["store"], webui_env["client"]
        api_key, tenant_id = webui_env["api_key"], webui_env["tenant_id"]

        sender = await _setup_agent(store, "Sender", api_key=api_key)
        r1 = await _setup_agent(store, "R1", api_key=api_key)
        r2 = await _setup_agent(store, "R2", api_key=api_key)

        await client.post(
            "/ui/api/messages/send",
            json={
                "from_agent_id": sender["agent_id"],
                "to_agent_id": "*",
                "text": "Build failed",
            },
            headers=_auth_header(tenant_id),
        )

        r1_resp = await client.get(
            f"/ui/api/agents/{r1['agent_id']}/inbox",
            headers=_auth_header(tenant_id),
        )
        r2_resp = await client.get(
            f"/ui/api/agents/{r2['agent_id']}/inbox",
            headers=_auth_header(tenant_id),
        )

        assert len(r1_resp.json()["messages"]) == 1
        assert r1_resp.json()["messages"][0]["body"] == "Build failed"
        assert len(r2_resp.json()["messages"]) == 1
        assert r2_resp.json()["messages"][0]["body"] == "Build failed"

    async def test_broadcast_excludes_sender_from_recipients(self, webui_env):
        """Sender does not receive their own broadcast in inbox."""
        store, client = webui_env["store"], webui_env["client"]
        api_key, tenant_id = webui_env["api_key"], webui_env["tenant_id"]

        sender = await _setup_agent(store, "Sender", api_key=api_key)
        await _setup_agent(store, "Recipient", api_key=api_key)

        await client.post(
            "/ui/api/messages/send",
            json={
                "from_agent_id": sender["agent_id"],
                "to_agent_id": "*",
                "text": "Broadcast",
            },
            headers=_auth_header(tenant_id),
        )

        sender_inbox = await client.get(
            f"/ui/api/agents/{sender['agent_id']}/inbox",
            headers=_auth_header(tenant_id),
        )
        assert len(sender_inbox.json()["messages"]) == 0

    async def test_broadcast_from_deregistered_sender_rejected(self, webui_env):
        """Broadcast from a deregistered sender is rejected."""
        store, client = webui_env["store"], webui_env["client"]
        api_key, tenant_id = webui_env["api_key"], webui_env["tenant_id"]

        await _setup_agent(store, "Active", api_key=api_key)
        dereg = await _setup_agent(
            store, "Deregistered", api_key=api_key, deregister=True
        )

        resp = await client.post(
            "/ui/api/messages/send",
            json={
                "from_agent_id": dereg["agent_id"],
                "to_agent_id": "*",
                "text": "Ghost broadcast",
            },
            headers=_auth_header(tenant_id),
        )
        assert resp.status_code in (400, 403, 404)

    async def test_broadcast_from_other_tenant_sender_rejected(self, webui_env):
        """Broadcast with a sender from another tenant is rejected."""
        store, client = webui_env["store"], webui_env["client"]
        api_key, tenant_id = webui_env["api_key"], webui_env["tenant_id"]
        other_api_key = webui_env["other_api_key"]

        other_sender = await _setup_agent(
            store, "Other Sender", api_key=other_api_key
        )
        await _setup_agent(store, "My Agent", api_key=api_key)

        resp = await client.post(
            "/ui/api/messages/send",
            json={
                "from_agent_id": other_sender["agent_id"],
                "to_agent_id": "*",
                "text": "Cross-tenant broadcast",
            },
            headers=_auth_header(tenant_id),
        )
        assert resp.status_code in (400, 403, 404)

    async def test_broadcast_appears_in_timeline(self, webui_env):
        """Broadcast delivery tasks appear in the timeline endpoint."""
        store, client = webui_env["store"], webui_env["client"]
        api_key, tenant_id = webui_env["api_key"], webui_env["tenant_id"]

        sender = await _setup_agent(store, "Sender", api_key=api_key)
        r1 = await _setup_agent(store, "R1", api_key=api_key)
        r2 = await _setup_agent(store, "R2", api_key=api_key)

        await client.post(
            "/ui/api/messages/send",
            json={
                "from_agent_id": sender["agent_id"],
                "to_agent_id": "*",
                "text": "Timeline broadcast",
            },
            headers=_auth_header(tenant_id),
        )

        resp = await client.get(
            "/ui/api/timeline", headers=_auth_header(tenant_id)
        )
        msgs = resp.json()["messages"]
        bodies = [m["body"] for m in msgs]
        assert "Timeline broadcast" in bodies
        broadcast_msgs = [m for m in msgs if m["body"] == "Timeline broadcast"]
        assert len(broadcast_msgs) == 2
        recipients = {m["to_agent_id"] for m in broadcast_msgs}
        assert r1["agent_id"] in recipients
        assert r2["agent_id"] in recipients
