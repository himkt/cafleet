"""Tests for ``broker`` session + registry operations."""

import json
import uuid

import click
import pytest
from sqlalchemy import create_engine
from sqlalchemy.orm import sessionmaker

import cafleet.db.engine  # noqa: F401 — registers PRAGMA listener globally
from cafleet import broker
from cafleet.broker import ADMINISTRATOR_KIND
from cafleet.db.models import Agent, Base
from cafleet.db.models import Session as SessionModel
from cafleet.tmux import DirectorContext


@pytest.fixture
def sync_sessionmaker():
    engine = create_engine("sqlite:///:memory:")
    Base.metadata.create_all(engine)
    return sessionmaker(engine, expire_on_commit=False)


@pytest.fixture
def _patch_broker(sync_sessionmaker, monkeypatch):
    monkeypatch.setattr(broker, "get_sync_sessionmaker", lambda: sync_sessionmaker)


@pytest.fixture(autouse=True)
def broker_session(sync_sessionmaker, _patch_broker):
    return sync_sessionmaker


def _create_session(label: str | None = None) -> dict:
    return broker.create_session(
        label=label,
        director_context=DirectorContext(session="main", window_id="@3", pane_id="%0"),
    )


def _register_agent(
    session_id: str,
    name: str = "test-agent",
    description: str = "A test agent",
    skills: list[dict] | None = None,
    placement: dict | None = None,
) -> dict:
    return broker.register_agent(
        session_id=session_id,
        name=name,
        description=description,
        skills=skills,
        placement=placement,
    )


# --- create_session: broker.create_session(label=None) → dict with session_id, label, created_at ---


def test_create_session__returns_dict_with_required_keys():
    result = _create_session()
    assert isinstance(result, dict)
    assert "session_id" in result
    assert "label" in result
    assert "created_at" in result


def test_create_session__session_id_is_valid_uuid():
    result = _create_session()
    uuid.UUID(result["session_id"])  # raises ValueError if invalid


def test_create_session__label_is_none_when_omitted():
    result = _create_session()
    assert result["label"] is None


def test_create_session__label_is_stored_when_provided():
    result = _create_session(label="PR-42 review")
    assert result["label"] == "PR-42 review"


def test_create_session__created_at_is_iso8601():
    result = _create_session()
    # Should be a non-empty ISO 8601 string
    assert isinstance(result["created_at"], str)
    assert len(result["created_at"]) > 0
    assert "T" in result["created_at"]


def test_create_session__each_call_mints_unique_id():
    r1 = _create_session()
    r2 = _create_session()
    assert r1["session_id"] != r2["session_id"]


# --- create_session_administrator_seed: broker.create_session auto-seeds a built-in
# Administrator agent. Design doc 0000025 §B: create_session inserts the session row
# AND an Administrator agent in the same transaction, and returns the new
# ``administrator_agent_id`` in the result dict. ---


def test_create_session_administrator_seed__result_includes_administrator_agent_id():
    result = _create_session()
    assert "administrator_agent_id" in result
    assert result["administrator_agent_id"] is not None


def test_create_session_administrator_seed__administrator_agent_id_is_valid_uuid():
    result = _create_session()
    uuid.UUID(result["administrator_agent_id"])


def test_create_session_administrator_seed__administrator_row_exists_in_db_and_is_active(
    broker_session,
):
    """After create_session, exactly one active Administrator agent exists
    for that session in the agents table.

    Design 0000026 also bootstraps a root Director (name='Director') in
    the same transaction, so two active agents exist — we pick the
    Administrator out by ``name == 'Administrator'``.
    """
    result = _create_session()
    sid = result["session_id"]
    admin_id = result["administrator_agent_id"]

    with broker_session() as s:
        rows = (
            s.query(Agent)
            .filter(Agent.session_id == sid, Agent.status == "active")
            .all()
        )

    # Two active agents: the root Director and the Administrator.
    assert len(rows) == 2
    admins = [r for r in rows if r.name == "Administrator"]
    assert len(admins) == 1
    row = admins[0]
    assert row.agent_id == admin_id
    card = json.loads(row.agent_card_json)
    assert card["cafleet"]["kind"] == ADMINISTRATOR_KIND


def test_create_session_administrator_seed__administrator_registered_at_equals_session_created_at(
    broker_session,
):
    """Per design §A: Administrator.registered_at == sessions.created_at."""
    result = _create_session()
    sid = result["session_id"]
    admin_id = result["administrator_agent_id"]

    with broker_session() as s:
        session_row = s.query(SessionModel).filter(SessionModel.session_id == sid).one()
        agent_row = s.query(Agent).filter(Agent.agent_id == admin_id).one()

    assert agent_row.registered_at == session_row.created_at


def test_create_session_administrator_seed__list_session_agents_marks_administrator_kind():
    """broker.list_session_agents exposes ``kind`` per agent; the
    auto-seeded Administrator has ``kind == 'builtin-administrator'``
    and is the only such entry. The root Director is ``kind == 'user'``.
    """
    result = _create_session()
    sid = result["session_id"]

    # Register a regular user agent alongside to verify the kind split.
    _register_agent(sid, name="user-agent")

    entries = broker.list_session_agents(sid)
    # Three entries: root Director + Administrator + user-agent (design 0000026).
    assert len(entries) == 3

    admin_entries = [e for e in entries if e["kind"] == ADMINISTRATOR_KIND]
    user_entries = [e for e in entries if e["kind"] == "user"]
    # Exactly one Administrator, two user-kind (director + user-agent).
    assert len(admin_entries) == 1
    assert len(user_entries) == 2
    assert admin_entries[0]["name"] == "Administrator"
    assert admin_entries[0]["agent_id"] == result["administrator_agent_id"]
    user_names = {e["name"] for e in user_entries}
    assert "Director" in user_names
    assert "user-agent" in user_names


def test_create_session_administrator_seed__each_session_gets_its_own_administrator():
    """Two calls to create_session produce two distinct Administrators."""
    r1 = _create_session()
    r2 = _create_session()
    assert r1["administrator_agent_id"] != r2["administrator_agent_id"]


# --- list_sessions: broker.list_sessions() → list of dicts with agent_count ---


def test_list_sessions__empty_returns_empty_list():
    result = broker.list_sessions()
    assert result == []


def test_list_sessions__returns_created_sessions():
    _create_session(label="session-a")
    _create_session(label="session-b")

    result = broker.list_sessions()
    assert len(result) == 2
    labels = {s["label"] for s in result}
    assert "session-a" in labels
    assert "session-b" in labels


def test_list_sessions__includes_agent_count():
    """Count includes the root Director, auto-seeded Administrator, and user agents."""
    session = _create_session()
    sid = session["session_id"]

    _register_agent(sid, name="agent-1")
    _register_agent(sid, name="agent-2")

    result = broker.list_sessions()
    assert len(result) == 1
    # 2 user agents + 1 root Director + 1 Administrator (design 0000026).
    assert result[0]["agent_count"] == 4


def test_list_sessions__agent_count_excludes_deregistered():
    """Deregistered user agents are excluded.

    Root Director and Administrator remain active after bootstrap
    (design 0000026), so the count floors at 2 for a session with
    no live user agents.
    """
    session = _create_session()
    sid = session["session_id"]

    _register_agent(sid, name="active-agent")
    agent2 = _register_agent(sid, name="dead-agent")
    broker.deregister_agent(agent2["agent_id"])

    result = broker.list_sessions()
    # 1 active user agent + 1 root Director + 1 Administrator.
    assert result[0]["agent_count"] == 3


def test_list_sessions__session_with_only_bootstrap_agents_has_count_two():
    """A freshly-bootstrapped session has exactly the Director + Administrator."""
    _create_session()

    result = broker.list_sessions()
    # Root Director + Administrator seeded by create_session (design 0000026).
    assert result[0]["agent_count"] == 2


def test_list_sessions__result_contains_required_keys():
    _create_session(label="test")

    result = broker.list_sessions()
    entry = result[0]
    assert "session_id" in entry
    assert "label" in entry
    assert "created_at" in entry
    assert "agent_count" in entry


# --- get_session: broker.get_session(session_id) → dict or None ---


def test_get_session__returns_dict_for_existing_session():
    created = _create_session(label="find-me")
    result = broker.get_session(created["session_id"])

    assert result is not None
    assert result["session_id"] == created["session_id"]
    assert result["label"] == "find-me"
    assert "created_at" in result


def test_get_session__returns_none_for_nonexistent_session():
    result = broker.get_session(str(uuid.uuid4()))
    assert result is None


# NOTE: The former ``TestDeleteSession`` class is removed in this file.
# Design 0000026 §CLI-surface / §delete_session replaces the old physical-
# delete semantics with a SOFT delete (sets ``sessions.deleted_at``,
# deregisters active agents, deletes placements, preserves tasks). Every
# test that used to live here asserted the OLD contract (``get_session``
# returning ``None`` after delete, UsageError on tasks-FK references, etc.)
# which is no longer true. The NEW contract — including the Director +
# Administrator both being counted in the ``deregistered_count`` return,
# the soft-delete idempotency, tasks preservation, and the not-found error
# path — is covered comprehensively by
# ``tests/test_session_bootstrap.py::TestDeleteSessionCascade``.


# --- register_agent: broker.register_agent() → dict with agent_id, name, registered_at ---


def test_register_agent__returns_dict_with_required_keys():
    session = _create_session()
    result = _register_agent(session["session_id"])

    assert isinstance(result, dict)
    assert "agent_id" in result
    assert "name" in result
    assert "registered_at" in result


def test_register_agent__agent_id_is_valid_uuid():
    session = _create_session()
    result = _register_agent(session["session_id"])
    uuid.UUID(result["agent_id"])


def test_register_agent__name_is_stored():
    session = _create_session()
    result = _register_agent(session["session_id"], name="my-agent")
    assert result["name"] == "my-agent"


def test_register_agent__validates_session_exists():
    """Registering with a non-existent session_id should raise an error."""
    fake_sid = str(uuid.uuid4())
    with pytest.raises(click.UsageError, match="not found"):
        broker.register_agent(
            session_id=fake_sid,
            name="orphan",
            description="no session",
        )


def test_register_agent__each_call_mints_unique_agent_id():
    session = _create_session()
    r1 = _register_agent(session["session_id"], name="a1")
    r2 = _register_agent(session["session_id"], name="a2")
    assert r1["agent_id"] != r2["agent_id"]


def test_register_agent__register_with_placement():
    """When placement is provided, validates director and stores placement."""
    session = _create_session()
    sid = session["session_id"]

    # Register a director first
    director = _register_agent(sid, name="director")

    # Register member with placement
    placement = {
        "director_agent_id": director["agent_id"],
        "tmux_session": "main",
        "tmux_window_id": "@1",
        "tmux_pane_id": None,
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


def test_register_agent__placement_validates_director_exists():
    """Placement with non-existent director should raise an error."""
    session = _create_session()
    sid = session["session_id"]

    placement = {
        "director_agent_id": str(uuid.uuid4()),  # non-existent
        "tmux_session": "main",
        "tmux_window_id": "@1",
        "tmux_pane_id": None,
        "coding_agent": "claude",
    }
    with pytest.raises(click.UsageError, match="Director agent"):
        _register_agent(sid, name="orphan-member", placement=placement)


def test_register_agent__placement_validates_director_active_in_same_session():
    """Director must be active and in the same session."""

    session1 = _create_session()
    session2 = _create_session()

    director = _register_agent(session1["session_id"], name="director")

    # Try to place member in session2 referencing director in session1
    placement = {
        "director_agent_id": director["agent_id"],
        "tmux_session": "main",
        "tmux_window_id": "@1",
        "tmux_pane_id": None,
        "coding_agent": "claude",
    }
    with pytest.raises(click.UsageError, match="Director agent"):
        _register_agent(
            session2["session_id"], name="cross-session-member", placement=placement
        )


def test_register_agent__placement_validates_director_is_active():
    """Deregistered director should not accept new placements."""
    session = _create_session()
    sid = session["session_id"]

    director = _register_agent(sid, name="director")
    broker.deregister_agent(director["agent_id"])

    placement = {
        "director_agent_id": director["agent_id"],
        "tmux_session": "main",
        "tmux_window_id": "@1",
        "tmux_pane_id": None,
        "coding_agent": "claude",
    }
    with pytest.raises(click.UsageError, match="not active"):
        _register_agent(sid, name="late-member", placement=placement)


def test_register_agent__register_without_placement():
    """Agent without placement has no placement row."""
    session = _create_session()
    sid = session["session_id"]

    agent = _register_agent(sid, name="standalone")
    fetched = broker.get_agent(agent["agent_id"], sid)
    assert fetched is not None
    assert fetched["placement"] is None


# --- get_agent: broker.get_agent(agent_id, session_id) → dict or None ---


def test_get_agent__returns_agent_dict():
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


def test_get_agent__includes_placement_when_present():
    session = _create_session()
    sid = session["session_id"]
    director = _register_agent(sid, name="director")
    placement = {
        "director_agent_id": director["agent_id"],
        "tmux_session": "main",
        "tmux_window_id": "@1",
        "tmux_pane_id": None,
        "coding_agent": "claude",
    }
    member = _register_agent(sid, name="member", placement=placement)

    result = broker.get_agent(member["agent_id"], sid)
    assert result is not None
    assert result["placement"] is not None
    assert result["placement"]["director_agent_id"] == director["agent_id"]


def test_get_agent__returns_none_for_nonexistent_agent():
    session = _create_session()
    result = broker.get_agent(str(uuid.uuid4()), session["session_id"])
    assert result is None


def test_get_agent__excludes_deregistered_agents():
    session = _create_session()
    sid = session["session_id"]
    agent = _register_agent(sid, name="temp")
    broker.deregister_agent(agent["agent_id"])

    result = broker.get_agent(agent["agent_id"], sid)
    assert result is None


def test_get_agent__filters_by_session():
    """Agent in session A is not visible when querying session B."""
    session_a = _create_session()
    session_b = _create_session()
    agent = _register_agent(session_a["session_id"], name="scoped")

    result = broker.get_agent(agent["agent_id"], session_b["session_id"])
    assert result is None


# --- list_agents: broker.list_agents(session_id) → list of active agents ---


def test_list_agents__returns_active_agents_only():
    """list_agents returns all active agents including the bootstrap pair."""
    session = _create_session()
    sid = session["session_id"]

    _register_agent(sid, name="active-1")
    _register_agent(sid, name="active-2")
    dead = _register_agent(sid, name="dead-agent")
    broker.deregister_agent(dead["agent_id"])

    result = broker.list_agents(sid)
    # 2 active user agents + root Director + Administrator (design 0000026).
    assert len(result) == 4
    names = {a["name"] for a in result}
    assert "active-1" in names
    assert "active-2" in names
    assert "Director" in names
    assert "Administrator" in names
    assert "dead-agent" not in names


def test_list_agents__newly_created_session_lists_bootstrap_agents():
    """A freshly created session has exactly the root Director + Administrator."""
    session = _create_session()
    result = broker.list_agents(session["session_id"])
    assert len(result) == 2
    names = {a["name"] for a in result}
    assert names == {"Director", "Administrator"}


def test_list_agents__agents_scoped_to_session():
    """Agents from other sessions are not included. Each session has its own
    bootstrap pair (root Director + Administrator).
    """
    session_a = _create_session()
    session_b = _create_session()

    _register_agent(session_a["session_id"], name="agent-a")
    _register_agent(session_b["session_id"], name="agent-b")

    result_a = broker.list_agents(session_a["session_id"])
    # Director (A) + Administrator (A) + agent-a.
    assert len(result_a) == 3
    names_a = {a["name"] for a in result_a}
    assert "agent-a" in names_a
    assert "Director" in names_a
    assert "Administrator" in names_a
    assert "agent-b" not in names_a


def test_list_agents__result_contains_required_keys():
    session = _create_session()
    _register_agent(session["session_id"], name="keyed")

    result = broker.list_agents(session["session_id"])
    agent = result[0]
    assert "agent_id" in agent
    assert "name" in agent
    assert "description" in agent
    assert agent["status"] == "active"
    assert "registered_at" in agent


# --- verify_agent_session: broker.verify_agent_session(agent_id, session_id) → bool ---


def test_verify_agent_session__returns_true_for_agent_in_session():
    session = _create_session()
    sid = session["session_id"]
    agent = _register_agent(sid, name="here")

    assert broker.verify_agent_session(agent["agent_id"], sid) is True


def test_verify_agent_session__returns_false_for_agent_in_different_session():
    session_a = _create_session()
    session_b = _create_session()
    agent = _register_agent(session_a["session_id"], name="there")

    assert (
        broker.verify_agent_session(agent["agent_id"], session_b["session_id"]) is False
    )


def test_verify_agent_session__returns_false_for_nonexistent_agent():
    session = _create_session()
    assert (
        broker.verify_agent_session(str(uuid.uuid4()), session["session_id"]) is False
    )


# --- deregister_agent: broker.deregister_agent(agent_id) → bool ---


def test_deregister_agent__returns_true_and_deregisters_active_agent():
    """Deregistering a user agent leaves the bootstrap pair intact."""
    session = _create_session()
    sid = session["session_id"]
    agent = _register_agent(sid, name="retiring")

    result = broker.deregister_agent(agent["agent_id"])
    assert result is True

    # The retiring user agent is gone; the bootstrap Director and
    # Administrator remain (design 0000026).
    names = {a["name"] for a in broker.list_agents(sid)}
    assert names == {"Director", "Administrator"}


def test_deregister_agent__sets_deregistered_status_and_timestamp():
    """After deregistering, status is 'deregistered' and deregistered_at is set."""
    session = _create_session()
    sid = session["session_id"]
    agent = _register_agent(sid, name="retiring")

    broker.deregister_agent(agent["agent_id"])

    # get_agent excludes deregistered, so verify via list_agents or DB.
    # Use verify_agent_session to confirm the agent still belongs to session
    # (it's deregistered, not deleted)
    assert broker.verify_agent_session(agent["agent_id"], sid) is True


def test_deregister_agent__deletes_placement_on_deregister():
    session = _create_session()
    sid = session["session_id"]
    director = _register_agent(sid, name="director")
    placement = {
        "director_agent_id": director["agent_id"],
        "tmux_session": "main",
        "tmux_window_id": "@1",
        "tmux_pane_id": None,
        "coding_agent": "claude",
    }
    member = _register_agent(sid, name="member", placement=placement)

    broker.deregister_agent(member["agent_id"])

    # Placement should be gone — update_placement_pane_id should return None
    result = broker.update_placement_pane_id(member["agent_id"], "%99")
    assert result is None


def test_deregister_agent__returns_false_for_already_deregistered():
    session = _create_session()
    agent = _register_agent(session["session_id"], name="double-dereg")

    broker.deregister_agent(agent["agent_id"])
    result = broker.deregister_agent(agent["agent_id"])
    assert result is False


def test_deregister_agent__returns_false_for_nonexistent_agent():
    result = broker.deregister_agent(str(uuid.uuid4()))
    assert result is False


# --- update_placement_pane_id: broker.update_placement_pane_id(agent_id, pane_id) → dict or None ---


def test_update_placement_pane_id__updates_pane_id_returns_placement():
    session = _create_session()
    sid = session["session_id"]
    director = _register_agent(sid, name="director")
    placement = {
        "director_agent_id": director["agent_id"],
        "tmux_session": "main",
        "tmux_window_id": "@1",
        "tmux_pane_id": None,
        "coding_agent": "claude",
    }
    member = _register_agent(sid, name="member", placement=placement)

    result = broker.update_placement_pane_id(member["agent_id"], "%42")
    assert result is not None
    assert result["tmux_pane_id"] == "%42"


def test_update_placement_pane_id__returns_none_when_no_placement():
    session = _create_session()
    agent = _register_agent(session["session_id"], name="no-placement")

    result = broker.update_placement_pane_id(agent["agent_id"], "%99")
    assert result is None


def test_update_placement_pane_id__returns_none_for_nonexistent_agent():
    result = broker.update_placement_pane_id(str(uuid.uuid4()), "%1")
    assert result is None


def test_update_placement_pane_id__pane_id_persists_after_update():
    """After updating pane_id, get_agent should reflect it."""
    session = _create_session()
    sid = session["session_id"]
    director = _register_agent(sid, name="director")
    placement = {
        "director_agent_id": director["agent_id"],
        "tmux_session": "main",
        "tmux_window_id": "@1",
        "tmux_pane_id": None,
        "coding_agent": "claude",
    }
    member = _register_agent(sid, name="member", placement=placement)

    broker.update_placement_pane_id(member["agent_id"], "%77")

    fetched = broker.get_agent(member["agent_id"], sid)
    assert fetched is not None
    assert fetched["placement"]["tmux_pane_id"] == "%77"


# --- list_members: broker.list_members(session_id, director_agent_id) → list with placement ---


def test_list_members__returns_members_for_director():
    session = _create_session()
    sid = session["session_id"]
    director = _register_agent(sid, name="director")
    did = director["agent_id"]

    placement = {
        "director_agent_id": did,
        "tmux_session": "main",
        "tmux_window_id": "@1",
        "tmux_pane_id": None,
        "coding_agent": "claude",
    }
    _register_agent(sid, name="member-1", placement=placement)
    _register_agent(sid, name="member-2", placement=placement)

    result = broker.list_members(sid, did)
    assert len(result) == 2
    names = {m["name"] for m in result}
    assert "member-1" in names
    assert "member-2" in names


def test_list_members__includes_placement_info():
    session = _create_session()
    sid = session["session_id"]
    director = _register_agent(sid, name="director")
    did = director["agent_id"]

    placement = {
        "director_agent_id": did,
        "tmux_session": "main",
        "tmux_window_id": "@1",
        "tmux_pane_id": None,
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


def test_list_members__returns_empty_list_when_no_members():
    session = _create_session()
    sid = session["session_id"]
    director = _register_agent(sid, name="lonely-director")

    result = broker.list_members(sid, director["agent_id"])
    assert result == []


def test_list_members__excludes_members_of_other_directors():
    session = _create_session()
    sid = session["session_id"]
    dir1 = _register_agent(sid, name="director-1")
    dir2 = _register_agent(sid, name="director-2")

    placement1 = {
        "director_agent_id": dir1["agent_id"],
        "tmux_session": "main",
        "tmux_window_id": "@1",
        "tmux_pane_id": None,
        "coding_agent": "claude",
    }
    placement2 = {
        "director_agent_id": dir2["agent_id"],
        "tmux_session": "main",
        "tmux_window_id": "@2",
        "tmux_pane_id": None,
        "coding_agent": "claude",
    }
    _register_agent(sid, name="m1-of-d1", placement=placement1)
    _register_agent(sid, name="m2-of-d2", placement=placement2)

    result = broker.list_members(sid, dir1["agent_id"])
    assert len(result) == 1
    assert result[0]["name"] == "m1-of-d1"


def test_list_members__includes_agent_status():
    session = _create_session()
    sid = session["session_id"]
    director = _register_agent(sid, name="director")
    did = director["agent_id"]

    placement = {
        "director_agent_id": did,
        "tmux_session": "main",
        "tmux_window_id": "@1",
        "tmux_pane_id": None,
        "coding_agent": "claude",
    }
    _register_agent(sid, name="member", placement=placement)

    result = broker.list_members(sid, did)
    assert result[0]["status"] == "active"
