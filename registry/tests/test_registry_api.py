"""Tests for api/registry.py — Registry REST API endpoints.

Covers: POST /api/v1/agents, GET /api/v1/agents, GET /api/v1/agents/{id},
DELETE /api/v1/agents/{id}. Tests authentication requirements, ownership
checks, and error responses, plus tenant-scoped registration/list/get
(access-control feature).
"""

import pytest
from fastapi import FastAPI, HTTPException
from httpx import ASGITransport, AsyncClient

from hikyaku_registry.api.registry import get_registry_store, registry_router
from hikyaku_registry.auth import get_authenticated_agent
from hikyaku_registry.registry_store import RegistryStore

_REG_OWNER_SUB = "auth0|registry-test-owner"


# ---------------------------------------------------------------------------
# Fixtures
# ---------------------------------------------------------------------------


@pytest.fixture
async def api_env(store: RegistryStore):
    """Set up test FastAPI app with registry router and SQL-backed store.

    Yields a dict with:
      - client: httpx.AsyncClient for making requests
      - store: RegistryStore backed by in-memory aiosqlite
      - app: the FastAPI app (for dependency overrides)
      - api_key: default API key for registration
      - tenant_id: SHA-256 hash of api_key (== api_keys.api_key_hash)
    """
    api_key, tenant_id, _ = await store.create_api_key(_REG_OWNER_SUB)

    app = FastAPI()
    app.include_router(registry_router, prefix="/api/v1")
    app.dependency_overrides[get_registry_store] = lambda: store

    transport = ASGITransport(app=app)
    async with AsyncClient(transport=transport, base_url="http://test") as client:
        yield {
            "client": client,
            "store": store,
            "app": app,
            "api_key": api_key,
            "tenant_id": tenant_id,
        }


async def _new_tenant(store: RegistryStore, owner_sub: str) -> tuple[str, str]:
    """Create a fresh API key and return (api_key, tenant_id)."""
    api_key, tenant_id, _ = await store.create_api_key(owner_sub)
    return api_key, tenant_id


async def _register_agent(
    client: AsyncClient,
    api_key: str,
    name: str = "Test Agent",
    description: str = "A test agent",
    skills=None,
):
    """Helper: register an agent via POST and return the response data."""
    body = {"name": name, "description": description}
    if skills is not None:
        body["skills"] = skills
    headers = {"Authorization": f"Bearer {api_key}"}
    resp = await client.post("/api/v1/agents", json=body, headers=headers)
    assert resp.status_code == 201
    return resp.json()


def _auth_header(api_key: str) -> dict:
    return {"Authorization": f"Bearer {api_key}"}


def _override_auth_as(app: FastAPI, agent_id: str, tenant_id: str = "test-tenant"):
    """Override auth dependency to return a fixed (agent_id, tenant_id) tuple."""

    async def _fixed_auth():
        return (agent_id, tenant_id)

    app.dependency_overrides[get_authenticated_agent] = _fixed_auth


def _override_auth_as_tenant(app: FastAPI, agent_id: str, tenant_id: str):
    """Override auth dependency to return a (agent_id, tenant_id) tuple."""

    async def _fixed_auth():
        return (agent_id, tenant_id)

    app.dependency_overrides[get_authenticated_agent] = _fixed_auth


def _override_auth_deny(app: FastAPI):
    """Override auth dependency to always raise HTTP 401."""

    async def _deny_auth():
        raise HTTPException(status_code=401, detail="Unauthorized")

    app.dependency_overrides[get_authenticated_agent] = _deny_auth


# ---------------------------------------------------------------------------
# POST /api/v1/agents — Register Agent (auth required)
# ---------------------------------------------------------------------------


class TestRegisterAgent:
    """Tests for POST /api/v1/agents."""

    async def test_register_returns_201(self, api_env):
        """Successful registration returns HTTP 201."""
        client, api_key = api_env["client"], api_env["api_key"]
        resp = await client.post(
            "/api/v1/agents",
            json={"name": "My Agent", "description": "A coding agent"},
            headers=_auth_header(api_key),
        )
        assert resp.status_code == 201

    async def test_register_returns_agent_id_and_api_key(self, api_env):
        """Response contains agent_id, api_key, name, registered_at."""
        client, api_key = api_env["client"], api_env["api_key"]
        data = await _register_agent(
            client, api_key, name="My Agent", description="Coder"
        )

        assert "agent_id" in data
        assert "api_key" in data
        assert data["name"] == "My Agent"
        assert "registered_at" in data

    async def test_register_api_key_format(self, api_env):
        """api_key has hky_ prefix + 32 hex characters."""
        client, api_key = api_env["client"], api_env["api_key"]
        data = await _register_agent(client, api_key)

        assert data["api_key"].startswith("hky_")
        assert len(data["api_key"]) == 36  # "hky_" (4) + 32 hex

    async def test_register_with_skills(self, api_env):
        """Registration with skills succeeds."""
        client, api_key = api_env["client"], api_env["api_key"]
        skills = [
            {
                "id": "python-dev",
                "name": "Python Development",
                "description": "Writes Python code",
                "tags": ["python", "backend"],
            }
        ]
        data = await _register_agent(client, api_key, skills=skills)
        assert "agent_id" in data

    async def test_register_without_skills(self, api_env):
        """Registration without skills succeeds (skills is optional)."""
        client, api_key = api_env["client"], api_env["api_key"]
        resp = await client.post(
            "/api/v1/agents",
            json={"name": "Plain Agent", "description": "No skills"},
            headers=_auth_header(api_key),
        )
        assert resp.status_code == 201

    async def test_register_missing_name_returns_error(self, api_env):
        """Missing name field returns 400 or 422."""
        client = api_env["client"]
        resp = await client.post(
            "/api/v1/agents",
            json={"description": "No name provided"},
        )
        assert resp.status_code in (400, 422)

    async def test_register_missing_description_returns_error(self, api_env):
        """Missing description field returns 400 or 422."""
        client = api_env["client"]
        resp = await client.post(
            "/api/v1/agents",
            json={"name": "No Description Agent"},
        )
        assert resp.status_code in (400, 422)

    async def test_register_empty_body_returns_error(self, api_env):
        """Empty request body returns 400 or 422."""
        client = api_env["client"]
        resp = await client.post("/api/v1/agents", json={})
        assert resp.status_code in (400, 422)

    async def test_register_multiple_agents_unique_ids(self, api_env):
        """Each registration produces a unique agent_id."""
        client, api_key = api_env["client"], api_env["api_key"]
        data1 = await _register_agent(client, api_key, name="Agent 1")
        data2 = await _register_agent(client, api_key, name="Agent 2")
        assert data1["agent_id"] != data2["agent_id"]


# ---------------------------------------------------------------------------
# GET /api/v1/agents — List Agents (auth required)
# ---------------------------------------------------------------------------


class TestListAgents:
    """Tests for GET /api/v1/agents."""

    async def test_list_returns_registered_agents(self, api_env):
        """Returns all registered agents in the agents array."""
        client, app, store = api_env["client"], api_env["app"], api_env["store"]
        api_key, tenant_id = api_env["api_key"], api_env["tenant_id"]

        r1 = await store.create_agent(
            name="Agent 1", description="Test", skills=None, api_key=api_key
        )
        await store.create_agent(
            name="Agent 2", description="Test", skills=None, api_key=api_key
        )

        _override_auth_as_tenant(app, r1["agent_id"], tenant_id)

        resp = await client.get("/api/v1/agents")
        assert resp.status_code == 200

        data = resp.json()
        assert "agents" in data
        names = {a["name"] for a in data["agents"]}
        assert "Agent 1" in names
        assert "Agent 2" in names

    async def test_list_empty_returns_empty_array(self, api_env):
        """Returns empty agents array when no agents registered."""
        client, app = api_env["client"], api_env["app"]

        _override_auth_as(app, "any-agent-id")

        resp = await client.get("/api/v1/agents")
        assert resp.status_code == 200

        data = resp.json()
        assert data["agents"] == []

    async def test_list_agent_has_required_fields(self, api_env):
        """Each agent in the list has agent_id, name, description, registered_at."""
        client, app, store = api_env["client"], api_env["app"], api_env["store"]
        api_key, tenant_id = api_env["api_key"], api_env["tenant_id"]

        r = await store.create_agent(
            name="Detailed Agent",
            description="Has all fields",
            skills=[
                {"id": "s1", "name": "Skill 1", "description": "A skill", "tags": []}
            ],
            api_key=api_key,
        )
        _override_auth_as_tenant(app, r["agent_id"], tenant_id)

        resp = await client.get("/api/v1/agents")
        data = resp.json()
        agent = next(a for a in data["agents"] if a["name"] == "Detailed Agent")

        assert "agent_id" in agent
        assert "name" in agent
        assert "description" in agent
        assert "registered_at" in agent

    async def test_list_excludes_deregistered(self, api_env):
        """Deregistered agents are not included in the list."""
        client, app, store = api_env["client"], api_env["app"], api_env["store"]
        api_key, tenant_id = api_env["api_key"], api_env["tenant_id"]

        r1 = await store.create_agent(
            name="Active Agent", description="Test", skills=None, api_key=api_key
        )
        r2 = await store.create_agent(
            name="Gone Agent", description="Test", skills=None, api_key=api_key
        )

        await store.deregister_agent(r2["agent_id"])

        _override_auth_as_tenant(app, r1["agent_id"], tenant_id)

        resp = await client.get("/api/v1/agents")
        data = resp.json()
        names = {a["name"] for a in data["agents"]}
        assert "Active Agent" in names
        assert "Gone Agent" not in names

    async def test_list_requires_auth(self, api_env):
        """GET /api/v1/agents without auth returns 401."""
        client, app = api_env["client"], api_env["app"]
        _override_auth_deny(app)

        resp = await client.get("/api/v1/agents")
        assert resp.status_code == 401


# ---------------------------------------------------------------------------
# GET /api/v1/agents/{agent_id} — Get Agent Detail (auth required)
# ---------------------------------------------------------------------------


class TestGetAgentDetail:
    """Tests for GET /api/v1/agents/{agent_id}."""

    async def test_get_existing_agent(self, api_env):
        """Returns agent detail for a registered agent."""
        client, app, store = api_env["client"], api_env["app"], api_env["store"]
        api_key, tenant_id = api_env["api_key"], api_env["tenant_id"]

        r = await store.create_agent(
            name="Detail Agent",
            description="Full detail",
            skills=None,
            api_key=api_key,
        )
        _override_auth_as_tenant(app, r["agent_id"], tenant_id)

        resp = await client.get(f"/api/v1/agents/{r['agent_id']}")
        assert resp.status_code == 200

        data = resp.json()
        assert data["name"] == "Detail Agent"

    async def test_get_unknown_agent_returns_404(self, api_env):
        """Returns 404 for non-existent agent_id."""
        client, app = api_env["client"], api_env["app"]
        _override_auth_as(app, "some-authenticated-agent")

        resp = await client.get("/api/v1/agents/00000000-0000-4000-8000-000000000000")
        assert resp.status_code == 404

    async def test_get_unknown_agent_error_format(self, api_env):
        """404 response uses the standard error format."""
        client, app = api_env["client"], api_env["app"]
        _override_auth_as(app, "some-authenticated-agent")

        resp = await client.get("/api/v1/agents/00000000-0000-4000-8000-000000000000")
        data = resp.json()

        assert "error" in data
        assert "code" in data["error"]
        assert data["error"]["code"] == "AGENT_NOT_FOUND"
        assert "message" in data["error"]

    async def test_get_deregistered_agent_returns_404(self, api_env):
        """Returns 404 for a deregistered agent."""
        client, app, store = api_env["client"], api_env["app"], api_env["store"]
        api_key, tenant_id = api_env["api_key"], api_env["tenant_id"]

        r = await _register_agent(client, api_key, name="Soon Gone")
        await store.deregister_agent(r["agent_id"])

        _override_auth_as_tenant(app, "some-agent-id", tenant_id)

        resp = await client.get(f"/api/v1/agents/{r['agent_id']}")
        assert resp.status_code == 404

    async def test_get_requires_auth(self, api_env):
        """GET /api/v1/agents/{id} without auth returns 401."""
        client, app = api_env["client"], api_env["app"]
        api_key = api_env["api_key"]

        r = await _register_agent(client, api_key, name="Auth Test Agent")
        _override_auth_deny(app)

        resp = await client.get(f"/api/v1/agents/{r['agent_id']}")
        assert resp.status_code == 401


# ---------------------------------------------------------------------------
# DELETE /api/v1/agents/{agent_id} — Deregister (auth + ownership)
# ---------------------------------------------------------------------------


class TestDeregisterAgent:
    """Tests for DELETE /api/v1/agents/{agent_id}."""

    async def test_owner_can_deregister(self, api_env):
        """Agent can deregister itself — returns 204."""
        client, app = api_env["client"], api_env["app"]
        api_key, tenant_id = api_env["api_key"], api_env["tenant_id"]

        r = await _register_agent(client, api_key, name="Self Delete")
        _override_auth_as_tenant(app, r["agent_id"], tenant_id)

        resp = await client.delete(f"/api/v1/agents/{r['agent_id']}")
        assert resp.status_code == 204

    async def test_deregistered_agent_removed_from_list(self, api_env):
        """After deregistration, agent no longer appears in GET /agents list."""
        client, app, store = api_env["client"], api_env["app"], api_env["store"]
        api_key, tenant_id = api_env["api_key"], api_env["tenant_id"]

        r1 = await store.create_agent(
            name="Keeper", description="Test", skills=None, api_key=api_key
        )
        r2 = await store.create_agent(
            name="Leaver", description="Test", skills=None, api_key=api_key
        )

        _override_auth_as_tenant(app, r2["agent_id"], tenant_id)
        await client.delete(f"/api/v1/agents/{r2['agent_id']}")

        _override_auth_as_tenant(app, r1["agent_id"], tenant_id)
        resp = await client.get("/api/v1/agents")
        names = {a["name"] for a in resp.json()["agents"]}
        assert "Leaver" not in names
        assert "Keeper" in names

    async def test_non_owner_gets_403(self, api_env):
        """Deleting another agent's registration returns 403."""
        client, app = api_env["client"], api_env["app"]
        api_key, tenant_id = api_env["api_key"], api_env["tenant_id"]

        r1 = await _register_agent(client, api_key, name="Agent 1")
        r2 = await _register_agent(client, api_key, name="Agent 2")

        _override_auth_as_tenant(app, r1["agent_id"], tenant_id)

        resp = await client.delete(f"/api/v1/agents/{r2['agent_id']}")
        assert resp.status_code == 403

    async def test_non_owner_error_format(self, api_env):
        """403 response uses the standard error format."""
        client, app = api_env["client"], api_env["app"]
        api_key, tenant_id = api_env["api_key"], api_env["tenant_id"]

        r1 = await _register_agent(client, api_key, name="Agent A")
        r2 = await _register_agent(client, api_key, name="Agent B")

        _override_auth_as_tenant(app, r1["agent_id"], tenant_id)

        resp = await client.delete(f"/api/v1/agents/{r2['agent_id']}")
        data = resp.json()

        assert "error" in data
        assert data["error"]["code"] == "FORBIDDEN"

    async def test_delete_unknown_agent_returns_404(self, api_env):
        """Deleting non-existent agent returns 404."""
        client, app = api_env["client"], api_env["app"]
        api_key, tenant_id = api_env["api_key"], api_env["tenant_id"]

        r = await _register_agent(client, api_key, name="Existing Agent")
        _override_auth_as_tenant(app, r["agent_id"], tenant_id)

        resp = await client.delete(
            "/api/v1/agents/00000000-0000-4000-8000-000000000000"
        )
        assert resp.status_code == 404

    async def test_delete_unknown_agent_error_format(self, api_env):
        """404 response on delete uses the standard error format."""
        client, app = api_env["client"], api_env["app"]
        _override_auth_as(app, "some-agent-id")

        resp = await client.delete(
            "/api/v1/agents/00000000-0000-4000-8000-000000000000"
        )
        data = resp.json()

        assert "error" in data
        assert data["error"]["code"] == "AGENT_NOT_FOUND"

    async def test_delete_requires_auth(self, api_env):
        """DELETE /api/v1/agents/{id} without auth returns 401."""
        client, app = api_env["client"], api_env["app"]
        api_key = api_env["api_key"]

        r = await _register_agent(client, api_key, name="Auth Test")
        _override_auth_deny(app)

        resp = await client.delete(f"/api/v1/agents/{r['agent_id']}")
        assert resp.status_code == 401


# ===========================================================================
# Multi-tenant API tests (access-control feature)
# ===========================================================================


class TestRegisterAgentTenant:
    """Tests for tenant-aware registration via POST /api/v1/agents.

    Registration always requires a valid, active API key.
    With Authorization → registers agent in the key's tenant.
    """

    async def test_register_with_active_key(self, api_env):
        """Registration with active API key succeeds and echoes back the key."""
        client, api_key = api_env["client"], api_env["api_key"]

        resp = await client.post(
            "/api/v1/agents",
            json={"name": "Tenant Agent", "description": "Joins tenant"},
            headers=_auth_header(api_key),
        )
        assert resp.status_code == 201

        data = resp.json()
        assert data["name"] == "Tenant Agent"
        assert data["api_key"] == api_key
        assert "agent_id" in data

    async def test_multiple_agents_same_key_different_ids(self, api_env):
        """Each agent registered with the same key gets a unique agent_id."""
        client, api_key = api_env["client"], api_env["api_key"]

        first = await _register_agent(client, api_key, name="Agent 1")
        second = await _register_agent(client, api_key, name="Agent 2")

        assert first["agent_id"] != second["agent_id"]
        assert first["api_key"] == second["api_key"] == api_key

    async def test_revoked_key_returns_401(self, api_env):
        """Registration with a revoked API key returns 401."""
        client, store = api_env["client"], api_env["store"]
        api_key, tenant_id = api_env["api_key"], api_env["tenant_id"]

        await store.revoke_api_key(tenant_id, _REG_OWNER_SUB)

        resp = await client.post(
            "/api/v1/agents",
            json={"name": "Late Agent", "description": "Too late"},
            headers=_auth_header(api_key),
        )
        assert resp.status_code == 401

    async def test_join_with_invalid_key_returns_401(self, api_env):
        """Registration with an API key that has no record → 401."""
        client = api_env["client"]

        resp = await client.post(
            "/api/v1/agents",
            json={"name": "Invalid", "description": "Bad key"},
            headers=_auth_header("hky_00000000000000000000000000000000"),
        )
        assert resp.status_code == 401


class TestTenantScopedListAgents:
    """Tests for GET /api/v1/agents with tenant isolation.

    List returns only agents in the caller's tenant.
    """

    async def test_list_returns_only_same_tenant_agents(self, api_env):
        """List agents returns only agents in the caller's tenant."""
        client, app, store = api_env["client"], api_env["app"], api_env["store"]

        key_a, tenant_a = await _new_tenant(store, "auth0|owner-a")
        key_b, _ = await _new_tenant(store, "auth0|owner-b")

        a1 = await store.create_agent(
            name="A1", description="Tenant A", skills=None, api_key=key_a
        )
        await store.create_agent(
            name="A2", description="Tenant A", skills=None, api_key=key_a
        )
        await store.create_agent(
            name="B1", description="Tenant B", skills=None, api_key=key_b
        )

        _override_auth_as_tenant(app, a1["agent_id"], tenant_a)

        resp = await client.get("/api/v1/agents")
        assert resp.status_code == 200

        data = resp.json()
        names = {a["name"] for a in data["agents"]}
        assert "A1" in names
        assert "A2" in names
        assert "B1" not in names

    async def test_list_empty_tenant_returns_empty(self, api_env):
        """List agents for a tenant with no agents returns empty array."""
        client, app = api_env["client"], api_env["app"]

        _override_auth_as_tenant(app, "any-agent", "nonexistent-tenant-hash")

        resp = await client.get("/api/v1/agents")
        assert resp.status_code == 200
        assert resp.json()["agents"] == []


class TestTenantScopedGetAgent:
    """Tests for GET /api/v1/agents/{id} with tenant isolation.

    Cross-tenant lookups return 404 (same as nonexistent).
    """

    async def test_get_same_tenant_agent_succeeds(self, api_env):
        """Getting an agent in the same tenant returns 200."""
        client, app, store = api_env["client"], api_env["app"], api_env["store"]

        key, tenant_hash = await _new_tenant(store, "auth0|same-tenant")

        a1 = await store.create_agent(
            name="A1", description="Same tenant", skills=None, api_key=key
        )
        a2 = await store.create_agent(
            name="A2", description="Same tenant", skills=None, api_key=key
        )

        _override_auth_as_tenant(app, a1["agent_id"], tenant_hash)

        resp = await client.get(f"/api/v1/agents/{a2['agent_id']}")
        assert resp.status_code == 200
        assert resp.json()["name"] == "A2"

    async def test_get_cross_tenant_agent_returns_404(self, api_env):
        """Getting an agent from a different tenant returns 404."""
        client, app, store = api_env["client"], api_env["app"], api_env["store"]

        key_a, tenant_a = await _new_tenant(store, "auth0|owner-a")
        key_b, _ = await _new_tenant(store, "auth0|owner-b")

        a1 = await store.create_agent(
            name="A1", description="Tenant A", skills=None, api_key=key_a
        )
        b1 = await store.create_agent(
            name="B1", description="Tenant B", skills=None, api_key=key_b
        )

        _override_auth_as_tenant(app, a1["agent_id"], tenant_a)

        resp = await client.get(f"/api/v1/agents/{b1['agent_id']}")
        assert resp.status_code == 404

    async def test_cross_tenant_404_same_as_nonexistent(self, api_env):
        """Cross-tenant 404 looks identical to nonexistent agent 404."""
        client, app, store = api_env["client"], api_env["app"], api_env["store"]

        key_a, tenant_a = await _new_tenant(store, "auth0|owner-a")
        key_b, _ = await _new_tenant(store, "auth0|owner-b")

        a1 = await store.create_agent(
            name="A1", description="Tenant A", skills=None, api_key=key_a
        )
        b1 = await store.create_agent(
            name="B1", description="Tenant B", skills=None, api_key=key_b
        )

        _override_auth_as_tenant(app, a1["agent_id"], tenant_a)

        cross_resp = await client.get(f"/api/v1/agents/{b1['agent_id']}")
        ghost_resp = await client.get(
            "/api/v1/agents/00000000-0000-4000-8000-000000000000"
        )

        assert cross_resp.status_code == 404
        assert ghost_resp.status_code == 404
        assert cross_resp.json()["error"]["code"] == ghost_resp.json()["error"]["code"]


# ---------------------------------------------------------------------------
# POST /api/v1/agents with placement — Register Agent with Placement
# ---------------------------------------------------------------------------


class TestRegisterAgentWithPlacement:
    """Tests for POST /api/v1/agents with the optional ``placement`` body field.

    When ``placement`` is provided, the endpoint requires an ``X-Agent-Id``
    header matching ``placement.director_agent_id`` and validates that the
    director belongs to the same tenant as the registration API key.
    """

    async def test_register_with_placement_sets_tmux_fields(self, api_env):
        """Registration with placement returns 201 and includes placement in response."""
        client, api_key = api_env["client"], api_env["api_key"]

        # Register director first (plain registration)
        director = await _register_agent(client, api_key, name="Director")

        # Register member with placement
        body = {
            "name": "Member-A",
            "description": "Placed member",
            "placement": {
                "director_agent_id": director["agent_id"],
                "tmux_session": "main",
                "tmux_window_id": "@3",
                "tmux_pane_id": None,
            },
        }
        headers = {
            **_auth_header(api_key),
            "x-agent-id": director["agent_id"],
        }
        resp = await client.post("/api/v1/agents", json=body, headers=headers)
        assert resp.status_code == 201

        data = resp.json()
        assert "placement" in data
        assert data["placement"]["director_agent_id"] == director["agent_id"]
        assert data["placement"]["tmux_session"] == "main"
        assert data["placement"]["tmux_window_id"] == "@3"
        assert data["placement"]["tmux_pane_id"] is None
        assert "created_at" in data["placement"]

    async def test_register_with_placement_requires_x_agent_id(self, api_env):
        """Registration with placement but no X-Agent-Id header returns 401."""
        client, api_key = api_env["client"], api_env["api_key"]

        director = await _register_agent(client, api_key, name="Director")

        body = {
            "name": "Member-B",
            "description": "Should fail",
            "placement": {
                "director_agent_id": director["agent_id"],
                "tmux_session": "main",
                "tmux_window_id": "@3",
            },
        }
        # No X-Agent-Id header — only Authorization
        resp = await client.post(
            "/api/v1/agents", json=body, headers=_auth_header(api_key)
        )
        assert resp.status_code == 401

        data = resp.json()
        assert data["error"]["code"] == "UNAUTHORIZED"

    async def test_register_with_placement_cross_tenant_director_403(self, api_env):
        """Placement pointing to a director in a different tenant returns 403."""
        client, store = api_env["client"], api_env["store"]
        api_key_a = api_env["api_key"]

        # Register director in tenant A
        director = await _register_agent(client, api_key_a, name="Director-A")

        # Create a second tenant
        api_key_b, _tenant_b = await _new_tenant(store, "auth0|owner-b")

        # Try to register member in tenant B with director from tenant A
        body = {
            "name": "Member-CrossTenant",
            "description": "Should fail",
            "placement": {
                "director_agent_id": director["agent_id"],
                "tmux_session": "main",
                "tmux_window_id": "@3",
            },
        }
        headers = {
            **_auth_header(api_key_b),
            "x-agent-id": director["agent_id"],
        }
        resp = await client.post("/api/v1/agents", json=body, headers=headers)
        assert resp.status_code == 403

        data = resp.json()
        assert data["error"]["code"] == "FORBIDDEN"


# ---------------------------------------------------------------------------
# DELETE /api/v1/agents/{agent_id} — Director-allowed deregister
# ---------------------------------------------------------------------------


class TestDeleteAgentAsDirector:
    """Tests for DELETE /api/v1/agents/{agent_id} with director authorization.

    The design doc relaxes the DELETE caller check: the director of a
    member's placement is allowed to deregister that member, in addition
    to the member itself (self-deregister).
    """

    async def test_delete_agent_as_director_allowed(self, api_env):
        """Director can deregister a member they placed — returns 204."""
        client, app, store = api_env["client"], api_env["app"], api_env["store"]
        api_key, tenant_id = api_env["api_key"], api_env["tenant_id"]

        from hikyaku_registry.models import PlacementCreate

        director = await store.create_agent(
            name="Director", description="Lead", skills=None, api_key=api_key
        )
        placement = PlacementCreate(
            director_agent_id=director["agent_id"],
            tmux_session="main",
            tmux_window_id="@3",
            tmux_pane_id=None,
        )
        member = await store.create_agent_with_placement(
            name="Member", description="Worker", skills=None,
            api_key=api_key, placement=placement,
        )

        _override_auth_as_tenant(app, director["agent_id"], tenant_id)

        resp = await client.delete(f"/api/v1/agents/{member['agent_id']}")
        assert resp.status_code == 204

    async def test_delete_agent_as_unrelated_agent_403(self, api_env):
        """Unrelated agent (neither self nor director) gets 403."""
        client, app, store = api_env["client"], api_env["app"], api_env["store"]
        api_key, tenant_id = api_env["api_key"], api_env["tenant_id"]

        from hikyaku_registry.models import PlacementCreate

        director = await store.create_agent(
            name="Director", description="Lead", skills=None, api_key=api_key
        )
        placement = PlacementCreate(
            director_agent_id=director["agent_id"],
            tmux_session="main",
            tmux_window_id="@3",
            tmux_pane_id=None,
        )
        member = await store.create_agent_with_placement(
            name="Member", description="Worker", skills=None,
            api_key=api_key, placement=placement,
        )
        bystander = await store.create_agent(
            name="Bystander", description="Unrelated", skills=None, api_key=api_key
        )

        # Auth as bystander — not the member, not the director
        _override_auth_as_tenant(app, bystander["agent_id"], tenant_id)

        resp = await client.delete(f"/api/v1/agents/{member['agent_id']}")
        assert resp.status_code == 403

        data = resp.json()
        assert data["error"]["code"] == "FORBIDDEN"

    async def test_delete_agent_removes_placement(self, api_env):
        """After director deletes member, the placement row is also removed."""
        client, app, store = api_env["client"], api_env["app"], api_env["store"]
        api_key, tenant_id = api_env["api_key"], api_env["tenant_id"]

        from hikyaku_registry.models import PlacementCreate

        director = await store.create_agent(
            name="Director", description="Lead", skills=None, api_key=api_key
        )
        placement = PlacementCreate(
            director_agent_id=director["agent_id"],
            tmux_session="main",
            tmux_window_id="@3",
            tmux_pane_id=None,
        )
        member = await store.create_agent_with_placement(
            name="Member", description="Worker", skills=None,
            api_key=api_key, placement=placement,
        )

        # Verify placement exists before delete
        p = await store.get_placement(member["agent_id"])
        assert p is not None

        _override_auth_as_tenant(app, director["agent_id"], tenant_id)

        resp = await client.delete(f"/api/v1/agents/{member['agent_id']}")
        assert resp.status_code == 204

        # Placement should be gone after deregistration
        p = await store.get_placement(member["agent_id"])
        assert p is None


# ---------------------------------------------------------------------------
# PATCH /api/v1/agents/{agent_id}/placement — Update Placement
# ---------------------------------------------------------------------------


class TestPatchPlacement:
    """Tests for PATCH /api/v1/agents/{agent_id}/placement.

    Two-pass write pattern: the director registers a member with
    ``tmux_pane_id=None`` (pending), then PATCHes the pane_id once the
    tmux pane is actually spawned.
    """

    async def test_patch_placement_sets_pane_id(self, api_env):
        """Director PATCHes pane_id on a pending placement — returns updated view."""
        client, app, store = api_env["client"], api_env["app"], api_env["store"]
        api_key, tenant_id = api_env["api_key"], api_env["tenant_id"]

        from hikyaku_registry.models import PlacementCreate

        director = await store.create_agent(
            name="Director", description="Lead", skills=None, api_key=api_key
        )
        placement = PlacementCreate(
            director_agent_id=director["agent_id"],
            tmux_session="main",
            tmux_window_id="@3",
            tmux_pane_id=None,
        )
        member = await store.create_agent_with_placement(
            name="Member", description="Worker", skills=None,
            api_key=api_key, placement=placement,
        )

        _override_auth_as_tenant(app, director["agent_id"], tenant_id)

        resp = await client.patch(
            f"/api/v1/agents/{member['agent_id']}/placement",
            json={"tmux_pane_id": "%42"},
        )
        assert resp.status_code == 200

        data = resp.json()
        assert data["tmux_pane_id"] == "%42"
        assert data["director_agent_id"] == director["agent_id"]
        assert data["tmux_session"] == "main"
        assert data["tmux_window_id"] == "@3"

    async def test_patch_placement_caller_must_be_director(self, api_env):
        """Non-director caller trying to PATCH placement gets 403."""
        client, app, store = api_env["client"], api_env["app"], api_env["store"]
        api_key, tenant_id = api_env["api_key"], api_env["tenant_id"]

        from hikyaku_registry.models import PlacementCreate

        director = await store.create_agent(
            name="Director", description="Lead", skills=None, api_key=api_key
        )
        placement = PlacementCreate(
            director_agent_id=director["agent_id"],
            tmux_session="main",
            tmux_window_id="@3",
            tmux_pane_id=None,
        )
        member = await store.create_agent_with_placement(
            name="Member", description="Worker", skills=None,
            api_key=api_key, placement=placement,
        )

        # Auth as the member itself — not the director
        _override_auth_as_tenant(app, member["agent_id"], tenant_id)

        resp = await client.patch(
            f"/api/v1/agents/{member['agent_id']}/placement",
            json={"tmux_pane_id": "%42"},
        )
        assert resp.status_code == 403

        data = resp.json()
        assert data["error"]["code"] == "FORBIDDEN"


# ---------------------------------------------------------------------------
# GET /api/v1/agents?director_agent_id=X — List by Director
# ---------------------------------------------------------------------------


class TestListAgentsFilterByDirector:
    """Tests for GET /api/v1/agents?director_agent_id=X.

    When the ``director_agent_id`` query param is set, the endpoint returns
    only active agents placed by that director (via
    ``store.list_placements_for_director``), each including a ``placement``
    object.
    """

    async def test_list_agents_filter_by_director(self, api_env):
        """Filtering by director returns only members placed by that director."""
        client, app, store = api_env["client"], api_env["app"], api_env["store"]
        api_key, tenant_id = api_env["api_key"], api_env["tenant_id"]

        from hikyaku_registry.models import PlacementCreate

        director = await store.create_agent(
            name="Director", description="Lead", skills=None, api_key=api_key
        )

        for name in ("Member-1", "Member-2"):
            placement = PlacementCreate(
                director_agent_id=director["agent_id"],
                tmux_session="main",
                tmux_window_id="@3",
                tmux_pane_id=None,
            )
            await store.create_agent_with_placement(
                name=name, description="Worker", skills=None,
                api_key=api_key, placement=placement,
            )

        # A plain agent with no placement — should not appear in filtered list
        await store.create_agent(
            name="Plain-Agent", description="No placement",
            skills=None, api_key=api_key,
        )

        _override_auth_as_tenant(app, director["agent_id"], tenant_id)

        resp = await client.get(
            f"/api/v1/agents?director_agent_id={director['agent_id']}"
        )
        assert resp.status_code == 200

        data = resp.json()
        names = {a["name"] for a in data["agents"]}
        assert names == {"Member-1", "Member-2"}

        # Each returned agent should carry a placement object
        for agent in data["agents"]:
            assert agent["placement"] is not None
            assert agent["placement"]["director_agent_id"] == director["agent_id"]
