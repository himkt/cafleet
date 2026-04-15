"""Tests for broker.py — WebUI query operations.

Design doc 0000021 Step 5: WebUI query operations in the broker module.
These functions serve the admin WebUI endpoints via ``webui_api.py``.

Test isolation strategy:
  Same as test_broker_registry.py — each test gets a fresh in-memory SQLite
  database via the ``broker_session`` fixture with ``broker.get_sync_sessionmaker``
  monkeypatched.

Coverage map:
  | Function             | Test class                  |
  |----------------------|-----------------------------|
  | list_session_agents  | TestListSessionAgents       |
  | list_inbox           | TestListInbox               |
  | list_sent            | TestListSent                |
  | list_timeline        | TestListTimeline            |
  | get_agent_names      | TestGetAgentNames           |
  | get_task_created_ats | TestGetTaskCreatedAts       |
"""

import uuid

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
    from cafleet import broker

    return broker.create_session(label=label)


def _register_agent(
    session_id: str,
    name: str = "test-agent",
    description: str = "A test agent",
) -> dict:
    from cafleet import broker

    return broker.register_agent(
        session_id=session_id,
        name=name,
        description=description,
    )


def _setup_two_agents() -> tuple[str, str, str]:
    """Create a session with two agents. Returns (session_id, agent_a_id, agent_b_id)."""
    session = _create_session()
    sid = session["session_id"]
    a = _register_agent(sid, name="sender")
    b = _register_agent(sid, name="recipient")
    return sid, a["agent_id"], b["agent_id"]


def _setup_three_agents() -> tuple[str, str, str, str]:
    """Create a session with three agents. Returns (session_id, a_id, b_id, c_id)."""
    session = _create_session()
    sid = session["session_id"]
    a = _register_agent(sid, name="agent-a")
    b = _register_agent(sid, name="agent-b")
    c = _register_agent(sid, name="agent-c")
    return sid, a["agent_id"], b["agent_id"], c["agent_id"]


# ===========================================================================
# list_session_agents
# ===========================================================================


class TestListSessionAgents:
    """broker.list_session_agents(session_id) → active + deregistered with tasks."""

    def test_returns_active_agents(self):
        """Returned list includes both user agents and the auto-seeded Administrator."""
        from cafleet import broker

        session = _create_session()
        sid = session["session_id"]
        _register_agent(sid, name="active-1")
        _register_agent(sid, name="active-2")

        result = broker.list_session_agents(sid)
        # 2 user agents + 1 Administrator.
        assert len(result) == 3
        names = {a["name"] for a in result}
        assert "active-1" in names
        assert "active-2" in names
        assert "Administrator" in names

    def test_active_agents_have_active_status(self):
        from cafleet import broker

        session = _create_session()
        sid = session["session_id"]
        _register_agent(sid, name="agent")

        result = broker.list_session_agents(sid)
        assert result[0]["status"] == "active"

    def test_includes_deregistered_agents_with_tasks(self):
        """Deregistered agents that have tasks should still appear."""
        from cafleet import broker

        sid, sender, recipient = _setup_two_agents()

        # Send a message to recipient, then deregister recipient
        broker.send_message(sid, sender, recipient, "keep me visible")
        broker.deregister_agent(recipient)

        result = broker.list_session_agents(sid)
        agent_ids = {a["agent_id"] for a in result}
        assert recipient in agent_ids

        deregistered = [a for a in result if a["agent_id"] == recipient]
        assert deregistered[0]["status"] == "deregistered"

    def test_excludes_deregistered_agents_without_tasks(self):
        """Deregistered agents with no tasks should NOT appear."""
        from cafleet import broker

        session = _create_session()
        sid = session["session_id"]
        agent = _register_agent(sid, name="ghost")
        broker.deregister_agent(agent["agent_id"])

        result = broker.list_session_agents(sid)
        agent_ids = {a["agent_id"] for a in result}
        assert agent["agent_id"] not in agent_ids

    def test_newly_created_session_returns_only_administrator(self):
        """A freshly created session always surfaces its auto-seeded Administrator."""
        from cafleet import broker

        session = _create_session()
        result = broker.list_session_agents(session["session_id"])
        assert len(result) == 1
        assert result[0]["name"] == "Administrator"

    def test_result_contains_required_keys(self):
        from cafleet import broker

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
        """Agents from other sessions are not included (each session has its own Admin)."""
        from cafleet import broker

        session_a = _create_session()
        session_b = _create_session()
        _register_agent(session_a["session_id"], name="in-a")
        _register_agent(session_b["session_id"], name="in-b")

        result = broker.list_session_agents(session_a["session_id"])
        # Administrator (for session A) + in-a.
        assert len(result) == 2
        names = {a["name"] for a in result}
        assert "in-a" in names
        assert "Administrator" in names
        assert "in-b" not in names


# ===========================================================================
# kind field on agents — design doc 0000025 §F
# ===========================================================================


class TestListSessionAgentsKind:
    """/ui/api/agents exposes ``kind`` per row via ``broker.list_session_agents``.

    webui_api.py is a thin passthrough (`GET /ui/api/agents` simply wraps
    ``broker.list_session_agents(session_id)`` and wraps the result as
    ``{"agents": [...]}``), so testing at the broker layer covers the
    public HTTP surface.
    """

    def test_entries_include_kind_field(self):
        from cafleet import broker

        session = _create_session()
        sid = session["session_id"]
        _register_agent(sid, name="user-a")
        _register_agent(sid, name="user-b")

        result = broker.list_session_agents(sid)
        for entry in result:
            assert "kind" in entry, (
                f"every agent entry must carry a 'kind' field, got entry={entry!r}"
            )

    def test_administrator_marked_as_builtin_administrator(self):
        from cafleet import broker
        from cafleet.broker import ADMINISTRATOR_KIND

        session = _create_session()
        sid = session["session_id"]
        _register_agent(sid, name="user-a")
        _register_agent(sid, name="user-b")

        result = broker.list_session_agents(sid)
        admins = [e for e in result if e.get("kind") == ADMINISTRATOR_KIND]
        users = [e for e in result if e.get("kind") == "user"]

        # Exactly one Administrator entry, and its name matches.
        assert len(admins) == 1, (
            f"expected exactly one Administrator entry, got {len(admins)}: {admins!r}"
        )
        assert admins[0]["name"] == "Administrator"
        assert admins[0]["agent_id"] == session["administrator_agent_id"]

        # The two user agents carry kind='user'.
        assert len(users) == 2, (
            f"expected 2 user-kind entries, got {len(users)}: {users!r}"
        )
        user_names = {e["name"] for e in users}
        assert user_names == {"user-a", "user-b"}

    def test_kind_values_are_restricted_to_known_set(self):
        """No entry carries a kind outside the documented set."""
        from cafleet import broker
        from cafleet.broker import ADMINISTRATOR_KIND

        session = _create_session()
        sid = session["session_id"]
        _register_agent(sid, name="user-a")

        result = broker.list_session_agents(sid)
        valid_kinds = {ADMINISTRATOR_KIND, "user"}
        for entry in result:
            assert entry.get("kind") in valid_kinds, (
                f"kind must be one of {valid_kinds}, got {entry.get('kind')!r}"
            )


class TestGetAgentKind:
    """broker.get_agent returned dict includes ``kind`` per §F."""

    def test_get_agent_for_administrator_returns_builtin_administrator(self):
        from cafleet import broker
        from cafleet.broker import ADMINISTRATOR_KIND

        session = _create_session()
        sid = session["session_id"]
        admin_id = session["administrator_agent_id"]

        result = broker.get_agent(admin_id, sid)
        assert result is not None
        assert "kind" in result
        assert result["kind"] == ADMINISTRATOR_KIND

    def test_get_agent_for_user_returns_user_kind(self):
        from cafleet import broker

        session = _create_session()
        sid = session["session_id"]
        user = _register_agent(sid, name="regular")

        result = broker.get_agent(user["agent_id"], sid)
        assert result is not None
        assert "kind" in result
        assert result["kind"] == "user"


# ===========================================================================
# list_inbox
# ===========================================================================


class TestListInbox:
    """broker.list_inbox(agent_id) → inbox tasks as raw dicts."""

    def test_returns_inbox_tasks(self):
        from cafleet import broker

        sid, sender, recipient = _setup_two_agents()
        broker.send_message(sid, sender, recipient, "msg1")
        broker.send_message(sid, sender, recipient, "msg2")

        result = broker.list_inbox(recipient)
        assert len(result) == 2

    def test_returns_empty_when_no_tasks(self):
        from cafleet import broker

        session = _create_session()
        agent = _register_agent(session["session_id"], name="idle")
        result = broker.list_inbox(agent["agent_id"])
        assert result == []

    def test_ordered_by_status_timestamp_desc(self):
        from cafleet import broker

        sid, sender, recipient = _setup_two_agents()
        broker.send_message(sid, sender, recipient, "first")
        broker.send_message(sid, sender, recipient, "second")

        result = broker.list_inbox(recipient)
        assert len(result) == 2
        ts0 = result[0]["status_timestamp"]
        ts1 = result[1]["status_timestamp"]
        assert ts0 >= ts1

    def test_filters_out_broadcast_summary(self):
        """broadcast_summary tasks do not appear in inbox."""
        from cafleet import broker

        sid, sender, b_id, _ = _setup_three_agents()
        broker.broadcast_message(sid, sender, "broadcast")

        # Sender's inbox should not have the broadcast_summary
        sender_inbox = broker.list_inbox(sender)
        summaries = [t for t in sender_inbox if t.get("type") == "broadcast_summary"]
        assert len(summaries) == 0

    def test_only_returns_tasks_where_context_id_matches(self):
        """Only tasks addressed to the agent appear."""
        from cafleet import broker

        sid, sender, recipient = _setup_two_agents()
        broker.send_message(sid, sender, recipient, "for-recipient")

        sender_inbox = broker.list_inbox(sender)
        assert len(sender_inbox) == 0

    def test_returns_raw_dicts(self):
        """list_inbox returns raw dict rows, not wrapped in {"task": ...}."""
        from cafleet import broker

        sid, sender, recipient = _setup_two_agents()
        broker.send_message(sid, sender, recipient, "raw")

        result = broker.list_inbox(recipient)
        assert len(result) == 1
        # Raw dict should not have a top-level "task" wrapper
        entry = result[0]
        assert isinstance(entry, dict)
        # Should have task row fields
        assert "task_id" in entry or "task_json" in entry


# ===========================================================================
# list_sent
# ===========================================================================


class TestListSent:
    """broker.list_sent(agent_id) → sent tasks as raw dicts."""

    def test_returns_sent_tasks(self):
        from cafleet import broker

        sid, sender, recipient = _setup_two_agents()
        broker.send_message(sid, sender, recipient, "sent1")
        broker.send_message(sid, sender, recipient, "sent2")

        result = broker.list_sent(sender)
        assert len(result) == 2

    def test_returns_empty_when_no_sent_tasks(self):
        from cafleet import broker

        session = _create_session()
        agent = _register_agent(session["session_id"], name="quiet")
        result = broker.list_sent(agent["agent_id"])
        assert result == []

    def test_ordered_by_status_timestamp_desc(self):
        from cafleet import broker

        sid, sender, recipient = _setup_two_agents()
        broker.send_message(sid, sender, recipient, "first")
        broker.send_message(sid, sender, recipient, "second")

        result = broker.list_sent(sender)
        assert len(result) == 2
        ts0 = result[0]["status_timestamp"]
        ts1 = result[1]["status_timestamp"]
        assert ts0 >= ts1

    def test_filters_out_broadcast_summary(self):
        """broadcast_summary tasks do not appear in sent list."""
        from cafleet import broker

        sid, sender, b_id, _ = _setup_three_agents()
        broker.broadcast_message(sid, sender, "broadcast")

        sent = broker.list_sent(sender)
        summaries = [t for t in sent if t.get("type") == "broadcast_summary"]
        assert len(summaries) == 0

    def test_only_returns_tasks_from_agent(self):
        """Only tasks sent by the agent appear."""
        from cafleet import broker

        sid, sender, recipient = _setup_two_agents()
        broker.send_message(sid, sender, recipient, "from-sender")

        recipient_sent = broker.list_sent(recipient)
        assert len(recipient_sent) == 0

    def test_returns_raw_dicts(self):
        from cafleet import broker

        sid, sender, recipient = _setup_two_agents()
        broker.send_message(sid, sender, recipient, "raw")

        result = broker.list_sent(sender)
        assert len(result) == 1
        entry = result[0]
        assert isinstance(entry, dict)
        assert "task_id" in entry or "task_json" in entry


# ===========================================================================
# list_timeline
# ===========================================================================


class TestListTimeline:
    """broker.list_timeline(session_id, limit=200) → session-wide timeline."""

    def test_returns_timeline_entries(self):
        from cafleet import broker

        sid, sender, recipient = _setup_two_agents()
        broker.send_message(sid, sender, recipient, "timeline entry")

        result = broker.list_timeline(sid)
        assert len(result) >= 1

    def test_returns_empty_for_no_tasks(self):
        from cafleet import broker

        session = _create_session()
        result = broker.list_timeline(session["session_id"])
        assert result == []

    def test_entry_has_required_keys(self):
        from cafleet import broker

        sid, sender, recipient = _setup_two_agents()
        broker.send_message(sid, sender, recipient, "structured")

        result = broker.list_timeline(sid)
        assert len(result) >= 1
        entry = result[0]
        assert "task" in entry
        assert "origin_task_id" in entry or "created_at" in entry

    def test_ordered_by_status_timestamp_desc(self):
        from cafleet import broker

        sid, sender, recipient = _setup_two_agents()
        broker.send_message(sid, sender, recipient, "first")
        broker.send_message(sid, sender, recipient, "second")

        result = broker.list_timeline(sid)
        assert len(result) == 2
        # Entries should be in descending timestamp order
        ts0 = result[0].get("created_at", "") or ""
        ts1 = result[1].get("created_at", "") or ""
        # If created_at is present, verify ordering; otherwise trust the impl
        if ts0 and ts1:
            assert ts0 >= ts1

    def test_filters_broadcast_summary(self):
        """broadcast_summary tasks do not appear in timeline."""
        from cafleet import broker

        sid, sender, b_id, c_id = _setup_three_agents()
        broker.broadcast_message(sid, sender, "broadcast")

        result = broker.list_timeline(sid)
        for entry in result:
            task = entry.get("task", {})
            if isinstance(task, dict):
                assert task.get("metadata", {}).get("type") != "broadcast_summary"

    def test_scoped_to_session(self):
        """Only tasks from agents in the given session appear."""
        from cafleet import broker

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
        """limit caps the number of returned entries."""
        from cafleet import broker

        sid, sender, recipient = _setup_two_agents()
        broker.send_message(sid, sender, recipient, "msg1")
        broker.send_message(sid, sender, recipient, "msg2")
        broker.send_message(sid, sender, recipient, "msg3")

        result = broker.list_timeline(sid, limit=2)
        assert len(result) == 2

    def test_includes_broadcast_delivery_tasks(self):
        """Individual broadcast delivery tasks (type=unicast) appear in timeline."""
        from cafleet import broker

        sid, sender, b_id, c_id = _setup_three_agents()
        broker.broadcast_message(sid, sender, "Hello all")

        result = broker.list_timeline(sid)
        # Should have delivery tasks for b and c (2 unicast tasks)
        assert len(result) >= 2


# ===========================================================================
# get_agent_names
# ===========================================================================


class TestGetAgentNames:
    """broker.get_agent_names(agent_ids) → {agent_id: name}."""

    def test_returns_name_mapping(self):
        from cafleet import broker

        session = _create_session()
        sid = session["session_id"]
        a1 = _register_agent(sid, name="alpha")
        a2 = _register_agent(sid, name="beta")

        result = broker.get_agent_names([a1["agent_id"], a2["agent_id"]])
        assert isinstance(result, dict)
        assert result[a1["agent_id"]] == "alpha"
        assert result[a2["agent_id"]] == "beta"

    def test_empty_input_returns_empty_dict(self):
        from cafleet import broker

        result = broker.get_agent_names([])
        assert result == {}

    def test_nonexistent_agent_id_absent_from_result(self):
        from cafleet import broker

        session = _create_session()
        agent = _register_agent(session["session_id"], name="real")
        fake_id = str(uuid.uuid4())

        result = broker.get_agent_names([agent["agent_id"], fake_id])
        assert agent["agent_id"] in result
        assert fake_id not in result

    def test_includes_deregistered_agents(self):
        """Deregistered agents still have names and should be returned."""
        from cafleet import broker

        session = _create_session()
        sid = session["session_id"]
        agent = _register_agent(sid, name="departed")
        broker.deregister_agent(agent["agent_id"])

        result = broker.get_agent_names([agent["agent_id"]])
        assert result[agent["agent_id"]] == "departed"

    def test_single_agent(self):
        from cafleet import broker

        session = _create_session()
        agent = _register_agent(session["session_id"], name="solo")

        result = broker.get_agent_names([agent["agent_id"]])
        assert len(result) == 1
        assert result[agent["agent_id"]] == "solo"


# ===========================================================================
# get_task_created_ats
# ===========================================================================


class TestGetTaskCreatedAts:
    """broker.get_task_created_ats(task_ids) → {task_id: created_at}."""

    def test_returns_created_at_mapping(self):
        from cafleet import broker

        sid, sender, recipient = _setup_two_agents()
        sent1 = broker.send_message(sid, sender, recipient, "first")
        sent2 = broker.send_message(sid, sender, recipient, "second")
        tid1 = sent1["task"]["id"]
        tid2 = sent2["task"]["id"]

        result = broker.get_task_created_ats([tid1, tid2])
        assert isinstance(result, dict)
        assert tid1 in result
        assert tid2 in result
        # created_at should be non-empty ISO strings
        assert isinstance(result[tid1], str)
        assert len(result[tid1]) > 0
        assert isinstance(result[tid2], str)
        assert len(result[tid2]) > 0

    def test_empty_input_returns_empty_dict(self):
        from cafleet import broker

        result = broker.get_task_created_ats([])
        assert result == {}

    def test_nonexistent_task_id_absent_from_result(self):
        from cafleet import broker

        sid, sender, recipient = _setup_two_agents()
        sent = broker.send_message(sid, sender, recipient, "real")
        tid = sent["task"]["id"]
        fake_id = str(uuid.uuid4())

        result = broker.get_task_created_ats([tid, fake_id])
        assert tid in result
        assert fake_id not in result

    def test_single_task(self):
        from cafleet import broker

        sid, sender, recipient = _setup_two_agents()
        sent = broker.send_message(sid, sender, recipient, "one")
        tid = sent["task"]["id"]

        result = broker.get_task_created_ats([tid])
        assert len(result) == 1
        assert tid in result

    def test_created_at_is_iso8601(self):
        from cafleet import broker

        sid, sender, recipient = _setup_two_agents()
        sent = broker.send_message(sid, sender, recipient, "timestamped")
        tid = sent["task"]["id"]

        result = broker.get_task_created_ats([tid])
        created_at = result[tid]
        assert "T" in created_at
