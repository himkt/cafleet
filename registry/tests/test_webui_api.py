"""Tests for webui_api.py — WebUI API endpoint behavior.

Design doc 0000015 Step 6 rewrites the WebUI API:
  - Delete ``GET /ui/api/auth/config``, ``POST /ui/api/keys``, ``GET /ui/api/keys``,
    ``DELETE /ui/api/keys/{id}``
  - Delete ``get_webui_tenant``; add ``get_webui_session`` reading ``X-Session-Id``
  - Add ``GET /ui/api/sessions`` (no session header required)
  - Update ``/ui/api/agents``, inbox, sent, messages/send to use ``get_webui_session``
  - Rename ``_get_tenant_agents`` → ``_get_session_agents``

No Auth0, no JWT, no bearer tokens, no api_keys.
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
from sqlalchemy.ext.asyncio import AsyncSession, async_sessionmaker

from hikyaku_registry.db.models import Session
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
                Session(session_id=session_id, label=label, created_at=created_at)
            )
    return session_id


def _session_header(session_id: str) -> dict:
    """Build X-Session-Id header."""
    return {"X-Session-Id": session_id}


async def _setup_agent(
    store: RegistryStore,
    name: str,
    session_id: str,
    deregister: bool = False,
) -> dict:
    """Create an agent, optionally deregister it. Returns create_agent result."""
    result = await store.create_agent(
        name=name, description=f"Test agent {name}", skills=None, session_id=session_id
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
    """Create and save a task. Returns the saved Task."""
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
async def webui_env(store: RegistryStore, task_store: TaskStore, db_sessionmaker):
    """Set up test FastAPI app with session-based auth.

    Yields a dict with:
      - client: httpx.AsyncClient for making requests
      - store: RegistryStore backed by in-memory aiosqlite
      - task_store: TaskStore backed by in-memory aiosqlite
      - executor: BrokerExecutor wired to the stores
      - app: the FastAPI app
      - session_id: primary session
      - other_session_id: secondary session (cross-session tests)
    """
    executor = BrokerExecutor(registry_store=store, task_store=task_store)

    app = FastAPI()
    app.include_router(webui_router)

    app.dependency_overrides[get_webui_store] = lambda: store
    app.dependency_overrides[get_webui_task_store] = lambda: task_store
    app.dependency_overrides[get_webui_executor] = lambda: executor

    session_id = await _create_test_session(db_sessionmaker, label="primary")
    other_session_id = await _create_test_session(db_sessionmaker, label="other")

    transport = ASGITransport(app=app)
    async with AsyncClient(transport=transport, base_url="http://test") as client:
        yield {
            "client": client,
            "store": store,
            "task_store": task_store,
            "executor": executor,
            "app": app,
            "session_id": session_id,
            "other_session_id": other_session_id,
            "db_sessionmaker": db_sessionmaker,
        }


# ===========================================================================
# GET /ui/api/sessions (new — no session header required)
# ===========================================================================


class TestListSessions:
    """Tests for GET /ui/api/sessions.

    Returns all sessions with agent_count. No session header required.
    """

    async def test_returns_200(self, webui_env):
        """GET /ui/api/sessions returns 200."""
        client = webui_env["client"]
        resp = await client.get("/ui/api/sessions")
        assert resp.status_code == 200

    async def test_returns_seeded_sessions(self, webui_env):
        """Returns sessions seeded by the fixture."""
        client, session_id = webui_env["client"], webui_env["session_id"]
        resp = await client.get("/ui/api/sessions")
        data = resp.json()
        ids = {s["session_id"] for s in data}
        assert session_id in ids

    async def test_no_auth_required(self, webui_env):
        """Endpoint succeeds without any auth headers."""
        client = webui_env["client"]
        resp = await client.get("/ui/api/sessions")
        assert resp.status_code == 200

    async def test_includes_agent_count(self, webui_env):
        """Each session has an agent_count field."""
        client, store, session_id = (
            webui_env["client"],
            webui_env["store"],
            webui_env["session_id"],
        )
        await _setup_agent(store, "A", session_id)
        await _setup_agent(store, "B", session_id)

        resp = await client.get("/ui/api/sessions")
        match = [s for s in resp.json() if s["session_id"] == session_id]
        assert len(match) == 1
        assert match[0]["agent_count"] == 2


# ===========================================================================
# GET /ui/api/agents (X-Session-Id)
# ===========================================================================


class TestAgentsList:
    """Tests for GET /ui/api/agents — agent list behavior."""

    async def test_returns_active_agents(self, webui_env):
        """GET /agents returns active agents in the session."""
        store, client, session_id = (
            webui_env["store"],
            webui_env["client"],
            webui_env["session_id"],
        )
        agent = await _setup_agent(store, "Active Agent", session_id)

        resp = await client.get(
            "/ui/api/agents", headers=_session_header(session_id)
        )
        assert resp.status_code == 200
        ids = {a["agent_id"] for a in resp.json()["agents"]}
        assert agent["agent_id"] in ids

    async def test_includes_deregistered_with_messages(self, webui_env):
        """GET /agents includes deregistered agents that still have messages."""
        store, task_store, client, session_id = (
            webui_env["store"],
            webui_env["task_store"],
            webui_env["client"],
            webui_env["session_id"],
        )
        sender = await _setup_agent(store, "Sender", session_id)
        active = await _setup_agent(store, "Active", session_id)
        dereg = await _setup_agent(store, "Deregistered", session_id)
        await _create_task(
            task_store,
            from_agent_id=sender["agent_id"],
            to_agent_id=dereg["agent_id"],
        )
        await store.deregister_agent(dereg["agent_id"])

        resp = await client.get(
            "/ui/api/agents", headers=_session_header(session_id)
        )
        assert resp.status_code == 200
        ids = {a["agent_id"] for a in resp.json()["agents"]}
        assert active["agent_id"] in ids
        assert dereg["agent_id"] in ids

    async def test_excludes_other_session_agents(self, webui_env):
        """GET /agents does not include agents from other sessions."""
        store, client, session_id, other = (
            webui_env["store"],
            webui_env["client"],
            webui_env["session_id"],
            webui_env["other_session_id"],
        )
        my_agent = await _setup_agent(store, "My Agent", session_id)
        other_agent = await _setup_agent(store, "Other Agent", other)

        resp = await client.get(
            "/ui/api/agents", headers=_session_header(session_id)
        )
        ids = {a["agent_id"] for a in resp.json()["agents"]}
        assert my_agent["agent_id"] in ids
        assert other_agent["agent_id"] not in ids

    async def test_missing_session_header_returns_400(self, webui_env):
        """GET /agents without X-Session-Id returns 400."""
        client = webui_env["client"]
        resp = await client.get("/ui/api/agents")
        assert resp.status_code == 400

    async def test_unknown_session_returns_404(self, webui_env):
        """GET /agents with unknown X-Session-Id returns 404."""
        client = webui_env["client"]
        resp = await client.get(
            "/ui/api/agents",
            headers=_session_header("nonexistent-session-id"),
        )
        assert resp.status_code == 404


# ===========================================================================
# GET /ui/api/agents/{agent_id}/inbox (X-Session-Id)
# ===========================================================================


class TestInbox:
    """Tests for GET /ui/api/agents/{agent_id}/inbox — message formatting."""

    async def test_returns_received_messages(self, webui_env):
        """Inbox returns messages where the agent is the recipient."""
        store, task_store, client, session_id = (
            webui_env["store"],
            webui_env["task_store"],
            webui_env["client"],
            webui_env["session_id"],
        )
        sender = await _setup_agent(store, "Sender", session_id)
        recipient = await _setup_agent(store, "Recipient", session_id)

        task = await _create_task(
            task_store,
            from_agent_id=sender["agent_id"],
            to_agent_id=recipient["agent_id"],
            text="Hello, Recipient!",
        )

        resp = await client.get(
            f"/ui/api/agents/{recipient['agent_id']}/inbox",
            headers=_session_header(session_id),
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
        store, task_store, client, session_id = (
            webui_env["store"],
            webui_env["task_store"],
            webui_env["client"],
            webui_env["session_id"],
        )
        sender = await _setup_agent(store, "Alice", session_id)
        recipient = await _setup_agent(store, "Bob", session_id)

        await _create_task(
            task_store,
            from_agent_id=sender["agent_id"],
            to_agent_id=recipient["agent_id"],
        )

        resp = await client.get(
            f"/ui/api/agents/{recipient['agent_id']}/inbox",
            headers=_session_header(session_id),
        )
        msg = resp.json()["messages"][0]
        assert msg["from_agent_name"] == "Alice"
        assert msg["to_agent_name"] == "Bob"

    async def test_filters_broadcast_summary(self, webui_env):
        """Inbox excludes broadcast_summary type tasks."""
        store, task_store, client, session_id = (
            webui_env["store"],
            webui_env["task_store"],
            webui_env["client"],
            webui_env["session_id"],
        )
        sender = await _setup_agent(store, "Sender", session_id)
        recipient = await _setup_agent(store, "Recipient", session_id)

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
            headers=_session_header(session_id),
        )
        data = resp.json()
        assert len(data["messages"]) == 1
        assert data["messages"][0]["body"] == "Regular message"

    async def test_empty_inbox_returns_empty_array(self, webui_env):
        """Agent with no inbox messages returns 200 with empty messages array."""
        store, client, session_id = (
            webui_env["store"],
            webui_env["client"],
            webui_env["session_id"],
        )
        agent = await _setup_agent(store, "Lonely Agent", session_id)

        resp = await client.get(
            f"/ui/api/agents/{agent['agent_id']}/inbox",
            headers=_session_header(session_id),
        )
        assert resp.status_code == 200
        assert resp.json()["messages"] == []

    async def test_agent_not_in_session_returns_404(self, webui_env):
        """Agent belonging to a different session returns 404."""
        store, client, session_id, other = (
            webui_env["store"],
            webui_env["client"],
            webui_env["session_id"],
            webui_env["other_session_id"],
        )
        other_agent = await _setup_agent(store, "Other", other)

        resp = await client.get(
            f"/ui/api/agents/{other_agent['agent_id']}/inbox",
            headers=_session_header(session_id),
        )
        assert resp.status_code == 404

    async def test_missing_session_header_returns_400(self, webui_env):
        """Inbox without X-Session-Id returns 400."""
        store, client, session_id = (
            webui_env["store"],
            webui_env["client"],
            webui_env["session_id"],
        )
        agent = await _setup_agent(store, "Test", session_id)

        resp = await client.get(f"/ui/api/agents/{agent['agent_id']}/inbox")
        assert resp.status_code == 400

    async def test_message_has_required_fields(self, webui_env):
        """Each message in inbox has all required fields per spec."""
        store, task_store, client, session_id = (
            webui_env["store"],
            webui_env["task_store"],
            webui_env["client"],
            webui_env["session_id"],
        )
        sender = await _setup_agent(store, "Sender", session_id)
        recipient = await _setup_agent(store, "Recipient", session_id)

        await _create_task(
            task_store,
            from_agent_id=sender["agent_id"],
            to_agent_id=recipient["agent_id"],
        )

        resp = await client.get(
            f"/ui/api/agents/{recipient['agent_id']}/inbox",
            headers=_session_header(session_id),
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
# GET /ui/api/agents/{agent_id}/sent (X-Session-Id)
# ===========================================================================


class TestSent:
    """Tests for GET /ui/api/agents/{agent_id}/sent — sent message listing."""

    async def test_returns_sent_messages(self, webui_env):
        """Sent returns messages where the agent is the sender."""
        store, task_store, client, session_id = (
            webui_env["store"],
            webui_env["task_store"],
            webui_env["client"],
            webui_env["session_id"],
        )
        sender = await _setup_agent(store, "Sender", session_id)
        recipient = await _setup_agent(store, "Recipient", session_id)

        task = await _create_task(
            task_store,
            from_agent_id=sender["agent_id"],
            to_agent_id=recipient["agent_id"],
            text="Sent message",
        )

        resp = await client.get(
            f"/ui/api/agents/{sender['agent_id']}/sent",
            headers=_session_header(session_id),
        )
        assert resp.status_code == 200
        msgs = resp.json()["messages"]
        assert len(msgs) == 1
        assert msgs[0]["body"] == "Sent message"

    async def test_empty_sent_returns_empty_array(self, webui_env):
        """Agent with no sent messages returns 200 with empty messages array."""
        store, client, session_id = (
            webui_env["store"],
            webui_env["client"],
            webui_env["session_id"],
        )
        agent = await _setup_agent(store, "Quiet Agent", session_id)

        resp = await client.get(
            f"/ui/api/agents/{agent['agent_id']}/sent",
            headers=_session_header(session_id),
        )
        assert resp.status_code == 200
        assert resp.json()["messages"] == []

    async def test_agent_not_in_session_returns_404(self, webui_env):
        """Agent belonging to a different session returns 404."""
        store, client, session_id, other = (
            webui_env["store"],
            webui_env["client"],
            webui_env["session_id"],
            webui_env["other_session_id"],
        )
        other_agent = await _setup_agent(store, "Other", other)

        resp = await client.get(
            f"/ui/api/agents/{other_agent['agent_id']}/sent",
            headers=_session_header(session_id),
        )
        assert resp.status_code == 404

    async def test_missing_session_header_returns_400(self, webui_env):
        """Sent without X-Session-Id returns 400."""
        store, client, session_id = (
            webui_env["store"],
            webui_env["client"],
            webui_env["session_id"],
        )
        agent = await _setup_agent(store, "Test", session_id)

        resp = await client.get(f"/ui/api/agents/{agent['agent_id']}/sent")
        assert resp.status_code == 400


# ===========================================================================
# POST /ui/api/messages/send (X-Session-Id)
# ===========================================================================


class TestSendMessage:
    """Tests for POST /ui/api/messages/send — WebUI message send."""

    async def test_send_unicast_succeeds(self, webui_env):
        """Unicast send via WebUI returns 200 with task_id."""
        store, client, session_id = (
            webui_env["store"],
            webui_env["client"],
            webui_env["session_id"],
        )
        sender = await _setup_agent(store, "Sender", session_id)
        recipient = await _setup_agent(store, "Recipient", session_id)

        resp = await client.post(
            "/ui/api/messages/send",
            headers=_session_header(session_id),
            json={
                "from_agent_id": sender["agent_id"],
                "to_agent_id": recipient["agent_id"],
                "text": "Hello from WebUI",
            },
        )
        assert resp.status_code == 200
        data = resp.json()
        assert "task_id" in data

    async def test_missing_session_header_returns_400(self, webui_env):
        """Send without X-Session-Id returns 400."""
        store, client, session_id = (
            webui_env["store"],
            webui_env["client"],
            webui_env["session_id"],
        )
        sender = await _setup_agent(store, "Sender", session_id)
        recipient = await _setup_agent(store, "Recipient", session_id)

        resp = await client.post(
            "/ui/api/messages/send",
            json={
                "from_agent_id": sender["agent_id"],
                "to_agent_id": recipient["agent_id"],
                "text": "Hello",
            },
        )
        assert resp.status_code == 400

    async def test_unknown_session_returns_404(self, webui_env):
        """Send with unknown X-Session-Id returns 404."""
        client = webui_env["client"]

        resp = await client.post(
            "/ui/api/messages/send",
            headers=_session_header("nonexistent-session-id"),
            json={
                "from_agent_id": "any",
                "to_agent_id": "any",
                "text": "Hello",
            },
        )
        assert resp.status_code == 404


# ===========================================================================
# Deleted endpoints and dependencies — verify removal
# ===========================================================================


class TestDeletedEndpoints:
    """Verify that Auth0/key endpoints are removed from webui_api."""

    async def test_auth_config_removed(self, webui_env):
        """GET /ui/api/auth/config returns 404 or 405."""
        client = webui_env["client"]
        resp = await client.get("/ui/api/auth/config")
        assert resp.status_code in (404, 405)

    async def test_post_keys_removed(self, webui_env):
        """POST /ui/api/keys returns 404 or 405."""
        client = webui_env["client"]
        resp = await client.post("/ui/api/keys")
        assert resp.status_code in (404, 405, 422)

    async def test_get_keys_removed(self, webui_env):
        """GET /ui/api/keys returns 404 or 405."""
        client = webui_env["client"]
        resp = await client.get("/ui/api/keys")
        assert resp.status_code in (404, 405)

    async def test_delete_keys_removed(self, webui_env):
        """DELETE /ui/api/keys/{id} returns 404 or 405."""
        client = webui_env["client"]
        resp = await client.delete("/ui/api/keys/some-id")
        assert resp.status_code in (404, 405)


class TestDeletedDependencies:
    """Verify that Auth0-related dependencies are removed from webui_api."""

    def test_get_webui_tenant_removed(self):
        """get_webui_tenant should not exist in webui_api module."""
        from hikyaku_registry import webui_api

        assert not hasattr(webui_api, "get_webui_tenant")

    def test_verify_auth0_user_not_imported(self):
        """webui_api should not import verify_auth0_user."""
        from hikyaku_registry import webui_api

        assert not hasattr(webui_api, "verify_auth0_user")

    def test_get_user_id_not_imported(self):
        """webui_api should not import get_user_id."""
        from hikyaku_registry import webui_api

        assert not hasattr(webui_api, "get_user_id")

    def test_extract_bearer_removed(self):
        """_extract_bearer should not exist in webui_api module."""
        from hikyaku_registry import webui_api

        assert not hasattr(webui_api, "_extract_bearer")

    def test_get_tenant_agents_renamed(self):
        """_get_tenant_agents should not exist (renamed to _get_session_agents)."""
        from hikyaku_registry import webui_api

        assert not hasattr(webui_api, "_get_tenant_agents")
