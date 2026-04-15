"""Tests for broker.py — Administrator agent helpers and constants.

Design doc 0000025 Step 2: Administrator-related module-level helpers.

This file tests the low-level primitives introduced for the built-in
Administrator agent:

- ``ADMINISTRATOR_KIND`` constant (module-level string)
- ``_administrator_agent_card(session_id)`` helper (dict builder)
- ``_is_administrator_card(agent_card_json)`` helper (JSON-string predicate)
- ``AdministratorProtectedError`` exception class

No database interaction is required for these tests — they exercise pure
helpers in ``cafleet.broker``.
"""

import json
import uuid

import pytest


# ---------------------------------------------------------------------------
# ADMINISTRATOR_KIND constant
# ---------------------------------------------------------------------------


class TestAdministratorKindConstant:
    """Module-level ``ADMINISTRATOR_KIND`` constant in cafleet.broker."""

    def test_constant_exists_and_is_importable(self):
        """ADMINISTRATOR_KIND must be importable directly from cafleet.broker."""
        from cafleet.broker import ADMINISTRATOR_KIND

        assert ADMINISTRATOR_KIND is not None

    def test_constant_value_is_builtin_administrator(self):
        """The canonical kind string is 'builtin-administrator'."""
        from cafleet.broker import ADMINISTRATOR_KIND

        assert ADMINISTRATOR_KIND == "builtin-administrator"

    def test_constant_is_string(self):
        from cafleet.broker import ADMINISTRATOR_KIND

        assert isinstance(ADMINISTRATOR_KIND, str)


# ---------------------------------------------------------------------------
# _administrator_agent_card helper
# ---------------------------------------------------------------------------


class TestAdministratorAgentCard:
    """``_administrator_agent_card(session_id)`` returns canonical card dict."""

    def test_returns_dict(self):
        from cafleet.broker import _administrator_agent_card

        session_id = str(uuid.uuid4())
        card = _administrator_agent_card(session_id)
        assert isinstance(card, dict)

    def test_name_is_administrator(self):
        from cafleet.broker import _administrator_agent_card

        session_id = str(uuid.uuid4())
        card = _administrator_agent_card(session_id)
        assert card["name"] == "Administrator"

    def test_description_contains_session_id_first_8_chars(self):
        """Per design §A, description includes the first 8 chars of session_id."""
        from cafleet.broker import _administrator_agent_card

        session_id = "3f9a1b2c-1234-5678-9abc-def012345678"
        card = _administrator_agent_card(session_id)
        assert "3f9a1b2c" in card["description"]

    def test_description_is_string(self):
        from cafleet.broker import _administrator_agent_card

        session_id = str(uuid.uuid4())
        card = _administrator_agent_card(session_id)
        assert isinstance(card["description"], str)
        assert len(card["description"]) > 0

    def test_skills_is_empty_list(self):
        from cafleet.broker import _administrator_agent_card

        session_id = str(uuid.uuid4())
        card = _administrator_agent_card(session_id)
        assert card["skills"] == []

    def test_cafleet_namespace_kind_matches_constant(self):
        """card['cafleet']['kind'] must equal ADMINISTRATOR_KIND."""
        from cafleet.broker import ADMINISTRATOR_KIND, _administrator_agent_card

        session_id = str(uuid.uuid4())
        card = _administrator_agent_card(session_id)
        assert "cafleet" in card
        assert isinstance(card["cafleet"], dict)
        assert card["cafleet"]["kind"] == ADMINISTRATOR_KIND

    def test_card_is_json_serializable(self):
        """The returned dict must be json.dumps()-able for storage in agent_card_json."""
        from cafleet.broker import _administrator_agent_card

        session_id = str(uuid.uuid4())
        card = _administrator_agent_card(session_id)
        serialized = json.dumps(card)
        assert isinstance(serialized, str)

    def test_different_session_ids_produce_different_descriptions(self):
        """Because the description embeds the session-id prefix."""
        from cafleet.broker import _administrator_agent_card

        sid_a = "aaaaaaaa-1111-1111-1111-111111111111"
        sid_b = "bbbbbbbb-2222-2222-2222-222222222222"
        card_a = _administrator_agent_card(sid_a)
        card_b = _administrator_agent_card(sid_b)
        assert card_a["description"] != card_b["description"]


# ---------------------------------------------------------------------------
# _is_administrator_card helper
# ---------------------------------------------------------------------------


class TestIsAdministratorCard:
    """``_is_administrator_card(agent_card_json)`` JSON-string predicate."""

    def test_returns_true_for_canonical_administrator_card(self):
        """A card produced by _administrator_agent_card must round-trip as True."""
        from cafleet.broker import _administrator_agent_card, _is_administrator_card

        session_id = str(uuid.uuid4())
        card = _administrator_agent_card(session_id)
        card_json = json.dumps(card)
        assert _is_administrator_card(card_json) is True

    def test_returns_true_for_hand_built_administrator_card(self):
        """Any JSON payload with cafleet.kind == 'builtin-administrator' counts."""
        from cafleet.broker import _is_administrator_card

        payload = {
            "name": "Administrator",
            "description": "anything",
            "skills": [],
            "cafleet": {"kind": "builtin-administrator"},
        }
        assert _is_administrator_card(json.dumps(payload)) is True

    def test_returns_false_for_normal_user_card(self):
        """A typical agent card (no 'cafleet' key) must return False."""
        from cafleet.broker import _is_administrator_card

        payload = {
            "name": "Claude-B",
            "description": "Reviewer",
            "skills": [],
        }
        assert _is_administrator_card(json.dumps(payload)) is False

    def test_returns_false_when_cafleet_key_missing(self):
        """Explicitly check a card that has other top-level keys but no 'cafleet'."""
        from cafleet.broker import _is_administrator_card

        payload = {"name": "x", "description": "y", "skills": []}
        assert _is_administrator_card(json.dumps(payload)) is False

    def test_returns_false_when_cafleet_kind_missing(self):
        """'cafleet' present but 'kind' subkey absent → False."""
        from cafleet.broker import _is_administrator_card

        payload = {
            "name": "x",
            "description": "y",
            "skills": [],
            "cafleet": {"other": "value"},
        }
        assert _is_administrator_card(json.dumps(payload)) is False

    def test_returns_false_when_cafleet_kind_is_different_value(self):
        """'cafleet.kind' present but not the administrator sentinel → False."""
        from cafleet.broker import _is_administrator_card

        payload = {
            "name": "x",
            "description": "y",
            "skills": [],
            "cafleet": {"kind": "user"},
        }
        assert _is_administrator_card(json.dumps(payload)) is False

    def test_returns_false_for_malformed_json(self):
        """Non-JSON string input must not raise — returns False."""
        from cafleet.broker import _is_administrator_card

        assert _is_administrator_card("{not valid json") is False

    def test_returns_false_for_empty_string(self):
        from cafleet.broker import _is_administrator_card

        assert _is_administrator_card("") is False

    def test_returns_false_for_json_array(self):
        """Well-formed JSON that is not an object → False (no cafleet.kind path)."""
        from cafleet.broker import _is_administrator_card

        assert _is_administrator_card("[1, 2, 3]") is False

    def test_returns_false_for_json_null(self):
        from cafleet.broker import _is_administrator_card

        assert _is_administrator_card("null") is False


# ---------------------------------------------------------------------------
# AdministratorProtectedError exception class
# ---------------------------------------------------------------------------


class TestAdministratorProtectedError:
    """Exception raised when an operation targets a built-in Administrator."""

    def test_class_is_importable(self):
        from cafleet.broker import AdministratorProtectedError

        assert AdministratorProtectedError is not None

    def test_is_subclass_of_exception(self):
        from cafleet.broker import AdministratorProtectedError

        assert issubclass(AdministratorProtectedError, Exception)

    def test_can_be_raised_and_caught(self):
        from cafleet.broker import AdministratorProtectedError

        with pytest.raises(AdministratorProtectedError):
            raise AdministratorProtectedError("Administrator cannot be deregistered")

    def test_preserves_message(self):
        """Instantiation with a message must round-trip via str()."""
        from cafleet.broker import AdministratorProtectedError

        msg = "Administrator cannot be a director"
        with pytest.raises(AdministratorProtectedError) as exc_info:
            raise AdministratorProtectedError(msg)
        assert msg in str(exc_info.value)
