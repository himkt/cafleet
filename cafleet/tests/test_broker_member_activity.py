"""Tests for ``broker.list_members_with_activity`` (design doc 0000049, Surface 8).

The broker exposes a per-member activity aggregation built from the ``tasks``
table — one row per active member of the calling Director, augmented with
``last_sent`` / ``last_recv`` / ``last_ack`` / ``idle`` columns.

Design doc requirements exercised here:

* Returns the same per-member identity + placement shape as ``broker.list_members``
  (so the existing JSON consumers stay compatible) plus the four new keys.
* ``last_sent`` is the most recent ``status_timestamp`` of tasks where the
  member is the *sender* (``from_agent_id``).
* ``last_recv`` is the most recent ``status_timestamp`` of tasks routed to the
  member's inbox (``context_id == member``), excluding ``broadcast_summary``
  rows so a broadcaster's own self-routed summary cannot pollute the proxy.
* ``last_ack`` is the most recent ``status_timestamp`` of tasks in the member's
  inbox that have transitioned to ``status_state == 'completed'``, again
  filtered to ``Task.type != 'broadcast_summary'`` (mirrors ``poll_tasks``;
  broadcast_summary rows are seeded ``status_state='completed'`` at send time
  and would otherwise pollute the proxy for the broadcaster).
* Missing-activity rows expose the four keys as ``None`` rather than omitting
  them, so downstream formatters can render a uniform table.
* The aggregation is scoped per-Director: members of another Director in the
  same session do not appear in the result.
* Deregistered members do not appear in the result.
"""

import uuid

import pytest
from sqlalchemy import create_engine
from sqlalchemy.orm import sessionmaker

import cafleet.db.engine  # noqa: F401 — registers PRAGMA listener globally
from cafleet import broker
from cafleet.db.models import Base
from cafleet.tmux import DirectorContext


@pytest.fixture
def sync_sessionmaker():
    engine = create_engine("sqlite:///:memory:")
    Base.metadata.create_all(engine)
    return sessionmaker(engine, expire_on_commit=False)


@pytest.fixture(autouse=True)
def _patch_broker(sync_sessionmaker, monkeypatch):
    monkeypatch.setattr(broker, "get_sync_sessionmaker", lambda: sync_sessionmaker)


def _bootstrap_session() -> tuple[str, str]:
    """Create a session and return ``(session_id, director_agent_id)``."""
    info = broker.create_session(
        label="activity-test",
        director_context=DirectorContext(session="main", window_id="@3", pane_id="%0"),
        coding_agent="claude",
    )
    return info["session_id"], info["director"]["agent_id"]


def _register_member(session_id: str, director_id: str, name: str, pane: str) -> str:
    """Register a fake member with a placement row pointing at ``director_id``."""
    placement = {
        "director_agent_id": director_id,
        "tmux_session": "main",
        "tmux_window_id": "@3",
        "tmux_pane_id": pane,
        "coding_agent": "claude",
    }
    agent = broker.register_agent(
        session_id=session_id,
        name=name,
        description=f"member {name}",
        placement=placement,
    )
    return agent["agent_id"]


def _setup_three_member_team() -> tuple[str, str, str, str, str]:
    """Three members spawned by the same Director.

    Returns ``(session_id, director_id, member_a, member_b, member_c)``.
    """
    sid, director_id = _bootstrap_session()
    a = _register_member(sid, director_id, "alice", "%10")
    b = _register_member(sid, director_id, "bob", "%11")
    c = _register_member(sid, director_id, "carol", "%12")
    return sid, director_id, a, b, c


# --- shape ---


def test_list_members_with_activity__returns_one_row_per_member():
    sid, director_id, a, b, c = _setup_three_member_team()

    rows = broker.list_members_with_activity(sid, director_id)

    assert len(rows) == 3
    assert {row["agent_id"] for row in rows} == {a, b, c}


def test_list_members_with_activity__row_carries_identity_and_placement():
    """The new function MUST stay shape-compatible with ``list_members`` so
    downstream JSON consumers see a strict superset of the existing keys."""
    sid, director_id, a, _b, _c = _setup_three_member_team()

    rows = broker.list_members_with_activity(sid, director_id)
    alice = next(r for r in rows if r["agent_id"] == a)

    assert alice["name"] == "alice"
    assert alice["status"] == "active"
    assert alice["placement"]["director_agent_id"] == director_id
    assert alice["placement"]["tmux_pane_id"] == "%10"


def test_list_members_with_activity__row_carries_activity_keys():
    """Every member row exposes the four activity keys regardless of whether
    the member has any tasks yet — so the formatter can render a uniform
    table without per-row branching."""
    sid, director_id, a, _b, _c = _setup_three_member_team()

    rows = broker.list_members_with_activity(sid, director_id)
    alice = next(r for r in rows if r["agent_id"] == a)

    assert "last_sent" in alice
    assert "last_recv" in alice
    assert "last_ack" in alice
    assert "idle" in alice


# --- last_sent ---


def test_last_sent__none_when_member_never_sent():
    sid, director_id, a, _b, _c = _setup_three_member_team()

    rows = broker.list_members_with_activity(sid, director_id)
    alice = next(r for r in rows if r["agent_id"] == a)

    assert alice["last_sent"] is None


def test_last_sent__most_recent_status_timestamp_of_sent_tasks():
    sid, director_id, a, b, _c = _setup_three_member_team()

    broker.send_message(sid, a, b, "first")
    second = broker.send_message(sid, a, b, "second")

    rows = broker.list_members_with_activity(sid, director_id)
    alice = next(r for r in rows if r["agent_id"] == a)

    assert alice["last_sent"] == second["task"]["status_timestamp"]


# --- last_recv ---


def test_last_recv__none_when_member_never_received():
    sid, director_id, a, _b, _c = _setup_three_member_team()

    rows = broker.list_members_with_activity(sid, director_id)
    alice = next(r for r in rows if r["agent_id"] == a)

    assert alice["last_recv"] is None


def test_last_recv__most_recent_status_timestamp_of_received_tasks():
    sid, director_id, a, b, _c = _setup_three_member_team()

    broker.send_message(sid, a, b, "first")
    second = broker.send_message(sid, a, b, "second")

    rows = broker.list_members_with_activity(sid, director_id)
    bob = next(r for r in rows if r["agent_id"] == b)

    assert bob["last_recv"] == second["task"]["status_timestamp"]


# --- last_ack ---


def test_last_ack__none_until_member_acks_a_task():
    sid, director_id, a, b, _c = _setup_three_member_team()

    broker.send_message(sid, a, b, "hi")

    rows = broker.list_members_with_activity(sid, director_id)
    bob = next(r for r in rows if r["agent_id"] == b)

    assert bob["last_ack"] is None


def test_last_ack__set_after_recipient_acks():
    sid, director_id, a, b, _c = _setup_three_member_team()

    sent = broker.send_message(sid, a, b, "hi")
    acked = broker.ack_task(b, sent["task"]["task_id"])

    rows = broker.list_members_with_activity(sid, director_id)
    bob = next(r for r in rows if r["agent_id"] == b)

    assert bob["last_ack"] == acked["task"]["status_timestamp"]


def test_last_ack__excludes_broadcast_summary_for_broadcaster():
    """Surface 8: the ``last_ack`` proxy MUST filter ``Task.type !=
    'broadcast_summary'`` (mirrors ``poll_tasks``). Broadcast summary tasks
    are persisted with ``status_state='completed'`` at send time and have
    ``context_id == broadcaster``; without the filter, ``last_ack`` for a
    broadcaster would surface every broadcast as a phantom ack."""
    sid, director_id, a, _b, _c = _setup_three_member_team()

    # alice broadcasts; the summary task lands in alice's own context with
    # type='broadcast_summary' and status_state='completed'.
    broker.broadcast_message(sid, a, "hello team")

    rows = broker.list_members_with_activity(sid, director_id)
    alice = next(r for r in rows if r["agent_id"] == a)

    # Alice has no real ack — only the broadcast_summary, which the proxy
    # MUST filter out.
    assert alice["last_ack"] is None


def test_last_ack__broadcast_summary_filter_keeps_real_acks():
    """Sanity check: the filter rejects broadcast_summary specifically, not
    every ``status_state='completed'`` row. Real acks for unicast deliveries
    still register in the proxy."""
    sid, director_id, a, b, _c = _setup_three_member_team()

    sent = broker.send_message(sid, b, a, "ping")
    acked = broker.ack_task(a, sent["task"]["task_id"])
    broker.broadcast_message(sid, a, "team-wide note")

    rows = broker.list_members_with_activity(sid, director_id)
    alice = next(r for r in rows if r["agent_id"] == a)

    assert alice["last_ack"] == acked["task"]["status_timestamp"]


def test_last_recv__excludes_broadcast_summary_for_broadcaster():
    """A broadcaster's own broadcast_summary task is routed to
    ``context_id == broadcaster`` but is not a real inbox delivery. The
    same filter that protects ``last_ack`` also keeps ``last_recv`` honest:
    polling alice's inbox via ``poll_tasks`` already excludes the summary
    (broker.py line 757), so the activity proxy MUST too."""
    sid, director_id, a, _b, _c = _setup_three_member_team()

    broker.broadcast_message(sid, a, "team-wide note")

    rows = broker.list_members_with_activity(sid, director_id)
    alice = next(r for r in rows if r["agent_id"] == a)

    assert alice["last_recv"] is None


# --- idle column ---


def test_idle__none_when_member_has_no_activity():
    """A member that has never sent or received a message has no
    meaningful idle duration to report."""
    sid, director_id, a, _b, _c = _setup_three_member_team()

    rows = broker.list_members_with_activity(sid, director_id)
    alice = next(r for r in rows if r["agent_id"] == a)

    assert alice["idle"] is None


def test_idle__present_after_any_activity():
    """Once the member has any send/receive event the proxy reports a
    non-None idle indicator. We deliberately do NOT pin the exact value
    (formatter / unit choice is a downstream concern); we only assert the
    column transitions out of ``None``."""
    sid, director_id, a, b, _c = _setup_three_member_team()

    broker.send_message(sid, a, b, "hello")

    rows = broker.list_members_with_activity(sid, director_id)
    alice = next(r for r in rows if r["agent_id"] == a)

    assert alice["idle"] is not None


# --- scoping ---


def test_list_members_with_activity__excludes_other_directors_members():
    """The aggregation reuses ``list_members``'s per-Director scoping:
    a row appears only when its placement's ``director_agent_id`` matches
    the caller. A second-level Director's own children must not leak into
    the root Director's view."""
    sid, director_id, a, b, c = _setup_three_member_team()

    # Spawn a second-level Director (parent = root) and give it its own
    # child member. ``outsider``'s placement points at ``second_director``,
    # not at the root.
    second_director = _register_member(sid, director_id, "director-two", "%20")
    _register_member(sid, second_director, "outsider", "%21")

    rows = broker.list_members_with_activity(sid, director_id)
    agent_ids = {row["agent_id"] for row in rows}

    # The root sees its own children (including the second-level Director)
    # but never reaches across the boundary into ``outsider``.
    assert agent_ids == {a, b, c, second_director}


def test_list_members_with_activity__excludes_deregistered_members():
    sid, director_id, a, b, c = _setup_three_member_team()

    broker.deregister_agent(c)

    rows = broker.list_members_with_activity(sid, director_id)

    assert {row["agent_id"] for row in rows} == {a, b}


def test_list_members_with_activity__empty_when_no_members():
    sid, director_id = _bootstrap_session()

    rows = broker.list_members_with_activity(sid, director_id)

    assert rows == []


def test_list_members_with_activity__rejects_unknown_session():
    """An unknown session-id surfaces an empty list rather than raising,
    matching ``broker.list_members``'s behaviour. This keeps the CLI's
    error handling unchanged."""
    bogus_session = str(uuid.uuid4())
    bogus_director = str(uuid.uuid4())

    rows = broker.list_members_with_activity(bogus_session, bogus_director)

    assert rows == []
