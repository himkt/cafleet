"""Tests for Administrator member helpers, constants, and broker guards."""

import json

import click
import pytest

from cafleet import broker
from cafleet.broker import ADMINISTRATOR_KIND
from cafleet.broker._shared import is_administrator
from cafleet.db.models import Member
from tests.broker._helpers import _create_fleet

_create_fleet_with_ctx = _create_fleet


def test_administrator_kind_constant__value_and_type():
    assert ADMINISTRATOR_KIND is not None
    assert ADMINISTRATOR_KIND == "builtin-administrator"
    assert isinstance(ADMINISTRATOR_KIND, str)


@pytest.mark.parametrize(
    ("scenario", "card_payload", "expected"),
    [
        (
            "canonical_administrator",
            {
                "name": "Administrator",
                "description": "Built-in administrator member",
                "skills": [],
                "cafleet": {"kind": ADMINISTRATOR_KIND},
            },
            True,
        ),
        (
            "hand_built_administrator",
            {
                "name": "Administrator",
                "description": "anything",
                "skills": [],
                "cafleet": {"kind": "builtin-administrator"},
            },
            True,
        ),
        (
            "normal_user_card",
            {"name": "Claude-B", "description": "Reviewer", "skills": []},
            False,
        ),
        (
            "missing_cafleet_key",
            {"name": "x", "description": "y", "skills": []},
            False,
        ),
        (
            "missing_kind_field",
            {
                "name": "x",
                "description": "y",
                "skills": [],
                "cafleet": {"other": "value"},
            },
            False,
        ),
        (
            "kind_is_user",
            {
                "name": "x",
                "description": "y",
                "skills": [],
                "cafleet": {"kind": "user"},
            },
            False,
        ),
    ],
)
def test_is_administrator__matrix(scenario, card_payload, expected):
    assert is_administrator(json.dumps(card_payload)) is expected


@pytest.mark.parametrize(
    ("scenario", "payload"),
    [
        ("malformed_json", "{not valid json"),
        ("empty_string", ""),
        ("none_input", None),
    ],
)
def test_is_administrator__invalid_inputs(scenario, payload):
    assert is_administrator(payload) is False


def test_deregister_administrator__protected_and_user_dereg_still_works(
    broker_session,
):
    fleet = _create_fleet_with_ctx()
    sid = fleet["fleet_id"]
    admin_id = fleet["administrator_member_id"]

    with pytest.raises(click.ClickException) as exc_info:
        broker.deregister_member(admin_id)
    assert "Administrator cannot be deregistered" in str(exc_info.value)

    # State unchanged.
    with broker_session() as s:
        row = s.query(Member).filter(Member.member_id == admin_id).one()
    assert row.status == "active"
    assert row.deregistered_at is None

    # Regular user member can still be deregistered.
    user = broker.register_member(fleet_id=sid, name="user", description="test user")
    assert broker.deregister_member(user["member_id"]) is True
    names = {a["name"] for a in broker.list_roster(sid)}
    assert names == {"Director", "Administrator"}
