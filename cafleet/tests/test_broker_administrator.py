"""Tests for Administrator agent helpers, constants, and broker guards."""

import json

import pytest
from sqlalchemy import create_engine
from sqlalchemy.orm import sessionmaker

import cafleet.db.engine  # noqa: F401 — registers PRAGMA listener globally
from cafleet import broker
from cafleet.broker import (
    ADMINISTRATOR_KIND,
    AdministratorProtectedError,
    _is_administrator,
)
from cafleet.db.models import Agent, Base
from cafleet.tmux import DirectorContext

_FAKE_DIRECTOR_CTX = DirectorContext(session="main", window_id="@3", pane_id="%0")


def _create_session_with_ctx():
    return broker.create_session(director_context=_FAKE_DIRECTOR_CTX)


def test_administrator_kind_constant__constant_exists_and_is_importable():
    assert ADMINISTRATOR_KIND is not None


def test_administrator_kind_constant__constant_value_is_builtin_administrator():
    assert ADMINISTRATOR_KIND == "builtin-administrator"


def test_administrator_kind_constant__constant_is_string():
    assert isinstance(ADMINISTRATOR_KIND, str)


def test_is_administrator__returns_true_for_canonical_administrator_card():
    payload = {
        "name": "Administrator",
        "description": "Built-in administrator agent for session 3f9a1b2c",
        "skills": [],
        "cafleet": {"kind": ADMINISTRATOR_KIND},
    }
    assert _is_administrator(json.dumps(payload)) is True


def test_is_administrator__returns_true_for_hand_built_administrator_card():
    payload = {
        "name": "Administrator",
        "description": "anything",
        "skills": [],
        "cafleet": {"kind": "builtin-administrator"},
    }
    assert _is_administrator(json.dumps(payload)) is True


def test_is_administrator__returns_false_for_normal_user_card():
    payload = {
        "name": "Claude-B",
        "description": "Reviewer",
        "skills": [],
    }
    assert _is_administrator(json.dumps(payload)) is False


def test_is_administrator__returns_false_when_cafleet_key_missing():
    payload = {"name": "x", "description": "y", "skills": []}
    assert _is_administrator(json.dumps(payload)) is False


def test_is_administrator__returns_false_when_cafleet_kind_missing():
    payload = {
        "name": "x",
        "description": "y",
        "skills": [],
        "cafleet": {"other": "value"},
    }
    assert _is_administrator(json.dumps(payload)) is False


def test_is_administrator__returns_false_when_cafleet_kind_is_different_value():
    payload = {
        "name": "x",
        "description": "y",
        "skills": [],
        "cafleet": {"kind": "user"},
    }
    assert _is_administrator(json.dumps(payload)) is False


def test_is_administrator__returns_false_for_malformed_json():
    assert _is_administrator("{not valid json") is False


def test_is_administrator__returns_false_for_empty_string():
    assert _is_administrator("") is False


def test_is_administrator__returns_false_for_none():
    assert _is_administrator(None) is False


def test_administrator_protected_error__class_is_importable():
    assert AdministratorProtectedError is not None


def test_administrator_protected_error__is_subclass_of_exception():
    assert issubclass(AdministratorProtectedError, Exception)


def test_administrator_protected_error__can_be_raised_and_caught():
    with pytest.raises(AdministratorProtectedError):
        raise AdministratorProtectedError("Administrator cannot be deregistered")


def test_administrator_protected_error__preserves_message():
    msg = "Administrator cannot be a director"
    with pytest.raises(AdministratorProtectedError) as exc_info:
        raise AdministratorProtectedError(msg)
    assert msg in str(exc_info.value)


@pytest.fixture
def sync_sessionmaker():
    engine = create_engine("sqlite:///:memory:")
    Base.metadata.create_all(engine)
    return sessionmaker(engine, expire_on_commit=False)


@pytest.fixture
def _patch_broker(sync_sessionmaker, monkeypatch):
    monkeypatch.setattr(broker, "get_sync_sessionmaker", lambda: sync_sessionmaker)


@pytest.fixture
def broker_db(sync_sessionmaker, _patch_broker):
    return sync_sessionmaker


def test_deregister_administrator_guard__raises_administrator_protected_error(broker_db):
    session = _create_session_with_ctx()
    admin_id = session["administrator_agent_id"]

    with pytest.raises(AdministratorProtectedError) as exc_info:
        broker.deregister_agent(admin_id)

    assert "Administrator cannot be deregistered" in str(exc_info.value)


def test_deregister_administrator_guard__admin_row_still_active_after_failed_deregister(broker_db):
    session = _create_session_with_ctx()
    admin_id = session["administrator_agent_id"]

    with pytest.raises(AdministratorProtectedError):
        broker.deregister_agent(admin_id)

    with broker_db() as s:
        row = s.query(Agent).filter(Agent.agent_id == admin_id).one()
    assert row.status == "active"
    assert row.deregistered_at is None


def test_deregister_administrator_guard__deregistering_user_agent_still_works(broker_db):
    session = _create_session_with_ctx()
    sid = session["session_id"]
    user = broker.register_agent(
        session_id=sid, name="user", description="A test user"
    )

    result = broker.deregister_agent(user["agent_id"])
    assert result is True

    names = {a["name"] for a in broker.list_agents(sid)}
    assert names == {"Director", "Administrator"}


def test_register_agent_placement_administrator_guard__raises_when_director_is_administrator(broker_db):
    session = _create_session_with_ctx()
    sid = session["session_id"]
    admin_id = session["administrator_agent_id"]

    placement = {
        "director_agent_id": admin_id,
        "tmux_session": "main",
        "tmux_window_id": "@1",
        "tmux_pane_id": None,
        "coding_agent": "claude",
    }
    with pytest.raises(AdministratorProtectedError) as exc_info:
        broker.register_agent(
            session_id=sid,
            name="member",
            description="member placed under Admin",
            placement=placement,
        )

    assert "Administrator cannot be a director" in str(exc_info.value)


def test_register_agent_placement_administrator_guard__admin_director_rejection_does_not_create_member(broker_db):
    session = _create_session_with_ctx()
    sid = session["session_id"]
    admin_id = session["administrator_agent_id"]

    placement = {
        "director_agent_id": admin_id,
        "tmux_session": "main",
        "tmux_window_id": "@1",
        "tmux_pane_id": None,
        "coding_agent": "claude",
    }
    with pytest.raises(AdministratorProtectedError):
        broker.register_agent(
            session_id=sid,
            name="rejected-member",
            description="should not exist",
            placement=placement,
        )

    names = {a["name"] for a in broker.list_agents(sid)}
    assert "rejected-member" not in names
    assert names == {"Director", "Administrator"}


def test_register_agent_placement_administrator_guard__placement_with_user_agent_director_still_works(broker_db):
    session = _create_session_with_ctx()
    sid = session["session_id"]

    director = broker.register_agent(
        session_id=sid, name="director", description="a user director"
    )

    placement = {
        "director_agent_id": director["agent_id"],
        "tmux_session": "main",
        "tmux_window_id": "@1",
        "tmux_pane_id": None,
        "coding_agent": "claude",
    }
    member = broker.register_agent(
        session_id=sid,
        name="member",
        description="member of a user director",
        placement=placement,
    )

    fetched = broker.get_agent(member["agent_id"], sid)
    assert fetched is not None
    assert fetched["placement"] is not None
    assert fetched["placement"]["director_agent_id"] == director["agent_id"]
