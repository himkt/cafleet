"""Tests for RegistryStore — SQL-backed agent + session store.

Uses the conftest.py ``store``, ``db_engine``, and ``db_sessionmaker`` fixtures.

## Test isolation strategy

The conftest fixture stack uses a function-scoped in-memory aiosqlite
engine that persists across tests within a single pytest session. To
prevent cross-test contamination without per-test cleanup, every
test generates a fresh session_id via ``_create_test_session()``
(UUID-based) and queries data scoped to that session. Two tests can
never see each other's sessions, agents, or tasks because their
scoping identifiers are disjoint.

## Coverage map

  | Method                              | Test class                       |
  |-------------------------------------|----------------------------------|
  | create_agent                        | TestCreateAgent                  |
  | create_agent_with_placement         | TestCreateAgentWithPlacement     |
  | get_agent                           | TestGetAgent                     |
  | list_active_agents                  | TestListActiveAgents             |
  | deregister_agent                    | TestDeregisterAgent              |
  | verify_agent_session                | TestVerifyAgentSession           |
  | list_sessions                       | TestListSessions                 |
  | get_session                         | TestGetSession                   |
  | get_agent_name                      | TestGetAgentName                 |
  | list_deregistered_agents_with_tasks | TestListDeregisteredAgentsWithTasks |
  | list_placements_for_director        | TestListPlacementsForDirector    |

Deleted methods (no tests):
  - create_api_key, list_api_keys, revoke_api_key, get_api_key_status,
    is_api_key_active, is_key_owner — removed per design doc 0000015
"""

import json
import uuid
from datetime import UTC, datetime

import pytest
from sqlalchemy import text
from sqlalchemy.exc import IntegrityError
from sqlalchemy.ext.asyncio import async_sessionmaker, AsyncSession

from hikyaku.db.models import Session


# ---------------------------------------------------------------------------
# Helpers
#
# All test data is scoped to a unique session_id per call so that tests
# sharing the session-scoped in-memory engine cannot contaminate each
# other.
# ---------------------------------------------------------------------------


async def _create_test_session(
    db_sessionmaker: async_sessionmaker[AsyncSession],
    *,
    session_id: str | None = None,
    label: str | None = None,
) -> str:
    """Seed a session row directly via the DB sessionmaker.

    Returns the ``session_id``. Session creation in production goes
    through the CLI (sync), not through RegistryStore, so tests seed
    sessions directly.
    """
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


async def _make_session_with_id(
    db_sessionmaker: async_sessionmaker[AsyncSession],
) -> str:
    """Create a session with a unique UUID and return the session_id."""
    return await _create_test_session(db_sessionmaker)


async def _seed_task_for_agent(db_engine, *, agent_id: str) -> None:
    """Insert one task into ``tasks`` with ``context_id = agent_id``.

    Used by ``TestListDeregisteredAgentsWithTasks`` to set up the
    "agent has at least one task" precondition.
    """
    now = datetime.now(UTC).isoformat()
    async with db_engine.begin() as conn:
        await conn.execute(
            text(
                "INSERT INTO tasks "
                "(task_id, context_id, from_agent_id, to_agent_id, "
                " type, created_at, status_state, status_timestamp, task_json) "
                "VALUES (:tid, :ctx, :from_, :to, 'unicast', :now, "
                "        'submitted', :now, '{}')"
            ),
            {
                "tid": str(uuid.uuid4()),
                "ctx": agent_id,
                "from_": agent_id,
                "to": agent_id,
                "now": now,
            },
        )


# ---------------------------------------------------------------------------
# create_agent
# ---------------------------------------------------------------------------


class TestCreateAgent:
    """Tests for ``RegistryStore.create_agent(session_id=...)``.

    Design doc 0000015 Step 4: ``create_agent`` takes ``session_id``
    directly (no ``api_key``, no SHA-256 derivation).
    """

    async def test_returns_required_fields(self, store, db_sessionmaker):
        """Result contains ``agent_id``, ``name``, ``registered_at``."""
        session_id = await _create_test_session(db_sessionmaker)
        result = await store.create_agent(
            "Test Agent", "A test agent", None, session_id=session_id
        )
        assert "agent_id" in result
        assert "name" in result
        assert "registered_at" in result
        assert result["name"] == "Test Agent"
        datetime.fromisoformat(result["registered_at"])

    async def test_no_api_key_in_result(self, store, db_sessionmaker):
        """Result does NOT contain ``api_key`` — key concept is removed."""
        session_id = await _create_test_session(db_sessionmaker)
        result = await store.create_agent(
            "Test Agent", "A test agent", None, session_id=session_id
        )
        assert "api_key" not in result

    async def test_with_skills_persists_in_agent_card(self, store, db_sessionmaker):
        """Skills passed to create_agent appear in ``agent_card_json``."""
        session_id = await _create_test_session(db_sessionmaker)
        skills = [
            {
                "id": "py",
                "name": "Python",
                "description": "writes Python",
                "tags": ["lang"],
            }
        ]
        result = await store.create_agent(
            "Skilled", "desc", skills, session_id=session_id
        )
        agent = await store.get_agent(result["agent_id"])
        assert agent is not None
        card = json.loads(agent["agent_card_json"])
        assert card.get("skills") and card["skills"][0]["id"] == "py"

    async def test_unique_agent_ids(self, store, db_sessionmaker):
        """Multiple agents under the same session get distinct ``agent_id``s."""
        session_id = await _create_test_session(db_sessionmaker)
        a = await store.create_agent("a", "d", None, session_id=session_id)
        b = await store.create_agent("b", "d", None, session_id=session_id)
        assert a["agent_id"] != b["agent_id"]

    async def test_rejects_unknown_session(self, store):
        """Creating an agent with a session_id not in sessions raises ``IntegrityError``.

        Verifies the FK constraint ``agents.session_id -> sessions.session_id``
        enforces session existence at INSERT time.
        """
        with pytest.raises(IntegrityError):
            await store.create_agent(
                "Orphan", "desc", None, session_id="nonexistent-session-id"
            )


# ---------------------------------------------------------------------------
# create_agent_with_placement
# ---------------------------------------------------------------------------


class TestCreateAgentWithPlacement:
    """Tests for ``RegistryStore.create_agent_with_placement``."""

    async def test_create_agent_with_placement(self, store, db_sessionmaker):
        """Both the agent row and the placement row are created atomically."""
        from hikyaku.models import PlacementCreate

        session_id = await _create_test_session(db_sessionmaker)
        director = await store.create_agent(
            "Director", "Lead agent", None, session_id=session_id
        )

        placement = PlacementCreate(
            director_agent_id=director["agent_id"],
            tmux_session="main",
            tmux_window_id="@3",
            tmux_pane_id=None,
        )
        result = await store.create_agent_with_placement(
            name="Member-A",
            description="Test member",
            skills=None,
            session_id=session_id,
            placement=placement,
        )

        assert "agent_id" in result
        assert result["name"] == "Member-A"
        assert "registered_at" in result

        agent = await store.get_agent(result["agent_id"])
        assert agent is not None
        assert agent["status"] == "active"

        p = await store.get_placement(agent_id=result["agent_id"])
        assert p is not None
        assert p["director_agent_id"] == director["agent_id"]
        assert p["tmux_session"] == "main"
        assert p["tmux_window_id"] == "@3"
        assert p["tmux_pane_id"] is None

    async def test_without_placement_creates_agent_only(self, store, db_sessionmaker):
        """Calling with ``placement=None`` creates agent but no placement row."""
        session_id = await _create_test_session(db_sessionmaker)
        result = await store.create_agent_with_placement(
            name="Plain Agent",
            description="No placement",
            skills=None,
            session_id=session_id,
            placement=None,
        )
        assert "agent_id" in result
        p = await store.get_placement(agent_id=result["agent_id"])
        assert p is None

    async def test_placement_with_pane_id(self, store, db_sessionmaker):
        """When ``tmux_pane_id`` is provided, it is stored correctly."""
        from hikyaku.models import PlacementCreate

        session_id = await _create_test_session(db_sessionmaker)
        director = await store.create_agent("Dir", "d", None, session_id=session_id)

        placement = PlacementCreate(
            director_agent_id=director["agent_id"],
            tmux_session="work",
            tmux_window_id="@5",
            tmux_pane_id="%12",
        )
        result = await store.create_agent_with_placement(
            name="Member-B",
            description="Has pane",
            skills=None,
            session_id=session_id,
            placement=placement,
        )

        p = await store.get_placement(agent_id=result["agent_id"])
        assert p is not None
        assert p["tmux_pane_id"] == "%12"


# ---------------------------------------------------------------------------
# get_agent
# ---------------------------------------------------------------------------


class TestGetAgent:
    """Tests for ``RegistryStore.get_agent``."""

    async def test_returns_existing_agent(self, store, db_sessionmaker):
        session_id = await _create_test_session(db_sessionmaker)
        created = await store.create_agent(
            "Test Agent", "A test agent", None, session_id=session_id
        )
        agent = await store.get_agent(created["agent_id"])
        assert agent is not None
        assert agent["agent_id"] == created["agent_id"]
        assert agent["name"] == "Test Agent"
        assert agent["description"] == "A test agent"
        assert agent["status"] == "active"

    async def test_returns_none_for_missing(self, store):
        agent = await store.get_agent("00000000-0000-4000-8000-000000000000")
        assert agent is None

    async def test_returns_record_for_deregistered_agent(self, store, db_sessionmaker):
        """Deregistered agents still have a get_agent record (soft delete)."""
        session_id = await _create_test_session(db_sessionmaker)
        created = await store.create_agent("a", "d", None, session_id=session_id)
        await store.deregister_agent(created["agent_id"])
        agent = await store.get_agent(created["agent_id"])
        assert agent is not None
        assert agent["status"] == "deregistered"


# ---------------------------------------------------------------------------
# list_active_agents
# ---------------------------------------------------------------------------


class TestListActiveAgents:
    """Tests for ``RegistryStore.list_active_agents(session_id=...)``.

    Always passes ``session_id`` so that tests are scoped to their own
    session and immune to cross-test contamination.
    """

    async def test_empty_for_new_session(self, store, db_sessionmaker):
        session_id = await _create_test_session(db_sessionmaker)
        agents = await store.list_active_agents(session_id=session_id)
        assert agents == []

    async def test_returns_all_active_for_session(self, store, db_sessionmaker):
        session_id = await _create_test_session(db_sessionmaker)
        await store.create_agent("a1", "d", None, session_id=session_id)
        await store.create_agent("a2", "d", None, session_id=session_id)
        agents = await store.list_active_agents(session_id=session_id)
        assert len(agents) == 2
        assert {a["name"] for a in agents} == {"a1", "a2"}

    async def test_excludes_deregistered_agents(self, store, db_sessionmaker):
        session_id = await _create_test_session(db_sessionmaker)
        a = await store.create_agent("a", "d", None, session_id=session_id)
        b = await store.create_agent("b", "d", None, session_id=session_id)
        await store.deregister_agent(a["agent_id"])
        agents = await store.list_active_agents(session_id=session_id)
        assert len(agents) == 1
        assert agents[0]["agent_id"] == b["agent_id"]

    async def test_filters_by_session_id_isolates_sessions(
        self, store, db_sessionmaker
    ):
        """Cross-session isolation: agents in session A do not appear in session B."""
        session_a = await _create_test_session(db_sessionmaker)
        session_b = await _create_test_session(db_sessionmaker)
        await store.create_agent("a", "d", None, session_id=session_a)
        await store.create_agent("b", "d", None, session_id=session_b)

        a_list = await store.list_active_agents(session_id=session_a)
        b_list = await store.list_active_agents(session_id=session_b)
        assert {x["name"] for x in a_list} == {"a"}
        assert {x["name"] for x in b_list} == {"b"}


# ---------------------------------------------------------------------------
# deregister_agent
# ---------------------------------------------------------------------------


class TestDeregisterAgent:
    """Tests for ``RegistryStore.deregister_agent``."""

    async def test_sets_status_and_timestamp(self, store, db_sessionmaker):
        session_id = await _create_test_session(db_sessionmaker)
        created = await store.create_agent("a", "d", None, session_id=session_id)
        result = await store.deregister_agent(created["agent_id"])
        assert result is True

        agent = await store.get_agent(created["agent_id"])
        assert agent["status"] == "deregistered"
        assert agent.get("deregistered_at"), "deregistered_at must be set"
        datetime.fromisoformat(agent["deregistered_at"])

    async def test_returns_false_for_already_deregistered(self, store, db_sessionmaker):
        """Idempotency: a second deregister call returns False (no-op)."""
        session_id = await _create_test_session(db_sessionmaker)
        created = await store.create_agent("a", "d", None, session_id=session_id)
        first = await store.deregister_agent(created["agent_id"])
        second = await store.deregister_agent(created["agent_id"])
        assert first is True
        assert second is False

    async def test_returns_false_for_missing_agent(self, store):
        result = await store.deregister_agent("00000000-0000-4000-8000-000000000000")
        assert result is False

    async def test_deregister_cascades_placement(self, store, db_sessionmaker):
        """Deregistering an agent also hard-deletes its placement row."""
        from hikyaku.models import PlacementCreate

        session_id = await _create_test_session(db_sessionmaker)
        director = await store.create_agent("Dir", "d", None, session_id=session_id)

        placement = PlacementCreate(
            director_agent_id=director["agent_id"],
            tmux_session="main",
            tmux_window_id="@3",
            tmux_pane_id="%7",
        )
        member = await store.create_agent_with_placement(
            name="Member",
            description="Will be deregistered",
            skills=None,
            session_id=session_id,
            placement=placement,
        )

        assert await store.get_placement(agent_id=member["agent_id"]) is not None
        result = await store.deregister_agent(member["agent_id"])
        assert result is True

        agent = await store.get_agent(member["agent_id"])
        assert agent["status"] == "deregistered"
        assert await store.get_placement(agent_id=member["agent_id"]) is None


# ---------------------------------------------------------------------------
# verify_agent_session (renamed from verify_agent_tenant)
# ---------------------------------------------------------------------------


class TestVerifyAgentSession:
    """Tests for ``RegistryStore.verify_agent_session``.

    Renamed from ``verify_agent_tenant`` per design doc 0000015 Step 4.
    """

    async def test_matching_session_returns_true(self, store, db_sessionmaker):
        session_id = await _create_test_session(db_sessionmaker)
        agent = await store.create_agent("a", "d", None, session_id=session_id)
        assert await store.verify_agent_session(agent["agent_id"], session_id) is True

    async def test_wrong_session_returns_false(self, store, db_sessionmaker):
        session_a = await _create_test_session(db_sessionmaker)
        session_b = await _create_test_session(db_sessionmaker)
        agent = await store.create_agent("a", "d", None, session_id=session_a)
        assert await store.verify_agent_session(agent["agent_id"], session_b) is False

    async def test_nonexistent_agent_returns_false(self, store, db_sessionmaker):
        session_id = await _create_test_session(db_sessionmaker)
        assert (
            await store.verify_agent_session(
                "00000000-0000-4000-8000-000000000000", session_id
            )
            is False
        )

    async def test_deregistered_agent_still_verifiable(self, store, db_sessionmaker):
        """A deregistered agent's row still has session_id; verify must still work."""
        session_id = await _create_test_session(db_sessionmaker)
        agent = await store.create_agent("a", "d", None, session_id=session_id)
        await store.deregister_agent(agent["agent_id"])
        assert await store.verify_agent_session(agent["agent_id"], session_id) is True


# ---------------------------------------------------------------------------
# list_sessions (new — async, read-only)
# ---------------------------------------------------------------------------


class TestListSessions:
    """Tests for ``RegistryStore.list_sessions``.

    Design doc 0000015 Step 4: async method returning all sessions
    with agent_count for the WebUI ``GET /ui/api/sessions`` endpoint.
    """

    async def test_returns_empty_when_no_sessions(self, store):
        """No sessions in the DB returns an empty list."""
        sessions = await store.list_sessions()
        # May not be strictly empty if other tests seeded sessions,
        # but a fresh fixture should be empty.
        assert isinstance(sessions, list)

    async def test_returns_seeded_session(self, store, db_sessionmaker):
        """A seeded session appears in the list."""
        session_id = await _create_test_session(db_sessionmaker, label="test-session")
        sessions = await store.list_sessions()
        match = [s for s in sessions if s["session_id"] == session_id]
        assert len(match) == 1
        assert match[0]["label"] == "test-session"
        assert "created_at" in match[0]
        assert "agent_count" in match[0]

    async def test_agent_count_reflects_active_agents(self, store, db_sessionmaker):
        """agent_count counts only active agents in the session."""
        session_id = await _create_test_session(db_sessionmaker)
        a = await store.create_agent("a", "d", None, session_id=session_id)
        await store.create_agent("b", "d", None, session_id=session_id)
        await store.deregister_agent(a["agent_id"])

        sessions = await store.list_sessions()
        match = [s for s in sessions if s["session_id"] == session_id]
        assert len(match) == 1
        assert match[0]["agent_count"] == 1, (
            "agent_count should only count active agents"
        )

    async def test_agent_count_zero_when_no_agents(self, store, db_sessionmaker):
        """A session with no agents has agent_count=0."""
        session_id = await _create_test_session(db_sessionmaker)
        sessions = await store.list_sessions()
        match = [s for s in sessions if s["session_id"] == session_id]
        assert len(match) == 1
        assert match[0]["agent_count"] == 0

    async def test_multiple_sessions_each_with_own_count(self, store, db_sessionmaker):
        """Multiple sessions have independent agent counts."""
        session_a = await _create_test_session(db_sessionmaker, label="A")
        session_b = await _create_test_session(db_sessionmaker, label="B")
        await store.create_agent("a1", "d", None, session_id=session_a)
        await store.create_agent("a2", "d", None, session_id=session_a)
        await store.create_agent("b1", "d", None, session_id=session_b)

        sessions = await store.list_sessions()
        a = [s for s in sessions if s["session_id"] == session_a][0]
        b = [s for s in sessions if s["session_id"] == session_b][0]
        assert a["agent_count"] == 2
        assert b["agent_count"] == 1

    async def test_session_with_null_label(self, store, db_sessionmaker):
        """A session with no label returns label=None."""
        session_id = await _create_test_session(db_sessionmaker, label=None)
        sessions = await store.list_sessions()
        match = [s for s in sessions if s["session_id"] == session_id]
        assert len(match) == 1
        assert match[0]["label"] is None


# ---------------------------------------------------------------------------
# get_session (new — async, read-only)
# ---------------------------------------------------------------------------


class TestGetSession:
    """Tests for ``RegistryStore.get_session``.

    Design doc 0000015 Step 4: async method returning a single session
    by session_id, or None if not found.
    """

    async def test_returns_existing_session(self, store, db_sessionmaker):
        session_id = await _create_test_session(db_sessionmaker, label="my-session")
        result = await store.get_session(session_id)
        assert result is not None
        assert result["session_id"] == session_id
        assert result["label"] == "my-session"
        assert "created_at" in result

    async def test_returns_none_for_missing(self, store):
        result = await store.get_session("nonexistent-session-id")
        assert result is None

    async def test_session_without_label(self, store, db_sessionmaker):
        """A session with no label returns label=None."""
        session_id = await _create_test_session(db_sessionmaker, label=None)
        result = await store.get_session(session_id)
        assert result is not None
        assert result["label"] is None


# ---------------------------------------------------------------------------
# get_agent_name
# ---------------------------------------------------------------------------


class TestGetAgentName:
    """Tests for ``RegistryStore.get_agent_name``."""

    async def test_returns_name_for_existing(self, store, db_sessionmaker):
        session_id = await _create_test_session(db_sessionmaker)
        agent = await store.create_agent("My Name", "d", None, session_id=session_id)
        assert await store.get_agent_name(agent["agent_id"]) == "My Name"

    async def test_returns_empty_string_for_missing(self, store):
        """Per the design doc contract: returns ``''`` (NOT ``None``) for missing."""
        result = await store.get_agent_name("00000000-0000-4000-8000-000000000000")
        assert result == "", (
            f"get_agent_name must return '' for missing agents, got {result!r}"
        )


# ---------------------------------------------------------------------------
# list_deregistered_agents_with_tasks
# ---------------------------------------------------------------------------


class TestListDeregisteredAgentsWithTasks:
    """Tests for ``RegistryStore.list_deregistered_agents_with_tasks(session_id)``.

    Renamed parameter from ``tenant_id`` to ``session_id`` per design
    doc 0000015 Step 4.
    """

    async def test_excludes_active_agents(self, store, db_engine, db_sessionmaker):
        """Active agents (with or without tasks) are excluded from the result."""
        session_id = await _create_test_session(db_sessionmaker)
        active = await store.create_agent("active", "d", None, session_id=session_id)
        await _seed_task_for_agent(db_engine, agent_id=active["agent_id"])

        result = await store.list_deregistered_agents_with_tasks(session_id)
        result_ids = {r["agent_id"] for r in result}
        assert active["agent_id"] not in result_ids

    async def test_excludes_deregistered_without_tasks(self, store, db_sessionmaker):
        """Deregistered agents with no tasks are excluded."""
        session_id = await _create_test_session(db_sessionmaker)
        agent = await store.create_agent("a", "d", None, session_id=session_id)
        await store.deregister_agent(agent["agent_id"])

        result = await store.list_deregistered_agents_with_tasks(session_id)
        assert result == []

    async def test_includes_deregistered_with_tasks(
        self, store, db_engine, db_sessionmaker
    ):
        """The matching case: deregistered AND has at least one task."""
        session_id = await _create_test_session(db_sessionmaker)
        agent = await store.create_agent("a", "d", None, session_id=session_id)
        await _seed_task_for_agent(db_engine, agent_id=agent["agent_id"])
        await store.deregister_agent(agent["agent_id"])

        result = await store.list_deregistered_agents_with_tasks(session_id)
        assert len(result) == 1
        assert result[0]["agent_id"] == agent["agent_id"]
        assert result[0]["name"] == "a"


# ---------------------------------------------------------------------------
# list_placements_for_director (session-scoped)
# ---------------------------------------------------------------------------


class TestListPlacementsForDirector:
    """Tests for ``RegistryStore.list_placements_for_director``.

    Uses ``session_id`` parameter (renamed from ``tenant_id``).
    """

    async def test_list_placements_for_director_session_scoped(
        self, store, db_sessionmaker
    ):
        """Only agents in the specified session whose
        ``placement.director_agent_id`` matches are returned."""
        from hikyaku.models import PlacementCreate

        session_id = await _create_test_session(db_sessionmaker)
        dir_a = await store.create_agent("Dir-A", "d", None, session_id=session_id)
        dir_b = await store.create_agent("Dir-B", "d", None, session_id=session_id)

        await store.create_agent_with_placement(
            name="A-Member-1",
            description="d",
            skills=None,
            session_id=session_id,
            placement=PlacementCreate(
                director_agent_id=dir_a["agent_id"],
                tmux_session="main",
                tmux_window_id="@1",
                tmux_pane_id="%1",
            ),
        )
        await store.create_agent_with_placement(
            name="A-Member-2",
            description="d",
            skills=None,
            session_id=session_id,
            placement=PlacementCreate(
                director_agent_id=dir_a["agent_id"],
                tmux_session="main",
                tmux_window_id="@1",
                tmux_pane_id="%2",
            ),
        )

        await store.create_agent_with_placement(
            name="B-Member-1",
            description="d",
            skills=None,
            session_id=session_id,
            placement=PlacementCreate(
                director_agent_id=dir_b["agent_id"],
                tmux_session="main",
                tmux_window_id="@2",
                tmux_pane_id="%3",
            ),
        )

        members_a = await store.list_placements_for_director(
            session_id=session_id, director_agent_id=dir_a["agent_id"]
        )
        names_a = {m["name"] for m in members_a}
        assert names_a == {"A-Member-1", "A-Member-2"}

        members_b = await store.list_placements_for_director(
            session_id=session_id, director_agent_id=dir_b["agent_id"]
        )
        names_b = {m["name"] for m in members_b}
        assert names_b == {"B-Member-1"}

    async def test_cross_session_isolation(self, store, db_sessionmaker):
        """Members in session B are not visible when querying session A."""
        from hikyaku.models import PlacementCreate

        session_a = await _create_test_session(db_sessionmaker)
        session_b = await _create_test_session(db_sessionmaker)

        dir_a = await store.create_agent("Dir-A", "d", None, session_id=session_a)
        dir_b = await store.create_agent("Dir-B", "d", None, session_id=session_b)

        await store.create_agent_with_placement(
            name="A-Member",
            description="d",
            skills=None,
            session_id=session_a,
            placement=PlacementCreate(
                director_agent_id=dir_a["agent_id"],
                tmux_session="main",
                tmux_window_id="@1",
                tmux_pane_id="%1",
            ),
        )
        await store.create_agent_with_placement(
            name="B-Member",
            description="d",
            skills=None,
            session_id=session_b,
            placement=PlacementCreate(
                director_agent_id=dir_b["agent_id"],
                tmux_session="main",
                tmux_window_id="@2",
                tmux_pane_id="%2",
            ),
        )

        result = await store.list_placements_for_director(
            session_id=session_a, director_agent_id=dir_b["agent_id"]
        )
        assert result == []

        result = await store.list_placements_for_director(
            session_id=session_a, director_agent_id=dir_a["agent_id"]
        )
        assert len(result) == 1
        assert result[0]["name"] == "A-Member"

    async def test_empty_when_no_members(self, store, db_sessionmaker):
        """A director with no members returns an empty list."""
        session_id = await _create_test_session(db_sessionmaker)
        director = await store.create_agent("Dir", "d", None, session_id=session_id)

        result = await store.list_placements_for_director(
            session_id=session_id, director_agent_id=director["agent_id"]
        )
        assert result == []

    async def test_excludes_deregistered_members(self, store, db_sessionmaker):
        """Deregistered members do not appear in the list."""
        from hikyaku.models import PlacementCreate

        session_id = await _create_test_session(db_sessionmaker)
        director = await store.create_agent("Dir", "d", None, session_id=session_id)

        await store.create_agent_with_placement(
            name="Active",
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
        m2 = await store.create_agent_with_placement(
            name="Gone",
            description="d",
            skills=None,
            session_id=session_id,
            placement=PlacementCreate(
                director_agent_id=director["agent_id"],
                tmux_session="main",
                tmux_window_id="@1",
                tmux_pane_id="%2",
            ),
        )

        await store.deregister_agent(m2["agent_id"])

        result = await store.list_placements_for_director(
            session_id=session_id, director_agent_id=director["agent_id"]
        )
        assert len(result) == 1
        assert result[0]["name"] == "Active"


# ---------------------------------------------------------------------------
# Deleted methods — verify they no longer exist
# ---------------------------------------------------------------------------


class TestDeletedApiKeyMethods:
    """Verify that API key methods are removed from RegistryStore.

    Design doc 0000015 Step 4: these methods are deleted entirely.
    """

    def test_create_api_key_removed(self, store):
        assert not hasattr(store, "create_api_key")

    def test_list_api_keys_removed(self, store):
        assert not hasattr(store, "list_api_keys")

    def test_revoke_api_key_removed(self, store):
        assert not hasattr(store, "revoke_api_key")

    def test_get_api_key_status_removed(self, store):
        assert not hasattr(store, "get_api_key_status")

    def test_is_api_key_active_removed(self, store):
        assert not hasattr(store, "is_api_key_active")

    def test_is_key_owner_removed(self, store):
        assert not hasattr(store, "is_key_owner")

    def test_verify_agent_tenant_removed(self, store):
        """Old name verify_agent_tenant must be gone (renamed to verify_agent_session)."""
        assert not hasattr(store, "verify_agent_tenant")
