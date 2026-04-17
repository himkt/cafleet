"""Tests for ``broker`` WebUI query operations."""

import uuid

import pytest
from sqlalchemy import create_engine
from sqlalchemy.orm import sessionmaker

import cafleet.db.engine  # noqa: F401 — registers PRAGMA listener globally
from cafleet import broker
from cafleet.broker import ADMINISTRATOR_KIND
from cafleet.db.models import Base
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
) -> dict:
    return broker.register_agent(
        session_id=session_id,
        name=name,
        description=description,
    )


def _setup_two_agents() -> tuple[str, str, str]:
    session = _create_session()
    sid = session["session_id"]
    a = _register_agent(sid, name="sender")
    b = _register_agent(sid, name="recipient")
    return sid, a["agent_id"], b["agent_id"]


def _setup_three_agents() -> tuple[str, str, str, str]:
    session = _create_session()
    sid = session["session_id"]
    a = _register_agent(sid, name="agent-a")
    b = _register_agent(sid, name="agent-b")
    c = _register_agent(sid, name="agent-c")
    return sid, a["agent_id"], b["agent_id"], c["agent_id"]


class TestListSessionAgents:
    def test_returns_active_agents(self):
        session = _create_session()
        sid = session["session_id"]
        _register_agent(sid, name="active-1")
        _register_agent(sid, name="active-2")

        result = broker.list_session_agents(sid)
        assert len(result) == 4
        names = {a["name"] for a in result}
        assert "active-1" in names
        assert "active-2" in names
        assert "director" in names
        assert "Administrator" in names

    def test_active_agents_have_active_status(self):
        session = _create_session()
        sid = session["session_id"]
        _register_agent(sid, name="agent")

        result = broker.list_session_agents(sid)
        assert result[0]["status"] == "active"

    def test_includes_deregistered_agents_with_tasks(self):
        sid, sender, recipient = _setup_two_agents()

        broker.send_message(sid, sender, recipient, "keep me visible")
        broker.deregister_agent(recipient)

        result = broker.list_session_agents(sid)
        agent_ids = {a["agent_id"] for a in result}
        assert recipient in agent_ids

        deregistered = [a for a in result if a["agent_id"] == recipient]
        assert deregistered[0]["status"] == "deregistered"

    def test_excludes_deregistered_agents_without_tasks(self):
        session = _create_session()
        sid = session["session_id"]
        agent = _register_agent(sid, name="ghost")
        broker.deregister_agent(agent["agent_id"])

        result = broker.list_session_agents(sid)
        agent_ids = {a["agent_id"] for a in result}
        assert agent["agent_id"] not in agent_ids

    def test_newly_created_session_returns_bootstrap_pair(self):
        session = _create_session()
        result = broker.list_session_agents(session["session_id"])
        assert len(result) == 2
        names = {a["name"] for a in result}
        assert names == {"director", "Administrator"}

    def test_result_contains_required_keys(self):
        session = _create_session()
        sid = session["session_id"]
        _register_agent(sid, name="keyed")

        result = broker.list_session_agents(sid)
        agent = result[0]
        assert "agent_id" in agent
        assert "name" in agent
        assert "description" in agent
        assert "status" in agent
        assert "registered_at" in agent

    def test_scoped_to_session(self):
        session_a = _create_session()
        session_b = _create_session()
        _register_agent(session_a["session_id"], name="in-a")
        _register_agent(session_b["session_id"], name="in-b")

        result = broker.list_session_agents(session_a["session_id"])
        assert len(result) == 3
        names = {a["name"] for a in result}
        assert "in-a" in names
        assert "director" in names
        assert "Administrator" in names
        assert "in-b" not in names


class TestListSessionAgentsKind:
    """Testing at the broker layer covers the public HTTP surface because
    webui_api.py is a thin passthrough around broker.list_session_agents.
    """

    def test_entries_include_kind_field(self):
        session = _create_session()
        sid = session["session_id"]
        _register_agent(sid, name="user-a")
        _register_agent(sid, name="user-b")

        result = broker.list_session_agents(sid)
        for entry in result:
            assert "kind" in entry

    def test_administrator_marked_as_builtin_administrator(self):
        session = _create_session()
        sid = session["session_id"]
        _register_agent(sid, name="user-a")
        _register_agent(sid, name="user-b")

        result = broker.list_session_agents(sid)
        admins = [e for e in result if e["kind"] == ADMINISTRATOR_KIND]
        users = [e for e in result if e["kind"] == "user"]

        assert len(admins) == 1
        assert admins[0]["name"] == "Administrator"
        assert admins[0]["agent_id"] == session["administrator_agent_id"]

        assert len(users) == 3
        user_names = {e["name"] for e in users}
        assert user_names == {"director", "user-a", "user-b"}

    def test_kind_values_are_restricted_to_known_set(self):
        session = _create_session()
        sid = session["session_id"]
        _register_agent(sid, name="user-a")

        result = broker.list_session_agents(sid)
        valid_kinds = {ADMINISTRATOR_KIND, "user"}
        for entry in result:
            assert entry["kind"] in valid_kinds


class TestGetAgentKind:
    def test_get_agent_for_administrator_returns_builtin_administrator(self):
        session = _create_session()
        sid = session["session_id"]
        admin_id = session["administrator_agent_id"]

        result = broker.get_agent(admin_id, sid)
        assert result is not None
        assert "kind" in result
        assert result["kind"] == ADMINISTRATOR_KIND

    def test_get_agent_for_user_returns_user_kind(self):
        session = _create_session()
        sid = session["session_id"]
        user = _register_agent(sid, name="regular")

        result = broker.get_agent(user["agent_id"], sid)
        assert result is not None
        assert "kind" in result
        assert result["kind"] == "user"


class TestListInbox:
    def test_returns_inbox_tasks(self):
        sid, sender, recipient = _setup_two_agents()
        broker.send_message(sid, sender, recipient, "msg1")
        broker.send_message(sid, sender, recipient, "msg2")

        result = broker.list_inbox(recipient)
        assert len(result) == 2

    def test_returns_empty_when_no_tasks(self):
        session = _create_session()
        agent = _register_agent(session["session_id"], name="idle")
        result = broker.list_inbox(agent["agent_id"])
        assert result == []

    def test_ordered_by_status_timestamp_desc(self):
        sid, sender, recipient = _setup_two_agents()
        broker.send_message(sid, sender, recipient, "first")
        broker.send_message(sid, sender, recipient, "second")

        result = broker.list_inbox(recipient)
        assert len(result) == 2
        ts0 = result[0]["status_timestamp"]
        ts1 = result[1]["status_timestamp"]
        assert ts0 >= ts1

    def test_filters_out_broadcast_summary(self):
        sid, sender, _b_id, _ = _setup_three_agents()
        broker.broadcast_message(sid, sender, "broadcast")

        sender_inbox = broker.list_inbox(sender)
        summaries = [t for t in sender_inbox if t["type"] == "broadcast_summary"]
        assert len(summaries) == 0

    def test_only_returns_tasks_where_context_id_matches(self):
        sid, sender, recipient = _setup_two_agents()
        broker.send_message(sid, sender, recipient, "for-recipient")

        sender_inbox = broker.list_inbox(sender)
        assert len(sender_inbox) == 0

    def test_returns_raw_dicts(self):
        sid, sender, recipient = _setup_two_agents()
        broker.send_message(sid, sender, recipient, "raw")

        result = broker.list_inbox(recipient)
        assert len(result) == 1
        entry = result[0]
        assert isinstance(entry, dict)
        assert "task_id" in entry or "task_json" in entry


class TestListSent:
    def test_returns_sent_tasks(self):
        sid, sender, recipient = _setup_two_agents()
        broker.send_message(sid, sender, recipient, "sent1")
        broker.send_message(sid, sender, recipient, "sent2")

        result = broker.list_sent(sender)
        assert len(result) == 2

    def test_returns_empty_when_no_sent_tasks(self):
        session = _create_session()
        agent = _register_agent(session["session_id"], name="quiet")
        result = broker.list_sent(agent["agent_id"])
        assert result == []

    def test_ordered_by_status_timestamp_desc(self):
        sid, sender, recipient = _setup_two_agents()
        broker.send_message(sid, sender, recipient, "first")
        broker.send_message(sid, sender, recipient, "second")

        result = broker.list_sent(sender)
        assert len(result) == 2
        ts0 = result[0]["status_timestamp"]
        ts1 = result[1]["status_timestamp"]
        assert ts0 >= ts1

    def test_filters_out_broadcast_summary(self):
        sid, sender, _b_id, _ = _setup_three_agents()
        broker.broadcast_message(sid, sender, "broadcast")

        sent = broker.list_sent(sender)
        summaries = [t for t in sent if t["type"] == "broadcast_summary"]
        assert len(summaries) == 0

    def test_only_returns_tasks_from_agent(self):
        sid, sender, recipient = _setup_two_agents()
        broker.send_message(sid, sender, recipient, "from-sender")

        recipient_sent = broker.list_sent(recipient)
        assert len(recipient_sent) == 0

    def test_returns_raw_dicts(self):
        sid, sender, recipient = _setup_two_agents()
        broker.send_message(sid, sender, recipient, "raw")

        result = broker.list_sent(sender)
        assert len(result) == 1
        entry = result[0]
        assert isinstance(entry, dict)
        assert "task_id" in entry or "task_json" in entry


class TestListTimeline:
    def test_returns_timeline_entries(self):
        sid, sender, recipient = _setup_two_agents()
        broker.send_message(sid, sender, recipient, "timeline entry")

        result = broker.list_timeline(sid)
        assert len(result) == 1

    def test_returns_empty_for_no_tasks(self):
        session = _create_session()
        result = broker.list_timeline(session["session_id"])
        assert result == []

    def test_entry_has_required_keys(self):
        sid, sender, recipient = _setup_two_agents()
        broker.send_message(sid, sender, recipient, "structured")

        result = broker.list_timeline(sid)
        assert len(result) == 1
        entry = result[0]
        assert "task" in entry
        assert "origin_task_id" in entry or "created_at" in entry

    def test_ordered_by_status_timestamp_desc(self):
        sid, sender, recipient = _setup_two_agents()
        broker.send_message(sid, sender, recipient, "first")
        broker.send_message(sid, sender, recipient, "second")

        result = broker.list_timeline(sid)
        assert len(result) == 2
        assert result[0]["created_at"] >= result[1]["created_at"]

    def test_filters_broadcast_summary(self):
        sid, sender, _b_id, _c_id = _setup_three_agents()
        broker.broadcast_message(sid, sender, "broadcast")

        result = broker.list_timeline(sid)
        for entry in result:
            assert entry["task"]["metadata"]["type"] != "broadcast_summary"

    def test_scoped_to_session(self):
        session_a = _create_session()
        session_b = _create_session()
        sid_a = session_a["session_id"]
        sid_b = session_b["session_id"]

        a1 = _register_agent(sid_a, name="a1")
        a2 = _register_agent(sid_a, name="a2")
        b1 = _register_agent(sid_b, name="b1")
        b2 = _register_agent(sid_b, name="b2")

        broker.send_message(sid_a, a1["agent_id"], a2["agent_id"], "session-a msg")
        broker.send_message(sid_b, b1["agent_id"], b2["agent_id"], "session-b msg")

        result_a = broker.list_timeline(sid_a)
        result_b = broker.list_timeline(sid_b)
        assert len(result_a) == 1
        assert len(result_b) == 1

    def test_limit_parameter(self):
        sid, sender, recipient = _setup_two_agents()
        broker.send_message(sid, sender, recipient, "msg1")
        broker.send_message(sid, sender, recipient, "msg2")
        broker.send_message(sid, sender, recipient, "msg3")

        result = broker.list_timeline(sid, limit=2)
        assert len(result) == 2

    def test_includes_broadcast_delivery_tasks(self):
        sid, sender, _b_id, _c_id = _setup_three_agents()
        broker.broadcast_message(sid, sender, "Hello all")

        result = broker.list_timeline(sid)
        assert len(result) >= 2


class TestGetAgentNames:
    def test_returns_name_mapping(self):
        session = _create_session()
        sid = session["session_id"]
        a1 = _register_agent(sid, name="alpha")
        a2 = _register_agent(sid, name="beta")

        result = broker.get_agent_names([a1["agent_id"], a2["agent_id"]])
        assert isinstance(result, dict)
        assert result[a1["agent_id"]] == "alpha"
        assert result[a2["agent_id"]] == "beta"

    def test_empty_input_returns_empty_dict(self):
        result = broker.get_agent_names([])
        assert result == {}

    def test_nonexistent_agent_id_absent_from_result(self):
        session = _create_session()
        agent = _register_agent(session["session_id"], name="real")
        fake_id = str(uuid.uuid4())

        result = broker.get_agent_names([agent["agent_id"], fake_id])
        assert agent["agent_id"] in result
        assert fake_id not in result

    def test_includes_deregistered_agents(self):
        session = _create_session()
        sid = session["session_id"]
        agent = _register_agent(sid, name="departed")
        broker.deregister_agent(agent["agent_id"])

        result = broker.get_agent_names([agent["agent_id"]])
        assert result[agent["agent_id"]] == "departed"

    def test_single_agent(self):
        session = _create_session()
        agent = _register_agent(session["session_id"], name="solo")

        result = broker.get_agent_names([agent["agent_id"]])
        assert len(result) == 1
        assert result[agent["agent_id"]] == "solo"


class TestGetTaskCreatedAts:
    def test_returns_created_at_mapping(self):
        sid, sender, recipient = _setup_two_agents()
        sent1 = broker.send_message(sid, sender, recipient, "first")
        sent2 = broker.send_message(sid, sender, recipient, "second")
        tid1 = sent1["task"]["id"]
        tid2 = sent2["task"]["id"]

        result = broker.get_task_created_ats([tid1, tid2])
        assert isinstance(result, dict)
        assert tid1 in result
        assert tid2 in result
        assert isinstance(result[tid1], str)
        assert len(result[tid1]) > 0
        assert isinstance(result[tid2], str)
        assert len(result[tid2]) > 0

    def test_empty_input_returns_empty_dict(self):
        result = broker.get_task_created_ats([])
        assert result == {}

    def test_nonexistent_task_id_absent_from_result(self):
        sid, sender, recipient = _setup_two_agents()
        sent = broker.send_message(sid, sender, recipient, "real")
        tid = sent["task"]["id"]
        fake_id = str(uuid.uuid4())

        result = broker.get_task_created_ats([tid, fake_id])
        assert tid in result
        assert fake_id not in result

    def test_single_task(self):
        sid, sender, recipient = _setup_two_agents()
        sent = broker.send_message(sid, sender, recipient, "one")
        tid = sent["task"]["id"]

        result = broker.get_task_created_ats([tid])
        assert len(result) == 1
        assert tid in result

    def test_created_at_is_iso8601(self):
        sid, sender, recipient = _setup_two_agents()
        sent = broker.send_message(sid, sender, recipient, "timestamped")
        tid = sent["task"]["id"]

        result = broker.get_task_created_ats([tid])
        created_at = result[tid]
        assert "T" in created_at
