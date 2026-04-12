"""Tests for api/registry.py — Registry REST API endpoints.

Design doc 0000015 Step 6 rewrites the REST API for session-based routing:

  - ``POST /agents`` accepts ``{session_id, name, description, skills}`` in body
  - ``GET /agents`` reads ``?session_id=`` query param
  - ``GET /agents/{id}`` reads ``X-Session-Id`` header
  - ``DELETE /agents/{id}`` reads ``X-Agent-Id`` header only
  - All 401 bearer errors become 400 ``SESSION_REQUIRED`` / 404 ``SESSION_NOT_FOUND``

No bearer tokens, no ``get_authenticated_agent``, no ``get_registration_tenant``.
"""

import uuid
from datetime import UTC, datetime

import pytest
from fastapi import FastAPI
from httpx import ASGITransport, AsyncClient
from sqlalchemy.ext.asyncio import AsyncSession, async_sessionmaker

from cafleet.api.registry import get_registry_store, registry_router
from cafleet.db.models import Session
from cafleet.registry_store import RegistryStore


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
                Session(
                    session_id=session_id,
                    label=label,
                    created_at=created_at,
                )
            )
    return session_id


# ---------------------------------------------------------------------------
# Fixture
# ---------------------------------------------------------------------------


@pytest.fixture
async def api_env(store: RegistryStore, db_sessionmaker):
    """Set up test FastAPI app with registry router and session-based store.

    Yields a dict with:
      - client: httpx.AsyncClient for making requests
      - store: RegistryStore backed by in-memory aiosqlite
      - app: the FastAPI app (for dependency overrides)
      - session_id: a pre-created session for test agents
    """
    session_id = await _create_test_session(db_sessionmaker)

    app = FastAPI()
    app.include_router(registry_router, prefix="/api/v1")
    app.dependency_overrides[get_registry_store] = lambda: store

    transport = ASGITransport(app=app)
    async with AsyncClient(transport=transport, base_url="http://test") as client:
        yield {
            "client": client,
            "store": store,
            "app": app,
            "session_id": session_id,
            "db_sessionmaker": db_sessionmaker,
        }


async def _register_agent(
    client: AsyncClient,
    session_id: str,
    name: str = "Test Agent",
    description: str = "A test agent",
    skills=None,
):
    """Helper: register an agent via POST and return the response data."""
    body = {"session_id": session_id, "name": name, "description": description}
    if skills is not None:
        body["skills"] = skills
    resp = await client.post("/api/v1/agents", json=body)
    assert resp.status_code == 201, f"Registration failed: {resp.text}"
    return resp.json()


# ---------------------------------------------------------------------------
# POST /api/v1/agents — Register Agent (session-based)
# ---------------------------------------------------------------------------


class TestRegisterAgent:
    """Tests for POST /api/v1/agents.

    Body includes ``session_id`` instead of bearer token.
    """

    async def test_register_returns_201(self, api_env):
        """Successful registration returns HTTP 201."""
        client, session_id = api_env["client"], api_env["session_id"]
        resp = await client.post(
            "/api/v1/agents",
            json={
                "session_id": session_id,
                "name": "My Agent",
                "description": "A coding agent",
            },
        )
        assert resp.status_code == 201

    async def test_register_returns_agent_id_and_name(self, api_env):
        """Response contains agent_id, name, registered_at (no api_key)."""
        client, session_id = api_env["client"], api_env["session_id"]
        data = await _register_agent(client, session_id)

        assert "agent_id" in data
        assert data["name"] == "Test Agent"
        assert "registered_at" in data
        assert "api_key" not in data

    async def test_register_with_skills(self, api_env):
        """Registration with skills succeeds."""
        client, session_id = api_env["client"], api_env["session_id"]
        skills = [
            {
                "id": "python-dev",
                "name": "Python Development",
                "description": "Writes Python code",
                "tags": ["python", "backend"],
            }
        ]
        data = await _register_agent(client, session_id, skills=skills)
        assert "agent_id" in data

    async def test_register_without_skills(self, api_env):
        """Registration without skills succeeds (skills is optional)."""
        client, session_id = api_env["client"], api_env["session_id"]
        resp = await client.post(
            "/api/v1/agents",
            json={
                "session_id": session_id,
                "name": "Plain Agent",
                "description": "No skills",
            },
        )
        assert resp.status_code == 201

    async def test_register_missing_session_id_returns_400(self, api_env):
        """Missing session_id in body returns 400 or 422."""
        client = api_env["client"]
        resp = await client.post(
            "/api/v1/agents",
            json={"name": "No Session", "description": "Missing session_id"},
        )
        assert resp.status_code in (400, 422)

    async def test_register_unknown_session_returns_404(self, api_env):
        """session_id not in sessions table returns 404."""
        client = api_env["client"]
        resp = await client.post(
            "/api/v1/agents",
            json={
                "session_id": "nonexistent-session-id",
                "name": "Orphan",
                "description": "Bad session",
            },
        )
        assert resp.status_code == 404

    async def test_register_missing_name_returns_error(self, api_env):
        """Missing name field returns 400 or 422."""
        client, session_id = api_env["client"], api_env["session_id"]
        resp = await client.post(
            "/api/v1/agents",
            json={"session_id": session_id, "description": "No name provided"},
        )
        assert resp.status_code in (400, 422)

    async def test_register_missing_description_returns_error(self, api_env):
        """Missing description field returns 400 or 422."""
        client, session_id = api_env["client"], api_env["session_id"]
        resp = await client.post(
            "/api/v1/agents",
            json={"session_id": session_id, "name": "No Description Agent"},
        )
        assert resp.status_code in (400, 422)

    async def test_register_multiple_agents_unique_ids(self, api_env):
        """Each registration produces a unique agent_id."""
        client, session_id = api_env["client"], api_env["session_id"]
        data1 = await _register_agent(client, session_id, name="Agent 1")
        data2 = await _register_agent(client, session_id, name="Agent 2")
        assert data1["agent_id"] != data2["agent_id"]

    async def test_register_with_placement_includes_coding_agent(self, api_env):
        """POST /agents with placement returns coding_agent in PlacementView."""
        client, store, session_id = (
            api_env["client"],
            api_env["store"],
            api_env["session_id"],
        )
        director = await store.create_agent("Dir", "d", None, session_id=session_id)

        resp = await client.post(
            "/api/v1/agents",
            json={
                "session_id": session_id,
                "name": "Codex-Member",
                "description": "Uses codex",
                "placement": {
                    "director_agent_id": director["agent_id"],
                    "tmux_session": "main",
                    "tmux_window_id": "@3",
                    "coding_agent": "codex",
                },
            },
            headers={"X-Agent-Id": director["agent_id"]},
        )
        assert resp.status_code == 201
        data = resp.json()
        assert data["placement"]["coding_agent"] == "codex"

    async def test_register_with_placement_default_coding_agent(self, api_env):
        """POST /agents with placement defaults coding_agent to 'claude'."""
        client, store, session_id = (
            api_env["client"],
            api_env["store"],
            api_env["session_id"],
        )
        director = await store.create_agent("Dir", "d", None, session_id=session_id)

        resp = await client.post(
            "/api/v1/agents",
            json={
                "session_id": session_id,
                "name": "Claude-Member",
                "description": "Default",
                "placement": {
                    "director_agent_id": director["agent_id"],
                    "tmux_session": "main",
                    "tmux_window_id": "@3",
                },
            },
            headers={"X-Agent-Id": director["agent_id"]},
        )
        assert resp.status_code == 201
        data = resp.json()
        assert data["placement"]["coding_agent"] == "claude"


# ---------------------------------------------------------------------------
# GET /api/v1/agents — List Agents (session_id query param)
# ---------------------------------------------------------------------------


class TestListAgents:
    """Tests for GET /api/v1/agents.

    Uses ``?session_id=`` query parameter instead of bearer auth.
    """

    async def test_list_returns_registered_agents(self, api_env):
        """Returns all registered agents in the session."""
        client, store, session_id = (
            api_env["client"],
            api_env["store"],
            api_env["session_id"],
        )
        await store.create_agent("Agent 1", "Test", None, session_id=session_id)
        await store.create_agent("Agent 2", "Test", None, session_id=session_id)

        resp = await client.get(f"/api/v1/agents?session_id={session_id}")
        assert resp.status_code == 200
        agents = resp.json()["agents"]
        assert len(agents) >= 2

    async def test_list_returns_empty_for_new_session(self, api_env):
        """A session with no agents returns an empty agents array."""
        client = api_env["client"]
        db_sm = api_env["db_sessionmaker"]
        new_session = await _create_test_session(db_sm)

        resp = await client.get(f"/api/v1/agents?session_id={new_session}")
        assert resp.status_code == 200
        assert resp.json()["agents"] == []

    async def test_list_excludes_deregistered(self, api_env):
        """Deregistered agents are excluded from the list."""
        client, store, session_id = (
            api_env["client"],
            api_env["store"],
            api_env["session_id"],
        )
        a = await store.create_agent("keep", "d", None, session_id=session_id)
        b = await store.create_agent("drop", "d", None, session_id=session_id)
        await store.deregister_agent(b["agent_id"])

        resp = await client.get(f"/api/v1/agents?session_id={session_id}")
        ids = {ag["agent_id"] for ag in resp.json()["agents"]}
        assert a["agent_id"] in ids
        assert b["agent_id"] not in ids

    async def test_list_cross_session_isolation(self, api_env):
        """Agents in session B are not visible when querying session A."""
        client, store, session_a = (
            api_env["client"],
            api_env["store"],
            api_env["session_id"],
        )
        db_sm = api_env["db_sessionmaker"]
        session_b = await _create_test_session(db_sm)

        await store.create_agent("a", "d", None, session_id=session_a)
        await store.create_agent("b", "d", None, session_id=session_b)

        resp = await client.get(f"/api/v1/agents?session_id={session_a}")
        names = {ag["name"] for ag in resp.json()["agents"]}
        assert "a" in names
        assert "b" not in names

    async def test_list_missing_session_id_returns_400(self, api_env):
        """GET /agents without ?session_id= returns 400."""
        client = api_env["client"]
        resp = await client.get("/api/v1/agents")
        assert resp.status_code == 400

    async def test_list_with_director_includes_coding_agent(self, api_env):
        """GET /agents?director_agent_id=... returns coding_agent in placement."""
        from cafleet.models import PlacementCreate

        client, store, session_id = (
            api_env["client"],
            api_env["store"],
            api_env["session_id"],
        )
        director = await store.create_agent("Dir", "d", None, session_id=session_id)

        await store.create_agent_with_placement(
            name="Claude-M",
            description="d",
            skills=None,
            session_id=session_id,
            placement=PlacementCreate(
                director_agent_id=director["agent_id"],
                tmux_session="main",
                tmux_window_id="@1",
                tmux_pane_id="%1",
                coding_agent="claude",
            ),
        )
        await store.create_agent_with_placement(
            name="Codex-M",
            description="d",
            skills=None,
            session_id=session_id,
            placement=PlacementCreate(
                director_agent_id=director["agent_id"],
                tmux_session="main",
                tmux_window_id="@1",
                tmux_pane_id="%2",
                coding_agent="codex",
            ),
        )

        resp = await client.get(
            f"/api/v1/agents?session_id={session_id}"
            f"&director_agent_id={director['agent_id']}"
        )
        assert resp.status_code == 200
        agents = resp.json()["agents"]
        by_name = {a["name"]: a for a in agents}
        assert by_name["Claude-M"]["placement"]["coding_agent"] == "claude"
        assert by_name["Codex-M"]["placement"]["coding_agent"] == "codex"


# ---------------------------------------------------------------------------
# GET /api/v1/agents/{agent_id} — Agent Detail (X-Session-Id header)
# ---------------------------------------------------------------------------


class TestGetAgentDetail:
    """Tests for GET /api/v1/agents/{agent_id}.

    Uses ``X-Session-Id`` header for session scoping.
    """

    async def test_returns_agent_detail(self, api_env):
        """Returns the agent record when session matches."""
        client, store, session_id = (
            api_env["client"],
            api_env["store"],
            api_env["session_id"],
        )
        agent = await store.create_agent("Test", "d", None, session_id=session_id)

        resp = await client.get(
            f"/api/v1/agents/{agent['agent_id']}",
            headers={"X-Session-Id": session_id},
        )
        assert resp.status_code == 200
        data = resp.json()
        assert data["agent_id"] == agent["agent_id"]
        assert data["name"] == "Test"

    async def test_agent_not_in_session_returns_404(self, api_env):
        """Agent belonging to a different session returns 404."""
        client, store, session_a = (
            api_env["client"],
            api_env["store"],
            api_env["session_id"],
        )
        db_sm = api_env["db_sessionmaker"]
        session_b = await _create_test_session(db_sm)
        agent = await store.create_agent("b-agent", "d", None, session_id=session_b)

        resp = await client.get(
            f"/api/v1/agents/{agent['agent_id']}",
            headers={"X-Session-Id": session_a},
        )
        assert resp.status_code == 404

    async def test_nonexistent_agent_returns_404(self, api_env):
        """Unknown agent_id returns 404."""
        client, session_id = api_env["client"], api_env["session_id"]
        resp = await client.get(
            "/api/v1/agents/00000000-0000-4000-8000-000000000000",
            headers={"X-Session-Id": session_id},
        )
        assert resp.status_code == 404

    async def test_deregistered_agent_returns_404(self, api_env):
        """Deregistered agent returns 404."""
        client, store, session_id = (
            api_env["client"],
            api_env["store"],
            api_env["session_id"],
        )
        agent = await store.create_agent("Gone", "d", None, session_id=session_id)
        await store.deregister_agent(agent["agent_id"])

        resp = await client.get(
            f"/api/v1/agents/{agent['agent_id']}",
            headers={"X-Session-Id": session_id},
        )
        assert resp.status_code == 404

    async def test_missing_session_header_returns_400(self, api_env):
        """Missing X-Session-Id header returns 400."""
        client, store, session_id = (
            api_env["client"],
            api_env["store"],
            api_env["session_id"],
        )
        agent = await store.create_agent("Test", "d", None, session_id=session_id)

        resp = await client.get(f"/api/v1/agents/{agent['agent_id']}")
        assert resp.status_code == 400

    async def test_detail_includes_coding_agent_in_placement(self, api_env):
        """GET /agents/{id} returns coding_agent in placement."""
        from cafleet.models import PlacementCreate

        client, store, session_id = (
            api_env["client"],
            api_env["store"],
            api_env["session_id"],
        )
        director = await store.create_agent("Dir", "d", None, session_id=session_id)
        member = await store.create_agent_with_placement(
            name="Codex-Agent",
            description="d",
            skills=None,
            session_id=session_id,
            placement=PlacementCreate(
                director_agent_id=director["agent_id"],
                tmux_session="main",
                tmux_window_id="@1",
                tmux_pane_id="%1",
                coding_agent="codex",
            ),
        )

        resp = await client.get(
            f"/api/v1/agents/{member['agent_id']}",
            headers={"X-Session-Id": session_id},
        )
        assert resp.status_code == 200
        data = resp.json()
        assert data["placement"]["coding_agent"] == "codex"


# ---------------------------------------------------------------------------
# DELETE /api/v1/agents/{agent_id} — Deregister (X-Agent-Id header)
# ---------------------------------------------------------------------------


class TestDeregisterAgent:
    """Tests for DELETE /api/v1/agents/{agent_id}.

    Uses ``X-Agent-Id`` header only (no session header needed).
    """

    async def test_self_deregister_returns_204(self, api_env):
        """Agent can deregister itself via X-Agent-Id."""
        client, store, session_id = (
            api_env["client"],
            api_env["store"],
            api_env["session_id"],
        )
        agent = await store.create_agent("Self", "d", None, session_id=session_id)

        resp = await client.delete(
            f"/api/v1/agents/{agent['agent_id']}",
            headers={"X-Agent-Id": agent["agent_id"]},
        )
        assert resp.status_code == 204

    async def test_director_can_deregister_member(self, api_env):
        """Director can deregister a member agent under its placement."""
        from cafleet.models import PlacementCreate

        client, store, session_id = (
            api_env["client"],
            api_env["store"],
            api_env["session_id"],
        )
        director = await store.create_agent("Dir", "d", None, session_id=session_id)
        member = await store.create_agent_with_placement(
            name="Member",
            description="d",
            skills=None,
            session_id=session_id,
            placement=PlacementCreate(
                director_agent_id=director["agent_id"],
                tmux_session="main",
                tmux_window_id="@1",
                tmux_pane_id="%1",
            ),
        )

        resp = await client.delete(
            f"/api/v1/agents/{member['agent_id']}",
            headers={"X-Agent-Id": director["agent_id"]},
        )
        assert resp.status_code == 204

    async def test_nonexistent_agent_returns_404(self, api_env):
        """Deleting a nonexistent agent returns 404."""
        client, store, session_id = (
            api_env["client"],
            api_env["store"],
            api_env["session_id"],
        )
        agent = await store.create_agent("Caller", "d", None, session_id=session_id)

        resp = await client.delete(
            "/api/v1/agents/00000000-0000-4000-8000-000000000000",
            headers={"X-Agent-Id": agent["agent_id"]},
        )
        assert resp.status_code == 404

    async def test_unrelated_agent_cannot_deregister_returns_403(self, api_env):
        """An agent cannot deregister another agent it doesn't direct."""
        client, store, session_id = (
            api_env["client"],
            api_env["store"],
            api_env["session_id"],
        )
        target = await store.create_agent("Target", "d", None, session_id=session_id)
        stranger = await store.create_agent(
            "Stranger", "d", None, session_id=session_id
        )

        resp = await client.delete(
            f"/api/v1/agents/{target['agent_id']}",
            headers={"X-Agent-Id": stranger["agent_id"]},
        )
        assert resp.status_code == 403

    async def test_missing_agent_id_header_returns_400(self, api_env):
        """Missing X-Agent-Id header returns 400."""
        client, store, session_id = (
            api_env["client"],
            api_env["store"],
            api_env["session_id"],
        )
        agent = await store.create_agent("Test", "d", None, session_id=session_id)

        resp = await client.delete(f"/api/v1/agents/{agent['agent_id']}")
        assert resp.status_code == 400


# ---------------------------------------------------------------------------
# PATCH /api/v1/agents/{agent_id}/placement — coding_agent in response
# ---------------------------------------------------------------------------


class TestPatchPlacement:
    """Tests for PATCH /api/v1/agents/{agent_id}/placement.

    Design doc 0000018: response PlacementView includes coding_agent
    (no new patch field — only tmux_pane_id is patched).
    """

    async def test_patch_response_includes_coding_agent(self, api_env):
        """PATCH placement response includes coding_agent from the stored row."""
        from cafleet.models import PlacementCreate

        client, store, session_id = (
            api_env["client"],
            api_env["store"],
            api_env["session_id"],
        )
        director = await store.create_agent("Dir", "d", None, session_id=session_id)
        member = await store.create_agent_with_placement(
            name="Member",
            description="d",
            skills=None,
            session_id=session_id,
            placement=PlacementCreate(
                director_agent_id=director["agent_id"],
                tmux_session="main",
                tmux_window_id="@1",
                tmux_pane_id=None,
                coding_agent="codex",
            ),
        )

        resp = await client.patch(
            f"/api/v1/agents/{member['agent_id']}/placement",
            json={"tmux_pane_id": "%9"},
            headers={"X-Agent-Id": director["agent_id"]},
        )
        assert resp.status_code == 200
        data = resp.json()
        assert data["coding_agent"] == "codex"
        assert data["tmux_pane_id"] == "%9"


# ---------------------------------------------------------------------------
# Deleted auth patterns — verify they no longer exist
# ---------------------------------------------------------------------------


class TestDeletedAuthPatterns:
    """Verify that bearer/api_key auth patterns are removed."""

    def test_get_authenticated_agent_not_imported(self):
        """registry.py should not import get_authenticated_agent."""
        from cafleet.api import registry

        assert not hasattr(registry, "get_authenticated_agent")

    def test_get_registration_tenant_not_imported(self):
        """registry.py should not import get_registration_tenant."""
        from cafleet.api import registry

        assert not hasattr(registry, "get_registration_tenant")
