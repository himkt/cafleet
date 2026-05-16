"""Tests for ``broker`` WebUI query operations."""

import uuid

import pytest

from cafleet import broker
from cafleet.broker import ADMINISTRATOR_KIND
from tests._broker_helpers import (
    _create_session,
    _register_agent,
    _setup_three_agents,
    _setup_two_agents,
)


@pytest.fixture(autouse=True)
def _autouse_broker(broker_session):
    return broker_session


# --- list_session_agents ------------------------------------------------


def test_list_session_agents__active_shape_required_keys_and_session_scope():
    session = _create_session()
    sid = session["session_id"]
    _register_agent(sid, name="active-1")
    _register_agent(sid, name="active-2")

    result = broker.list_session_agents(sid)
    assert len(result) == 4
    assert {a["name"] for a in result} == {
        "active-1",
        "active-2",
        "Director",
        "Administrator",
    }
    agent = result[0]
    for key in ("agent_id", "name", "description", "status", "registered_at"):
        assert key in agent
    assert agent["status"] == "active"

    # Newly-created session lists exactly Director + Administrator.
    bare = _create_session()
    bare_result = broker.list_session_agents(bare["session_id"])
    assert {a["name"] for a in bare_result} == {"Director", "Administrator"}

    # Scoped per session.
    other = _create_session()
    _register_agent(other["session_id"], name="in-other")
    assert "in-other" not in {a["name"] for a in broker.list_session_agents(sid)}


def test_list_session_agents__deregistered_with_or_without_tasks():
    sid, sender, recipient = _setup_two_agents()
    broker.send_message(sid, sender, recipient, "keep visible")
    broker.deregister_agent(recipient)
    result = broker.list_session_agents(sid)
    deregistered = [a for a in result if a["agent_id"] == recipient]
    assert deregistered[0]["status"] == "deregistered"

    ghost = _register_agent(sid, name="ghost")
    broker.deregister_agent(ghost["agent_id"])
    result = broker.list_session_agents(sid)
    assert ghost["agent_id"] not in {a["agent_id"] for a in result}


@pytest.mark.parametrize(
    ("scenario", "expected_kind"),
    [
        ("administrator", ADMINISTRATOR_KIND),
        ("user_agent", "user"),
    ],
)
def test_get_agent__kind_field(scenario, expected_kind):
    session = _create_session()
    sid = session["session_id"]
    if scenario == "administrator":
        agent_id = session["administrator_agent_id"]
    else:
        agent_id = _register_agent(sid, name="regular")["agent_id"]
    result = broker.get_agent(agent_id, sid)
    assert result["kind"] == expected_kind

    # All list_session_agents entries have a kind in {administrator, user}.
    all_entries = broker.list_session_agents(sid)
    for entry in all_entries:
        assert entry["kind"] in {ADMINISTRATOR_KIND, "user"}


# --- list_inbox ---------------------------------------------------------


def test_list_inbox__shape_ordering_and_typed_columns():
    sid, sender, recipient = _setup_two_agents()
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
    idle = _register_agent(_create_session()["session_id"], name="idle")
    assert broker.list_inbox(idle["agent_id"]) == []


def test_list_inbox__filters_broadcast_summary_and_context_id_scope():
    sid, sender, _b_id, _ = _setup_three_agents()
    broker.broadcast_message(sid, sender, "broadcast")
    summaries = [
        t for t in broker.list_inbox(sender) if t["type"] == "broadcast_summary"
    ]
    assert summaries == []

    sid2, sender2, recipient2 = _setup_two_agents()
    broker.send_message(sid2, sender2, recipient2, "for-recipient")
    assert broker.list_inbox(sender2) == []


# --- list_sent ----------------------------------------------------------


def test_list_sent__shape_ordering_and_typed_columns():
    sid, sender, recipient = _setup_two_agents()
    broker.send_message(sid, sender, recipient, "first")
    broker.send_message(sid, sender, recipient, "second")
    result = broker.list_sent(sender)
    assert len(result) == 2
    assert result[0]["status_timestamp"] >= result[1]["status_timestamp"]
    entry = result[0]
    assert "task_id" in entry
    assert "text" in entry
    assert "task_json" not in entry

    quiet = _register_agent(_create_session()["session_id"], name="quiet")
    assert broker.list_sent(quiet["agent_id"]) == []


def test_list_sent__filters_broadcast_summary_and_from_agent_scope():
    sid, sender, _b_id, _ = _setup_three_agents()
    broker.broadcast_message(sid, sender, "broadcast")
    summaries = [
        t for t in broker.list_sent(sender) if t["type"] == "broadcast_summary"
    ]
    assert summaries == []

    sid2, sender2, recipient2 = _setup_two_agents()
    broker.send_message(sid2, sender2, recipient2, "from-sender")
    assert broker.list_sent(recipient2) == []


# --- list_timeline ------------------------------------------------------


def test_list_timeline__shape_ordering_and_typed_columns():
    sid, sender, recipient = _setup_two_agents()
    broker.send_message(sid, sender, recipient, "first")
    broker.send_message(sid, sender, recipient, "second")
    result = broker.list_timeline(sid)
    assert len(result) == 2
    assert result[0]["created_at"] >= result[1]["created_at"]
    entry = result[0]
    for key in ("task_id", "origin_task_id", "created_at", "text"):
        assert key in entry

    assert broker.list_timeline(_create_session()["session_id"]) == []


def test_list_timeline__filters_broadcast_summary_includes_delivery():
    sid, sender, _b_id, _ = _setup_three_agents()
    broker.broadcast_message(sid, sender, "broadcast")
    result = broker.list_timeline(sid)
    assert all(entry["type"] != "broadcast_summary" for entry in result)
    # Delivery tasks (unicast) ARE present.
    assert len(result) >= 2


def test_list_timeline__session_scope_and_limit():
    session_a = _create_session()
    session_b = _create_session()
    a1 = _register_agent(session_a["session_id"], name="a1")
    a2 = _register_agent(session_a["session_id"], name="a2")
    b1 = _register_agent(session_b["session_id"], name="b1")
    b2 = _register_agent(session_b["session_id"], name="b2")
    broker.send_message(
        session_a["session_id"], a1["agent_id"], a2["agent_id"], "a-msg"
    )
    broker.send_message(
        session_b["session_id"], b1["agent_id"], b2["agent_id"], "b-msg"
    )
    assert len(broker.list_timeline(session_a["session_id"])) == 1
    assert len(broker.list_timeline(session_b["session_id"])) == 1

    sid, sender, recipient = _setup_two_agents()
    for body in ("m1", "m2", "m3"):
        broker.send_message(sid, sender, recipient, body)
    assert len(broker.list_timeline(sid, limit=2)) == 2


# --- get_agent_names ----------------------------------------------------


def test_get_agent_names__returns_name_mapping_basic():
    session = _create_session()
    sid = session["session_id"]
    a1 = _register_agent(sid, name="alpha")
    a2 = _register_agent(sid, name="beta")
    result = broker.get_agent_names([a1["agent_id"], a2["agent_id"]])
    assert result == {a1["agent_id"]: "alpha", a2["agent_id"]: "beta"}


@pytest.mark.parametrize(
    "scenario",
    ["empty_input", "nonexistent_id_absent", "deregistered_still_resolves"],
)
def test_get_agent_names__edge_cases(scenario):
    if scenario == "empty_input":
        assert broker.get_agent_names([]) == {}
    elif scenario == "nonexistent_id_absent":
        session = _create_session()
        agent = _register_agent(session["session_id"], name="real")
        fake = str(uuid.uuid4())
        result = broker.get_agent_names([agent["agent_id"], fake])
        assert agent["agent_id"] in result
        assert fake not in result
    else:
        session = _create_session()
        agent = _register_agent(session["session_id"], name="departed")
        broker.deregister_agent(agent["agent_id"])
        result = broker.get_agent_names([agent["agent_id"]])
        assert result[agent["agent_id"]] == "departed"
