"""Tests for ``broker`` session + registry operations.

Per principle (ii)/(iii) of design 0000061: per-key projection chains collapse
into per-API parametrized "shape + behaviour" pairs.
"""

import json
import uuid

import click
import pytest

from cafleet import broker
from cafleet.broker import ADMINISTRATOR_KIND
from cafleet.db.models import Agent
from cafleet.db.models import Session as SessionModel
from tests._broker_helpers import _create_session, _register_agent


@pytest.fixture(autouse=True)
def _autouse_broker(broker_session):
    return broker_session


# --- create_session ------------------------------------------------------


def test_create_session__shape_and_label_handling():
    r_default = _create_session()
    assert {"session_id", "label", "created_at"}.issubset(r_default.keys())
    uuid.UUID(r_default["session_id"])
    assert r_default["label"] is None
    assert "T" in r_default["created_at"]

    r_labeled = _create_session(label="PR-42 review")
    assert r_labeled["label"] == "PR-42 review"
    assert r_labeled["session_id"] != r_default["session_id"]


def test_create_session__administrator_seed_shape_and_uniqueness(broker_session):
    r1 = _create_session()
    sid = r1["session_id"]
    admin_id = r1["administrator_agent_id"]
    uuid.UUID(admin_id)

    with broker_session() as s:
        rows = (
            s.query(Agent)
            .filter(Agent.session_id == sid, Agent.status == "active")
            .all()
        )
    assert len(rows) == 2
    admins = [r for r in rows if r.name == "Administrator"]
    assert len(admins) == 1
    assert admins[0].agent_id == admin_id
    card = json.loads(admins[0].agent_card_json)
    assert card["cafleet"]["kind"] == ADMINISTRATOR_KIND

    # Administrator registered_at matches sessions.created_at.
    with broker_session() as s:
        session_row = s.query(SessionModel).filter(SessionModel.session_id == sid).one()
        agent_row = s.query(Agent).filter(Agent.agent_id == admin_id).one()
    assert agent_row.registered_at == session_row.created_at

    # Each session mints its own Administrator.
    r2 = _create_session()
    assert r2["administrator_agent_id"] != admin_id


def test_create_session__list_session_agents_marks_administrator_kind():
    result = _create_session()
    sid = result["session_id"]
    _register_agent(sid, name="user-agent")
    entries = broker.list_session_agents(sid)
    assert len(entries) == 3
    admin_entries = [e for e in entries if e["kind"] == ADMINISTRATOR_KIND]
    user_entries = [e for e in entries if e["kind"] == "user"]
    assert len(admin_entries) == 1
    assert admin_entries[0]["name"] == "Administrator"
    assert admin_entries[0]["agent_id"] == result["administrator_agent_id"]
    assert {e["name"] for e in user_entries} == {"Director", "user-agent"}


# --- list_sessions -------------------------------------------------------


def test_list_sessions__empty_and_non_empty_with_agent_count():
    assert broker.list_sessions() == []

    session = _create_session(label="session-a")
    sid = session["session_id"]
    _register_agent(sid, name="agent-1")
    _register_agent(sid, name="agent-2")
    dead = _register_agent(sid, name="dead-agent")
    broker.deregister_agent(dead["agent_id"])

    rows = broker.list_sessions()
    assert len(rows) == 1
    row = rows[0]
    assert set(row.keys()) >= {"session_id", "label", "created_at", "agent_count"}
    assert row["label"] == "session-a"
    # Two user agents (one deregistered → excluded) + Director + Administrator.
    assert row["agent_count"] == 4


def test_list_sessions__bootstrap_only_count_is_two():
    _create_session()
    rows = broker.list_sessions()
    assert rows[0]["agent_count"] == 2


# --- get_session ---------------------------------------------------------


def test_get_session__existing_and_nonexistent():
    created = _create_session(label="find-me")
    found = broker.get_session(created["session_id"])
    assert found is not None
    assert found["session_id"] == created["session_id"]
    assert found["label"] == "find-me"
    assert "created_at" in found

    assert broker.get_session(str(uuid.uuid4())) is None


# --- register_agent ------------------------------------------------------


def test_register_agent__shape_and_unique_id():
    session = _create_session()
    r1 = _register_agent(session["session_id"], name="a1")
    r2 = _register_agent(session["session_id"], name="a2")
    assert {"agent_id", "name", "registered_at"}.issubset(r1.keys())
    uuid.UUID(r1["agent_id"])
    assert r1["name"] == "a1"
    assert r1["agent_id"] != r2["agent_id"]


@pytest.mark.parametrize(
    ("scenario", "expected_match"),
    [
        ("missing_session", "not found"),
        ("director_not_found", "Director agent"),
        ("director_cross_session", "Director agent"),
        ("director_deregistered", "not active"),
    ],
)
def test_register_agent__validation_failures(scenario, expected_match):
    if scenario == "missing_session":
        with pytest.raises(click.UsageError, match=expected_match):
            broker.register_agent(
                session_id=str(uuid.uuid4()), name="orphan", description="no session",
            )
        return

    session = _create_session()
    sid = session["session_id"]
    if scenario == "director_not_found":
        placement = {
            "director_agent_id": str(uuid.uuid4()),
            "tmux_session": "main", "tmux_window_id": "@1",
            "tmux_pane_id": None, "coding_agent": "claude",
        }
        with pytest.raises(click.UsageError, match=expected_match):
            _register_agent(sid, name="orphan-member", placement=placement)
    elif scenario == "director_cross_session":
        director = _register_agent(sid, name="director")
        session2 = _create_session()
        placement = {
            "director_agent_id": director["agent_id"],
            "tmux_session": "main", "tmux_window_id": "@1",
            "tmux_pane_id": None, "coding_agent": "claude",
        }
        with pytest.raises(click.UsageError, match=expected_match):
            _register_agent(
                session2["session_id"], name="cross-session-member", placement=placement,
            )
    else:  # director_deregistered
        director = _register_agent(sid, name="director")
        broker.deregister_agent(director["agent_id"])
        placement = {
            "director_agent_id": director["agent_id"],
            "tmux_session": "main", "tmux_window_id": "@1",
            "tmux_pane_id": None, "coding_agent": "claude",
        }
        with pytest.raises(click.UsageError, match=expected_match):
            _register_agent(sid, name="late-member", placement=placement)


@pytest.mark.parametrize("with_placement", [True, False])
def test_register_agent__placement_stored_or_absent(with_placement):
    session = _create_session()
    sid = session["session_id"]
    if with_placement:
        director = _register_agent(sid, name="director")
        placement = {
            "director_agent_id": director["agent_id"],
            "tmux_session": "main", "tmux_window_id": "@1",
            "tmux_pane_id": None, "coding_agent": "claude",
        }
        member = _register_agent(sid, name="member", placement=placement)
        fetched = broker.get_agent(member["agent_id"], sid)
        assert fetched["placement"] is not None
        assert fetched["placement"]["director_agent_id"] == director["agent_id"]
        assert fetched["placement"]["tmux_session"] == "main"
    else:
        agent = _register_agent(sid, name="standalone")
        fetched = broker.get_agent(agent["agent_id"], sid)
        assert fetched["placement"] is None


# --- get_agent -----------------------------------------------------------


def test_get_agent__returns_typed_envelope():
    session = _create_session()
    sid = session["session_id"]
    agent = _register_agent(sid, name="visible", description="test desc")
    result = broker.get_agent(agent["agent_id"], sid)
    assert result["agent_id"] == agent["agent_id"]
    assert result["name"] == "visible"
    assert result["description"] == "test desc"
    assert result["status"] == "active"
    assert "registered_at" in result


@pytest.mark.parametrize(
    "scenario",
    ["nonexistent_agent", "deregistered_agent", "wrong_session"],
)
def test_get_agent__returns_none_for_missing(scenario):
    session = _create_session()
    sid = session["session_id"]
    if scenario == "nonexistent_agent":
        assert broker.get_agent(str(uuid.uuid4()), sid) is None
    elif scenario == "deregistered_agent":
        agent = _register_agent(sid, name="temp")
        broker.deregister_agent(agent["agent_id"])
        assert broker.get_agent(agent["agent_id"], sid) is None
    else:
        other = _create_session()
        agent = _register_agent(sid, name="scoped")
        assert broker.get_agent(agent["agent_id"], other["session_id"]) is None


# --- list_agents ---------------------------------------------------------


def test_list_agents__active_only_with_required_keys():
    session = _create_session()
    sid = session["session_id"]
    _register_agent(sid, name="active-1")
    _register_agent(sid, name="active-2")
    dead = _register_agent(sid, name="dead-agent")
    broker.deregister_agent(dead["agent_id"])

    result = broker.list_agents(sid)
    assert len(result) == 4  # 2 user + Director + Administrator
    names = {a["name"] for a in result}
    assert names == {"active-1", "active-2", "Director", "Administrator"}
    agent = result[0]
    assert {"agent_id", "name", "description", "status", "registered_at"}.issubset(
        agent.keys()
    )
    assert agent["status"] == "active"


def test_list_agents__bootstrap_only_lists_director_and_admin():
    session = _create_session()
    result = broker.list_agents(session["session_id"])
    assert {a["name"] for a in result} == {"Director", "Administrator"}


def test_list_agents__scoped_per_session():
    session_a = _create_session()
    session_b = _create_session()
    _register_agent(session_a["session_id"], name="agent-a")
    _register_agent(session_b["session_id"], name="agent-b")
    result_a = broker.list_agents(session_a["session_id"])
    names_a = {a["name"] for a in result_a}
    assert "agent-a" in names_a
    assert "agent-b" not in names_a


# --- verify_agent_session ------------------------------------------------


@pytest.mark.parametrize(
    ("scenario", "expected"),
    [
        ("agent_in_session", True),
        ("agent_in_different_session", False),
        ("nonexistent_agent", False),
    ],
)
def test_verify_agent_session__matrix(scenario, expected):
    session = _create_session()
    sid = session["session_id"]
    if scenario == "agent_in_session":
        agent = _register_agent(sid, name="here")
        assert broker.verify_agent_session(agent["agent_id"], sid) is expected
    elif scenario == "agent_in_different_session":
        other = _create_session()
        agent = _register_agent(sid, name="there")
        assert broker.verify_agent_session(agent["agent_id"], other["session_id"]) is expected
    else:
        assert broker.verify_agent_session(str(uuid.uuid4()), sid) is expected


# --- deregister_agent ----------------------------------------------------


def test_deregister_agent__active_agent_returns_true():
    session = _create_session()
    sid = session["session_id"]
    agent = _register_agent(sid, name="retiring")
    assert broker.deregister_agent(agent["agent_id"]) is True
    names = {a["name"] for a in broker.list_agents(sid)}
    assert names == {"Director", "Administrator"}
    # The deregistered agent still belongs to the session (verify_agent_session).
    assert broker.verify_agent_session(agent["agent_id"], sid) is True


@pytest.mark.parametrize(
    ("scenario", "expected"),
    [("already_deregistered", False), ("nonexistent_agent", False)],
)
def test_deregister_agent__idempotent_and_missing(scenario, expected):
    session = _create_session()
    if scenario == "already_deregistered":
        agent = _register_agent(session["session_id"], name="x")
        broker.deregister_agent(agent["agent_id"])
        assert broker.deregister_agent(agent["agent_id"]) is expected
    else:
        assert broker.deregister_agent(str(uuid.uuid4())) is expected


def test_deregister_agent__deletes_placement():
    session = _create_session()
    sid = session["session_id"]
    director = _register_agent(sid, name="director")
    placement = {
        "director_agent_id": director["agent_id"],
        "tmux_session": "main", "tmux_window_id": "@1",
        "tmux_pane_id": None, "coding_agent": "claude",
    }
    member = _register_agent(sid, name="member", placement=placement)
    broker.deregister_agent(member["agent_id"])
    assert broker.update_placement_pane_id(member["agent_id"], "%99") is None


# --- update_placement_pane_id -------------------------------------------


def test_update_placement_pane_id__updates_and_persists():
    session = _create_session()
    sid = session["session_id"]
    director = _register_agent(sid, name="director")
    placement = {
        "director_agent_id": director["agent_id"],
        "tmux_session": "main", "tmux_window_id": "@1",
        "tmux_pane_id": None, "coding_agent": "claude",
    }
    member = _register_agent(sid, name="member", placement=placement)
    result = broker.update_placement_pane_id(member["agent_id"], "%42")
    assert result["tmux_pane_id"] == "%42"
    fetched = broker.get_agent(member["agent_id"], sid)
    assert fetched["placement"]["tmux_pane_id"] == "%42"


@pytest.mark.parametrize("scenario", ["no_placement", "nonexistent_agent"])
def test_update_placement_pane_id__returns_none_for_missing(scenario):
    session = _create_session()
    if scenario == "no_placement":
        agent = _register_agent(session["session_id"], name="no-placement")
        assert broker.update_placement_pane_id(agent["agent_id"], "%99") is None
    else:
        assert broker.update_placement_pane_id(str(uuid.uuid4()), "%1") is None


# --- list_members --------------------------------------------------------


def test_list_members__returns_members_with_placement_info():
    session = _create_session()
    sid = session["session_id"]
    director = _register_agent(sid, name="director")
    did = director["agent_id"]
    placement = {
        "director_agent_id": did,
        "tmux_session": "main", "tmux_window_id": "@1",
        "tmux_pane_id": None, "coding_agent": "claude",
    }
    _register_agent(sid, name="member-1", placement=placement)
    _register_agent(sid, name="member-2", placement=placement)

    result = broker.list_members(sid, did)
    assert len(result) == 2
    assert {m["name"] for m in result} == {"member-1", "member-2"}
    member = result[0]
    assert "placement" in member
    assert member["placement"]["tmux_session"] == "main"
    assert member["placement"]["director_agent_id"] == did
    assert member["status"] == "active"


def test_list_members__per_director_isolation_and_empty_case():
    session = _create_session()
    sid = session["session_id"]
    dir1 = _register_agent(sid, name="director-1")
    dir2 = _register_agent(sid, name="director-2")
    lonely = _register_agent(sid, name="lonely-director")
    placement1 = {
        "director_agent_id": dir1["agent_id"],
        "tmux_session": "main", "tmux_window_id": "@1",
        "tmux_pane_id": None, "coding_agent": "claude",
    }
    placement2 = {
        "director_agent_id": dir2["agent_id"],
        "tmux_session": "main", "tmux_window_id": "@2",
        "tmux_pane_id": None, "coding_agent": "claude",
    }
    _register_agent(sid, name="m1-of-d1", placement=placement1)
    _register_agent(sid, name="m2-of-d2", placement=placement2)

    result = broker.list_members(sid, dir1["agent_id"])
    assert len(result) == 1
    assert result[0]["name"] == "m1-of-d1"
    assert broker.list_members(sid, lonely["agent_id"]) == []
