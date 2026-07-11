"""Tests for ``broker`` WebUI query operations."""

import pytest

from cafleet import broker
from tests.broker._helpers import (
    _create_fleet,
    _register_member,
    _setup_three_members,
    _setup_two_members,
)

# --- list_roster (WebUI variant) ----------------------------------------


def test_list_roster__active_shape_required_keys_and_fleet_scope():
    fleet = _create_fleet()
    sid = fleet["fleet_id"]
    _register_member(sid, name="active-1")
    _register_member(sid, name="active-2")

    result = broker.list_roster(sid, include_task_holders=True)
    assert len(result) == 4
    assert {a["name"] for a in result} == {
        "active-1",
        "active-2",
        "Director",
        "Administrator",
    }
    entry = result[0]
    for key in ("member_id", "name", "description", "status", "registered_at"):
        assert key in entry
    assert entry["status"] == "active"

    # Newly-created fleet lists exactly Director + Administrator.
    bare = _create_fleet()
    bare_result = broker.list_roster(bare["fleet_id"], include_task_holders=True)
    assert {a["name"] for a in bare_result} == {"Director", "Administrator"}

    # Scoped per fleet.
    other = _create_fleet()
    _register_member(other["fleet_id"], name="in-other")
    assert "in-other" not in {
        a["name"] for a in broker.list_roster(sid, include_task_holders=True)
    }


def test_list_roster__deregistered_with_or_without_tasks():
    sid, sender, recipient = _setup_two_members()
    broker.send_message(sid, sender, recipient, "keep visible")
    broker.deregister_member(recipient)
    result = broker.list_roster(sid, include_task_holders=True)
    deregistered = [a for a in result if a["member_id"] == recipient]
    assert deregistered[0]["status"] == "deregistered"

    ghost = _register_member(sid, name="ghost")
    broker.deregister_member(ghost["member_id"])
    result = broker.list_roster(sid, include_task_holders=True)
    assert ghost["member_id"] not in {a["member_id"] for a in result}


@pytest.mark.parametrize(
    ("scenario", "expected_kind"),
    [
        ("administrator", "administrator"),
        ("user_member", "member"),
    ],
)
def test_get_member__kind_field(scenario, expected_kind):
    fleet = _create_fleet()
    sid = fleet["fleet_id"]
    if scenario == "administrator":
        member_id = fleet["administrator_member_id"]
    else:
        member_id = _register_member(sid, name="regular")["member_id"]
    result = broker.get_member(member_id, sid)
    assert result["kind"] == expected_kind

    # Every roster entry carries the unified 4-value kind.
    all_entries = broker.list_roster(sid, include_task_holders=True)
    for entry in all_entries:
        assert entry["kind"] in {"director", "administrator", "monitor", "member"}


# --- list_inbox ---------------------------------------------------------


def test_list_inbox__shape_ordering_and_typed_columns():
    sid, sender, recipient = _setup_two_members()
    broker.send_message(sid, sender, recipient, "first")
    broker.send_message(sid, sender, recipient, "second")
    result = broker.list_inbox(recipient)
    assert len(result) == 2
    # Most recent first.
    assert result[0]["status_timestamp"] >= result[1]["status_timestamp"]
    entry = result[0]
    assert "task_id" in entry
    assert "text" in entry
    assert "task_json" not in entry

    # Empty inbox case.
    idle = _register_member(_create_fleet()["fleet_id"], name="idle")
    assert broker.list_inbox(idle["member_id"]) == []


def test_list_inbox__filters_broadcast_summary_and_context_id_scope():
    sid, sender, _b_id, _ = _setup_three_members()
    broker.broadcast_message(sid, sender, "broadcast")
    summaries = [
        t for t in broker.list_inbox(sender) if t["type"] == "broadcast_summary"
    ]
    assert summaries == []

    sid2, sender2, recipient2 = _setup_two_members()
    broker.send_message(sid2, sender2, recipient2, "for-recipient")
    assert broker.list_inbox(sender2) == []


# --- list_sent ----------------------------------------------------------


def test_list_sent__shape_ordering_and_typed_columns():
    sid, sender, recipient = _setup_two_members()
    broker.send_message(sid, sender, recipient, "first")
    broker.send_message(sid, sender, recipient, "second")
    result = broker.list_sent(sender)
    assert len(result) == 2
    assert result[0]["status_timestamp"] >= result[1]["status_timestamp"]
    entry = result[0]
    assert "task_id" in entry
    assert "text" in entry
    assert "task_json" not in entry

    quiet = _register_member(_create_fleet()["fleet_id"], name="quiet")
    assert broker.list_sent(quiet["member_id"]) == []


def test_list_sent__filters_broadcast_summary_and_from_member_scope():
    sid, sender, _b_id, _ = _setup_three_members()
    broker.broadcast_message(sid, sender, "broadcast")
    summaries = [
        t for t in broker.list_sent(sender) if t["type"] == "broadcast_summary"
    ]
    assert summaries == []

    sid2, sender2, recipient2 = _setup_two_members()
    broker.send_message(sid2, sender2, recipient2, "from-sender")
    assert broker.list_sent(recipient2) == []


# --- list_timeline ------------------------------------------------------


def test_list_timeline__shape_ordering_and_typed_columns():
    sid, sender, recipient = _setup_two_members()
    broker.send_message(sid, sender, recipient, "first")
    broker.send_message(sid, sender, recipient, "second")
    result = broker.list_timeline(sid)
    assert len(result) == 2
    assert result[0]["created_at"] >= result[1]["created_at"]
    entry = result[0]
    for key in ("task_id", "origin_task_id", "created_at", "text"):
        assert key in entry

    assert broker.list_timeline(_create_fleet()["fleet_id"]) == []


def test_list_timeline__filters_broadcast_summary_includes_delivery():
    sid, sender, _b_id, _ = _setup_three_members()
    broker.broadcast_message(sid, sender, "broadcast")
    result = broker.list_timeline(sid)
    assert all(entry["type"] != "broadcast_summary" for entry in result)
    # Delivery tasks (unicast) ARE present.
    assert len(result) >= 2


def test_list_timeline__fleet_scope_and_limit():
    fleet_a = _create_fleet()
    fleet_b = _create_fleet()
    a1 = _register_member(fleet_a["fleet_id"], name="a1")
    a2 = _register_member(fleet_a["fleet_id"], name="a2")
    b1 = _register_member(fleet_b["fleet_id"], name="b1")
    b2 = _register_member(fleet_b["fleet_id"], name="b2")
    broker.send_message(fleet_a["fleet_id"], a1["member_id"], a2["member_id"], "a-msg")
    broker.send_message(fleet_b["fleet_id"], b1["member_id"], b2["member_id"], "b-msg")
    assert len(broker.list_timeline(fleet_a["fleet_id"])) == 1
    assert len(broker.list_timeline(fleet_b["fleet_id"])) == 1

    sid, sender, recipient = _setup_two_members()
    for body in ("m1", "m2", "m3"):
        broker.send_message(sid, sender, recipient, body)
    assert len(broker.list_timeline(sid, limit=2)) == 2


# --- get_member_names ----------------------------------------------------


def test_get_member_names__returns_name_mapping_basic():
    fleet = _create_fleet()
    sid = fleet["fleet_id"]
    a1 = _register_member(sid, name="alpha")
    a2 = _register_member(sid, name="beta")
    result = broker.get_member_names([a1["member_id"], a2["member_id"]])
    assert result == {a1["member_id"]: "alpha", a2["member_id"]: "beta"}


@pytest.mark.parametrize(
    "scenario",
    ["empty_input", "nonexistent_id_absent", "deregistered_still_resolves"],
)
def test_get_member_names__edge_cases(scenario):
    if scenario == "empty_input":
        assert broker.get_member_names([]) == {}
    elif scenario == "nonexistent_id_absent":
        fleet = _create_fleet()
        member = _register_member(fleet["fleet_id"], name="real")
        fake = 999999
        result = broker.get_member_names([member["member_id"], fake])
        assert member["member_id"] in result
        assert fake not in result
    else:
        fleet = _create_fleet()
        member = _register_member(fleet["fleet_id"], name="departed")
        broker.deregister_member(member["member_id"])
        result = broker.get_member_names([member["member_id"]])
        assert result[member["member_id"]] == "departed"
