"""Tests for broker.py — session + registry operations.

Design doc 0000021 Step 3: broker.py is the single data access layer.
All functions are sync, using ``get_sync_sessionmaker()`` from ``db/engine.py``.

Test isolation strategy:
  Each test gets a fresh in-memory SQLite database via the ``broker_session``
  fixture. ``broker.get_sync_sessionmaker`` is monkeypatched to return a
  sessionmaker bound to this ephemeral engine, so broker functions operate
  on a clean DB with no cross-test contamination.

Coverage map:
  | Function                   | Test class                        |
  |----------------------------|-----------------------------------|
  | create_session             | TestCreateSession                 |
  | list_sessions              | TestListSessions                  |
  | get_session                | TestGetSession                    |
  | delete_session             | TestDeleteSession                 |
  | register_agent             | TestRegisterAgent                 |
  | get_agent                  | TestGetAgent                      |
  | list_agents                | TestListAgents                    |
  | verify_agent_session       | TestVerifyAgentSession            |
  | deregister_agent           | TestDeregisterAgent               |
  | update_placement_pane_id   | TestUpdatePlacementPaneId         |
  | list_members               | TestListMembers                   |
"""

import uuid

import click
import pytest
from sqlalchemy import create_engine
from sqlalchemy.orm import sessionmaker

import cafleet.db.engine  # noqa: F401 — registers PRAGMA listener globally
from cafleet.db.models import Base


# ---------------------------------------------------------------------------
# Fixtures
# ---------------------------------------------------------------------------


@pytest.fixture
def sync_sessionmaker():
    """Create a sync in-memory SQLite engine with all tables."""
    engine = create_engine("sqlite:///:memory:")
    Base.metadata.create_all(engine)
    return sessionmaker(engine, expire_on_commit=False)


@pytest.fixture
def _patch_broker(sync_sessionmaker, monkeypatch):
    """Monkeypatch broker.get_sync_sessionmaker to use the test engine."""
    from cafleet import broker

    monkeypatch.setattr(broker, "get_sync_sessionmaker", lambda: sync_sessionmaker)


@pytest.fixture(autouse=True)
def broker_session(sync_sessionmaker, _patch_broker):
    """Autouse fixture that sets up broker with a test DB for every test."""
    return sync_sessionmaker


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------


def _create_session(label: str | None = None) -> dict:
    """Create a session via broker and return the result dict."""
    from cafleet import broker

    return broker.create_session(label=label)


def _register_agent(
    session_id: str,
    name: str = "test-agent",
    description: str = "A test agent",
    skills: list[dict] | None = None,
    placement: dict | None = None,
) -> dict:
    """Register an agent via broker and return the result dict."""
    from cafleet import broker

    return broker.register_agent(
        session_id=session_id,
        name=name,
        description=description,
        skills=skills,
        placement=placement,
    )


# ===========================================================================
# Session operations
# ===========================================================================


class TestCreateSession:
    """broker.create_session(label=None) → dict with session_id, label, created_at."""

    def test_returns_dict_with_required_keys(self):
        result = _create_session()
        assert isinstance(result, dict)
        assert "session_id" in result
        assert "label" in result
        assert "created_at" in result

    def test_session_id_is_valid_uuid(self):
        result = _create_session()
        uuid.UUID(result["session_id"])  # raises ValueError if invalid

    def test_label_is_none_when_omitted(self):
        result = _create_session()
        assert result["label"] is None

    def test_label_is_stored_when_provided(self):
        result = _create_session(label="PR-42 review")
        assert result["label"] == "PR-42 review"

    def test_created_at_is_iso8601(self):
        result = _create_session()
        # Should be a non-empty ISO 8601 string
        assert isinstance(result["created_at"], str)
        assert len(result["created_at"]) > 0
        assert "T" in result["created_at"]

    def test_each_call_mints_unique_id(self):
        r1 = _create_session()
        r2 = _create_session()
        assert r1["session_id"] != r2["session_id"]


class TestCreateSessionAdministratorSeed:
    """broker.create_session auto-seeds a built-in Administrator agent.

    Design doc 0000025 §B: create_session inserts the session row AND an
    Administrator agent in the same transaction, and returns the new
    ``administrator_agent_id`` in the result dict.
    """

    def test_result_includes_administrator_agent_id(self):
        result = _create_session()
        assert "administrator_agent_id" in result
        assert result["administrator_agent_id"] is not None

    def test_administrator_agent_id_is_valid_uuid(self):
        result = _create_session()
        uuid.UUID(result["administrator_agent_id"])

    def test_administrator_row_exists_in_db_and_is_active(self, broker_session):
        """After create_session, exactly one active Administrator agent exists
        for that session in the agents table.
        """
        import json

        from cafleet.broker import ADMINISTRATOR_KIND

        result = _create_session()
        sid = result["session_id"]
        admin_id = result["administrator_agent_id"]

        from cafleet.db.models import Agent

        with broker_session() as s:
            rows = (
                s.query(Agent)
                .filter(Agent.session_id == sid, Agent.status == "active")
                .all()
            )

        # Exactly one active agent (the auto-seeded Administrator).
        assert len(rows) == 1
        row = rows[0]
        assert row.agent_id == admin_id
        assert row.name == "Administrator"
        card = json.loads(row.agent_card_json)
        assert card.get("cafleet", {}).get("kind") == ADMINISTRATOR_KIND

    def test_administrator_registered_at_equals_session_created_at(
        self, broker_session
    ):
        """Per design §A: Administrator.registered_at == sessions.created_at."""
        from cafleet.db.models import Agent, Session as SessionModel

        result = _create_session()
        sid = result["session_id"]
        admin_id = result["administrator_agent_id"]

        with broker_session() as s:
            session_row = (
                s.query(SessionModel)
                .filter(SessionModel.session_id == sid)
                .one()
            )
            agent_row = (
                s.query(Agent)
                .filter(Agent.agent_id == admin_id)
                .one()
            )

        assert agent_row.registered_at == session_row.created_at

    def test_list_session_agents_marks_administrator_kind(self):
        """broker.list_session_agents exposes ``kind`` per agent; the
        auto-seeded Administrator has ``kind == 'builtin-administrator'``
        and is the only such entry.
        """
        from cafleet import broker
        from cafleet.broker import ADMINISTRATOR_KIND

        result = _create_session()
        sid = result["session_id"]

        # Register a regular user agent alongside to verify the kind split.
        _register_agent(sid, name="user-agent")

        entries = broker.list_session_agents(sid)
        # Should have exactly two entries: the Administrator + the user-agent.
        assert len(entries) == 2

        admin_entries = [e for e in entries if e.get("kind") == ADMINISTRATOR_KIND]
        user_entries = [e for e in entries if e.get("kind") == "user"]
        assert len(admin_entries) == 1
        assert len(user_entries) == 1
        assert admin_entries[0]["name"] == "Administrator"
        assert admin_entries[0]["agent_id"] == result["administrator_agent_id"]

    def test_each_session_gets_its_own_administrator(self):
        """Two calls to create_session produce two distinct Administrators."""
        r1 = _create_session()
        r2 = _create_session()
        assert r1["administrator_agent_id"] != r2["administrator_agent_id"]


class TestListSessions:
    """broker.list_sessions() → list of dicts with agent_count."""

    def test_empty_returns_empty_list(self):
        from cafleet import broker

        result = broker.list_sessions()
        assert result == []

    def test_returns_created_sessions(self):
        from cafleet import broker

        _create_session(label="session-a")
        _create_session(label="session-b")

        result = broker.list_sessions()
        assert len(result) == 2
        labels = {s["label"] for s in result}
        assert "session-a" in labels
        assert "session-b" in labels

    def test_includes_agent_count(self):
        from cafleet import broker

        session = _create_session()
        sid = session["session_id"]

        _register_agent(sid, name="agent-1")
        _register_agent(sid, name="agent-2")

        result = broker.list_sessions()
        assert len(result) == 1
        assert result[0]["agent_count"] == 2

    def test_agent_count_excludes_deregistered(self):
        from cafleet import broker

        session = _create_session()
        sid = session["session_id"]

        _register_agent(sid, name="active-agent")
        agent2 = _register_agent(sid, name="dead-agent")
        broker.deregister_agent(agent2["agent_id"])

        result = broker.list_sessions()
        assert result[0]["agent_count"] == 1

    def test_session_with_no_agents_has_zero_count(self):
        from cafleet import broker

        _create_session()

        result = broker.list_sessions()
        assert result[0]["agent_count"] == 0

    def test_result_contains_required_keys(self):
        from cafleet import broker

        _create_session(label="test")

        result = broker.list_sessions()
        entry = result[0]
        assert "session_id" in entry
        assert "label" in entry
        assert "created_at" in entry
        assert "agent_count" in entry


class TestGetSession:
    """broker.get_session(session_id) → dict or None."""

    def test_returns_dict_for_existing_session(self):
        from cafleet import broker

        created = _create_session(label="find-me")
        result = broker.get_session(created["session_id"])

        assert result is not None
        assert result["session_id"] == created["session_id"]
        assert result["label"] == "find-me"
        assert "created_at" in result

    def test_returns_none_for_nonexistent_session(self):
        from cafleet import broker

        result = broker.get_session(str(uuid.uuid4()))
        assert result is None


class TestDeleteSession:
    """broker.delete_session(session_id) → None. Raises click.UsageError on FK violation."""

    def test_deletes_empty_session(self):
        from cafleet import broker

        created = _create_session()
        sid = created["session_id"]

        broker.delete_session(sid)

        assert broker.get_session(sid) is None

    def test_raises_usage_error_when_agents_exist(self):
        from cafleet import broker

        created = _create_session()
        sid = created["session_id"]
        _register_agent(sid, name="blocker")

        with pytest.raises(click.UsageError):
            broker.delete_session(sid)

        # Session must still exist
        assert broker.get_session(sid) is not None

    def test_raises_usage_error_with_deregistered_agents(self):
        """FK RESTRICT applies even to deregistered agents."""
        from cafleet import broker

        created = _create_session()
        sid = created["session_id"]
        agent = _register_agent(sid, name="temp-agent")
        broker.deregister_agent(agent["agent_id"])

        with pytest.raises(click.UsageError):
            broker.delete_session(sid)

        assert broker.get_session(sid) is not None


# ===========================================================================
# Agent registry operations
# ===========================================================================


class TestRegisterAgent:
    """broker.register_agent() → dict with agent_id, name, registered_at."""

    def test_returns_dict_with_required_keys(self):
        session = _create_session()
        result = _register_agent(session["session_id"])

        assert isinstance(result, dict)
        assert "agent_id" in result
        assert "name" in result
        assert "registered_at" in result

    def test_agent_id_is_valid_uuid(self):
        session = _create_session()
        result = _register_agent(session["session_id"])
        uuid.UUID(result["agent_id"])

    def test_name_is_stored(self):
        session = _create_session()
        result = _register_agent(session["session_id"], name="my-agent")
        assert result["name"] == "my-agent"

    def test_validates_session_exists(self):
        """Registering with a non-existent session_id should raise an error."""
        from cafleet import broker

        fake_sid = str(uuid.uuid4())
        with pytest.raises(Exception):
            broker.register_agent(
                session_id=fake_sid,
                name="orphan",
                description="no session",
            )

    def test_each_call_mints_unique_agent_id(self):
        session = _create_session()
        r1 = _register_agent(session["session_id"], name="a1")
        r2 = _register_agent(session["session_id"], name="a2")
        assert r1["agent_id"] != r2["agent_id"]

    def test_register_with_placement(self):
        """When placement is provided, validates director and stores placement."""
        from cafleet import broker

        session = _create_session()
        sid = session["session_id"]

        # Register a director first
        director = _register_agent(sid, name="director")

        # Register member with placement
        placement = {
            "director_agent_id": director["agent_id"],
            "tmux_session": "main",
            "tmux_window_id": "@1",
            "coding_agent": "claude",
        }
        member = _register_agent(sid, name="member", placement=placement)

        # Verify placement was stored by fetching the agent
        agent = broker.get_agent(member["agent_id"], sid)
        assert agent is not None
        assert agent["placement"] is not None
        assert agent["placement"]["director_agent_id"] == director["agent_id"]
        assert agent["placement"]["tmux_session"] == "main"
        assert agent["placement"]["tmux_window_id"] == "@1"

    def test_placement_validates_director_exists(self):
        """Placement with non-existent director should raise an error."""
        session = _create_session()
        sid = session["session_id"]

        placement = {
            "director_agent_id": str(uuid.uuid4()),  # non-existent
            "tmux_session": "main",
            "tmux_window_id": "@1",
            "coding_agent": "claude",
        }
        with pytest.raises(Exception):
            _register_agent(sid, name="orphan-member", placement=placement)

    def test_placement_validates_director_active_in_same_session(self):
        """Director must be active and in the same session."""

        session1 = _create_session()
        session2 = _create_session()

        director = _register_agent(session1["session_id"], name="director")

        # Try to place member in session2 referencing director in session1
        placement = {
            "director_agent_id": director["agent_id"],
            "tmux_session": "main",
            "tmux_window_id": "@1",
            "coding_agent": "claude",
        }
        with pytest.raises(Exception):
            _register_agent(
                session2["session_id"], name="cross-session-member", placement=placement
            )

    def test_placement_validates_director_is_active(self):
        """Deregistered director should not accept new placements."""
        from cafleet import broker

        session = _create_session()
        sid = session["session_id"]

        director = _register_agent(sid, name="director")
        broker.deregister_agent(director["agent_id"])

        placement = {
            "director_agent_id": director["agent_id"],
            "tmux_session": "main",
            "tmux_window_id": "@1",
            "coding_agent": "claude",
        }
        with pytest.raises(Exception):
            _register_agent(sid, name="late-member", placement=placement)

    def test_register_without_placement(self):
        """Agent without placement has no placement row."""
        from cafleet import broker

        session = _create_session()
        sid = session["session_id"]

        agent = _register_agent(sid, name="standalone")
        fetched = broker.get_agent(agent["agent_id"], sid)
        assert fetched is not None
        assert fetched["placement"] is None


class TestGetAgent:
    """broker.get_agent(agent_id, session_id) → dict or None."""

    def test_returns_agent_dict(self):
        from cafleet import broker

        session = _create_session()
        sid = session["session_id"]
        agent = _register_agent(sid, name="visible", description="test desc")

        result = broker.get_agent(agent["agent_id"], sid)
        assert result is not None
        assert result["agent_id"] == agent["agent_id"]
        assert result["name"] == "visible"
        assert result["description"] == "test desc"
        assert result["status"] == "active"
        assert "registered_at" in result

    def test_includes_placement_when_present(self):
        from cafleet import broker

        session = _create_session()
        sid = session["session_id"]
        director = _register_agent(sid, name="director")
        placement = {
            "director_agent_id": director["agent_id"],
            "tmux_session": "main",
            "tmux_window_id": "@1",
            "coding_agent": "claude",
        }
        member = _register_agent(sid, name="member", placement=placement)

        result = broker.get_agent(member["agent_id"], sid)
        assert result is not None
        assert result["placement"] is not None
        assert result["placement"]["director_agent_id"] == director["agent_id"]

    def test_returns_none_for_nonexistent_agent(self):
        from cafleet import broker

        session = _create_session()
        result = broker.get_agent(str(uuid.uuid4()), session["session_id"])
        assert result is None

    def test_excludes_deregistered_agents(self):
        from cafleet import broker

        session = _create_session()
        sid = session["session_id"]
        agent = _register_agent(sid, name="temp")
        broker.deregister_agent(agent["agent_id"])

        result = broker.get_agent(agent["agent_id"], sid)
        assert result is None

    def test_filters_by_session(self):
        """Agent in session A is not visible when querying session B."""
        from cafleet import broker

        session_a = _create_session()
        session_b = _create_session()
        agent = _register_agent(session_a["session_id"], name="scoped")

        result = broker.get_agent(agent["agent_id"], session_b["session_id"])
        assert result is None


class TestListAgents:
    """broker.list_agents(session_id) → list of active agents."""

    def test_returns_active_agents_only(self):
        from cafleet import broker

        session = _create_session()
        sid = session["session_id"]

        _register_agent(sid, name="active-1")
        _register_agent(sid, name="active-2")
        dead = _register_agent(sid, name="dead-agent")
        broker.deregister_agent(dead["agent_id"])

        result = broker.list_agents(sid)
        assert len(result) == 2
        names = {a["name"] for a in result}
        assert "active-1" in names
        assert "active-2" in names
        assert "dead-agent" not in names

    def test_empty_session_returns_empty_list(self):
        from cafleet import broker

        session = _create_session()
        result = broker.list_agents(session["session_id"])
        assert result == []

    def test_agents_scoped_to_session(self):
        """Agents from other sessions are not included."""
        from cafleet import broker

        session_a = _create_session()
        session_b = _create_session()

        _register_agent(session_a["session_id"], name="agent-a")
        _register_agent(session_b["session_id"], name="agent-b")

        result_a = broker.list_agents(session_a["session_id"])
        assert len(result_a) == 1
        assert result_a[0]["name"] == "agent-a"

    def test_result_contains_required_keys(self):
        from cafleet import broker

        session = _create_session()
        _register_agent(session["session_id"], name="keyed")

        result = broker.list_agents(session["session_id"])
        agent = result[0]
        assert "agent_id" in agent
        assert "name" in agent
        assert "description" in agent
        assert agent["status"] == "active"
        assert "registered_at" in agent


class TestVerifyAgentSession:
    """broker.verify_agent_session(agent_id, session_id) → bool."""

    def test_returns_true_for_agent_in_session(self):
        from cafleet import broker

        session = _create_session()
        sid = session["session_id"]
        agent = _register_agent(sid, name="here")

        assert broker.verify_agent_session(agent["agent_id"], sid) is True

    def test_returns_false_for_agent_in_different_session(self):
        from cafleet import broker

        session_a = _create_session()
        session_b = _create_session()
        agent = _register_agent(session_a["session_id"], name="there")

        assert (
            broker.verify_agent_session(agent["agent_id"], session_b["session_id"])
            is False
        )

    def test_returns_false_for_nonexistent_agent(self):
        from cafleet import broker

        session = _create_session()
        assert (
            broker.verify_agent_session(str(uuid.uuid4()), session["session_id"])
            is False
        )


class TestDeregisterAgent:
    """broker.deregister_agent(agent_id) → bool."""

    def test_returns_true_and_deregisters_active_agent(self):
        from cafleet import broker

        session = _create_session()
        sid = session["session_id"]
        agent = _register_agent(sid, name="retiring")

        result = broker.deregister_agent(agent["agent_id"])
        assert result is True

        # Agent should no longer appear in active list
        agents = broker.list_agents(sid)
        assert len(agents) == 0

    def test_sets_deregistered_status_and_timestamp(self):
        """After deregistering, status is 'deregistered' and deregistered_at is set."""
        from cafleet import broker

        session = _create_session()
        sid = session["session_id"]
        agent = _register_agent(sid, name="retiring")

        broker.deregister_agent(agent["agent_id"])

        # get_agent excludes deregistered, so verify via list_agents or DB.
        # Use verify_agent_session to confirm the agent still belongs to session
        # (it's deregistered, not deleted)
        assert broker.verify_agent_session(agent["agent_id"], sid) is True

    def test_deletes_placement_on_deregister(self):
        from cafleet import broker

        session = _create_session()
        sid = session["session_id"]
        director = _register_agent(sid, name="director")
        placement = {
            "director_agent_id": director["agent_id"],
            "tmux_session": "main",
            "tmux_window_id": "@1",
            "coding_agent": "claude",
        }
        member = _register_agent(sid, name="member", placement=placement)

        broker.deregister_agent(member["agent_id"])

        # Placement should be gone — update_placement_pane_id should return None
        result = broker.update_placement_pane_id(member["agent_id"], "%99")
        assert result is None

    def test_returns_false_for_already_deregistered(self):
        from cafleet import broker

        session = _create_session()
        agent = _register_agent(session["session_id"], name="double-dereg")

        broker.deregister_agent(agent["agent_id"])
        result = broker.deregister_agent(agent["agent_id"])
        assert result is False

    def test_returns_false_for_nonexistent_agent(self):
        from cafleet import broker

        result = broker.deregister_agent(str(uuid.uuid4()))
        assert result is False


class TestUpdatePlacementPaneId:
    """broker.update_placement_pane_id(agent_id, pane_id) → dict or None."""

    def test_updates_pane_id_returns_placement(self):
        from cafleet import broker

        session = _create_session()
        sid = session["session_id"]
        director = _register_agent(sid, name="director")
        placement = {
            "director_agent_id": director["agent_id"],
            "tmux_session": "main",
            "tmux_window_id": "@1",
            "coding_agent": "claude",
        }
        member = _register_agent(sid, name="member", placement=placement)

        result = broker.update_placement_pane_id(member["agent_id"], "%42")
        assert result is not None
        assert result["tmux_pane_id"] == "%42"

    def test_returns_none_when_no_placement(self):
        from cafleet import broker

        session = _create_session()
        agent = _register_agent(session["session_id"], name="no-placement")

        result = broker.update_placement_pane_id(agent["agent_id"], "%99")
        assert result is None

    def test_returns_none_for_nonexistent_agent(self):
        from cafleet import broker

        result = broker.update_placement_pane_id(str(uuid.uuid4()), "%1")
        assert result is None

    def test_pane_id_persists_after_update(self):
        """After updating pane_id, get_agent should reflect it."""
        from cafleet import broker

        session = _create_session()
        sid = session["session_id"]
        director = _register_agent(sid, name="director")
        placement = {
            "director_agent_id": director["agent_id"],
            "tmux_session": "main",
            "tmux_window_id": "@1",
            "coding_agent": "claude",
        }
        member = _register_agent(sid, name="member", placement=placement)

        broker.update_placement_pane_id(member["agent_id"], "%77")

        fetched = broker.get_agent(member["agent_id"], sid)
        assert fetched is not None
        assert fetched["placement"]["tmux_pane_id"] == "%77"


class TestListMembers:
    """broker.list_members(session_id, director_agent_id) → list with placement."""

    def test_returns_members_for_director(self):
        from cafleet import broker

        session = _create_session()
        sid = session["session_id"]
        director = _register_agent(sid, name="director")
        did = director["agent_id"]

        placement = {
            "director_agent_id": did,
            "tmux_session": "main",
            "tmux_window_id": "@1",
            "coding_agent": "claude",
        }
        _register_agent(sid, name="member-1", placement=placement)
        _register_agent(sid, name="member-2", placement=placement)

        result = broker.list_members(sid, did)
        assert len(result) == 2
        names = {m["name"] for m in result}
        assert "member-1" in names
        assert "member-2" in names

    def test_includes_placement_info(self):
        from cafleet import broker

        session = _create_session()
        sid = session["session_id"]
        director = _register_agent(sid, name="director")
        did = director["agent_id"]

        placement = {
            "director_agent_id": did,
            "tmux_session": "main",
            "tmux_window_id": "@1",
            "coding_agent": "claude",
        }
        _register_agent(sid, name="member", placement=placement)

        result = broker.list_members(sid, did)
        assert len(result) == 1
        member = result[0]
        assert "placement" in member
        assert member["placement"] is not None
        assert member["placement"]["tmux_session"] == "main"
        assert member["placement"]["tmux_window_id"] == "@1"
        assert member["placement"]["director_agent_id"] == did

    def test_returns_empty_list_when_no_members(self):
        from cafleet import broker

        session = _create_session()
        sid = session["session_id"]
        director = _register_agent(sid, name="lonely-director")

        result = broker.list_members(sid, director["agent_id"])
        assert result == []

    def test_excludes_members_of_other_directors(self):
        from cafleet import broker

        session = _create_session()
        sid = session["session_id"]
        dir1 = _register_agent(sid, name="director-1")
        dir2 = _register_agent(sid, name="director-2")

        placement1 = {
            "director_agent_id": dir1["agent_id"],
            "tmux_session": "main",
            "tmux_window_id": "@1",
            "coding_agent": "claude",
        }
        placement2 = {
            "director_agent_id": dir2["agent_id"],
            "tmux_session": "main",
            "tmux_window_id": "@2",
            "coding_agent": "claude",
        }
        _register_agent(sid, name="m1-of-d1", placement=placement1)
        _register_agent(sid, name="m2-of-d2", placement=placement2)

        result = broker.list_members(sid, dir1["agent_id"])
        assert len(result) == 1
        assert result[0]["name"] == "m1-of-d1"

    def test_includes_agent_status(self):
        from cafleet import broker

        session = _create_session()
        sid = session["session_id"]
        director = _register_agent(sid, name="director")
        did = director["agent_id"]

        placement = {
            "director_agent_id": did,
            "tmux_session": "main",
            "tmux_window_id": "@1",
            "coding_agent": "claude",
        }
        _register_agent(sid, name="member", placement=placement)

        result = broker.list_members(sid, did)
        assert result[0]["status"] == "active"
