"""Tests for WebUI auth migration — JWT + X-Tenant-Id auth.

Covers: get_webui_tenant dependency, modified endpoints (agents, inbox, sent,
messages/send) with JWT + X-Tenant-Id auth, removal of POST /ui/api/login,
and removal of _authenticate_tenant helper.

Design doc reference: Step 4 — WebUI Auth Migration (Backend).
"""

import uuid
from datetime import UTC, datetime

import pytest
import fakeredis.aioredis
from httpx import AsyncClient, ASGITransport
from fastapi import FastAPI
from a2a.types import (
    Artifact,
    Part,
    Task,
    TaskState,
    TaskStatus,
    TextPart,
)

from hikyaku_registry.webui_api import (
    webui_router,
    get_webui_store,
    get_webui_task_store,
    get_webui_executor,
)
from hikyaku_registry.auth import verify_auth0_user, get_user_id
from hikyaku_registry.registry_store import RegistryStore
from hikyaku_registry.task_store import RedisTaskStore
from hikyaku_registry.executor import BrokerExecutor


# ---------------------------------------------------------------------------
# Test constants
# ---------------------------------------------------------------------------

_TEST_SUB_A = "auth0|user-aaa"
_TEST_SUB_B = "auth0|user-bbb"
_TEST_JWT_TOKEN = "eyJ0eXAiOiJKV1QiLCJhbGciOiJSUzI1NiJ9.test.sig"


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------


def _jwt_header(tenant_id: str | None = None) -> dict:
    """Build headers with JWT and optional X-Tenant-Id."""
    headers = {"Authorization": f"Bearer {_TEST_JWT_TOKEN}"}
    if tenant_id is not None:
        headers["X-Tenant-Id"] = tenant_id
    return headers


async def _create_task(
    task_store: RedisTaskStore,
    from_agent_id: str,
    to_agent_id: str,
    text: str = "Hello",
    msg_type: str = "unicast",
    state: TaskState = TaskState.input_required,
) -> Task:
    """Create and save a task in Redis. Returns the saved Task."""
    created_at = datetime.now(UTC).isoformat()

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
        metadata={
            "fromAgentId": from_agent_id,
            "toAgentId": to_agent_id,
            "type": msg_type,
        },
    )
    await task_store.save(task)
    return task


# ---------------------------------------------------------------------------
# Fixture
# ---------------------------------------------------------------------------


@pytest.fixture
async def auth_env():
    """Set up test FastAPI app with JWT auth dependency overrides.

    Auth0 user defaults to _TEST_SUB_A. Use _set_user() to switch.

    Yields a dict with:
      - client: httpx.AsyncClient
      - store: RegistryStore (fakeredis)
      - task_store: RedisTaskStore (fakeredis)
      - redis: raw fakeredis client
      - app: FastAPI app
    """
    redis = fakeredis.aioredis.FakeRedis(decode_responses=True)
    store = RegistryStore(redis)
    task_store = RedisTaskStore(redis)
    executor = BrokerExecutor(registry_store=store, task_store=task_store)

    app = FastAPI()
    app.include_router(webui_router)

    app.dependency_overrides[get_webui_store] = lambda: store
    app.dependency_overrides[get_webui_task_store] = lambda: task_store
    app.dependency_overrides[get_webui_executor] = lambda: executor

    # Default auth: user A
    _set_user_on_app(app, _TEST_SUB_A)

    transport = ASGITransport(app=app)
    async with AsyncClient(transport=transport, base_url="http://test") as client:
        yield {
            "client": client,
            "store": store,
            "task_store": task_store,
            "redis": redis,
            "app": app,
        }

    await redis.aclose()


def _set_user_on_app(app: FastAPI, sub: str):
    """Set the authenticated user on the app."""

    async def _mock_verify(request=None, cred=None):
        if request is not None:
            request.scope["auth0"] = {"sub": sub}
            request.scope["token"] = _TEST_JWT_TOKEN
        return None

    app.dependency_overrides[verify_auth0_user] = _mock_verify
    app.dependency_overrides[get_user_id] = lambda: sub


async def _setup_tenant(store: RegistryStore, owner_sub: str) -> tuple:
    """Create an API key and register an agent under it.

    Returns (api_key, tenant_id, agent_id).
    """
    api_key, tenant_id, _ = await store.create_api_key(owner_sub)
    result = await store.create_agent(
        name="Test Agent", description="Test", api_key=api_key
    )
    return api_key, tenant_id, result["agent_id"]


# ===========================================================================
# get_webui_tenant dependency tests (via endpoint behavior)
# ===========================================================================


class TestGetWebuiTenant:
    """Tests for get_webui_tenant dependency behavior via GET /ui/api/agents.

    Validates JWT auth, X-Tenant-Id extraction, and ownership verification.
    """

    @pytest.mark.asyncio
    async def test_valid_jwt_and_owned_tenant_returns_200(self, auth_env):
        """Valid JWT + owned X-Tenant-Id returns 200."""
        store, client = auth_env["store"], auth_env["client"]

        _, tenant_id, _ = await _setup_tenant(store, _TEST_SUB_A)

        resp = await client.get("/ui/api/agents", headers=_jwt_header(tenant_id))

        assert resp.status_code == 200

    @pytest.mark.asyncio
    async def test_missing_tenant_id_header_returns_400(self, auth_env):
        """Missing X-Tenant-Id header returns 400."""
        client = auth_env["client"]

        # JWT present but no X-Tenant-Id
        resp = await client.get("/ui/api/agents", headers=_jwt_header(tenant_id=None))

        assert resp.status_code == 400

    @pytest.mark.asyncio
    async def test_non_owned_tenant_returns_403(self, auth_env):
        """X-Tenant-Id not owned by the authenticated user returns 403."""
        app, store, client = (
            auth_env["app"],
            auth_env["store"],
            auth_env["client"],
        )

        # User B creates a tenant
        _, tenant_id_b, _ = await _setup_tenant(store, _TEST_SUB_B)

        # User A tries to access user B's tenant
        _set_user_on_app(app, _TEST_SUB_A)
        resp = await client.get("/ui/api/agents", headers=_jwt_header(tenant_id_b))

        assert resp.status_code == 403

    @pytest.mark.asyncio
    async def test_missing_jwt_returns_401(self, auth_env):
        """Missing JWT returns 401."""
        app, store, client = (
            auth_env["app"],
            auth_env["store"],
            auth_env["client"],
        )

        _, tenant_id, _ = await _setup_tenant(store, _TEST_SUB_A)

        # Remove auth overrides
        app.dependency_overrides.pop(verify_auth0_user, None)
        app.dependency_overrides.pop(get_user_id, None)

        resp = await client.get("/ui/api/agents", headers={"X-Tenant-Id": tenant_id})

        assert resp.status_code == 401 or resp.status_code == 403

    @pytest.mark.asyncio
    async def test_nonexistent_tenant_id_returns_403(self, auth_env):
        """X-Tenant-Id that exists nowhere returns 403."""
        client = auth_env["client"]

        resp = await client.get(
            "/ui/api/agents",
            headers=_jwt_header("nonexistent_hash_value"),
        )

        assert resp.status_code == 403


# ===========================================================================
# GET /ui/api/agents (JWT + X-Tenant-Id)
# ===========================================================================


class TestAgentsListJwt:
    """Tests for GET /ui/api/agents with JWT + X-Tenant-Id auth.

    Replaces API key auth. Returns agents in the specified tenant.
    """

    @pytest.mark.asyncio
    async def test_returns_tenant_agents(self, auth_env):
        """Returns agents belonging to the specified tenant."""
        store, client = auth_env["store"], auth_env["client"]

        api_key, tenant_id, agent_id = await _setup_tenant(store, _TEST_SUB_A)

        resp = await client.get("/ui/api/agents", headers=_jwt_header(tenant_id))

        assert resp.status_code == 200
        ids = {a["agent_id"] for a in resp.json()["agents"]}
        assert agent_id in ids

    @pytest.mark.asyncio
    async def test_excludes_other_tenant_agents(self, auth_env):
        """Agents from other tenants are excluded."""
        store, client = auth_env["store"], auth_env["client"]

        _, tenant_id_a, agent_a = await _setup_tenant(store, _TEST_SUB_A)

        # Create another tenant under same user
        api_key_b, tenant_id_b, _ = await store.create_api_key(_TEST_SUB_A)
        r_b = await store.create_agent(
            name="Other Agent", description="Test", api_key=api_key_b
        )

        resp = await client.get("/ui/api/agents", headers=_jwt_header(tenant_id_a))

        ids = {a["agent_id"] for a in resp.json()["agents"]}
        assert agent_a in ids
        assert r_b["agent_id"] not in ids

    @pytest.mark.asyncio
    async def test_400_without_tenant_id(self, auth_env):
        """GET /agents without X-Tenant-Id returns 400."""
        client = auth_env["client"]

        resp = await client.get("/ui/api/agents", headers=_jwt_header(tenant_id=None))

        assert resp.status_code == 400

    @pytest.mark.asyncio
    async def test_403_for_non_owned_tenant(self, auth_env):
        """GET /agents with non-owned X-Tenant-Id returns 403."""
        store, client = auth_env["store"], auth_env["client"]

        _, tenant_id_b, _ = await _setup_tenant(store, _TEST_SUB_B)

        resp = await client.get("/ui/api/agents", headers=_jwt_header(tenant_id_b))

        assert resp.status_code == 403


# ===========================================================================
# GET /ui/api/agents/{agent_id}/inbox (JWT + X-Tenant-Id)
# ===========================================================================


class TestInboxJwt:
    """Tests for GET /ui/api/agents/{agent_id}/inbox with JWT + X-Tenant-Id."""

    @pytest.mark.asyncio
    async def test_returns_inbox_messages(self, auth_env):
        """Returns messages for the specified agent in the tenant."""
        store, task_store, client = (
            auth_env["store"],
            auth_env["task_store"],
            auth_env["client"],
        )

        api_key, tenant_id, agent_id = await _setup_tenant(store, _TEST_SUB_A)

        # Create a second agent as sender
        r_sender = await store.create_agent(
            name="Sender", description="Test", api_key=api_key
        )

        await _create_task(
            task_store,
            from_agent_id=r_sender["agent_id"],
            to_agent_id=agent_id,
            text="Hello from sender",
        )

        resp = await client.get(
            f"/ui/api/agents/{agent_id}/inbox",
            headers=_jwt_header(tenant_id),
        )

        assert resp.status_code == 200
        msgs = resp.json()["messages"]
        assert len(msgs) == 1
        assert msgs[0]["body"] == "Hello from sender"

    @pytest.mark.asyncio
    async def test_empty_inbox(self, auth_env):
        """Agent with no messages returns empty list."""
        store, client = auth_env["store"], auth_env["client"]

        _, tenant_id, agent_id = await _setup_tenant(store, _TEST_SUB_A)

        resp = await client.get(
            f"/ui/api/agents/{agent_id}/inbox",
            headers=_jwt_header(tenant_id),
        )

        assert resp.status_code == 200
        assert resp.json()["messages"] == []

    @pytest.mark.asyncio
    async def test_400_without_tenant_id(self, auth_env):
        """Inbox without X-Tenant-Id returns 400."""
        store, client = auth_env["store"], auth_env["client"]

        _, _, agent_id = await _setup_tenant(store, _TEST_SUB_A)

        resp = await client.get(
            f"/ui/api/agents/{agent_id}/inbox",
            headers=_jwt_header(tenant_id=None),
        )

        assert resp.status_code == 400

    @pytest.mark.asyncio
    async def test_403_for_non_owned_tenant(self, auth_env):
        """Inbox with non-owned X-Tenant-Id returns 403."""
        store, client = auth_env["store"], auth_env["client"]

        _, tenant_id_b, agent_id = await _setup_tenant(store, _TEST_SUB_B)

        resp = await client.get(
            f"/ui/api/agents/{agent_id}/inbox",
            headers=_jwt_header(tenant_id_b),
        )

        assert resp.status_code == 403

    @pytest.mark.asyncio
    async def test_agent_not_in_tenant_returns_404(self, auth_env):
        """Agent belonging to a different tenant returns 404."""
        store, client = auth_env["store"], auth_env["client"]

        _, tenant_id_a, _ = await _setup_tenant(store, _TEST_SUB_A)

        # Create agent under a different tenant (same user)
        api_key_b, _, _ = await store.create_api_key(_TEST_SUB_A)
        r_b = await store.create_agent(
            name="Other Agent", description="Test", api_key=api_key_b
        )

        resp = await client.get(
            f"/ui/api/agents/{r_b['agent_id']}/inbox",
            headers=_jwt_header(tenant_id_a),
        )

        assert resp.status_code == 404


# ===========================================================================
# GET /ui/api/agents/{agent_id}/sent (JWT + X-Tenant-Id)
# ===========================================================================


class TestSentJwt:
    """Tests for GET /ui/api/agents/{agent_id}/sent with JWT + X-Tenant-Id."""

    @pytest.mark.asyncio
    async def test_returns_sent_messages(self, auth_env):
        """Returns messages sent by the specified agent."""
        store, task_store, client, redis = (
            auth_env["store"],
            auth_env["task_store"],
            auth_env["client"],
            auth_env["redis"],
        )

        api_key, tenant_id, sender_id = await _setup_tenant(store, _TEST_SUB_A)
        r_recipient = await store.create_agent(
            name="Recipient", description="Test", api_key=api_key
        )

        task = await _create_task(
            task_store,
            from_agent_id=sender_id,
            to_agent_id=r_recipient["agent_id"],
            text="Sent message",
        )

        # Track sender for the sent endpoint
        await redis.sadd(f"tasks:sender:{sender_id}", task.id)

        resp = await client.get(
            f"/ui/api/agents/{sender_id}/sent",
            headers=_jwt_header(tenant_id),
        )

        assert resp.status_code == 200
        msgs = resp.json()["messages"]
        assert len(msgs) == 1
        assert msgs[0]["body"] == "Sent message"

    @pytest.mark.asyncio
    async def test_empty_sent(self, auth_env):
        """Agent with no sent messages returns empty list."""
        store, client = auth_env["store"], auth_env["client"]

        _, tenant_id, agent_id = await _setup_tenant(store, _TEST_SUB_A)

        resp = await client.get(
            f"/ui/api/agents/{agent_id}/sent",
            headers=_jwt_header(tenant_id),
        )

        assert resp.status_code == 200
        assert resp.json()["messages"] == []

    @pytest.mark.asyncio
    async def test_400_without_tenant_id(self, auth_env):
        """Sent without X-Tenant-Id returns 400."""
        store, client = auth_env["store"], auth_env["client"]

        _, _, agent_id = await _setup_tenant(store, _TEST_SUB_A)

        resp = await client.get(
            f"/ui/api/agents/{agent_id}/sent",
            headers=_jwt_header(tenant_id=None),
        )

        assert resp.status_code == 400

    @pytest.mark.asyncio
    async def test_403_for_non_owned_tenant(self, auth_env):
        """Sent with non-owned X-Tenant-Id returns 403."""
        store, client = auth_env["store"], auth_env["client"]

        _, tenant_id_b, agent_id = await _setup_tenant(store, _TEST_SUB_B)

        resp = await client.get(
            f"/ui/api/agents/{agent_id}/sent",
            headers=_jwt_header(tenant_id_b),
        )

        assert resp.status_code == 403

    @pytest.mark.asyncio
    async def test_agent_not_in_tenant_returns_404(self, auth_env):
        """Agent belonging to a different tenant returns 404."""
        store, client = auth_env["store"], auth_env["client"]

        _, tenant_id_a, _ = await _setup_tenant(store, _TEST_SUB_A)

        api_key_b, _, _ = await store.create_api_key(_TEST_SUB_A)
        r_b = await store.create_agent(
            name="Other Agent", description="Test", api_key=api_key_b
        )

        resp = await client.get(
            f"/ui/api/agents/{r_b['agent_id']}/sent",
            headers=_jwt_header(tenant_id_a),
        )

        assert resp.status_code == 404


# ===========================================================================
# POST /ui/api/messages/send (JWT + X-Tenant-Id)
# ===========================================================================


class TestSendMessageJwt:
    """Tests for POST /ui/api/messages/send with JWT + X-Tenant-Id."""

    @pytest.mark.asyncio
    async def test_400_without_tenant_id(self, auth_env):
        """Send without X-Tenant-Id returns 400."""
        store, client = auth_env["store"], auth_env["client"]

        api_key, _, _ = await _setup_tenant(store, _TEST_SUB_A)
        r2 = await store.create_agent(
            name="Recipient", description="Test", api_key=api_key
        )

        resp = await client.post(
            "/ui/api/messages/send",
            headers=_jwt_header(tenant_id=None),
            json={
                "from_agent_id": "any-agent",
                "to_agent_id": r2["agent_id"],
                "text": "Hello",
            },
        )

        assert resp.status_code == 400

    @pytest.mark.asyncio
    async def test_403_for_non_owned_tenant(self, auth_env):
        """Send with non-owned X-Tenant-Id returns 403."""
        store, client = auth_env["store"], auth_env["client"]

        _, tenant_id_b, agent_b = await _setup_tenant(store, _TEST_SUB_B)

        resp = await client.post(
            "/ui/api/messages/send",
            headers=_jwt_header(tenant_id_b),
            json={
                "from_agent_id": agent_b,
                "to_agent_id": agent_b,
                "text": "Hello",
            },
        )

        assert resp.status_code == 403

    @pytest.mark.asyncio
    async def test_401_without_jwt(self, auth_env):
        """Send without JWT returns 401."""
        app, store, client = (
            auth_env["app"],
            auth_env["store"],
            auth_env["client"],
        )

        _, tenant_id, _ = await _setup_tenant(store, _TEST_SUB_A)

        app.dependency_overrides.pop(verify_auth0_user, None)
        app.dependency_overrides.pop(get_user_id, None)

        resp = await client.post(
            "/ui/api/messages/send",
            headers={"X-Tenant-Id": tenant_id},
            json={
                "from_agent_id": "any",
                "to_agent_id": "any",
                "text": "Hello",
            },
        )

        assert resp.status_code == 401 or resp.status_code == 403


# ===========================================================================
# POST /ui/api/login — removed
# ===========================================================================


class TestLoginRemoved:
    """Tests that POST /ui/api/login endpoint is removed.

    The login endpoint is replaced by Auth0 OIDC flow.
    """

    @pytest.mark.asyncio
    async def test_login_endpoint_removed(self, auth_env):
        """POST /ui/api/login returns 404 or 405 (endpoint no longer exists)."""
        client = auth_env["client"]

        resp = await client.post(
            "/ui/api/login",
            headers={"Authorization": "Bearer some_token"},
        )

        assert resp.status_code in (404, 405)


# ===========================================================================
# _authenticate_tenant helper — removed
# ===========================================================================


class TestAuthenticateTenantRemoved:
    """Tests that _authenticate_tenant helper is removed from webui_api.

    Replaced by get_webui_tenant dependency.
    """

    def test_authenticate_tenant_not_importable(self):
        """_authenticate_tenant should not exist in webui_api module."""
        from hikyaku_registry import webui_api

        assert not hasattr(webui_api, "_authenticate_tenant")
