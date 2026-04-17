"""Tests for the Administrator agent helpers, constants, and broker guards (design 0000025)."""

import json
import uuid

import pytest
from sqlalchemy import create_engine
from sqlalchemy.orm import sessionmaker

import cafleet.db.engine  # noqa: F401 — registers PRAGMA listener globally
from cafleet import broker
from cafleet.broker import (
    ADMINISTRATOR_KIND,
    AdministratorProtectedError,
    _administrator_agent_card,
    _is_administrator_card,
)
from cafleet.db.models import Agent, Base
from cafleet.tmux import DirectorContext

_FAKE_DIRECTOR_CTX = DirectorContext(session="main", window_id="@3", pane_id="%0")


def _create_session_with_ctx():
    return broker.create_session(director_context=_FAKE_DIRECTOR_CTX)


class TestAdministratorKindConstant:
    """Module-level ``ADMINISTRATOR_KIND`` constant in cafleet.broker."""

    def test_constant_exists_and_is_importable(self):
        """ADMINISTRATOR_KIND must be importable directly from cafleet.broker."""
        assert ADMINISTRATOR_KIND is not None

    def test_constant_value_is_builtin_administrator(self):
        """The canonical kind string is 'builtin-administrator'."""
        assert ADMINISTRATOR_KIND == "builtin-administrator"

    def test_constant_is_string(self):
        assert isinstance(ADMINISTRATOR_KIND, str)


class TestAdministratorAgentCard:
    """``_administrator_agent_card(session_id)`` returns canonical card dict."""

    def test_returns_dict(self):
        session_id = str(uuid.uuid4())
        card = _administrator_agent_card(session_id)
        assert isinstance(card, dict)

    def test_name_is_administrator(self):
        session_id = str(uuid.uuid4())
        card = _administrator_agent_card(session_id)
        assert card["name"] == "Administrator"

    def test_description_contains_session_id_first_8_chars(self):
        """Per design §A, description includes the first 8 chars of session_id."""
        session_id = "3f9a1b2c-1234-5678-9abc-def012345678"
        card = _administrator_agent_card(session_id)
        assert "3f9a1b2c" in card["description"]

    def test_description_is_string(self):
        session_id = str(uuid.uuid4())
        card = _administrator_agent_card(session_id)
        assert isinstance(card["description"], str)
        assert len(card["description"]) > 0

    def test_skills_is_empty_list(self):
        session_id = str(uuid.uuid4())
        card = _administrator_agent_card(session_id)
        assert card["skills"] == []

    def test_cafleet_namespace_kind_matches_constant(self):
        """card['cafleet']['kind'] must equal ADMINISTRATOR_KIND."""
        session_id = str(uuid.uuid4())
        card = _administrator_agent_card(session_id)
        assert "cafleet" in card
        assert isinstance(card["cafleet"], dict)
        assert card["cafleet"]["kind"] == ADMINISTRATOR_KIND

    def test_card_is_json_serializable(self):
        """The returned dict must be json.dumps()-able for storage in agent_card_json."""
        session_id = str(uuid.uuid4())
        card = _administrator_agent_card(session_id)
        serialized = json.dumps(card)
        assert isinstance(serialized, str)

    def test_different_session_ids_produce_different_descriptions(self):
        """Because the description embeds the session-id prefix."""
        sid_a = "aaaaaaaa-1111-1111-1111-111111111111"
        sid_b = "bbbbbbbb-2222-2222-2222-222222222222"
        card_a = _administrator_agent_card(sid_a)
        card_b = _administrator_agent_card(sid_b)
        assert card_a["description"] != card_b["description"]


class TestIsAdministratorCard:
    """``_is_administrator_card(agent_card_json)`` JSON-string predicate."""

    def test_returns_true_for_canonical_administrator_card(self):
        """A card produced by _administrator_agent_card must round-trip as True."""
        session_id = str(uuid.uuid4())
        card = _administrator_agent_card(session_id)
        card_json = json.dumps(card)
        assert _is_administrator_card(card_json) is True

    def test_returns_true_for_hand_built_administrator_card(self):
        """Any JSON payload with cafleet.kind == 'builtin-administrator' counts."""
        payload = {
            "name": "Administrator",
            "description": "anything",
            "skills": [],
            "cafleet": {"kind": "builtin-administrator"},
        }
        assert _is_administrator_card(json.dumps(payload)) is True

    def test_returns_false_for_normal_user_card(self):
        """A typical agent card (no 'cafleet' key) must return False."""
        payload = {
            "name": "Claude-B",
            "description": "Reviewer",
            "skills": [],
        }
        assert _is_administrator_card(json.dumps(payload)) is False

    def test_returns_false_when_cafleet_key_missing(self):
        """Explicitly check a card that has other top-level keys but no 'cafleet'."""
        payload = {"name": "x", "description": "y", "skills": []}
        assert _is_administrator_card(json.dumps(payload)) is False

    def test_returns_false_when_cafleet_kind_missing(self):
        """'cafleet' present but 'kind' subkey absent → False."""
        payload = {
            "name": "x",
            "description": "y",
            "skills": [],
            "cafleet": {"other": "value"},
        }
        assert _is_administrator_card(json.dumps(payload)) is False

    def test_returns_false_when_cafleet_kind_is_different_value(self):
        """'cafleet.kind' present but not the administrator sentinel → False."""
        payload = {
            "name": "x",
            "description": "y",
            "skills": [],
            "cafleet": {"kind": "user"},
        }
        assert _is_administrator_card(json.dumps(payload)) is False

    def test_returns_false_for_malformed_json(self):
        """Non-JSON string input must not raise — returns False."""
        assert _is_administrator_card("{not valid json") is False

    def test_returns_false_for_empty_string(self):
        assert _is_administrator_card("") is False

    def test_returns_false_for_json_array(self):
        """Well-formed JSON that is not an object → False (no cafleet.kind path)."""
        assert _is_administrator_card("[1, 2, 3]") is False

    def test_returns_false_for_json_null(self):
        assert _is_administrator_card("null") is False


class TestAdministratorProtectedError:
    """Exception raised when an operation targets a built-in Administrator."""

    def test_class_is_importable(self):
        assert AdministratorProtectedError is not None

    def test_is_subclass_of_exception(self):
        assert issubclass(AdministratorProtectedError, Exception)

    def test_can_be_raised_and_caught(self):
        with pytest.raises(AdministratorProtectedError):
            raise AdministratorProtectedError("Administrator cannot be deregistered")

    def test_preserves_message(self):
        """Instantiation with a message must round-trip via str()."""
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


class TestDeregisterAdministratorGuard:
    """Design doc §D: deregister_agent refuses to touch the Administrator."""

    def test_raises_administrator_protected_error(self, broker_db):
        session = _create_session_with_ctx()
        admin_id = session["administrator_agent_id"]

        with pytest.raises(AdministratorProtectedError) as exc_info:
            broker.deregister_agent(admin_id)

        assert "Administrator cannot be deregistered" in str(exc_info.value)

    def test_admin_row_still_active_after_failed_deregister(self, broker_db):
        """A failed deregister leaves the Administrator's status unchanged."""

        session = _create_session_with_ctx()
        admin_id = session["administrator_agent_id"]

        with pytest.raises(AdministratorProtectedError):
            broker.deregister_agent(admin_id)

        with broker_db() as s:
            row = s.query(Agent).filter(Agent.agent_id == admin_id).one()
        assert row.status == "active"
        assert row.deregistered_at is None

    def test_deregistering_user_agent_still_works(self, broker_db):
        """Regression guard: user agents can still be deregistered normally."""

        session = _create_session_with_ctx()
        sid = session["session_id"]
        user = broker.register_agent(
            session_id=sid, name="user", description="A test user"
        )

        result = broker.deregister_agent(user["agent_id"])
        assert result is True

        # The root Director and Administrator seeded at bootstrap (design
        # 0000026) both remain; only the user agent was deregistered.
        names = {a["name"] for a in broker.list_agents(sid)}
        assert names == {"director", "Administrator"}


class TestRegisterAgentPlacementAdministratorGuard:
    """Design doc §D: the Administrator must never be handed a tmux pane.

    When ``register_agent`` is called with ``placement.director_agent_id``
    pointing at an Administrator, the broker raises
    ``AdministratorProtectedError``.
    """

    def test_raises_when_director_is_administrator(self, broker_db):
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

    def test_admin_director_rejection_does_not_create_member(self, broker_db):
        """After the guard fires, no member agent row exists."""
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
        # Only the bootstrapped agents (Director + Administrator per design
        # 0000026) should remain.
        assert names == {"director", "Administrator"}

    def test_placement_with_user_agent_director_still_works(self, broker_db):
        """Regression guard: normal user-agent directors still accept placements."""

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
