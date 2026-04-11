"""Tests for key management WebUI endpoints.

Covers: GET /ui/api/auth/config, POST /ui/api/keys, GET /ui/api/keys,
DELETE /ui/api/keys/{tenant_id}.
Verifies Auth0 JWT auth, key CRUD operations, and ownership enforcement.

The Redis-backed predecessor wired fakeredis into a local FastAPI app and
used ``redis.hget`` / ``sismember`` to assert stored state. The SQL rewrite
uses the shared ``store`` fixture from conftest.py (in-memory aiosqlite)
and checks persisted state via ``store.get_api_key_status`` /
``store.list_api_keys`` / ``store.get_agent``.
"""

import hashlib

import pytest
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

_TEST_SUB_A = "auth0|user-aaa"
_TEST_SUB_B = "auth0|user-bbb"
_TEST_JWT_TOKEN = "eyJ0eXAiOiJKV1QiLCJhbGciOiJSUzI1NiJ9.test.sig"


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------


def _jwt_header() -> dict:
    return {"Authorization": f"Bearer {_TEST_JWT_TOKEN}"}


# ---------------------------------------------------------------------------
# Fixture
# ---------------------------------------------------------------------------


@pytest.fixture
async def key_env(store: RegistryStore, task_store: TaskStore):
    """Set up test FastAPI app with SQL store and auth dependency overrides.

    Overrides verify_auth0_user to always succeed and get_user_id to
    return _TEST_SUB_A by default. Tests can swap the user by calling
    ``_set_user`` on the returned app.

    Yields a dict with:
      - client: httpx.AsyncClient for making requests
      - store: RegistryStore backed by in-memory aiosqlite (conftest)
      - task_store: TaskStore backed by in-memory aiosqlite (conftest)
      - app: the FastAPI app (for mutating dependency_overrides)
    """
    executor = BrokerExecutor(registry_store=store, task_store=task_store)

    app = FastAPI()
    app.include_router(webui_router)

    app.dependency_overrides[get_webui_store] = lambda: store
    app.dependency_overrides[get_webui_task_store] = lambda: task_store
    app.dependency_overrides[get_webui_executor] = lambda: executor

    async def _mock_verify(request=None, cred=None):
        if request is not None:
            request.scope["auth0"] = {"sub": _TEST_SUB_A}
            request.scope["token"] = _TEST_JWT_TOKEN
        return None

    app.dependency_overrides[verify_auth0_user] = _mock_verify
    app.dependency_overrides[get_user_id] = lambda: _TEST_SUB_A

    transport = ASGITransport(app=app)
    async with AsyncClient(transport=transport, base_url="http://test") as client:
        yield {
            "client": client,
            "store": store,
            "task_store": task_store,
            "app": app,
        }


def _set_user(app: FastAPI, sub: str):
    """Change the authenticated user for subsequent requests."""

    async def _mock_verify(request=None, cred=None):
        if request is not None:
            request.scope["auth0"] = {"sub": sub}
            request.scope["token"] = _TEST_JWT_TOKEN
        return None

    app.dependency_overrides[verify_auth0_user] = _mock_verify
    app.dependency_overrides[get_user_id] = lambda: sub


# ===========================================================================
# GET /ui/api/auth/config
# ===========================================================================


class TestAuthConfig:
    """Tests for GET /ui/api/auth/config.

    Returns Auth0 domain, client_id, and audience for SPA initialization.
    No authentication required.
    """

    async def test_returns_200(self, key_env):
        """GET /ui/api/auth/config returns 200."""
        client = key_env["client"]

        resp = await client.get("/ui/api/auth/config")

        assert resp.status_code == 200

    async def test_returns_expected_fields(self, key_env):
        """Response contains domain, client_id, and audience fields."""
        client = key_env["client"]

        resp = await client.get("/ui/api/auth/config")
        data = resp.json()

        assert "domain" in data
        assert "client_id" in data
        assert "audience" in data

    async def test_no_auth_required(self, key_env):
        """Endpoint succeeds without Authorization header."""
        app, client = key_env["app"], key_env["client"]

        app.dependency_overrides.pop(verify_auth0_user, None)
        app.dependency_overrides.pop(get_user_id, None)

        resp = await client.get("/ui/api/auth/config")

        assert resp.status_code == 200


# ===========================================================================
# POST /ui/api/keys
# ===========================================================================


class TestCreateKey:
    """Tests for POST /ui/api/keys.

    Creates a new API key for the authenticated user. Returns the raw
    key (shown only once), tenant_id (hash), and created_at.
    """

    async def test_returns_201(self, key_env):
        """POST /ui/api/keys returns 201 on success."""
        client = key_env["client"]

        resp = await client.post("/ui/api/keys", headers=_jwt_header())

        assert resp.status_code == 201

    async def test_returns_api_key_tenant_id_and_created_at(self, key_env):
        """Response contains api_key, tenant_id, and created_at."""
        client = key_env["client"]

        resp = await client.post("/ui/api/keys", headers=_jwt_header())
        data = resp.json()

        assert "api_key" in data
        assert "tenant_id" in data
        assert "created_at" in data

    async def test_api_key_has_hky_prefix(self, key_env):
        """Returned api_key starts with 'hky_'."""
        client = key_env["client"]

        resp = await client.post("/ui/api/keys", headers=_jwt_header())
        data = resp.json()

        assert data["api_key"].startswith("hky_")

    async def test_tenant_id_is_sha256_of_api_key(self, key_env):
        """tenant_id equals SHA256(api_key)."""
        client = key_env["client"]

        resp = await client.post("/ui/api/keys", headers=_jwt_header())
        data = resp.json()

        expected_hash = hashlib.sha256(data["api_key"].encode()).hexdigest()
        assert data["tenant_id"] == expected_hash

    async def test_key_persisted_as_active(self, key_env):
        """Created key has status='active' in the SQL store."""
        client, store = key_env["client"], key_env["store"]

        resp = await client.post("/ui/api/keys", headers=_jwt_header())
        tenant_id = resp.json()["tenant_id"]

        status = await store.get_api_key_status(tenant_id)
        assert status == "active"

    async def test_key_owner_recorded(self, key_env):
        """Created key is owned by the authenticated user."""
        client, store = key_env["client"], key_env["store"]

        resp = await client.post("/ui/api/keys", headers=_jwt_header())
        tenant_id = resp.json()["tenant_id"]

        assert await store.is_key_owner(tenant_id, _TEST_SUB_A) is True
        assert await store.is_key_owner(tenant_id, _TEST_SUB_B) is False

    async def test_multiple_creates_produce_unique_keys(self, key_env):
        """Multiple POST calls produce unique api_key and tenant_id."""
        client = key_env["client"]

        resp1 = await client.post("/ui/api/keys", headers=_jwt_header())
        resp2 = await client.post("/ui/api/keys", headers=_jwt_header())

        assert resp1.json()["api_key"] != resp2.json()["api_key"]
        assert resp1.json()["tenant_id"] != resp2.json()["tenant_id"]

    async def test_requires_auth(self, key_env):
        """POST /ui/api/keys without valid JWT returns 401 or 403."""
        app, client = key_env["app"], key_env["client"]

        app.dependency_overrides.pop(verify_auth0_user, None)
        app.dependency_overrides.pop(get_user_id, None)

        resp = await client.post("/ui/api/keys")

        assert resp.status_code in (401, 403)


# ===========================================================================
# GET /ui/api/keys
# ===========================================================================


class TestListKeys:
    """Tests for GET /ui/api/keys.

    Lists all API keys owned by the authenticated user.
    """

    async def test_returns_200(self, key_env):
        """GET /ui/api/keys returns 200."""
        client = key_env["client"]

        resp = await client.get("/ui/api/keys", headers=_jwt_header())

        assert resp.status_code == 200

    async def test_empty_list_for_new_user(self, key_env):
        """User with no keys returns empty list."""
        client = key_env["client"]

        resp = await client.get("/ui/api/keys", headers=_jwt_header())
        data = resp.json()

        assert isinstance(data, list)
        assert len(data) == 0

    async def test_returns_created_keys(self, key_env):
        """Lists keys after creation."""
        client = key_env["client"]

        await client.post("/ui/api/keys", headers=_jwt_header())
        await client.post("/ui/api/keys", headers=_jwt_header())

        resp = await client.get("/ui/api/keys", headers=_jwt_header())
        data = resp.json()

        assert len(data) == 2

    async def test_each_key_has_required_fields(self, key_env):
        """Each key in list has tenant_id, key_prefix, created_at, status, agent_count."""
        client = key_env["client"]

        await client.post("/ui/api/keys", headers=_jwt_header())

        resp = await client.get("/ui/api/keys", headers=_jwt_header())
        data = resp.json()

        assert len(data) == 1
        key = data[0]
        required_fields = {
            "tenant_id",
            "key_prefix",
            "created_at",
            "status",
            "agent_count",
        }
        assert required_fields.issubset(set(key.keys()))

    async def test_does_not_return_raw_key(self, key_env):
        """List response does NOT include the raw api_key."""
        client = key_env["client"]

        await client.post("/ui/api/keys", headers=_jwt_header())

        resp = await client.get("/ui/api/keys", headers=_jwt_header())
        data = resp.json()

        assert "api_key" not in data[0]

    async def test_key_prefix_starts_with_hky(self, key_env):
        """key_prefix starts with 'hky_' (first 8 chars of raw key)."""
        client = key_env["client"]

        await client.post("/ui/api/keys", headers=_jwt_header())

        resp = await client.get("/ui/api/keys", headers=_jwt_header())
        data = resp.json()

        assert data[0]["key_prefix"].startswith("hky_")
        assert len(data[0]["key_prefix"]) == 8

    async def test_agent_count_reflects_registered_agents(self, key_env):
        """agent_count matches the number of active agents in the tenant."""
        client, store = key_env["client"], key_env["store"]

        create_resp = await client.post("/ui/api/keys", headers=_jwt_header())
        api_key = create_resp.json()["api_key"]

        await store.create_agent(name="Agent 1", description="Test", api_key=api_key)
        await store.create_agent(name="Agent 2", description="Test", api_key=api_key)

        resp = await client.get("/ui/api/keys", headers=_jwt_header())
        data = resp.json()

        assert data[0]["agent_count"] == 2

    async def test_does_not_return_other_users_keys(self, key_env):
        """Keys created by other users are not returned."""
        app, client = key_env["app"], key_env["client"]

        await client.post("/ui/api/keys", headers=_jwt_header())

        _set_user(app, _TEST_SUB_B)

        resp = await client.get("/ui/api/keys", headers=_jwt_header())
        data = resp.json()

        assert len(data) == 0

    async def test_requires_auth(self, key_env):
        """GET /ui/api/keys without valid JWT returns 401 or 403."""
        app, client = key_env["app"], key_env["client"]

        app.dependency_overrides.pop(verify_auth0_user, None)
        app.dependency_overrides.pop(get_user_id, None)

        resp = await client.get("/ui/api/keys")

        assert resp.status_code in (401, 403)


# ===========================================================================
# DELETE /ui/api/keys/{tenant_id}
# ===========================================================================


class TestRevokeKey:
    """Tests for DELETE /ui/api/keys/{tenant_id}.

    Revokes an API key, deregisters all agents under the tenant.
    Requires JWT auth and ownership of the key.
    """

    async def test_returns_204(self, key_env):
        """DELETE /ui/api/keys/{tenant_id} returns 204 on success."""
        client = key_env["client"]

        create_resp = await client.post("/ui/api/keys", headers=_jwt_header())
        tenant_id = create_resp.json()["tenant_id"]

        resp = await client.delete(
            f"/ui/api/keys/{tenant_id}", headers=_jwt_header()
        )

        assert resp.status_code == 204

    async def test_sets_key_status_to_revoked(self, key_env):
        """Revoked key has status 'revoked' in the store."""
        client, store = key_env["client"], key_env["store"]

        create_resp = await client.post("/ui/api/keys", headers=_jwt_header())
        tenant_id = create_resp.json()["tenant_id"]

        await client.delete(f"/ui/api/keys/{tenant_id}", headers=_jwt_header())

        status = await store.get_api_key_status(tenant_id)
        assert status == "revoked"

    async def test_deregisters_all_tenant_agents(self, key_env):
        """All active agents under the revoked key are deregistered."""
        client, store = key_env["client"], key_env["store"]

        create_resp = await client.post("/ui/api/keys", headers=_jwt_header())
        data = create_resp.json()
        api_key, tenant_id = data["api_key"], data["tenant_id"]

        r1 = await store.create_agent(
            name="Agent 1", description="Test", api_key=api_key
        )
        r2 = await store.create_agent(
            name="Agent 2", description="Test", api_key=api_key
        )

        await client.delete(f"/ui/api/keys/{tenant_id}", headers=_jwt_header())

        a1 = await store.get_agent(r1["agent_id"])
        a2 = await store.get_agent(r2["agent_id"])
        assert a1 is not None and a1["status"] == "deregistered"
        assert a2 is not None and a2["status"] == "deregistered"

    async def test_non_owned_key_returns_404(self, key_env):
        """Deleting a key not owned by the user returns 404."""
        app, client = key_env["app"], key_env["client"]

        create_resp = await client.post("/ui/api/keys", headers=_jwt_header())
        tenant_id = create_resp.json()["tenant_id"]

        _set_user(app, _TEST_SUB_B)

        resp = await client.delete(
            f"/ui/api/keys/{tenant_id}", headers=_jwt_header()
        )

        assert resp.status_code == 404

    async def test_nonexistent_key_returns_404(self, key_env):
        """Deleting a non-existent tenant_id returns 404."""
        client = key_env["client"]

        resp = await client.delete(
            "/ui/api/keys/nonexistent_hash", headers=_jwt_header()
        )

        assert resp.status_code == 404

    async def test_revoked_key_shows_in_list_as_revoked(self, key_env):
        """After revocation, key still appears in GET /ui/api/keys with status 'revoked'."""
        client = key_env["client"]

        create_resp = await client.post("/ui/api/keys", headers=_jwt_header())
        tenant_id = create_resp.json()["tenant_id"]

        await client.delete(f"/ui/api/keys/{tenant_id}", headers=_jwt_header())

        list_resp = await client.get("/ui/api/keys", headers=_jwt_header())
        keys = list_resp.json()

        assert len(keys) == 1
        assert keys[0]["status"] == "revoked"
        assert keys[0]["tenant_id"] == tenant_id

    async def test_does_not_affect_other_keys(self, key_env):
        """Revoking one key does not affect other keys from the same user."""
        client, store = key_env["client"], key_env["store"]

        resp1 = await client.post("/ui/api/keys", headers=_jwt_header())
        resp2 = await client.post("/ui/api/keys", headers=_jwt_header())
        tenant_id_1 = resp1.json()["tenant_id"]
        tenant_id_2 = resp2.json()["tenant_id"]

        await client.delete(
            f"/ui/api/keys/{tenant_id_1}", headers=_jwt_header()
        )

        status = await store.get_api_key_status(tenant_id_2)
        assert status == "active"

    async def test_requires_auth(self, key_env):
        """DELETE /ui/api/keys/{tenant_id} without valid JWT returns 401 or 403."""
        app, client, store = (
            key_env["app"],
            key_env["client"],
            key_env["store"],
        )

        _, api_key_hash, _ = await store.create_api_key(_TEST_SUB_A)

        app.dependency_overrides.pop(verify_auth0_user, None)
        app.dependency_overrides.pop(get_user_id, None)

        resp = await client.delete(f"/ui/api/keys/{api_key_hash}")

        assert resp.status_code in (401, 403)
